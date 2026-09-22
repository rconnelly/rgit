//! Spawn system `git`. No git2/gix.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Stdio;

use anyhow::{bail, Context, Result};
use tokio::process::Command;

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
    let output = Command::new("git")
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
    let output = Command::new("git")
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
pub async fn default_branch(repo: &Path) -> Result<String> {
    match git_stdout(repo, &["symbolic-ref", "--short", "HEAD"]).await {
        Ok(name) if !name.is_empty() => Ok(name),
        _ => {
            let refs = list_refs(repo).await?;
            if refs.contains_key("refs/heads/master") {
                Ok("master".into())
            } else if refs.contains_key("refs/heads/main") {
                Ok("main".into())
            } else {
                bail!("repository has no default branch yet");
            }
        }
    }
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
    let mut child = Command::new("git")
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

/// Show a blob at `sha:path` if it exists.
pub async fn show_path(repo: &Path, sha: &str, path: &str) -> Result<Option<String>> {
    let spec = format!("{sha}:{path}");
    let output = git_output_raw(repo, &["cat-file", "-p", &spec]).await?;
    if !output.status.success() {
        return Ok(None);
    }
    Ok(Some(String::from_utf8_lossy(&output.stdout).into_owned()))
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
    Command::new("git")
        .current_dir(repo)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("spawn git")
}
