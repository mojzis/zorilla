//! Configuration loading for zorilla.
//!
//! Two surfaces live here:
//!
//! 1. [`Config`] — discovery globs and raw per-rule tables as read from
//!    either `[tool.zorilla]` in `pyproject.toml` or a standalone
//!    `zorilla.toml`. This is what the CLI constructs and threads into
//!    [`crate::lint`].
//! 2. [`RuleConfig`] — a typed per-rule projection of the above, computed
//!    by [`Config::rule_config`] and handed to every rule via
//!    [`crate::rules::Context::config`]. Rules read only from
//!    [`RuleConfig`]; the raw TOML tables never reach them.
//!
//! Rule-specific config shapes defined so far: [`Zr003Config`] (assertion
//! helpers), [`Zr004Config`] (`max_asserts`), [`Zr005Config`]
//! (`allowed_prefixes`), [`Zr006Config`] (`max_patches`), and
//! [`Zr008Config`] (`max_patches` for context-manager patches). Future
//! phases add more.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Default include globs — pytest's idiomatic layouts.
pub const DEFAULT_INCLUDE: &[&str] =
    &["tests/**/*.py", "**/test_*.py", "**/*_test.py", "**/conftest.py"];

/// Default exclude globs.
pub const DEFAULT_EXCLUDE: &[&str] = &["**/fixtures/**"];

/// Default `ZR004.max_asserts` — anything strictly greater fires the rule.
pub const DEFAULT_ZR004_MAX_ASSERTS: usize = 4;

/// Default `ZR006.max_patches` — anything strictly greater fires the rule.
pub const DEFAULT_ZR006_MAX_PATCHES: usize = 3;

/// Default `ZR008.max_patches` — anything strictly greater fires the rule.
pub const DEFAULT_ZR008_MAX_PATCHES: usize = 3;

/// Built-in assertion-helper names for ZR003.
///
/// Match is by the **final identifier** of the call target — so
/// `self.assertEqual(...)` and a bare `assertEqual(...)` both match
/// `assertEqual`. Users extend this list via
/// `[tool.zorilla.rules.ZR003] extra_helpers = [...]`.
pub const DEFAULT_ZR003_HELPERS: &[&str] = &[
    "assertEqual",
    "assertNotEqual",
    "assertTrue",
    "assertFalse",
    "assertIs",
    "assertIsNot",
    "assertIsNone",
    "assertIsNotNone",
    "assertIn",
    "assertNotIn",
    "assertIsInstance",
    "assertNotIsInstance",
    "assertRaises",
    "assertRaisesRegex",
    "assertWarns",
    "assertWarnsRegex",
    "assertAlmostEqual",
    "assertNotAlmostEqual",
    "assertGreater",
    "assertGreaterEqual",
    "assertLess",
    "assertLessEqual",
    "assertRegex",
    "assertNotRegex",
    "assertCountEqual",
    "assertListEqual",
    "assertTupleEqual",
    "assertDictEqual",
    "assertSetEqual",
    "assertMultiLineEqual",
    "assertSequenceEqual",
    "fail",
];

/// Effective configuration after merging file + defaults.
#[derive(Debug, Clone)]
pub struct Config {
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    /// Raw per-rule tables as parsed from TOML.
    ///
    /// Exposed so [`Self::rule_config`] can project them into the typed
    /// per-rule structs used by the engine. Kept crate-visible-only — the
    /// public API is `rule_config()`.
    rules: RawRules,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            include: DEFAULT_INCLUDE.iter().map(|s| (*s).to_string()).collect(),
            exclude: DEFAULT_EXCLUDE.iter().map(|s| (*s).to_string()).collect(),
            rules: RawRules::default(),
        }
    }
}

/// Typed per-rule configuration, derived from [`Config`].
///
/// The engine builds one of these per `lint()` call and threads it into
/// every [`crate::rules::Context`]. Rules read fields off it directly;
/// they never see raw TOML.
#[derive(Debug, Clone, Default)]
pub struct RuleConfig {
    /// Rule codes explicitly disabled via `[tool.zorilla.rules.ZR00X]
    /// enabled = false`.
    pub disabled: HashSet<&'static str>,
    /// Knobs for ZR003.
    pub zr003: Zr003Config,
    /// Knobs for ZR004.
    pub zr004: Zr004Config,
    /// Knobs for ZR005.
    pub zr005: Zr005Config,
    /// Knobs for ZR006.
    pub zr006: Zr006Config,
    /// Knobs for ZR008.
    pub zr008: Zr008Config,
}

/// Typed knobs for `ZR003 no-assertion`.
#[derive(Debug, Clone)]
pub struct Zr003Config {
    /// Merged set of built-in + user-supplied assertion helper names.
    ///
    /// Matching is by the final identifier of a call target (see
    /// [`crate::ast::call_final_name`]).
    pub helpers: HashSet<String>,
}

impl Default for Zr003Config {
    fn default() -> Self {
        Self { helpers: DEFAULT_ZR003_HELPERS.iter().map(|s| (*s).to_string()).collect() }
    }
}

/// Typed knobs for `ZR004 assertion-roulette`.
#[derive(Debug, Clone)]
pub struct Zr004Config {
    /// Maximum number of bare `assert` statements allowed per test. The
    /// rule fires when `bare_count > max_asserts`.
    pub max_asserts: usize,
}

impl Default for Zr004Config {
    fn default() -> Self {
        Self { max_asserts: DEFAULT_ZR004_MAX_ASSERTS }
    }
}

/// Typed knobs for `ZR005 mystery-guest`.
#[derive(Debug, Clone, Default)]
pub struct Zr005Config {
    /// String-literal prefixes that are *not* flagged as mystery guests.
    ///
    /// If a literal would otherwise fire (absolute path / URL / home path)
    /// but starts with any of these prefixes, the finding is suppressed.
    /// Matching is plain `str::starts_with`, case-sensitive, no globbing.
    /// Empty-string entries are stripped at load time (an empty prefix
    /// would silence every literal — almost always a config typo).
    pub allowed_prefixes: Vec<String>,
}

/// Typed knobs for `ZR006 patch-stack`.
#[derive(Debug, Clone)]
pub struct Zr006Config {
    /// Maximum number of stacked `@patch` decorators allowed per test.
    /// The rule fires when `patch_count > max_patches`.
    pub max_patches: usize,
}

impl Default for Zr006Config {
    fn default() -> Self {
        Self { max_patches: DEFAULT_ZR006_MAX_PATCHES }
    }
}

/// Typed knobs for `ZR008 context-patch-stack`.
#[derive(Debug, Clone)]
pub struct Zr008Config {
    /// Maximum number of `patch(...)` context-manager items allowed per
    /// test. The rule fires when `patch_count > max_patches`.
    pub max_patches: usize,
}

impl Default for Zr008Config {
    fn default() -> Self {
        Self { max_patches: DEFAULT_ZR008_MAX_PATCHES }
    }
}

/// Raw `[tool.zorilla]` section as it appears in TOML.
#[derive(Debug, Default, Deserialize, Clone)]
struct RawConfig {
    #[serde(default)]
    include: Option<Vec<String>>,
    #[serde(default)]
    exclude: Option<Vec<String>>,
    #[serde(default)]
    rules: RawRules,
}

/// `[tool.zorilla.rules]` container — one field per known ZR code.
#[derive(Debug, Default, Deserialize, Clone)]
struct RawRules {
    #[serde(default, rename = "ZR001")]
    zr001: RawRuleCommon,
    #[serde(default, rename = "ZR002")]
    zr002: RawRuleCommon,
    #[serde(default, rename = "ZR003")]
    zr003: RawZr003,
    #[serde(default, rename = "ZR004")]
    zr004: RawZr004,
    #[serde(default, rename = "ZR005")]
    zr005: RawZr005,
    #[serde(default, rename = "ZR006")]
    zr006: RawZr006,
    #[serde(default, rename = "ZR007")]
    zr007: RawRuleCommon,
    #[serde(default, rename = "ZR008")]
    zr008: RawZr008,
}

#[derive(Debug, Default, Deserialize, Clone)]
struct RawRuleCommon {
    #[serde(default)]
    enabled: Option<bool>,
}

#[derive(Debug, Default, Deserialize, Clone)]
struct RawZr003 {
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    extra_helpers: Option<Vec<String>>,
}

#[derive(Debug, Default, Deserialize, Clone)]
struct RawZr004 {
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    max_asserts: Option<usize>,
}

#[derive(Debug, Default, Deserialize, Clone)]
struct RawZr005 {
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    allowed_prefixes: Option<Vec<String>>,
}

#[derive(Debug, Default, Deserialize, Clone)]
struct RawZr006 {
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    max_patches: Option<usize>,
}

#[derive(Debug, Default, Deserialize, Clone)]
struct RawZr008 {
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    max_patches: Option<usize>,
}

#[derive(Debug, Default, Deserialize)]
struct PyprojectTool {
    #[serde(default)]
    zorilla: Option<RawConfig>,
}

#[derive(Debug, Default, Deserialize)]
struct Pyproject {
    #[serde(default)]
    tool: Option<PyprojectTool>,
}

/// Error surfaced when config loading fails.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// IO error reading a candidate config file.
    #[error("reading {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// TOML parse failure.
    #[error("parsing {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
}

impl Config {
    /// Load configuration by searching upward from `start` for either
    /// `pyproject.toml` (with `[tool.zorilla]`) or `zorilla.toml`. If
    /// nothing is found, returns [`Config::default`].
    ///
    /// `start` is usually the current working directory.
    pub fn discover(start: &Path) -> Result<Self, ConfigError> {
        let mut current = Some(start);
        while let Some(dir) = current {
            let zorilla_toml = dir.join("zorilla.toml");
            if zorilla_toml.is_file() {
                return Self::load_standalone(&zorilla_toml);
            }

            let pyproject = dir.join("pyproject.toml");
            if pyproject.is_file() {
                if let Some(cfg) = Self::try_load_pyproject(&pyproject)? {
                    return Ok(cfg);
                }
            }

            current = dir.parent();
        }
        Ok(Self::default())
    }

    /// Load `zorilla.toml` directly.
    pub fn load_standalone(path: &Path) -> Result<Self, ConfigError> {
        let text = std::fs::read_to_string(path)
            .map_err(|source| ConfigError::Io { path: path.to_path_buf(), source })?;
        let raw: RawConfig = toml::from_str(&text)
            .map_err(|source| ConfigError::Parse { path: path.to_path_buf(), source })?;
        Ok(Self::from_raw(raw))
    }

    fn try_load_pyproject(path: &Path) -> Result<Option<Self>, ConfigError> {
        let text = std::fs::read_to_string(path)
            .map_err(|source| ConfigError::Io { path: path.to_path_buf(), source })?;
        let parsed: Pyproject = toml::from_str(&text)
            .map_err(|source| ConfigError::Parse { path: path.to_path_buf(), source })?;
        let raw = parsed.tool.and_then(|t| t.zorilla);
        Ok(raw.map(Self::from_raw))
    }

    fn from_raw(raw: RawConfig) -> Self {
        let defaults = Self::default();
        Self {
            include: raw.include.unwrap_or(defaults.include),
            exclude: raw.exclude.unwrap_or(defaults.exclude),
            rules: raw.rules,
        }
    }

    /// Project the raw rule tables into the typed [`RuleConfig`] the
    /// engine + rules consume.
    #[must_use]
    pub fn rule_config(&self) -> RuleConfig {
        let mut disabled = HashSet::new();
        if self.rules.zr001.enabled == Some(false) {
            disabled.insert("ZR001");
        }
        if self.rules.zr002.enabled == Some(false) {
            disabled.insert("ZR002");
        }
        if self.rules.zr003.enabled == Some(false) {
            disabled.insert("ZR003");
        }
        if self.rules.zr004.enabled == Some(false) {
            disabled.insert("ZR004");
        }
        if self.rules.zr005.enabled == Some(false) {
            disabled.insert("ZR005");
        }
        if self.rules.zr006.enabled == Some(false) {
            disabled.insert("ZR006");
        }
        if self.rules.zr007.enabled == Some(false) {
            disabled.insert("ZR007");
        }
        if self.rules.zr008.enabled == Some(false) {
            disabled.insert("ZR008");
        }

        let mut helpers = Zr003Config::default().helpers;
        if let Some(extra) = &self.rules.zr003.extra_helpers {
            for h in extra {
                helpers.insert(h.clone());
            }
        }

        let max_asserts = self.rules.zr004.max_asserts.unwrap_or(DEFAULT_ZR004_MAX_ASSERTS);

        // Drop empty entries: `""` would be a `starts_with("")` wildcard
        // that silences every literal — almost certainly a config typo.
        let allowed_prefixes: Vec<String> = self
            .rules
            .zr005
            .allowed_prefixes
            .clone()
            .unwrap_or_default()
            .into_iter()
            .filter(|p| !p.is_empty())
            .collect();

        let max_patches = self.rules.zr006.max_patches.unwrap_or(DEFAULT_ZR006_MAX_PATCHES);

        let zr008_max_patches = self.rules.zr008.max_patches.unwrap_or(DEFAULT_ZR008_MAX_PATCHES);

        RuleConfig {
            disabled,
            zr003: Zr003Config { helpers },
            zr004: Zr004Config { max_asserts },
            zr005: Zr005Config { allowed_prefixes },
            zr006: Zr006Config { max_patches },
            zr008: Zr008Config { max_patches: zr008_max_patches },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn defaults_populate_include_and_exclude() {
        let cfg = Config::default();
        assert!(cfg.include.iter().any(|g| g == "tests/**/*.py"));
        assert!(cfg.exclude.iter().any(|g| g == "**/fixtures/**"));
    }

    #[test]
    fn discover_with_no_config_returns_defaults() {
        let tmp = TempDir::new().unwrap();
        let cfg = Config::discover(tmp.path()).unwrap();
        assert_eq!(cfg.include, Config::default().include);
    }

    #[test]
    fn discover_reads_pyproject_tool_zorilla() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("pyproject.toml"),
            "[tool.zorilla]\ninclude = [\"tests/unit/**/*.py\"]\n",
        )
        .unwrap();
        let cfg = Config::discover(tmp.path()).unwrap();
        assert_eq!(cfg.include, vec!["tests/unit/**/*.py".to_string()]);
        // exclude falls back to default when omitted.
        assert_eq!(cfg.exclude, Config::default().exclude);
    }

    #[test]
    fn discover_prefers_zorilla_toml_over_pyproject() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("pyproject.toml"),
            "[tool.zorilla]\ninclude = [\"from-pyproject/**/*.py\"]\n",
        )
        .unwrap();
        std::fs::write(tmp.path().join("zorilla.toml"), "include = [\"from-zorilla/**/*.py\"]\n")
            .unwrap();
        let cfg = Config::discover(tmp.path()).unwrap();
        assert_eq!(cfg.include, vec!["from-zorilla/**/*.py".to_string()]);
    }

    #[test]
    fn pyproject_without_tool_zorilla_falls_back_to_defaults() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("pyproject.toml"), "[project]\nname = \"unrelated\"\n")
            .unwrap();
        let cfg = Config::discover(tmp.path()).unwrap();
        assert_eq!(cfg.include, Config::default().include);
    }

    #[test]
    fn rule_config_defaults_are_sane() {
        let rc = Config::default().rule_config();
        assert!(rc.disabled.is_empty());
        assert!(rc.zr003.helpers.contains("assertEqual"));
        assert!(rc.zr003.helpers.contains("fail"));
        assert_eq!(rc.zr004.max_asserts, DEFAULT_ZR004_MAX_ASSERTS);
    }

    #[test]
    fn rule_config_disables_listed_rules() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("zorilla.toml"),
            "[rules.ZR003]\nenabled = false\n\
             [rules.ZR001]\nenabled = false\n\
             [rules.ZR008]\nenabled = false\n",
        )
        .unwrap();
        let cfg = Config::discover(tmp.path()).unwrap();
        let rc = cfg.rule_config();
        assert!(rc.disabled.contains("ZR001"));
        assert!(rc.disabled.contains("ZR003"));
        assert!(rc.disabled.contains("ZR008"));
        assert!(!rc.disabled.contains("ZR002"));
    }

    #[test]
    fn rule_config_merges_extra_helpers_into_zr003() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("zorilla.toml"),
            "[rules.ZR003]\nextra_helpers = [\"expect\", \"verify\"]\n",
        )
        .unwrap();
        let cfg = Config::discover(tmp.path()).unwrap();
        let rc = cfg.rule_config();
        assert!(rc.zr003.helpers.contains("expect"));
        assert!(rc.zr003.helpers.contains("verify"));
        // Built-ins still there.
        assert!(rc.zr003.helpers.contains("assertEqual"));
    }

    #[test]
    fn rule_config_reads_zr004_max_asserts() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("zorilla.toml"), "[rules.ZR004]\nmax_asserts = 2\n")
            .unwrap();
        let cfg = Config::discover(tmp.path()).unwrap();
        assert_eq!(cfg.rule_config().zr004.max_asserts, 2);
    }

    #[test]
    fn rule_config_toml_keys_are_case_sensitive_uppercase() {
        // Contract: PLAN.md §Configuration specifies uppercase TOML keys
        // (`[rules.ZR003]`). The serde `rename = "ZR003"` attribute makes
        // matching case-sensitive, so `[rules.zr003]` is silently
        // ignored. This test documents that behavior — change it
        // deliberately if we ever start accepting both cases.
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("zorilla.toml"), "[rules.zr003]\nenabled = false\n")
            .unwrap();
        let cfg = Config::discover(tmp.path()).unwrap();
        let rc = cfg.rule_config();
        assert!(
            !rc.disabled.contains("ZR003"),
            "lowercase key `zr003` must not disable ZR003; only `ZR003` does"
        );

        // Sanity: the uppercase form works.
        std::fs::write(tmp.path().join("zorilla.toml"), "[rules.ZR003]\nenabled = false\n")
            .unwrap();
        let cfg = Config::discover(tmp.path()).unwrap();
        assert!(cfg.rule_config().disabled.contains("ZR003"));
    }

    #[test]
    fn rule_config_reads_zr005_allowed_prefixes() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("zorilla.toml"),
            "[rules.ZR005]\nallowed_prefixes = [\"http://localhost\", \"/tmp/\"]\n",
        )
        .unwrap();
        let cfg = Config::discover(tmp.path()).unwrap();
        let rc = cfg.rule_config();
        assert_eq!(
            rc.zr005.allowed_prefixes,
            vec!["http://localhost".to_string(), "/tmp/".to_string()]
        );
    }

    #[test]
    fn rule_config_defaults_zr005_allowed_prefixes_empty() {
        let rc = Config::default().rule_config();
        assert!(rc.zr005.allowed_prefixes.is_empty());
    }

    #[test]
    fn rule_config_strips_empty_zr005_allowed_prefixes() {
        // An empty-string entry would make `starts_with("")` true for
        // every literal, silently disabling ZR005. Drop it at load time.
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("zorilla.toml"),
            "[rules.ZR005]\nallowed_prefixes = [\"\", \"/tmp/\", \"\"]\n",
        )
        .unwrap();
        let cfg = Config::discover(tmp.path()).unwrap();
        assert_eq!(cfg.rule_config().zr005.allowed_prefixes, vec!["/tmp/".to_string()]);
    }

    #[test]
    fn rule_config_reads_zr006_max_patches() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("zorilla.toml"), "[rules.ZR006]\nmax_patches = 5\n")
            .unwrap();
        let cfg = Config::discover(tmp.path()).unwrap();
        assert_eq!(cfg.rule_config().zr006.max_patches, 5);
    }

    #[test]
    fn rule_config_defaults_zr006_max_patches() {
        let rc = Config::default().rule_config();
        assert_eq!(rc.zr006.max_patches, DEFAULT_ZR006_MAX_PATCHES);
    }

    #[test]
    fn rule_config_reads_zr008_max_patches() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("zorilla.toml"), "[rules.ZR008]\nmax_patches = 5\n")
            .unwrap();
        let cfg = Config::discover(tmp.path()).unwrap();
        assert_eq!(cfg.rule_config().zr008.max_patches, 5);
    }

    #[test]
    fn rule_config_defaults_zr008_max_patches() {
        let rc = Config::default().rule_config();
        assert_eq!(rc.zr008.max_patches, DEFAULT_ZR008_MAX_PATCHES);
    }

    #[test]
    fn rule_config_zr008_is_enabled_by_default() {
        // No config file => no `enabled = false` for ZR008 => not in
        // `disabled`. Mirrors how the engine treats every default-on rule.
        let rc = Config::default().rule_config();
        assert!(!rc.disabled.contains("ZR008"));
    }

    #[test]
    fn rule_config_reads_pyproject_rules_table() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("pyproject.toml"),
            "[tool.zorilla.rules.ZR004]\nmax_asserts = 7\n",
        )
        .unwrap();
        let cfg = Config::discover(tmp.path()).unwrap();
        assert_eq!(cfg.rule_config().zr004.max_asserts, 7);
    }
}
