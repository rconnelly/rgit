//! Detect and surgically rewrite project version files.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use semver::Version;
use toml_edit::{value, DocumentMut};

use crate::git;
use crate::version;

/// A version file that is a source of truth (not a lockfile).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Source {
    /// Path relative to the project root.
    pub relative: PathBuf,
    /// How to read and write this file.
    pub kind: SourceKind,
    /// Parsed SemVer 2.0 version.
    pub version: Version,
}

/// Kind of source manifest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SourceKind {
    /// `Cargo.toml` `[package].version` or `[workspace.package].version`.
    Cargo { package_name: Option<String> },
    /// `package.json` top-level `version`.
    PackageJson,
    /// `pyproject.toml` `[project].version` or `[tool.poetry].version`.
    Pyproject,
    /// `composer.json` top-level `version`.
    Composer,
    /// `pubspec.yaml` top-level `version`.
    Pubspec,
    /// Helm `Chart.yaml` top-level `version`.
    Chart,
    /// `VERSION` or `version.txt`.
    VersionFile,
}

/// Lockfile or other derived version field kept in sync with a source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Derived {
    /// Path relative to the project root.
    pub relative: PathBuf,
    /// How to rewrite this file.
    pub kind: DerivedKind,
}

/// Kind of derived version file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DerivedKind {
    /// Root package stanza in `Cargo.lock`.
    CargoLock { package_name: String },
    /// `package-lock.json` root and `packages[""]` version.
    PackageLock,
}

/// All detected version files in a work tree.
#[derive(Clone, Debug, Default)]
pub struct Manifests {
    /// Source-of-truth files.
    pub sources: Vec<Source>,
    /// Lockfiles rewritten alongside sources.
    pub derived: Vec<Derived>,
}

impl Manifests {
    /// Shared SemVer across sources, or an error if they disagree or none exist.
    pub fn agreed_version(&self) -> Result<Version> {
        let Some(first) = self.sources.first() else {
            bail!(
                "no version files found (looked for Cargo.toml, package.json, pyproject.toml, composer.json, pubspec.yaml, Chart.yaml, VERSION)"
            );
        };
        for src in &self.sources {
            if src.version != first.version {
                bail!(
                    "version mismatch: {} is {} but {} is {}",
                    first.relative.display(),
                    first.version,
                    src.relative.display(),
                    src.version
                );
            }
        }
        Ok(first.version.clone())
    }

    /// Relative paths that `version bump` / `release` rewrites.
    pub fn paths(&self) -> Vec<PathBuf> {
        let mut paths: Vec<_> = self.sources.iter().map(|s| s.relative.clone()).collect();
        paths.extend(self.derived.iter().map(|d| d.relative.clone()));
        paths
    }
}

/// Scan `root` for known version files.
pub fn detect(root: &Path) -> Result<Manifests> {
    let mut manifests = Manifests::default();
    if let Some(src) = read_cargo(root)? {
        let name = match &src.kind {
            SourceKind::Cargo { package_name } => package_name.clone(),
            _ => None,
        };
        manifests.sources.push(src);
        if let Some(name) = name {
            let lock = root.join("Cargo.lock");
            if lock.exists() {
                manifests.derived.push(Derived {
                    relative: PathBuf::from("Cargo.lock"),
                    kind: DerivedKind::CargoLock { package_name: name },
                });
            }
        }
    }
    if let Some(src) = read_json_source(root, "package.json", SourceKind::PackageJson)? {
        manifests.sources.push(src);
        if root.join("package-lock.json").exists() {
            manifests.derived.push(Derived {
                relative: PathBuf::from("package-lock.json"),
                kind: DerivedKind::PackageLock,
            });
        }
    }
    if let Some(src) = read_pyproject(root)? {
        manifests.sources.push(src);
    }
    if let Some(src) = read_json_source(root, "composer.json", SourceKind::Composer)? {
        manifests.sources.push(src);
    }
    if let Some(src) = read_yaml_source(root, "pubspec.yaml", SourceKind::Pubspec)? {
        manifests.sources.push(src);
    }
    if let Some(src) = read_yaml_source(root, "Chart.yaml", SourceKind::Chart)? {
        manifests.sources.push(src);
    }
    for name in ["VERSION", "version.txt"] {
        if let Some(src) = read_version_file(root, name)? {
            manifests.sources.push(src);
        }
    }
    Ok(manifests)
}

/// Rewrite every detected file to `new` (canonical SemVer, no `v`).
pub fn set_version(root: &Path, manifests: &Manifests, new: &str) -> Result<Vec<PathBuf>> {
    let mut written = Vec::new();
    for src in &manifests.sources {
        let path = root.join(&src.relative);
        let text =
            std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        let next = rewrite_source(&src.kind, &text, new)?;
        std::fs::write(&path, next).with_context(|| format!("write {}", path.display()))?;
        written.push(src.relative.clone());
    }
    for der in &manifests.derived {
        let path = root.join(&der.relative);
        let text =
            std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        let next = rewrite_derived(&der.kind, &text, new)?;
        std::fs::write(&path, next).with_context(|| format!("write {}", path.display()))?;
        written.push(der.relative.clone());
    }
    Ok(written)
}

/// Versions of source files at `commit` (and lockfiles if present).
pub async fn versions_in_tree(repo: &Path, commit: &str) -> Result<Vec<(String, Version)>> {
    let mut out = Vec::new();
    for path in [
        "Cargo.toml",
        "package.json",
        "pyproject.toml",
        "composer.json",
        "pubspec.yaml",
        "Chart.yaml",
        "VERSION",
        "version.txt",
        "Cargo.lock",
        "package-lock.json",
    ] {
        let Some(text) = git::show_path(repo, commit, path).await? else {
            continue;
        };
        if let Some(ver) = version_in_file(path, &text)? {
            out.push((path.to_string(), ver));
        }
    }
    Ok(out)
}

/// Require every versioned file at `commit` to equal `expected`.
pub async fn require_tree_version(repo: &Path, commit: &str, expected: &Version) -> Result<()> {
    let found = versions_in_tree(repo, commit).await?;
    if found.is_empty() {
        bail!("tag {expected} has no version files in the tagged commit");
    }
    for (path, ver) in found {
        if ver != *expected {
            bail!("tag {expected} does not match {path} ({ver})");
        }
    }
    Ok(())
}

fn version_in_file(path: &str, text: &str) -> Result<Option<Version>> {
    match path {
        "Cargo.toml" => Ok(cargo_version_str(text)?
            .map(|s| version::parse(&s))
            .transpose()?),
        "package.json" | "composer.json" => Ok(json_root_version(text)?
            .map(|s| version::parse(&s))
            .transpose()?),
        "pyproject.toml" => Ok(pyproject_version_str(text)?
            .map(|s| version::parse(&s))
            .transpose()?),
        "pubspec.yaml" | "Chart.yaml" => Ok(yaml_top_level_version(text)
            .map(|s| version::parse(&s))
            .transpose()?),
        "VERSION" | "version.txt" => {
            let line = text.lines().next().unwrap_or("").trim();
            if line.is_empty() {
                Ok(None)
            } else {
                Ok(Some(version::parse(line)?))
            }
        }
        "package-lock.json" => Ok(json_root_version(text)?
            .map(|s| version::parse(&s))
            .transpose()?),
        "Cargo.lock" => Ok(None),
        _ => Ok(None),
    }
}

fn read_cargo(root: &Path) -> Result<Option<Source>> {
    let path = root.join("Cargo.toml");
    if !path.exists() {
        return Ok(None);
    }
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let Some(ver) = cargo_version_str(&text)? else {
        return Ok(None);
    };
    let name = cargo_package_name(&text)?;
    Ok(Some(Source {
        relative: PathBuf::from("Cargo.toml"),
        kind: SourceKind::Cargo { package_name: name },
        version: version::parse(&ver)?,
    }))
}

fn cargo_package_name(text: &str) -> Result<Option<String>> {
    let doc: DocumentMut = text.parse().context("parse Cargo.toml")?;
    Ok(doc
        .get("package")
        .and_then(|item| item.get("name"))
        .and_then(|item| item.as_str())
        .map(str::to_string))
}

fn cargo_version_str(text: &str) -> Result<Option<String>> {
    let doc: DocumentMut = text.parse().context("parse Cargo.toml")?;
    if let Some(v) = table_string(&doc, &["package", "version"]) {
        return Ok(Some(v));
    }
    Ok(table_string(&doc, &["workspace", "package", "version"]))
}

fn read_pyproject(root: &Path) -> Result<Option<Source>> {
    let path = root.join("pyproject.toml");
    if !path.exists() {
        return Ok(None);
    }
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let Some(ver) = pyproject_version_str(&text)? else {
        return Ok(None);
    };
    Ok(Some(Source {
        relative: PathBuf::from("pyproject.toml"),
        kind: SourceKind::Pyproject,
        version: version::parse(&ver)?,
    }))
}

fn pyproject_version_str(text: &str) -> Result<Option<String>> {
    let doc: DocumentMut = text.parse().context("parse pyproject.toml")?;
    if let Some(v) = table_string(&doc, &["project", "version"]) {
        return Ok(Some(v));
    }
    Ok(table_string(&doc, &["tool", "poetry", "version"]))
}

fn table_string(doc: &DocumentMut, path: &[&str]) -> Option<String> {
    let mut item = doc.as_item();
    for key in path {
        item = item.get(key)?;
    }
    item.as_str().map(str::to_string)
}

fn read_json_source(root: &Path, name: &str, kind: SourceKind) -> Result<Option<Source>> {
    let path = root.join(name);
    if !path.exists() {
        return Ok(None);
    }
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let Some(ver) = json_root_version(&text)? else {
        return Ok(None);
    };
    Ok(Some(Source {
        relative: PathBuf::from(name),
        kind,
        version: version::parse(&ver)?,
    }))
}

fn json_root_version(text: &str) -> Result<Option<String>> {
    let value: serde_json::Value = serde_json::from_str(text).context("parse JSON")?;
    Ok(value
        .get("version")
        .and_then(|v| v.as_str())
        .map(str::to_string))
}

fn read_yaml_source(root: &Path, name: &str, kind: SourceKind) -> Result<Option<Source>> {
    let path = root.join(name);
    if !path.exists() {
        return Ok(None);
    }
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let Some(ver) = yaml_top_level_version(&text) else {
        return Ok(None);
    };
    Ok(Some(Source {
        relative: PathBuf::from(name),
        kind,
        version: version::parse(&ver)?,
    }))
}

fn yaml_top_level_version(text: &str) -> Option<String> {
    for line in text.lines() {
        if line.starts_with(' ') || line.starts_with('\t') {
            continue;
        }
        let Some(rest) = line.strip_prefix("version:") else {
            continue;
        };
        let value = strip_yaml_quotes(rest.trim());
        if value.is_empty() {
            return None;
        }
        return Some(value);
    }
    None
}

fn strip_yaml_quotes(value: &str) -> String {
    let value = value.trim();
    if (value.starts_with('"') && value.ends_with('"') && value.len() >= 2)
        || (value.starts_with('\'') && value.ends_with('\'') && value.len() >= 2)
    {
        value[1..value.len() - 1].to_string()
    } else {
        value.to_string()
    }
}

fn read_version_file(root: &Path, name: &str) -> Result<Option<Source>> {
    let path = root.join(name);
    if !path.exists() {
        return Ok(None);
    }
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let line = text.lines().next().unwrap_or("").trim();
    if line.is_empty() {
        return Ok(None);
    }
    Ok(Some(Source {
        relative: PathBuf::from(name),
        kind: SourceKind::VersionFile,
        version: version::parse(line)?,
    }))
}

fn rewrite_source(kind: &SourceKind, text: &str, new: &str) -> Result<String> {
    match kind {
        SourceKind::Cargo { .. } => rewrite_cargo(text, new),
        SourceKind::PackageJson | SourceKind::Composer => {
            replace_json_strings(text, &[(&["version"], new)])
        }
        SourceKind::Pyproject => rewrite_pyproject(text, new),
        SourceKind::Pubspec | SourceKind::Chart => replace_yaml_top_level_version(text, new),
        SourceKind::VersionFile => Ok(format!("{new}\n")),
    }
}

fn rewrite_derived(kind: &DerivedKind, text: &str, new: &str) -> Result<String> {
    match kind {
        DerivedKind::CargoLock { package_name } => rewrite_cargo_lock(text, package_name, new),
        DerivedKind::PackageLock => replace_json_strings(
            text,
            &[(&["version"], new), (&["packages", "", "version"], new)],
        ),
    }
}

fn rewrite_cargo(text: &str, new: &str) -> Result<String> {
    let mut doc: DocumentMut = text.parse().context("parse Cargo.toml")?;
    if table_string(&doc, &["package", "version"]).is_some() {
        doc["package"]["version"] = value(new);
        return Ok(doc.to_string());
    }
    if table_string(&doc, &["workspace", "package", "version"]).is_some() {
        doc["workspace"]["package"]["version"] = value(new);
        return Ok(doc.to_string());
    }
    bail!("Cargo.toml has no package.version or workspace.package.version");
}

fn rewrite_pyproject(text: &str, new: &str) -> Result<String> {
    let mut doc: DocumentMut = text.parse().context("parse pyproject.toml")?;
    if table_string(&doc, &["project", "version"]).is_some() {
        doc["project"]["version"] = value(new);
        return Ok(doc.to_string());
    }
    if table_string(&doc, &["tool", "poetry", "version"]).is_some() {
        doc["tool"]["poetry"]["version"] = value(new);
        return Ok(doc.to_string());
    }
    bail!("pyproject.toml has no project.version or tool.poetry.version");
}

fn rewrite_cargo_lock(text: &str, package_name: &str, new: &str) -> Result<String> {
    let mut doc: DocumentMut = text.parse().context("parse Cargo.lock")?;
    let Some(packages) = doc
        .get_mut("package")
        .and_then(|item| item.as_array_of_tables_mut())
    else {
        bail!("Cargo.lock has no [[package]] tables");
    };
    let mut found = false;
    for pkg in packages.iter_mut() {
        if pkg.get("name").and_then(|n| n.as_str()) == Some(package_name) {
            pkg["version"] = value(new);
            found = true;
            break;
        }
    }
    if !found {
        bail!("Cargo.lock has no package named {package_name}");
    }
    Ok(doc.to_string())
}

fn replace_yaml_top_level_version(text: &str, new: &str) -> Result<String> {
    let mut out = Vec::new();
    let mut replaced = false;
    for line in text.lines() {
        if !replaced && !line.starts_with(' ') && !line.starts_with('\t') {
            if let Some(rest) = line.strip_prefix("version:") {
                let current = rest.trim();
                let quote = if current.starts_with('"') {
                    "\""
                } else if current.starts_with('\'') {
                    "'"
                } else {
                    ""
                };
                out.push(format!("version: {quote}{new}{quote}"));
                replaced = true;
                continue;
            }
        }
        out.push(line.to_string());
    }
    if !replaced {
        bail!("no top-level version field");
    }
    let mut joined = out.join("\n");
    if text.ends_with('\n') {
        joined.push('\n');
    }
    Ok(joined)
}

/// Replace JSON string values at object key paths, preserving surrounding text.
fn replace_json_strings(text: &str, updates: &[(&[&str], &str)]) -> Result<String> {
    let mut cur = Cursor {
        s: text,
        i: 0,
        out: String::with_capacity(text.len() + 16),
    };
    let mut path = Vec::new();
    parse_value(&mut cur, &mut path, updates)?;
    cur.skip_ws();
    if cur.i != cur.s.len() {
        cur.out.push_str(&cur.s[cur.i..]);
    }
    Ok(cur.out)
}

struct Cursor<'a> {
    s: &'a str,
    i: usize,
    out: String,
}

impl Cursor<'_> {
    fn peek(&self) -> Option<char> {
        self.s[self.i..].chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let mut chars = self.s[self.i..].chars();
        let c = chars.next()?;
        self.out.push(c);
        self.i += c.len_utf8();
        Some(c)
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(c) if c.is_whitespace()) {
            self.bump();
        }
    }
}

fn parse_value(
    cur: &mut Cursor<'_>,
    path: &mut Vec<String>,
    updates: &[(&[&str], &str)],
) -> Result<()> {
    cur.skip_ws();
    match cur.peek() {
        Some('{') => parse_object(cur, path, updates),
        Some('[') => parse_array(cur, path, updates),
        Some('"') => {
            copy_json_string(cur)?;
            Ok(())
        }
        Some('t') => copy_literal(cur, "true"),
        Some('f') => copy_literal(cur, "false"),
        Some('n') => copy_literal(cur, "null"),
        Some('-' | '0'..='9') => copy_number(cur),
        other => bail!("invalid JSON value starting with {other:?}"),
    }
}

fn parse_object(
    cur: &mut Cursor<'_>,
    path: &mut Vec<String>,
    updates: &[(&[&str], &str)],
) -> Result<()> {
    cur.bump();
    loop {
        cur.skip_ws();
        match cur.peek() {
            Some('}') => {
                cur.bump();
                return Ok(());
            }
            Some('"') => {
                let key = copy_json_string(cur)?;
                cur.skip_ws();
                if cur.peek() != Some(':') {
                    bail!("expected ':' in JSON object");
                }
                cur.bump();
                cur.skip_ws();
                path.push(key);
                let replace = updates
                    .iter()
                    .find(|(want, _)| path_eq(path, want))
                    .map(|(_, v)| *v);
                if let Some(new) = replace {
                    if cur.peek() == Some('"') {
                        replace_json_string(cur, new)?;
                    } else {
                        parse_value(cur, path, updates)?;
                    }
                } else {
                    parse_value(cur, path, updates)?;
                }
                path.pop();
                cur.skip_ws();
                match cur.peek() {
                    Some(',') => {
                        cur.bump();
                    }
                    Some('}') => {
                        cur.bump();
                        return Ok(());
                    }
                    other => bail!("expected ',' or '}}' in JSON object, got {other:?}"),
                }
            }
            other => bail!("invalid JSON object, got {other:?}"),
        }
    }
}

fn path_eq(path: &[String], want: &[&str]) -> bool {
    path.len() == want.len() && path.iter().zip(want.iter()).all(|(a, b)| a == b)
}

fn parse_array(
    cur: &mut Cursor<'_>,
    path: &mut Vec<String>,
    updates: &[(&[&str], &str)],
) -> Result<()> {
    cur.bump();
    loop {
        cur.skip_ws();
        if cur.peek() == Some(']') {
            cur.bump();
            return Ok(());
        }
        parse_value(cur, path, updates)?;
        cur.skip_ws();
        match cur.peek() {
            Some(',') => {
                cur.bump();
            }
            Some(']') => {
                cur.bump();
                return Ok(());
            }
            other => bail!("expected ',' or ']' in JSON array, got {other:?}"),
        }
    }
}

fn copy_json_string(cur: &mut Cursor<'_>) -> Result<String> {
    json_string(cur, None)
}

fn replace_json_string(cur: &mut Cursor<'_>, new: &str) -> Result<String> {
    json_string(cur, Some(new))
}

fn json_string(cur: &mut Cursor<'_>, replace: Option<&str>) -> Result<String> {
    if cur.peek() != Some('"') {
        bail!("expected JSON string");
    }
    let start = cur.i;
    cur.i += 1;
    let mut decoded = String::new();
    let mut escaped = false;
    loop {
        let Some(c) = cur.s[cur.i..].chars().next() else {
            bail!("unterminated JSON string");
        };
        cur.i += c.len_utf8();
        if escaped {
            decoded.push(c);
            escaped = false;
            continue;
        }
        if c == '\\' {
            escaped = true;
            continue;
        }
        if c == '"' {
            break;
        }
        decoded.push(c);
    }
    if let Some(new) = replace {
        cur.out.push_str(&serde_json::to_string(new)?);
    } else {
        cur.out.push_str(&cur.s[start..cur.i]);
    }
    Ok(decoded)
}

fn copy_literal(cur: &mut Cursor<'_>, lit: &str) -> Result<()> {
    if !cur.s[cur.i..].starts_with(lit) {
        bail!("expected `{lit}`");
    }
    cur.out.push_str(lit);
    cur.i += lit.len();
    Ok(())
}

fn copy_number(cur: &mut Cursor<'_>) -> Result<()> {
    let start = cur.i;
    if cur.peek() == Some('-') {
        cur.i += 1;
    }
    while matches!(cur.peek(), Some(c) if c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '+' | '-'))
    {
        cur.i += 1;
    }
    if start == cur.i {
        bail!("expected JSON number");
    }
    cur.out.push_str(&cur.s[start..cur.i]);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_each_manifest_kind() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n# keep\n",
        )
        .unwrap();
        std::fs::write(
            root.join("Cargo.lock"),
            "[[package]]\nname = \"demo\"\nversion = \"0.1.0\"\n\n[[package]]\nname = \"other\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();
        std::fs::write(
            root.join("package.json"),
            "{\n  \"name\": \"demo\",\n  \"version\": \"0.1.0\"\n}\n",
        )
        .unwrap();
        std::fs::write(
            root.join("package-lock.json"),
            "{\n  \"name\": \"demo\",\n  \"version\": \"0.1.0\",\n  \"packages\": {\n    \"\": {\n      \"name\": \"demo\",\n      \"version\": \"0.1.0\"\n    }\n  }\n}\n",
        )
        .unwrap();
        std::fs::write(
            root.join("pyproject.toml"),
            "[project]\nname = \"demo\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        std::fs::write(
            root.join("composer.json"),
            "{\n  \"version\": \"0.1.0\"\n}\n",
        )
        .unwrap();
        std::fs::write(root.join("pubspec.yaml"), "name: demo\nversion: 0.1.0+1\n").unwrap();
        std::fs::write(
            root.join("Chart.yaml"),
            "apiVersion: v2\nname: demo\nversion: 0.1.0\nappVersion: \"1.0.0\"\n",
        )
        .unwrap();
        std::fs::write(root.join("VERSION"), "0.1.0\n").unwrap();

        let found = detect(root).unwrap();
        assert_eq!(found.sources.len(), 7);
        set_version(root, &found, "0.2.0").unwrap();

        let again = detect(root).unwrap();
        assert_eq!(again.agreed_version().unwrap(), Version::new(0, 2, 0));
        let cargo = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
        assert!(cargo.contains("# keep"));
        let lock = std::fs::read_to_string(root.join("Cargo.lock")).unwrap();
        assert!(lock.contains("name = \"demo\"\nversion = \"0.2.0\""));
        assert!(lock.contains("name = \"other\"\nversion = \"1.0.0\""));
        let pkg = std::fs::read_to_string(root.join("package.json")).unwrap();
        assert!(pkg.contains("\"name\": \"demo\""));
        let plock = std::fs::read_to_string(root.join("package-lock.json")).unwrap();
        assert!(plock.contains("\"version\": \"0.2.0\""));
        assert!(!plock.contains("\"version\": \"0.1.0\""));
        let chart = std::fs::read_to_string(root.join("Chart.yaml")).unwrap();
        assert!(chart.contains("version: 0.2.0"));
        assert!(chart.contains("appVersion: \"1.0.0\""));
        let pubspec = std::fs::read_to_string(root.join("pubspec.yaml")).unwrap();
        assert_eq!(pubspec, "name: demo\nversion: 0.2.0\n");
    }

    #[test]
    fn mismatch_is_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("Cargo.toml"),
            "[package]\nname = \"a\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        std::fs::write(tmp.path().join("VERSION"), "0.2.0\n").unwrap();
        let err = detect(tmp.path()).unwrap().agreed_version().unwrap_err();
        assert!(err.to_string().contains("mismatch"));
    }
}
