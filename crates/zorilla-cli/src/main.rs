//! `zorilla` — thin clap wrapper over `zorilla-core`.
//!
//! Phase 2 ships the `check` subcommand with the grouped text emitter.
//! Exit codes: `0` (clean), `1` (findings), `2` (error).

use std::fmt::Write as _;
use std::io::{BufRead, IsTerminal as _};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Context;
use clap::{Parser, Subcommand, ValueEnum};
use zorilla_core::guide::{self, Topic};
use zorilla_core::{
    compute_overview, compute_stats, format_overview_json, format_overview_text, format_stats_json,
    format_stats_text, lint, lint_with_filter, rule_name_for, rules, ChangedLines, Config,
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
        /// Manifest of changed lines (FILE[:start-end,...] per line). Use "-" for stdin.
        /// Filters findings to those whose line falls inside one of the ranges
        /// for that file. Files not in the manifest are kept entirely.
        #[arg(long, value_name = "FILE", conflicts_with = "files_from")]
        changed_lines: Option<PathBuf>,
    },
    /// Print short instructions for using zorilla here.
    ///
    /// With no topic, picks `setup` or `triage` by looking for a
    /// `zorilla.toml`, a `[tool.zorilla]` table in `pyproject.toml`, or a
    /// `.pre-commit-config.yaml` naming the hook — in that order — in the
    /// current directory or any directory above it, stopping at the
    /// repository root. `tune` is a reference and is never auto-selected.
    Guide {
        /// Which instructions to print. Omit to have zorilla choose.
        #[arg(value_enum)]
        topic: Option<Topic>,
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
    /// Print a per-file overview of a scan (findings grouped by file,
    /// with a trailing count of clean files). Always exits 0 — this is
    /// a report, not a gate.
    Overview {
        /// Paths to scan. Defaults to the current directory.
        #[arg(value_name = "PATH")]
        paths: Vec<PathBuf>,
        /// Output format. Stats/overview support `text` and `json`;
        /// SARIF does not fit per-file overviews.
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
/// (`stats`, `overview`). Deliberately separate from [`Format`] so
/// SARIF cannot be routed into a summary by accident.
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
        Command::Check { paths, format, files_from, files, changed_lines } => {
            check(paths, format, files_from.as_deref(), files, changed_lines.as_deref())
        }
        Command::Guide { topic } => {
            print!("{}", guide_output(topic));
            Ok(ExitCode::SUCCESS)
        }
        Command::ListRules => Ok(list_rules()),
        Command::Explain { code } => Ok(explain(&code)),
        Command::Stats { paths, format, files_from, files } => {
            stats(paths, format, files_from.as_deref(), files)
        }
        Command::Overview { paths, format, files_from, files } => {
            overview(paths, format, files_from.as_deref(), files)
        }
    }
}

/// Resolve the guide topic and render it.
///
/// Detection runs against the current directory rather than the paths of some
/// other subcommand: the question `zorilla guide` answers is "is zorilla set up
/// where I am standing", and the guide tells its reader to stand at the
/// repository root.
fn guide_output(topic: Option<Topic>) -> String {
    if let Some(topic) = topic {
        return guide::render(topic, guide::Selection::Explicit);
    }
    // An absolute directory, because `detect` walks upward and `Path::new(".")`
    // has no parent to walk to. A cwd we cannot read is "not configured": the
    // point of `guide` is to print instructions, never to fail.
    let start = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let source = guide::detect(&start);
    guide::render(guide::auto_topic(source), guide::Selection::Auto(source))
}

fn check(
    paths: Vec<PathBuf>,
    format: Format,
    files_from: Option<&Path>,
    files: Vec<PathBuf>,
    changed_lines: Option<&Path>,
) -> anyhow::Result<ExitCode> {
    let changed = match changed_lines {
        Some(src) => Some(ChangedLines::from_path(src).context("parsing --changed-lines")?),
        None => None,
    };

    // Empty-manifest short-circuit: if the manifest is present and
    // empty AND the user provided no other inputs (positional paths,
    // `--files`, or `--files-from` — the last being mutually exclusive
    // with `--changed-lines` so it's always None here, but we check it
    // anyway to keep the rule explicit), exit 0 with no work. This is
    // the "pre-commit ran but no Python lines changed" path.
    if let Some(cl) = changed.as_ref() {
        if cl.is_empty() && paths.is_empty() && files.is_empty() && files_from.is_none() {
            return Ok(ExitCode::SUCCESS);
        }
    }

    let manifest_paths: Vec<PathBuf> =
        changed.as_ref().map(|cl| cl.paths().map(Path::to_path_buf).collect()).unwrap_or_default();

    let paths = resolve_inputs(paths, files, files_from, manifest_paths)?;
    let config = load_config_for(&paths)?;

    let report = lint_with_filter(&paths, &config, changed.as_ref()).context("running lint")?;
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
    let paths = resolve_inputs(paths, files, files_from, Vec::new())?;
    let config = load_config_for(&paths)?;

    let report = lint(&paths, &config).context("running lint")?;
    let stats = compute_stats(&report);
    let rendered = match format {
        SummaryFormat::Text => format_stats_text(&stats),
        SummaryFormat::Json => format_stats_json(&stats),
    };
    print!("{rendered}");
    Ok(ExitCode::SUCCESS)
}

/// Handle the `overview` subcommand — group findings by file and render
/// a per-file view. Shares `resolve_inputs` and `load_config_for` with
/// `check`. Text output uses ANSI color when stdout is a terminal and
/// the `NO_COLOR` env var is unset (honoring the conventional opt-out
/// at <https://no-color.org>). Always exits 0 — `overview` is a report,
/// not a gate.
fn overview(
    paths: Vec<PathBuf>,
    format: SummaryFormat,
    files_from: Option<&Path>,
    files: Vec<PathBuf>,
) -> anyhow::Result<ExitCode> {
    let paths = resolve_inputs(paths, files, files_from, Vec::new())?;
    let config = load_config_for(&paths)?;

    let report = lint(&paths, &config).context("running lint")?;
    let overview = compute_overview(&report);
    let rendered = match format {
        SummaryFormat::Text => {
            let use_color = should_use_color();
            format_overview_text(&overview, use_color)
        }
        SummaryFormat::Json => format_overview_json(&overview),
    };
    print!("{rendered}");
    Ok(ExitCode::SUCCESS)
}

/// Color detection for the `overview` text emitter. Returns `true`
/// only when stdout is a terminal AND the `NO_COLOR` env var is not
/// set to a non-empty string — matches <https://no-color.org/>.
fn should_use_color() -> bool {
    if std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty()) {
        return false;
    }
    std::io::stdout().is_terminal()
}

/// Merge positional paths, `--files` repeats, `--files-from` lines, and
/// paths extracted from a `--changed-lines` manifest into a single input
/// list. If every source is empty, default to the current working
/// directory — matches the existing bare `zorilla check` behaviour.
///
/// `manifest_paths` is the 4th input source: paths a manifest mentioned
/// explicitly. Manifest paths are added as `--files`-equivalents so the
/// discovery walker treats them as explicit file arguments (bypassing
/// `include` globs the same way `--files` does).
fn resolve_inputs(
    paths: Vec<PathBuf>,
    files: Vec<PathBuf>,
    files_from: Option<&Path>,
    manifest_paths: Vec<PathBuf>,
) -> anyhow::Result<Vec<PathBuf>> {
    let mut merged: Vec<PathBuf> = Vec::new();
    merged.extend(paths);
    merged.extend(files);
    if let Some(src) = files_from {
        merged.extend(read_files_from(src)?);
    }
    merged.extend(manifest_paths);

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

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory as _;

    /// Every zorilla command the guides show must be one the CLI accepts.
    ///
    /// This lives here rather than in `zorilla-core`'s `guide` module because
    /// `Cli` is defined in the binary. The extraction it depends on is covered
    /// by `guide::tests::extraction_finds_every_command_the_guides_show`, so an
    /// empty extraction cannot pass as "all valid" in either place.
    #[test]
    fn every_command_shown_in_a_guide_parses() {
        let mut checked = 0_usize;
        for topic in Topic::ALL {
            for argv in guide::embedded_invocations(topic) {
                checked += 1;
                let parsed = Cli::command().try_get_matches_from(&argv);
                assert!(
                    parsed.is_ok(),
                    "guide `{}` shows `{}`, which the CLI rejects: {}",
                    topic.name(),
                    argv.join(" "),
                    parsed.unwrap_err(),
                );
            }
        }
        assert!(checked >= 6, "expected several commands across the guides, found {checked}");
    }

    /// An agent that ran `zorilla guide` with no topic needs to know why it got
    /// what it got, and where to stand when it runs it.
    #[test]
    fn guide_help_states_the_auto_selection_rule() {
        let mut cmd = Cli::command();
        let guide_cmd = cmd
            .get_subcommands_mut()
            .find(|c| c.get_name() == "guide")
            .expect("the `guide` subcommand should exist");
        let help = guide_cmd.render_long_help().to_string();
        assert!(help.contains("setup"), "guide --help should name the setup topic: {help}");
        assert!(help.contains("triage"), "guide --help should name the triage topic: {help}");
        assert!(
            help.contains("never auto-selected"),
            "guide --help should say tune is never auto-selected: {help}",
        );
        // Anchored on wording only the upward walk can honestly claim, so
        // help that still describes a current-directory-only lookup fails.
        assert!(
            help.contains("any directory above"),
            "guide --help should say detection walks upward: {help}",
        );
    }

    #[test]
    fn guide_topics_parse_as_values() {
        for topic in Topic::ALL {
            assert!(
                Cli::command().try_get_matches_from(["zorilla", "guide", topic.name()]).is_ok(),
                "`zorilla guide {}` should parse",
                topic.name(),
            );
        }
        assert!(
            Cli::command().try_get_matches_from(["zorilla", "guide", "how"]).is_err(),
            "an unknown topic should be rejected, not silently defaulted",
        );
        assert!(
            Cli::command().try_get_matches_from(["zorilla", "guide"]).is_ok(),
            "the topic is optional",
        );
    }

    #[test]
    fn explicit_topic_renders_that_topic_and_no_arrow() {
        let out = guide_output(Some(Topic::Tune));
        assert!(out.starts_with("# zorilla guide: tune\n"), "got: {out}");
        assert!(out.ends_with(Topic::Tune.text()), "the docs page is emitted verbatim");
    }
}
