//! Spawn system `git`. No git2/gix.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{bail, Context, Result};
use tokio::process::Command;

/// `git -c safe.directory=* …` so group-shared forge repos are readable.
/// Git 2.35+ refuses a repo whose directory uid ≠ the process uid (CVE-2022-24765).
/// Serve runs as `rabun-git`; web create/browse runs as `rgit-web`.
pub fn command() -> Command {
    let mut cmd = Command::new("git");
    cmd.args(["-c", "safe.directory=*"]);
    cmd
}

/// Run `git -C repo args...` and fail on non-zero exit.
pub async fn git(repo: &Path, args: &[&str]) -> Result<()> {
    let output = git_output_raw(repo, args).await?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

/// Run `git -C repo args...` and return stdout (trimmed) on success.
pub async fn git_stdout(repo: &Path, args: &[&str]) -> Result<String> {
    let output = git_output_raw(repo, args).await?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// `git` without `-C` (for `init --bare` on a new path).
pub async fn git_global(args: &[&str]) -> Result<()> {
    let output = command()
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("spawn git")?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

/// Whether `git` is on PATH.
pub fn git_on_path() -> bool {
    std::process::Command::new("git")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// All refs in a (usually bare) repo: name → object id.
pub async fn list_refs(repo: &Path) -> Result<BTreeMap<String, String>> {
    let output = command()
        .current_dir(repo)
        .args(["for-each-ref", "--format=%(refname)%09%(objectname)"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("spawn git for-each-ref")?;
    if !output.status.success() {
        bail!(
            "git for-each-ref failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let mut map = BTreeMap::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let Some((name, sha)) = line.split_once('\t') else {
            continue;
        };
        if !name.is_empty() && !sha.is_empty() {
            map.insert(name.to_string(), sha.to_string());
        }
    }
    Ok(map)
}

/// Default branch name (`master` / `main`) without `refs/heads/`.
/// Unborn HEAD (new bare repo, no commits) is an error, not `master`.
pub async fn default_branch(repo: &Path) -> Result<String> {
    let candidate = match git_stdout(repo, &["symbolic-ref", "--short", "HEAD"]).await {
        Ok(name) if !name.is_empty() => name,
        _ => {
            let refs = list_refs(repo).await?;
            if refs.contains_key("refs/heads/master") {
                "master".into()
            } else if refs.contains_key("refs/heads/main") {
                "main".into()
            } else {
                bail!("empty repository");
            }
        }
    };
    if rev_parse(repo, &candidate).await.is_err() {
        bail!("empty repository");
    }
    Ok(candidate)
}

/// True when HEAD does not resolve (no commits yet).
pub async fn is_unborn(repo: &Path) -> bool {
    rev_parse(repo, "HEAD").await.is_err()
}

/// Resolve a branch or SHA to a 40-character object name.
pub async fn rev_parse(repo: &Path, spec: &str) -> Result<String> {
    git_stdout(repo, &["rev-parse", "--verify", spec]).await
}

/// True when `ancestor` is an ancestor of `desc`.
pub async fn is_ancestor(repo: &Path, ancestor: &str, desc: &str) -> Result<bool> {
    let output = git_output_raw(repo, &["merge-base", "--is-ancestor", ancestor, desc]).await?;
    Ok(output.status.success())
}

/// Write a blob and return its object id.
pub async fn hash_blob(repo: &Path, bytes: &[u8]) -> Result<String> {
    let mut child = command()
        .current_dir(repo)
        .args(["hash-object", "-w", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("spawn git hash-object")?;
    {
        use tokio::io::AsyncWriteExt;
        let stdin = child.stdin.as_mut().context("hash-object stdin")?;
        stdin.write_all(bytes).await.context("write blob")?;
    }
    let output = child.wait_with_output().await.context("wait hash-object")?;
    if !output.status.success() {
        bail!(
            "git hash-object failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// `git cat-file -p <spec>`.
pub async fn cat_file(repo: &Path, spec: &str) -> Result<String> {
    git_stdout(repo, &["cat-file", "-p", spec]).await
}

/// `git cat-file -p <spec>` as bytes (blobs may be binary).
pub async fn cat_file_bytes(repo: &Path, spec: &str) -> Result<Vec<u8>> {
    let output = git_output_raw(repo, &["cat-file", "-p", spec]).await?;
    if !output.status.success() {
        bail!(
            "git cat-file failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(output.stdout)
}

/// Show a blob at `sha:path` if it exists.
pub async fn show_path(repo: &Path, sha: &str, path: &str) -> Result<Option<String>> {
    let spec = format!("{sha}:{path}");
    let output = git_output_raw(repo, &["cat-file", "-p", &spec]).await?;
    if !output.status.success() {
        return Ok(None);
    }
    Ok(Some(String::from_utf8_lossy(&output.stdout).into_owned()))
}

/// True when `oid` is a git zero SHA (new or deleted ref).
pub fn is_zero_oid(oid: &str) -> bool {
    !oid.is_empty() && oid.chars().all(|c| c == '0')
}

/// Work tree root (`rev-parse --show-toplevel`) for a path inside a repo.
pub async fn work_tree_root(dir: &Path) -> Result<PathBuf> {
    git_stdout(dir, &["rev-parse", "--show-toplevel"])
        .await
        .map(PathBuf::from)
}

/// Path inside `.git` (`rev-parse --git-path`), made absolute using `dir`.
pub async fn git_path(dir: &Path, spec: &str) -> Result<PathBuf> {
    let raw = git_stdout(dir, &["rev-parse", "--git-path", spec]).await?;
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(dir.join(path))
    }
}

/// Raw commit message (`git log -1 --format=%B`).
pub async fn commit_message(repo: &Path, sha: &str) -> Result<String> {
    git_stdout(repo, &["log", "-1", "--format=%B", sha]).await
}

/// Peel a tag or commit-ish to a commit object name.
pub async fn peel_commit(repo: &Path, spec: &str) -> Result<String> {
    let peeled = format!("{spec}^{{commit}}");
    git_stdout(repo, &["rev-parse", "--verify", &peeled]).await
}

/// `git rev-list` object names (one per line).
pub async fn rev_list(repo: &Path, extra: &[&str]) -> Result<Vec<String>> {
    let mut args = Vec::with_capacity(extra.len() + 1);
    args.push("rev-list");
    args.extend_from_slice(extra);
    let stdout = git_stdout(repo, &args).await?;
    Ok(stdout
        .lines()
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
}

/// Commits introduced by updating `old` → `new` (no merges, oldest first).
pub async fn new_commits(repo: &Path, old: &str, new: &str) -> Result<Vec<String>> {
    if is_zero_oid(new) {
        return Ok(Vec::new());
    }
    if is_zero_oid(old) {
        rev_list(repo, &["--no-merges", "--reverse", new, "--not", "--all"]).await
    } else {
        let range = format!("{old}..{new}");
        rev_list(repo, &["--no-merges", "--reverse", &range]).await
    }
}

/// Nearest ancestor tag matching `prefix*` (`v*`), if any.
pub async fn nearest_version_tag(repo: &Path, prefix: &str) -> Result<Option<String>> {
    let pattern = format!("{prefix}*");
    let output = git_output_raw(
        repo,
        &["describe", "--tags", "--abbrev=0", "--match", &pattern],
    )
    .await?;
    if !output.status.success() {
        return Ok(None);
    }
    let tag = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if tag.is_empty() {
        Ok(None)
    } else {
        Ok(Some(tag))
    }
}

/// `git remote get-url`, or `None` when the remote is missing.
pub async fn remote_url(repo: &Path, name: &str) -> Result<Option<String>> {
    let output = git_output_raw(repo, &["remote", "get-url", name]).await?;
    if !output.status.success() {
        return Ok(None);
    }
    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if url.is_empty() {
        Ok(None)
    } else {
        Ok(Some(url))
    }
}

/// File names under `prefix` at `sha`.
pub async fn ls_tree_prefix(repo: &Path, sha: &str, prefix: &str) -> Result<Vec<String>> {
    let output = git_output_raw(repo, &["ls-tree", "--name-only", "-r", sha, "--", prefix]).await?;
    if !output.status.success() {
        return Ok(Vec::new());
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .collect())
}

async fn git_output_raw(repo: &Path, args: &[&str]) -> Result<std::process::Output> {
    command()
        .current_dir(repo)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("spawn git")
}
