//! Web passwords and bearer tokens (`users.yaml` + `tokens.yaml`).
//!
//! SSH stays public-key only. The web UI (`rgit-web`) calls these commands with
//! `--json`. Tokens are stored as SHA-256 hashes; the raw token is shown once.

use anyhow::{bail, Context, Result};
use argon2::password_hash::{
    rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString,
};
use argon2::Argon2;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::acl::Actor;
use crate::names::valid_user;
use crate::now_iso;
use crate::output;
use crate::store::Store;

const TOKEN_PREFIX: &str = "rgit_";
const MIN_PASSWORD: usize = 8;

/// `rgit auth` subcommands.
#[derive(Clone, Debug)]
pub enum Command {
    /// Verify password and issue a token.
    Login { user: String, password: String },
    /// Revoke the current `--token`.
    Logout,
    /// Describe the current actor.
    Whoami,
    /// Issue a token without a password (operator / forge admin / self).
    TokenCreate { user: Option<String> },
    /// List token prefixes (no secrets).
    TokenList { user: Option<String> },
    /// Revoke by raw token or prefix.
    TokenRevoke { token: String },
}

/// Session issued by login / token create.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    /// Bearer token (`rgit_…`). Omitted on whoami.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// Forge login.
    pub user: String,
    /// Forge-wide admin.
    pub admin: bool,
    /// `user` or `operator` or `anonymous`.
    pub actor: String,
}

/// Public token listing row.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TokenInfo {
    /// First characters of the token (`rgit_` + 8 hex chars).
    pub prefix: String,
    /// Owner login.
    pub user: String,
    /// RFC 3339.
    pub created_at: String,
}

/// On-disk token row (hash only).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TokenRecord {
    /// SHA-256 hex of the raw token.
    pub hash: String,
    /// Display prefix.
    pub prefix: String,
    /// Owner login.
    pub user: String,
    /// RFC 3339.
    pub created_at: String,
}

/// `tokens.yaml`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TokensFile {
    /// Issued tokens.
    #[serde(default)]
    pub tokens: Vec<TokenRecord>,
}

/// Hash a password (PHC argon2id).
pub fn hash_password(password: &str) -> Result<String> {
    require_password_strength(password)?;
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|err| anyhow::anyhow!("hash password: {err}"))?;
    Ok(hash.to_string())
}

/// True when `password` matches the stored PHC string.
pub fn verify_password_hash(password: &str, stored: &str) -> Result<bool> {
    let parsed =
        PasswordHash::new(stored).map_err(|err| anyhow::anyhow!("parse password hash: {err}"))?;
    match Argon2::default().verify_password(password.as_bytes(), &parsed) {
        Ok(()) => Ok(true),
        Err(argon2::password_hash::Error::Password) => Ok(false),
        Err(err) => Err(anyhow::anyhow!("verify password: {err}")),
    }
}

fn require_password_strength(password: &str) -> Result<()> {
    if password.len() < MIN_PASSWORD {
        bail!("password must be at least {MIN_PASSWORD} characters");
    }
    Ok(())
}

/// SHA-256 hex of a bearer token.
pub fn token_hash(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn new_token() -> String {
    format!(
        "{TOKEN_PREFIX}{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

fn token_prefix(token: &str) -> String {
    let take = TOKEN_PREFIX.len() + 8;
    if token.len() <= take {
        token.to_string()
    } else {
        token[..take].to_string()
    }
}

/// Resolve a bearer token to a user actor.
pub fn actor_from_token(store: &Store, token: &str) -> Result<Actor> {
    let token = token.trim();
    if token.is_empty() {
        bail!("empty token");
    }
    let hash = token_hash(token);
    let file = store.load_tokens()?;
    let Some(row) = file.tokens.iter().find(|row| row.hash == hash) else {
        bail!("invalid token");
    };
    if store.load_users()?.by_name(&row.user).is_none() {
        bail!("token user {} not found", row.user);
    }
    Ok(Actor::User(row.user.clone()))
}

/// Set or replace a user's web password.
pub fn set_password(store: &Store, actor: &Actor, user: &str, password: &str) -> Result<()> {
    valid_user(user)?;
    if !actor.is_forge_admin(store)? && actor.name() != Some(user) {
        bail!("only forge admins can set another user's password");
    }
    if store.load_users()?.by_name(user).is_none() {
        bail!("user {user} not found");
    }
    let hash = hash_password(password)?;
    store.set_password_hash(user, Some(&hash))?;
    Ok(())
}

/// Run an auth command.
pub fn execute(
    store: &Store,
    actor: &Actor,
    command: Command,
    json: bool,
    token: Option<&str>,
) -> Result<String> {
    match command {
        Command::Login { user, password } => login(store, &user, &password, json),
        Command::Logout => logout(store, token, json),
        Command::Whoami => whoami(store, actor, json),
        Command::TokenCreate { user } => token_create(store, actor, user.as_deref(), json),
        Command::TokenList { user } => token_list(store, actor, user.as_deref(), json),
        Command::TokenRevoke { token: raw } => token_revoke(store, actor, &raw, json),
    }
}

fn login(store: &Store, user: &str, password: &str, json: bool) -> Result<String> {
    valid_user(user)?;
    let users = store.load_users()?;
    let Some(record) = users.by_name(user) else {
        bail!("invalid user or password");
    };
    let Some(hash) = record.password_hash.as_deref() else {
        bail!("user {user} has no web password; rgit user passwd {user}");
    };
    if !verify_password_hash(password, hash)? {
        bail!("invalid user or password");
    }
    let raw = store.issue_token(user)?;
    let session = Session {
        token: Some(raw),
        user: user.to_string(),
        admin: record.admin,
        actor: "user".into(),
    };
    output::pick(
        json,
        &session,
        format!(
            "logged in as {user}\ntoken {}\n",
            session.token.as_deref().unwrap_or("")
        ),
    )
}

fn logout(store: &Store, token: Option<&str>, json: bool) -> Result<String> {
    let Some(token) = token.map(str::trim).filter(|value| !value.is_empty()) else {
        bail!("logout requires --token");
    };
    store.revoke_token(token)?;
    output::pick(
        json,
        &serde_json::json!({ "ok": true }),
        "logged out\n".into(),
    )
}

fn whoami(store: &Store, actor: &Actor, json: bool) -> Result<String> {
    let session = match actor {
        Actor::Operator => Session {
            token: None,
            user: "operator".into(),
            admin: true,
            actor: "operator".into(),
        },
        Actor::Anonymous => Session {
            token: None,
            user: "anonymous".into(),
            admin: false,
            actor: "anonymous".into(),
        },
        Actor::User(name) => Session {
            token: None,
            user: name.clone(),
            admin: store.load_users()?.is_admin(name),
            actor: "user".into(),
        },
    };
    output::pick(
        json,
        &session,
        format!("{} ({})\n", session.user, session.actor),
    )
}

fn token_create(store: &Store, actor: &Actor, user: Option<&str>, json: bool) -> Result<String> {
    let target = match (user, actor) {
        (Some(name), _) => {
            if !actor.is_forge_admin(store)? && actor.name() != Some(name) {
                bail!("only forge admins can create tokens for other users");
            }
            name.to_string()
        }
        (None, Actor::User(name)) => name.clone(),
        (None, Actor::Operator) => bail!("token create needs --user"),
        (None, Actor::Anonymous) => bail!("sign in to create a token"),
    };
    if store.load_users()?.by_name(&target).is_none() {
        bail!("user {target} not found");
    }
    let raw = store.issue_token(&target)?;
    let admin = store.load_users()?.is_admin(&target);
    let session = Session {
        token: Some(raw),
        user: target,
        admin,
        actor: "user".into(),
    };
    output::pick(
        json,
        &session,
        format!("token {}\n", session.token.as_deref().unwrap_or("")),
    )
}

fn token_list(store: &Store, actor: &Actor, user: Option<&str>, json: bool) -> Result<String> {
    let admin = actor.is_forge_admin(store)?;
    if let Some(name) = user {
        if !admin && actor.name() != Some(name) {
            bail!("only forge admins can list other users' tokens");
        }
    } else if !admin && actor.name().is_none() {
        bail!("sign in to list tokens");
    }
    let rows: Vec<TokenInfo> = store
        .load_tokens()?
        .tokens
        .into_iter()
        .filter(|row| match (user, admin, actor.name()) {
            (Some(name), _, _) => row.user == name,
            (None, true, _) => true,
            (None, false, Some(name)) => row.user == name,
            (None, false, None) => false,
        })
        .map(|row| TokenInfo {
            prefix: row.prefix,
            user: row.user,
            created_at: row.created_at,
        })
        .collect();
    let text = if rows.is_empty() {
        "(no tokens)\n".into()
    } else {
        rows.iter()
            .map(|row| format!("{} {} {}\n", row.prefix, row.user, row.created_at))
            .collect()
    };
    output::pick(json, &serde_json::json!({ "tokens": rows }), text)
}

fn token_revoke(store: &Store, actor: &Actor, raw: &str, json: bool) -> Result<String> {
    let file = store.load_tokens()?;
    let hash = token_hash(raw.trim());
    let row = file
        .tokens
        .iter()
        .find(|row| row.hash == hash || row.prefix == raw.trim())
        .with_context(|| "token not found")?;
    if !actor.is_forge_admin(store)? && actor.name() != Some(row.user.as_str()) {
        bail!("only the owner or a forge admin can revoke this token");
    }
    let ident = if row.hash == hash {
        raw.trim().to_string()
    } else {
        row.hash.clone()
    };
    if row.hash == hash {
        store.revoke_token(raw.trim())?;
    } else {
        store.revoke_token_hash(&row.hash)?;
    }
    let _ = ident;
    output::pick(json, &serde_json::json!({ "ok": true }), "revoked\n".into())
}

impl Store {
    /// Issue a new bearer token for `user`.
    pub fn issue_token(&self, user: &str) -> Result<String> {
        valid_user(user)?;
        let raw = new_token();
        let mut file = self.load_tokens()?;
        file.tokens.push(TokenRecord {
            hash: token_hash(&raw),
            prefix: token_prefix(&raw),
            user: user.to_string(),
            created_at: now_iso(),
        });
        self.save_tokens(&file)?;
        Ok(raw)
    }

    /// Drop a token by raw secret.
    pub fn revoke_token(&self, token: &str) -> Result<()> {
        self.revoke_token_hash(&token_hash(token.trim()))
    }

    /// Drop a token by stored hash.
    pub fn revoke_token_hash(&self, hash: &str) -> Result<()> {
        let mut file = self.load_tokens()?;
        let before = file.tokens.len();
        file.tokens.retain(|row| row.hash != hash);
        if file.tokens.len() == before {
            bail!("token not found");
        }
        self.save_tokens(&file)
    }

    /// Drop every token for `user`.
    pub fn revoke_user_tokens(&self, user: &str) -> Result<()> {
        let mut file = self.load_tokens()?;
        file.tokens.retain(|row| row.user != user);
        self.save_tokens(&file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    #[test]
    fn password_round_trip() {
        let hash = hash_password("correct-horse").unwrap();
        assert!(verify_password_hash("correct-horse", &hash).unwrap());
        assert!(!verify_password_hash("wrong-password", &hash).unwrap());
    }

    #[test]
    fn token_login() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path());
        store.ensure_layout().unwrap();
        store.add_user("ada", true).unwrap();
        set_password(&store, &Actor::Operator, "ada", "correct-horse").unwrap();
        let out = login(&store, "ada", "correct-horse", true).unwrap();
        let session: Session = serde_json::from_str(out.trim()).unwrap();
        let token = session.token.expect("token");
        let actor = actor_from_token(&store, &token).unwrap();
        assert_eq!(actor, Actor::User("ada".into()));
        store.revoke_token(&token).unwrap();
        assert!(actor_from_token(&store, &token).is_err());
    }
}
