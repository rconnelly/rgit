//! File + env configuration. Host keys live on disk; no cloud tokens.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

/// Worker settings loaded from `rabun-git.toml` and env var *names*.
#[derive(Clone, Debug, Deserialize)]
pub struct Config {
    /// Env var for the forge data root (bare repos, users, keys, runs).
    #[serde(default = "default_root_env")]
    pub root_env: String,
    /// SSH bind for git and management commands (`host:port`).
    #[serde(default = "default_ssh_bind")]
    pub ssh_bind: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            root_env: default_root_env(),
            ssh_bind: default_ssh_bind(),
        }
    }
}

fn default_root_env() -> String {
    "RABUN_GIT_ROOT".into()
}

fn default_ssh_bind() -> String {
    crate::ssh::DEFAULT_SSH_BIND.into()
}

impl Config {
    /// Load `rabun-git.toml` from `config_path` or the current directory.
    pub fn load(config_path: Option<&Path>) -> Result<Self> {
        let mut cfg = Config::default();
        if let Some(path) = config_path {
            if path.exists() {
                cfg.merge_file(path)?;
            }
        } else if let Ok(cwd) = std::env::current_dir() {
            let path = cwd.join("rabun-git.toml");
            if path.exists() {
                cfg.merge_file(&path)?;
            }
        }
        if let Ok(bind) = std::env::var("RABUN_GIT_SSH_BIND") {
            let bind = bind.trim();
            if !bind.is_empty() {
                cfg.ssh_bind = bind.to_string();
            }
        }
        Ok(cfg)
    }

    fn merge_file(&mut self, path: &Path) -> Result<()> {
        let contents =
            std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        let parsed: Config =
            toml::from_str(&contents).with_context(|| format!("parse {}", path.display()))?;
        *self = parsed;
        Ok(())
    }

    /// Forge data root (`data/git` unless `RABUN_GIT_ROOT` is set).
    pub fn root(&self) -> PathBuf {
        env_path(&self.root_env, "data/git")
    }

    /// SSH listen address after optional CLI override.
    pub fn ssh_bind(&self, override_bind: Option<&str>) -> Result<SocketAddr> {
        let bind = override_bind
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(self.ssh_bind.trim());
        bind.parse().with_context(|| {
            format!("SSH bind {bind} must be host:port (for example 0.0.0.0:2222)")
        })
    }
}

fn env_path(var: &str, fallback: &str) -> PathBuf {
    std::env::var(var)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(fallback))
}

/// Default TOML written by `rabun-git init`.
pub fn default_toml() -> &'static str {
    "root_env = \"RABUN_GIT_ROOT\"\n\
     ssh_bind = \"0.0.0.0:2222\"\n"
}

/// Default `.env.example` written by `rabun-git init`.
pub fn default_env_example() -> &'static str {
    include_str!("../.env.example")
}

/// Resolve `rabun-git.toml` next to the current directory unless overridden.
pub fn config_path(config_path: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = config_path {
        return Ok(path.to_path_buf());
    }
    let cwd = std::env::current_dir().context("current directory")?;
    Ok(cwd.join("rabun-git.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_defaults() {
        let cfg = Config::default();
        if std::env::var("RABUN_GIT_ROOT")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .is_none()
        {
            assert_eq!(cfg.root(), PathBuf::from("data/git"));
        }
    }

    #[test]
    fn ssh_bind_parses() {
        let cfg = Config::default();
        assert!(cfg.ssh_bind(None).is_ok());
        assert!(cfg.ssh_bind(Some("127.0.0.1:0")).is_ok());
        assert!(cfg.ssh_bind(Some("not-an-addr")).is_err());
    }
}
