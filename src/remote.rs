//! Named forge hosts on this machine (`rabun-git origin …`).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::cli::RemoteCommands;
use crate::names::valid_user;
use crate::store::atomic_write;

/// Commands that cannot be used as a remote name.
pub const RESERVED_NAMES: &[&str] = &[
    "init", "check", "status", "view", "serve", "shell", "user", "key", "repo", "access",
    "request", "run", "hook", "remote", "help", "agent",
];

/// Parsed SSH target for a named remote.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteTarget {
    /// SSH user (default `git`).
    pub user: String,
    /// Hostname or IP.
    pub host: String,
    /// SSH port (default `2222`).
    pub port: u16,
}

/// A saved remote plus leftover argv after stripping the name.
#[derive(Clone, Debug)]
pub struct ClientInvoke {
    /// Alias (`origin`).
    pub name: String,
    /// Parsed URL.
    pub target: RemoteTarget,
    /// Identity from `remotes.toml`, if any.
    pub identity: Option<PathBuf>,
    /// Host SSH for `key copy` (port 22).
    pub admin: RemoteTarget,
    /// Argv after removing the remote name (may include `--identity`).
    pub args: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct RemotesFile {
    #[serde(default)]
    remotes: BTreeMap<String, RemoteEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct RemoteEntry {
    url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    identity: Option<PathBuf>,
    /// Host SSH for `key copy` (`user@HOST`, default `$USER@<forge-host>:22`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    host: Option<String>,
}

/// Path to the remotes file on this machine (`RABUN_GIT_REMOTES` or XDG config).
pub fn remotes_path() -> PathBuf {
    if let Ok(path) = std::env::var("RABUN_GIT_REMOTES") {
        let path = path.trim();
        if !path.is_empty() {
            return PathBuf::from(path);
        }
    }
    let base = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|home| PathBuf::from(home).join(".config"))
        })
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("rabun-git").join("remotes.toml")
}

/// Run `remote add|list|show|remove` against the default remotes file.
pub fn manage(command: RemoteCommands) -> Result<String> {
    manage_at(&remotes_path(), command)
}

/// Same as [`manage`] with an explicit file (tests).
pub fn manage_at(path: &Path, command: RemoteCommands) -> Result<String> {
    match command {
        RemoteCommands::Add {
            name,
            url,
            identity,
            host,
        } => {
            validate_name(&name)?;
            parse_url(&url)?;
            if let Some(host) = &host {
                parse_host_url(host)?;
            }
            let mut file = load(path)?;
            if file.remotes.contains_key(&name) {
                bail!("remote {name} already exists; rabun-git remote remove {name}");
            }
            file.remotes.insert(
                name.clone(),
                RemoteEntry {
                    url: url.clone(),
                    identity,
                    host,
                },
            );
            save(path, &file)?;
            Ok(format!("remote {name} added\n"))
        }
        RemoteCommands::List => {
            let file = load(path)?;
            if file.remotes.is_empty() {
                return Ok("(no remotes)\n".into());
            }
            let mut lines = Vec::new();
            for (name, entry) in &file.remotes {
                lines.push(format!("{name} {}", entry.url));
            }
            Ok(lines.join("\n") + "\n")
        }
        RemoteCommands::Show { name } => {
            let file = load(path)?;
            let Some(entry) = file.remotes.get(&name) else {
                bail!("remote {name} not found");
            };
            let mut lines = vec![format!("name: {name}"), format!("url: {}", entry.url)];
            if let Some(identity) = &entry.identity {
                lines.push(format!("identity: {}", identity.display()));
            }
            if let Some(host) = &entry.host {
                lines.push(format!("host: {host}"));
            }
            Ok(lines.join("\n") + "\n")
        }
        RemoteCommands::Remove { name } => {
            let mut file = load(path)?;
            if file.remotes.remove(&name).is_none() {
                bail!("remote {name} not found");
            }
            save(path, &file)?;
            Ok(format!("remote {name} removed\n"))
        }
    }
}

/// If the first positional argv is a saved remote name, return a client invoke.
pub fn detect_invoke() -> Result<Option<ClientInvoke>> {
    detect_invoke_from(std::env::args().skip(1), &remotes_path())
}

/// Same as [`detect_invoke`] with explicit argv (no program name) and remotes file.
pub fn detect_invoke_from<I, S>(args: I, remotes_file: &Path) -> Result<Option<ClientInvoke>>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let args: Vec<String> = args
        .into_iter()
        .map(|value| value.as_ref().to_string())
        .collect();
    let Some(idx) = first_positional_index(&args) else {
        return Ok(None);
    };
    let name = args[idx].clone();
    let file = load(remotes_file)?;
    let Some(entry) = file.remotes.get(&name) else {
        return Ok(None);
    };
    let target = parse_url(&entry.url)?;
    let admin = match &entry.host {
        Some(host) => parse_host_url(host)?,
        None => default_admin_target(&target),
    };
    let mut rest = args;
    rest.remove(idx);
    Ok(Some(ClientInvoke {
        name,
        target,
        identity: entry.identity.clone(),
        admin,
        args: rest,
    }))
}

/// Management argv to send over SSH: drop local globals, rewrite `key add --file`.
pub fn payload_args(args: &[String]) -> Result<Vec<String>> {
    let mut out = strip_globals(args);
    rewrite_key_file(&mut out)?;
    if out.is_empty() {
        bail!("missing command after remote name");
    }
    Ok(out)
}

/// `ssh -p PORT [-i identity] user@host -- <quoted command>`.
pub fn ssh_exec(target: &RemoteTarget, identity: Option<&Path>, payload: &[String]) -> Result<()> {
    if payload.is_empty() {
        bail!("missing command after remote name");
    }
    let cmd = quote_payload(payload)?;
    let mut ssh = Command::new("ssh");
    if let Some(identity) = identity {
        ssh.arg("-i").arg(identity);
    }
    ssh.arg("-p").arg(target.port.to_string());
    ssh.arg(format!("{}@{}", target.user, target.host));
    ssh.arg("--");
    ssh.arg(&cmd);
    let status = ssh
        .status()
        .with_context(|| format!("run ssh {}@{}", target.user, target.host))?;
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}

/// Same as [`ssh_exec`] but returns stdout (and fails as an error, no process exit).
pub fn ssh_capture(
    target: &RemoteTarget,
    identity: Option<&Path>,
    payload: &[String],
) -> Result<String> {
    if payload.is_empty() {
        bail!("missing command after remote name");
    }
    let cmd = quote_payload(payload)?;
    let mut ssh = Command::new("ssh");
    if let Some(identity) = identity {
        ssh.arg("-i").arg(identity);
    }
    ssh.arg("-p").arg(target.port.to_string());
    ssh.arg("-q");
    ssh.arg(format!("{}@{}", target.user, target.host));
    ssh.arg("--");
    ssh.arg(&cmd);
    let output = ssh
        .output()
        .with_context(|| format!("run ssh {}@{}", target.user, target.host))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "ssh {}@{} exited {}: {stderr}",
            target.user,
            target.host,
            output.status.code().unwrap_or(1)
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Parse `HOST`, `user@HOST`, `user@HOST:port`, or `ssh://user@HOST:port`.
pub fn parse_url(raw: &str) -> Result<RemoteTarget> {
    let raw = raw.trim();
    if raw.is_empty() {
        bail!("empty remote URL");
    }
    if let Some(rest) = raw.strip_prefix("ssh://") {
        let hostpart = rest.split('/').next().unwrap_or(rest);
        return parse_user_host_port(hostpart, 2222, "git");
    }
    parse_user_host_port(raw, 2222, "git")
}

/// Parse host SSH for `key copy` (`HOST`, `user@HOST`, default port 22).
pub fn parse_host_url(raw: &str) -> Result<RemoteTarget> {
    let raw = raw.trim();
    if raw.is_empty() {
        bail!("empty host SSH URL");
    }
    if let Some(rest) = raw.strip_prefix("ssh://") {
        let hostpart = rest.split('/').next().unwrap_or(rest);
        return parse_user_host_port(hostpart, 22, &local_username());
    }
    parse_user_host_port(raw, 22, &local_username())
}

/// `$USER@<forge-host>:22` for `key copy` when no `host` is saved.
pub fn default_admin_target(forge: &RemoteTarget) -> RemoteTarget {
    RemoteTarget {
        user: local_username(),
        host: forge.host.clone(),
        port: 22,
    }
}

fn local_username() -> String {
    std::env::var("USER")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "git".into())
}

fn parse_user_host_port(raw: &str, default_port: u16, default_user: &str) -> Result<RemoteTarget> {
    let (user, hostport) = match raw.split_once('@') {
        Some((user, rest)) => {
            if user.is_empty() {
                bail!("remote URL is missing the SSH user");
            }
            (user.to_string(), rest)
        }
        None => (default_user.to_string(), raw),
    };
    let (host, port) = match hostport.rsplit_once(':') {
        Some((host, port)) if port.chars().all(|c| c.is_ascii_digit()) => {
            let port: u16 = port
                .parse()
                .with_context(|| format!("invalid SSH port {port:?}"))?;
            if port == 0 {
                bail!("SSH port must be 1-65535");
            }
            (host.to_string(), port)
        }
        _ => (hostport.to_string(), default_port),
    };
    if host.is_empty() {
        bail!("remote URL is missing a host");
    }
    Ok(RemoteTarget { user, host, port })
}

fn validate_name(name: &str) -> Result<()> {
    valid_user(name).with_context(|| format!("invalid remote name {name:?}"))?;
    if RESERVED_NAMES.contains(&name) {
        bail!("remote name {name:?} is reserved for a rabun-git command");
    }
    Ok(())
}

fn load(path: &Path) -> Result<RemotesFile> {
    if !path.exists() {
        return Ok(RemotesFile::default());
    }
    let text = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    if text.trim().is_empty() {
        return Ok(RemotesFile::default());
    }
    toml::from_str(&text).with_context(|| format!("parse {}", path.display()))
}

fn save(path: &Path, file: &RemotesFile) -> Result<()> {
    let text = toml::to_string_pretty(file).context("serialize remotes.toml")?;
    atomic_write(path, text.as_bytes())
}

fn first_positional_index(args: &[String]) -> Option<usize> {
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        if arg == "--" {
            return (i + 1 < args.len()).then_some(i + 1);
        }
        if arg == "--config" || arg == "--identity" {
            i += 2;
            continue;
        }
        if arg.starts_with("--config=") || arg.starts_with("--identity=") {
            i += 1;
            continue;
        }
        if arg.starts_with('-') {
            i += 1;
            continue;
        }
        return Some(i);
    }
    None
}

fn strip_globals(args: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        if arg == "--config" || arg == "--identity" {
            i += 2;
            continue;
        }
        if arg.starts_with("--config=") || arg.starts_with("--identity=") {
            i += 1;
            continue;
        }
        out.push(args[i].clone());
        i += 1;
    }
    out
}

fn rewrite_key_file(args: &mut [String]) -> Result<()> {
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--file" {
            let path = args
                .get(i + 1)
                .ok_or_else(|| anyhow::anyhow!("key add --file needs a path"))?;
            let text = fs::read_to_string(path)
                .with_context(|| format!("read {}", Path::new(path).display()))?;
            args[i] = "--literal".into();
            args[i + 1] = text;
            return Ok(());
        }
        if let Some(path) = args[i].strip_prefix("--file=") {
            let text = fs::read_to_string(path)
                .with_context(|| format!("read {}", Path::new(path).display()))?;
            args[i] = format!("--literal={text}");
            return Ok(());
        }
        i += 1;
    }
    Ok(())
}

/// Copy a local public key to the forge over host SSH (port 22), not git port 2222.
pub fn copy_key(
    invoke: &ClientInvoke,
    user: Option<&str>,
    file: Option<&Path>,
    admin: bool,
    host: Option<&str>,
    identity: Option<&Path>,
) -> Result<()> {
    let user = user
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(local_username);
    valid_user(&user)?;
    let path = match file {
        Some(path) => path.to_path_buf(),
        None => default_pub_file()?,
    };
    let text = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let admin_target = match host {
        Some(host) => parse_host_url(host)?,
        None => invoke.admin.clone(),
    };
    let script = host_install_script(&user, &text, admin)?;
    let remote = copy_remote_command(&script)?;
    let mut ssh = Command::new("ssh");
    ssh.arg("-q");
    ssh.arg("-t");
    if let Some(identity) = identity {
        ssh.arg("-i").arg(identity);
    }
    ssh.arg("-p").arg(admin_target.port.to_string());
    ssh.arg(format!("{}@{}", admin_target.user, admin_target.host));
    ssh.arg("--");
    ssh.arg(&remote);
    let status = ssh
        .status()
        .with_context(|| format!("run ssh {}@{}", admin_target.user, admin_target.host))?;
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    eprintln!("copied key for {user} to {}", invoke.name);
    Ok(())
}

/// systemd working directory / data root on Ubuntu.
const SERVICE_HOME: &str = "/var/lib/rabun-git";
/// dotenv loaded by the host binary and by `key copy`.
const SYSTEM_ENV: &str = "/etc/rabun-git/rabun-git.env";
/// Default `RABUN_GIT_CONFIG` when the env file is missing.
const SYSTEM_CONFIG: &str = "/etc/rabun-git/rabun-git.toml";

/// Shell run on the host as `rabun-git` (tests).
pub fn host_install_script(user: &str, openssh: &str, admin: bool) -> Result<String> {
    let user = shlex::try_quote(user).map_err(|err| anyhow::anyhow!("{err}"))?;
    let key = shlex::try_quote(openssh.trim()).map_err(|err| anyhow::anyhow!("{err}"))?;
    let ensure = if admin {
        format!("rabun-git user add {user} --admin >/dev/null")
    } else {
        format!(
            "rabun-git user list | awk '{{print $1}}' | grep -qx {user} || rabun-git user add {user} >/dev/null"
        )
    };
    // sudo -H keeps the SSH cwd (often /home/$USER). Without ROOT, rabun-git
    // writes ./data/git and the service user cannot create it.
    Ok(format!(
        "cd {SERVICE_HOME} && ([ -r {SYSTEM_ENV} ] && set -a && . {SYSTEM_ENV} && set +a; :) && export RABUN_GIT_ROOT=\"${{RABUN_GIT_ROOT:-{SERVICE_HOME}}}\" RABUN_GIT_CONFIG=\"${{RABUN_GIT_CONFIG:-{SYSTEM_CONFIG}}}\" && {ensure} && rabun-git key add {user} --literal {key} >/dev/null"
    ))
}

/// One ssh remote argument: sudo + bash -c with the script quoted.
fn copy_remote_command(script: &str) -> Result<String> {
    quote_payload(&[
        "sudo".into(),
        "-u".into(),
        "rabun-git".into(),
        "-H".into(),
        "--".into(),
        "/bin/bash".into(),
        "--noprofile".into(),
        "--norc".into(),
        "-c".into(),
        script.to_string(),
    ])
}

fn default_pub_file() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME is unset; pass --file")?;
    for name in ["id_ed25519.pub", "id_rsa.pub"] {
        let path = PathBuf::from(&home).join(".ssh").join(name);
        if path.is_file() {
            return Ok(path);
        }
    }
    bail!("no ~/.ssh/id_ed25519.pub or id_rsa.pub; pass --file");
}

fn quote_payload(payload: &[String]) -> Result<String> {
    let mut parts = Vec::new();
    for arg in payload {
        let quoted = shlex::try_quote(arg).map_err(|err| anyhow::anyhow!("{err}"))?;
        parts.push(quoted.into_owned());
    }
    Ok(parts.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_url_forms() {
        assert_eq!(
            parse_url("damascus").unwrap(),
            RemoteTarget {
                user: "git".into(),
                host: "damascus".into(),
                port: 2222,
            }
        );
        assert_eq!(
            parse_url("git@damascus").unwrap(),
            RemoteTarget {
                user: "git".into(),
                host: "damascus".into(),
                port: 2222,
            }
        );
        assert_eq!(
            parse_url("git@damascus:2222").unwrap(),
            RemoteTarget {
                user: "git".into(),
                host: "damascus".into(),
                port: 2222,
            }
        );
        assert_eq!(
            parse_url("ssh://git@damascus:2222").unwrap(),
            RemoteTarget {
                user: "git".into(),
                host: "damascus".into(),
                port: 2222,
            }
        );
        assert_eq!(
            parse_url("ssh://ada@git.example.com:2200/ada/website.git").unwrap(),
            RemoteTarget {
                user: "ada".into(),
                host: "git.example.com".into(),
                port: 2200,
            }
        );
    }

    #[test]
    fn reserved_and_add_list_remove() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("remotes.toml");
        let err = manage_at(
            &path,
            RemoteCommands::Add {
                name: "repo".into(),
                url: "git@damascus".into(),
                identity: None,
                host: None,
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("reserved"));

        let out = manage_at(
            &path,
            RemoteCommands::Add {
                name: "origin".into(),
                url: "git@damascus".into(),
                identity: None,
                host: None,
            },
        )
        .unwrap();
        assert!(out.contains("origin"));
        assert!(manage_at(&path, RemoteCommands::List)
            .unwrap()
            .contains("origin git@damascus"));
        assert!(manage_at(
            &path,
            RemoteCommands::Show {
                name: "origin".into()
            }
        )
        .unwrap()
        .contains("url: git@damascus"));
        manage_at(
            &path,
            RemoteCommands::Remove {
                name: "origin".into(),
            },
        )
        .unwrap();
        assert_eq!(
            manage_at(&path, RemoteCommands::List).unwrap(),
            "(no remotes)\n"
        );
    }

    #[test]
    fn detect_strips_name_and_rewrites_file() {
        let tmp = tempfile::tempdir().unwrap();
        let remotes = tmp.path().join("remotes.toml");
        manage_at(
            &remotes,
            RemoteCommands::Add {
                name: "origin".into(),
                url: "git@damascus:2222".into(),
                identity: Some(PathBuf::from("/id")),
                host: Some("ryan@damascus".into()),
            },
        )
        .unwrap();
        let pub_path = tmp.path().join("id.pub");
        fs::write(&pub_path, "ssh-ed25519 AAAA test\n").unwrap();

        let invoke = detect_invoke_from(
            [
                "--identity",
                "/override",
                "origin",
                "key",
                "add",
                "ryan",
                "--file",
                pub_path.to_str().unwrap(),
            ],
            &remotes,
        )
        .unwrap()
        .expect("origin");
        assert_eq!(invoke.name, "origin");
        assert_eq!(invoke.target.host, "damascus");
        assert_eq!(invoke.identity.as_deref(), Some(Path::new("/id")));
        assert_eq!(
            invoke.admin,
            RemoteTarget {
                user: "ryan".into(),
                host: "damascus".into(),
                port: 22,
            }
        );
        assert!(manage_at(
            &remotes,
            RemoteCommands::Show {
                name: "origin".into()
            }
        )
        .unwrap()
        .contains("host: ryan@damascus"));

        let payload = payload_args(&invoke.args).unwrap();
        assert_eq!(payload[0], "key");
        assert_eq!(payload[1], "add");
        assert_eq!(payload[2], "ryan");
        assert_eq!(payload[3], "--literal");
        assert!(payload[4].contains("ssh-ed25519"));
        assert!(!payload
            .iter()
            .any(|a| a == "--identity" || a == "/override"));
    }

    #[test]
    fn parse_host_url_defaults_port_22() {
        let target = parse_host_url("damascus").unwrap();
        assert_eq!(target.host, "damascus");
        assert_eq!(target.port, 22);
        assert!(!target.user.is_empty());
        assert_eq!(
            parse_host_url("ryan@damascus:22").unwrap(),
            RemoteTarget {
                user: "ryan".into(),
                host: "damascus".into(),
                port: 22,
            }
        );
        assert_eq!(
            parse_host_url("ssh://ada@git.example.com/unused").unwrap(),
            RemoteTarget {
                user: "ada".into(),
                host: "git.example.com".into(),
                port: 22,
            }
        );
    }

    #[test]
    fn default_admin_uses_local_user_and_port_22() {
        let admin = default_admin_target(&RemoteTarget {
            user: "git".into(),
            host: "damascus".into(),
            port: 2222,
        });
        assert_eq!(admin.host, "damascus");
        assert_eq!(admin.port, 22);
        assert_eq!(admin.user, local_username());
    }

    #[test]
    fn host_install_script_quotes_key() {
        let admin = host_install_script("ryan", "ssh-ed25519 AAAA comment", true).unwrap();
        assert!(admin.contains("user add ryan --admin >/dev/null"));
        assert!(admin.contains("key add ryan --literal"));
        assert!(admin.contains(">/dev/null"));
        assert!(admin.contains("ssh-ed25519 AAAA comment"));
        let ordinary = host_install_script("ada", "ssh-ed25519 BBBB", false).unwrap();
        assert!(ordinary.contains("user add ada"));
        assert!(!ordinary.contains("--admin"));
        assert!(ordinary.contains("grep -qx ada"));
        assert!(admin.contains("cd /var/lib/rabun-git"));
        assert!(admin.contains("RABUN_GIT_ROOT="));
        assert!(!admin.starts_with("set"));
        assert!(!admin.contains('\n'));

        let remote = copy_remote_command(&admin).unwrap();
        let split = shlex::split(&remote).unwrap();
        assert_eq!(split[0], "sudo");
        assert_eq!(split[split.len() - 2], "-c");
        assert_eq!(split.last().unwrap(), &admin);
        assert!(split.last().unwrap().contains("ssh-ed25519 AAAA comment"));
    }

    #[test]
    fn quote_keeps_key_spaces() {
        let cmd = quote_payload(&[
            "key".into(),
            "add".into(),
            "ryan".into(),
            "--literal".into(),
            "ssh-ed25519 AAAA comment".into(),
        ])
        .unwrap();
        let split = shlex::split(&cmd).unwrap();
        assert_eq!(split.last().unwrap(), "ssh-ed25519 AAAA comment");
    }
}
