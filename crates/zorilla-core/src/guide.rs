//! Agent-facing instructions for the three moments someone meets zorilla.
//!
//! The prose lives in `docs/guide/*.md` and is pulled in with [`include_str!`],
//! the same way rule documentation is, so the repository and the CLI serve the
//! same bytes. There is no guide text in this file, and there must never be: a
//! second copy is a copy that drifts.
//!
//! What keeps the guides honest is the test module at the bottom. Every command
//! a guide shows is fed through the real clap `Command` (in the binary's own
//! tests, which is the only place the `Cli` type exists); every `ZR###` it names
//! must be a registered rule; every config key it names must be one the
//! deserializer accepts. A guide cannot drift from the tool it describes without
//! failing the build.

use std::path::Path;

/// Instructions for a repository that has no zorilla configuration yet.
const SETUP: &str = include_str!("../../../docs/guide/setup.md");
/// Instructions for turning a check run's findings into edits.
const TRIAGE: &str = include_str!("../../../docs/guide/triage.md");
/// Reference for suppression, scope and the per-rule knobs.
const TUNE: &str = include_str!("../../../docs/guide/tune.md");

/// Which set of instructions to print.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
pub enum Topic {
    /// zorilla is not configured in this repository yet.
    Setup,
    /// A check run reported findings; what to do with them.
    Triage,
    /// Suppression, scope and per-rule knobs.
    Tune,
}

impl Topic {
    /// The topic's name as written on the command line and in the header.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Setup => "setup",
            Self::Triage => "triage",
            Self::Tune => "tune",
        }
    }

    /// The guide text, byte-identical to the docs page it is included from.
    #[must_use]
    pub fn text(self) -> &'static str {
        match self {
            Self::Setup => SETUP,
            Self::Triage => TRIAGE,
            Self::Tune => TUNE,
        }
    }

    /// Every topic, for tests and for exhaustive rendering.
    pub const ALL: [Self; 3] = [Self::Setup, Self::Triage, Self::Tune];
}

/// What made a repository count as configured.
///
/// The variants are ordered by the precedence [`detect`] applies, which is the
/// same precedence [`crate::Config::discover`] uses for the two config files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigSource {
    /// A `zorilla.toml` at or above the working directory.
    ZorillaToml,
    /// A `pyproject.toml` carrying a `[tool.zorilla]` table.
    PyProject,
    /// A `.pre-commit-config.yaml` wiring up a zorilla hook.
    PreCommit,
}

impl ConfigSource {
    /// How the header line names this source.
    fn label(self) -> &'static str {
        match self {
            Self::ZorillaToml => "zorilla.toml",
            Self::PyProject => "pyproject.toml [tool.zorilla]",
            Self::PreCommit => ".pre-commit-config.yaml",
        }
    }
}

/// How the printed topic was chosen.
///
/// Carried into the header so a reader — usually an agent that did not pass a
/// topic — can see why it got the text it got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selection {
    /// The caller named the topic on the command line.
    Explicit,
    /// The topic was derived from what was found in the working directory.
    Auto(Option<ConfigSource>),
}

/// Report whether zorilla is configured at or above `dir`, and by what.
///
/// Walks upward the way [`crate::Config::discover`] does — checking each
/// directory in full before ascending — because the two have to agree. A reader
/// standing in `tests/unit` whose `zorilla check` honours the config at the
/// repository root must not be told the repository is unconfigured and handed
/// setup instructions for a second one.
///
/// One deliberate difference: the walk stops at the directory holding `.git`.
/// `discover` is unbounded because a lint run should honour whatever config it
/// is given; this answers "is zorilla set up in *this repository*", and a
/// `zorilla.toml` in a parent checkout is not an answer to that. Outside a
/// checkout there is no boundary to find and the walk runs to the filesystem
/// root, exactly as `discover` does.
///
/// Unreadable or unparseable files are treated as absent rather than as errors:
/// a broken `pyproject.toml` is a reason to say "not configured", not a reason
/// to refuse to print instructions.
#[must_use]
pub fn detect(dir: &Path) -> Option<ConfigSource> {
    let mut current = Some(dir);
    while let Some(dir) = current {
        if let Some(source) = detect_in(dir) {
            return Some(source);
        }
        if dir.join(".git").exists() {
            return None;
        }
        current = dir.parent();
    }
    None
}

/// The per-directory half of [`detect`] — the precedence `zorilla guide tune`
/// documents, applied to one directory.
///
/// The `.pre-commit-config.yaml` arm has no counterpart in
/// [`crate::Config::discover`]: it answers "is zorilla wired up here", not
/// "which file configures it". The two can therefore name different sources in
/// a nested checkout, which is cosmetic — the chosen topic is the same.
fn detect_in(dir: &Path) -> Option<ConfigSource> {
    if dir.join("zorilla.toml").is_file() {
        return Some(ConfigSource::ZorillaToml);
    }
    if pyproject_has_zorilla_table(&dir.join("pyproject.toml")) {
        return Some(ConfigSource::PyProject);
    }
    if pre_commit_references_zorilla(&dir.join(".pre-commit-config.yaml")) {
        return Some(ConfigSource::PreCommit);
    }
    None
}

/// Parse `pyproject.toml` and report whether it carries a `[tool.zorilla]`
/// table.
///
/// Parsed rather than grepped: `tool.zorilla` appearing in a comment, a string,
/// or another tool's table is not configuration.
fn pyproject_has_zorilla_table(path: &Path) -> bool {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return false;
    };
    // `toml::from_str` for a document, not `str::parse` — the latter parses a
    // bare TOML *value* and rejects every real pyproject.toml.
    let Ok(document) = toml::from_str::<toml::Table>(&contents) else {
        return false;
    };
    document.get("tool").and_then(|tool| tool.get("zorilla")).is_some_and(toml::Value::is_table)
}

/// Report whether `.pre-commit-config.yaml` wires up zorilla.
///
/// Matched textually rather than parsed: pulling in a YAML dependency to answer
/// one boolean is not worth it, and the two shapes that matter — the hook repo
/// URL and a hook id — are unambiguous on their own line.
fn pre_commit_references_zorilla(path: &Path) -> bool {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return false;
    };
    contents.lines().any(|line| {
        // Strip any trailing YAML comment: a repository that only *mentions*
        // zorilla in a comment has not wired it up.
        let line = line.split('#').next().unwrap_or_default();
        line.contains("mojzis/zorilla") || yaml_id_names_zorilla(line)
    })
}

/// Report whether a YAML line is an `id:` entry naming a zorilla hook.
///
/// Accepts `zorilla` and the `zorilla-*` family, both of which mean the
/// repository has already decided to run zorilla.
fn yaml_id_names_zorilla(line: &str) -> bool {
    let trimmed = line.trim().trim_start_matches("- ").trim_start();
    let Some(value) = trimmed.strip_prefix("id:") else {
        return false;
    };
    let value = value.trim().trim_matches(['"', '\''].as_slice());
    value == "zorilla" || value.starts_with("zorilla-")
}

/// The topic to print when the caller named none.
///
/// `tune` is never auto-selected: it is a reference, and nothing about a
/// repository's state says "you need the reference right now".
#[must_use]
pub fn auto_topic(source: Option<ConfigSource>) -> Topic {
    if source.is_some() {
        Topic::Triage
    } else {
        Topic::Setup
    }
}

/// The first line of every guide, naming the topic and how it was chosen.
fn header(topic: Topic, selection: Selection) -> String {
    match selection {
        Selection::Explicit => format!("# zorilla guide: {}", topic.name()),
        Selection::Auto(None) => {
            format!("# zorilla guide: not configured here -> {}", topic.name())
        }
        Selection::Auto(Some(source)) => {
            format!("# zorilla guide: configured via {} -> {}", source.label(), topic.name())
        }
    }
}

/// The complete guide output: header line, blank line, then the docs page
/// verbatim.
#[must_use]
pub fn render(topic: Topic, selection: Selection) -> String {
    format!("{}\n\n{}", header(topic, selection), topic.text())
}

/// One-line footer shown at the foot of any text report that surfaces findings.
///
/// This is the only breadcrumb from a failed gate to the instructions, so it is
/// printed whenever `text` output reports something, TTY or not — aggregators
/// and commit hooks capture stdout, and a footer that only appeared on a
/// terminal would be missing from every place it is actually needed. Returned
/// without a trailing newline so each formatter controls its own spacing.
#[must_use]
pub fn report_footer() -> &'static str {
    "Findings are not verdicts. Run `zorilla guide triage` for the remedy \
     ladder, including when `# zorilla: ignore -- <reason>` is the right answer."
}

/// Append the footer to a text report, blank line and all.
///
/// One emitter for both text formatters: `check` and `overview` decide *whether*
/// there is anything to triage, but they must not disagree about how the footer
/// is spaced.
pub(crate) fn push_report_footer(out: &mut String) {
    out.push('\n');
    out.push_str(report_footer());
    out.push('\n');
}

/// Every zorilla invocation the guides show, as argv vectors ready for clap.
///
/// Public because the check that matters — feeding each one through the real
/// `Command` — can only run where the `Cli` type lives, which is the binary. A
/// guide that shows a command the CLI would reject is worse than no guide.
///
/// Placeholders like `<changed file>` are dropped: they are holes for the reader
/// to fill, not arguments, and one of them contains a space.
#[must_use]
#[doc(hidden)]
pub fn embedded_invocations(topic: Topic) -> Vec<Vec<String>> {
    command_lines(topic.text()).iter().flat_map(|line| zorilla_invocations(line)).collect()
}

/// Command strings written in a guide: inline backtick spans that invoke
/// zorilla, plus every non-blank line of a fenced `bash` block.
fn command_lines(text: &str) -> Vec<String> {
    // Every span, not just those that open with `zorilla`: a span may be a
    // pipeline (`git diff | zorilla check --files-from -`) whose zorilla half is
    // the part worth checking. `zorilla_invocations` drops the rest.
    let mut out: Vec<String> = inline_code_spans(text);
    out.extend(
        lines_with_fence(text)
            .filter(|&(line, fence)| fence == Some("bash") && !line.trim().is_empty())
            .map(|(line, _)| line.to_owned()),
    );
    out
}

/// Split a command line on pipes and keep the segments that invoke zorilla.
fn zorilla_invocations(line: &str) -> Vec<Vec<String>> {
    line.split('|')
        .map(str::trim)
        .filter(|segment| is_zorilla_command(segment))
        .map(|segment| {
            segment
                .split_whitespace()
                // `<changed file>` splits into two tokens, so both brackets have
                // to be looked for; dropping either token alone leaves a stray.
                .filter(|token| !token.contains('<') && !token.contains('>'))
                .map(str::to_owned)
                .collect()
        })
        .collect()
}

/// Is this text the start of a `zorilla` invocation?
///
/// Deliberately not `starts_with("zorilla")`: that would swallow `zorilla.toml`
/// and feed the config file's name to clap as a subcommand.
fn is_zorilla_command(text: &str) -> bool {
    text == "zorilla" || text.starts_with("zorilla ")
}

/// Inline `code` spans, in source order. Fenced blocks are skipped: they hold
/// TOML and YAML, which the test module's `fenced_toml_entries` reads with its
/// own rules.
fn inline_code_spans(text: &str) -> Vec<String> {
    let mut spans = Vec::new();
    for (line, fence) in lines_with_fence(text) {
        if fence.is_some() {
            continue;
        }
        let mut rest = line;
        while let Some(open) = rest.find('`') {
            let after = &rest[open + 1..];
            let Some(close) = after.find('`') else { break };
            spans.push(after[..close].to_owned());
            rest = &after[close + 1..];
        }
    }
    spans
}

/// Each line of `text` paired with the info string of the fence it sits in, or
/// `None` when it sits outside one. Fence markers themselves are not yielded.
///
/// One walker for every extractor: they disagreed about what opened a fence for
/// exactly as long as there were two of them.
fn lines_with_fence(text: &str) -> impl Iterator<Item = (&str, Option<&str>)> {
    let mut fence: Option<&str> = None;
    text.lines().filter_map(move |line| {
        // `trim_start` so a fence indented inside a list item still registers;
        // otherwise its contents would be scanned as prose.
        if let Some(info) = line.trim_start().strip_prefix("```") {
            fence = if fence.is_some() { None } else { Some(info.trim()) };
            return None;
        }
        Some((line, fence))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::keys;
    use crate::rules::registry;

    /// No topic may exceed this many lines. Failing this test is the point of
    /// it: a guide that grows past a screenful stops being read.
    const LINE_CAP: usize = 60;

    fn write(dir: &Path, name: &str, contents: &str) {
        std::fs::write(dir.join(name), contents).expect("fixture write should succeed");
    }

    /// A temporary directory that reads as a repository root.
    ///
    /// The `.git` marker is what stops [`detect`]'s upward walk, which keeps
    /// every detection test hermetic: without it the walk would leave the
    /// tempdir and a stray `zorilla.toml` anywhere above `/tmp` would decide
    /// the result.
    fn tempdir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir should be creatable");
        std::fs::create_dir(dir.path().join(".git")).expect("git marker should be creatable");
        dir
    }

    // --- Content rules ---

    #[test]
    fn every_topic_fits_the_line_cap() {
        for topic in Topic::ALL {
            let lines = topic.text().trim_end().lines().count();
            assert!(
                lines <= LINE_CAP,
                "guide `{}` is {lines} lines, cap is {LINE_CAP}; cut it rather than raising \
                 the cap",
                topic.name(),
            );
        }
    }

    #[test]
    fn every_topic_is_plain_ascii() {
        for topic in Topic::ALL {
            let offender = topic.text().chars().find(|c| !c.is_ascii());
            assert!(
                offender.is_none(),
                "guide `{}` contains the non-ASCII character {offender:?}; guides are piped \
                 and captured, so they stay ASCII",
                topic.name(),
            );
        }
    }

    #[test]
    fn every_topic_ends_with_a_single_next_line() {
        for topic in Topic::ALL {
            let trimmed = topic.text().trim_end();
            let last = trimmed.lines().next_back().expect("guide should not be empty");
            assert!(
                last.starts_with("next: run "),
                "guide `{}` must end with a `next: run` line, ends with {last:?}",
                topic.name(),
            );
            let count = trimmed.lines().filter(|l| l.starts_with("next: run ")).count();
            assert_eq!(count, 1, "guide `{}` should have exactly one next line", topic.name());
            // The next line is the command an agent is likeliest to run, so it
            // has to be backticked: that is what puts it through
            // `inline_code_spans` and on to the clap check in the binary. A
            // bare `next: run zorilla fix --all` would otherwise ship green.
            assert!(
                inline_code_spans(last).iter().any(|span| is_zorilla_command(span)),
                "guide `{}` next line must quote its command in backticks so it is \
                 checked; got {last:?}",
                topic.name(),
            );
        }
    }

    #[test]
    fn triage_states_the_prohibitions_verbatim() {
        let text = Topic::Triage.text();
        for phrase in [
            "Do not add `# zorilla: ignore` without a reason.",
            "Do not suppress a finding to pass a gate.",
            "Do not weaken an assertion to satisfy a rule.",
        ] {
            assert!(text.contains(phrase), "triage guide should contain {phrase:?}");
        }
    }

    #[test]
    fn triage_names_every_rule_the_linter_ships() {
        // The ladder is the only place an agent is told what to *do* about a
        // code. A rule that ships without an entry leaves the agent guessing.
        let text = Topic::Triage.text();
        for rule in registry::all() {
            assert!(
                text.contains(rule.code()),
                "triage guide has no remedy for `{}` ({})",
                rule.code(),
                rule.name(),
            );
            assert!(
                text.contains(rule.name()),
                "triage guide names `{}` but not `{}`; a registry rename would drift silently",
                rule.code(),
                rule.name(),
            );
        }
    }

    #[test]
    fn setup_pins_the_current_release() {
        // The `rev:` in the pre-commit block is copy-pasted verbatim by whoever
        // reads this guide, so a stale pin silently holds them on an old
        // release. Bump it with the version.
        let expected = format!("rev: v{}", env!("CARGO_PKG_VERSION"));
        assert!(Topic::Setup.text().contains(&expected), "setup guide should pin `{expected}`",);
    }

    #[test]
    fn setup_recommends_the_dev_dependency_over_uvx() {
        let text = Topic::Setup.text();
        assert!(text.contains("uv add --dev zorilla"), "setup should show the dev dependency");
        assert!(text.contains("Prefer the dev"), "setup should recommend it over uvx");
    }

    #[test]
    fn setup_states_which_subcommand_gates() {
        // `stats` and `overview` always exit 0. Wiring either into a gate
        // produces a gate that can never fail, which is worse than no gate.
        // Matched on whitespace-normalised text: the guide sits under a line
        // cap, so reflowing this paragraph is a normal edit and must not fail
        // the test for a cosmetic reason.
        let text = Topic::Setup.text().split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(
            text.contains("`stats` and `overview` always exit 0"),
            "setup should say the report subcommands never fail a gate",
        );
    }

    #[test]
    fn setup_wires_zorilla_into_madoqua_and_says_a_path_list_is_valid() {
        // Braindump todo 132: the aesop dogfood set `pass_files = false`
        // blind because the guide did not say what a trailing file list does.
        let text = Topic::Setup.text().split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(text.contains("[tool.madoqua]"), "setup should show the madoqua step");
        assert!(
            text.contains("`zorilla check tests a.py b.py` lints the directory and both files"),
            "setup should say a mixed path list is valid",
        );
    }

    #[test]
    fn tune_documents_the_reason_syntax_and_precedence() {
        let text = Topic::Tune.text();
        assert!(text.contains("# zorilla: ignore -- <reason>"), "tune should show the reason form",);
        assert!(
            text.contains("`zorilla.toml` > `pyproject.toml [tool.zorilla]`"),
            "tune should state precedence",
        );
    }

    #[test]
    fn tune_documents_the_strict_same_line_rule() {
        // Readers arrive expecting other linters' "applies to the next line"
        // semantics. `Suppressions` does not implement that, so the guide has to
        // say so or every suppression it prompts lands one line too high.
        assert!(
            Topic::Tune.text().contains("on that same line"),
            "tune should state the strict same-line rule",
        );
    }

    // --- Every command in the guides must be a real invocation ---
    //
    // The commands are fed through the real clap `Command` in the binary's own
    // tests (`zorilla-cli/src/main.rs`), which is the only place the `Cli` type
    // exists. What is checked here is that the extraction those tests rely on
    // actually finds the commands, so an empty result can never pass as "all
    // valid".

    #[test]
    fn extraction_finds_every_command_the_guides_show() {
        let setup = embedded_invocations(Topic::Setup);
        assert!(
            setup.contains(&vec!["zorilla".to_owned(), "check".to_owned(), ".".to_owned()]),
            "setup shows `zorilla check .`, extraction returned {setup:?}",
        );
        assert!(
            setup.iter().any(|argv| argv.contains(&"--files-from".to_owned())),
            "the piped `git diff | zorilla check --files-from -` segment should be extracted",
        );
        let triage = embedded_invocations(Topic::Triage);
        assert!(
            triage.contains(&vec![
                "zorilla".to_owned(),
                "check".to_owned(),
                "tests/test_orders.py".to_owned(),
            ]),
            "the re-check recipe lives in a bash fence and should be extracted; got {triage:?}",
        );
        for topic in Topic::ALL {
            let found = embedded_invocations(topic);
            assert!(!found.is_empty(), "guide `{}` shows no commands at all", topic.name());
            for argv in &found {
                assert_eq!(argv.first().map(String::as_str), Some("zorilla"), "argv is {argv:?}");
            }
        }
    }

    #[test]
    fn extraction_does_not_mistake_the_config_file_for_a_command() {
        // `zorilla.toml` is named in every guide. Fed to clap it would be an
        // unknown subcommand, so the "every command parses" test would fail on
        // prose that is perfectly correct.
        for topic in Topic::ALL {
            for argv in embedded_invocations(topic) {
                assert!(
                    !argv.iter().any(|token| token.contains(".toml")),
                    "guide `{}` yielded {argv:?}, which is a filename, not a command",
                    topic.name(),
                );
            }
        }
    }

    // --- Every rule code and config key in the guides must exist ---

    #[test]
    fn every_rule_code_in_the_guides_is_registered() {
        let mut checked = 0_usize;
        for topic in Topic::ALL {
            for code in rule_codes_mentioned(topic.text()) {
                checked += 1;
                assert!(
                    registry::find(&code).is_some(),
                    "guide `{}` names rule `{code}`, which is not registered",
                    topic.name(),
                );
            }
        }
        assert!(checked >= 8, "expected the guides to name several rules, found {checked}");
    }

    #[test]
    fn every_qualified_config_key_in_the_guides_exists() {
        let accepted = keys::qualified_rule_keys();
        let mut checked = 0_usize;
        for topic in Topic::ALL {
            for span in inline_code_spans(topic.text()) {
                let Some((code, key)) = span.split_once('.') else { continue };
                if !looks_like_rule_code(code) {
                    continue;
                }
                checked += 1;
                assert!(
                    accepted.contains(&span),
                    "guide `{}` names `{code}.{key}`, which the config deserializer does not \
                     accept under that rule",
                    topic.name(),
                );
            }
        }
        assert!(
            checked >= 5,
            "expected the guides to name several per-rule knobs as `CODE.key`, found {checked}",
        );
    }

    #[test]
    fn every_config_key_in_a_toml_fence_exists() {
        let mut checked = 0_usize;
        for topic in Topic::ALL {
            for entry in fenced_toml_entries(topic.text()) {
                let Some(scope) = zorilla_scope(&entry) else { continue };
                checked += 1;
                let accepted = match &scope {
                    // Checked against the exact table the key sits in, not
                    // against one flat set: `max_asserts` is a real key, and a
                    // flat set would accept it under `[tool.zorilla]`, where
                    // zorilla parses and discards it.
                    FenceScope::TopLevel => keys::top_level_keys().contains(&entry.key.as_str()),
                    FenceScope::Rule(code) => keys::keys_for_code(code)
                        .is_some_and(|fields| fields.contains(&entry.key.as_str())),
                };
                assert!(
                    accepted,
                    "guide `{}` shows `{}` under `[{}]`, which the config deserializer does \
                     not accept there",
                    topic.name(),
                    entry.key,
                    entry.table,
                );
            }
        }
        assert!(
            checked >= 4,
            "expected the guides' toml examples to set several keys, found {checked}",
        );
    }

    /// A `key = value` line from a `toml` fence, with the table it sits under.
    ///
    /// `table` is the header verbatim (`tool.zorilla.rules.ZR004`), so the caller
    /// decides which tables are zorilla's to check.
    #[derive(Debug, PartialEq, Eq)]
    struct TomlEntry {
        table: String,
        key: String,
    }

    /// Every `key = value` line inside the guides' ```` ```toml ```` fences, paired
    /// with its table header.
    ///
    /// Guides show config in fenced blocks, which [`inline_code_spans`] skips — so
    /// without this the example that a reader copy-pastes would be the one part of a
    /// guide nothing checks.
    fn fenced_toml_entries(text: &str) -> Vec<TomlEntry> {
        let mut out = Vec::new();
        let mut table = String::new();
        let mut previous_fence = None;
        for (line, fence) in lines_with_fence(text) {
            // A header does not survive the fence it was written in: a later
            // block opening with a bare `key = value` must not be attributed to
            // the previous block's table and checked against the wrong keys.
            if fence != previous_fence {
                table.clear();
                previous_fence = fence;
            }
            if fence != Some("toml") {
                continue;
            }
            let trimmed = line.trim();
            if let Some(header) = trimmed.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                table = header.to_owned();
                continue;
            }
            if let Some((key, _)) = trimmed.split_once('=') {
                let key = key.trim();
                if !key.is_empty() {
                    out.push(TomlEntry { table: table.clone(), key: key.to_owned() });
                }
            }
        }
        out
    }

    /// Which zorilla config table a fenced `key = value` line sits in.
    #[derive(Debug, PartialEq, Eq)]
    enum FenceScope {
        /// Directly under `[tool.zorilla]` / the root of a `zorilla.toml`.
        TopLevel,
        /// Under `[tool.zorilla.rules.<code>]`, carrying that code.
        Rule(String),
    }

    /// Place a fenced `key = value` line in zorilla's config namespace, or
    /// `None` when the table belongs to some other tool — a `[tool.poe.tasks]`
    /// example is a legitimate thing for the setup guide to show.
    fn zorilla_scope(entry: &TomlEntry) -> Option<FenceScope> {
        let table = entry.table.strip_prefix("tool.").unwrap_or(&entry.table);
        let rest = table.strip_prefix("zorilla")?;
        let rest = rest.strip_prefix('.').unwrap_or(rest);
        if rest.is_empty() {
            return Some(FenceScope::TopLevel);
        }
        rest.strip_prefix("rules.").map(|code| FenceScope::Rule(code.to_owned()))
    }

    /// Every `ZR###` token in `text`, in occurrence order, repeats included.
    ///
    /// Byte windows rather than a regex: `every_topic_is_plain_ascii` already
    /// guarantees the text is ASCII, and this saves a dependency.
    fn rule_codes_mentioned(text: &str) -> Vec<String> {
        text.as_bytes()
            .windows(5)
            .filter_map(|w| std::str::from_utf8(w).ok())
            .filter(|w| looks_like_rule_code(w))
            .map(str::to_owned)
            .collect()
    }

    /// `ZR` followed by exactly three digits.
    fn looks_like_rule_code(s: &str) -> bool {
        s.len() == 5 && s.starts_with("ZR") && s[2..].bytes().all(|b| b.is_ascii_digit())
    }

    // --- Detection ---

    #[test]
    fn empty_directory_is_not_configured() {
        let dir = tempdir();
        assert_eq!(detect(dir.path()), None);
        assert_eq!(auto_topic(detect(dir.path())), Topic::Setup);
    }

    #[test]
    fn zorilla_toml_is_configured() {
        let dir = tempdir();
        write(dir.path(), "zorilla.toml", "include = [\"tests/**/*.py\"]\n");
        assert_eq!(detect(dir.path()), Some(ConfigSource::ZorillaToml));
        assert_eq!(auto_topic(detect(dir.path())), Topic::Triage);
    }

    #[test]
    fn pyproject_with_tool_zorilla_is_configured() {
        let dir = tempdir();
        write(dir.path(), "pyproject.toml", "[project]\nname = \"x\"\n\n[tool.zorilla.rules]\n");
        assert_eq!(detect(dir.path()), Some(ConfigSource::PyProject));
    }

    #[test]
    fn config_above_the_start_directory_is_configured() {
        // `Config::discover` walks upward, so a lint run from `tests/unit`
        // honours the root config. If `detect` did not, `guide` would hand a
        // configured repository the setup instructions.
        let root = tempdir();
        write(root.path(), "pyproject.toml", "[tool.zorilla]\n");
        let nested = root.path().join("tests").join("unit");
        std::fs::create_dir_all(&nested).expect("nested dirs should be creatable");
        assert_eq!(detect(&nested), Some(ConfigSource::PyProject));
        assert_eq!(auto_topic(detect(&nested)), Topic::Triage);
    }

    #[test]
    fn the_walk_stops_at_the_repository_root() {
        // A `zorilla.toml` outside the checkout is not this repository's
        // configuration, however close it sits on disk.
        let outer = tempfile::tempdir().expect("tempdir should be creatable");
        write(outer.path(), "zorilla.toml", "include = []\n");
        let repo = outer.path().join("repo");
        std::fs::create_dir_all(repo.join(".git")).expect("repo should be creatable");
        assert_eq!(detect(&repo), None);
    }

    #[test]
    fn pyproject_without_tool_zorilla_is_not_configured() {
        let dir = tempdir();
        write(dir.path(), "pyproject.toml", "[project]\nname = \"x\"\n\n[tool.ruff]\n");
        assert_eq!(detect(dir.path()), None);
    }

    #[test]
    fn pyproject_mentioning_zorilla_in_a_string_is_not_configured() {
        let dir = tempdir();
        write(dir.path(), "pyproject.toml", "[project]\ndependencies = [\"tool.zorilla\"]\n");
        assert_eq!(
            detect(dir.path()),
            None,
            "detection parses the TOML, so a mention in a string is not a config table",
        );
    }

    #[test]
    fn unparseable_pyproject_is_not_configured() {
        let dir = tempdir();
        write(dir.path(), "pyproject.toml", "[project\nname = \n");
        assert_eq!(detect(dir.path()), None);
    }

    #[test]
    fn unreadable_pyproject_is_not_configured() {
        // A directory where a file should be: `read_to_string` fails rather than
        // returning bad TOML, which is the other arm of `detect`'s tolerance.
        let dir = tempdir();
        std::fs::create_dir(dir.path().join("pyproject.toml")).expect("create dir");
        assert_eq!(
            detect(dir.path()),
            None,
            "an unreadable file is 'not configured', not a reason to refuse to print",
        );
    }

    #[test]
    fn pre_commit_repo_reference_is_configured() {
        let dir = tempdir();
        write(
            dir.path(),
            ".pre-commit-config.yaml",
            "repos:\n  - repo: https://github.com/mojzis/zorilla\n    rev: v0.1.4\n    \
             hooks:\n      - id: zorilla\n",
        );
        assert_eq!(detect(dir.path()), Some(ConfigSource::PreCommit));
    }

    #[test]
    fn pre_commit_local_hook_id_is_configured() {
        let dir = tempdir();
        write(
            dir.path(),
            ".pre-commit-config.yaml",
            "repos:\n  - repo: local\n    hooks:\n      - id: zorilla\n        entry: \
             zorilla check\n",
        );
        assert_eq!(detect(dir.path()), Some(ConfigSource::PreCommit));
    }

    #[test]
    fn pre_commit_without_zorilla_is_not_configured() {
        let dir = tempdir();
        write(
            dir.path(),
            ".pre-commit-config.yaml",
            "repos:\n  - repo: local\n    hooks:\n      - id: ruff\n",
        );
        assert_eq!(detect(dir.path()), None);
    }

    #[test]
    fn pre_commit_mention_in_a_comment_is_not_configured() {
        let dir = tempdir();
        write(
            dir.path(),
            ".pre-commit-config.yaml",
            "repos: []\n# consider mojzis/zorilla one day\n",
        );
        assert_eq!(detect(dir.path()), None);
    }

    #[test]
    fn config_file_wins_over_pyproject_and_pre_commit() {
        let dir = tempdir();
        write(dir.path(), "zorilla.toml", "include = []\n");
        write(dir.path(), "pyproject.toml", "[tool.zorilla]\n");
        write(
            dir.path(),
            ".pre-commit-config.yaml",
            "repos:\n  - repo: https://github.com/mojzis/zorilla\n",
        );
        assert_eq!(detect(dir.path()), Some(ConfigSource::ZorillaToml));
    }

    #[test]
    fn pyproject_wins_over_pre_commit() {
        let dir = tempdir();
        write(dir.path(), "pyproject.toml", "[tool.zorilla]\n");
        write(
            dir.path(),
            ".pre-commit-config.yaml",
            "repos:\n  - repo: https://github.com/mojzis/zorilla\n",
        );
        assert_eq!(detect(dir.path()), Some(ConfigSource::PyProject));
    }

    // --- Rendering ---

    #[test]
    fn explicit_header_omits_the_arrow() {
        assert_eq!(header(Topic::Tune, Selection::Explicit), "# zorilla guide: tune");
        assert_eq!(header(Topic::Setup, Selection::Explicit), "# zorilla guide: setup");
    }

    #[test]
    fn auto_header_names_the_reason() {
        assert_eq!(
            header(Topic::Setup, Selection::Auto(None)),
            "# zorilla guide: not configured here -> setup"
        );
        assert_eq!(
            header(Topic::Triage, Selection::Auto(Some(ConfigSource::PyProject))),
            "# zorilla guide: configured via pyproject.toml [tool.zorilla] -> triage"
        );
        assert_eq!(
            header(Topic::Triage, Selection::Auto(Some(ConfigSource::ZorillaToml))),
            "# zorilla guide: configured via zorilla.toml -> triage"
        );
        assert_eq!(
            header(Topic::Triage, Selection::Auto(Some(ConfigSource::PreCommit))),
            "# zorilla guide: configured via .pre-commit-config.yaml -> triage"
        );
    }

    #[test]
    fn render_is_the_header_then_the_docs_page_verbatim() {
        let rendered = render(Topic::Triage, Selection::Explicit);
        let expected = format!("# zorilla guide: triage\n\n{}", Topic::Triage.text());
        assert_eq!(rendered, expected);
        assert!(
            rendered.ends_with(Topic::Triage.text()),
            "the CLI must emit the docs page byte for byte",
        );
    }

    // --- Footer ---

    #[test]
    fn footer_points_at_the_triage_guide_and_the_reason_syntax() {
        let footer = report_footer();
        assert!(
            footer.contains("zorilla guide triage"),
            "the footer is the only path from a failed gate to the guide",
        );
        assert!(footer.contains("# zorilla: ignore"), "the footer shows the inline directive");
        assert!(!footer.contains('\n'), "the footer is one line; callers add the newline");
        assert!(footer.is_ascii(), "the footer is piped and captured, so it stays ASCII");
    }
}
