//! Self-hosted git forge CLI (`init`, `check`, `status`, `serve`).
//!
//! Hosts bare repositories over SSH, stores merge requests as git refs, and
//! runs a small in-repo YAML workflow subset. This is not a Burton worker: there
//! is no Postgres and this process does not write warehouse trees.
//!
//! The user guide lives in `doc/README.md` in the repository.

pub mod acl;
pub mod agent;
pub mod auth;
pub mod browse;
pub mod cli;
pub mod config;
pub mod dispatch;
pub mod git;
pub mod hook;
pub mod names;
pub mod output;
pub mod release;
pub mod remote;
pub mod repo;
pub mod request;
pub mod runner;
pub mod setup;
pub mod ssh;
pub mod status;
pub mod store;
pub mod version;
pub mod view;
pub mod workflow;

use anyhow::Result;

use crate::config::Config;

/// Verify the data root, `git` on PATH, SSH bind, and an admin SSH key.
pub fn check(config: &Config) -> Result<()> {
    setup::check(config)
}

/// Print companion status (`rabun.companion/v1`, no keys).
pub fn print_status(config: &Config) -> Result<()> {
    status::print_status(config)
}

/// Listen for SSH git and management commands until SIGINT/SIGTERM.
pub async fn serve(config: &Config, bind: Option<String>) -> Result<()> {
    ssh::serve(config, bind).await
}

/// RFC 3339 UTC timestamp with millisecond precision.
pub fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}
