//! CLI for the Rabun git forge (`init`, `check`, `serve`, repo/request/run).

use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, ExitCode};

use anyhow::{bail, Context, Result};
use clap::Parser;

use rabun_git::acl::Actor;
use rabun_git::cli::{Cli, Commands};
use rabun_git::config::Config;
use rabun_git::store::Store;

/// systemd env file written by Ubuntu bootstrap.
const SYSTEM_ENV: &str = "/etc/rabun-git/rabun-git.env";
/// Default unix user for `serve` and operator `shell`.
const DEFAULT_RUN_AS: &str = "rabun-git";
/// systemd working directory / forge root on Ubuntu.
const SERVICE_HOME: &str = "/var/lib/rabun-git";
/// Prompt while `rabun-git shell` is active (`RABUN_GIT_SHELL=1`).
const SHELL_PS1: &str = r"\[\e[0;36m\](rabun-git)\[\e[0m\] \w \$ ";

#[tokio::main]
async fn main() -> ExitCode {
    load_env();
    init_tracing();
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{err:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<()> {
    let cli = Cli::parse();
    if matches!(cli.command, Commands::Shell) {
        return run_operator_shell();
    }
    if cli.command.requires_service_uid() && systemd_env_present() && !is_service_user() {
        bail!(
            "this command must run as {} so {SERVICE_HOME} stays writable by the service; start a session with `rabun-git shell`",
            run_as_user()
        );
    }
    match cli.command {
        Commands::Init => {
            let path = rabun_git::setup::init(cli.config.as_deref())?;
            print!("{}", rabun_git::setup::next_steps(&path));
            Ok(())
        }
        Commands::Check => {
            let config = Config::load(cli.config.as_deref())?;
            rabun_git::check(&config)
        }
        Commands::Status => {
            let config = Config::load(cli.config.as_deref())?;
            rabun_git::print_status(&config)
        }
        Commands::Serve { bind } => {
            let config = Config::load(cli.config.as_deref())?;
            rabun_git::serve(&config, bind).await
        }
        Commands::Shell => unreachable!("handled above"),
        other => {
            let config = Config::load(cli.config.as_deref())?;
            let store = Store::open(config.root());
            store.ensure_layout()?;
            let out = rabun_git::dispatch::execute(&store, &Actor::Operator, other).await?;
            print!("{out}");
            Ok(())
        }
    }
}

/// Interactive bash as the systemd user so operator commands own forge files.
fn run_operator_shell() -> Result<()> {
    let target = run_as_user();
    eprintln!("Forge operator as {target}. Prompt shows (rabun-git); type `exit` to leave.");
    if is_service_user() {
        let home = Path::new(SERVICE_HOME);
        if home.is_dir() {
            std::env::set_current_dir(home).with_context(|| format!("chdir {}", home.display()))?;
        }
        let err = operator_bash().exec();
        return Err(anyhow::anyhow!("exec /bin/bash: {err}"));
    }
    // Do not use sudo --chdir / -D: Ubuntu sudoers rejects it with /usr/bin/env.
    let start = if Path::new(SERVICE_HOME).is_dir() {
        format!("cd {SERVICE_HOME} && exec /bin/bash --norc --noprofile -i")
    } else {
        "exec /bin/bash --norc --noprofile -i".into()
    };
    let err = Command::new("sudo")
        .args([
            "-u",
            &target,
            "-H",
            "--",
            "env",
            "RABUN_GIT_SHELL=1",
            &format!("PS1={SHELL_PS1}"),
            "/bin/bash",
            "--norc",
            "--noprofile",
            "-c",
            &start,
        ])
        .exec();
    Err(anyhow::anyhow!("exec sudo -u {target} bash: {err}"))
}

fn operator_bash() -> Command {
    let mut bash = Command::new("/bin/bash");
    bash.env("RABUN_GIT_SHELL", "1")
        .env("PS1", SHELL_PS1)
        .args(["--norc", "--noprofile", "-i"]);
    bash
}

fn run_as_user() -> String {
    std::env::var("RABUN_GIT_RUN_AS")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_RUN_AS.into())
}

fn systemd_env_present() -> bool {
    Path::new(SYSTEM_ENV).is_file()
}

fn is_service_user() -> bool {
    let Some(current) = uid(&["-u"]) else {
        return false;
    };
    let Some(want) = uid(&["-u", &run_as_user()]) else {
        return false;
    };
    current == want
}

fn uid(args: &[&str]) -> Option<u32> {
    let output = Command::new("id").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout).trim().parse().ok()
}

/// Load `/etc/rabun-git/rabun-git.env` when present, then cwd `.env`.
/// Existing process variables win (systemd `EnvironmentFile` already applied them).
fn load_env() {
    if Path::new(SYSTEM_ENV).is_file() {
        let _ = dotenvy::from_filename(SYSTEM_ENV);
    }
    let _ = dotenvy::dotenv();
}

fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "rabun_git=info".into());
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(false)
        .init();
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use rabun_git::cli::Cli;

    #[test]
    fn clap_debug_assert() {
        Cli::command().debug_assert();
    }

    #[test]
    fn crate_version_is_semver() {
        let version = env!("CARGO_PKG_VERSION");
        let core = version.split(['-', '+']).next().expect("semver core");
        let mut parts = core.split('.');
        for label in ["major", "minor", "patch"] {
            parts
                .next()
                .unwrap_or_else(|| panic!("missing {label}"))
                .parse::<u64>()
                .unwrap_or_else(|_| panic!("{label} must be a number"));
        }
        assert!(
            parts.next().is_none(),
            "semver core must be major.minor.patch"
        );
    }
}
