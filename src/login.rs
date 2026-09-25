//! This-machine web sign-on (`rgit login`): generate a key, open the browser,
//! persist the identity on a named remote.

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use russh::keys::ssh_key::LineEnding;
use russh::keys::{Algorithm, PrivateKey};
use serde_json::json;

use crate::device::{DevicePoll, DeviceStart};
use crate::remote;

/// Inputs for `rgit login`.
pub struct LoginRequest {
    /// Remote alias (`origin`).
    pub remote: String,
    /// Git host (`rgit.rs` or `git@HOST`).
    pub host: Option<String>,
    /// rgit-web origin (`https://rgit.rs`).
    pub web: Option<String>,
    /// Existing private key; generated when missing.
    pub identity: Option<PathBuf>,
    /// Skip `xdg-open`.
    pub no_open: bool,
    /// Remotes file (tests override).
    pub remotes_file: PathBuf,
}

/// JSON HTTP used by login (injectable in tests).
pub trait DeviceHttp {
    /// POST JSON; returns status and parsed body.
    fn post_json(&self, url: &str, body: &serde_json::Value) -> Result<(u16, serde_json::Value)>;
}

/// `ureq` client for production.
pub struct UreqHttp;

impl DeviceHttp for UreqHttp {
    fn post_json(&self, url: &str, body: &serde_json::Value) -> Result<(u16, serde_json::Value)> {
        match ureq::post(url)
            .set("Content-Type", "application/json")
            .send_json(body.clone())
        {
            Ok(resp) => {
                let status = resp.status();
                let parsed: serde_json::Value = resp.into_json().context("parse rgit-web JSON")?;
                Ok((status, parsed))
            }
            Err(ureq::Error::Status(code, resp)) => {
                let parsed = resp
                    .into_json()
                    .unwrap_or_else(|_| json!({ "error": format!("HTTP {code}") }));
                Ok((code, parsed))
            }
            Err(err) => Err(anyhow::anyhow!("talk to rgit-web: {err}")),
        }
    }
}

/// Sign in through rgit-web and save the SSH identity.
pub fn run(req: LoginRequest) -> Result<String> {
    run_with(req, &UreqHttp)
}

/// Same as [`run`] with an injected HTTP client.
pub fn run_with(req: LoginRequest, http: &dyn DeviceHttp) -> Result<String> {
    let remote = req.remote.trim();
    if remote.is_empty() {
        bail!("remote name is required");
    }
    let existing = remote::get(&req.remotes_file, remote)?;
    let host_raw = req
        .host
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .or_else(|| existing.as_ref().map(|entry| entry.url.clone()))
        .ok_or_else(|| anyhow::anyhow!("pass --host HOST (for example rgit.rs)"))?;
    let target = remote::parse_url(&host_raw)?;
    let url = host_raw.clone();
    let web = req
        .web
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.trim_end_matches('/').to_string())
        .or_else(|| existing.as_ref().and_then(|entry| entry.web.clone()))
        .unwrap_or_else(|| default_web(&target.host));

    let identity = match req.identity {
        Some(path) => path,
        None => existing
            .as_ref()
            .and_then(|entry| entry.identity.clone())
            .unwrap_or_else(|| default_identity_path(remote)),
    };
    let public_key = ensure_identity(&identity)?;
    let hostname = default_hostname();

    let start_url = format!("{web}/api/auth/device/start");
    let (status, body) = http.post_json(
        &start_url,
        &json!({ "public_key": public_key, "hostname": hostname }),
    )?;
    if status != 200 {
        bail!(
            "device start failed: {}",
            body.get("error")
                .and_then(|v| v.as_str())
                .unwrap_or(&body.to_string())
        );
    }
    let started: DeviceStart = serde_json::from_value(body).context("parse device start")?;
    let verify = format!(
        "{web}{}?code={}",
        if started.verification_uri.starts_with('/') {
            started.verification_uri.clone()
        } else {
            format!("/{}", started.verification_uri)
        },
        started.user_code
    );
    eprintln!("First copy your one-time code: {}", started.user_code);
    eprintln!("Then open: {verify}");
    if !req.no_open {
        open_browser(&verify);
    }

    let deadline = Instant::now() + Duration::from_secs(started.expires_in.max(1));
    let interval = Duration::from_secs(started.interval);
    let poll_url = format!("{web}/api/auth/device/poll");
    loop {
        let (status, body) =
            http.post_json(&poll_url, &json!({ "device_code": started.device_code }))?;
        if status != 200 {
            bail!(
                "device poll failed: {}",
                body.get("error")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&body.to_string())
            );
        }
        let poll: DevicePoll = serde_json::from_value(body).context("parse device poll")?;
        match poll.status.as_str() {
            "pending" => {
                if Instant::now() >= deadline {
                    bail!("device login expired; run rgit login again");
                }
                thread::sleep(interval);
            }
            "authorized" => {
                let user = poll.user.unwrap_or_else(|| "unknown".into());
                remote::upsert(
                    &req.remotes_file,
                    remote,
                    &url,
                    Some(identity.clone()),
                    Some(web),
                )?;
                return Ok(format!(
                    "logged in as {user} on {remote} ({})\nidentity {}\n",
                    target.host,
                    identity.display()
                ));
            }
            "denied" => bail!("device login was denied in the browser"),
            "expired" => bail!("device login expired; run rgit login again"),
            other => bail!("unexpected device status {other:?}"),
        }
    }
}

/// Forget the saved identity for `remote` (does not delete the forge key).
pub fn logout(remotes_file: &Path, remote: &str) -> Result<String> {
    remote::clear_identity(remotes_file, remote)?;
    Ok(format!("logged out of {remote} (forge SSH key kept)\n"))
}

/// Print saved remotes and whether an identity is set.
pub fn status(remotes_file: &Path) -> Result<String> {
    remote::status_text(remotes_file)
}

fn default_web(host: &str) -> String {
    if host == "localhost" || host == "127.0.0.1" {
        "http://127.0.0.1:3010".into()
    } else {
        format!("https://{host}")
    }
}

fn default_identity_path(remote: &str) -> PathBuf {
    remote::config_dir().join(format!("id_{remote}_ed25519"))
}

fn default_hostname() -> String {
    if let Ok(value) = std::env::var("HOSTNAME") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    if let Ok(output) = Command::new("hostname").output() {
        let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !text.is_empty() {
            return text;
        }
    }
    "laptop".into()
}

fn ensure_identity(path: &Path) -> Result<String> {
    if path.exists() {
        let text = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        let key = PrivateKey::from_openssh(&text)
            .map_err(|err| anyhow::anyhow!("parse {}: {err}", path.display()))?;
        return key
            .public_key()
            .to_openssh()
            .map_err(|err| anyhow::anyhow!("encode public key: {err}"));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519)
        .map_err(|err| anyhow::anyhow!("generate key: {err}"))?;
    let encoded = key
        .to_openssh(LineEnding::LF)
        .map_err(|err| anyhow::anyhow!("encode private key: {err}"))?;
    {
        let mut file =
            fs::File::create(path).with_context(|| format!("create {}", path.display()))?;
        file.write_all(encoded.as_bytes())
            .with_context(|| format!("write {}", path.display()))?;
    }
    let mut perms = fs::metadata(path)?.permissions();
    perms.set_mode(0o600);
    fs::set_permissions(path, perms).ok();
    let public = key
        .public_key()
        .to_openssh()
        .map_err(|err| anyhow::anyhow!("encode public key: {err}"))?;
    let pub_path = PathBuf::from(format!("{}.pub", path.display()));
    fs::write(&pub_path, format!("{public}\n")).ok();
    Ok(public)
}

fn open_browser(url: &str) {
    let _ = Command::new("xdg-open")
        .arg(url)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct SeqHttp {
        calls: Mutex<Vec<(String, serde_json::Value)>>,
        responses: Mutex<Vec<(u16, serde_json::Value)>>,
    }

    impl SeqHttp {
        fn new(responses: Vec<(u16, serde_json::Value)>) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                responses: Mutex::new(responses),
            }
        }
    }

    impl DeviceHttp for SeqHttp {
        fn post_json(
            &self,
            url: &str,
            body: &serde_json::Value,
        ) -> Result<(u16, serde_json::Value)> {
            self.calls
                .lock()
                .unwrap()
                .push((url.to_string(), body.clone()));
            let mut q = self.responses.lock().unwrap();
            if q.is_empty() {
                bail!("no more mock responses");
            }
            Ok(q.remove(0))
        }
    }

    #[test]
    fn login_persists_identity() {
        let tmp = tempfile::tempdir().unwrap();
        let remotes = tmp.path().join("remotes.toml");
        let identity = tmp.path().join("id_origin_ed25519");
        let http = SeqHttp::new(vec![
            (
                200,
                json!({
                    "user_code": "ABCD-EFGH",
                    "device_code": "secret",
                    "verification_uri": "/login/device",
                    "expires_in": 30,
                    "interval": 0
                }),
            ),
            (200, json!({ "status": "pending" })),
            (200, json!({ "status": "authorized", "user": "ada" })),
        ]);
        let out = run_with(
            LoginRequest {
                remote: "origin".into(),
                host: Some("rgit.rs".into()),
                web: Some("http://127.0.0.1:3010".into()),
                identity: Some(identity.clone()),
                no_open: true,
                remotes_file: remotes.clone(),
            },
            &http,
        )
        .unwrap();
        assert!(out.contains("logged in as ada"));
        assert!(identity.exists());
        let shown = crate::remote::manage_at(
            &remotes,
            crate::cli::RemoteCommands::Show {
                name: "origin".into(),
            },
        )
        .unwrap();
        assert!(shown.contains("identity:"));
        assert!(shown.contains("web: http://127.0.0.1:3010"));
        let logout_out = logout(&remotes, "origin").unwrap();
        assert!(logout_out.contains("logged out"));
        assert!(identity.exists());
        let after = crate::remote::manage_at(
            &remotes,
            crate::cli::RemoteCommands::Show {
                name: "origin".into(),
            },
        )
        .unwrap();
        assert!(!after.contains("identity:"));
    }
}
