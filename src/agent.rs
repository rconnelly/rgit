//! Builder agents: register, claim queued jobs, poll loop on this machine.

use std::path::Path;
use std::time::Duration;

use anyhow::{bail, Context, Result};

use crate::acl::{self, Actor};
use crate::cli::AgentCommands;
use crate::names::RepoName;
use crate::now_iso;
use crate::remote::{self, ClientInvoke};
use crate::runner::{self, JobSpec};
use crate::store::Store;

/// `agent register|list|next|log|finish` against the store.
pub fn execute(store: &Store, actor: &Actor, command: AgentCommands) -> Result<String> {
    match command {
        AgentCommands::Register {
            name,
            labels,
            file,
            literal,
        } => {
            acl::require_forge_admin(store, actor)?;
            if store.load_users()?.by_name(&name).is_none() {
                store.add_user(&name, false)?;
            }
            store.upsert_builder(&name, &labels)?;
            if file.is_some() || literal.is_some() {
                let text = match (file, literal) {
                    (Some(path), None) => std::fs::read_to_string(&path)
                        .with_context(|| format!("read {}", path.display()))?,
                    (None, Some(text)) => text,
                    _ => bail!("agent register key needs --file or --literal, not both"),
                };
                store.add_keys(&name, &text)?;
            }
            Ok(format!("builder {name} registered\n"))
        }
        AgentCommands::List => {
            acl::require_forge_admin(store, actor)?;
            let file = store.load_builders()?;
            if file.builders.is_empty() {
                return Ok("(no builders)\n".into());
            }
            let mut lines = Vec::new();
            for b in file.builders {
                lines.push(format!("{} {}", b.name, b.labels.join(",")));
            }
            Ok(lines.join("\n") + "\n")
        }
        AgentCommands::Next { labels } => next(store, actor, &labels),
        AgentCommands::Log { run_id, literal } => {
            require_builder(store, actor)?;
            let text = match literal {
                Some(text) => text,
                None => {
                    let mut buf = String::new();
                    std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf)
                        .context("read agent log from stdin")?;
                    buf
                }
            };
            let (_, dir) = runner::find_run_dir(store, &run_id)?;
            require_run_agent(&dir, actor)?;
            runner::append_log(&dir, &text)?;
            Ok(String::new())
        }
        AgentCommands::Finish { run_id, status } => {
            require_builder(store, actor)?;
            if status != "passed" && status != "failed" {
                bail!("agent finish --status must be passed or failed");
            }
            let (_, dir) = runner::find_run_dir(store, &run_id)?;
            require_run_agent(&dir, actor)?;
            let mut run = runner::load_status(&dir)?;
            run.status = status.clone();
            run.finished_at = now_iso();
            runner::write_status(&dir, &run)?;
            Ok(format!("run {run_id} {status}\n"))
        }
    }
}

fn require_builder(store: &Store, actor: &Actor) -> Result<String> {
    let Some(name) = actor.name() else {
        bail!("only a registered builder can claim jobs");
    };
    if store.builder_by_name(name)?.is_none() {
        bail!("{name} is not a registered builder");
    }
    Ok(name.to_string())
}

fn require_run_agent(dir: &Path, actor: &Actor) -> Result<()> {
    let status = runner::load_status(dir)?;
    let Some(name) = actor.name() else {
        bail!("only the claiming builder can update this run");
    };
    if status.agent != name {
        bail!("run is claimed by {}, not {name}", status.agent);
    }
    Ok(())
}

fn next(store: &Store, actor: &Actor, labels: &[String]) -> Result<String> {
    let name = require_builder(store, actor)?;
    let builder = store
        .builder_by_name(&name)?
        .context("builder disappeared")?;
    let want = if labels.is_empty() {
        builder.labels.clone()
    } else {
        labels.to_vec()
    };
    if want.is_empty() {
        bail!("agent next needs --label or registered labels");
    }
    let Some((repo, dir, _)) = runner::next_queued(store, &want)? else {
        return Ok("(no jobs)\n".into());
    };
    if !runner::claim_run(&dir, &name)? {
        return Ok("(no jobs)\n".into());
    }
    let spec = runner::load_job_spec(&dir)?;
    let _ = repo;
    Ok(serde_yml::to_string(&spec)?)
}

/// Poll `remote` for jobs matching `labels` (this machine).
pub fn poll(labels: &[String], remote: &str) -> Result<()> {
    if labels.is_empty() {
        bail!("rgit agent --labels LABEL");
    }
    let remotes = remote::remotes_path();
    let Some(invoke) = remote::detect_invoke_from([remote], &remotes)? else {
        bail!("unknown remote {remote}; rgit remote add {remote} git@HOST");
    };
    eprintln!(
        "agent polling {} as {}@{}:{} labels={}",
        invoke.name,
        invoke.target.user,
        invoke.target.host,
        invoke.target.port,
        labels.join(",")
    );
    loop {
        match claim_once(&invoke, labels) {
            Ok(Some(spec)) => {
                if let Err(err) = execute_claimed(&invoke, &spec) {
                    eprintln!("job {} failed: {err:#}", spec.id);
                    let _ = finish(&invoke, &spec.id, "failed");
                }
            }
            Ok(None) => std::thread::sleep(Duration::from_secs(5)),
            Err(err) => {
                eprintln!("agent next: {err:#}");
                std::thread::sleep(Duration::from_secs(5));
            }
        }
    }
}

fn claim_once(invoke: &ClientInvoke, labels: &[String]) -> Result<Option<JobSpec>> {
    let mut payload = vec!["agent".into(), "next".into()];
    for label in labels {
        payload.push("--label".into());
        payload.push(label.clone());
    }
    let out = remote::ssh_capture(&invoke.target, invoke.identity.as_deref(), &payload)?;
    let out = out.trim();
    if out.is_empty() || out == "(no jobs)" {
        return Ok(None);
    }
    let spec: JobSpec = serde_yml::from_str(out).context("parse claimed job.yaml")?;
    Ok(Some(spec))
}

fn execute_claimed(invoke: &ClientInvoke, spec: &JobSpec) -> Result<()> {
    let repo = RepoName::parse(&spec.repo)?;
    let root = std::env::temp_dir().join(format!("rgit-agent-{}", spec.id));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).with_context(|| format!("create {}", root.display()))?;
    let work = root.join("work");
    let url = format!(
        "ssh://{}@{}:{}/{}.git",
        invoke.target.user, invoke.target.host, invoke.target.port, repo
    );
    let mut clone = std::process::Command::new("git");
    clone.args(["clone", "--quiet", &url]);
    clone.arg(&work);
    if let Some(identity) = &invoke.identity {
        clone.env(
            "GIT_SSH_COMMAND",
            format!(
                "ssh -i {} -p {} -o BatchMode=yes",
                identity.display(),
                invoke.target.port
            ),
        );
    }
    let status = clone.status().context("git clone")?;
    if !status.success() {
        bail!("git clone {url} failed");
    }
    git_cmd(&work, &["fetch", "--quiet", "origin", &spec.sha])?;
    git_cmd(&work, &["checkout", "--quiet", &spec.sha])?;
    let timeout = Duration::from_secs(spec.timeout_minutes.saturating_mul(60));
    let steps: Vec<&str> = spec.steps.iter().map(String::as_str).collect();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("tokio for agent steps")?;
    let result = rt.block_on(runner::run_steps(
        &spec.shell,
        &spec.env,
        &steps,
        &work,
        &root,
        timeout,
    ));
    let log = root.join("log.txt");
    if log.exists() {
        if let Ok(text) = std::fs::read_to_string(&log) {
            let _ = send_log(invoke, &spec.id, &text);
        }
    }
    let _ = std::fs::remove_dir_all(&root);
    match result {
        Ok(()) => finish(invoke, &spec.id, "passed"),
        Err(err) => {
            let _ = send_log(invoke, &spec.id, &format!("ERROR: {err:#}\n"));
            finish(invoke, &spec.id, "failed")
        }
    }
}

fn git_cmd(repo: &Path, args: &[&str]) -> Result<()> {
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .status()
        .with_context(|| format!("git {}", args.join(" ")))?;
    if !status.success() {
        bail!("git {} failed", args.join(" "));
    }
    Ok(())
}

fn send_log(invoke: &ClientInvoke, run_id: &str, text: &str) -> Result<()> {
    if text.is_empty() {
        return Ok(());
    }
    remote::ssh_capture(
        &invoke.target,
        invoke.identity.as_deref(),
        &[
            "agent".into(),
            "log".into(),
            run_id.into(),
            "--literal".into(),
            text.into(),
        ],
    )?;
    Ok(())
}

fn finish(invoke: &ClientInvoke, run_id: &str, status: &str) -> Result<()> {
    remote::ssh_capture(
        &invoke.target,
        invoke.identity.as_deref(),
        &[
            "agent".into(),
            "finish".into(),
            run_id.into(),
            "--status".into(),
            status.into(),
        ],
    )?;
    Ok(())
}
