//! Git `hooks/update` — protected branches and request-new refs.

use anyhow::{bail, Result};

use crate::acl::{self, Actor, Role};
use crate::names::{self, RepoName};
use crate::store::Store;

/// Called as `rabun-git hook update <ref> <old> <new>`.
///
/// Reads `RABUN_GIT_USER` and `RABUN_GIT_REPO` from the environment (set by
/// `git-receive-pack` in [`crate::ssh`]).
pub fn update(refname: &str, _old: &str, new: &str) -> Result<()> {
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
        return Ok(());
    }
    if refname.starts_with("refs/heads/") || refname.starts_with("refs/tags/") {
        acl::require(&store, &actor, &repo, Role::Write)?;
        return Ok(());
    }
    acl::require(&store, &actor, &repo, Role::Write)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_when_env_missing() {
        std::env::remove_var("RABUN_GIT_USER");
        std::env::remove_var("RABUN_GIT_REPO");
        std::env::remove_var("RABUN_GIT_ROOT");
        update("refs/heads/master", "0", "abc").unwrap();
    }
}
