//! `init` and `check`.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::config::{self, Config};
use crate::git;
use crate::store::Store;

const ENV_EXAMPLE: &str = include_str!("../.env.example");

/// Write `rabun-git.toml`, `.env.example`, and an empty forge root.
pub fn init(config_path: Option<&Path>) -> Result<PathBuf> {
    let toml_path = config::config_path(config_path)?;
    if !toml_path.exists() {
        if let Some(parent) = toml_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create {}", parent.display()))?;
        }
        std::fs::write(&toml_path, config::default_toml())
            .with_context(|| format!("write {}", toml_path.display()))?;
    }
    let env_example = toml_path
        .parent()
        .map(|dir| dir.join(".env.example"))
        .unwrap_or_else(|| PathBuf::from(".env.example"));
    if !env_example.exists() {
        std::fs::write(&env_example, ENV_EXAMPLE).context("write .env.example")?;
    }
    let cfg = Config::load(Some(&toml_path))?;
    let root = std::env::var(&cfg.root_env)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            toml_path
                .parent()
                .map(|dir| dir.join("data/git"))
                .unwrap_or_else(|| PathBuf::from("data/git"))
        });
    Store::open(root).ensure_layout()?;
    Ok(toml_path)
}

/// Human-readable next steps after [`init`].
pub fn next_steps(toml_path: &Path) -> String {
    format!(
        "Wrote {} and .env.example.\n\
         Next:\n\
         1. Optional: copy .env.example to .env and set RABUN_GIT_ROOT\n\
         2. rabun-git user add YOURNAME --admin\n\
         3. rabun-git key add YOURNAME --file ~/.ssh/id_ed25519.pub\n\
         4. rabun-git check\n\
         5. rabun-git serve\n\
         6. git clone ssh://git@HOST:2222/owner/name.git\n",
        toml_path.display()
    )
}

/// Verify required paths and that an admin can authenticate.
pub fn check(config: &Config) -> Result<()> {
    if !git::git_on_path() {
        bail!("git is not on PATH");
    }
    let root = config.root();
    std::fs::create_dir_all(&root).with_context(|| format!("create {}", root.display()))?;
    let probe = root.join(".rabun-git-write-test");
    std::fs::write(&probe, b"ok").with_context(|| format!("write {}", root.display()))?;
    let _ = std::fs::remove_file(&probe);
    println!("root: {}", root.display());
    println!("ssh bind: {}", config.ssh_bind(None)?);
    let store = Store::open(&root);
    store.ensure_layout()?;
    let users = store.load_users()?;
    let admins: Vec<_> = users.users.iter().filter(|u| u.admin).cloned().collect();
    if admins.is_empty() {
        bail!("no admin user; rabun-git user add NAME --admin");
    }
    let mut keyed_admin = false;
    for admin in &admins {
        if !store.public_keys(&admin.name)?.is_empty() {
            keyed_admin = true;
            break;
        }
    }
    if !keyed_admin {
        bail!(
            "admin user(s) have no SSH keys; rabun-git key add {} --file KEY.pub",
            admins[0].name
        );
    }
    println!(
        "admins: {}",
        admins
            .iter()
            .map(|u| u.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!("git: ok");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_writes_toml_and_env_example() {
        let tmp = tempfile::tempdir().unwrap();
        let toml_path = tmp.path().join("rabun-git.toml");
        init(Some(&toml_path)).unwrap();
        let contents = std::fs::read_to_string(&toml_path).unwrap();
        assert!(contents.contains("RABUN_GIT_ROOT"));
        assert!(contents.contains("ssh_bind"));
        let env = std::fs::read_to_string(tmp.path().join(".env.example")).unwrap();
        assert!(env.contains("RABUN_GIT_ROOT"));
        assert!(!env.contains("sk_live"));
        assert!(tmp.path().join("data/git/users.yaml").exists());
        init(Some(&toml_path)).unwrap();
        assert_eq!(contents, std::fs::read_to_string(&toml_path).unwrap());
    }
}
