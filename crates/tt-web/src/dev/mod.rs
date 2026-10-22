//! `tt-web dev` (F13): a local server on a throwaway database full of fake
//! data, and the access-control checks run against it. See
//! `docs/dev/DEV_ENVIRONMENT.md`.
//!
//! Safety first: dev mode has its own data folder (`./dev-data/`), never
//! reads `DATABASE_URL` or any `.env`, refuses to run when `TEALTEAM_ENV` is
//! `prod` or when the folder is (or links to) the production data, and makes
//! no network request: the upstream is `Upstream::off()` with no FTC key, the
//! backups and the tether watch are not started, and the scouting assistant
//! is not read from the environment, so `/ask` says it is not set up.

mod checks;
mod seed;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use tracing::info;
use tt_core::season;
use tt_repo_sqlite::SqliteRepo;

use crate::startup::{AppState, router};
use crate::upstream::Upstream;

/// Where dev data lives, under the folder the command runs in.
pub const DIR: &str = "dev-data";
const DB_FILE: &str = "tealteam-dev.db";
/// The production data folder, which dev mode must never touch.
const PRODUCTION: &str = "/srv/tealteam";
const DEFAULT_PORT: u16 = 8080;
/// How many ports above the default are tried when it is busy.
const PORT_TRIES: u16 = 20;

/// The flags of `tt-web dev`.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Options {
    /// Wipe the dev data first.
    pub fresh: bool,
    /// Seed, check, report, and exit: do not serve.
    pub check: bool,
    /// Exactly this port. Without it, 8080 or the next free one.
    pub port: Option<u16>,
}

impl Options {
    pub fn parse(args: impl Iterator<Item = String>) -> Result<Self, String> {
        let mut options = Options::default();
        let mut args = args;
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--fresh" => options.fresh = true,
                "--check" => options.check = true,
                "--port" => {
                    let raw = args.next().ok_or("--port needs a number")?;
                    options.port = Some(port(&raw)?);
                }
                other => match other.strip_prefix("--port=") {
                    Some(raw) => options.port = Some(port(raw)?),
                    None => return Err(format!("unknown dev option {other:?}")),
                },
            }
        }
        Ok(options)
    }
}

fn port(raw: &str) -> Result<u16, String> {
    raw.parse::<u16>()
        .ok()
        .filter(|p| *p > 0)
        .ok_or_else(|| format!("{raw:?} is not a port number"))
}

/// Refuse a folder that is, is inside, or links into the production data.
fn guard(dir: &Path) -> anyhow::Result<()> {
    let production = Path::new(PRODUCTION);
    for candidate in [Some(dir.to_path_buf()), std::fs::canonicalize(dir).ok()]
        .into_iter()
        .flatten()
    {
        anyhow::ensure!(
            !candidate.starts_with(production),
            "refusing to use {} for dev data: it is the production data folder",
            candidate.display()
        );
    }
    Ok(())
}

/// Refuse to run against a production environment.
fn guard_environment(get: impl Fn(&str) -> Option<String>) -> anyhow::Result<()> {
    let env = get("TEALTEAM_ENV").unwrap_or_default();
    anyhow::ensure!(
        !env.trim().eq_ignore_ascii_case("prod"),
        "refusing to run dev mode: TEALTEAM_ENV is {:?}. Dev mode makes fake accounts with \
         the password {:?}; unset it to run here.",
        env.trim(),
        seed::PASSWORD
    );
    Ok(())
}

fn url(path: &Path) -> String {
    format!("sqlite://{}?mode=rwc", path.display())
}

fn files_of(db: &Path) -> Vec<PathBuf> {
    ["", "-wal", "-shm"]
        .iter()
        .map(|suffix| PathBuf::from(format!("{}{suffix}", db.display())))
        .collect()
}

/// An `AppState` on `database`, migrated, with the upstream off.
async fn open(database: &Path) -> anyhow::Result<AppState> {
    let repo = SqliteRepo::connect(&url(database))
        .with_context(|| format!("opening {}", database.display()))?;
    tt_repo_sqlite::migrate::apply(repo.pool())
        .await
        .context("applying migrations")?;
    Ok(AppState {
        repo: Arc::new(repo),
        season: Arc::new(season::current_season().context("embedded season schema")?),
        upstream: Arc::new(Upstream::off()),
        snapshots: Default::default(),
        tokens: Default::default(),
    })
}

/// Make the dev database in `dir` if it is missing, and return its path.
///
/// Seeded into a side file and renamed into place only when whole, so a
/// seed that stops half way is redone from nothing, not served.
pub async fn prepare(dir: &Path, fresh: bool) -> anyhow::Result<PathBuf> {
    guard(dir)?;
    std::fs::create_dir_all(dir).with_context(|| format!("making {}", dir.display()))?;
    guard(dir)?;
    let db = dir.join(DB_FILE);
    let side = dir.join(format!("{DB_FILE}.seeding"));
    if fresh {
        for file in files_of(&db).into_iter().chain(files_of(&side)) {
            let _ = std::fs::remove_file(file);
        }
    }
    if db.exists() {
        return Ok(db);
    }
    for file in files_of(&side) {
        let _ = std::fs::remove_file(file);
    }
    let state = open(&side).await?;
    seed::seed(&state).await.context("seeding the dev data")?;
    state.repo.pool().close().await;
    for (from, to) in files_of(&side).into_iter().zip(files_of(&db)) {
        if from.exists() {
            std::fs::rename(&from, &to)?;
        }
    }
    info!("seeded {}", db.display());
    Ok(db)
}

/// `tt-web dev`.
pub async fn run(options: Options) -> anyhow::Result<()> {
    // Deliberately before anything reads a `.env`, and never reading one.
    guard_environment(|key| std::env::var(key).ok())?;
    crate::startup::init_tracing();

    let dir = std::env::current_dir()?.join(DIR);
    let db = prepare(&dir, options.fresh).await?;
    let state = open(&db).await?;
    println!(
        "DEV DATA in {} (fake accounts, password {:?})",
        dir.display(),
        seed::PASSWORD
    );

    let report = checks::run(&state, &dir).await;
    print!("{}", report.render(options.check));
    if options.check {
        anyhow::ensure!(
            report.failed() == 0,
            "{} dev check(s) failed",
            report.failed()
        );
        return Ok(());
    }
    if report.failed() > 0 {
        eprintln!(
            "WARNING: {} of {} dev checks failed (listed above). Serving anyway.",
            report.failed(),
            report.total()
        );
    }

    let listener = bind(options.port).await?;
    let addr = listener.local_addr()?;
    print!("{}", users_table());
    println!(
        "\nDEV DATA - not production. http://localhost:{}/ (this machine only)",
        addr.port()
    );
    println!(
        "Sign in with an email above and the password {:?}. Ctrl-C stops it.",
        seed::PASSWORD
    );
    info!("dev server on http://{addr}");

    let app = router(state).layer(axum::middleware::map_response(
        |mut response: axum::response::Response| async move {
            response.headers_mut().insert(
                "x-dev-data",
                axum::http::HeaderValue::from_static("DEV DATA - not production"),
            );
            response
        },
    ));
    axum::serve(listener, app).await.context("server error")
}

async fn bind(port: Option<u16>) -> anyhow::Result<tokio::net::TcpListener> {
    let local = |p: u16| tokio::net::TcpListener::bind(("127.0.0.1", p));
    if let Some(p) = port {
        return local(p)
            .await
            .with_context(|| format!("port {p} is not free"));
    }
    for p in DEFAULT_PORT..DEFAULT_PORT + PORT_TRIES {
        if let Ok(listener) = local(p).await {
            return Ok(listener);
        }
    }
    anyhow::bail!(
        "no free port from {DEFAULT_PORT} to {}: pass --port N",
        DEFAULT_PORT + PORT_TRIES - 1
    )
}

/// The accounts: name, email, role, team.
pub fn users_table() -> String {
    let mut out = format!(
        "\n{:<18} {:<22} {:<30} {}\n",
        "NAME", "EMAIL", "ROLE", "TEAM"
    );
    for u in seed::USERS {
        out.push_str(&format!(
            "{:<18} {:<22} {:<30} {}\n",
            u.name,
            u.email,
            u.role,
            u.team.map_or("-".to_string(), |t| t.to_string())
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Options, String> {
        Options::parse(args.iter().map(|a| a.to_string()))
    }

    #[test]
    fn the_flags_are_read() {
        assert_eq!(parse(&[]), Ok(Options::default()));
        assert_eq!(
            parse(&["--fresh", "--check", "--port", "9001"]),
            Ok(Options {
                fresh: true,
                check: true,
                port: Some(9001)
            })
        );
        assert_eq!(parse(&["--port=9002"]).unwrap().port, Some(9002));
        assert!(parse(&["--port"]).is_err());
        assert!(parse(&["--port", "x"]).is_err());
        assert!(parse(&["--prod"]).unwrap_err().contains("--prod"));
    }

    #[test]
    fn a_production_environment_is_refused() {
        let env = |value: &'static str| move |k: &str| (k == "TEALTEAM_ENV").then(|| value.into());
        assert!(guard_environment(env("prod")).is_err());
        assert!(guard_environment(env(" PROD ")).is_err());
        assert!(guard_environment(env("test")).is_ok());
        assert!(guard_environment(|_| None).is_ok());
    }

    #[test]
    fn the_production_data_folder_is_refused() {
        assert!(guard(Path::new("/srv/tealteam")).is_err());
        assert!(guard(Path::new("/srv/tealteam/data/dev-data")).is_err());
        assert!(guard(Path::new("/srv/tealteam-other")).is_ok());
        assert!(guard(Path::new("/tmp/dev-data")).is_ok());
    }
}
