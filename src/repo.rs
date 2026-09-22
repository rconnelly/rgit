//! Bare repositories and receive-pack hooks.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::acl::{self, Actor, Role};
use crate::git;
use crate::names::RepoName;
use crate::store::Store;

const UPDATE_HOOK: &str = r#"#!/bin/sh
set -e
if [ -z "$RABUN_GIT_BIN" ]; then
  exit 0
fi
exec "$RABUN_GIT_BIN" hook update "$1" "$2" "$3"
"#;

/// Create a bare repo; SSH users may only create under their own owner.
pub async fn create(store: &Store, actor: &Actor, name: &RepoName) -> Result<()> {
    match actor {
        Actor::Operator => {}
        Actor::User(user) => {
            if actor.is_forge_admin(store)? {
                // ok
            } else if name.owner != *user {
                bail!("users may only create repos under their own name ({user}/...)");
            }
        }
    }
    let path = store.repo_path(name);
    if path.exists() {
        bail!("repository {name} already exists");
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    git::git_global(&["init", "--bare", &path.to_string_lossy()]).await?;
    install_hooks(&path)?;
    if let Some(user) = actor.name() {
        store.grant(user, name, Role::Admin)?;
    }
    Ok(())
}

/// Repos the actor can read.
pub fn list(store: &Store, actor: &Actor) -> Result<Vec<RepoName>> {
    let mut out = Vec::new();
    for repo in store.list_repos()? {
        if acl::role(store, actor, &repo)?.is_some() {
            out.push(repo);
        }
    }
    Ok(out)
}

/// Show one repo (must be readable).
pub fn show(store: &Store, actor: &Actor, name: &RepoName) -> Result<String> {
    acl::require(store, actor, name, Role::Read)?;
    let path = store.repo_path(name);
    if !path.exists() {
        bail!("repository {name} not found");
    }
    let access = store.load_access()?;
    let mut lines = vec![
        format!("repository: {name}"),
        format!("path: {}", path.display()),
    ];
    if let Some(roles) = access.repos.get(&name.to_string()) {
        for (user, role) in roles {
            lines.push(format!("access: {user} {role}"));
        }
    }
    Ok(lines.join("\n") + "\n")
}

/// Install the update hook that defers to `rabun-git hook update`.
pub fn install_hooks(repo: &Path) -> Result<()> {
    let hook = repo.join("hooks").join("update");
    if let Some(parent) = hook.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::write(&hook, UPDATE_HOOK).with_context(|| format!("write {}", hook.display()))?;
    let mut perms = std::fs::metadata(&hook)?.permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&hook, perms).with_context(|| format!("chmod {}", hook.display()))?;
    Ok(())
}

/// Path to the current binary for hook `RABUN_GIT_BIN`.
pub fn current_bin() -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("rabun-git"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    #[tokio::test]
    async fn create_lists_repo() {
        if !git::git_on_path() {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path());
        store.ensure_layout().unwrap();
        store.add_user("alice", true).unwrap();
        let name = RepoName::parse("alice/app").unwrap();
        create(&store, &Actor::User("alice".into()), &name)
            .await
            .unwrap();
        let listed = list(&store, &Actor::User("alice".into())).unwrap();
        assert_eq!(listed.len(), 1);
        assert!(store.repo_path(&name).join("HEAD").exists());
        assert!(store.repo_path(&name).join("hooks/update").exists());
    }
}
