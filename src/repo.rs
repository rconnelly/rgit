//! Bare repositories and receive-pack hooks.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Serialize;

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
        Actor::Anonymous => bail!("sign in to create a repository"),
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

/// Repos where `user` has a role in `access.yaml`.
pub fn list_for_user(store: &Store, actor: &Actor, user: &str) -> Result<Vec<RepoName>> {
    if !actor.is_forge_admin(store)? && actor.name() != Some(user) {
        bail!("only forge admins can list other users' repositories");
    }
    let access = store.load_access()?;
    let mut out = Vec::new();
    for repo in store.list_repos()? {
        if access.role(&repo, user).is_some() {
            out.push(repo);
        }
    }
    Ok(out)
}

/// One repository as shown to the web UI / `--json`.
#[derive(Clone, Debug, Serialize)]
pub struct RepoInfo {
    /// `owner/name`.
    pub name: String,
    /// Bare repo path.
    pub path: String,
    /// `public` or `private`.
    pub visibility: String,
    /// Default branch, if the repo has commits.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_branch: Option<String>,
    /// SSH clone URL.
    pub clone_url: String,
    /// First line of git `description`, if set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Effective role for the current actor (`read` / `write` / `admin`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// ACL entries.
    pub access: Vec<AccessEntry>,
}

/// One ACL row.
#[derive(Clone, Debug, Serialize)]
pub struct AccessEntry {
    /// Forge login.
    pub user: String,
    /// `read`, `write`, or `admin`.
    pub role: String,
}

/// SSH clone URL for `name`.
pub fn clone_url(name: &RepoName) -> String {
    let bind = std::env::var("RABUN_GIT_SSH_BIND").unwrap_or_else(|_| "0.0.0.0:2222".into());
    let (host, port) = match bind.rsplit_once(':') {
        Some((h, p)) => {
            let host = std::env::var("RABUN_GIT_PUBLIC_HOST")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| {
                    if h == "0.0.0.0" || h == "::" || h.is_empty() {
                        "localhost".into()
                    } else {
                        h.trim_matches(|c| c == '[' || c == ']').to_string()
                    }
                });
            (host, p.to_string())
        }
        None => (
            std::env::var("RABUN_GIT_PUBLIC_HOST").unwrap_or_else(|_| "localhost".into()),
            "2222".into(),
        ),
    };
    format!("ssh://git@{host}:{port}/{name}.git")
}

/// Show one repo (must be readable).
pub fn show(store: &Store, actor: &Actor, name: &RepoName) -> Result<String> {
    Ok(format_info(&info_sync(store, actor, name)?))
}

/// Structured repo metadata (async default branch).
pub async fn info(store: &Store, actor: &Actor, name: &RepoName) -> Result<RepoInfo> {
    let mut info = info_sync(store, actor, name)?;
    let path = store.repo_path(name);
    info.default_branch = git::default_branch(&path).await.ok();
    Ok(info)
}

fn info_sync(store: &Store, actor: &Actor, name: &RepoName) -> Result<RepoInfo> {
    acl::require(store, actor, name, Role::Read)?;
    let path = store.repo_path(name);
    if !path.exists() {
        bail!("repository {name} not found");
    }
    let access = store.load_access()?;
    let mut entries = Vec::new();
    if let Some(roles) = access.repos.get(&name.to_string()) {
        for (user, role) in roles {
            entries.push(AccessEntry {
                user: user.clone(),
                role: role.to_string(),
            });
        }
    }
    let description = std::fs::read_to_string(path.join("description"))
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty() && !text.starts_with("Unnamed repository"));
    let visibility = if store.is_public(name)? {
        "public"
    } else {
        "private"
    };
    let role = acl::role(store, actor, name)?.map(|r| r.to_string());
    Ok(RepoInfo {
        name: name.to_string(),
        path: path.display().to_string(),
        visibility: visibility.into(),
        default_branch: None,
        clone_url: clone_url(name),
        description,
        role,
        access: entries,
    })
}

fn format_info(info: &RepoInfo) -> String {
    let mut lines = vec![
        format!("repository: {}", info.name),
        format!("path: {}", info.path),
        format!("visibility: {}", info.visibility),
        format!("clone: {}", info.clone_url),
    ];
    if let Some(branch) = &info.default_branch {
        lines.push(format!("default_branch: {branch}"));
    }
    if let Some(description) = &info.description {
        lines.push(format!("description: {description}"));
    }
    for entry in &info.access {
        lines.push(format!("access: {} {}", entry.user, entry.role));
    }
    lines.join("\n") + "\n"
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

    #[test]
    fn list_for_user_acl() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path());
        store.ensure_layout().unwrap();
        store.add_user("alice", true).unwrap();
        store.add_user("bob", false).unwrap();
        let alice_app = RepoName::parse("alice/app").unwrap();
        let bob_notes = RepoName::parse("bob/notes").unwrap();
        std::fs::create_dir_all(store.repo_path(&alice_app)).unwrap();
        std::fs::create_dir_all(store.repo_path(&bob_notes)).unwrap();
        store.grant("alice", &alice_app, Role::Admin).unwrap();
        store.grant("bob", &alice_app, Role::Read).unwrap();
        store.grant("bob", &bob_notes, Role::Admin).unwrap();

        let as_alice = Actor::User("alice".into());
        let as_bob = Actor::User("bob".into());
        let bob_repos = list_for_user(&store, &as_alice, "bob").unwrap();
        assert_eq!(
            bob_repos
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            vec!["alice/app", "bob/notes"]
        );
        let self_repos = list_for_user(&store, &as_bob, "bob").unwrap();
        assert_eq!(self_repos.len(), 2);
        let err = list_for_user(&store, &as_bob, "alice").unwrap_err();
        assert!(err.to_string().contains("only forge admins"));
    }
}
