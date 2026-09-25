//! SSH git-upload-pack / git-receive-pack and management commands.

use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::Parser;
use russh::keys::ssh_key::LineEnding;
use russh::keys::{Algorithm, PrivateKey, PublicKey};
use russh::server::{Auth, ChannelOpenHandle, Handle, Handler, Msg, Server, Session};
use russh::{Channel, ChannelId, ChannelMsg};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::acl::{self, Actor, Role};
use crate::cli::Cli;
use crate::config::Config;
use crate::dispatch;
use crate::git;
use crate::names::RepoName;
use crate::repo;
use crate::status::{self, CompanionStatus, StatusHandle};
use crate::store::Store;

/// Default SSH bind (`host:port`).
pub const DEFAULT_SSH_BIND: &str = "0.0.0.0:2222";

/// Listen until SIGINT/SIGTERM.
pub async fn serve(config: &Config, bind: Option<String>) -> Result<()> {
    let store = Store::open(config.root());
    store.ensure_layout()?;
    let host_key = load_or_create_host_key(&store.host_key_path())?;
    let addr = config.ssh_bind(bind.as_deref())?;
    let started_at = Instant::now();
    let health = status::health_bind();
    let health_url = status::health_url_for_bind(health.as_deref());
    let handle: StatusHandle = Arc::new(std::sync::RwLock::new(CompanionStatus::starting(
        config,
        started_at,
        health_url.clone(),
    )));
    status::publish(
        &status::status_file(config),
        &handle,
        CompanionStatus::starting(config, started_at, health_url),
    )?;
    if let Some(bind) = health {
        status::spawn_health(bind, handle, started_at);
    }

    let ssh_config = russh::server::Config {
        inactivity_timeout: Some(Duration::from_secs(3600)),
        auth_rejection_time: Duration::from_secs(1),
        auth_rejection_time_initial: Some(Duration::from_secs(0)),
        keys: vec![host_key],
        ..Default::default()
    };
    let ssh_config = Arc::new(ssh_config);
    let socket = TcpListener::bind(addr)
        .await
        .with_context(|| format!("bind SSH {addr}"))?;
    tracing::info!("rabun-git SSH on {addr}");
    let mut server = GitServer { store };
    let running = server.run_on_socket(ssh_config, &socket);
    tokio::select! {
        result = running => result.context("SSH server")?,
        _ = shutdown_signal() => tracing::info!("shutting down"),
    }
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("SIGTERM handler");
        tokio::select! {
            _ = ctrl_c => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = ctrl_c.await;
    }
}

fn reject() -> Auth {
    Auth::Reject {
        proceed_with_methods: None,
        partial_success: false,
    }
}

fn load_or_create_host_key(path: &Path) -> Result<PrivateKey> {
    use std::os::unix::fs::PermissionsExt;

    if path.exists() {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("read host key {}", path.display()))?;
        return PrivateKey::from_openssh(&text)
            .map_err(|err| anyhow::anyhow!("parse host key {}: {err}", path.display()));
    }
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519)
        .map_err(|err| anyhow::anyhow!("generate host key: {err}"))?;
    let encoded = key
        .to_openssh(LineEnding::LF)
        .map_err(|err| anyhow::anyhow!("encode host key: {err}"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::write(path, encoded.as_bytes())
        .with_context(|| format!("write {}", path.display()))?;
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(0o600);
    let _ = std::fs::set_permissions(path, perms);
    Ok(key)
}

struct GitServer {
    store: Store,
}

impl Server for GitServer {
    type Handler = GitHandler;

    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> GitHandler {
        GitHandler {
            store: self.store.clone(),
            user: None,
            channel: None,
        }
    }
}

struct GitHandler {
    store: Store,
    user: Option<String>,
    channel: Option<Channel<Msg>>,
}

impl Handler for GitHandler {
    type Error = anyhow::Error;

    async fn auth_publickey(&mut self, user: &str, key: &PublicKey) -> Result<Auth, Self::Error> {
        let owner = self.store.user_for_key(key)?;
        let Some(owner) = owner else {
            return Ok(reject());
        };
        if user == "git" || user == owner {
            self.user = Some(owner);
            Ok(Auth::Accept)
        } else {
            Ok(reject())
        }
    }

    async fn auth_none(&mut self, _: &str) -> Result<Auth, Self::Error> {
        Ok(reject())
    }

    async fn auth_password(&mut self, _: &str, _: &str) -> Result<Auth, Self::Error> {
        Ok(reject())
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.channel = Some(channel);
        reply.accept().await;
        Ok(())
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let cmd = String::from_utf8_lossy(data).into_owned();
        let Some(ch) = self.channel.take() else {
            session.channel_failure(channel)?;
            return Ok(());
        };
        session.channel_success(channel)?;
        let store = self.store.clone();
        let user = self.user.clone();
        let handle = session.handle();
        tokio::spawn(async move {
            if let Err(err) = handle_exec(store, user, ch, cmd, handle).await {
                tracing::warn!("ssh exec: {err:#}");
            }
        });
        Ok(())
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let Some(ch) = self.channel.take() else {
            session.channel_failure(channel)?;
            return Ok(());
        };
        session.channel_success(channel)?;
        let store = self.store.clone();
        let user = self.user.clone().unwrap_or_else(|| "unknown".into());
        tokio::spawn(async move {
            let _ = send_text(ch, greeting(&store, &user), 0).await;
        });
        Ok(())
    }
}

fn greeting(store: &Store, user: &str) -> String {
    let repos = repo::list(store, &Actor::User(user.to_string())).unwrap_or_default();
    let mut s = format!("Hi {user}, this is rabun-git.\nAuthenticated over SSH.\n");
    if repos.is_empty() {
        s.push_str("No repositories yet.\n");
    } else {
        s.push_str("Repos:\n");
        for r in repos {
            s.push_str(&format!("  {r}\n"));
        }
    }
    s
}

async fn handle_exec(
    store: Store,
    user: Option<String>,
    channel: Channel<Msg>,
    cmd: String,
    _handle: Handle,
) -> Result<()> {
    let Some(user) = user else {
        return send_text(channel, "not authenticated\n".into(), 1).await;
    };
    if let Some((service, repo)) = parse_git_exec(&cmd) {
        return handle_git(&store, &user, channel, service, repo).await;
    }
    match parse_management(&cmd) {
        Ok(cli) => {
            if cli.command.ssh_forbidden() {
                return send_text(channel, "command not available over SSH\n".into(), 1).await;
            }
            match dispatch::execute_fmt(
                &store,
                &Actor::User(user),
                cli.command,
                cli.json,
                cli.token.as_deref(),
            )
            .await
            {
                Ok(out) => send_text(channel, out, 0).await,
                Err(err) => send_text(channel, format!("{err:#}\n"), 1).await,
            }
        }
        Err(err) => send_text(channel, format!("{err}\n"), 2).await,
    }
}

fn parse_management(cmd: &str) -> Result<Cli> {
    let split = shlex::split(cmd).ok_or_else(|| anyhow::anyhow!("invalid quoting"))?;
    let mut argv = vec!["rabun-git".to_string()];
    argv.extend(split);
    Cli::try_parse_from(&argv).map_err(|err| anyhow::anyhow!("{err}"))
}

/// Git SSH exec: upload-pack or receive-pack.
#[derive(Clone, Copy, Debug)]
enum GitService {
    Upload,
    Receive,
}

fn parse_git_exec(cmd: &str) -> Option<(GitService, RepoName)> {
    let cmd = cmd.trim();
    let (svc, rest) = if let Some(rest) = cmd.strip_prefix("git-upload-pack") {
        (GitService::Upload, rest)
    } else {
        let rest = cmd.strip_prefix("git-receive-pack")?;
        (GitService::Receive, rest)
    };
    let rest = rest.trim().trim_matches('\'').trim_matches('"').trim();
    let rest = rest.trim_start_matches('/');
    RepoName::parse(rest).ok().map(|name| (svc, name))
}

async fn handle_git(
    store: &Store,
    user: &str,
    channel: Channel<Msg>,
    service: GitService,
    repo: RepoName,
) -> Result<()> {
    let actor = Actor::User(user.to_string());
    let needed = match service {
        GitService::Upload => Role::Read,
        GitService::Receive => Role::Write,
    };
    if let Err(err) = acl::require(store, &actor, &repo, needed) {
        return send_text(channel, format!("{err:#}\n"), 1).await;
    }
    let path = store.repo_path(&repo);
    if !path.exists() {
        return send_text(channel, format!("repository {repo} not found\n"), 1).await;
    }
    let before = if matches!(service, GitService::Receive) {
        git::list_refs(&path).await.unwrap_or_default()
    } else {
        Default::default()
    };
    let sub = match service {
        GitService::Upload => "upload-pack",
        GitService::Receive => "receive-pack",
    };
    let mut child = git::command()
        .arg(sub)
        .arg(&path)
        .env("RABUN_GIT_USER", user)
        .env("RABUN_GIT_REPO", repo.to_string())
        .env("RABUN_GIT_ROOT", store.root())
        .env("RABUN_GIT_BIN", repo::current_bin())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("spawn git pack")?;
    let code = pipe_child(channel, &mut child).await?;
    if matches!(service, GitService::Receive) && code == 0 {
        if let Err(err) = dispatch::after_receive(store, user, &repo, before).await {
            tracing::warn!("after receive {repo}: {err:#}");
        }
    }
    Ok(())
}

async fn pipe_child(mut channel: Channel<Msg>, child: &mut tokio::process::Child) -> Result<u32> {
    let mut stdin = child.stdin.take();
    let mut stdout = child.stdout.take().context("git stdout")?;
    let mut stderr = child.stderr.take().context("git stderr")?;
    let mut out_buf = vec![0u8; 32 * 1024];
    let mut err_buf = vec![0u8; 8 * 1024];
    let mut stdout_done = false;
    let mut stderr_done = false;
    let mut channel_eof = false;
    loop {
        tokio::select! {
            msg = channel.wait(), if !channel_eof => {
                match msg {
                    Some(ChannelMsg::Data { ref data }) => {
                        if let Some(ref mut s) = stdin {
                            s.write_all(data).await?;
                        }
                    }
                    Some(ChannelMsg::Eof) | None => {
                        stdin.take();
                        channel_eof = true;
                    }
                    Some(ChannelMsg::Close) => {
                        stdin.take();
                        break;
                    }
                    _ => {}
                }
            }
            n = stdout.read(&mut out_buf), if !stdout_done => {
                match n {
                    Ok(0) => stdout_done = true,
                    Ok(n) => channel.data(&out_buf[..n]).await?,
                    Err(err) => return Err(err.into()),
                }
            }
            n = stderr.read(&mut err_buf), if !stderr_done => {
                match n {
                    Ok(0) => stderr_done = true,
                    Ok(n) => {
                        let _ = channel.extended_data(1, &err_buf[..n]).await;
                    }
                    Err(_) => stderr_done = true,
                }
            }
        }
        if stdout_done && stderr_done && channel_eof {
            break;
        }
        if stdout_done && stderr_done {
            // Git closed stdio; wait for the process even if the client keeps the channel.
            break;
        }
    }
    drop(stdin);
    let status = child.wait().await.context("wait git pack")?;
    let code = status.code().unwrap_or(1) as u32;
    let _ = channel.eof().await;
    let _ = channel.exit_status(code).await;
    let _ = channel.close().await;
    Ok(code)
}

async fn send_text(channel: Channel<Msg>, text: String, code: u32) -> Result<()> {
    if !text.is_empty() {
        channel.data(text.as_bytes()).await?;
    }
    channel.eof().await?;
    channel.exit_status(code).await?;
    channel.close().await?;
    Ok(())
}

/// Used by tests to parse git SSH exec lines.
pub fn parse_git_exec_for_test(cmd: &str) -> Option<String> {
    parse_git_exec(cmd).map(|(_, n)| n.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_git_commands() {
        assert_eq!(
            parse_git_exec_for_test("git-upload-pack 'alice/app.git'").as_deref(),
            Some("alice/app")
        );
        assert_eq!(
            parse_git_exec_for_test("git-receive-pack '/acme/lib'").as_deref(),
            Some("acme/lib")
        );
        assert!(parse_git_exec("request list alice/app").is_none());
    }
}
