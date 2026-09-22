//! CLI for the Rabun git forge (`init`, `check`, `serve`, repo/request/run).

use std::process::ExitCode;

use anyhow::Result;
use clap::Parser;

use rabun_git::acl::Actor;
use rabun_git::cli::{Cli, Commands};
use rabun_git::config::Config;
use rabun_git::store::Store;

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

/// Load `/etc/rabun-git/rabun-git.env` when present, then cwd `.env`.
/// Existing process variables win (systemd `EnvironmentFile` already applied them).
fn load_env() {
    const SYSTEM_ENV: &str = "/etc/rabun-git/rabun-git.env";
    if std::path::Path::new(SYSTEM_ENV).is_file() {
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
