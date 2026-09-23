//! [Semantic Versioning 2.0.0](https://semver.org/spec/v2.0.0.html) for crate
//! versions and git tags.

use anyhow::{anyhow, Result};
use semver::Version;

/// Crate version from `Cargo.toml` (SemVer 2.0.0, no `v` prefix).
pub fn crate_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Git tag for a crate release: `v` + `Cargo.toml` version.
pub fn crate_git_tag() -> String {
    format!("v{}", crate_version())
}

/// Parse a version or git tag as SemVer 2.0.0.
///
/// A single leading `v` or `V` is allowed (git tag convention). It is not part
/// of the SemVer grammar. The remainder must be `MAJOR.MINOR.PATCH` with
/// optional pre-release and build metadata.
pub fn parse(input: &str) -> Result<Version> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(anyhow!(
            "empty version; SemVer 2.0.0 requires MAJOR.MINOR.PATCH"
        ));
    }
    let rest = match trimmed.as_bytes().first() {
        Some(b'v' | b'V') => &trimmed[1..],
        _ => trimmed,
    };
    if rest.is_empty() {
        return Err(anyhow!(
            "`{input}` is not SemVer 2.0.0; expected MAJOR.MINOR.PATCH after an optional v prefix"
        ));
    }
    Version::parse(rest).map_err(|err| {
        anyhow!(
            "`{input}` is not SemVer 2.0.0 ({err}); expected MAJOR.MINOR.PATCH with optional pre-release and build metadata"
        )
    })
}

/// True when this version has no pre-release identifiers (build metadata is ignored).
pub fn is_stable(version: &Version) -> bool {
    version.pre.is_empty()
}

/// True when `next` is greater than `current` per SemVer 2.0 precedence.
pub fn is_upgrade(current_tag: &str, next_tag: &str) -> Result<bool> {
    Ok(parse(next_tag)? > parse(current_tag)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use semver::Version;

    #[test]
    fn crate_version_is_semver() {
        let v = parse(crate_version()).unwrap();
        assert_eq!(v, Version::parse(crate_version()).unwrap());
        assert_eq!(crate_git_tag(), format!("v{}", crate_version()));
        assert!(is_stable(&v), "published crate version should be stable");
    }

    #[test]
    fn accepts_optional_v_prefix() {
        let a = parse("0.12.0").unwrap();
        let b = parse("v0.12.0").unwrap();
        let c = parse("V0.12.0").unwrap();
        assert_eq!(a, b);
        assert_eq!(b, c);
        assert_eq!(a, Version::new(0, 12, 0));
    }

    #[test]
    fn accepts_prerelease_and_build() {
        let pre = parse("v1.2.3-alpha.1").unwrap();
        assert_eq!(pre.pre.as_str(), "alpha.1");
        assert!(!is_stable(&pre));
        let build = parse("1.0.0+20130313144700").unwrap();
        assert_eq!(build.build.as_str(), "20130313144700");
        assert!(is_stable(&build));
    }

    #[test]
    fn rejects_non_semver() {
        for bad in ["", "v", "latest", "main", "1.0", "01.0.0", "v1", "1.2.3.4"] {
            assert!(parse(bad).is_err(), "expected error for `{bad}`");
        }
    }

    #[test]
    fn precedence_matches_semver_spec() {
        assert!(is_upgrade("v0.7.0", "v0.8.0").unwrap());
        assert!(is_upgrade("1.0.0-alpha", "1.0.0").unwrap());
        assert!(!is_upgrade("v1.0.0", "v1.0.0").unwrap());
        assert!(!is_upgrade("v1.2.0", "v1.1.9").unwrap());
    }
}
