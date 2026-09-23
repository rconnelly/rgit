//! Clap command tree shared by the local binary and SSH management commands.

use std::path::PathBuf;

use clap::{ArgGroup, Parser, Subcommand};

use crate::acl::Role;
use crate::names::RepoName;

/// Self-hosted git forge: SSH remotes, merge requests, YAML workflows.
#[derive(Parser)]
#[command(
    name = "rabun-git",
    version,
    about = "Self-hosted git forge: SSH remotes, merge requests, YAML workflows"
)]
pub struct Cli {
    /// Path to rabun-git.toml
    #[arg(long, global = true, env = "RABUN_GIT_CONFIG")]
    pub config: Option<PathBuf>,
    /// SSH private key for a named remote (`rabun-git origin …`)
    #[arg(long, global = true, env = "RABUN_GIT_SSH_IDENTITY")]
    pub identity: Option<PathBuf>,
    #[command(subcommand)]
    pub command: Commands,
}

/// Top-level subcommands.
#[derive(Subcommand)]
pub enum Commands {
    /// Write rabun-git.toml, .env.example, and an empty forge root
    Init,
    /// Verify data root, git, SSH bind, and an admin key
    Check,
    /// Print companion status (`rabun.companion/v1`, no keys)
    Status,
    /// SSH git + management commands (loopback health)
    Serve {
        /// SSH bind (overrides config / `RABUN_GIT_SSH_BIND`)
        #[arg(long)]
        bind: Option<String>,
    },
    /// Interactive bash as the systemd user (`rabun-git`; host only)
    Shell,
    /// Laptop-only named forge hosts (`~/.config/rabun-git/remotes.toml`)
    Remote {
        #[command(subcommand)]
        command: RemoteCommands,
    },
    /// Forge users (`users.yaml`)
    User {
        #[command(subcommand)]
        command: UserCommands,
    },
    /// SSH public keys (`keys/<user>.pub`)
    Key {
        #[command(subcommand)]
        command: KeyCommands,
    },
    /// Bare repositories
    Repo {
        #[command(subcommand)]
        command: RepoCommands,
    },
    /// Per-repo ACL (`access.yaml`)
    Access {
        #[command(subcommand)]
        command: AccessCommands,
    },
    /// Merge requests (`refs/rabun/requests/*`)
    Request {
        #[command(subcommand)]
        command: RequestCommands,
    },
    /// Workflow run logs
    Run {
        #[command(subcommand)]
        command: RunCommands,
    },
    /// Git hooks (called from `hooks/update`, not over SSH)
    #[command(hide = true)]
    Hook {
        #[command(subcommand)]
        command: HookCommands,
    },
}

/// `rabun-git user` subcommands.
#[derive(Subcommand)]
pub enum UserCommands {
    /// Add a user
    Add {
        /// Login name
        name: String,
        /// Forge-wide admin (bypass ACL, manage users)
        #[arg(long)]
        admin: bool,
    },
    /// List users
    List,
    /// Remove a user and their keys
    Remove {
        /// Login name
        name: String,
    },
}

/// `rabun-git key` subcommands.
#[derive(Subcommand)]
pub enum KeyCommands {
    /// Append an OpenSSH public key for a user
    #[command(group(ArgGroup::new("key_src").required(true).args(["file", "literal"])))]
    Add {
        /// Login name
        user: String,
        /// File containing one or more OpenSSH public keys
        #[arg(long)]
        file: Option<PathBuf>,
        /// OpenSSH public key text (laptop client / SSH)
        #[arg(long)]
        literal: Option<String>,
    },
    /// List keys for a user (fingerprints, no secrets)
    List {
        /// Login name
        user: String,
    },
}

/// `rabun-git repo` subcommands.
#[derive(Subcommand)]
pub enum RepoCommands {
    /// Create a bare repository
    Create {
        /// `owner/name`
        name: RepoName,
    },
    /// List repositories the actor can read
    List {
        /// Forge login; list repositories that user can access
        #[arg(long)]
        user: Option<String>,
    },
    /// Show one repository
    Show {
        /// `owner/name`
        name: RepoName,
    },
}

/// `rabun-git access` subcommands.
#[derive(Subcommand)]
pub enum AccessCommands {
    /// Grant a role on a repository
    Grant {
        /// Login name
        user: String,
        /// `owner/name`
        repo: RepoName,
        /// `read`, `write`, or `admin`
        #[arg(long, default_value = "write")]
        role: Role,
    },
    /// Revoke a user's role on a repository
    Revoke {
        /// Login name
        user: String,
        /// `owner/name`
        repo: RepoName,
    },
}

/// `rabun-git request` subcommands.
#[derive(Subcommand)]
pub enum RequestCommands {
    /// Open a merge request from an existing branch or SHA
    Create {
        /// `owner/name`
        repo: RepoName,
        /// Branch name or commit SHA
        #[arg(long)]
        head: String,
        /// Target branch (default: HEAD of the bare repo)
        #[arg(long)]
        base: Option<String>,
        /// Short title
        #[arg(long)]
        title: String,
        /// Optional body
        #[arg(long)]
        body: Option<String>,
    },
    /// List merge requests
    List {
        /// `owner/name`
        repo: RepoName,
    },
    /// Show one merge request
    Show {
        /// `owner/name`
        repo: RepoName,
        /// Request id
        id: u64,
    },
    /// Add a review
    Review {
        /// `owner/name`
        repo: RepoName,
        /// Request id
        id: u64,
        /// Approve (fast-forward merge still requires `request merge`)
        #[arg(long, conflicts_with_all = ["reject"])]
        approve: bool,
        /// Reject
        #[arg(long)]
        reject: bool,
        /// Comment text (optional with `--approve` / `--reject`)
        #[arg(long)]
        comment: Option<String>,
    },
    /// Fast-forward the base branch to the request head
    Merge {
        /// `owner/name`
        repo: RepoName,
        /// Request id
        id: u64,
    },
}

/// `rabun-git run` subcommands.
#[derive(Subcommand)]
pub enum RunCommands {
    /// List workflow runs
    List {
        /// `owner/name`
        repo: RepoName,
    },
    /// Show run status YAML
    Show {
        /// `owner/name`
        repo: RepoName,
        /// Run id
        id: String,
    },
    /// Print captured log
    Logs {
        /// `owner/name`
        repo: RepoName,
        /// Run id
        id: String,
    },
}

/// Laptop-only `rabun-git remote` subcommands.
#[derive(Subcommand)]
pub enum RemoteCommands {
    /// Save a forge host alias (default name: `origin`)
    Add {
        /// Alias used as `rabun-git <name> …`
        name: String,
        /// `HOST`, `user@HOST`, `user@HOST:port`, or `ssh://user@HOST:port`
        url: String,
        /// SSH private key for this remote
        #[arg(long)]
        identity: Option<PathBuf>,
    },
    /// List saved remotes
    List,
    /// Show one remote
    Show {
        /// Alias
        name: String,
    },
    /// Delete a saved remote
    Remove {
        /// Alias
        name: String,
    },
}

/// Internal git hook entrypoints.
#[derive(Subcommand)]
pub enum HookCommands {
    /// `hooks/update` — reject protected branch pushes for non-admins
    Update {
        /// Ref being updated
        refname: String,
        /// Previous object name
        old: String,
        /// New object name
        new: String,
    },
}

impl Commands {
    /// Commands that must not be exposed over SSH.
    pub fn ssh_forbidden(&self) -> bool {
        matches!(
            self,
            Commands::Init
                | Commands::Check
                | Commands::Status
                | Commands::Serve { .. }
                | Commands::Shell
                | Commands::Remote { .. }
                | Commands::Hook { .. }
        )
    }

    /// Host commands that write forge data and must run as the systemd user.
    pub fn requires_service_uid(&self) -> bool {
        matches!(
            self,
            Commands::Init
                | Commands::User { .. }
                | Commands::Key { .. }
                | Commands::Repo { .. }
                | Commands::Access { .. }
                | Commands::Request { .. }
                | Commands::Run { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_is_host_only() {
        assert!(Commands::Shell.ssh_forbidden());
        assert!(!Commands::Shell.requires_service_uid());
        assert!(Commands::User {
            command: UserCommands::List
        }
        .requires_service_uid());
        assert!(!Commands::Check.requires_service_uid());
        assert!(Commands::Remote {
            command: RemoteCommands::List
        }
        .ssh_forbidden());
        assert!(!Commands::Remote {
            command: RemoteCommands::List
        }
        .requires_service_uid());
    }
}
