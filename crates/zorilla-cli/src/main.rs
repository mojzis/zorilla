//! `zorilla` — thin clap wrapper over `zorilla-core`.
//!
//! Phase 2 ships the `check` subcommand with the grouped text emitter.
//! Exit codes: `0` (clean), `1` (findings), `2` (error).

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Context;
use clap::{Parser, Subcommand};
use zorilla_core::{lint, rule_name_for, Config};

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
    // Use the first user-supplied path as the base for display paths —
    // matches `path_arg.join(relative_from_walker)` shape from context.md
    // gotchas.
    let base: &Path = paths.first().map_or_else(|| Path::new(""), PathBuf::as_path);
    let text = report.render_text(base, rule_name_for);
    print!("{text}");
    Ok(ExitCode::from(report.exit_code()))
}
