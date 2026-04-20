//! `zorilla` — thin clap wrapper over `zorilla-core`.
//!
//! Phase 2 ships the `check` subcommand with the grouped text emitter.
//! Exit codes: `0` (clean), `1` (findings), `2` (error).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Context;
use clap::{Parser, Subcommand, ValueEnum};
use zorilla_core::{lint, rule_name_for, rules, Config};

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
        /// Output format.
        #[arg(long, value_enum, default_value_t = Format::Text)]
        format: Format,
    },
    /// List all available rules.
    ListRules,
    /// Print the long-form documentation for a rule.
    Explain {
        /// Rule code, e.g. "ZR001" (case-insensitive).
        code: String,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Format {
    Text,
    Json,
    Sarif,
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
        Command::Check { paths, format } => check(paths, format),
        Command::ListRules => Ok(list_rules()),
        Command::Explain { code } => Ok(explain(&code)),
    }
}

fn check(paths: Vec<PathBuf>, format: Format) -> anyhow::Result<ExitCode> {
    let cwd = std::env::current_dir().context("getting current directory")?;
    let paths = if paths.is_empty() { vec![cwd] } else { paths };

    // Discover config starting from the first target path so running
    // `zorilla check /some/project/tests/` from a different cwd still
    // picks up that project's `pyproject.toml` / `zorilla.toml`. Matches
    // the behavior of ruff / black / mypy. Fall back to the path itself
    // when it's a directory.
    let first = &paths[0];
    let search_start: &Path =
        if first.is_file() { first.parent().unwrap_or(first.as_path()) } else { first.as_path() };
    let config = Config::discover(search_start).context("loading configuration")?;

    let report = lint(&paths, &config).context("running lint")?;
    // Use the first user-supplied path as the base for display paths —
    // matches `path_arg.join(relative_from_walker)` shape from context.md
    // gotchas.
    let base: &Path = paths.first().map_or_else(|| Path::new(""), PathBuf::as_path);
    // All three renderers return a string ending in `\n`, so `print!`
    // (not `println!`) produces exactly one trailing newline on stdout
    // regardless of format. JSON and SARIF also skip the text summary
    // line so consumers can feed stdout straight into parsers.
    let rendered = match format {
        Format::Text => report.render_text(base, rule_name_for),
        Format::Json => report.render_json(base),
        Format::Sarif => report.render_sarif(base),
    };
    print!("{rendered}");
    Ok(ExitCode::from(report.exit_code()))
}

/// Handle the `list-rules` subcommand — dump every registered rule in a
/// padded text table. Column widths mirror the brief: CODE=6, NAME=26.
/// The trailing DEFAULT column is unpadded — padding the last column
/// would emit 4–5 trailing spaces on every row for no alignment benefit
/// and trip whitespace-sensitive tooling (gitleaks, editors, diff).
/// The table is built into a single `String` so we emit one `print!`
/// call, matching the `check` handler's output shape.
fn list_rules() -> ExitCode {
    let mut out = String::new();
    // Header; ignore write-to-String failures because `String` never errors.
    let _ = writeln!(out, "{:<6}{:<26}DEFAULT", "CODE", "NAME");
    for rule in rules::registry::all() {
        let default = if rule.default_enabled() { "on" } else { "off" };
        let _ = writeln!(out, "{:<6}{:<26}{}", rule.code(), rule.name(), default);
    }
    print!("{out}");
    ExitCode::SUCCESS
}

/// Handle the `explain <code>` subcommand — print the embedded
/// documentation for the named rule. Rule lookup is case-insensitive:
/// the user can type either `ZR001` or `zr001`. Unknown codes exit with
/// status 2 and an error message on stderr.
fn explain(code: &str) -> ExitCode {
    let upper = code.to_uppercase();
    if let Some(rule) = rules::registry::find(&upper) {
        print!("{}", rule.doc());
        ExitCode::SUCCESS
    } else {
        eprintln!("zorilla: unknown rule: {code}");
        ExitCode::from(2)
    }
}
