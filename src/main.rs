//! CLI for the Rabun git forge (`init`, `check`, `serve`, repo/request/run).

use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use anyhow::{bail, Context, Result};

use rabun_git::acl::Actor;
use rabun_git::cli::{Commands, KeyCommands};
use rabun_git::config::Config;
use rabun_git::remote;
use rabun_git::store::Store;

/// systemd env file written by Ubuntu bootstrap.
const SYSTEM_ENV: &str = "/etc/rabun-git/rabun-git.env";
/// Default unix user for `serve` and operator `shell`.
const DEFAULT_RUN_AS: &str = "rabun-git";
/// systemd working directory / forge root on Ubuntu.
const SERVICE_HOME: &str = "/var/lib/rabun-git";
/// Interactive bash overwrites env `PS1`; this rcfile sets the session prompt.
const SHELL_RC: &str = r#"export RABUN_GIT_SHELL=1
cd /var/lib/rabun-git 2>/dev/null || true
PS1='\[\e[0;36m\](rabun-git)\[\e[0m\] \w \$ '
"#;

#[tokio::main]
async fn main() -> ExitCode {
    load_env();
    init_tracing();
    let json = std::env::args_os().any(|arg| arg == "--json")
        || std::env::var_os("RABUN_GIT_JSON").is_some();
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            if json {
                print!("{}", rabun_git::output::error_json(&err));
            } else {
                eprintln!("{err:#}");
            }
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<()> {
    if let Some(invoke) = remote::detect_invoke()? {
        return run_named_remote(invoke);
    }
    let cli = rabun_git::cli::parse();
    if matches!(cli.command, Commands::Shell) {
        return run_operator_shell();
    }
    if matches!(cli.command, Commands::Remote { .. }) {
        let Commands::Remote { command } = cli.command else {
            unreachable!("just matched Remote");
        };
        print!("{}", remote::manage(command)?);
        return Ok(());
    }
    if let Commands::Agent {
        labels,
        remote,
        command: None,
    } = &cli.command
    {
        return rabun_git::agent::poll(labels, remote.as_deref().unwrap_or("origin"));
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
        Commands::View {
            target,
            git_ref,
            bind,
            open,
        } => {
            let config = Config::load(cli.config.as_deref())?;
            rabun_git::view::run(
                &config,
                rabun_git::view::Options {
                    target,
                    git_ref,
                    bind,
                    open,
                },
            )
            .await
        }
        Commands::Version { command } => {
            let out = rabun_git::release::run(command).await?;
            print!("{out}");
            Ok(())
        }
        Commands::Serve { bind } => {
            let config = Config::load(cli.config.as_deref())?;
            rabun_git::serve(&config, bind).await
        }
        Commands::Shell | Commands::Remote { .. } => unreachable!("handled above"),
        other => {
            let json = cli.json;
            let token = cli.token.clone();
            let config = Config::load(cli.config.as_deref())?;
            let store = Store::open(config.root());
            store.ensure_layout()?;
            let actor = resolve_actor(&store, cli.anonymous, token.as_deref())?;
            let out =
                rabun_git::dispatch::execute_fmt(&store, &actor, other, json, token.as_deref())
                    .await?;
            print!("{out}");
            Ok(())
        }
    }
}

fn resolve_actor(store: &Store, anonymous: bool, token: Option<&str>) -> Result<Actor> {
    if anonymous && token.is_some() {
        bail!("use --token or --anonymous, not both");
    }
    if anonymous {
        return Ok(Actor::Anonymous);
    }
    if let Some(token) = token.map(str::trim).filter(|value| !value.is_empty()) {
        return rabun_git::auth::actor_from_token(store, token);
    }
    Ok(Actor::Operator)
}

/// `rabun-git origin …` — SSH to a saved forge host.
fn run_named_remote(invoke: remote::ClientInvoke) -> Result<()> {
    let mut parse_from = vec![rabun_git::cli::invoked_name()];
    parse_from.extend(invoke.args.iter().cloned());
    let cli = rabun_git::cli::parse_from(&parse_from);
    if let Commands::Agent {
        labels,
        command: None,
        ..
    } = &cli.command
    {
        return rabun_git::agent::poll(labels, &invoke.name);
    }
    if let Commands::Key {
        command:
            KeyCommands::Copy {
                user,
                file,
                admin,
                host,
            },
    } = cli.command
    {
        return remote::copy_key(
            &invoke,
            user.as_deref(),
            file.as_deref(),
            admin,
            host.as_deref(),
            cli.identity.as_deref(),
        );
    }
    if cli.command.ssh_forbidden() {
        bail!("run this on the forge host, not through `{}`", invoke.name);
    }
    let identity = cli.identity.or(invoke.identity);
    let payload = remote::payload_args(&invoke.args)?;
    remote::ssh_exec(&invoke.target, identity.as_deref(), &payload)
}

/// Interactive bash as the systemd user so operator commands own forge files.
fn run_operator_shell() -> Result<()> {
    let target = run_as_user();
    let rc = write_shell_rc()?;
    eprintln!("Forge operator as {target}. Prompt shows (rabun-git); type `exit` to leave.");
    if is_service_user() {
        let err = operator_bash(&rc).exec();
        return Err(anyhow::anyhow!("exec /bin/bash: {err}"));
    }
    // No sudo --chdir / -D: Ubuntu sudoers rejects it with /usr/bin/env.
    let err = Command::new("sudo")
        .args([
            "-u",
            &target,
            "-H",
            "--",
            "/bin/bash",
            "--rcfile",
            rc.to_str().context("shell rc path")?,
            "-i",
        ])
        .exec();
    Err(anyhow::anyhow!("exec sudo -u {target} bash: {err}"))
}

fn write_shell_rc() -> Result<PathBuf> {
    let path = std::env::temp_dir().join(format!("rabun-git-shell.{}.rc", std::process::id()));
    std::fs::write(&path, SHELL_RC).with_context(|| format!("write {}", path.display()))?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
        .with_context(|| format!("chmod {}", path.display()))?;
    Ok(path)
}

fn operator_bash(rc: &Path) -> Command {
    let mut bash = Command::new("/bin/bash");
    bash.args(["--rcfile", rc.to_str().unwrap_or("/dev/null"), "-i"]);
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
    fn shell_rc_sets_session_prompt() {
        assert!(super::SHELL_RC.contains("(rabun-git)"));
        assert!(super::SHELL_RC.contains("RABUN_GIT_SHELL=1"));
    }

    #[test]
    fn crate_version_is_semver() {
        rabun_git::version::parse(env!("CARGO_PKG_VERSION")).unwrap();
        assert_eq!(
            rabun_git::version::crate_git_tag(),
            format!("v{}", env!("CARGO_PKG_VERSION"))
        );
    }
}
