//! Startup configuration read from the environment (and a `.env` file for
//! local development), plus the path lookups for the shared `migrations/` and
//! `static/` directories.
//!
//! The variable names match the Go and .NET ports so one `.env` file at the
//! repo root can drive whichever implementation is running.

use std::path::PathBuf;

const DEFAULT_DATABASE_URL: &str =
    "postgres://user:password@127.0.0.1:5432/yourdb?sslmode=disable";

/// Everything [`crate::main`] needs from the environment.
pub struct Config {
    /// `"test"` (the default) or `"prod"`. Test mode resets migration history
    /// on boot, like the Go app's `-env=test`.
    pub app_env: String,
    /// TCP port to bind, from `PORT`. Defaults to `8080`.
    pub port: String,
    /// PostgreSQL connection string. In prod mode `RENDER_DATABASE_URL` wins
    /// over `DATABASE_URL` so a Render deployment picks up its managed
    /// database without extra configuration.
    pub database_url: String,
}

impl Config {
    /// Reads the configuration, loading `.env` files first.
    ///
    /// Fails if `TEALTEAM_ENV` is neither `test` nor `prod`, or if prod mode
    /// is requested without a database URL.
    pub fn from_environment() -> anyhow::Result<Self> {
        // Load .env for local development (app dir first, then repo root).
        load_dotenv(".env");
        load_dotenv("../../.env");

        let app_env = std::env::var("TEALTEAM_ENV")
            .unwrap_or_else(|_| "test".into())
            .trim()
            .to_lowercase();
        if app_env != "test" && app_env != "prod" {
            anyhow::bail!("invalid environment: {app_env}");
        }

        let port = std::env::var("PORT")
            .ok()
            .filter(|p| !p.trim().is_empty())
            .unwrap_or_else(|| "8080".into());

        let database_url = if app_env == "prod" {
            std::env::var("RENDER_DATABASE_URL")
                .or_else(|_| std::env::var("DATABASE_URL"))
                .map_err(|_| {
                    anyhow::anyhow!(
                        "production mode requires RENDER_DATABASE_URL or DATABASE_URL environment variable"
                    )
                })?
        } else {
            std::env::var("DATABASE_URL").unwrap_or_else(|_| DEFAULT_DATABASE_URL.into())
        };

        Ok(Self {
            app_env,
            port,
            database_url,
        })
    }

    /// True in test mode, where migration history is wiped on every boot so a
    /// scratch database always rebuilds from scratch.
    pub fn is_test(&self) -> bool {
        self.app_env == "test"
    }
}

/// Minimal `.env` loader mirroring godotenv: `KEY=value` per line, `#`
/// comments and blank lines skipped, surrounding quotes stripped. A missing
/// file is not an error, and variables already set in the real environment
/// always win.
fn load_dotenv(path: &str) {
    let Ok(content) = std::fs::read_to_string(path) else {
        return;
    };
    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some(idx) = line.find('=') else { continue };
        if idx == 0 {
            continue;
        }
        let key = line[..idx].trim();
        let value = line[idx + 1..].trim().trim_matches('"');
        if std::env::var(key).is_err() {
            std::env::set_var(key, value);
        }
    }
}

/// Directory holding the shared `*.sql` migrations, or the bare name as a
/// last resort so the error message names something useful.
pub fn migrations_dir() -> PathBuf {
    find_upwards("migrations").unwrap_or_else(|| "migrations".into())
}

/// Directory served at `/static` (vendored Unpoly, site CSS/JS).
pub fn static_dir() -> PathBuf {
    find_upwards("static").unwrap_or_else(|| "static".into())
}

/// Locate a directory named `name` by walking up from both the executable's
/// location and the current working directory. This lets the binary find the
/// shared `migrations/` and `static/` dirs whether it's launched via
/// `cargo run` (cwd = crate root) or directly from `target/release/`, and finds
/// a copy sitting next to the binary in a publish layout.
pub fn find_upwards(name: &str) -> Option<PathBuf> {
    let mut starts = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            starts.push(dir.to_path_buf());
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        starts.push(cwd);
    }
    for start in starts {
        for ancestor in start.ancestors() {
            let candidate = ancestor.join(name);
            if candidate.is_dir() {
                return Some(candidate);
            }
        }
    }
    None
}
