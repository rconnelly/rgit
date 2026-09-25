//! Execute clap commands against a [`Store`].

use anyhow::{bail, Result};
use serde::Serialize;

use crate::acl::{self, Actor, Role};
use crate::auth;
use crate::browse;
use crate::cli::{
    AccessCommands, AuthCommands, Commands, HookCommands, KeyCommands, RepoCommands,
    RequestCommands, RunCommands, TokenCommands, UserCommands,
};
use crate::names::RepoName;
use crate::output;
use crate::store::Store;
use crate::{device, git, hook, repo, request, runner};

/// Run a management command; returns stdout (no trailing requirement).
pub async fn execute(store: &Store, actor: &Actor, command: Commands) -> Result<String> {
    execute_fmt(store, actor, command, false, None).await
}

/// Run a management command, optionally as JSON.
pub async fn execute_fmt(
    store: &Store,
    actor: &Actor,
    command: Commands,
    json: bool,
    token: Option<&str>,
) -> Result<String> {
    match command {
        Commands::Auth {
            command: AuthCommands::Device { command },
        } => device::execute(store, actor, command, json),
        Commands::Auth { command } => auth_cmd(store, actor, command, json, token),
        Commands::User { command } => user(store, actor, command, json),
        Commands::Key { command } => key(store, actor, command, json),
        Commands::Repo { command } => repo_cmd(store, actor, command, json).await,
        Commands::Access { command } => access(store, actor, command, json),
        Commands::Request { command } => request_cmd(store, actor, command, json).await,
        Commands::Run { command } => run_cmd(store, actor, command, json),
        Commands::Hook { command } => hook_cmd(command).await,
        Commands::Agent {
            command: Some(command),
            ..
        } => crate::agent::execute(store, actor, command),
        Commands::Init
        | Commands::Check
        | Commands::Status
        | Commands::View { .. }
        | Commands::Version { .. }
        | Commands::Serve { .. }
        | Commands::Shell
        | Commands::Remote { .. }
        | Commands::Login { .. }
        | Commands::Logout { .. }
        | Commands::Agent { command: None, .. } => {
            bail!("command is not available here")
        }
    }
}

fn auth_cmd(
    store: &Store,
    actor: &Actor,
    command: AuthCommands,
    json: bool,
    token: Option<&str>,
) -> Result<String> {
    let inner = match command {
        AuthCommands::Login { user, password } => auth::Command::Login { user, password },
        AuthCommands::Register { user, password } => auth::Command::Register { user, password },
        AuthCommands::Logout => auth::Command::Logout,
        AuthCommands::Whoami => auth::Command::Whoami,
        AuthCommands::Token {
            command: TokenCommands::Create { user },
        } => auth::Command::TokenCreate { user },
        AuthCommands::Token {
            command: TokenCommands::List { user },
        } => auth::Command::TokenList { user },
        AuthCommands::Token {
            command: TokenCommands::Revoke { token },
        } => auth::Command::TokenRevoke { token },
        AuthCommands::Device { .. } => unreachable!("device commands are handled in execute_fmt"),
    };
    auth::execute(store, actor, inner, json, token)
}

fn user(store: &Store, actor: &Actor, command: UserCommands, json: bool) -> Result<String> {
    match command {
        UserCommands::Add {
            name,
            admin,
            password,
        } => {
            acl::require_forge_admin(store, actor)?;
            store.add_user(&name, admin)?;
            if let Some(password) = password {
                auth::set_password(store, actor, &name, &password)?;
            }
            output::pick(
                json,
                &serde_json::json!({ "user": name, "admin": admin }),
                format!("user {name} added\n"),
            )
        }
        UserCommands::List => {
            acl::require_forge_admin(store, actor)?;
            #[derive(Serialize)]
            struct Row {
                name: String,
                admin: bool,
                has_password: bool,
            }
            let rows: Vec<Row> = store
                .load_users()?
                .users
                .into_iter()
                .map(|u| Row {
                    name: u.name,
                    admin: u.admin,
                    has_password: u.password_hash.is_some(),
                })
                .collect();
            let text = if rows.is_empty() {
                String::new()
            } else {
                rows.iter()
                    .map(|u| format!("{} {}\n", u.name, if u.admin { "admin" } else { "user" }))
                    .collect()
            };
            output::pick(json, &serde_json::json!({ "users": rows }), text)
        }
        UserCommands::Remove { name } => {
            acl::require_forge_admin(store, actor)?;
            store.remove_user(&name)?;
            output::pick(
                json,
                &serde_json::json!({ "removed": name }),
                format!("user {name} removed\n"),
            )
        }
        UserCommands::Passwd { name, password } => {
            auth::set_password(store, actor, &name, &password)?;
            output::pick(
                json,
                &serde_json::json!({ "user": name, "ok": true }),
                format!("password set for {name}\n"),
            )
        }
    }
}

fn key(store: &Store, actor: &Actor, command: KeyCommands, json: bool) -> Result<String> {
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
            output::pick(
                json,
                &serde_json::json!({ "user": user, "added": n }),
                format!("added {n} key(s) for {user}\n"),
            )
        }
        KeyCommands::List { user } => {
            if !actor.is_forge_admin(store)? && actor.name() != Some(user.as_str()) {
                anyhow::bail!("only forge admins can list other users' keys");
            }
            let fps = store.key_fingerprints(&user)?;
            let text = if fps.is_empty() {
                format!("no keys for {user}\n")
            } else {
                fps.join("\n") + "\n"
            };
            output::pick(
                json,
                &serde_json::json!({ "user": user, "fingerprints": fps }),
                text,
            )
        }
        KeyCommands::Copy { .. } => {
            bail!("key copy is this-machine only; rgit origin key copy USER")
        }
    }
}

async fn repo_cmd(
    store: &Store,
    actor: &Actor,
    command: RepoCommands,
    json: bool,
) -> Result<String> {
    match command {
        RepoCommands::Create { name, public } => {
            repo::create(store, actor, &name).await?;
            if public {
                store.set_visibility(&name, true)?;
            }
            output::pick(
                json,
                &serde_json::json!({
                    "name": name.to_string(),
                    "visibility": if public { "public" } else { "private" },
                    "clone_url": repo::clone_url(&name),
                }),
                format!("created {name}\n"),
            )
        }
        RepoCommands::List { user } => {
            let repos = if let Some(user) = user {
                repo::list_for_user(store, actor, &user)?
            } else {
                repo::list(store, actor)?
            };
            let mut summaries = Vec::new();
            for name in &repos {
                let info = repo::info(store, actor, name)
                    .await
                    .unwrap_or(repo::RepoInfo {
                        name: name.to_string(),
                        path: store.repo_path(name).display().to_string(),
                        visibility: if store.is_public(name).unwrap_or(false) {
                            "public".into()
                        } else {
                            "private".into()
                        },
                        default_branch: None,
                        clone_url: repo::clone_url(name),
                        description: None,
                        role: acl::role(store, actor, name)?.map(|r| r.to_string()),
                        access: Vec::new(),
                    });
                summaries.push(info);
            }
            let text = if repos.is_empty() {
                String::from("(no repositories)\n")
            } else {
                repos
                    .iter()
                    .map(|r| r.to_string())
                    .collect::<Vec<_>>()
                    .join("\n")
                    + "\n"
            };
            output::pick(json, &serde_json::json!({ "repos": summaries }), text)
        }
        RepoCommands::Show { name } => {
            let info = repo::info(store, actor, &name).await?;
            output::pick(json, &info, repo::show(store, actor, &name)?)
        }
        RepoCommands::Tree {
            name,
            git_ref,
            path,
        } => {
            let tree = browse::tree(store, actor, &name, &git_ref, &path).await?;
            let text = tree
                .entries
                .iter()
                .map(|e| format!("{}\t{}\n", e.kind, e.name))
                .collect();
            browse::emit(json, &tree, text)
        }
        RepoCommands::Blob {
            name,
            git_ref,
            path,
        } => {
            let blob = browse::blob(store, actor, &name, &git_ref, &path).await?;
            let text = blob.content.clone().unwrap_or_else(|| {
                if blob.binary {
                    format!("(binary {} bytes)\n", blob.size)
                } else {
                    format!("(truncated {} bytes)\n", blob.size)
                }
            });
            browse::emit(json, &blob, text)
        }
        RepoCommands::Blame {
            name,
            git_ref,
            path,
        } => {
            let blame = browse::blame(store, actor, &name, &git_ref, &path).await?;
            let text = blame
                .lines
                .iter()
                .map(|l| format!("{:.8} {:>4} {}\n", l.sha, l.line, l.text))
                .collect();
            browse::emit(json, &blame, text)
        }
        RepoCommands::Log {
            name,
            git_ref,
            path,
            limit,
        } => {
            let log = browse::log(store, actor, &name, &git_ref, path.as_deref(), limit).await?;
            let text = log
                .commits
                .iter()
                .map(|c| format!("{} {}\n", c.short, c.subject))
                .collect();
            browse::emit(json, &log, text)
        }
        RepoCommands::Commit { name, sha } => {
            let commit = browse::commit(store, actor, &name, &sha).await?;
            let text = format!(
                "{} {}\n{} <{}>\n{}\n\n{}\n{}\n",
                commit.sha,
                commit.short,
                commit.author,
                commit.email,
                commit.date,
                commit.subject,
                commit.body
            );
            browse::emit(json, &commit, text)
        }
        RepoCommands::Refs { name } => {
            let refs = browse::refs(store, actor, &name).await?;
            let mut text = String::new();
            for b in &refs.branches {
                text.push_str(&format!("branch {}\t{}\n", b.name, b.sha));
            }
            for t in &refs.tags {
                text.push_str(&format!("tag {}\t{}\n", t.name, t.sha));
            }
            browse::emit(json, &refs, text)
        }
        RepoCommands::Diff { name, base, head } => {
            let diff = browse::diff(store, actor, &name, &base, &head).await?;
            browse::emit(json, &diff, diff.diff.clone())
        }
        RepoCommands::Visibility {
            name,
            public,
            private,
        } => {
            acl::require(store, actor, &name, Role::Admin)?;
            if !public && !private {
                bail!("pass --public or --private");
            }
            store.set_visibility(&name, public)?;
            let vis = if public { "public" } else { "private" };
            output::pick(
                json,
                &serde_json::json!({ "name": name.to_string(), "visibility": vis }),
                format!("{name} is {vis}\n"),
            )
        }
    }
}

fn access(store: &Store, actor: &Actor, command: AccessCommands, json: bool) -> Result<String> {
    match command {
        AccessCommands::Grant { user, repo, role } => {
            require_repo_admin(store, actor, &repo)?;
            store.grant(&user, &repo, role)?;
            output::pick(
                json,
                &serde_json::json!({ "user": user, "repo": repo.to_string(), "role": role.to_string() }),
                format!("granted {role} on {repo} to {user}\n"),
            )
        }
        AccessCommands::Revoke { user, repo } => {
            require_repo_admin(store, actor, &repo)?;
            store.revoke(&user, &repo)?;
            output::pick(
                json,
                &serde_json::json!({ "user": user, "repo": repo.to_string() }),
                format!("revoked {user} on {repo}\n"),
            )
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

async fn request_cmd(
    store: &Store,
    actor: &Actor,
    command: RequestCommands,
    json: bool,
) -> Result<String> {
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
            output::pick(
                json,
                &meta,
                format!(
                    "created request #{}\n{}",
                    meta.id,
                    request::format_meta(&meta)
                ),
            )
        }
        RequestCommands::List { repo } => {
            let list = request::list(store, actor, &repo).await?;
            let text = if list.is_empty() {
                "(no requests)\n".into()
            } else {
                list.iter().map(request::format_meta).collect::<String>()
            };
            output::pick(json, &serde_json::json!({ "requests": list }), text)
        }
        RequestCommands::Show { repo, id } => {
            let meta = request::show(store, actor, &repo, id).await?;
            let (head, base) = request::shas(&store.repo_path(&repo), id)
                .await
                .unwrap_or_default();
            output::pick(
                json,
                &serde_json::json!({
                    "id": meta.id,
                    "title": meta.title,
                    "author": meta.author,
                    "body": meta.body,
                    "state": meta.state,
                    "base_branch": meta.base_branch,
                    "head_branch": meta.head_branch,
                    "created_at": meta.created_at,
                    "reviews": meta.reviews,
                    "head_sha": head,
                    "base_sha": base,
                }),
                request::format_meta(&meta),
            )
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
            output::pick(json, &meta, request::format_meta(&meta))
        }
        RequestCommands::Merge { repo, id } => {
            let meta = request::merge(store, actor, &repo, id).await?;
            output::pick(
                json,
                &meta,
                format!("merged #{id} into {}\n", meta.base_branch),
            )
        }
        RequestCommands::Diff { repo, id } => {
            acl::require(store, actor, &repo, Role::Read)?;
            let (head, base) = request::shas(&store.repo_path(&repo), id).await?;
            let diff = browse::diff(store, actor, &repo, &base, &head).await?;
            browse::emit(json, &diff, diff.diff.clone())
        }
    }
}

fn run_cmd(store: &Store, actor: &Actor, command: RunCommands, json: bool) -> Result<String> {
    match command {
        RunCommands::List { repo } => {
            acl::require(store, actor, &repo, Role::Read)?;
            let runs = runner::list(store, &repo)?;
            let text = if runs.is_empty() {
                "(no runs)\n".into()
            } else {
                let mut s = String::new();
                for run in &runs {
                    s.push_str(&format!(
                        "{} {} {} {} {} {} {}\n",
                        run.id,
                        run.status,
                        run.workflow,
                        run.job,
                        run.sha,
                        if run.runs_on.is_empty() {
                            "host"
                        } else {
                            run.runs_on.as_str()
                        },
                        if run.agent.is_empty() {
                            "-"
                        } else {
                            run.agent.as_str()
                        }
                    ));
                }
                s
            };
            output::pick(json, &serde_json::json!({ "runs": runs }), text)
        }
        RunCommands::Show { repo, id } => {
            acl::require(store, actor, &repo, Role::Read)?;
            let run = runner::show(store, &repo, &id)?;
            if json {
                output::to_json(&run)
            } else {
                Ok(serde_yml::to_string(&run)?)
            }
        }
        RunCommands::Logs { repo, id } => {
            acl::require(store, actor, &repo, Role::Read)?;
            let logs = runner::logs(store, &repo, &id)?;
            output::pick(json, &serde_json::json!({ "logs": logs }), logs)
        }
    }
}

async fn hook_cmd(command: HookCommands) -> Result<String> {
    match command {
        HookCommands::Update { refname, old, new } => {
            hook::update(&refname, &old, &new).await?;
            Ok(String::new())
        }
        HookCommands::CommitMsg { path } => {
            hook::commit_msg(&path)?;
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
    use crate::cli::AgentCommands;
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

    #[tokio::test]
    async fn agent_register_and_list() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path());
        store.ensure_layout().unwrap();
        store.add_user("ada", true).unwrap();
        let out = execute(
            &store,
            &Actor::Operator,
            Commands::Agent {
                labels: Vec::new(),
                remote: None,
                command: Some(AgentCommands::Register {
                    name: "mac".into(),
                    labels: vec!["macos".into()],
                    file: None,
                    literal: None,
                }),
            },
        )
        .await
        .unwrap();
        assert!(out.contains("registered"));
        let list = execute(
            &store,
            &Actor::Operator,
            Commands::Agent {
                labels: Vec::new(),
                remote: None,
                command: Some(AgentCommands::List),
            },
        )
        .await
        .unwrap();
        assert!(list.contains("mac macos"));
        assert!(store.has_builder_label("macos"));
    }
}
