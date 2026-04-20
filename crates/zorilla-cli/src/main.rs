//! `zorilla` — thin clap wrapper over `zorilla-core`.
//!
//! Phase 2 ships the `check` subcommand with the grouped text emitter.
//! Exit codes: `0` (clean), `1` (findings), `2` (error).

use std::fmt::Write as _;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Context;
use clap::{Parser, Subcommand, ValueEnum};
use zorilla_core::{
    compute_stats, format_stats_json, format_stats_text, lint, rule_name_for, rules, Config,
};

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
        /// Read paths to lint from this file (one per line). Use "-" to read stdin.
        #[arg(long, value_name = "FILE")]
        files_from: Option<PathBuf>,
        /// Additional path to lint. May be repeated.
        #[arg(long = "files", value_name = "PATH")]
        files: Vec<PathBuf>,
    },
    /// List all available rules.
    ListRules,
    /// Print the long-form documentation for a rule.
    Explain {
        /// Rule code, e.g. "ZR001" (case-insensitive).
        code: String,
    },
    /// Print aggregate statistics for a scan (files scanned,
    /// findings per rule). Always exits 0 — this is a report, not a gate.
    Stats {
        /// Paths to scan. Defaults to the current directory.
        #[arg(value_name = "PATH")]
        paths: Vec<PathBuf>,
        /// Output format. Stats/overview support `text` and `json`;
        /// SARIF does not fit aggregate summaries.
        #[arg(long, value_enum, default_value_t = SummaryFormat::Text)]
        format: SummaryFormat,
        /// Read paths to scan from this file (one per line). Use "-" to read stdin.
        #[arg(long, value_name = "FILE")]
        files_from: Option<PathBuf>,
        /// Additional path to scan. May be repeated.
        #[arg(long = "files", value_name = "PATH")]
        files: Vec<PathBuf>,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Format {
    Text,
    Json,
    Sarif,
}

/// Output formats supported by aggregate-summary subcommands
/// (`stats`, future `overview`). Deliberately separate from
/// [`Format`] so SARIF cannot be routed into a summary by accident.
#[derive(Debug, Clone, Copy, ValueEnum)]
enum SummaryFormat {
    Text,
    Json,
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
        Command::Check { paths, format, files_from, files } => {
            check(paths, format, files_from.as_deref(), files)
        }
        Command::ListRules => Ok(list_rules()),
        Command::Explain { code } => Ok(explain(&code)),
        Command::Stats { paths, format, files_from, files } => {
            stats(paths, format, files_from.as_deref(), files)
        }
    }
}

fn check(
    paths: Vec<PathBuf>,
    format: Format,
    files_from: Option<&Path>,
    files: Vec<PathBuf>,
) -> anyhow::Result<ExitCode> {
    let paths = resolve_inputs(paths, files, files_from)?;
    let config = load_config_for(&paths)?;

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

/// Handle the `stats` subcommand — post-process a scan into aggregate
/// counters and a per-rule breakdown. Shares `resolve_inputs` and
/// `load_config_for` with `check` so both flows see the same paths and
/// config. Always exits 0 — `stats` is a report, not a gate.
fn stats(
    paths: Vec<PathBuf>,
    format: SummaryFormat,
    files_from: Option<&Path>,
    files: Vec<PathBuf>,
) -> anyhow::Result<ExitCode> {
    let paths = resolve_inputs(paths, files, files_from)?;
    let config = load_config_for(&paths)?;

    let report = lint(&paths, &config).context("running lint")?;
    let stats = compute_stats(&report);
    let rendered = match format {
        SummaryFormat::Text => format_stats_text(&stats),
        SummaryFormat::Json => format_stats_json(&stats).context("formatting stats as JSON")?,
    };
    print!("{rendered}");
    Ok(ExitCode::SUCCESS)
}

/// Merge positional paths, `--files` repeats, and `--files-from` lines
/// into a single input list. If every source is empty, default to the
/// current working directory — matches the existing bare
/// `zorilla check` behaviour.
fn resolve_inputs(
    paths: Vec<PathBuf>,
    files: Vec<PathBuf>,
    files_from: Option<&Path>,
) -> anyhow::Result<Vec<PathBuf>> {
    let mut merged: Vec<PathBuf> = Vec::new();
    merged.extend(paths);
    merged.extend(files);
    if let Some(src) = files_from {
        merged.extend(read_files_from(src)?);
    }

    if merged.is_empty() {
        Ok(vec![std::env::current_dir().context("getting current directory")?])
    } else {
        Ok(merged)
    }
}

/// Discover config starting from the first target path so running
/// `zorilla check /some/project/tests/` from a different cwd still
/// picks up that project's `pyproject.toml` / `zorilla.toml`. Matches
/// the behavior of ruff / black / mypy. Fall back to the path itself
/// when it's a directory.
fn load_config_for(paths: &[PathBuf]) -> anyhow::Result<Config> {
    // `resolve_inputs` guarantees `paths` is non-empty; be defensive
    // anyway so a future refactor that bypasses it can't panic.
    let first = paths.first().map_or_else(|| Path::new("."), PathBuf::as_path);
    let search_start: &Path = if first.is_file() { first.parent().unwrap_or(first) } else { first };
    Config::discover(search_start).context("loading configuration")
}

/// Read paths (one per line) from either stdin (when `src == "-"`) or
/// the file at `src`. Skips blank lines and `#` comments; strips
/// trailing `\r` for Windows line endings and trims surrounding
/// whitespace. Empty input yields an empty Vec — the caller is
/// responsible for defaulting to CWD.
fn read_files_from(src: &Path) -> anyhow::Result<Vec<PathBuf>> {
    if src.as_os_str() == "-" {
        let stdin = std::io::stdin();
        let mut out = Vec::new();
        for line in stdin.lock().lines() {
            let line = line.context("reading --files-from stdin")?;
            if let Some(path) = parse_files_from_line(&line) {
                out.push(path);
            }
        }
        Ok(out)
    } else {
        let contents = std::fs::read_to_string(src).context("reading --files-from")?;
        let mut out = Vec::new();
        for line in contents.lines() {
            if let Some(path) = parse_files_from_line(line) {
                out.push(path);
            }
        }
        Ok(out)
    }
}

/// Normalise one line from `--files-from` input. Returns `None` for
/// blank lines and `#`-comments; returns `Some(PathBuf)` otherwise.
fn parse_files_from_line(line: &str) -> Option<PathBuf> {
    let trimmed = line.strip_suffix('\r').unwrap_or(line).trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        None
    } else {
        Some(PathBuf::from(trimmed))
    }
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
