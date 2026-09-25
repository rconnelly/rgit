//! On-disk forge layout: users, keys, ACL, repos, run logs.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use russh::keys::{HashAlg, PublicKey};
use serde::{Deserialize, Serialize};

use crate::acl::Role;
use crate::names::{valid_user, RepoName};

/// Filesystem layout under [`crate::config::Config::root`].
#[derive(Clone, Debug)]
pub struct Store {
    root: PathBuf,
}

impl Store {
    /// Open a store at `root` (does not create files).
    pub fn open(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Absolute data root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Create directories and empty YAML files if missing.
    pub fn ensure_layout(&self) -> Result<()> {
        for dir in ["keys", "repos", "runs"] {
            fs::create_dir_all(self.root.join(dir))
                .with_context(|| format!("create {}/{dir}", self.root.display()))?;
        }
        if !self.users_path().exists() {
            self.save_users(&UsersFile::default())?;
        }
        if !self.access_path().exists() {
            self.save_access(&AccessFile::default())?;
        }
        if !self.builders_path().exists() {
            self.save_builders(&BuildersFile::default())?;
        }
        if !self.tokens_path().exists() {
            self.save_tokens(&crate::auth::TokensFile::default())?;
        }
        if !self.visibility_path().exists() {
            self.save_visibility(&VisibilityFile::default())?;
        }
        Ok(())
    }

    /// Host Ed25519 key path (OpenSSH format).
    pub fn host_key_path(&self) -> PathBuf {
        self.root.join("ssh_host_ed25519_key")
    }

    fn users_path(&self) -> PathBuf {
        self.root.join("users.yaml")
    }

    fn access_path(&self) -> PathBuf {
        self.root.join("access.yaml")
    }

    fn builders_path(&self) -> PathBuf {
        self.root.join("builders.yaml")
    }

    fn tokens_path(&self) -> PathBuf {
        self.root.join("tokens.yaml")
    }

    fn visibility_path(&self) -> PathBuf {
        self.root.join("visibility.yaml")
    }

    /// `keys/<user>.pub`
    pub fn keys_path(&self, user: &str) -> PathBuf {
        self.root.join("keys").join(format!("{user}.pub"))
    }

    /// Bare repo directory.
    pub fn repo_path(&self, name: &RepoName) -> PathBuf {
        self.root.join("repos").join(name.dir_name())
    }

    /// Workflow run directory for a repo.
    pub fn runs_dir(&self, name: &RepoName) -> PathBuf {
        self.root.join("runs").join(&name.owner).join(&name.name)
    }

    /// Load `users.yaml`.
    pub fn load_users(&self) -> Result<UsersFile> {
        read_yaml(&self.users_path())
    }

    /// Write `users.yaml` atomically.
    pub fn save_users(&self, users: &UsersFile) -> Result<()> {
        write_yaml(&self.users_path(), users)
    }

    /// Load `access.yaml`.
    pub fn load_access(&self) -> Result<AccessFile> {
        read_yaml(&self.access_path())
    }

    /// Write `access.yaml` atomically.
    pub fn save_access(&self, access: &AccessFile) -> Result<()> {
        write_yaml(&self.access_path(), access)
    }

    /// Load `builders.yaml`.
    pub fn load_builders(&self) -> Result<BuildersFile> {
        read_yaml(&self.builders_path())
    }

    /// Write `builders.yaml` atomically.
    pub fn save_builders(&self, builders: &BuildersFile) -> Result<()> {
        write_yaml(&self.builders_path(), builders)
    }

    /// True when any registered builder advertises `label`.
    pub fn has_builder_label(&self, label: &str) -> bool {
        self.load_builders()
            .ok()
            .map(|file| {
                file.builders
                    .iter()
                    .any(|b| b.labels.iter().any(|l| l == label))
            })
            .unwrap_or(false)
    }

    /// Builder record for a forge login, if registered.
    pub fn builder_by_name(&self, name: &str) -> Result<Option<BuilderRecord>> {
        Ok(self
            .load_builders()?
            .builders
            .into_iter()
            .find(|b| b.name == name))
    }

    /// Register or update a builder (name + labels).
    pub fn upsert_builder(&self, name: &str, labels: &[String]) -> Result<()> {
        valid_user(name)?;
        if labels.is_empty() {
            bail!("builder {name} needs at least one --label");
        }
        let mut file = self.load_builders()?;
        if let Some(existing) = file.builders.iter_mut().find(|b| b.name == name) {
            existing.labels = labels.to_vec();
        } else {
            file.builders.push(BuilderRecord {
                name: name.to_string(),
                labels: labels.to_vec(),
            });
        }
        self.save_builders(&file)
    }

    /// Add a user (idempotent name, last write wins for admin flag).
    pub fn add_user(&self, name: &str, admin: bool) -> Result<()> {
        valid_user(name)?;
        let mut users = self.load_users()?;
        if let Some(existing) = users.users.iter_mut().find(|u| u.name == name) {
            existing.admin = admin;
        } else {
            users.users.push(UserRecord {
                name: name.to_string(),
                admin,
                password_hash: None,
            });
        }
        self.save_users(&users)
    }

    /// Remove a user and their key file.
    pub fn remove_user(&self, name: &str) -> Result<()> {
        let mut users = self.load_users()?;
        let before = users.users.len();
        users.users.retain(|u| u.name != name);
        if users.users.len() == before {
            bail!("user {name} not found");
        }
        self.save_users(&users)?;
        let _ = self.revoke_user_tokens(name);
        let keys = self.keys_path(name);
        if keys.exists() {
            fs::remove_file(&keys).with_context(|| format!("remove {}", keys.display()))?;
        }
        let mut access = self.load_access()?;
        for roles in access.repos.values_mut() {
            roles.shift_remove(name);
        }
        self.save_access(&access)
    }

    /// Append OpenSSH public keys for `user` (user must exist).
    pub fn add_keys(&self, user: &str, openssh: &str) -> Result<usize> {
        valid_user(user)?;
        if self.load_users()?.by_name(user).is_none() {
            bail!("user {user} not found; rabun-git user add {user}");
        }
        let mut parsed = Vec::new();
        for line in openssh.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let key = PublicKey::from_openssh(line)
                .with_context(|| format!("parse OpenSSH public key for {user}"))?;
            parsed.push(key.to_openssh().context("encode public key")?);
        }
        if parsed.is_empty() {
            bail!("no OpenSSH public keys in input");
        }
        let path = self.keys_path(user);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).ok();
        }
        let mut existing = if path.exists() {
            fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?
        } else {
            String::new()
        };
        if !existing.is_empty() && !existing.ends_with('\n') {
            existing.push('\n');
        }
        let added = parsed.len();
        for key in parsed {
            if !existing.lines().any(|line| line.trim() == key.trim()) {
                existing.push_str(key.trim());
                existing.push('\n');
            }
        }
        atomic_write(&path, existing.as_bytes())?;
        Ok(added)
    }

    /// Parse stored public keys for `user`.
    pub fn public_keys(&self, user: &str) -> Result<Vec<PublicKey>> {
        let path = self.keys_path(user);
        if !path.exists() {
            return Ok(Vec::new());
        }
        let text = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        let mut keys = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            keys.push(
                PublicKey::from_openssh(line)
                    .with_context(|| format!("parse {} (line {line:?})", path.display()))?,
            );
        }
        Ok(keys)
    }

    /// Fingerprints (SHA-256) for listing; never prints the key material in full.
    pub fn key_fingerprints(&self, user: &str) -> Result<Vec<String>> {
        Ok(self
            .public_keys(user)?
            .into_iter()
            .map(|key| key.fingerprint(HashAlg::Sha256).to_string())
            .collect())
    }

    /// Find the user who owns `key`.
    pub fn user_for_key(&self, key: &PublicKey) -> Result<Option<String>> {
        let users = self.load_users()?;
        for user in &users.users {
            for stored in self.public_keys(&user.name)? {
                if stored == *key {
                    return Ok(Some(user.name.clone()));
                }
            }
        }
        Ok(None)
    }

    /// List `owner/name` repos on disk.
    pub fn list_repos(&self) -> Result<Vec<RepoName>> {
        let repos = self.root.join("repos");
        if !repos.exists() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for owner_ent in
            fs::read_dir(&repos).with_context(|| format!("read {}", repos.display()))?
        {
            let owner_ent = owner_ent?;
            if !owner_ent.file_type()?.is_dir() {
                continue;
            }
            let owner = owner_ent.file_name().to_string_lossy().into_owned();
            for repo_ent in fs::read_dir(owner_ent.path())? {
                let repo_ent = repo_ent?;
                if !repo_ent.file_type()?.is_dir() {
                    continue;
                }
                let fname = repo_ent.file_name().to_string_lossy().into_owned();
                let name = fname.strip_suffix(".git").unwrap_or(&fname);
                if let Ok(repo) = RepoName::parse(&format!("{owner}/{name}")) {
                    out.push(repo);
                }
            }
        }
        out.sort_by_key(|a| a.to_string());
        Ok(out)
    }

    /// Grant `role` to `user` on `repo`.
    pub fn grant(&self, user: &str, repo: &RepoName, role: Role) -> Result<()> {
        valid_user(user)?;
        if self.load_users()?.by_name(user).is_none() {
            bail!("user {user} not found");
        }
        let mut access = self.load_access()?;
        access
            .repos
            .entry(repo.to_string())
            .or_default()
            .insert(user.to_string(), role);
        self.save_access(&access)
    }

    /// Remove `user` from `repo` ACL.
    pub fn revoke(&self, user: &str, repo: &RepoName) -> Result<()> {
        let mut access = self.load_access()?;
        let Some(roles) = access.repos.get_mut(&repo.to_string()) else {
            bail!("no ACL entries for {repo}");
        };
        if roles.shift_remove(user).is_none() {
            bail!("{user} has no role on {repo}");
        }
        self.save_access(&access)
    }

    /// Load `tokens.yaml`.
    pub fn load_tokens(&self) -> Result<crate::auth::TokensFile> {
        read_yaml(&self.tokens_path())
    }

    /// Write `tokens.yaml` atomically (mode follows umask; keep the forge root private).
    pub fn save_tokens(&self, tokens: &crate::auth::TokensFile) -> Result<()> {
        write_yaml(&self.tokens_path(), tokens)
    }

    /// Load `visibility.yaml`.
    pub fn load_visibility(&self) -> Result<VisibilityFile> {
        read_yaml(&self.visibility_path())
    }

    /// Write `visibility.yaml`.
    pub fn save_visibility(&self, file: &VisibilityFile) -> Result<()> {
        write_yaml(&self.visibility_path(), file)
    }

    /// True when the repo is listed as public.
    pub fn is_public(&self, repo: &RepoName) -> Result<bool> {
        Ok(self
            .load_visibility()?
            .repos
            .get(&repo.to_string())
            .map(|value| value == "public")
            .unwrap_or(false))
    }

    /// Set `public` or `private` for `repo`.
    pub fn set_visibility(&self, repo: &RepoName, public: bool) -> Result<()> {
        let mut file = self.load_visibility()?;
        let key = repo.to_string();
        if public {
            file.repos.insert(key, "public".into());
        } else {
            file.repos.shift_remove(&key);
        }
        self.save_visibility(&file)
    }

    /// Replace the stored password hash (`None` clears it).
    pub fn set_password_hash(&self, user: &str, hash: Option<&str>) -> Result<()> {
        let mut users = self.load_users()?;
        let Some(record) = users.users.iter_mut().find(|u| u.name == user) else {
            bail!("user {user} not found");
        };
        record.password_hash = hash.map(str::to_string);
        self.save_users(&users)
    }
}

/// `users.yaml` document.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct UsersFile {
    /// Accounts allowed to authenticate (once they have keys).
    #[serde(default)]
    pub users: Vec<UserRecord>,
}

impl UsersFile {
    /// Lookup by login.
    pub fn by_name(&self, name: &str) -> Option<&UserRecord> {
        self.users.iter().find(|u| u.name == name)
    }

    /// Forge-wide admin flag.
    pub fn is_admin(&self, name: &str) -> bool {
        self.by_name(name).map(|u| u.admin).unwrap_or(false)
    }
}

/// One forge user.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UserRecord {
    /// Login name.
    pub name: String,
    /// When true, bypasses per-repo ACL.
    #[serde(default)]
    pub admin: bool,
    /// Argon2id PHC string for web login. SSH does not use this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password_hash: Option<String>,
}

/// `visibility.yaml` — `owner/name` → `public` (missing means private).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct VisibilityFile {
    /// Per-repo visibility.
    #[serde(default)]
    pub repos: indexmap::IndexMap<String, String>,
}

/// `access.yaml` document.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AccessFile {
    /// `owner/name` → user → role.
    #[serde(default)]
    pub repos: indexmap::IndexMap<String, indexmap::IndexMap<String, Role>>,
}

/// `builders.yaml` document.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BuildersFile {
    /// Registered agents.
    #[serde(default)]
    pub builders: Vec<BuilderRecord>,
}

/// One builder agent.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BuilderRecord {
    /// Forge login / agent name.
    pub name: String,
    /// Labels this agent claims (`linux`, `macos`, `windows`).
    #[serde(default)]
    pub labels: Vec<String>,
}

impl AccessFile {
    /// Role for `user` on `repo`, if any.
    pub fn role(&self, repo: &RepoName, user: &str) -> Option<Role> {
        self.repos.get(&repo.to_string())?.get(user).copied()
    }
}

fn read_yaml<T: for<'de> Deserialize<'de> + Default>(path: &Path) -> Result<T> {
    match fs::read_to_string(path) {
        Ok(text) if text.trim().is_empty() => Ok(T::default()),
        Ok(text) => serde_yml::from_str(&text).with_context(|| format!("parse {}", path.display())),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(err) => Err(err).with_context(|| format!("read {}", path.display())),
    }
}

fn write_yaml<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let text = serde_yml::to_string(value).context("serialize yaml")?;
    atomic_write(path, text.as_bytes())
}

/// Write `data` via a sibling temp file + rename.
pub fn atomic_write(path: &Path, data: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let tmp = path.with_extension("tmp");
    {
        let mut file =
            fs::File::create(&tmp).with_context(|| format!("create {}", tmp.display()))?;
        file.write_all(data)
            .with_context(|| format!("write {}", tmp.display()))?;
        file.sync_all()
            .with_context(|| format!("sync {}", tmp.display()))?;
    }
    fs::rename(&tmp, path).with_context(|| format!("rename {}", tmp.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn users_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path());
        store.ensure_layout().unwrap();
        store.add_user("alice", true).unwrap();
        assert!(store.load_users().unwrap().is_admin("alice"));
        store.remove_user("alice").unwrap();
        assert!(store.load_users().unwrap().by_name("alice").is_none());
    }
}
