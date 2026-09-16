use std::path::PathBuf;
use std::process::ExitCode;

use boswell::config::{Config, default_config_path};
use boswell::daemon;
use clap::{Parser, Subcommand};
use log::error;

#[derive(Parser)]
#[command(
    version,
    about = "Commit and push watched git repositories as they change"
)]
struct Cli {
    /// Config file to read (default: $XDG_CONFIG_HOME/boswell/config.toml)
    #[arg(long, global = true, value_name = "PATH")]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Sync every configured repository once and exit
    Once,
}

fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let cli = Cli::parse();
    let path = cli.config.unwrap_or_else(default_config_path);
    let config = match Config::load(&path) {
        Ok(config) => config,
        Err(e) => {
            error!("{e}");
            return ExitCode::FAILURE;
        }
    };

    match cli.command {
        Some(Command::Once) => {
            if daemon::once(&config) {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        None => match daemon::run(config) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                error!("{e}");
                ExitCode::FAILURE
            }
        },
    }
}
