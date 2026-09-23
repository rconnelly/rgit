//! Git `hooks/update` and `hooks/commit-msg`.

use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::acl::{self, Actor, Role};
use crate::names::{self, RepoName};
use crate::release;
use crate::store::Store;

/// Called as `rabun-git hook update <ref> <old> <new>`.
///
/// Reads `RABUN_GIT_USER` and `RABUN_GIT_REPO` from the environment (set by
/// `git-receive-pack` in [`crate::ssh`]).
pub async fn update(refname: &str, old: &str, new: &str) -> Result<()> {
    let user = std::env::var("RABUN_GIT_USER").unwrap_or_default();
    let repo = std::env::var("RABUN_GIT_REPO").unwrap_or_default();
    let root = std::env::var("RABUN_GIT_ROOT").unwrap_or_default();
    if user.is_empty() || repo.is_empty() || root.is_empty() {
        // Direct git without the forge wrapper: allow (local tests, operator).
        return Ok(());
    }
    let store = Store::open(root);
    let repo = RepoName::parse(&repo)?;
    let actor = Actor::User(user);
    let zero = new.chars().all(|c| c == '0');
    if names::request_new_ref(refname).is_some() {
        acl::require(&store, &actor, &repo, Role::Write)?;
        return Ok(());
    }
    if refname.starts_with("refs/rabun/requests/") {
        // Allocated request refs are server-managed.
        bail!("refusing update of {refname}; open a request with refs/rabun/requests/new/<branch>");
    }
    if names::is_protected_ref(refname) {
        acl::require(&store, &actor, &repo, Role::Admin)?;
        if zero {
            bail!("refusing to delete protected branch {refname}");
        }
    } else {
        acl::require(&store, &actor, &repo, Role::Write)?;
    }
    let path = store.repo_path(&repo);
    release::enforce_push(&path, refname, old, new).await
}

/// Called as `rabun-git hook commit-msg <path>` from a local `commit-msg` hook.
pub fn commit_msg(path: &Path) -> Result<()> {
    let text = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    crate::release::commit::validate(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn allows_when_env_missing() {
        std::env::remove_var("RABUN_GIT_USER");
        std::env::remove_var("RABUN_GIT_REPO");
        std::env::remove_var("RABUN_GIT_ROOT");
        update("refs/heads/master", "0", "abc").await.unwrap();
    }

    #[test]
    fn commit_msg_rejects_non_conventional() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), "not conventional\n").unwrap();
        assert!(commit_msg(tmp.path()).is_err());
        std::fs::write(tmp.path(), "feat: ok\n").unwrap();
        commit_msg(tmp.path()).unwrap();
        std::fs::write(tmp.path(), "Merge branch 'x'\n").unwrap();
        commit_msg(tmp.path()).unwrap();
    }
}
