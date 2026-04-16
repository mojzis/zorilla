//! `zorilla` — thin clap wrapper over `zorilla-core`.
//!
//! Phase 1 ships only the `check` subcommand. It prints the Phase 1
//! summary line (`"N findings in M files discovered."`) and uses exit
//! codes `0` (clean), `1` (findings), `2` (error).

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Context;
use clap::{Parser, Subcommand};
use zorilla_core::{lint, Config};

#[derive(Debug, Parser)]
#[command(name = "zorilla", about = "pytest test-smell linter", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Lint the given paths (files or directories).
    Check {
        /// Paths to check. Defaults to the current directory.
        #[arg(value_name = "PATH")]
        paths: Vec<PathBuf>,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("zorilla: error: {err:#}");
            ExitCode::from(2)
        }
    }
}

fn run(cli: Cli) -> anyhow::Result<ExitCode> {
    match cli.command {
        Command::Check { paths } => check(paths),
    }
}

fn check(paths: Vec<PathBuf>) -> anyhow::Result<ExitCode> {
    let cwd = std::env::current_dir().context("getting current directory")?;
    let config = Config::discover(&cwd).context("loading configuration")?;

    let paths = if paths.is_empty() { vec![cwd] } else { paths };

    let report = lint(&paths, &config).context("running lint")?;
    println!("{}", report.summary_line());
    Ok(ExitCode::from(u8::try_from(report.exit_code()).unwrap_or(2)))
}
