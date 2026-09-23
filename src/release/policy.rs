//! In-repo `.rabun/version.toml` policy.

use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::git;

/// Release and enforcement settings.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(default)]
pub struct Policy {
    /// Git tag prefix (`v` → `v1.2.3`).
    pub tag_prefix: String,
    /// Keep a Changelog path relative to the work tree.
    pub changelog: String,
    /// Push-time and (when installed) commit-msg gates.
    pub enforce: Enforce,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            tag_prefix: "v".into(),
            changelog: "CHANGELOG.md".into(),
            enforce: Enforce::default(),
        }
    }
}

/// Opt-in forge (and local) enforcement flags.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(default)]
pub struct Enforce {
    /// Reject non-conventional commit messages.
    pub commits: bool,
    /// Require SemVer 2.0 git tag names (`vMAJOR.MINOR.PATCH`).
    pub tags: bool,
    /// Require tagged trees' version files to match the tag.
    pub manifests: bool,
}

#[derive(Deserialize)]
struct File {
    #[serde(default)]
    version: Policy,
}

/// Parse `.rabun/version.toml` contents. Missing `[version]` uses defaults.
pub fn parse(text: &str) -> Result<Policy> {
    let file: File = toml::from_str(text).context("parse .rabun/version.toml")?;
    Ok(file.version)
}

/// Load from a work tree. Missing file is defaults (no forge enforcement).
pub fn load_file(root: &Path) -> Result<Policy> {
    let path = root.join(".rabun").join("version.toml");
    if !path.exists() {
        return Ok(Policy::default());
    }
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    parse(&text)
}

/// Load policy from a commit tree. `None` when the file is absent.
pub async fn load_from_tree(repo: &Path, commit: &str) -> Result<Option<Policy>> {
    match git::show_path(repo, commit, ".rabun/version.toml").await? {
        None => Ok(None),
        Some(text) => Ok(Some(parse(&text)?)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_overrides() {
        let empty = parse("").unwrap();
        assert_eq!(empty, Policy::default());
        assert!(!empty.enforce.commits);

        let p = parse(
            r#"
[version]
tag_prefix = "ver"
changelog = "docs/changes.md"

[version.enforce]
commits = true
tags = true
manifests = true
"#,
        )
        .unwrap();
        assert_eq!(p.tag_prefix, "ver");
        assert_eq!(p.changelog, "docs/changes.md");
        assert!(p.enforce.commits && p.enforce.tags && p.enforce.manifests);
    }
}
