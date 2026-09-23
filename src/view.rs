//! Loopback Zola preview of a local git tree (`rgit view`).
//!
//! This is a this-machine command. It does not open HTTP on `rabun-git serve`.

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{bail, Context, Result};

use crate::config::Config;
use crate::git;
use crate::names::RepoName;

/// Skip blobs larger than this (512 KiB).
pub const MAX_BLOB_BYTES: u64 = 512 * 1024;

const SKIP_COMPONENTS: &[&str] = &[".git", "target", "node_modules", ".cache", "dist"];
const THEME_URL: &str = "https://codeberg.org/RiPetitor/devlab-theme";
const THEME_TAG: &str = "v0.7.0";
const THEME_COMMIT: &str = "8fea270cd282e4b0137fb39ae63603698509b6fa";
const README_CANDIDATES: &[&str] = &["README.md", "README", "readme.md", "Readme.md"];

const OVERLAY_BASE: &str = include_str!("../view-site/templates/base.html");
const OVERLAY_INDEX: &str = include_str!("../view-site/templates/index.html");
const OVERLAY_SECTION: &str = include_str!("../view-site/templates/section.html");
const OVERLAY_PAGE: &str = include_str!("../view-site/templates/page.html");
const OVERLAY_CRUMBS: &str = include_str!("../view-site/templates/partials/repo-crumbs.html");
const OVERLAY_THEME_INIT: &str = include_str!("../view-site/templates/partials/theme-init.html");
const OVERLAY_CSS: &str = include_str!("../view-site/static/custom.css");
const OVERLAY_JS: &str = include_str!("../view-site/static/repo.js");
const OVERLAY_MARK: &str = include_str!("../view-site/static/brand/rabun.svg");

/// CLI options for [`run`].
#[derive(Clone, Debug)]
pub struct Options {
    /// Path or `owner/name`.
    pub target: Option<String>,
    /// Git revision (default HEAD).
    pub git_ref: String,
    /// Loopback `host:port`.
    pub bind: String,
    /// Open the preview URL in a browser.
    pub open: bool,
}

/// A git directory `rgit view` can read.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedRepo {
    /// Working tree or bare repo path (`git -C` here).
    pub path: PathBuf,
    /// `owner/name` or directory name shown in the site.
    pub display_name: String,
}

/// Output of [`generate_site`].
#[derive(Clone, Debug)]
pub struct GeneratedSite {
    /// Zola site root.
    pub site_dir: PathBuf,
    /// Title used in `zola.toml`.
    pub display_name: String,
    /// Git paths written as blob pages.
    pub blob_paths: Vec<String>,
    /// Whether home used a README file.
    pub readme: Option<String>,
}

/// Resolve the tree, generate a Zola site, and run `zola serve` on loopback.
pub async fn run(config: &Config, opts: Options) -> Result<()> {
    if !git::git_on_path() {
        bail!("rgit view needs git on PATH");
    }
    if !zola_on_path() {
        bail!(
            "rgit view needs Zola 0.23.4 or newer on PATH (DevLab's floor).\n\
             Install: https://www.getzola.org/documentation/getting-started/installation/"
        );
    }
    let cwd = std::env::current_dir().context("current directory")?;
    let repo = resolve_repo(opts.target.as_deref(), &cwd, &config.root()).await?;
    let (host, port) = parse_loopback_bind(&opts.bind)?;
    let url = preview_url(&host, port);
    let sha = git::rev_parse(&repo.path, &opts.git_ref).await?;
    let short = short_sha(&sha);
    let site_dir = cache_root()?.join("view").join(format!(
        "{}-{short}",
        sanitize_component(&repo.display_name)
    ));
    std::fs::create_dir_all(&site_dir).with_context(|| format!("create {}", site_dir.display()))?;
    generate_site(
        &repo.path,
        &repo.display_name,
        &opts.git_ref,
        &url,
        &site_dir,
    )
    .await?;
    let theme_dir = cache_root()?.join("themes").join("devlab-theme");
    ensure_theme(&theme_dir).await?;
    link_theme(&site_dir, &theme_dir)?;
    eprintln!("Preview {} at {url}", repo.display_name);
    if opts.open {
        open_browser(&url);
    }
    run_zola_serve(&site_dir, &host, port)
}

/// Path that exists, else forge `owner/name.git`, else cwd.
pub async fn resolve_repo(
    target: Option<&str>,
    cwd: &Path,
    forge_root: &Path,
) -> Result<ResolvedRepo> {
    match target {
        Some(raw) => {
            let path = PathBuf::from(raw);
            let candidate = if path.is_absolute() {
                path
            } else {
                cwd.join(&path)
            };
            if candidate.exists() {
                if candidate.is_file() {
                    bail!(
                        "rgit view needs a repository directory, not a file ({})",
                        candidate.display()
                    );
                }
                return inspect_repo(&candidate).await;
            }
            if let Ok(name) = RepoName::parse(raw) {
                let bare = forge_root.join("repos").join(name.dir_name());
                if bare.exists() {
                    return inspect_repo(&bare).await;
                }
                bail!(
                    "repository {name} not found at {}\n\
                     Clone it first, then run rgit view in the working copy.",
                    bare.display()
                );
            }
            bail!("rgit view expected a git path or owner/name, got {raw:?}");
        }
        None => inspect_repo(cwd).await,
    }
}

/// Write Zola content and overlays under `site_dir` (theme is not required).
pub async fn generate_site(
    repo: &Path,
    display_name: &str,
    git_ref: &str,
    base_url: &str,
    site_dir: &Path,
) -> Result<GeneratedSite> {
    let sha = git::rev_parse(repo, git_ref).await?;
    let commit = commit_info(repo, &sha).await?;
    let entries = list_blobs(repo, &sha).await?;
    let clone_url = git::git_stdout(repo, &["remote", "get-url", "origin"])
        .await
        .ok()
        .filter(|url| !url.is_empty());

    let content = site_dir.join("content");
    if content.exists() {
        std::fs::remove_dir_all(&content)
            .with_context(|| format!("clear {}", content.display()))?;
    }
    std::fs::create_dir_all(content.join("tree"))
        .with_context(|| format!("create {}", content.display()))?;

    write_overlays(site_dir)?;
    write_zola_toml(site_dir, display_name, base_url)?;

    let readme = pick_readme(&entries);
    let readme_body = if let Some(path) = readme {
        match load_text_blob(repo, &sha, path).await {
            Ok(Some(text)) => Some((path.to_string(), text)),
            _ => None,
        }
    } else {
        None
    };
    write_home(
        &content.join("_index.md"),
        display_name,
        clone_url.as_deref(),
        &commit,
        readme_body.as_ref().map(|(_, body)| body.as_str()),
    )?;

    let mut dirs = BTreeSet::new();
    dirs.insert(String::new());
    let mut blob_paths = Vec::new();
    for entry in &entries {
        if skip_path(&entry.path) || entry.size > MAX_BLOB_BYTES {
            continue;
        }
        collect_dirs(&entry.path, &mut dirs);
        match write_blob_page(repo, &sha, site_dir, entry).await {
            Ok(true) => blob_paths.push(entry.path.clone()),
            Ok(false) => {}
            Err(err) => {
                tracing::warn!("skip {}: {err:#}", entry.path);
            }
        }
    }
    for dir in dirs {
        write_tree_section(site_dir, &dir)?;
    }

    Ok(GeneratedSite {
        site_dir: site_dir.to_path_buf(),
        display_name: display_name.to_string(),
        blob_paths,
        readme: readme_body.map(|(path, _)| path),
    })
}

/// True when a git path should not appear in the tree.
pub fn skip_path(path: &str) -> bool {
    path.split('/').any(|part| SKIP_COMPONENTS.contains(&part))
}

/// True when bytes should not be dumped as text.
pub fn is_binary(bytes: &[u8], path: &str) -> bool {
    binary_extension(path) || bytes.contains(&0) || std::str::from_utf8(bytes).is_err()
}

/// Fence language for a git path.
pub fn language_for(path: &str) -> String {
    let ext = Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("");
    match ext {
        "rs" => "rust".into(),
        "toml" => "toml".into(),
        "md" => "markdown".into(),
        "py" => "python".into(),
        "js" => "javascript".into(),
        "ts" => "typescript".into(),
        "tsx" => "tsx".into(),
        "jsx" => "jsx".into(),
        "json" => "json".into(),
        "yml" | "yaml" => "yaml".into(),
        "sh" | "bash" => "bash".into(),
        "html" | "htm" => "html".into(),
        "css" => "css".into(),
        "scss" => "scss".into(),
        "svg" => "xml".into(),
        "xml" => "xml".into(),
        "go" => "go".into(),
        "c" | "h" => "c".into(),
        "cc" | "cpp" | "cxx" | "hpp" => "cpp".into(),
        "java" => "java".into(),
        "kt" => "kotlin".into(),
        "rb" => "ruby".into(),
        "php" => "php".into(),
        "sql" => "sql".into(),
        "lock" => "toml".into(),
        "" => "text".into(),
        other => other.to_ascii_lowercase(),
    }
}

/// Escape Tera tags so Zola does not interpret repository text as templates.
pub fn escape_tera(body: &str) -> String {
    body.replace("{{", "{&lbrace;").replace("{%", "{&percnt;")
}

struct BlobEntry {
    path: String,
    size: u64,
}

struct CommitInfo {
    sha: String,
    subject: String,
    author: String,
    date: String,
}

async fn inspect_repo(path: &Path) -> Result<ResolvedRepo> {
    git::git_stdout(path, &["rev-parse", "--git-dir"])
        .await
        .with_context(|| format!("{} is not a git repository", path.display()))?;
    let path =
        std::fs::canonicalize(path).with_context(|| format!("canonicalize {}", path.display()))?;
    let work = git::git_stdout(&path, &["rev-parse", "--show-toplevel"])
        .await
        .ok()
        .map(PathBuf::from);
    let path = work.unwrap_or(path);
    let mut display_name = display_from_path(&path);
    if let Ok(url) = git::git_stdout(&path, &["remote", "get-url", "origin"]).await {
        if let Some(name) = name_from_clone_url(&url) {
            display_name = name;
        }
    }
    Ok(ResolvedRepo { path, display_name })
}

fn display_from_path(path: &Path) -> String {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("repository");
    let stripped = name.strip_suffix(".git").unwrap_or(name);
    if let Some(owner) = path
        .parent()
        .and_then(|parent| parent.file_name())
        .and_then(|name| name.to_str())
    {
        if name.ends_with(".git") && RepoName::parse(&format!("{owner}/{stripped}")).is_ok() {
            return format!("{owner}/{stripped}");
        }
    }
    stripped.to_string()
}

fn name_from_clone_url(url: &str) -> Option<String> {
    let url = url.trim();
    let path = if let Some(rest) = url.strip_prefix("ssh://") {
        rest.split_once('/')?.1
    } else if let Some((left, right)) = url.split_once(':') {
        if right.contains('/') && !left.contains("://") {
            right
        } else {
            url
        }
    } else {
        url
    };
    let path = path.trim_start_matches('/').trim_end_matches('/');
    if let Ok(name) = RepoName::parse(path) {
        return Some(name.to_string());
    }
    let path = Path::new(path);
    let name = path.file_name()?.to_str()?;
    let owner = path.parent()?.file_name()?.to_str()?;
    RepoName::parse(&format!("{owner}/{name}"))
        .ok()
        .map(|n| n.to_string())
}

async fn commit_info(repo: &Path, sha: &str) -> Result<CommitInfo> {
    let line = git::git_stdout(
        repo,
        &[
            "log",
            "-1",
            "--date=short",
            "--format=%h\t%s\t%an\t%ad",
            sha,
        ],
    )
    .await?;
    let mut parts = line.splitn(4, '\t');
    let short = parts.next().unwrap_or(sha).to_string();
    let subject = parts.next().unwrap_or("").to_string();
    let author = parts.next().unwrap_or("").to_string();
    let date = parts.next().unwrap_or("").to_string();
    Ok(CommitInfo {
        sha: short,
        subject,
        author,
        date,
    })
}

async fn list_blobs(repo: &Path, sha: &str) -> Result<Vec<BlobEntry>> {
    let out = git::git_stdout(repo, &["ls-tree", "-r", "--long", sha]).await?;
    let mut entries = Vec::new();
    for line in out.lines() {
        let Some((meta, path)) = line.split_once('\t') else {
            continue;
        };
        let mut bits = meta.split_whitespace();
        let _mode = bits.next();
        let kind = bits.next().unwrap_or("");
        let _oid = bits.next();
        let size = bits.next().unwrap_or("0").parse().unwrap_or(0);
        if kind != "blob" || path.is_empty() {
            continue;
        }
        entries.push(BlobEntry {
            path: path.to_string(),
            size,
        });
    }
    Ok(entries)
}

fn pick_readme(entries: &[BlobEntry]) -> Option<&str> {
    README_CANDIDATES
        .iter()
        .find(|name| {
            entries
                .iter()
                .any(|entry| entry.path == **name && entry.size <= MAX_BLOB_BYTES)
        })
        .copied()
}

async fn load_text_blob(repo: &Path, sha: &str, path: &str) -> Result<Option<String>> {
    let spec = format!("{sha}:{path}");
    let bytes = git::cat_file_bytes(repo, &spec).await?;
    if is_binary(&bytes, path) {
        return Ok(None);
    }
    Ok(Some(String::from_utf8_lossy(&bytes).into_owned()))
}

fn collect_dirs(path: &str, dirs: &mut BTreeSet<String>) {
    let mut prefix = String::new();
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() < 2 {
        return;
    }
    for part in &parts[..parts.len() - 1] {
        if !prefix.is_empty() {
            prefix.push('/');
        }
        prefix.push_str(part);
        dirs.insert(prefix.clone());
    }
}

fn write_home(
    path: &Path,
    display_name: &str,
    clone_url: Option<&str>,
    commit: &CommitInfo,
    readme: Option<&str>,
) -> Result<()> {
    let mut extra = format!(
        "[extra]\nrepo_name = {}\ncommit_sha = {}\ncommit_subject = {}\ncommit_author = {}\ncommit_date = {}\n",
        toml_str(display_name),
        toml_str(&commit.sha),
        toml_str(&commit.subject),
        toml_str(&commit.author),
        toml_str(&commit.date),
    );
    if let Some(url) = clone_url {
        extra.push_str(&format!("clone_url = {}\n", toml_str(url)));
    }
    let body = match readme {
        Some(text) => escape_tera(text),
        None => "This tree has no README.md.\n".into(),
    };
    let page = format!(
        "+++\ntitle = {}\ndescription = {}\n{extra}+++\n\n{body}",
        toml_str(display_name),
        toml_str(&format!("Local preview of {display_name}")),
    );
    std::fs::write(path, page).with_context(|| format!("write {}", path.display()))
}

fn write_tree_section(site_dir: &Path, dir: &str) -> Result<()> {
    let rel = if dir.is_empty() {
        PathBuf::from("content/tree/_index.md")
    } else {
        PathBuf::from("content/tree").join(dir).join("_index.md")
    };
    let path = site_dir.join(&rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let title = if dir.is_empty() {
        "Files".to_string()
    } else {
        dir.rsplit('/').next().unwrap_or(dir).to_string()
    };
    let description = if dir.is_empty() {
        "Repository file tree".to_string()
    } else {
        format!("{dir}/")
    };
    let page = format!(
        "+++\ntitle = {}\ndescription = {}\nsort_by = \"title\"\ntemplate = \"section.html\"\npage_template = \"page.html\"\n+++\n",
        toml_str(&title),
        toml_str(&description),
    );
    std::fs::write(&path, page).with_context(|| format!("write {}", path.display()))
}

async fn write_blob_page(
    repo: &Path,
    sha: &str,
    site_dir: &Path,
    entry: &BlobEntry,
) -> Result<bool> {
    let spec = format!("{}:{}", sha, entry.path);
    let bytes = git::cat_file_bytes(repo, &spec).await?;
    let file_name = entry.path.rsplit('/').next().unwrap_or(entry.path.as_str());
    let rel = blob_content_rel(&entry.path);
    let path = site_dir.join(&rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let permalink = format!("tree/{}", entry.path);
    let mut body = String::new();
    if is_binary(&bytes, &entry.path) {
        body.push_str(&format!("Binary file ({} bytes). Not shown.\n", entry.size));
    } else {
        let text = String::from_utf8_lossy(&bytes);
        if is_markdown_path(&entry.path) {
            body.push_str(&escape_tera(&text));
        } else {
            let fence = fence_for(&text);
            let lang = language_for(&entry.path);
            body.push_str(&format!(
                "{fence}{lang}\n{}\n{fence}\n",
                escape_tera(&text).trim_end()
            ));
        }
    }
    let page = format!(
        "+++\ntitle = {}\ndescription = {}\nslug = {}\npath = {}\ntemplate = \"page.html\"\n+++\n\n{body}",
        toml_str(file_name),
        toml_str(&entry.path),
        toml_str(file_name),
        toml_str(&permalink),
    );
    std::fs::write(&path, page).with_context(|| format!("write {}", path.display()))?;
    Ok(true)
}

fn blob_content_rel(git_path: &str) -> PathBuf {
    let mut out = PathBuf::from("content/tree");
    let parts: Vec<&str> = git_path.split('/').collect();
    for (i, part) in parts.iter().enumerate() {
        let last = i + 1 == parts.len();
        if last {
            if *part == "_index.md" || *part == "_index" {
                out.push("__index.md.md");
            } else {
                out.push(format!("{part}.md"));
            }
        } else {
            out.push(part);
        }
    }
    out
}

fn is_markdown_path(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
}

fn binary_extension(path: &str) -> bool {
    matches!(
        Path::new(path)
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase())
            .as_deref(),
        Some(
            "png"
                | "jpg"
                | "jpeg"
                | "gif"
                | "webp"
                | "ico"
                | "pdf"
                | "wasm"
                | "zip"
                | "gz"
                | "tgz"
                | "bz2"
                | "xz"
                | "7z"
                | "so"
                | "dylib"
                | "dll"
                | "exe"
                | "o"
                | "a"
                | "woff"
                | "woff2"
                | "ttf"
                | "otf"
                | "bin"
                | "class"
                | "jar"
        )
    )
}

fn fence_for(body: &str) -> String {
    let mut longest = 2;
    let mut cur = 0;
    for c in body.chars() {
        if c == '`' {
            cur += 1;
            longest = longest.max(cur);
        } else {
            cur = 0;
        }
    }
    "`".repeat(longest + 1)
}

fn write_overlays(site_dir: &Path) -> Result<()> {
    write_embed(&site_dir.join("templates/base.html"), OVERLAY_BASE)?;
    write_embed(&site_dir.join("templates/index.html"), OVERLAY_INDEX)?;
    write_embed(&site_dir.join("templates/section.html"), OVERLAY_SECTION)?;
    write_embed(&site_dir.join("templates/page.html"), OVERLAY_PAGE)?;
    write_embed(
        &site_dir.join("templates/partials/repo-crumbs.html"),
        OVERLAY_CRUMBS,
    )?;
    write_embed(
        &site_dir.join("templates/partials/theme-init.html"),
        OVERLAY_THEME_INIT,
    )?;
    write_embed(&site_dir.join("static/custom.css"), OVERLAY_CSS)?;
    write_embed(&site_dir.join("static/repo.js"), OVERLAY_JS)?;
    write_embed(&site_dir.join("static/brand/rabun.svg"), OVERLAY_MARK)?;
    Ok(())
}

fn write_embed(path: &Path, contents: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    std::fs::write(path, contents).with_context(|| format!("write {}", path.display()))
}

fn write_zola_toml(site_dir: &Path, display_name: &str, base_url: &str) -> Result<()> {
    let title = toml_str(display_name);
    let url = toml_str(base_url.trim_end_matches('/'));
    let note = toml_str("Loopback preview. Not served by rabun-git serve.");
    let contents = format!(
        r#"base_url = {url}
title = {title}
description = {title}
theme = "devlab-theme"
compile_sass = true

[markdown.highlighting]
light_theme = "github-light"
dark_theme = "github-dark"

[extra.brand]
id = "rabun"

[extra.devlab.brand]
logo_text = {title}
footer_text = {title}
show_logo_mark = true
logo_mark_path = "/brand/rabun.svg"

[extra.devlab.footer]
show = true
note = {note}

[extra.devlab.navigation]
links = [
  {{ name = "Home", path = "/" }},
  {{ name = "Browse", path = "/tree/" }},
]

[extra.devlab.appearance]
default_mode = "system"
show_toggle = true

[extra.devlab.search]
enabled = false
"#
    );
    std::fs::write(site_dir.join("zola.toml"), contents)
        .with_context(|| format!("write {}/zola.toml", site_dir.display()))
}

fn toml_str(value: &str) -> String {
    let escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', " ");
    format!("\"{escaped}\"")
}

fn parse_loopback_bind(bind: &str) -> Result<(String, u16)> {
    let addr: SocketAddr = bind
        .trim()
        .parse()
        .with_context(|| format!("view bind {bind} must be host:port"))?;
    if !addr.ip().is_loopback() {
        bail!("rgit view only binds loopback (127.0.0.1 or ::1), got {bind}");
    }
    Ok((addr.ip().to_string(), addr.port()))
}

fn preview_url(host: &str, port: u16) -> String {
    if host.contains(':') {
        format!("http://[{host}]:{port}/")
    } else {
        format!("http://{host}:{port}/")
    }
}

fn cache_root() -> Result<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CACHE_HOME") {
        let xdg = xdg.trim();
        if !xdg.is_empty() {
            return Ok(PathBuf::from(xdg).join("rabun-git"));
        }
    }
    let home = std::env::var("HOME").context("HOME is unset; cannot cache rgit view")?;
    Ok(PathBuf::from(home).join(".cache/rabun-git"))
}

fn sanitize_component(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
            out.push(c);
        } else {
            out.push('-');
        }
    }
    if out.is_empty() {
        "repo".into()
    } else {
        out
    }
}

fn short_sha(sha: &str) -> &str {
    let end = sha.len().min(12);
    &sha[..end]
}

fn zola_on_path() -> bool {
    std::process::Command::new("zola")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

async fn ensure_theme(dest: &Path) -> Result<()> {
    if dest.join(".git").exists() {
        if let Ok(head) = git::git_stdout(dest, &["rev-parse", "HEAD"]).await {
            if head.starts_with(THEME_COMMIT) || THEME_COMMIT.starts_with(&head) {
                return Ok(());
            }
        }
        std::fs::remove_dir_all(dest)
            .with_context(|| format!("replace theme cache {}", dest.display()))?;
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    eprintln!("Fetching DevLab {THEME_TAG} into {}", dest.display());
    git::git_global(&[
        "clone",
        "--branch",
        THEME_TAG,
        "--depth",
        "1",
        THEME_URL,
        dest.to_str().context("theme path")?,
    ])
    .await
    .with_context(|| {
        format!("clone DevLab {THEME_TAG} (first rgit view needs network to fetch the theme)")
    })?;
    Ok(())
}

fn link_theme(site_dir: &Path, theme_dir: &Path) -> Result<()> {
    let themes = site_dir.join("themes");
    std::fs::create_dir_all(&themes).with_context(|| format!("create {}", themes.display()))?;
    let link = themes.join("devlab-theme");
    if link.exists() || link.symlink_metadata().is_ok() {
        let meta = link.symlink_metadata().ok();
        if meta.as_ref().is_some_and(|m| m.file_type().is_symlink()) {
            std::fs::remove_file(&link).with_context(|| format!("remove {}", link.display()))?;
        } else if link.is_dir() {
            std::fs::remove_dir_all(&link).with_context(|| format!("remove {}", link.display()))?;
        } else if link.exists() {
            std::fs::remove_file(&link).with_context(|| format!("remove {}", link.display()))?;
        }
    }
    let target = std::fs::canonicalize(theme_dir)
        .with_context(|| format!("canonicalize {}", theme_dir.display()))?;
    std::os::unix::fs::symlink(&target, &link)
        .with_context(|| format!("symlink {} -> {}", link.display(), target.display()))
}

fn open_browser(url: &str) {
    let _ = std::process::Command::new("xdg-open")
        .arg(url)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

fn run_zola_serve(site_dir: &Path, host: &str, port: u16) -> Result<()> {
    let status = std::process::Command::new("zola")
        .current_dir(site_dir)
        .args(["serve", "--interface", host, "--port", &port.to_string()])
        .status()
        .context("spawn zola serve")?;
    if !status.success() {
        bail!("zola serve exited {}", status.code().unwrap_or(1));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::process::Command;

    async fn git_in(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .current_dir(dir)
            .args(args)
            .status()
            .await
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    async fn seed_work(root: &Path) -> PathBuf {
        let work = root.join("src");
        std::fs::create_dir_all(work.join("src")).unwrap();
        std::fs::create_dir_all(work.join("target/debug")).unwrap();
        std::fs::write(
            work.join("README.md"),
            "# Hello\n\nSee `{{name}}` and `{% if %}`.\n",
        )
        .unwrap();
        std::fs::write(work.join("src/cli.rs"), "fn main() {}\n").unwrap();
        std::fs::write(work.join("src/notes.md"), "notes\n").unwrap();
        std::fs::write(work.join("target/debug/app"), "skip me\n").unwrap();
        std::fs::write(work.join("icon.png"), [0x89, b'P', b'N', b'G', 0]).unwrap();
        git_in(&work, &["init", "-b", "master"]).await;
        git_in(&work, &["config", "user.email", "t@t"]).await;
        git_in(&work, &["config", "user.name", "t"]).await;
        git_in(&work, &["add", "."]).await;
        git_in(&work, &["commit", "-m", "init"]).await;
        work
    }

    #[test]
    fn skip_and_binary_helpers() {
        assert!(skip_path("target/debug/app"));
        assert!(skip_path("node_modules/pkg/index.js"));
        assert!(!skip_path("src/cli.rs"));
        assert!(is_binary(&[0, 1, 2], "blob.bin"));
        assert!(is_binary(b"ok", "photo.png"));
        assert!(!is_binary(b"fn main() {}", "src/cli.rs"));
        assert_eq!(language_for("src/cli.rs"), "rust");
        assert_eq!(
            escape_tera("{{name}} {% if %}"),
            "{&lbrace;name}} {&percnt; if %}"
        );
        assert_eq!(
            blob_content_rel("src/cli.rs"),
            PathBuf::from("content/tree/src/cli.rs.md")
        );
        assert_eq!(
            blob_content_rel("docs/_index.md"),
            PathBuf::from("content/tree/docs/__index.md.md")
        );
    }

    #[test]
    fn clone_url_names() {
        assert_eq!(
            name_from_clone_url("ssh://git@git.example.com:2222/ada/website.git").as_deref(),
            Some("ada/website")
        );
        assert_eq!(
            name_from_clone_url("git@git.example.com:ada/website.git").as_deref(),
            Some("ada/website")
        );
        assert_eq!(
            name_from_clone_url("/var/lib/rabun-git/repos/ada/website.git").as_deref(),
            Some("ada/website")
        );
    }

    #[test]
    fn loopback_only() {
        let (host, port) = parse_loopback_bind("127.0.0.1:1111").unwrap();
        assert_eq!(host, "127.0.0.1");
        assert_eq!(port, 1111);
        assert!(parse_loopback_bind("0.0.0.0:1111").is_err());
    }

    #[tokio::test]
    async fn resolve_path_and_forge_name() {
        if !git::git_on_path() {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let work = seed_work(tmp.path()).await;
        let resolved = resolve_repo(Some(work.to_str().unwrap()), tmp.path(), tmp.path())
            .await
            .unwrap();
        assert_eq!(resolved.path, std::fs::canonicalize(&work).unwrap());

        let missing = resolve_repo(Some("ada/website"), tmp.path(), tmp.path()).await;
        assert!(missing.unwrap_err().to_string().contains("not found"));

        let forge = tmp.path().join("forge");
        let bare = forge.join("repos/ada/website.git");
        std::fs::create_dir_all(bare.parent().unwrap()).unwrap();
        git::git_global(&[
            "clone",
            "--bare",
            work.to_str().unwrap(),
            bare.to_str().unwrap(),
        ])
        .await
        .unwrap();
        let named = resolve_repo(Some("ada/website"), tmp.path(), &forge)
            .await
            .unwrap();
        assert_eq!(named.display_name, "ada/website");
    }

    #[tokio::test]
    async fn generate_readme_tree_and_skips() {
        if !git::git_on_path() {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let work = seed_work(tmp.path()).await;
        git_in(
            &work,
            &[
                "remote",
                "add",
                "origin",
                "ssh://git@git.example.com:2222/ada/website.git",
            ],
        )
        .await;
        let site = tmp.path().join("site");
        let generated = generate_site(
            &work,
            "ada/website",
            "HEAD",
            "http://127.0.0.1:1111/",
            &site,
        )
        .await
        .unwrap();
        assert_eq!(generated.readme.as_deref(), Some("README.md"));
        assert!(generated.blob_paths.iter().any(|p| p == "src/cli.rs"));
        assert!(generated.blob_paths.iter().any(|p| p == "src/notes.md"));
        assert!(!generated
            .blob_paths
            .iter()
            .any(|p| p.starts_with("target/")));

        let home = std::fs::read_to_string(site.join("content/_index.md")).unwrap();
        assert!(home.contains("[extra]"));
        assert!(home.contains("repo_name = \"ada/website\""));
        assert!(home.contains("clone_url = \"ssh://git@git.example.com:2222/ada/website.git\""));
        assert!(home.contains("{&lbrace;name}}"));
        assert!(home.contains("{&percnt; if %}"));

        let blob = std::fs::read_to_string(site.join("content/tree/src/cli.rs.md")).unwrap();
        assert!(blob.contains("path = \"tree/src/cli.rs\""));
        assert!(blob.contains("```rust") || blob.contains("rust"));
        assert!(blob.contains("fn main()"));

        let notes = std::fs::read_to_string(site.join("content/tree/src/notes.md.md")).unwrap();
        assert!(notes.contains("notes"));
        assert!(site.join("content/tree/_index.md").exists());
        assert!(site.join("content/tree/src/_index.md").exists());
        assert!(!site.join("content/tree/target/_index.md").exists());

        let toml = std::fs::read_to_string(site.join("zola.toml")).unwrap();
        assert!(toml.contains("theme = \"devlab-theme\""));
        assert!(toml.contains("name = \"Browse\""));
        assert!(toml.contains("path = \"/tree/\""));
        let index = std::fs::read_to_string(site.join("templates/index.html")).unwrap();
        assert!(index.contains("Browse files"));
        assert!(index.contains("/tree/"));
        assert!(index.contains("data-copy-url"));
        assert!(index.contains("repo-commit-sha"));
        assert!(site.join("templates/section.html").exists());
        assert!(site.join("templates/page.html").exists());
        assert!(site.join("templates/partials/repo-crumbs.html").exists());
        assert!(site.join("static/custom.css").exists());
        assert!(site.join("static/repo.js").exists());
    }

    #[tokio::test]
    #[ignore = "needs zola and a cached DevLab theme"]
    async fn zola_builds_generated_site() {
        if !git::git_on_path() || !zola_on_path() {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let work = seed_work(tmp.path()).await;
        let site = tmp.path().join("site");
        generate_site(
            &work,
            "ada/website",
            "HEAD",
            "http://127.0.0.1:1111/",
            &site,
        )
        .await
        .unwrap();
        let theme = cache_root()
            .map(|root| root.join("themes").join("devlab-theme"))
            .unwrap_or_else(|_| tmp.path().join("themes/devlab-theme"));
        if !theme.join(".git").exists() {
            ensure_theme(&theme).await.unwrap();
        }
        link_theme(&site, &theme).unwrap();
        let status = std::process::Command::new("zola")
            .current_dir(&site)
            .args(["build"])
            .status()
            .unwrap();
        assert!(status.success());
        assert!(site.join("public/index.html").exists());
        assert!(site.join("public/tree/index.html").exists());
    }
}
