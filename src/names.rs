//! `owner/name` repository identifiers.

use std::fmt;
use std::str::FromStr;

use anyhow::{bail, Result};

/// Validated `owner/name` (alphanumeric, dot, underscore, hyphen).
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct RepoName {
    /// Owner segment (user or org).
    pub owner: String,
    /// Repository name.
    pub name: String,
}

impl RepoName {
    /// Parse `owner/name`, optional leading `/` and trailing `.git`.
    pub fn parse(raw: &str) -> Result<Self> {
        let trimmed = raw.trim().trim_start_matches('/').trim_end_matches('/');
        let trimmed = trimmed.strip_suffix(".git").unwrap_or(trimmed);
        let Some((owner, name)) = trimmed.split_once('/') else {
            bail!("repository must be owner/name, got {raw:?}");
        };
        if owner.is_empty() || name.is_empty() || name.contains('/') {
            bail!("repository must be owner/name, got {raw:?}");
        }
        if !valid_segment(owner) || !valid_segment(name) {
            bail!("repository {raw:?} has invalid characters");
        }
        Ok(Self {
            owner: owner.to_string(),
            name: name.to_string(),
        })
    }

    /// `owner/name`.
    pub fn as_str(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }

    /// Directory name under `repos/` (`owner/name.git`).
    pub fn dir_name(&self) -> String {
        format!("{}/{}.git", self.owner, self.name)
    }
}

fn valid_segment(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
        && s != "."
        && s != ".."
        && !s.starts_with('.')
}

impl fmt::Display for RepoName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.owner, self.name)
    }
}

impl FromStr for RepoName {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        Self::parse(s)
    }
}

/// Login names: same character set as a repo segment.
pub fn valid_user(name: &str) -> Result<()> {
    if !valid_segment(name) {
        bail!("invalid user name {name:?}");
    }
    Ok(())
}

/// Protected branch refs that non-admins cannot push directly.
pub fn is_protected_ref(refname: &str) -> bool {
    refname == "refs/heads/master" || refname == "refs/heads/main"
}

/// Refs created by `git push origin HEAD:refs/rabun/requests/new/<branch>`.
pub fn request_new_ref(refname: &str) -> Option<&str> {
    refname.strip_prefix("refs/rabun/requests/new/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_owner_name() {
        let n = RepoName::parse("/Acme/app.git").unwrap();
        assert_eq!(n.owner, "Acme");
        assert_eq!(n.name, "app");
        assert_eq!(n.to_string(), "Acme/app");
    }

    #[test]
    fn rejects_bad_names() {
        assert!(RepoName::parse("nopath").is_err());
        assert!(RepoName::parse("../etc/passwd").is_err());
        assert!(RepoName::parse("a/b/c").is_err());
        assert!(RepoName::parse(".hidden/x").is_err());
    }
}
