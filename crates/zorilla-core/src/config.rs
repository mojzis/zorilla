//! Configuration loading for zorilla.
//!
//! Phase 1 exposes only `include` / `exclude` globs. The loader searches
//! upward from the starting directory for either a `pyproject.toml`
//! containing `[tool.zorilla]` or a standalone `zorilla.toml`. The first
//! match wins; CLI flags are expected to override values in memory after
//! loading.

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Default include globs — pytest's idiomatic layouts.
pub const DEFAULT_INCLUDE: &[&str] =
    &["tests/**/*.py", "**/test_*.py", "**/*_test.py", "**/conftest.py"];

/// Default exclude globs.
pub const DEFAULT_EXCLUDE: &[&str] = &["**/fixtures/**"];

/// Effective configuration after merging file + defaults.
#[derive(Debug, Clone)]
pub struct Config {
    pub include: Vec<String>,
    pub exclude: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            include: DEFAULT_INCLUDE.iter().map(|s| (*s).to_string()).collect(),
            exclude: DEFAULT_EXCLUDE.iter().map(|s| (*s).to_string()).collect(),
        }
    }
}

/// Raw `[tool.zorilla]` section as it appears in TOML.
#[derive(Debug, Default, Deserialize)]
struct RawConfig {
    #[serde(default)]
    include: Option<Vec<String>>,
    #[serde(default)]
    exclude: Option<Vec<String>>,
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
}
