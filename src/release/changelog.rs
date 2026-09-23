//! [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) read/write.

use indexmap::IndexMap;

use super::commit::{changelog_item, changelog_section, Commit};

const TEMPLATE: &str = "\
# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning 2.0.0](https://semver.org/spec/v2.0.0.html).

## [Unreleased]
";

const SECTION_ORDER: &[&str] = &[
    "Added",
    "Changed",
    "Deprecated",
    "Removed",
    "Fixed",
    "Security",
];

/// Notes generated from conventional commits (no file header).
pub fn render_notes(commits: &[Commit]) -> String {
    let mut sections: IndexMap<&str, Vec<String>> = IndexMap::new();
    for name in SECTION_ORDER {
        sections.insert(*name, Vec::new());
    }
    for commit in commits {
        let Some(section) = changelog_section(commit) else {
            continue;
        };
        sections
            .entry(section)
            .or_default()
            .push(format!("- {}", changelog_item(commit)));
    }
    format_sections(&sections)
}

/// Insert a versioned section under Unreleased, and refresh compare/tag footer links.
pub fn apply_release(
    existing: &str,
    version: &str,
    date: &str,
    generated: &str,
    repo_url: Option<&str>,
    tag_prefix: &str,
) -> String {
    let text = if existing.trim().is_empty() {
        TEMPLATE.to_string()
    } else {
        existing.to_string()
    };
    let (header, unreleased, rest) = split_unreleased(&text);
    let body = merge_unreleased(&unreleased, generated);
    let mut out = String::new();
    out.push_str(&header);
    if !header.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("## [Unreleased]\n");
    out.push('\n');
    out.push_str(&format!("## [{version}] - {date}\n"));
    if !body.is_empty() {
        out.push('\n');
        out.push_str(&body);
        if !body.ends_with('\n') {
            out.push('\n');
        }
    }
    out.push('\n');
    let rest = rest.trim_start_matches('\n');
    let (sections, footer) = split_footer(rest);
    if !sections.is_empty() {
        out.push_str(sections);
        if !sections.ends_with('\n') {
            out.push('\n');
        }
        if !footer.is_empty() {
            out.push('\n');
        }
    }
    out.push_str(&next_footer(footer, version, repo_url, tag_prefix));
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// HTTPS repo URL from a git remote or manifest `repository` field.
pub fn https_repo_url(raw: &str) -> Option<String> {
    let s = raw.trim().trim_end_matches('/').trim_end_matches(".git");
    if let Some(rest) = s.strip_prefix("git@github.com:") {
        return Some(format!("https://github.com/{rest}"));
    }
    if let Some(rest) = s.strip_prefix("ssh://git@github.com/") {
        return Some(format!("https://github.com/{rest}"));
    }
    if let Some(rest) = s.strip_prefix("git@") {
        let (host, path) = rest.split_once(':')?;
        return Some(format!("https://{host}/{path}"));
    }
    if s.starts_with("https://") || s.starts_with("http://") {
        let s = s.replacen("http://", "https://", 1);
        return Some(s);
    }
    None
}

fn split_unreleased(text: &str) -> (String, String, String) {
    const MARKER: &str = "## [Unreleased]";
    if let Some(idx) = text.find(MARKER) {
        let header = text[..idx].to_string();
        let after = &text[idx + MARKER.len()..];
        let after = after.strip_prefix('\n').unwrap_or(after);
        return match next_version_heading(after) {
            Some(at) => (
                header,
                after[..at].trim().to_string(),
                after[at..].to_string(),
            ),
            None => (header, after.trim().to_string(), String::new()),
        };
    }
    (
        text.trim_end().to_string() + "\n\n",
        String::new(),
        String::new(),
    )
}

fn next_version_heading(text: &str) -> Option<usize> {
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_end_matches('\n');
        if trimmed.starts_with("## [") && !trimmed.starts_with("## [Unreleased]") {
            return Some(offset);
        }
        offset += line.len();
    }
    None
}

fn split_footer(rest: &str) -> (&str, &str) {
    if let Some(idx) = rest.find("\n[") {
        if rest[idx + 2..].contains("]:") {
            return (&rest[..idx + 1], rest[idx + 1..].trim_start());
        }
    }
    if rest.starts_with('[') && rest.contains("]:") {
        return ("", rest);
    }
    (rest, "")
}

fn merge_unreleased(unreleased: &str, generated: &str) -> String {
    if unreleased.is_empty() {
        return generated.trim().to_string();
    }
    if generated.trim().is_empty() {
        return unreleased.trim().to_string();
    }
    if !unreleased.contains("### ") {
        return format!("{}\n\n{}", unreleased.trim(), generated.trim());
    }
    let mut sections = parse_sections(unreleased);
    let generated_sections = parse_sections(generated);
    for (name, items) in generated_sections {
        sections.entry(name).or_default().extend(items);
    }
    format_owned_sections(&sections)
}

fn parse_sections(text: &str) -> IndexMap<String, Vec<String>> {
    let mut map: IndexMap<String, Vec<String>> = IndexMap::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        if let Some(name) = line.strip_prefix("### ") {
            current = Some(name.trim().to_string());
            map.entry(name.trim().to_string()).or_default();
            continue;
        }
        if let Some(name) = &current {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            map.entry(name.clone()).or_default().push(line.to_string());
        }
    }
    map
}

fn format_sections(sections: &IndexMap<&str, Vec<String>>) -> String {
    let mut out = String::new();
    for (name, items) in sections {
        if items.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str("### ");
        out.push_str(name);
        out.push('\n');
        out.push('\n');
        out.push_str(&items.join("\n"));
        out.push('\n');
    }
    out
}

fn format_owned_sections(sections: &IndexMap<String, Vec<String>>) -> String {
    let mut out = String::new();
    let mut seen = std::collections::HashSet::new();
    for name in SECTION_ORDER {
        if let Some(items) = sections.get(*name) {
            if items.is_empty() {
                continue;
            }
            seen.insert(*name);
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str("### ");
            out.push_str(name);
            out.push('\n');
            out.push('\n');
            out.push_str(&items.join("\n"));
            out.push('\n');
        }
    }
    for (name, items) in sections {
        if seen.contains(name.as_str()) || items.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str("### ");
        out.push_str(name);
        out.push('\n');
        out.push('\n');
        out.push_str(&items.join("\n"));
        out.push('\n');
    }
    out
}

fn next_footer(existing: &str, version: &str, repo_url: Option<&str>, tag_prefix: &str) -> String {
    let url = match repo_url {
        Some(url) => url.to_string(),
        None => match url_from_existing_footer(existing) {
            Some(url) => url,
            None => return existing.trim().to_string(),
        },
    };
    let url = url.trim_end_matches('/');
    let tag = format!("{tag_prefix}{version}");
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!("[Unreleased]: {url}/compare/{tag}...HEAD"));
    lines.push(format!("[{version}]: {url}/releases/tag/{tag}"));
    for line in existing.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with("[Unreleased]:") {
            continue;
        }
        if trimmed.starts_with(&format!("[{version}]:")) {
            continue;
        }
        lines.push(trimmed.to_string());
    }
    lines.join("\n") + "\n"
}

fn url_from_existing_footer(footer: &str) -> Option<String> {
    for line in footer.lines() {
        if let Some(rest) = line.strip_prefix("[Unreleased]:") {
            let url = rest.trim();
            if let Some(base) = url.split("/compare/").next() {
                if base.starts_with("http") {
                    return Some(base.to_string());
                }
            }
        }
        if let Some((_, url)) = line.split_once("]: ") {
            if let Some(base) = url.split("/releases/tag/").next() {
                if base.starts_with("http") {
                    return Some(base.to_string());
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::release::commit::parse;

    #[test]
    fn inserts_version_and_footer() {
        let feat = parse("feat(api): add login").unwrap();
        let fix = parse("fix: nil panic").unwrap();
        let generated = render_notes(&[feat, fix]);
        let out = apply_release(
            "",
            "1.0.0",
            "2026-09-23",
            &generated,
            Some("https://github.com/acme/app"),
            "v",
        );
        assert!(out.contains("## [Unreleased]\n\n## [1.0.0] - 2026-09-23"));
        assert!(out.contains("### Added\n\n- **api:** add login"));
        assert!(out.contains("### Fixed\n\n- nil panic"));
        assert!(out.contains("[Unreleased]: https://github.com/acme/app/compare/v1.0.0...HEAD"));
        assert!(out.contains("[1.0.0]: https://github.com/acme/app/releases/tag/v1.0.0"));
    }

    #[test]
    fn keeps_unreleased_notes_and_old_versions() {
        let existing = "\
# Changelog

## [Unreleased]

### Added

- handwritten

## [0.1.0] - 2026-01-01

### Added

- start

[Unreleased]: https://github.com/acme/app/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/acme/app/releases/tag/v0.1.0
";
        let feat = parse("feat: more").unwrap();
        let generated = render_notes(&[feat]);
        let out = apply_release(existing, "0.2.0", "2026-09-23", &generated, None, "v");
        assert!(out.contains("- handwritten"));
        assert!(out.contains("- more"));
        assert!(out.contains("## [0.1.0] - 2026-01-01"));
        assert!(out.contains("[Unreleased]: https://github.com/acme/app/compare/v0.2.0...HEAD"));
        assert!(out.contains("[0.2.0]: https://github.com/acme/app/releases/tag/v0.2.0"));
        assert!(out.contains("[0.1.0]: https://github.com/acme/app/releases/tag/v0.1.0"));
    }

    #[test]
    fn https_from_ssh_remote() {
        assert_eq!(
            https_repo_url("git@github.com:rconnelly/rgit.git").as_deref(),
            Some("https://github.com/rconnelly/rgit")
        );
    }
}
