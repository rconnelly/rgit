//! Git tree / blob / blame / log for the web UI (`rgit repo tree|blob|blame|…`).
//!
//! This is the forge-side git extension layer (the "rgit-repo" surface). It
//! shells out to system `git`; it does not replace git objects.

use anyhow::{bail, Context, Result};
use serde::Serialize;

use crate::acl::{self, Actor, Role};
use crate::git;
use crate::names::RepoName;
use crate::output;
use crate::store::Store;
use crate::view::MAX_BLOB_BYTES;

/// Directory listing.
#[derive(Clone, Debug, Serialize)]
pub struct Tree {
    /// `owner/name`.
    pub repo: String,
    /// Resolved revision.
    #[serde(rename = "ref")]
    pub git_ref: String,
    /// Path inside the tree (empty = root).
    pub path: String,
    /// Entries in this directory.
    pub entries: Vec<TreeEntry>,
}

/// One `ls-tree` row.
#[derive(Clone, Debug, Serialize)]
pub struct TreeEntry {
    /// Basename.
    pub name: String,
    /// `blob`, `tree`, `commit`, or `tag`.
    pub kind: String,
    /// Git mode (`100644`, `040000`, …).
    pub mode: String,
    /// Blob size in bytes, if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    /// Object name.
    pub sha: String,
}

/// File contents at a revision.
#[derive(Clone, Debug, Serialize)]
pub struct Blob {
    /// `owner/name`.
    pub repo: String,
    /// Resolved revision.
    #[serde(rename = "ref")]
    pub git_ref: String,
    /// Path inside the tree.
    pub path: String,
    /// Blob object name.
    pub sha: String,
    /// Byte length.
    pub size: u64,
    /// True when the blob has NUL bytes.
    pub binary: bool,
    /// True when contents were omitted for size.
    pub truncated: bool,
    /// UTF-8 text (lossy) when not binary/truncated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

/// `git blame` for one file.
#[derive(Clone, Debug, Serialize)]
pub struct Blame {
    /// `owner/name`.
    pub repo: String,
    /// Resolved revision.
    #[serde(rename = "ref")]
    pub git_ref: String,
    /// Path inside the tree.
    pub path: String,
    /// One row per source line.
    pub lines: Vec<BlameLine>,
}

/// One blamed line.
#[derive(Clone, Debug, Serialize)]
pub struct BlameLine {
    /// 1-based line number in the final file.
    pub line: u32,
    /// Commit that last touched this line.
    pub sha: String,
    /// Author name.
    pub author: String,
    /// Author email.
    pub email: String,
    /// Unix author time.
    pub author_time: i64,
    /// Line text without the trailing newline.
    pub text: String,
}

/// Commit history.
#[derive(Clone, Debug, Serialize)]
pub struct Log {
    /// `owner/name`.
    pub repo: String,
    /// Resolved revision.
    #[serde(rename = "ref")]
    pub git_ref: String,
    /// Optional path filter.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Commits, newest first.
    pub commits: Vec<CommitInfo>,
}

/// One commit.
#[derive(Clone, Debug, Serialize)]
pub struct CommitInfo {
    /// Full object name.
    pub sha: String,
    /// Abbreviated object name.
    pub short: String,
    /// Author name.
    pub author: String,
    /// Author email.
    pub email: String,
    /// Author date (ISO-8601).
    pub date: String,
    /// First line of the message.
    pub subject: String,
    /// Rest of the message (may be empty).
    #[serde(default)]
    pub body: String,
}

/// Branches and tags.
#[derive(Clone, Debug, Serialize)]
pub struct Refs {
    /// `owner/name`.
    pub repo: String,
    /// Default branch (no `refs/heads/`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_branch: Option<String>,
    /// `refs/heads/*`.
    pub branches: Vec<RefItem>,
    /// `refs/tags/*`.
    pub tags: Vec<RefItem>,
}

/// One named ref.
#[derive(Clone, Debug, Serialize)]
pub struct RefItem {
    /// Short name (`master`, `v1.0.0`).
    pub name: String,
    /// Object name.
    pub sha: String,
}

/// Unified diff plus commits between two tips.
#[derive(Clone, Debug, Serialize)]
pub struct Diff {
    /// `owner/name`.
    pub repo: String,
    /// Base object name.
    pub base: String,
    /// Head object name.
    pub head: String,
    /// Unified diff (`git diff --no-color`).
    pub diff: String,
    /// Commits reachable from head but not base.
    pub commits: Vec<CommitInfo>,
}

async fn readable_path(
    store: &Store,
    actor: &Actor,
    name: &RepoName,
) -> Result<std::path::PathBuf> {
    acl::require(store, actor, name, Role::Read)?;
    let path = store.repo_path(name);
    if !path.exists() {
        bail!("repository {name} not found");
    }
    Ok(path)
}

/// `git ls-tree` for a directory.
pub async fn tree(
    store: &Store,
    actor: &Actor,
    name: &RepoName,
    git_ref: &str,
    path: &str,
) -> Result<Tree> {
    let repo = readable_path(store, actor, name).await?;
    let resolved = git::rev_parse(&repo, git_ref).await?;
    let path = path.trim_matches('/').to_string();
    let spec = if path.is_empty() {
        resolved.clone()
    } else {
        format!("{resolved}:{path}")
    };
    let kind = git_stdout_opt(&repo, &["cat-file", "-t", &spec]).await?;
    match kind.as_deref() {
        Some("tree") => {}
        Some("commit") if path.is_empty() => {}
        Some("blob") => bail!("{path} is a file; use repo blob"),
        Some(other) => bail!("cannot list {path} ({other})"),
        None => bail!("path {path} not found at {git_ref}"),
    }
    let raw = git::git_stdout(&repo, &["ls-tree", "-l", &spec]).await?;
    let mut entries = Vec::new();
    for line in raw.lines() {
        if let Some(entry) = parse_ls_tree(line) {
            entries.push(entry);
        }
    }
    Ok(Tree {
        repo: name.to_string(),
        git_ref: resolved,
        path,
        entries,
    })
}

fn parse_ls_tree(line: &str) -> Option<TreeEntry> {
    let (meta, name) = line.split_once('\t')?;
    let mut parts = meta.split_whitespace();
    let mode = parts.next()?.to_string();
    let kind = parts.next()?.to_string();
    let sha = parts.next()?.to_string();
    let size_raw = parts.next().unwrap_or("-");
    let size = size_raw.parse().ok();
    let name = name.rsplit('/').next().unwrap_or(name).to_string();
    Some(TreeEntry {
        name,
        kind,
        mode,
        size,
        sha,
    })
}

/// File contents (text, size-capped).
pub async fn blob(
    store: &Store,
    actor: &Actor,
    name: &RepoName,
    git_ref: &str,
    path: &str,
) -> Result<Blob> {
    let repo = readable_path(store, actor, name).await?;
    let path = path.trim_matches('/').to_string();
    if path.is_empty() {
        bail!("blob needs --path");
    }
    let resolved = git::rev_parse(&repo, git_ref).await?;
    let spec = format!("{resolved}:{path}");
    let kind = git_stdout_opt(&repo, &["cat-file", "-t", &spec])
        .await?
        .with_context(|| format!("path {path} not found at {git_ref}"))?;
    if kind != "blob" {
        bail!("{path} is a {kind}; use repo tree");
    }
    let sha = git::git_stdout(&repo, &["rev-parse", &spec]).await?;
    let bytes = git::cat_file_bytes(&repo, &spec).await?;
    let size = bytes.len() as u64;
    let binary = bytes.contains(&0);
    let truncated = size > MAX_BLOB_BYTES;
    let content = if binary || truncated {
        None
    } else {
        Some(String::from_utf8_lossy(&bytes).into_owned())
    };
    Ok(Blob {
        repo: name.to_string(),
        git_ref: resolved,
        path,
        sha,
        size,
        binary,
        truncated,
        content,
    })
}

/// `git blame --line-porcelain`.
pub async fn blame(
    store: &Store,
    actor: &Actor,
    name: &RepoName,
    git_ref: &str,
    path: &str,
) -> Result<Blame> {
    let repo = readable_path(store, actor, name).await?;
    let path = path.trim_matches('/').to_string();
    if path.is_empty() {
        bail!("blame needs --path");
    }
    let resolved = git::rev_parse(&repo, git_ref).await?;
    let raw = git::git_stdout(
        &repo,
        &["blame", "--line-porcelain", &resolved, "--", &path],
    )
    .await?;
    Ok(Blame {
        repo: name.to_string(),
        git_ref: resolved,
        path,
        lines: parse_blame(&raw),
    })
}

fn parse_blame(raw: &str) -> Vec<BlameLine> {
    let mut lines = Vec::new();
    let mut sha = String::new();
    let mut author = String::new();
    let mut email = String::new();
    let mut author_time: i64 = 0;
    let mut final_line: u32 = 0;
    for line in raw.lines() {
        if let Some(text) = line.strip_prefix('\t') {
            lines.push(BlameLine {
                line: final_line,
                sha: sha.clone(),
                author: author.clone(),
                email: email.clone(),
                author_time,
                text: text.to_string(),
            });
            continue;
        }
        if let Some(rest) = line.strip_prefix("author ") {
            author = rest.to_string();
        } else if let Some(rest) = line.strip_prefix("author-mail ") {
            email = rest.trim_matches(|c| c == '<' || c == '>').to_string();
        } else if let Some(rest) = line.strip_prefix("author-time ") {
            author_time = rest.parse().unwrap_or(0);
        } else if looks_like_blame_header(line) {
            let mut parts = line.split_whitespace();
            sha = parts.next().unwrap_or("").to_string();
            let _orig = parts.next();
            final_line = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        }
    }
    lines
}

fn looks_like_blame_header(line: &str) -> bool {
    let mut parts = line.split_whitespace();
    let Some(sha) = parts.next() else {
        return false;
    };
    sha.len() >= 40 && sha.chars().all(|c| c.is_ascii_hexdigit()) && parts.next().is_some()
}

/// `git log` (newest first).
pub async fn log(
    store: &Store,
    actor: &Actor,
    name: &RepoName,
    git_ref: &str,
    path: Option<&str>,
    limit: usize,
) -> Result<Log> {
    let repo = readable_path(store, actor, name).await?;
    let resolved = git::rev_parse(&repo, git_ref).await?;
    let limit_s = limit.max(1).to_string();
    let mut args = vec![
        "log".into(),
        format!("-{limit_s}"),
        "--format=%H%x1f%h%x1f%an%x1f%ae%x1f%aI%x1f%s%x1f%b%x1e".into(),
        resolved.clone(),
    ];
    let path = path
        .map(|p| p.trim_matches('/').to_string())
        .filter(|p| !p.is_empty());
    if let Some(path) = &path {
        args.push("--".into());
        args.push(path.clone());
    }
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let raw = git::git_stdout(&repo, &arg_refs).await.unwrap_or_default();
    Ok(Log {
        repo: name.to_string(),
        git_ref: resolved,
        path,
        commits: parse_log(&raw),
    })
}

fn parse_log(raw: &str) -> Vec<CommitInfo> {
    raw.split('\u{1e}')
        .filter(|chunk| !chunk.trim().is_empty())
        .filter_map(|chunk| {
            let chunk = chunk.trim_start_matches('\n');
            let mut parts = chunk.split('\u{1f}');
            Some(CommitInfo {
                sha: parts.next()?.to_string(),
                short: parts.next()?.to_string(),
                author: parts.next()?.to_string(),
                email: parts.next()?.to_string(),
                date: parts.next()?.to_string(),
                subject: parts.next()?.to_string(),
                body: parts.next().unwrap_or("").trim().to_string(),
            })
        })
        .collect()
}

/// One commit, including the full message.
pub async fn commit(
    store: &Store,
    actor: &Actor,
    name: &RepoName,
    sha: &str,
) -> Result<CommitInfo> {
    let log = log(store, actor, name, sha, None, 1).await?;
    log.commits
        .into_iter()
        .next()
        .with_context(|| format!("commit {sha} not found"))
}

/// Branches and tags.
pub async fn refs(store: &Store, actor: &Actor, name: &RepoName) -> Result<Refs> {
    let repo = readable_path(store, actor, name).await?;
    let map = git::list_refs(&repo).await?;
    let mut branches = Vec::new();
    let mut tags = Vec::new();
    for (refname, sha) in &map {
        if let Some(branch) = refname.strip_prefix("refs/heads/") {
            branches.push(RefItem {
                name: branch.to_string(),
                sha: sha.clone(),
            });
        } else if let Some(tag) = refname.strip_prefix("refs/tags/") {
            tags.push(RefItem {
                name: tag.to_string(),
                sha: sha.clone(),
            });
        }
    }
    let default_branch = git::default_branch(&repo).await.ok();
    Ok(Refs {
        repo: name.to_string(),
        default_branch,
        branches,
        tags,
    })
}

/// Unified diff `base..head` plus commits on head.
pub async fn diff(
    store: &Store,
    actor: &Actor,
    name: &RepoName,
    base: &str,
    head: &str,
) -> Result<Diff> {
    let repo = readable_path(store, actor, name).await?;
    let base_sha = git::rev_parse(&repo, base).await?;
    let head_sha = git::rev_parse(&repo, head).await?;
    let diff = git::git_stdout(
        &repo,
        &["diff", "--no-color", &format!("{base_sha}..{head_sha}")],
    )
    .await
    .unwrap_or_default();
    let range = format!("{base_sha}..{head_sha}");
    let raw = git::git_stdout(
        &repo,
        &[
            "log",
            "--format=%H%x1f%h%x1f%an%x1f%ae%x1f%aI%x1f%s%x1f%b%x1e",
            &range,
        ],
    )
    .await
    .unwrap_or_default();
    Ok(Diff {
        repo: name.to_string(),
        base: base_sha,
        head: head_sha,
        diff,
        commits: parse_log(&raw),
    })
}

async fn git_stdout_opt(repo: &std::path::Path, args: &[&str]) -> Result<Option<String>> {
    match git::git_stdout(repo, args).await {
        Ok(s) if s.is_empty() => Ok(None),
        Ok(s) => Ok(Some(s)),
        Err(_) => Ok(None),
    }
}

/// Format a browse result for the CLI.
pub fn emit<T: Serialize>(json: bool, value: &T, text: String) -> Result<String> {
    output::pick(json, value, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ls_tree_blob() {
        let entry =
            parse_ls_tree("100644 blob abcdef0123456789abcdef0123456789abcdef01 12\tREADME.md")
                .unwrap();
        assert_eq!(entry.name, "README.md");
        assert_eq!(entry.kind, "blob");
        assert_eq!(entry.size, Some(12));
    }

    #[test]
    fn parse_blame_sample() {
        let raw = "\
aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa 1 1 1
author Ada
author-mail <ada@example.com>
author-time 1700000000
author-tz +0000
filename README.md
\thello
";
        let lines = parse_blame(raw);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "hello");
        assert_eq!(lines[0].author, "Ada");
        assert_eq!(lines[0].line, 1);
    }
}
