//! Trusted local workflow runner: worktree + `sh -c` with a timeout.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::process::Command;

use crate::git;
use crate::names::RepoName;
use crate::now_iso;
use crate::store::{self, Store};
use crate::workflow::{Event, Workflow};

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
    for (job_id, job) in &workflow.jobs {
        let run_id = uuid::Uuid::new_v4().to_string();
        let dir = store.runs_dir(repo).join(&run_id);
        std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
        let mut status = RunStatus {
            id: run_id.clone(),
            workflow: if workflow.name.is_empty() {
                file.to_string()
            } else {
                workflow.name.clone()
            },
            job: job_id.clone(),
            event: event.as_str().to_string(),
            sha: sha.to_string(),
            git_ref: git_ref.clone(),
            status: "running".into(),
            started_at: now_iso(),
            finished_at: String::new(),
        };
        write_status(&dir, &status)?;
        let timeout = Duration::from_secs(workflow.timeout_for(job).saturating_mul(60));
        let result = run_job(store, repo, workflow, job, sha, &dir, timeout).await;
        status.finished_at = now_iso();
        match result {
            Ok(()) => status.status = "passed".into(),
            Err(err) => {
                status.status = "failed".into();
                append_log(&dir, &format!("ERROR: {err:#}\n"))?;
                tracing::warn!("workflow {} job {job_id} failed: {err:#}", workflow.name);
            }
        }
        write_status(&dir, &status)?;
    }
    Ok(())
}

async fn run_job(
    store: &Store,
    repo: &RepoName,
    workflow: &Workflow,
    job: &crate::workflow::Job,
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
    let outcome = run_steps(workflow, job, &work, run_dir, timeout).await;
    let _ = git::git(
        &bare,
        &["worktree", "remove", "--force", &work.to_string_lossy()],
    )
    .await;
    let _ = std::fs::remove_dir_all(&work);
    outcome
}

async fn run_steps(
    workflow: &Workflow,
    job: &crate::workflow::Job,
    work: &Path,
    run_dir: &Path,
    timeout: Duration,
) -> Result<()> {
    for (i, step) in job.steps.iter().enumerate() {
        append_log(run_dir, &format!("$ {}\n", step.run))?;
        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            .arg(&step.run)
            .current_dir(work)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (k, v) in &workflow.env {
            cmd.env(k, v);
        }
        for (k, v) in &job.env {
            cmd.env(k, v);
        }
        let output = tokio::time::timeout(timeout, cmd.output())
            .await
            .context("step timed out")?
            .context("spawn sh")?;
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

fn write_status(dir: &Path, status: &RunStatus) -> Result<()> {
    let yaml = serde_yml::to_string(status).context("serialize run status")?;
    store::atomic_write(&dir.join("status.yaml"), yaml.as_bytes())
}

fn append_log(dir: &Path, chunk: &str) -> Result<()> {
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
}
