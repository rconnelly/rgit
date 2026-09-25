//! Device authorization for CLI web sign-on (`devices.yaml`).
//!
//! The laptop CLI holds a private key and a `device_code`. The website looks up
//! a short `user_code`, and a signed-in user attaches the stored public key to
//! their account. Only SHA-256 of the device code is stored.

use anyhow::{bail, Result};
use russh::keys::{HashAlg, PublicKey};
use serde::{Deserialize, Serialize};

use crate::acl::Actor;
use crate::auth::token_hash;
use crate::cli::DeviceCommands;
use crate::output;
use crate::store::Store;

/// Seconds a pending device grant stays valid.
pub const EXPIRES_IN: u64 = 900;
/// Suggested poll interval for the laptop CLI.
pub const POLL_INTERVAL: u64 = 5;

const USER_CODE_LEN: usize = 8;
const USER_CODE_ALPHABET: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// `devices.yaml`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DevicesFile {
    /// Pending and recently completed grants.
    #[serde(default)]
    pub devices: Vec<DeviceRecord>,
}

/// One device grant row.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeviceRecord {
    /// SHA-256 hex of the raw device code.
    pub device_code_hash: String,
    /// Display code (`ABCD-EFGH`).
    pub user_code: String,
    /// Canonical OpenSSH public key (one line).
    pub public_key: String,
    /// SHA-256 fingerprint (`SHA256:…`).
    pub fingerprint: String,
    /// Client-supplied hostname (display only).
    #[serde(default)]
    pub hostname: String,
    /// RFC 3339.
    pub created_at: String,
    /// RFC 3339.
    pub expires_at: String,
    /// `pending`, `authorized`, or `denied`.
    pub status: String,
    /// Set when authorized.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
}

/// JSON from `auth device start`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeviceStart {
    /// Short code typed in the browser.
    pub user_code: String,
    /// Secret the CLI polls with (shown once).
    pub device_code: String,
    /// Path on rgit-web (`/login/device`).
    pub verification_uri: String,
    /// Seconds until the grant expires.
    pub expires_in: u64,
    /// Suggested poll interval in seconds.
    pub interval: u64,
}

/// JSON from `auth device poll`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DevicePoll {
    /// `pending`, `authorized`, `denied`, or `expired`.
    pub status: String,
    /// Forge login when authorized.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
}

/// JSON from `auth device show`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeviceShow {
    /// Display code.
    pub user_code: String,
    /// Client hostname.
    pub hostname: String,
    /// Public key fingerprint.
    pub fingerprint: String,
    /// `pending`, `authorized`, or `denied`.
    pub status: String,
    /// RFC 3339.
    pub expires_at: String,
}

/// Run `auth device *`.
pub fn execute(
    store: &Store,
    actor: &Actor,
    command: DeviceCommands,
    json: bool,
) -> Result<String> {
    match command {
        DeviceCommands::Start {
            public_key,
            hostname,
        } => start(store, &public_key, hostname.as_deref(), json),
        DeviceCommands::Poll { device_code } => poll(store, &device_code, json),
        DeviceCommands::Show { user_code } => show(store, actor, &user_code, json),
        DeviceCommands::Approve { user_code } => approve(store, actor, &user_code, json),
        DeviceCommands::Deny { user_code } => deny(store, actor, &user_code, json),
    }
}

fn start(store: &Store, public_key: &str, hostname: Option<&str>, json: bool) -> Result<String> {
    let (canonical, fingerprint) = parse_public_key(public_key)?;
    let hostname = sanitize_hostname(hostname.unwrap_or(""));
    let mut file = store.load_devices()?;
    prune_expired(&mut file);
    let user_code = unique_user_code(&file);
    let device_code = new_device_code();
    let now = chrono::Utc::now();
    let expires = now + chrono::TimeDelta::seconds(EXPIRES_IN as i64);
    file.devices.push(DeviceRecord {
        device_code_hash: token_hash(&device_code),
        user_code: user_code.clone(),
        public_key: canonical,
        fingerprint,
        hostname,
        created_at: now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        expires_at: expires.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        status: "pending".into(),
        user: None,
    });
    store.save_devices(&file)?;
    let body = DeviceStart {
        user_code: user_code.clone(),
        device_code,
        verification_uri: "/login/device".into(),
        expires_in: EXPIRES_IN,
        interval: POLL_INTERVAL,
    };
    output::pick(
        json,
        &body,
        format!(
            "user code {user_code}\nopen /login/device and enter the code\nexpires in {EXPIRES_IN}s\n"
        ),
    )
}

fn poll(store: &Store, device_code: &str, json: bool) -> Result<String> {
    let hash = token_hash(device_code.trim());
    let file = store.load_devices()?;
    let Some(row) = file.devices.iter().find(|row| row.device_code_hash == hash) else {
        return poll_status("expired", None, json);
    };
    if is_expired(row) {
        return poll_status("expired", None, json);
    }
    match row.status.as_str() {
        "authorized" => poll_status("authorized", row.user.clone(), json),
        "denied" => poll_status("denied", None, json),
        _ => poll_status("pending", None, json),
    }
}

fn poll_status(status: &str, user: Option<String>, json: bool) -> Result<String> {
    let body = DevicePoll {
        status: status.into(),
        user,
    };
    let text = match (status, &body.user) {
        ("authorized", Some(name)) => format!("authorized as {name}\n"),
        ("denied", _) => "denied\n".into(),
        ("expired", _) => "expired\n".into(),
        _ => "pending\n".into(),
    };
    output::pick(json, &body, text)
}

fn show(store: &Store, actor: &Actor, user_code: &str, json: bool) -> Result<String> {
    require_signed_in(actor)?;
    let row = pending_lookup(store, user_code)?;
    let body = DeviceShow {
        user_code: row.user_code.clone(),
        hostname: row.hostname.clone(),
        fingerprint: row.fingerprint.clone(),
        status: row.status.clone(),
        expires_at: row.expires_at.clone(),
    };
    output::pick(
        json,
        &body,
        format!(
            "{} {}\n{}\n{}\n",
            body.user_code, body.status, body.hostname, body.fingerprint
        ),
    )
}

fn approve(store: &Store, actor: &Actor, user_code: &str, json: bool) -> Result<String> {
    let Some(user) = actor.name().map(str::to_string) else {
        bail!("sign in to approve a device");
    };
    let mut file = store.load_devices()?;
    let idx =
        find_user_code(&file, user_code).ok_or_else(|| anyhow::anyhow!("device not found"))?;
    if is_expired(&file.devices[idx]) {
        bail!("device not found");
    }
    if file.devices[idx].status != "pending" {
        bail!("already used");
    }
    let public_key = file.devices[idx].public_key.clone();
    let n = store.add_keys(&user, &public_key)?;
    file.devices[idx].status = "authorized".into();
    file.devices[idx].user = Some(user.clone());
    store.save_devices(&file)?;
    output::pick(
        json,
        &serde_json::json!({ "ok": true, "user": user, "added": n }),
        format!("authorized {user}, added {n} key(s)\n"),
    )
}

fn deny(store: &Store, actor: &Actor, user_code: &str, json: bool) -> Result<String> {
    require_signed_in(actor)?;
    let mut file = store.load_devices()?;
    let idx =
        find_user_code(&file, user_code).ok_or_else(|| anyhow::anyhow!("device not found"))?;
    if is_expired(&file.devices[idx]) {
        bail!("device not found");
    }
    if file.devices[idx].status != "pending" {
        bail!("already used");
    }
    file.devices[idx].status = "denied".into();
    store.save_devices(&file)?;
    output::pick(json, &serde_json::json!({ "ok": true }), "denied\n".into())
}

fn require_signed_in(actor: &Actor) -> Result<()> {
    match actor {
        Actor::Anonymous => bail!("sign in to view this device"),
        Actor::User(_) | Actor::Operator => Ok(()),
    }
}

fn pending_lookup(store: &Store, user_code: &str) -> Result<DeviceRecord> {
    let file = store.load_devices()?;
    let idx =
        find_user_code(&file, user_code).ok_or_else(|| anyhow::anyhow!("device not found"))?;
    if is_expired(&file.devices[idx]) {
        bail!("device not found");
    }
    Ok(file.devices[idx].clone())
}

fn find_user_code(file: &DevicesFile, user_code: &str) -> Option<usize> {
    let want = normalize_user_code(user_code);
    file.devices
        .iter()
        .position(|row| normalize_user_code(&row.user_code) == want)
}

fn prune_expired(file: &mut DevicesFile) {
    file.devices
        .retain(|row| !is_expired(row) || row.status != "pending");
}

fn is_expired(row: &DeviceRecord) -> bool {
    let Ok(expires) = chrono::DateTime::parse_from_rfc3339(&row.expires_at) else {
        return true;
    };
    expires < chrono::Utc::now()
}

fn parse_public_key(openssh: &str) -> Result<(String, String)> {
    let line = openssh
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .ok_or_else(|| anyhow::anyhow!("no OpenSSH public key in input"))?;
    let key = PublicKey::from_openssh(line)
        .map_err(|err| anyhow::anyhow!("parse OpenSSH public key: {err}"))?;
    let canonical = key
        .to_openssh()
        .map_err(|err| anyhow::anyhow!("encode public key: {err}"))?;
    let fingerprint = key.fingerprint(HashAlg::Sha256).to_string();
    Ok((canonical, fingerprint))
}

fn sanitize_hostname(raw: &str) -> String {
    let trimmed: String = raw
        .chars()
        .take(64)
        .filter(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '-' || *c == '_')
        .collect();
    if trimmed.is_empty() {
        "laptop".into()
    } else {
        trimmed
    }
}

fn unique_user_code(file: &DevicesFile) -> String {
    for _ in 0..32 {
        let code = new_user_code();
        let norm = normalize_user_code(&code);
        if !file
            .devices
            .iter()
            .any(|row| !is_expired(row) && normalize_user_code(&row.user_code) == norm)
        {
            return code;
        }
    }
    new_user_code()
}

fn new_user_code() -> String {
    use rand::RngExt;
    let mut rng = rand::rng();
    let mut raw = String::new();
    for _ in 0..USER_CODE_LEN {
        let idx = rng.random_range(0..USER_CODE_ALPHABET.len());
        raw.push(USER_CODE_ALPHABET[idx] as char);
    }
    format!("{}-{}", &raw[..4], &raw[4..])
}

fn new_device_code() -> String {
    use rand::Rng;
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn normalize_user_code(raw: &str) -> String {
    raw.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::now_iso;
    use russh::keys::{Algorithm, PrivateKey};

    fn sample_key() -> String {
        let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
        key.public_key().to_openssh().unwrap()
    }

    fn start_json(store: &Store, key: &str, host: &str) -> DeviceStart {
        let out = start(store, key, Some(host), true).unwrap();
        serde_json::from_str(out.trim()).unwrap()
    }

    #[test]
    fn start_approve_poll() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path());
        store.ensure_layout().unwrap();
        store.add_user("ada", true).unwrap();
        let key = sample_key();
        let started = start_json(&store, &key, "ryans-laptop");
        assert_eq!(started.expires_in, EXPIRES_IN);
        assert!(started.user_code.contains('-'));

        let pending: DevicePoll =
            serde_json::from_str(poll(&store, &started.device_code, true).unwrap().trim()).unwrap();
        assert_eq!(pending.status, "pending");

        let shown: DeviceShow = serde_json::from_str(
            show(&store, &Actor::User("ada".into()), &started.user_code, true)
                .unwrap()
                .trim(),
        )
        .unwrap();
        assert_eq!(shown.hostname, "ryans-laptop");
        assert!(shown.fingerprint.contains("SHA256:"));

        let err = approve(&store, &Actor::Anonymous, &started.user_code, true).unwrap_err();
        assert!(err.to_string().contains("sign in"));

        approve(&store, &Actor::User("ada".into()), &started.user_code, true).unwrap();
        assert!(!store.key_fingerprints("ada").unwrap().is_empty());

        let done: DevicePoll =
            serde_json::from_str(poll(&store, &started.device_code, true).unwrap().trim()).unwrap();
        assert_eq!(done.status, "authorized");
        assert_eq!(done.user.as_deref(), Some("ada"));

        let reused =
            approve(&store, &Actor::User("ada".into()), &started.user_code, true).unwrap_err();
        assert!(reused.to_string().contains("already used"));
    }

    #[test]
    fn deny_and_unknown_poll_are_expired() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path());
        store.ensure_layout().unwrap();
        store.add_user("linus", false).unwrap();
        let started = start_json(&store, &sample_key(), "box");
        deny(
            &store,
            &Actor::User("linus".into()),
            &started.user_code,
            true,
        )
        .unwrap();
        let denied: DevicePoll =
            serde_json::from_str(poll(&store, &started.device_code, true).unwrap().trim()).unwrap();
        assert_eq!(denied.status, "denied");
        let unknown: DevicePoll =
            serde_json::from_str(poll(&store, "no-such-device-code", true).unwrap().trim())
                .unwrap();
        assert_eq!(unknown.status, "expired");
    }

    #[test]
    fn expired_row_is_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path());
        store.ensure_layout().unwrap();
        store.add_user("ada", true).unwrap();
        let key = sample_key();
        let (canonical, fingerprint) = parse_public_key(&key).unwrap();
        let mut file = DevicesFile::default();
        file.devices.push(DeviceRecord {
            device_code_hash: token_hash("secret"),
            user_code: "ABCD-EFGH".into(),
            public_key: canonical,
            fingerprint,
            hostname: "old".into(),
            created_at: now_iso(),
            expires_at: "2000-01-01T00:00:00.000Z".into(),
            status: "pending".into(),
            user: None,
        });
        store.save_devices(&file).unwrap();
        let err = show(&store, &Actor::User("ada".into()), "ABCD-EFGH", true).unwrap_err();
        assert!(err.to_string().contains("not found"));
        let poll: DevicePoll =
            serde_json::from_str(poll(&store, "secret", true).unwrap().trim()).unwrap();
        assert_eq!(poll.status, "expired");
    }

    #[test]
    fn user_code_normalizes_dashes() {
        assert_eq!(normalize_user_code("ab-cd-efgh"), "ABCDEFGH");
        assert_eq!(normalize_user_code("ABCD-EFGH"), "ABCDEFGH");
    }
}
