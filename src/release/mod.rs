//! Working-tree SemVer 2.0, Conventional Commits, and changelog management.

pub(crate) mod changelog;
pub(crate) mod commit;
mod manifest;
mod policy;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use semver::Version;

use crate::cli::{VersionBump, VersionCommands, VersionHookCommands};
use crate::git;
use crate::version::{self, Bump};

/// Run a `rgit version` subcommand in the process working directory.
pub async fn run(command: VersionCommands) -> Result<String> {
    let cwd = std::env::current_dir().context("current directory")?;
    run_in(&cwd, command).await
}

/// Run a `rgit version` subcommand in `dir` (tests and the CLI).
pub async fn run_in(dir: &Path, command: VersionCommands) -> Result<String> {
    match command {
        VersionCommands::Show => show(dir).await,
        VersionCommands::Check { range } => check(dir, range.as_deref()).await,
        VersionCommands::Bump { level, to, dry_run } => {
            bump(dir, level, to.as_deref(), dry_run).await
        }
        VersionCommands::Changelog { from } => changelog_cmd(dir, from.as_deref()).await,
        VersionCommands::Release {
            level,
            to,
            dry_run,
            no_tag,
        } => release(dir, level, to.as_deref(), dry_run, no_tag).await,
        VersionCommands::Hook { command } => match command {
            VersionHookCommands::Install => hook_install(dir).await,
        },
    }
}

/// Forge `hooks/update`: optional Conventional Commits and SemVer tag policy.
pub async fn enforce_push(repo: &Path, refname: &str, old: &str, new: &str) -> Result<()> {
    if git::is_zero_oid(new) || refname.starts_with("refs/rabun/") {
        return Ok(());
    }
    let commit = if refname.starts_with("refs/tags/") {
        git::peel_commit(repo, new).await?
    } else if refname.starts_with("refs/heads/") {
        new.to_string()
    } else {
        return Ok(());
    };
    let Some(policy) = policy::load_from_tree(repo, &commit).await? else {
        return Ok(());
    };
    if refname.starts_with("refs/tags/") {
        if policy.enforce.tags {
            let name = refname.strip_prefix("refs/tags/").context("tag ref")?;
            let parsed = version::parse(name)?;
            let expected = version::tag_for(&parsed, &policy.tag_prefix);
            if name != expected {
                bail!(
                    "tag {name} must be {expected} (SemVer 2.0.0 with prefix {})",
                    policy.tag_prefix
                );
            }
            if policy.enforce.manifests {
                manifest::require_tree_version(repo, &commit, &parsed).await?;
            }
        }
        return Ok(());
    }
    if policy.enforce.commits {
        for sha in git::new_commits(repo, old, new).await? {
            let msg = git::commit_message(repo, &sha).await?;
            commit::validate(&msg)
                .with_context(|| format!("commit {sha} is not Conventional Commits 1.0.0"))?;
        }
    }
    Ok(())
}

struct Workspace {
    root: PathBuf,
    is_git: bool,
    policy: policy::Policy,
}

async fn open(dir: &Path) -> Result<Workspace> {
    let (root, is_git) = match git::work_tree_root(dir).await {
        Ok(root) => (root, true),
        Err(_) => (dir.to_path_buf(), false),
    };
    let policy = policy::load_file(&root)?;
    Ok(Workspace {
        root,
        is_git,
        policy,
    })
}

fn require_git(ws: &Workspace) -> Result<()> {
    if !ws.is_git {
        bail!("not a git repository");
    }
    Ok(())
}

async fn show(dir: &Path) -> Result<String> {
    let ws = open(dir).await?;
    let manifests = manifest::detect(&ws.root)?;
    let ver = manifests.agreed_version()?;
    let mut lines = vec![ver.to_string()];
    for path in manifests.paths() {
        lines.push(path.display().to_string());
    }
    let log = ws.root.join(&ws.policy.changelog);
    if log.exists() {
        let rel = ws.policy.changelog.clone();
        if !lines.iter().any(|l| l == &rel) {
            lines.push(rel);
        }
    }
    Ok(lines.join("\n") + "\n")
}

async fn check(dir: &Path, range: Option<&str>) -> Result<String> {
    let ws = open(dir).await?;
    require_git(&ws)?;
    let range = match range {
        Some(range) => range.to_string(),
        None => default_range(&ws).await?,
    };
    let shas = git::rev_list(&ws.root, &["--no-merges", "--reverse", &range]).await?;
    let mut n = 0;
    let mut errors = Vec::new();
    for sha in &shas {
        let msg = git::commit_message(&ws.root, sha).await?;
        match commit::classify(&msg) {
            Ok(commit::Message::Exempt | commit::Message::Conventional(_)) => n += 1,
            Err(err) => errors.push(format!("{sha}: {err}")),
        }
    }
    if !errors.is_empty() {
        bail!("{}", errors.join("\n"));
    }
    Ok(format!("ok: {n} commits ({range})\n"))
}

async fn bump(
    dir: &Path,
    level: Option<VersionBump>,
    to: Option<&str>,
    dry_run: bool,
) -> Result<String> {
    let ws = open(dir).await?;
    let manifests = manifest::detect(&ws.root)?;
    let planned = plan_version(&ws, &manifests, level, to).await?;
    let next = planned.next.to_string();
    if dry_run {
        return Ok(format!(
            "would bump {} -> {} ({})\n{}\n",
            planned.current,
            planned.next,
            planned.label(),
            display_paths(&manifests)
        ));
    }
    manifest::set_version(&ws.root, &manifests, &next)?;
    Ok(format!(
        "{} -> {} ({})\n{}\n",
        planned.current,
        planned.next,
        planned.label(),
        display_paths(&manifests)
    ))
}

async fn changelog_cmd(dir: &Path, from: Option<&str>) -> Result<String> {
    let ws = open(dir).await?;
    require_git(&ws)?;
    let from = match from {
        Some(from) => Some(from.to_string()),
        None => git::nearest_version_tag(&ws.root, &ws.policy.tag_prefix).await?,
    };
    let commits = load_commits(&ws, from.as_deref()).await?;
    Ok(changelog::render_notes(&commits))
}

async fn release(
    dir: &Path,
    level: Option<VersionBump>,
    to: Option<&str>,
    dry_run: bool,
    no_tag: bool,
) -> Result<String> {
    let ws = open(dir).await?;
    require_git(&ws)?;
    let manifests = manifest::detect(&ws.root)?;
    let planned = plan_version(&ws, &manifests, level, to).await?;
    let next = planned.next.to_string();
    let from = git::nearest_version_tag(&ws.root, &ws.policy.tag_prefix).await?;
    let commits = load_commits(&ws, from.as_deref()).await?;
    let generated = changelog::render_notes(&commits);
    let log_path = ws.root.join(&ws.policy.changelog);
    let existing = if log_path.exists() {
        std::fs::read_to_string(&log_path)
            .with_context(|| format!("read {}", log_path.display()))?
    } else {
        String::new()
    };
    let repo_url = repo_url(&ws).await;
    let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let next_log = changelog::apply_release(
        &existing,
        &next,
        &date,
        &generated,
        repo_url.as_deref(),
        &ws.policy.tag_prefix,
    );
    let tag = version::tag_for(&planned.next, &ws.policy.tag_prefix);
    if dry_run {
        return Ok(format!(
            "would release {} -> {} ({})\ntag {tag}\n{}\n{}\n{}",
            planned.current,
            planned.next,
            planned.label(),
            display_paths(&manifests),
            ws.policy.changelog,
            next_log
        ));
    }
    manifest::set_version(&ws.root, &manifests, &next)?;
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::write(&log_path, next_log).with_context(|| format!("write {}", log_path.display()))?;
    let mut add: Vec<String> = manifests
        .paths()
        .into_iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    add.push(ws.policy.changelog.clone());
    let mut args = vec!["add", "--"];
    for path in &add {
        args.push(path);
    }
    git::git(&ws.root, &args).await?;
    let message = format!("chore(release): {next}");
    git::git(&ws.root, &["commit", "-m", &message]).await?;
    if !no_tag {
        git::git(&ws.root, &["tag", "-a", &tag, "-m", &tag]).await?;
    }
    let mut out = format!(
        "{} -> {} ({})\ncommitted {message}\n",
        planned.current,
        planned.next,
        planned.label()
    );
    if !no_tag {
        out.push_str(&format!("tagged {tag}\n"));
    }
    Ok(out)
}

async fn hook_install(dir: &Path) -> Result<String> {
    let ws = open(dir).await?;
    require_git(&ws)?;
    let path = git::git_path(&ws.root, "hooks/commit-msg").await?;
    let bin = crate::repo::current_bin();
    let bin_display = bin.to_string_lossy();
    let quoted = shlex::try_quote(&bin_display)
        .map_err(|_| anyhow!("binary path cannot be quoted for a hook"))?;
    let script = format!("#!/bin/sh\nset -e\nexec {quoted} hook commit-msg \"$1\"\n");
    if path.exists() {
        let existing =
            std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        if !existing.contains("hook commit-msg") {
            bail!(
                "refusing to overwrite {} (not an rgit commit-msg hook)",
                path.display()
            );
        }
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::write(&path, script).with_context(|| format!("write {}", path.display()))?;
    let mut perms = std::fs::metadata(&path)?.permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&path, perms).with_context(|| format!("chmod {}", path.display()))?;
    Ok(format!("installed {}\n", path.display()))
}

struct Planned {
    current: Version,
    next: Version,
    bump: Option<Bump>,
}

impl Planned {
    fn label(&self) -> String {
        match self.bump {
            Some(bump) => bump.as_str().to_string(),
            None => format!("to {}", self.next),
        }
    }
}

async fn plan_version(
    ws: &Workspace,
    manifests: &manifest::Manifests,
    level: Option<VersionBump>,
    to: Option<&str>,
) -> Result<Planned> {
    let current = manifests.agreed_version()?;
    if let Some(to) = to {
        let next = version::parse(to)?;
        if next <= current {
            bail!("{next} is not greater than {current}");
        }
        return Ok(Planned {
            current,
            next,
            bump: None,
        });
    }
    let bump = match level.unwrap_or(VersionBump::Auto) {
        VersionBump::Auto => {
            require_git(ws)?;
            let from = git::nearest_version_tag(&ws.root, &ws.policy.tag_prefix).await?;
            let commits = load_commits(ws, from.as_deref()).await?;
            commit::inferred_bump(&commits, &current).ok_or_else(|| {
                anyhow!(
                    "no feat/fix/perf/breaking commits since last tag; pass patch, minor, major, or --to"
                )
            })?
        }
        VersionBump::Patch => Bump::Patch,
        VersionBump::Minor => Bump::Minor,
        VersionBump::Major => Bump::Major,
    };
    Ok(Planned {
        next: version::bump(&current, bump),
        current,
        bump: Some(bump),
    })
}

async fn default_range(ws: &Workspace) -> Result<String> {
    match git::nearest_version_tag(&ws.root, &ws.policy.tag_prefix).await? {
        Some(tag) => Ok(format!("{tag}..HEAD")),
        None => Ok("HEAD".into()),
    }
}

async fn load_commits(ws: &Workspace, from: Option<&str>) -> Result<Vec<commit::Commit>> {
    let range = match from {
        Some(tag) => format!("{tag}..HEAD"),
        None => "HEAD".to_string(),
    };
    let shas = git::rev_list(&ws.root, &["--no-merges", "--reverse", &range]).await?;
    let mut out = Vec::new();
    for sha in shas {
        let msg = git::commit_message(&ws.root, &sha).await?;
        match commit::classify(&msg) {
            Ok(commit::Message::Exempt) => {}
            Ok(commit::Message::Conventional(c)) => out.push(c),
            Err(_) => {}
        }
    }
    Ok(out)
}

async fn repo_url(ws: &Workspace) -> Option<String> {
    if let Ok(Some(url)) = git::remote_url(&ws.root, "origin").await {
        if let Some(https) = changelog::https_repo_url(&url) {
            return Some(https);
        }
    }
    let cargo = ws.root.join("Cargo.toml");
    if let Ok(text) = std::fs::read_to_string(cargo) {
        if let Ok(doc) = text.parse::<toml_edit::DocumentMut>() {
            if let Some(url) = doc
                .get("package")
                .and_then(|item| item.get("repository"))
                .and_then(|item| item.as_str())
            {
                return changelog::https_repo_url(url);
            }
        }
    }
    None
}

fn display_paths(manifests: &manifest::Manifests) -> String {
    manifests
        .paths()
        .into_iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bump_and_release_in_temp_repo() {
        if !git::git_on_path() {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nrepository = \"https://github.com/acme/demo\"\n",
        )
        .unwrap();
        git::git_global(&["init", "-b", "master", &root.to_string_lossy()])
            .await
            .unwrap();
        git::git(root, &["config", "user.email", "t@t"])
            .await
            .unwrap();
        git::git(root, &["config", "user.name", "t"]).await.unwrap();
        git::git(root, &["add", "."]).await.unwrap();
        git::git(root, &["commit", "-m", "feat: initial"])
            .await
            .unwrap();

        let out = run_in(root, VersionCommands::Show).await.unwrap();
        assert!(out.starts_with("0.1.0\n"));

        let out = run_in(
            root,
            VersionCommands::Release {
                level: None,
                to: None,
                dry_run: true,
                no_tag: false,
            },
        )
        .await
        .unwrap();
        assert!(out.contains("would release 0.1.0 -> 0.2.0"));

        run_in(
            root,
            VersionCommands::Release {
                level: None,
                to: None,
                dry_run: false,
                no_tag: false,
            },
        )
        .await
        .unwrap();
        let cargo = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
        assert!(cargo.contains("version = \"0.2.0\""));
        let log = std::fs::read_to_string(root.join("CHANGELOG.md")).unwrap();
        assert!(log.contains("## [0.2.0]"));
        assert!(log.contains("feat: initial") || log.contains("- initial"));
        let tag = git::git_stdout(root, &["tag", "-l"]).await.unwrap();
        assert!(tag.contains("v0.2.0"));
        let msg = git::git_stdout(root, &["log", "-1", "--format=%s"])
            .await
            .unwrap();
        assert_eq!(msg, "chore(release): 0.2.0");
    }
}
