//! `rabun.companion/v1` heartbeat: atomic `status.json` and loopback `GET /health`.

use std::io::Write;
use std::net::{SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::config::Config;
use crate::now_iso;
use crate::version;

/// JSON schema id for companion heartbeats.
pub const SCHEMA: &str = "rabun.companion/v1";
/// App id in Rabun `[[apps]]`.
pub const APP_NAME: &str = "git";
/// systemd unit beside `rabun.service`.
pub const UNIT_NAME: &str = "rabun-git";
/// Default loopback bind for long-running `serve`.
pub const DEFAULT_HEALTH_BIND: &str = "127.0.0.1:8792";

/// Shared snapshot served by `/health`.
pub type StatusHandle = Arc<RwLock<CompanionStatus>>;

/// Companion heartbeat (`rabun.companion/v1`).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CompanionStatus {
    /// `"rabun.companion/v1"`.
    pub schema: String,
    /// `true` when serve is running without a recorded error.
    pub ok: bool,
    /// App id (`git`).
    pub name: String,
    /// Binary name.
    pub command: String,
    /// Crate version.
    pub version: String,
    /// systemd unit when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    /// Process id of this serve.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    /// Seconds since this process started.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uptime_seconds: Option<u64>,
    /// When this snapshot was written (RFC 3339).
    pub checked_at: String,
    /// Last successful or idle activity.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activity: Option<CompanionActivity>,
    /// Last error, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<CompanionError>,
    /// Loopback health URL this process serves.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health_url: Option<String>,
}

/// Last serve activity (no secrets).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CompanionActivity {
    /// One-line summary for the dashboard.
    pub summary: String,
    /// When that activity happened (RFC 3339).
    pub last_at: String,
    /// Paths and bind facts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Map<String, Value>>,
}

/// Last error from serve.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CompanionError {
    /// When the error was recorded (RFC 3339).
    pub at: String,
    /// Operator-facing message (no secrets).
    pub message: String,
}

impl CompanionStatus {
    /// Snapshot for a process that is not serving.
    pub fn offline(config: &Config) -> Self {
        let checked_at = now_iso();
        Self {
            schema: SCHEMA.into(),
            ok: false,
            name: APP_NAME.into(),
            command: env!("CARGO_PKG_NAME").into(),
            version: version::crate_version().into(),
            unit: Some(UNIT_NAME.into()),
            pid: None,
            uptime_seconds: None,
            checked_at: checked_at.clone(),
            activity: Some(CompanionActivity {
                summary: "not running".into(),
                last_at: checked_at,
                details: Some(path_details(config)),
            }),
            error: None,
            health_url: health_url_for_bind(health_bind().as_deref()),
        }
    }

    /// Starting heartbeat before SSH accept.
    pub fn starting(config: &Config, started_at: Instant, health_url: Option<String>) -> Self {
        let checked_at = now_iso();
        Self {
            schema: SCHEMA.into(),
            ok: true,
            name: APP_NAME.into(),
            command: env!("CARGO_PKG_NAME").into(),
            version: version::crate_version().into(),
            unit: Some(UNIT_NAME.into()),
            pid: Some(std::process::id()),
            uptime_seconds: Some(started_at.elapsed().as_secs()),
            checked_at: checked_at.clone(),
            activity: Some(CompanionActivity {
                summary: "serve starting".into(),
                last_at: checked_at,
                details: Some(path_details(config)),
            }),
            error: None,
            health_url,
        }
    }
}

/// Default status file: `$RABUN_GIT_ROOT/status.json`.
pub fn status_file(config: &Config) -> PathBuf {
    if let Some(path) = nonempty_env("RABUN_GIT_STATUS_FILE") {
        return PathBuf::from(path);
    }
    config.root().join("status.json")
}

/// Loopback bind for `/health`. Unset → [`DEFAULT_HEALTH_BIND`]; empty or `off` disables.
pub fn health_bind() -> Option<String> {
    match std::env::var("RABUN_GIT_HEALTH_BIND") {
        Ok(value) => {
            let value = value.trim();
            if value.is_empty() || value.eq_ignore_ascii_case("off") {
                None
            } else {
                Some(value.to_string())
            }
        }
        Err(_) => Some(DEFAULT_HEALTH_BIND.into()),
    }
}

/// `http://{bind}/health` when bind is loopback host:port.
pub fn health_url_for_bind(bind: Option<&str>) -> Option<String> {
    let bind = bind?;
    Some(format!("http://{bind}/health"))
}

/// Write `status.json` atomically (temp file + rename).
pub fn write_status_file(path: &Path, status: &CompanionStatus) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let json = serde_json::to_vec_pretty(status).context("serialize companion status")?;
    let tmp = path.with_extension("json.tmp");
    {
        let mut file =
            std::fs::File::create(&tmp).with_context(|| format!("create {}", tmp.display()))?;
        file.write_all(&json)
            .with_context(|| format!("write {}", tmp.display()))?;
        file.sync_all()
            .with_context(|| format!("sync {}", tmp.display()))?;
    }
    std::fs::rename(&tmp, path).with_context(|| format!("rename {}", tmp.display()))?;
    Ok(())
}

/// Load a previously written heartbeat, if present.
pub fn read_status_file(path: &Path) -> Result<Option<CompanionStatus>> {
    if !path.is_file() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let status: CompanionStatus =
        serde_json::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
    Ok(Some(status))
}

/// Print the on-disk heartbeat or an offline snapshot (no secrets).
pub fn print_status(config: &Config) -> Result<()> {
    let path = status_file(config);
    let status = match read_status_file(&path)? {
        Some(status) => status,
        None => CompanionStatus::offline(config),
    };
    serde_json::to_writer_pretty(std::io::stdout(), &status).context("write status json")?;
    println!();
    Ok(())
}

/// Persist a snapshot to disk and to the in-memory `/health` handle.
pub fn publish(path: &Path, handle: &StatusHandle, status: CompanionStatus) -> Result<()> {
    write_status_file(path, &status)?;
    *write_handle(handle) = status;
    Ok(())
}

fn write_handle(handle: &StatusHandle) -> std::sync::RwLockWriteGuard<'_, CompanionStatus> {
    handle
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// `host:port` for loopback bind.
pub fn parse_health_addr(bind: &str) -> Result<SocketAddr> {
    bind.parse().with_context(|| {
        format!("health bind {bind} must be host:port (for example {DEFAULT_HEALTH_BIND})")
    })
}

fn bind_health_listener(bind: &str) -> Result<TcpListener> {
    let addr = parse_health_addr(bind)?;
    let listener = TcpListener::bind(addr).with_context(|| format!("bind {bind}"))?;
    listener
        .set_nonblocking(true)
        .context("health socket nonblocking")?;
    Ok(listener)
}

/// Serve `GET /health` on a dedicated thread.
pub fn spawn_health(bind: String, handle: StatusHandle, started_at: Instant) {
    if let Err(err) = std::thread::Builder::new()
        .name("rabun-git-health".into())
        .spawn(move || {
            if let Err(err) = run_health_thread(&bind, handle, started_at) {
                tracing::warn!("health server stopped: {err:#}");
            }
        })
    {
        tracing::warn!("health server skipped: {err:#}");
    }
}

fn run_health_thread(bind: &str, handle: StatusHandle, started_at: Instant) -> Result<()> {
    let listener = bind_health_listener(bind)?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .context("health runtime")?;
    rt.block_on(async move {
        tokio::select! {
            result = serve_health_listener(listener, handle.clone()) => result,
            _ = pulse_health(handle, started_at) => Ok(()),
        }
    })
}

async fn pulse_health(handle: StatusHandle, started_at: Instant) {
    let mut interval = tokio::time::interval(Duration::from_secs(15));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        interval.tick().await;
        let mut snap = write_handle(&handle);
        snap.uptime_seconds = Some(started_at.elapsed().as_secs());
        snap.checked_at = now_iso();
    }
}

async fn serve_health_listener(listener: TcpListener, handle: StatusHandle) -> Result<()> {
    let listener = tokio::net::TcpListener::from_std(listener).context("health tokio listener")?;
    let app = Router::new()
        .route("/health", get(health))
        .with_state(handle);
    axum::serve(listener, app).await.context("health serve")?;
    Ok(())
}

async fn health(State(handle): State<StatusHandle>) -> Json<CompanionStatus> {
    Json(
        handle
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone(),
    )
}

fn path_details(config: &Config) -> Map<String, Value> {
    let mut details = Map::new();
    details.insert(
        "root".into(),
        Value::String(config.root().display().to_string()),
    );
    details.insert("sshBind".into(), Value::String(config.ssh_bind.clone()));
    details
}

fn nonempty_env(var: &str) -> Option<String> {
    std::env::var(var)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}
