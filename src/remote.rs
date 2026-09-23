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
    "init", "check", "status", "serve", "shell", "user", "key", "repo", "access", "request", "run",
    "hook", "remote", "help",
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
        } => {
            validate_name(&name)?;
            parse_url(&url)?;
            let mut file = load(path)?;
            if file.remotes.contains_key(&name) {
                bail!("remote {name} already exists; rabun-git remote remove {name}");
            }
            file.remotes.insert(
                name.clone(),
                RemoteEntry {
                    url: url.clone(),
                    identity,
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
    let mut rest = args;
    rest.remove(idx);
    Ok(Some(ClientInvoke {
        name,
        target,
        identity: entry.identity.clone(),
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

/// Parse `HOST`, `user@HOST`, `user@HOST:port`, or `ssh://user@HOST:port`.
pub fn parse_url(raw: &str) -> Result<RemoteTarget> {
    let raw = raw.trim();
    if raw.is_empty() {
        bail!("empty remote URL");
    }
    if let Some(rest) = raw.strip_prefix("ssh://") {
        let hostpart = rest.split('/').next().unwrap_or(rest);
        return parse_user_host_port(hostpart);
    }
    parse_user_host_port(raw)
}

fn parse_user_host_port(raw: &str) -> Result<RemoteTarget> {
    let (user, hostport) = match raw.split_once('@') {
        Some((user, rest)) => {
            if user.is_empty() {
                bail!("remote URL is missing the SSH user");
            }
            (user.to_string(), rest)
        }
        None => ("git".into(), raw),
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
        _ => (hostport.to_string(), 2222),
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
