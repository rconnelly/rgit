//! Trusted local workflow runner: worktree + `sh -c` with a timeout.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::process::Command;

use crate::git;
use crate::names::RepoName;
use crate::now_iso;
use crate::store::{self, Store};
use crate::workflow::{Event, Job, Workflow};

/// Persisted run status (`runs/<owner>/<name>/<id>/status.yaml`).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunStatus {
    /// UUID.
    pub id: String,
    /// Workflow `name:` or file stem.
    pub workflow: String,
    /// Job id.
    pub job: String,
    /// `push`, `tag`, or `request`.
    pub event: String,
    /// Commit SHA.
    pub sha: String,
    /// Git ref when known.
    #[serde(default)]
    pub git_ref: String,
    /// `queued`, `running`, `passed`, `failed`.
    pub status: String,
    /// RFC 3339.
    pub started_at: String,
    /// RFC 3339 when finished.
    #[serde(default)]
    pub finished_at: String,
    /// Builder label (`linux` / `macos` / `windows`). Empty = forge host.
    #[serde(default)]
    pub runs_on: String,
    /// Builder that claimed the job.
    #[serde(default)]
    pub agent: String,
}

/// Payload written beside `status.yaml` for queued / agent jobs.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobSpec {
    /// Run id (same as directory name).
    pub id: String,
    /// `owner/name`.
    pub repo: String,
    /// Commit SHA.
    pub sha: String,
    /// Git ref when known.
    #[serde(default)]
    pub git_ref: String,
    /// Workflow name.
    pub workflow: String,
    /// Job id from YAML.
    pub job: String,
    /// `push`, `tag`, or `request`.
    pub event: String,
    /// Builder label.
    pub runs_on: String,
    /// `sh`, `bash`, or `pwsh`.
    pub shell: String,
    /// Merged workflow + job env.
    #[serde(default)]
    pub env: indexmap::IndexMap<String, String>,
    /// `run:` scripts in order.
    pub steps: Vec<String>,
    /// Timeout in minutes.
    pub timeout_minutes: u64,
}

/// Execute every matching job in `workflow` at `sha`.
#[allow(clippy::too_many_arguments)]
pub async fn run_workflow(
    store: &Store,
    repo: &RepoName,
    workflow: &Workflow,
    file: &str,
    event: Event,
    sha: &str,
    _request_id: u64,
    branch: Option<&str>,
) -> Result<()> {
    let git_ref = match event {
        Event::Push => branch
            .map(|b| format!("refs/heads/{b}"))
            .unwrap_or_default(),
        Event::Tag => String::new(),
        Event::Request => String::from("refs/rabun/requests"),
    };
    let workflow_name = if workflow.name.is_empty() {
        file.to_string()
    } else {
        workflow.name.clone()
    };
    for (job_id, job) in &workflow.jobs {
        let labels = job.runs_on_labels();
        let targets = if labels.is_empty() {
            vec![String::new()]
        } else {
            labels
        };
        for label in targets {
            enqueue_or_run(
                store,
                repo,
                workflow,
                job,
                job_id,
                &workflow_name,
                event,
                sha,
                &git_ref,
                &label,
            )
            .await?;
        }
    }
    Ok(())
}

/// Queue for a builder, or run on the forge when `runs-on` is empty / local linux.
#[allow(clippy::too_many_arguments)]
async fn enqueue_or_run(
    store: &Store,
    repo: &RepoName,
    workflow: &Workflow,
    job: &Job,
    job_id: &str,
    workflow_name: &str,
    event: Event,
    sha: &str,
    git_ref: &str,
    label: &str,
) -> Result<()> {
    let run_id = uuid::Uuid::new_v4().to_string();
    let dir = store.runs_dir(repo).join(&run_id);
    std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    let spec = JobSpec {
        id: run_id.clone(),
        repo: repo.to_string(),
        sha: sha.to_string(),
        git_ref: git_ref.to_string(),
        workflow: workflow_name.to_string(),
        job: job_id.to_string(),
        event: event.as_str().to_string(),
        runs_on: label.to_string(),
        shell: job.shell_for(label),
        env: {
            let mut env = workflow.env.clone();
            env.extend(job.env.clone());
            env
        },
        steps: job.steps.iter().map(|step| step.run.clone()).collect(),
        timeout_minutes: workflow.timeout_for(job),
    };
    write_job_spec(&dir, &spec)?;
    let local = should_run_local(store, label);
    let mut status = RunStatus {
        id: run_id,
        workflow: workflow_name.to_string(),
        job: job_id.to_string(),
        event: event.as_str().to_string(),
        sha: sha.to_string(),
        git_ref: git_ref.to_string(),
        status: if local { "running" } else { "queued" }.into(),
        started_at: now_iso(),
        finished_at: String::new(),
        runs_on: label.to_string(),
        agent: String::new(),
    };
    write_status(&dir, &status)?;
    if !local {
        return Ok(());
    }
    let timeout = Duration::from_secs(spec.timeout_minutes.saturating_mul(60));
    let result = run_job(store, repo, job, sha, &dir, timeout).await;
    status.finished_at = now_iso();
    match result {
        Ok(()) => status.status = "passed".into(),
        Err(err) => {
            status.status = "failed".into();
            append_log(&dir, &format!("ERROR: {err:#}\n"))?;
            tracing::warn!("workflow {workflow_name} job {job_id} failed: {err:#}");
        }
    }
    write_status(&dir, &status)?;
    Ok(())
}

fn should_run_local(store: &Store, label: &str) -> bool {
    if label.is_empty() {
        return true;
    }
    label == "linux" && !store.has_builder_label(label)
}

async fn run_job(
    store: &Store,
    repo: &RepoName,
    job: &Job,
    sha: &str,
    run_dir: &Path,
    timeout: Duration,
) -> Result<()> {
    let bare = store.repo_path(repo);
    let work = run_dir.join("work");
    git::git(
        &bare,
        &["worktree", "add", "--detach", &work.to_string_lossy(), sha],
    )
    .await?;
    let spec = load_job_spec(run_dir)?;
    let outcome = run_steps(
        &spec.shell,
        &spec.env,
        &job.steps.iter().map(|s| s.run.as_str()).collect::<Vec<_>>(),
        &work,
        run_dir,
        timeout,
    )
    .await;
    let _ = git::git(
        &bare,
        &["worktree", "remove", "--force", &work.to_string_lossy()],
    )
    .await;
    let _ = std::fs::remove_dir_all(&work);
    outcome
}

/// Run `run:` steps with `shell` in `work`.
pub async fn run_steps(
    shell: &str,
    env: &indexmap::IndexMap<String, String>,
    steps: &[&str],
    work: &Path,
    run_dir: &Path,
    timeout: Duration,
) -> Result<()> {
    for (i, step) in steps.iter().enumerate() {
        append_log(run_dir, &format!("$ {step}\n"))?;
        let mut cmd = shell_command(shell, step);
        cmd.current_dir(work)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (k, v) in env {
            cmd.env(k, v);
        }
        let output = tokio::time::timeout(timeout, cmd.output())
            .await
            .context("step timed out")?
            .with_context(|| format!("spawn {shell}"))?;
        append_log(run_dir, &String::from_utf8_lossy(&output.stdout))?;
        append_log(run_dir, &String::from_utf8_lossy(&output.stderr))?;
        if !output.status.success() {
            anyhow::bail!(
                "step {} exited {}",
                i + 1,
                output.status.code().unwrap_or(1)
            );
        }
    }
    Ok(())
}

/// `sh -c`, `bash -c`, or `pwsh -NoProfile -Command`.
pub fn shell_command(shell: &str, script: &str) -> Command {
    match shell {
        "pwsh" => {
            let mut cmd = Command::new("pwsh");
            cmd.args(["-NoProfile", "-Command", script]);
            cmd
        }
        "bash" => {
            let mut cmd = Command::new("bash");
            cmd.args(["-c", script]);
            cmd
        }
        _ => {
            let mut cmd = Command::new("sh");
            cmd.args(["-c", script]);
            cmd
        }
    }
}

pub(crate) fn write_status(dir: &Path, status: &RunStatus) -> Result<()> {
    let yaml = serde_yml::to_string(status).context("serialize run status")?;
    store::atomic_write(&dir.join("status.yaml"), yaml.as_bytes())
}

pub(crate) fn write_job_spec(dir: &Path, spec: &JobSpec) -> Result<()> {
    let yaml = serde_yml::to_string(spec).context("serialize job spec")?;
    store::atomic_write(&dir.join("job.yaml"), yaml.as_bytes())
}

pub(crate) fn load_job_spec(dir: &Path) -> Result<JobSpec> {
    let path = dir.join("job.yaml");
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    serde_yml::from_str(&text).context("parse job.yaml")
}

pub(crate) fn load_status(dir: &Path) -> Result<RunStatus> {
    let path = dir.join("status.yaml");
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    serde_yml::from_str(&text).context("parse status.yaml")
}

pub(crate) fn append_log(dir: &Path, chunk: &str) -> Result<()> {
    use std::io::Write;
    let path = dir.join("log.txt");
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("open {}", path.display()))?;
    file.write_all(chunk.as_bytes())?;
    Ok(())
}

/// All runs on the forge (every repo), newest first.
pub fn list_all(store: &Store) -> Result<Vec<(RepoName, RunStatus, PathBuf)>> {
    let root = store.root().join("runs");
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for owner_ent in std::fs::read_dir(&root).with_context(|| format!("read {}", root.display()))? {
        let owner_ent = owner_ent?;
        if !owner_ent.file_type()?.is_dir() {
            continue;
        }
        let owner = owner_ent.file_name().to_string_lossy().into_owned();
        for name_ent in std::fs::read_dir(owner_ent.path())? {
            let name_ent = name_ent?;
            if !name_ent.file_type()?.is_dir() {
                continue;
            }
            let name = name_ent.file_name().to_string_lossy().into_owned();
            let Ok(repo) = RepoName::parse(&format!("{owner}/{name}")) else {
                continue;
            };
            for run_ent in std::fs::read_dir(name_ent.path())? {
                let run_ent = run_ent?;
                if !run_ent.file_type()?.is_dir() {
                    continue;
                }
                let dir = run_ent.path();
                if !dir.join("status.yaml").exists() {
                    continue;
                }
                if let Ok(status) = load_status(&dir) {
                    out.push((repo.clone(), status, dir));
                }
            }
        }
    }
    out.sort_by(|a, b| b.1.started_at.cmp(&a.1.started_at));
    Ok(out)
}

/// Directory for a run id (any repo).
pub fn find_run_dir(store: &Store, id: &str) -> Result<(RepoName, PathBuf)> {
    for (repo, status, dir) in list_all(store)? {
        if status.id == id {
            return Ok((repo, dir));
        }
    }
    anyhow::bail!("run {id} not found")
}

/// Oldest queued job whose `runs_on` is in `labels`.
pub fn next_queued(
    store: &Store,
    labels: &[String],
) -> Result<Option<(RepoName, PathBuf, RunStatus)>> {
    let mut matches: Vec<(RepoName, PathBuf, RunStatus)> = list_all(store)?
        .into_iter()
        .filter(|(_, status, _)| {
            status.status == "queued" && labels.iter().any(|l| l == &status.runs_on)
        })
        .map(|(repo, status, dir)| (repo, dir, status))
        .collect();
    matches.sort_by(|a, b| a.2.started_at.cmp(&b.2.started_at));
    Ok(matches.into_iter().next())
}

/// Claim `queued` → `running` for `agent`. Returns false if another agent won.
pub fn claim_run(dir: &Path, agent: &str) -> Result<bool> {
    let lock = dir.join("claimed");
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock)
    {
        Ok(_) => {}
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => return Ok(false),
        Err(err) => return Err(err).with_context(|| format!("claim {}", lock.display())),
    }
    let mut status = load_status(dir)?;
    if status.status != "queued" {
        let _ = std::fs::remove_file(&lock);
        return Ok(false);
    }
    status.status = "running".into();
    status.agent = agent.to_string();
    status.started_at = now_iso();
    write_status(dir, &status)?;
    Ok(true)
}

/// Runs for a repo, newest first.
pub fn list(store: &Store, repo: &RepoName) -> Result<Vec<RunStatus>> {
    let dir = store.runs_dir(repo);
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for ent in std::fs::read_dir(&dir).with_context(|| format!("read {}", dir.display()))? {
        let ent = ent?;
        if !ent.file_type()?.is_dir() {
            continue;
        }
        let status_path = ent.path().join("status.yaml");
        if !status_path.exists() {
            continue;
        }
        let text = std::fs::read_to_string(&status_path)?;
        if let Ok(status) = serde_yml::from_str::<RunStatus>(&text) {
            out.push(status);
        }
    }
    out.sort_by(|a, b| b.started_at.cmp(&a.started_at));
    Ok(out)
}

/// Load one run.
pub fn show(store: &Store, repo: &RepoName, id: &str) -> Result<RunStatus> {
    let path = store.runs_dir(repo).join(id).join("status.yaml");
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("run {id} not found in {repo}"))?;
    serde_yml::from_str(&text).context("parse run status")
}

/// Captured log text.
pub fn logs(store: &Store, repo: &RepoName, id: &str) -> Result<String> {
    let path = store.runs_dir(repo).join(id).join("log.txt");
    if !path.exists() {
        return Ok(String::new());
    }
    std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acl::Actor;
    use crate::repo;
    use crate::workflow;

    #[tokio::test]
    async fn runner_true_and_false() {
        if !git::git_on_path() {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path());
        store.ensure_layout().unwrap();
        store.add_user("alice", true).unwrap();
        let name = RepoName::parse("alice/app").unwrap();
        repo::create(&store, &Actor::Operator, &name).await.unwrap();
        let bare = store.repo_path(&name);
        let work = tmp.path().join("seed");
        std::fs::create_dir_all(work.join(".rabun/workflows")).unwrap();
        std::fs::write(
            work.join(".rabun/workflows/ci.yml"),
            "name: ci\non:\n  push:\n    branches: [master]\njobs:\n  ok:\n    steps:\n      - run: true\n",
        )
        .unwrap();
        git::git_global(&["init", "-b", "master", &work.to_string_lossy()])
            .await
            .unwrap();
        git::git(&work, &["config", "user.email", "t@t"])
            .await
            .unwrap();
        git::git(&work, &["config", "user.name", "t"])
            .await
            .unwrap();
        git::git(&work, &["add", "."]).await.unwrap();
        git::git(&work, &["commit", "-m", "init"]).await.unwrap();
        git::git(&work, &["remote", "add", "origin", &bare.to_string_lossy()])
            .await
            .unwrap();
        git::git(&work, &["push", "-u", "origin", "HEAD:master"])
            .await
            .unwrap();
        let sha = git::rev_parse(&bare, "HEAD").await.unwrap();
        let wf = workflow::parse(
            "name: ci\non:\n  push:\n    branches: [master]\njobs:\n  ok:\n    steps:\n      - run: true\n",
        )
        .unwrap();
        run_workflow(
            &store,
            &name,
            &wf,
            "ci.yml",
            Event::Push,
            &sha,
            0,
            Some("master"),
        )
        .await
        .unwrap();
        let runs = list(&store, &name).unwrap();
        assert_eq!(runs[0].status, "passed");

        let wf_fail = workflow::parse(
            "name: ci\non:\n  push:\n    branches: [master]\njobs:\n  bad:\n    steps:\n      - run: false\n",
        )
        .unwrap();
        run_workflow(
            &store,
            &name,
            &wf_fail,
            "ci.yml",
            Event::Push,
            &sha,
            0,
            Some("master"),
        )
        .await
        .unwrap();
        let runs = list(&store, &name).unwrap();
        assert!(runs.iter().any(|r| r.status == "failed"));
    }

    #[tokio::test]
    async fn runs_on_list_fans_out_and_queues() {
        if !git::git_on_path() {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path());
        store.ensure_layout().unwrap();
        store.add_user("alice", true).unwrap();
        store.upsert_builder("mac", &["macos".into()]).unwrap();
        let name = RepoName::parse("alice/app").unwrap();
        repo::create(&store, &Actor::Operator, &name).await.unwrap();
        let bare = store.repo_path(&name);
        let work = tmp.path().join("seed");
        std::fs::create_dir_all(&work).unwrap();
        git::git_global(&["init", "-b", "master", &work.to_string_lossy()])
            .await
            .unwrap();
        git::git(&work, &["config", "user.email", "t@t"])
            .await
            .unwrap();
        git::git(&work, &["config", "user.name", "t"])
            .await
            .unwrap();
        std::fs::write(work.join("README"), "x").unwrap();
        git::git(&work, &["add", "."]).await.unwrap();
        git::git(&work, &["commit", "-m", "init"]).await.unwrap();
        git::git(&work, &["remote", "add", "origin", &bare.to_string_lossy()])
            .await
            .unwrap();
        git::git(&work, &["push", "-u", "origin", "HEAD:master"])
            .await
            .unwrap();
        let sha = git::rev_parse(&bare, "HEAD").await.unwrap();
        let wf = workflow::parse(
            "name: ci\non:\n  push:\n    branches: [master]\njobs:\n  t:\n    runs-on: [linux, macos]\n    steps:\n      - run: true\n",
        )
        .unwrap();
        run_workflow(
            &store,
            &name,
            &wf,
            "ci.yml",
            Event::Push,
            &sha,
            0,
            Some("master"),
        )
        .await
        .unwrap();
        let runs = list(&store, &name).unwrap();
        assert_eq!(runs.len(), 2);
        let linux = runs.iter().find(|r| r.runs_on == "linux").unwrap();
        let macos = runs.iter().find(|r| r.runs_on == "macos").unwrap();
        assert_eq!(linux.status, "passed");
        assert_eq!(macos.status, "queued");
        let dir = store.runs_dir(&name).join(&macos.id);
        assert!(claim_run(&dir, "mac").unwrap());
        assert!(!claim_run(&dir, "other").unwrap());
        assert_eq!(show(&store, &name, &macos.id).unwrap().agent, "mac");
    }
}
