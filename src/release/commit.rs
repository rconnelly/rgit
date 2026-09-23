//! [Conventional Commits 1.0.0](https://www.conventionalcommits.org/en/v1.0.0/).

use anyhow::{bail, Result};
use semver::Version;

use crate::version::Bump;

/// Parsed Conventional Commits header plus body.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Commit {
    /// Type (`feat`, `fix`, …).
    pub kind: String,
    /// Optional scope (`feat(api):`).
    pub scope: Option<String>,
    /// `!` after type/scope or a `BREAKING CHANGE` footer.
    pub breaking: bool,
    /// Text after `: ` on the subject line.
    pub description: String,
    /// Remainder after the subject (trimmed).
    pub body: String,
}

/// How a commit message is classified for policy and bumps.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Message {
    /// Matches Conventional Commits 1.0.0.
    Conventional(Commit),
    /// Git-generated merge, revert, or fixup; skip enforcement.
    Exempt,
}

/// Strip `#` comment lines (as `commit-msg` files still contain them).
pub fn strip_comments(message: &str) -> String {
    message
        .lines()
        .filter(|line| !line.starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

/// True for git-generated subjects that are not Conventional Commits.
pub fn is_exempt(message: &str) -> bool {
    let first = message.lines().next().unwrap_or("").trim();
    first.starts_with("Merge ")
        || first.starts_with("Revert ")
        || first.starts_with("fixup!")
        || first.starts_with("squash!")
        || first.starts_with("amend!")
}

/// Parse or exempt a raw commit message (including `#` comments).
pub fn classify(message: &str) -> Result<Message> {
    let cleaned = strip_comments(message);
    if cleaned.is_empty() {
        bail!("empty commit message; Conventional Commits 1.0.0 requires type: description");
    }
    if is_exempt(&cleaned) {
        return Ok(Message::Exempt);
    }
    Ok(Message::Conventional(parse(&cleaned)?))
}

/// Require a Conventional Commits 1.0.0 message (exempt git templates).
pub fn validate(message: &str) -> Result<()> {
    classify(message).map(|_| ())
}

/// Parse a cleaned Conventional Commits message (no leading comments).
pub fn parse(message: &str) -> Result<Commit> {
    let message = message.trim();
    let Some((header, rest)) = message.split_once('\n') else {
        return parse_header(message, "");
    };
    let body = rest.trim().to_string();
    parse_header(header.trim_end(), &body)
}

fn parse_header(header: &str, body: &str) -> Result<Commit> {
    let bytes = header.as_bytes();
    let mut i = 0;
    if i >= bytes.len() || !bytes[i].is_ascii_lowercase() {
        bail!(
            "commit message is not Conventional Commits 1.0.0; expected type: description, got `{header}`"
        );
    }
    i += 1;
    while i < bytes.len() && bytes[i].is_ascii_alphanumeric() {
        i += 1;
    }
    let kind = header[..i].to_string();
    let mut scope = None;
    if i < bytes.len() && bytes[i] == b'(' {
        i += 1;
        let start = i;
        while i < bytes.len() && bytes[i] != b')' {
            i += 1;
        }
        if i >= bytes.len() {
            bail!("unclosed scope in commit header `{header}`");
        }
        if start == i {
            bail!("empty commit scope in `{header}`");
        }
        scope = Some(header[start..i].to_string());
        i += 1;
    }
    let mut breaking = false;
    if i < bytes.len() && bytes[i] == b'!' {
        breaking = true;
        i += 1;
    }
    if i + 2 > header.len() || &header[i..i + 2] != ": " {
        bail!(
            "commit message is not Conventional Commits 1.0.0; expected `: ` after type, got `{header}`"
        );
    }
    i += 2;
    let description = header[i..].trim().to_string();
    if description.is_empty() {
        bail!("empty commit description in `{header}`");
    }
    if footer_breaking(body) {
        breaking = true;
    }
    Ok(Commit {
        kind,
        scope,
        breaking,
        description,
        body: body.to_string(),
    })
}

fn footer_breaking(body: &str) -> bool {
    body.lines()
        .any(|line| line.starts_with("BREAKING CHANGE:") || line.starts_with("BREAKING-CHANGE:"))
}

/// True when this commit should not affect an automatic SemVer bump.
pub fn skip_for_bump(commit: &Commit) -> bool {
    commit.kind == "chore" && commit.scope.as_deref() == Some("release")
}

/// Infer a bump from conventional commits. `0.x` breaking changes are minor.
pub fn inferred_bump(commits: &[Commit], current: &Version) -> Option<Bump> {
    let mut best: Option<Bump> = None;
    for commit in commits {
        if skip_for_bump(commit) {
            continue;
        }
        let mut level = match commit.kind.as_str() {
            "feat" => Some(Bump::Minor),
            "fix" | "perf" => Some(Bump::Patch),
            _ => None,
        };
        if commit.breaking {
            level = Some(if current.major == 0 {
                Bump::Minor
            } else {
                Bump::Major
            });
        }
        if let Some(level) = level {
            best = Some(match best {
                Some(prev) => prev.max(level),
                None => level,
            });
        }
    }
    best
}

/// Keep a Changelog heading for a commit type, if any.
pub fn changelog_section(commit: &Commit) -> Option<&'static str> {
    let section = match commit.kind.as_str() {
        "feat" => Some("Added"),
        "fix" => Some("Fixed"),
        "perf" | "refactor" | "docs" | "style" | "revert" => Some("Changed"),
        "security" => Some("Security"),
        "remove" | "removed" => Some("Removed"),
        _ => None,
    };
    if section.is_some() {
        return section;
    }
    if commit.breaking {
        Some("Changed")
    } else {
        None
    }
}

/// One Keep a Changelog bullet for a conventional commit.
pub fn changelog_item(commit: &Commit) -> String {
    let desc = &commit.description;
    match (commit.breaking, commit.scope.as_deref()) {
        (true, Some(scope)) => format!("**BREAKING ({scope}):** {desc}"),
        (true, None) => format!("**BREAKING:** {desc}"),
        (false, Some(scope)) => format!("**{scope}:** {desc}"),
        (false, None) => desc.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use semver::Version;

    #[test]
    fn parses_types_scopes_and_breaking() {
        let a = parse("feat: add login").unwrap();
        assert_eq!(a.kind, "feat");
        assert_eq!(a.description, "add login");
        assert!(!a.breaking);

        let b = parse("feat(api): add login").unwrap();
        assert_eq!(b.scope.as_deref(), Some("api"));

        let c = parse("feat!: drop v1").unwrap();
        assert!(c.breaking);

        let d = parse("feat(api)!: drop v1").unwrap();
        assert!(d.breaking);
        assert_eq!(d.scope.as_deref(), Some("api"));

        let e = parse("fix: nil\n\nBREAKING CHANGE: gone").unwrap();
        assert!(e.breaking);
    }

    #[test]
    fn rejects_invalid() {
        for bad in [
            "hello",
            "feat:",
            "feat: ",
            "FEAT: x",
            "feat(api: x",
            "feat(): x",
        ] {
            assert!(parse(bad).is_err(), "expected error for `{bad}`");
        }
    }

    #[test]
    fn classify_exempts_git_templates() {
        assert_eq!(classify("Merge branch 'x'").unwrap(), Message::Exempt);
        assert_eq!(classify("Revert \"feat: x\"\n").unwrap(), Message::Exempt);
        assert!(matches!(
            classify("feat: ok\n# please enter").unwrap(),
            Message::Conventional(_)
        ));
        assert!(classify("").is_err());
        assert!(classify("# only comments\n").is_err());
    }

    #[test]
    fn infers_semver_including_zero_major() {
        let v0 = Version::new(0, 12, 1);
        let v1 = Version::new(1, 0, 0);
        assert_eq!(
            inferred_bump(&[parse("fix: x").unwrap()], &v0),
            Some(Bump::Patch)
        );
        assert_eq!(
            inferred_bump(&[parse("feat: x").unwrap()], &v0),
            Some(Bump::Minor)
        );
        assert_eq!(
            inferred_bump(&[parse("feat!: x").unwrap()], &v0),
            Some(Bump::Minor)
        );
        assert_eq!(
            inferred_bump(&[parse("feat!: x").unwrap()], &v1),
            Some(Bump::Major)
        );
        assert_eq!(
            inferred_bump(&[parse("feat: x").unwrap(), parse("fix: x").unwrap()], &v0),
            Some(Bump::Minor)
        );
        assert_eq!(
            inferred_bump(&[parse("chore(release): 0.12.2").unwrap()], &v0),
            None
        );
    }
}
