//! Roles and path ACL (`access.yaml` plus forge admins in `users.yaml`).

use std::fmt;
use std::str::FromStr;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use crate::names::RepoName;
use crate::store::Store;

/// Who is performing an operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Actor {
    /// Local CLI on the forge host (full access).
    Operator,
    /// Authenticated SSH user or web token.
    User(String),
    /// Unauthenticated web visitor (public repos only).
    Anonymous,
}

impl Actor {
    /// SSH or local user name, if any.
    pub fn name(&self) -> Option<&str> {
        match self {
            Actor::Operator | Actor::Anonymous => None,
            Actor::User(name) => Some(name.as_str()),
        }
    }

    /// Display name for request author fields.
    pub fn author(&self) -> &str {
        match self {
            Actor::Operator => "operator",
            Actor::Anonymous => "anonymous",
            Actor::User(name) => name.as_str(),
        }
    }

    /// True when this actor is a forge-wide admin (or the local operator).
    pub fn is_forge_admin(&self, store: &Store) -> Result<bool> {
        match self {
            Actor::Operator => Ok(true),
            Actor::Anonymous => Ok(false),
            Actor::User(name) => Ok(store.load_users()?.is_admin(name)),
        }
    }
}

/// Access level on a single repository.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// Clone, fetch, list requests and runs.
    Read,
    /// Push non-protected branches, open/review requests.
    Write,
    /// Push protected branches, merge, grant access on this repo.
    Admin,
}

impl Role {
    /// Whether this role includes `needed`.
    pub fn implies(self, needed: Role) -> bool {
        self >= needed
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Role::Read => write!(f, "read"),
            Role::Write => write!(f, "write"),
            Role::Admin => write!(f, "admin"),
        }
    }
}

impl FromStr for Role {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "read" => Ok(Role::Read),
            "write" => Ok(Role::Write),
            "admin" => Ok(Role::Admin),
            other => bail!("role must be read, write, or admin (got {other:?})"),
        }
    }
}

/// Effective role for `actor` on `repo` (forge admin counts as repo admin).
pub fn role(store: &Store, actor: &Actor, repo: &RepoName) -> Result<Option<Role>> {
    if actor.is_forge_admin(store)? {
        return Ok(Some(Role::Admin));
    }
    let granted = match actor {
        Actor::User(name) => store.load_access()?.role(repo, name),
        Actor::Operator => return Ok(Some(Role::Admin)),
        Actor::Anonymous => None,
    };
    if granted.is_some() {
        return Ok(granted);
    }
    if store.is_public(repo)? {
        return Ok(Some(Role::Read));
    }
    Ok(None)
}

/// Require `needed` on `repo`.
pub fn require(store: &Store, actor: &Actor, repo: &RepoName, needed: Role) -> Result<Role> {
    match role(store, actor, repo)? {
        Some(have) if have.implies(needed) => Ok(have),
        Some(have) => bail!(
            "{actor_label} has {have} on {repo}, need {needed}",
            actor_label = actor.author()
        ),
        None => bail!("{} has no access to {repo}", actor.author()),
    }
}

/// Require forge admin.
pub fn require_forge_admin(store: &Store, actor: &Actor) -> Result<()> {
    if actor.is_forge_admin(store)? {
        Ok(())
    } else {
        bail!("{} is not a forge admin", actor.author())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_order() {
        assert!(Role::Admin.implies(Role::Write));
        assert!(Role::Write.implies(Role::Read));
        assert!(!Role::Read.implies(Role::Write));
    }
}
