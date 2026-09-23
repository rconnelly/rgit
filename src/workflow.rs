//! In-repo YAML workflows (`.rabun/workflows/*.yml`).

use anyhow::{Context, Result};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_yml::Value;

use crate::git;
use crate::names::RepoName;
use crate::runner;
use crate::store::Store;

/// Why a workflow is being considered.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Event {
    /// Branch push (`refs/heads/*`).
    Push,
    /// Tag push (`refs/tags/*`).
    Tag,
    /// Merge request opened or updated.
    Request,
}

impl Event {
    /// Trigger name in YAML (`push` / `tag` / `request`).
    pub fn as_str(self) -> &'static str {
        match self {
            Event::Push => "push",
            Event::Tag => "tag",
            Event::Request => "request",
        }
    }
}

/// One workflow file.
#[derive(Clone, Debug, Deserialize)]
pub struct Workflow {
    /// Display name.
    #[serde(default)]
    pub name: String,
    /// Triggers.
    #[serde(rename = "on")]
    pub on: On,
    /// Jobs keyed by id (run sequentially in file order).
    #[serde(default)]
    pub jobs: IndexMap<String, Job>,
    /// Extra environment for every step.
    #[serde(default)]
    pub env: IndexMap<String, String>,
    /// Default per-job timeout (minutes).
    #[serde(default)]
    pub timeout_minutes: Option<u64>,
}

/// `on:` block.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct On {
    /// Branch pushes.
    #[serde(default)]
    pub push: Option<PushOn>,
    /// Any tag push when the key is present and not `false`.
    #[serde(default, deserialize_with = "present_flag")]
    pub tag: bool,
    /// Merge request events when the key is present and not `false`.
    #[serde(default, deserialize_with = "present_flag")]
    pub request: bool,
}

fn present_flag<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    Ok(!matches!(value, Value::Bool(false)))
}

/// `on.push`.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct PushOn {
    /// If empty, all branches match.
    #[serde(default)]
    pub branches: Vec<String>,
}

/// One job.
#[derive(Clone, Debug, Deserialize)]
pub struct Job {
    /// Shell steps (`run:` only).
    #[serde(default)]
    pub steps: Vec<Step>,
    /// Override workflow timeout.
    #[serde(default)]
    pub timeout_minutes: Option<u64>,
    /// Extra env for this job.
    #[serde(default)]
    pub env: IndexMap<String, String>,
    /// Builder labels (`linux`, `macos`, `windows`). Empty = forge host.
    #[serde(default, rename = "runs-on", deserialize_with = "runs_on_labels")]
    pub runs_on: Vec<String>,
    /// `sh`, `bash`, or `pwsh`. Empty = default for the label.
    #[serde(default)]
    pub shell: Option<String>,
}

fn runs_on_labels<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    match value {
        Value::String(label) => {
            let label = label.trim();
            if label.is_empty() {
                Ok(Vec::new())
            } else {
                Ok(vec![label.to_string()])
            }
        }
        Value::Sequence(seq) => {
            let mut out = Vec::new();
            for item in seq {
                if let Value::String(label) = item {
                    let label = label.trim();
                    if !label.is_empty() {
                        out.push(label.to_string());
                    }
                }
            }
            Ok(out)
        }
        _ => Ok(Vec::new()),
    }
}

impl Job {
    /// Labels to fan out. Empty means run on the forge host.
    pub fn runs_on_labels(&self) -> Vec<String> {
        self.runs_on.clone()
    }

    /// Shell for `label` (`windows` → `pwsh`, else `sh`) unless `shell:` is set.
    pub fn shell_for(&self, label: &str) -> String {
        if let Some(shell) = &self.shell {
            let shell = shell.trim();
            if !shell.is_empty() {
                return shell.to_string();
            }
        }
        if label == "windows" {
            "pwsh".into()
        } else {
            "sh".into()
        }
    }
}

/// A single `run:` step.
#[derive(Clone, Debug, Deserialize)]
pub struct Step {
    /// Command passed to `sh -c`.
    pub run: String,
}

/// Parse a workflow document.
pub fn parse(yaml: &str) -> Result<Workflow> {
    serde_yml::from_str(yaml).context("parse workflow yaml")
}

impl Workflow {
    /// Whether this workflow should run for `event` / `branch` (branch is `None` for tags).
    pub fn matches(&self, event: Event, branch: Option<&str>) -> bool {
        match event {
            Event::Push => {
                let Some(push) = &self.on.push else {
                    return false;
                };
                match branch {
                    None => true,
                    Some(name) => {
                        push.branches.is_empty() || push.branches.iter().any(|b| b == name)
                    }
                }
            }
            Event::Tag => self.on.tag,
            Event::Request => self.on.request,
        }
    }

    /// Timeout for `job`.
    pub fn timeout_for(&self, job: &Job) -> u64 {
        job.timeout_minutes
            .or(self.timeout_minutes)
            .unwrap_or(30)
            .max(1)
    }
}

/// Infer event + branch from a git ref.
pub fn event_for_ref(refname: &str) -> Option<(Event, Option<String>)> {
    if let Some(branch) = refname.strip_prefix("refs/heads/") {
        return Some((Event::Push, Some(branch.to_string())));
    }
    if refname.starts_with("refs/tags/") {
        return Some((Event::Tag, None));
    }
    if refname.contains("/requests/") && refname.ends_with("/head") {
        return Some((Event::Request, None));
    }
    None
}

/// Load `.rabun/workflows/*.{yml,yaml}` at `sha` and run matching jobs.
pub async fn trigger(
    store: &Store,
    repo: &RepoName,
    event: Event,
    sha: &str,
    request_id: u64,
    branch: Option<&str>,
) -> Result<()> {
    let path = store.repo_path(repo);
    let files = git::ls_tree_prefix(&path, sha, ".rabun/workflows").await?;
    for file in files {
        if !(file.ends_with(".yml") || file.ends_with(".yaml")) {
            continue;
        }
        let Some(text) = git::show_path(&path, sha, &file).await? else {
            continue;
        };
        let wf = match parse(&text) {
            Ok(wf) => wf,
            Err(err) => {
                tracing::warn!("skip {file} in {repo}: {err:#}");
                continue;
            }
        };
        if wf.matches(event, branch) {
            runner::run_workflow(store, repo, &wf, &file, event, sha, request_id, branch).await?;
        }
    }
    Ok(())
}

/// Trigger from a changed ref (push/tag/request head).
pub async fn trigger_ref(store: &Store, repo: &RepoName, refname: &str, sha: &str) -> Result<()> {
    let Some((event, branch)) = event_for_ref(refname) else {
        return Ok(());
    };
    trigger(store, repo, event, sha, 0, branch.as_deref()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_workflow() {
        let yaml = r#"
name: ci
on:
  push:
    branches: [master]
  tag:
  request:
jobs:
  test:
    steps:
      - run: cargo test --locked
"#;
        let wf = parse(yaml).unwrap();
        assert_eq!(wf.name, "ci");
        assert!(wf.matches(Event::Push, Some("master")));
        assert!(!wf.matches(Event::Push, Some("dev")));
        assert!(wf.matches(Event::Tag, None));
        assert!(wf.matches(Event::Request, None));
        assert_eq!(wf.jobs["test"].steps[0].run, "cargo test --locked");
        assert!(wf.jobs["test"].runs_on.is_empty());
    }

    #[test]
    fn parse_runs_on_string_and_list() {
        let one = parse(
            "name: ci\non:\n  push:\n    branches: [master]\njobs:\n  t:\n    runs-on: macos\n    steps:\n      - run: true\n",
        )
        .unwrap();
        assert_eq!(one.jobs["t"].runs_on_labels(), vec!["macos"]);
        assert_eq!(one.jobs["t"].shell_for("macos"), "sh");
        let many = parse(
            "name: ci\non:\n  push:\n    branches: [master]\njobs:\n  t:\n    runs-on: [linux, macos, windows]\n    shell: bash\n    steps:\n      - run: true\n",
        )
        .unwrap();
        assert_eq!(
            many.jobs["t"].runs_on_labels(),
            vec!["linux", "macos", "windows"]
        );
        assert_eq!(many.jobs["t"].shell_for("windows"), "bash");
    }
}
