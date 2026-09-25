//! Clap command tree shared by the local binary and SSH management commands.

use std::path::PathBuf;

use clap::{ArgGroup, CommandFactory, FromArgMatches, Parser, Subcommand};

use crate::acl::Role;
use crate::names::RepoName;

/// Self-hosted git forge: SSH remotes, merge requests, YAML workflows.
#[derive(Parser)]
#[command(
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
    /// Machine-readable JSON on stdout (for `rgit-web` and scripts)
    #[arg(long, global = true, env = "RABUN_GIT_JSON")]
    pub json: bool,
    /// Web session token (`rgit_…`)
    #[arg(long, global = true, env = "RABUN_GIT_TOKEN")]
    pub token: Option<String>,
    /// Unauthenticated actor (public repos only)
    #[arg(long, global = true)]
    pub anonymous: bool,
    #[command(subcommand)]
    pub command: Commands,
}

/// Basename used in clap help (`rgit` or `rabun-git`).
pub fn invoked_name() -> String {
    std::env::args_os()
        .next()
        .as_ref()
        .map(std::path::Path::new)
        .and_then(|path| path.file_name())
        .and_then(|name| name.to_str())
        .filter(|name| *name == "rgit" || *name == "rabun-git")
        .unwrap_or("rabun-git")
        .to_string()
}

/// Parse argv using the invoked basename so `rgit --help` says `Usage: rgit`.
pub fn parse() -> Cli {
    parse_from(std::env::args_os())
}

/// Parse an explicit argv (first element is the program name).
pub fn parse_from<I, T>(itr: I) -> Cli
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    let args: Vec<std::ffi::OsString> = itr.into_iter().map(Into::into).collect();
    let name = match args.first().and_then(|arg| {
        std::path::Path::new(arg)
            .file_name()
            .and_then(|name| name.to_str())
    }) {
        Some("rgit") => "rgit",
        _ => "rabun-git",
    };
    let cmd = Cli::command().name(name);
    let matches = cmd
        .try_get_matches_from(args)
        .unwrap_or_else(|err| err.exit());
    Cli::from_arg_matches(&matches).unwrap_or_else(|err| err.exit())
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
    /// Loopback Zola preview of a git tree (this machine only)
    View {
        /// Path or `owner/name` (default: current directory)
        target: Option<String>,
        /// Git revision (default HEAD)
        #[arg(long = "ref", default_value = "HEAD")]
        git_ref: String,
        /// Loopback address (default 127.0.0.1:1111)
        #[arg(long, default_value = "127.0.0.1:1111")]
        bind: String,
        /// Open the preview in a browser
        #[arg(long)]
        open: bool,
    },
    /// SemVer 2.0, Conventional Commits, and CHANGELOG.md (this machine)
    Version {
        #[command(subcommand)]
        command: VersionCommands,
    },
    /// SSH git + management commands (loopback health)
    Serve {
        /// SSH bind (overrides config / `RABUN_GIT_SSH_BIND`)
        #[arg(long)]
        bind: Option<String>,
    },
    /// Interactive bash as the systemd user (`rabun-git`; host only)
    Shell,
    /// Named forge hosts on this machine (`~/.config/rabun-git/remotes.toml`)
    Remote {
        #[command(subcommand)]
        command: RemoteCommands,
    },
    /// Web passwords and session tokens
    Auth {
        #[command(subcommand)]
        command: AuthCommands,
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
    /// Builder agents (`rgit agent --labels macos` or `rgit origin agent …`)
    Agent {
        /// Labels this process claims (poll loop)
        #[arg(long)]
        labels: Vec<String>,
        /// Named remote for the poll loop (default `origin`)
        #[arg(long)]
        remote: Option<String>,
        #[command(subcommand)]
        command: Option<AgentCommands>,
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
        /// Web password (SSH still uses keys)
        #[arg(long, env = "RABUN_GIT_PASSWORD")]
        password: Option<String>,
    },
    /// List users
    List,
    /// Remove a user and their keys
    Remove {
        /// Login name
        name: String,
    },
    /// Set a web password
    Passwd {
        /// Login name
        name: String,
        /// New password
        #[arg(long, env = "RABUN_GIT_PASSWORD")]
        password: String,
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
        /// OpenSSH public key text (this machine / SSH)
        #[arg(long)]
        literal: Option<String>,
    },
    /// List keys for a user (fingerprints, no secrets)
    List {
        /// Login name
        user: String,
    },
    /// Copy a public key to a named remote over host SSH (port 22)
    Copy {
        /// Forge login (default: this machine's username)
        user: Option<String>,
        /// Public key file (default: `~/.ssh/id_ed25519.pub`)
        #[arg(long)]
        file: Option<PathBuf>,
        /// Create the forge user as admin if missing
        #[arg(long)]
        admin: bool,
        /// Host SSH (`user@HOST`, default `$USER@<forge-host>:22`)
        #[arg(long)]
        host: Option<String>,
    },
}

/// `rabun-git repo` subcommands.
#[derive(Subcommand)]
pub enum RepoCommands {
    /// Create a bare repository
    Create {
        /// `owner/name`
        name: RepoName,
        /// Anyone may clone and browse (still write-protected)
        #[arg(long)]
        public: bool,
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
    /// Directory listing at a revision
    Tree {
        /// `owner/name`
        name: RepoName,
        /// Git revision (default HEAD)
        #[arg(long = "ref", default_value = "HEAD")]
        git_ref: String,
        /// Directory path (default: repository root)
        #[arg(long, default_value = "")]
        path: String,
    },
    /// File contents at a revision
    Blob {
        /// `owner/name`
        name: RepoName,
        /// Git revision (default HEAD)
        #[arg(long = "ref", default_value = "HEAD")]
        git_ref: String,
        /// File path
        #[arg(long)]
        path: String,
    },
    /// `git blame` for a file
    Blame {
        /// `owner/name`
        name: RepoName,
        /// Git revision (default HEAD)
        #[arg(long = "ref", default_value = "HEAD")]
        git_ref: String,
        /// File path
        #[arg(long)]
        path: String,
    },
    /// Commit history
    Log {
        /// `owner/name`
        name: RepoName,
        /// Git revision (default HEAD)
        #[arg(long = "ref", default_value = "HEAD")]
        git_ref: String,
        /// Optional path filter
        #[arg(long)]
        path: Option<String>,
        /// Maximum commits (default 50)
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
    /// One commit
    Commit {
        /// `owner/name`
        name: RepoName,
        /// Commit SHA or ref
        sha: String,
    },
    /// Branches and tags
    Refs {
        /// `owner/name`
        name: RepoName,
    },
    /// Unified diff between two revisions
    Diff {
        /// `owner/name`
        name: RepoName,
        /// Base revision
        #[arg(long)]
        base: String,
        /// Head revision
        #[arg(long)]
        head: String,
    },
    /// Set public or private
    Visibility {
        /// `owner/name`
        name: RepoName,
        /// Anyone may browse
        #[arg(long, conflicts_with = "private")]
        public: bool,
        /// ACL only
        #[arg(long)]
        private: bool,
    },
}

/// `rabun-git auth` subcommands (web passwords and tokens).
#[derive(Subcommand)]
pub enum AuthCommands {
    /// Verify a password and issue a bearer token
    Login {
        /// Login name
        #[arg(long)]
        user: String,
        /// Web password
        #[arg(long, env = "RABUN_GIT_PASSWORD")]
        password: String,
    },
    /// Create a non-admin user with a web password (`--anonymous`; rgit-web sign-up)
    Register {
        /// Login name
        #[arg(long)]
        user: String,
        /// Web password
        #[arg(long, env = "RABUN_GIT_PASSWORD")]
        password: String,
    },
    /// Revoke the current `--token`
    Logout,
    /// Describe the current actor
    Whoami,
    /// Bearer tokens
    Token {
        #[command(subcommand)]
        command: TokenCommands,
    },
}

/// `rabun-git auth token` subcommands.
#[derive(Subcommand)]
pub enum TokenCommands {
    /// Issue a token (no password)
    Create {
        /// Login name (default: current user)
        user: Option<String>,
    },
    /// List token prefixes
    List {
        /// Login name (default: current user, or all for admins)
        #[arg(long)]
        user: Option<String>,
    },
    /// Revoke by raw token or prefix
    Revoke {
        /// Raw token or prefix
        token: String,
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
    /// Unified diff and commits for a request
    Diff {
        /// `owner/name`
        repo: RepoName,
        /// Request id
        id: u64,
    },
}

/// `rabun-git agent` subcommands (over SSH except the poll loop).
#[derive(Subcommand)]
pub enum AgentCommands {
    /// Register a builder (forge admin)
    Register {
        /// Builder / forge login
        name: String,
        /// Labels this builder claims
        #[arg(long = "label", alias = "labels")]
        labels: Vec<String>,
        /// Public key file
        #[arg(long)]
        file: Option<PathBuf>,
        /// Public key text
        #[arg(long)]
        literal: Option<String>,
    },
    /// List builders
    List,
    /// Claim the oldest queued job matching labels
    Next {
        /// Labels to claim (default: this builder's registered labels)
        #[arg(long = "label", alias = "labels")]
        labels: Vec<String>,
    },
    /// Append to a run log
    Log {
        /// Run id
        run_id: String,
        /// Log chunk
        #[arg(long)]
        literal: Option<String>,
    },
    /// Mark a claimed run finished
    Finish {
        /// Run id
        run_id: String,
        /// `passed` or `failed`
        #[arg(long)]
        status: String,
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

/// `remote` subcommands (this machine only).
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
        /// Host SSH for `key copy` (`user@HOST`, default `$USER@<forge-host>:22`)
        #[arg(long)]
        host: Option<String>,
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
    /// `hooks/update` — ACL plus optional version policy
    Update {
        /// Ref being updated
        refname: String,
        /// Previous object name
        old: String,
        /// New object name
        new: String,
    },
    /// `hooks/commit-msg` — require Conventional Commits 1.0.0
    CommitMsg {
        /// Path to the commit message file
        path: PathBuf,
    },
}

/// `rgit version` subcommands (working tree; not over SSH).
#[derive(Subcommand)]
pub enum VersionCommands {
    /// Print the agreed version and files
    Show,
    /// Validate Conventional Commits in a revision range
    Check {
        /// Git revision range (default: last version tag..HEAD)
        range: Option<String>,
    },
    /// Rewrite version files only
    Bump {
        /// `auto`, `patch`, `minor`, or `major` (default: auto)
        #[arg(value_enum)]
        level: Option<VersionBump>,
        /// Set this SemVer 2.0 version instead of incrementing
        #[arg(long)]
        to: Option<String>,
        /// Print the plan without writing files
        #[arg(long)]
        dry_run: bool,
    },
    /// Preview Keep a Changelog notes from commits
    Changelog {
        /// Start tag (default: nearest version tag)
        #[arg(long)]
        from: Option<String>,
    },
    /// Bump files, update CHANGELOG.md, commit, and tag
    Release {
        /// `auto`, `patch`, `minor`, or `major` (default: auto)
        #[arg(value_enum)]
        level: Option<VersionBump>,
        /// Set this SemVer 2.0 version instead of incrementing
        #[arg(long)]
        to: Option<String>,
        /// Print the plan without writing files
        #[arg(long)]
        dry_run: bool,
        /// Update files and commit without creating a git tag
        #[arg(long)]
        no_tag: bool,
    },
    /// Install a local `commit-msg` hook
    Hook {
        #[command(subcommand)]
        command: VersionHookCommands,
    },
}

/// `rgit version hook` subcommands.
#[derive(Subcommand)]
pub enum VersionHookCommands {
    /// Write `.git/hooks/commit-msg`
    Install,
}

/// SemVer increment, or infer from Conventional Commits.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, clap::ValueEnum)]
pub enum VersionBump {
    /// Infer from `feat` / `fix` / `perf` / breaking commits since the last tag
    #[default]
    Auto,
    /// Increment PATCH
    Patch,
    /// Increment MINOR
    Minor,
    /// Increment MAJOR
    Major,
}

impl Commands {
    /// Commands that must not be exposed over SSH.
    pub fn ssh_forbidden(&self) -> bool {
        matches!(
            self,
            Commands::Init
                | Commands::Check
                | Commands::Status
                | Commands::View { .. }
                | Commands::Version { .. }
                | Commands::Serve { .. }
                | Commands::Shell
                | Commands::Remote { .. }
                | Commands::Hook { .. }
                | Commands::Key {
                    command: KeyCommands::Copy { .. },
                }
                | Commands::Agent { command: None, .. }
        )
    }

    /// Host commands that write forge data and must run as the systemd user.
    ///
    /// `auth login|logout|whoami|register` are excluded so `rgit-web` (user
    /// `rgit-web`, group `rabun-git`) can issue tokens and accept sign-up.
    pub fn requires_service_uid(&self) -> bool {
        matches!(
            self,
            Commands::Init
                | Commands::User { .. }
                | Commands::Repo { .. }
                | Commands::Access { .. }
                | Commands::Request { .. }
                | Commands::Run { .. }
                | Commands::Key {
                    command: KeyCommands::Add { .. } | KeyCommands::List { .. },
                }
                | Commands::Agent {
                    command: Some(_),
                    ..
                }
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
        assert!(!Commands::Auth {
            command: AuthCommands::Login {
                user: "ada".into(),
                password: "correct-horse".into(),
            }
        }
        .requires_service_uid());
        assert!(!Commands::Auth {
            command: AuthCommands::Register {
                user: "linus".into(),
                password: "correct-horse".into(),
            }
        }
        .requires_service_uid());
        assert!(!Commands::Check.requires_service_uid());
        assert!(Commands::View {
            target: None,
            git_ref: "HEAD".into(),
            bind: "127.0.0.1:1111".into(),
            open: false,
        }
        .ssh_forbidden());
        assert!(!Commands::View {
            target: None,
            git_ref: "HEAD".into(),
            bind: "127.0.0.1:1111".into(),
            open: false,
        }
        .requires_service_uid());
        assert!(Commands::Version {
            command: VersionCommands::Show
        }
        .ssh_forbidden());
        assert!(!Commands::Version {
            command: VersionCommands::Show
        }
        .requires_service_uid());
        assert!(Commands::Remote {
            command: RemoteCommands::List
        }
        .ssh_forbidden());
        assert!(!Commands::Remote {
            command: RemoteCommands::List
        }
        .requires_service_uid());
        assert!(Commands::Key {
            command: KeyCommands::Copy {
                user: None,
                file: None,
                admin: false,
                host: None,
            }
        }
        .ssh_forbidden());
        assert!(!Commands::Key {
            command: KeyCommands::Copy {
                user: None,
                file: None,
                admin: false,
                host: None,
            }
        }
        .requires_service_uid());
        assert!(Commands::Agent {
            labels: vec!["macos".into()],
            remote: None,
            command: None,
        }
        .ssh_forbidden());
        assert!(Commands::Agent {
            labels: Vec::new(),
            remote: None,
            command: Some(AgentCommands::List),
        }
        .requires_service_uid());
        assert!(!Commands::Agent {
            labels: Vec::new(),
            remote: None,
            command: Some(AgentCommands::List),
        }
        .ssh_forbidden());
    }

    #[test]
    fn help_follows_argv0() {
        let mut rgit = Cli::command().name("rgit");
        assert_eq!(rgit.get_name(), "rgit");
        assert!(rgit.render_long_help().to_string().contains("Usage: rgit"));
        let mut long = Cli::command().name("rabun-git");
        assert!(long
            .render_long_help()
            .to_string()
            .contains("Usage: rabun-git"));
    }
}
