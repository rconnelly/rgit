//! Clap command tree shared by the local binary and SSH management commands.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

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
    Add {
        /// Login name
        user: String,
        /// File containing one or more OpenSSH public keys
        #[arg(long)]
        file: PathBuf,
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
    List,
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
                | Commands::Hook { .. }
        )
    }
}
