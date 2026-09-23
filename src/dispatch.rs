//! Execute clap commands against a [`Store`].

use anyhow::{bail, Result};

use crate::acl::{self, Actor, Role};
use crate::cli::{
    AccessCommands, Commands, HookCommands, KeyCommands, RepoCommands, RequestCommands,
    RunCommands, UserCommands,
};
use crate::names::RepoName;
use crate::store::Store;
use crate::{git, hook, repo, request, runner};

/// Run a management command; returns stdout (no trailing requirement).
pub async fn execute(store: &Store, actor: &Actor, command: Commands) -> Result<String> {
    match command {
        Commands::User { command } => user(store, actor, command),
        Commands::Key { command } => key(store, actor, command),
        Commands::Repo { command } => repo_cmd(store, actor, command).await,
        Commands::Access { command } => access(store, actor, command),
        Commands::Request { command } => request_cmd(store, actor, command).await,
        Commands::Run { command } => run_cmd(store, actor, command),
        Commands::Hook { command } => hook_cmd(command),
        Commands::Init
        | Commands::Check
        | Commands::Status
        | Commands::Serve { .. }
        | Commands::Shell
        | Commands::Remote { .. } => {
            bail!("command is not available here")
        }
    }
}

fn user(store: &Store, actor: &Actor, command: UserCommands) -> Result<String> {
    match command {
        UserCommands::Add { name, admin } => {
            acl::require_forge_admin(store, actor)?;
            store.add_user(&name, admin)?;
            Ok(format!("user {name} added\n"))
        }
        UserCommands::List => {
            acl::require_forge_admin(store, actor)?;
            let mut lines = Vec::new();
            for u in store.load_users()?.users {
                let kind = if u.admin { "admin" } else { "user" };
                lines.push(format!("{} {kind}", u.name));
            }
            Ok(lines.join("\n") + "\n")
        }
        UserCommands::Remove { name } => {
            acl::require_forge_admin(store, actor)?;
            store.remove_user(&name)?;
            Ok(format!("user {name} removed\n"))
        }
    }
}

fn key(store: &Store, actor: &Actor, command: KeyCommands) -> Result<String> {
    match command {
        KeyCommands::Add {
            user,
            file,
            literal,
        } => {
            if !actor.is_forge_admin(store)? && actor.name() != Some(user.as_str()) {
                anyhow::bail!("only forge admins can add keys for other users");
            }
            let text = match (file, literal) {
                (Some(path), None) => std::fs::read_to_string(&path)
                    .map_err(|err| anyhow::anyhow!("read {}: {err}", path.display()))?,
                (None, Some(text)) => text,
                _ => anyhow::bail!("key add requires --file or --literal"),
            };
            let n = store.add_keys(&user, &text)?;
            Ok(format!("added {n} key(s) for {user}\n"))
        }
        KeyCommands::List { user } => {
            if !actor.is_forge_admin(store)? && actor.name() != Some(user.as_str()) {
                anyhow::bail!("only forge admins can list other users' keys");
            }
            let fps = store.key_fingerprints(&user)?;
            if fps.is_empty() {
                Ok(format!("no keys for {user}\n"))
            } else {
                Ok(fps.join("\n") + "\n")
            }
        }
        KeyCommands::Copy { .. } => {
            bail!("key copy is this-machine only; rgit origin key copy USER")
        }
    }
}

async fn repo_cmd(store: &Store, actor: &Actor, command: RepoCommands) -> Result<String> {
    match command {
        RepoCommands::Create { name } => {
            repo::create(store, actor, &name).await?;
            Ok(format!("created {name}\n"))
        }
        RepoCommands::List { user } => {
            let repos = if let Some(user) = user {
                repo::list_for_user(store, actor, &user)?
            } else {
                repo::list(store, actor)?
            };
            if repos.is_empty() {
                Ok(String::from("(no repositories)\n"))
            } else {
                Ok(repos
                    .into_iter()
                    .map(|r| r.to_string())
                    .collect::<Vec<_>>()
                    .join("\n")
                    + "\n")
            }
        }
        RepoCommands::Show { name } => repo::show(store, actor, &name),
    }
}

fn access(store: &Store, actor: &Actor, command: AccessCommands) -> Result<String> {
    match command {
        AccessCommands::Grant { user, repo, role } => {
            require_repo_admin(store, actor, &repo)?;
            store.grant(&user, &repo, role)?;
            Ok(format!("granted {role} on {repo} to {user}\n"))
        }
        AccessCommands::Revoke { user, repo } => {
            require_repo_admin(store, actor, &repo)?;
            store.revoke(&user, &repo)?;
            Ok(format!("revoked {user} on {repo}\n"))
        }
    }
}

fn require_repo_admin(store: &Store, actor: &Actor, repo: &RepoName) -> Result<()> {
    if actor.is_forge_admin(store)? {
        return Ok(());
    }
    acl::require(store, actor, repo, Role::Admin)?;
    Ok(())
}

async fn request_cmd(store: &Store, actor: &Actor, command: RequestCommands) -> Result<String> {
    match command {
        RequestCommands::Create {
            repo,
            head,
            base,
            title,
            body,
        } => {
            let meta = request::create(
                store,
                actor,
                &repo,
                &head,
                base.as_deref(),
                &title,
                body.as_deref(),
            )
            .await?;
            Ok(format!(
                "created request #{}\n{}",
                meta.id,
                request::format_meta(&meta)
            ))
        }
        RequestCommands::List { repo } => {
            let list = request::list(store, actor, &repo).await?;
            if list.is_empty() {
                return Ok("(no requests)\n".into());
            }
            Ok(list.iter().map(request::format_meta).collect::<String>())
        }
        RequestCommands::Show { repo, id } => {
            let meta = request::show(store, actor, &repo, id).await?;
            Ok(request::format_meta(&meta))
        }
        RequestCommands::Review {
            repo,
            id,
            approve,
            reject,
            comment,
        } => {
            let verdict = if approve {
                "approve"
            } else if reject {
                "reject"
            } else {
                "comment"
            };
            let meta =
                request::review(store, actor, &repo, id, verdict, comment.as_deref()).await?;
            Ok(request::format_meta(&meta))
        }
        RequestCommands::Merge { repo, id } => {
            let meta = request::merge(store, actor, &repo, id).await?;
            Ok(format!("merged #{id} into {}\n", meta.base_branch))
        }
    }
}

fn run_cmd(store: &Store, actor: &Actor, command: RunCommands) -> Result<String> {
    match command {
        RunCommands::List { repo } => {
            acl::require(store, actor, &repo, Role::Read)?;
            let runs = runner::list(store, &repo)?;
            if runs.is_empty() {
                return Ok("(no runs)\n".into());
            }
            let mut s = String::new();
            for run in runs {
                s.push_str(&format!(
                    "{} {} {} {} {}\n",
                    run.id, run.status, run.workflow, run.job, run.sha
                ));
            }
            Ok(s)
        }
        RunCommands::Show { repo, id } => {
            acl::require(store, actor, &repo, Role::Read)?;
            let run = runner::show(store, &repo, &id)?;
            Ok(serde_yml::to_string(&run)?)
        }
        RunCommands::Logs { repo, id } => {
            acl::require(store, actor, &repo, Role::Read)?;
            runner::logs(store, &repo, &id)
        }
    }
}

fn hook_cmd(command: HookCommands) -> Result<String> {
    match command {
        HookCommands::Update { refname, old, new } => {
            hook::update(&refname, &old, &new)?;
            Ok(String::new())
        }
    }
}

/// After `git-receive-pack`, allocate request ids and start workflows for new tips.
pub async fn after_receive(
    store: &Store,
    user: &str,
    repo: &RepoName,
    before: std::collections::BTreeMap<String, String>,
) -> Result<()> {
    let actor = Actor::User(user.to_string());
    request::promote_new_refs(store, &actor, repo).await?;
    let after = git::list_refs(&store.repo_path(repo)).await?;
    for (refname, sha) in after {
        if before.get(&refname) == Some(&sha) {
            continue;
        }
        crate::workflow::trigger_ref(store, repo, &refname, &sha).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use russh::keys::{Algorithm, PrivateKey};

    #[tokio::test]
    async fn key_add_literal() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path());
        store.ensure_layout().unwrap();
        store.add_user("ada", true).unwrap();
        let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
        let literal = key.public_key().to_openssh().unwrap();
        let out = execute(
            &store,
            &Actor::Operator,
            Commands::Key {
                command: KeyCommands::Add {
                    user: "ada".into(),
                    file: None,
                    literal: Some(literal),
                },
            },
        )
        .await
        .unwrap();
        assert!(out.contains("added 1"));
        assert!(!store.key_fingerprints("ada").unwrap().is_empty());
    }
}
