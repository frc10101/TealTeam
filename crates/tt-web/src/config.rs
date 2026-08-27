//! Configuration loading (F4).
//!
//! Precedence, highest first:
//!
//!   1. Real environment variables
//!   2. `.env` next to the working directory
//!   3. `.env` at the repo root (walking up)
//!   4. Compiled defaults
//!
//! The retired implementation loaded `.env` from the app directory then the repo
//! root with existing environment variables always winning, and that ordering is
//! preserved -- a systemd unit or a compose file must be able to override a
//! stale `.env` that someone left on the Pi.
//!
//! Deliberately hand-rolled rather than pulling a dotenv crate: it is thirty
//! lines, the format is trivial, and it removes a dependency from the one binary
//! that has to keep working on event day.

use std::path::Path;
use tracing::{debug, warn};

/// Where the app looks for its database and how it listens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// SQLite connection string.
    pub database_url: String,
    /// TCP port. Bound on `0.0.0.0` so LAN clients can reach it.
    pub port: u16,
    /// Whether destructive schema resets are permitted.
    pub allow_schema_reset: bool,
}

/// Runtime mode.
///
/// The retired implementation defaulted this to `test`, and `test` dropped the
/// migration history on boot while the initial migration opened with
/// `DROP TABLE ... CASCADE`. One missing environment variable erased the event's
/// data (REBUILD_SPEC.md 12.10).
///
/// So: the default is safe, and destruction requires typing the word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Normal operation. Migrations apply forward only. **The default.**
    Prod,
    /// Development. Permits schema resets.
    Dev,
}

impl Mode {
    fn parse(raw: &str) -> Result<Self, ConfigError> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "" | "prod" | "production" => Ok(Self::Prod),
            "dev" | "development" => Ok(Self::Dev),
            other => Err(ConfigError::InvalidMode(other.to_string())),
        }
    }

    pub fn allows_schema_reset(self) -> bool {
        matches!(self, Self::Dev)
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("TEALTEAM_ENV must be 'prod' or 'dev', got {0:?}")]
    InvalidMode(String),

    #[error("PORT must be a number between 1 and 65535, got {0:?}")]
    InvalidPort(String),
}

pub const DEFAULT_PORT: u16 = 8080;
pub const DEFAULT_DATABASE_URL: &str = "sqlite://./tealteam.db";

impl Config {
    /// Read configuration from the process environment.
    ///
    /// Call [`load_dotenv_files`] first if `.env` support is wanted.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    /// Testable core of [`Config::from_env`].
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let mode = Mode::parse(&get("TEALTEAM_ENV").unwrap_or_default())?;

        let port = match get("PORT") {
            Some(raw) if !raw.trim().is_empty() => raw
                .trim()
                .parse::<u16>()
                .ok()
                .filter(|p| *p > 0)
                .ok_or(ConfigError::InvalidPort(raw))?,
            _ => DEFAULT_PORT,
        };

        let database_url = get("DATABASE_URL")
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| DEFAULT_DATABASE_URL.to_string());

        Ok(Self {
            database_url,
            port,
            allow_schema_reset: mode.allows_schema_reset(),
        })
    }
}

/// Load `.env` from the working directory, then from each ancestor up to the
/// repo root. Existing environment variables always win, and so does the first
/// file to define a key -- so a crate-local `.env` overrides the repo root's.
pub fn load_dotenv_files() {
    let Ok(cwd) = std::env::current_dir() else {
        return;
    };

    for dir in cwd.ancestors() {
        let candidate = dir.join(".env");
        if candidate.is_file() {
            load_dotenv(&candidate);
        }
    }
}

fn load_dotenv(path: &Path) {
    let Ok(content) = std::fs::read_to_string(path) else {
        warn!("could not read {}", path.display());
        return;
    };

    let mut applied = 0usize;
    for (key, value) in parse_dotenv(&content) {
        // SAFETY: single-threaded startup, before any task is spawned.
        if std::env::var(&key).is_err() {
            unsafe { std::env::set_var(&key, &value) };
            applied += 1;
        }
    }
    debug!("loaded {applied} variable(s) from {}", path.display());
}

/// Parse `KEY=value` lines. Blank lines and `#` comments are skipped; surrounding
/// single or double quotes are stripped; `export ` prefixes are tolerated.
pub fn parse_dotenv(content: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();

    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };

        let key = key.trim();
        if key.is_empty() {
            continue;
        }

        let value = value.trim();
        let value = value
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
            .unwrap_or(value);

        out.push((key.to_string(), value.to_string()));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn lookup(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |key: &str| map.get(key).cloned()
    }

    #[test]
    fn empty_environment_yields_safe_defaults() {
        let config = Config::from_lookup(lookup(&[])).expect("defaults");
        assert_eq!(config.port, DEFAULT_PORT);
        assert_eq!(config.database_url, DEFAULT_DATABASE_URL);
        // The bug that erased an event's data: unset must never mean destructive.
        assert!(!config.allow_schema_reset);
    }

    #[test]
    fn schema_reset_requires_explicit_dev_mode() {
        assert!(
            !Config::from_lookup(lookup(&[("TEALTEAM_ENV", "prod")]))
                .unwrap()
                .allow_schema_reset
        );
        assert!(
            Config::from_lookup(lookup(&[("TEALTEAM_ENV", "dev")]))
                .unwrap()
                .allow_schema_reset
        );
    }

    #[test]
    fn unknown_mode_is_a_startup_error_not_a_silent_default() {
        // "test" was the retired implementation's dangerous default. It is not a
        // valid mode now, and naming it must fail loudly rather than fall back.
        assert_eq!(
            Config::from_lookup(lookup(&[("TEALTEAM_ENV", "test")])),
            Err(ConfigError::InvalidMode("test".into()))
        );
    }

    #[test]
    fn mode_parsing_tolerates_case_and_whitespace() {
        assert_eq!(Mode::parse("  PROD "), Ok(Mode::Prod));
        assert_eq!(Mode::parse("Development"), Ok(Mode::Dev));
    }

    #[test]
    fn blank_values_fall_back_to_defaults() {
        let config = Config::from_lookup(lookup(&[("PORT", "  "), ("DATABASE_URL", "")])).unwrap();
        assert_eq!(config.port, DEFAULT_PORT);
        assert_eq!(config.database_url, DEFAULT_DATABASE_URL);
    }

    #[test]
    fn invalid_port_is_rejected() {
        assert!(matches!(
            Config::from_lookup(lookup(&[("PORT", "0")])),
            Err(ConfigError::InvalidPort(_))
        ));
        assert!(matches!(
            Config::from_lookup(lookup(&[("PORT", "99999")])),
            Err(ConfigError::InvalidPort(_))
        ));
        assert!(matches!(
            Config::from_lookup(lookup(&[("PORT", "eighty")])),
            Err(ConfigError::InvalidPort(_))
        ));
    }

    #[test]
    fn dotenv_parsing_handles_the_shapes_that_appear_in_practice() {
        let parsed = parse_dotenv(
            r#"
            # a comment
            PORT=9090
            DATABASE_URL="sqlite://./x.db"
            SINGLE='quoted'
            export EXPORTED=yes

            MALFORMED
            =novalue
            TRAILING=has spaces
            "#,
        );

        assert_eq!(
            parsed,
            vec![
                ("PORT".into(), "9090".into()),
                ("DATABASE_URL".into(), "sqlite://./x.db".into()),
                ("SINGLE".into(), "quoted".into()),
                ("EXPORTED".into(), "yes".into()),
                ("TRAILING".into(), "has spaces".into()),
            ]
        );
    }

    #[test]
    fn dotenv_keeps_inner_equals_signs() {
        // Connection strings and keys routinely contain '='.
        assert_eq!(
            parse_dotenv("DATABASE_URL=sqlite://x.db?mode=rwc"),
            vec![("DATABASE_URL".into(), "sqlite://x.db?mode=rwc".into())]
        );
    }
}
