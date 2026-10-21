//! The dev environment: `cargo run -- dev` (DEV1).
//!
//! A server on a throwaway database in `./dev-data/`, full of fake but
//! realistic data, so a person can click around as any role. Before it
//! serves, [`checks`] drive the real router as every seeded user and assert
//! the access and data-sharing rules the code and docs state.
//!
//! What keeps it from touching anything real:
//!
//! - It reads no `.env` and ignores `DATABASE_URL`: the data directory is
//!   `./dev-data/`, always.
//! - It refuses to start when `TEALTEAM_ENV=prod` is in the process
//!   environment, or when the data directory is, or is inside, `/srv/tealteam`.
//! - Upstream is off: no sync, no tether watch, no backups, and no client that
//!   could make a network request ([`Upstream::disabled`]).
//! - It listens on 127.0.0.1 only. Every account's password is `dev`.
//!
//! `--fresh` wipes the data directory first, but only one this command made
//! (it holds a marker file), so a mistyped working directory cannot delete
//! somebody's folder.

pub mod checks;
mod seed;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use axum::Router;
use axum::body::Body;
use axum::http::{HeaderValue, Request, header};
use axum::middleware::Next;
use axum::response::Response;
use tracing::{info, warn};
use tt_core::season;
use tt_core::user::Roles;
use tt_repo_sqlite::SqliteRepo;

use crate::startup::{self, AppState};
use crate::upstream::Upstream;

/// The data directory, under the working directory.
pub const DATA_DIR: &str = "dev-data";
/// The production data directory, which dev mode must never be pointed at.
const PRODUCTION_DIR: &str = "/srv/tealteam";
/// Written into a data directory this command made.
const MARKER: &str = ".tealteam-dev";
/// Written once seeding has finished, so a half-seeded directory is redone.
const SEEDED: &str = ".seeded";
const DEFAULT_PORT: u16 = 8080;

/// What `dev` was asked to do.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Options {
    /// Wipe the dev data first.
    pub fresh: bool,
    /// Seed and check, print the report, exit; do not serve.
    pub check: bool,
    /// Where to start looking for a free port.
    pub port: Option<u16>,
}

impl Options {
    pub fn parse(mut args: impl Iterator<Item = String>) -> Result<Self, String> {
        let mut options = Self::default();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--fresh" => options.fresh = true,
                "--check" => options.check = true,
                "--port" => {
                    let raw = args.next().ok_or("--port needs a number")?;
                    options.port = Some(
                        raw.parse::<u16>()
                            .ok()
                            .filter(|p| *p > 0)
                            .ok_or(format!("--port must be 1 to 65535, got {raw:?}"))?,
                    );
                }
                other => return Err(format!("unexpected argument {other:?} for dev")),
            }
        }
        Ok(options)
    }
}

/// One seeded account.
pub struct DevUser {
    /// What the checks call it.
    pub key: &'static str,
    pub name: &'static str,
    pub email: &'static str,
    pub team: Option<i32>,
    pub roles: Roles,
    /// What to tell a person about this account.
    pub label: &'static str,
}

const fn roles(admin: bool, lead: bool, coach: bool) -> Roles {
    Roles {
        is_admin: admin,
        is_lead_scout: lead,
        is_coach: coach,
    }
}

pub const OUR_TEAM: i32 = 10101;
pub const OTHER_TEAM: i32 = 4455;
pub const PASSWORD: &str = "dev";

/// Every account, all with [`PASSWORD`]. The order is the order of ids.
pub const USERS: &[DevUser] = &[
    DevUser {
        key: "admin",
        name: "Ada Admin",
        email: "admin@dev.test",
        team: Some(OUR_TEAM),
        roles: roles(true, false, false),
        label: "admin",
    },
    DevUser {
        key: "lead",
        name: "Lena Lead",
        email: "lead@dev.test",
        team: Some(OUR_TEAM),
        roles: roles(false, true, false),
        label: "lead scout",
    },
    DevUser {
        key: "coach",
        name: "Carl Coach",
        email: "coach@dev.test",
        team: Some(OUR_TEAM),
        roles: roles(false, false, true),
        label: "drive coach",
    },
    DevUser {
        key: "scout1",
        name: "Sam Scout",
        email: "scout1@dev.test",
        team: Some(OUR_TEAM),
        roles: Roles::SCOUT,
        label: "scout (owns the registered tablet)",
    },
    DevUser {
        key: "scout2",
        name: "Sofia Scout",
        email: "scout2@dev.test",
        team: Some(OUR_TEAM),
        roles: Roles::SCOUT,
        label: "scout (has a refused entry waiting)",
    },
    DevUser {
        key: "scout3",
        name: "Sid Scout",
        email: "scout3@dev.test",
        team: Some(OUR_TEAM),
        roles: Roles::SCOUT,
        label: "scout (has a declined observation)",
    },
    DevUser {
        key: "scout4",
        name: "Sana Scout",
        email: "scout4@dev.test",
        team: Some(OUR_TEAM),
        roles: Roles::SCOUT,
        label: "scout (has an observation put back to waiting)",
    },
    DevUser {
        key: "signedout",
        name: "Sol Signedout",
        email: "signedout@dev.test",
        team: Some(OUR_TEAM),
        roles: Roles::SCOUT,
        label: "scout signed out everywhere (offline tokens before now are dead)",
    },
    DevUser {
        key: "lead2",
        name: "Lou Otherlead",
        email: "lead2@dev.test",
        team: Some(OTHER_TEAM),
        roles: roles(false, true, false),
        label: "lead scout of the other team",
    },
    DevUser {
        key: "scout5",
        name: "Oz Otherscout",
        email: "scout5@dev.test",
        team: Some(OTHER_TEAM),
        roles: Roles::SCOUT,
        label: "scout of the other team (has a refused entry waiting)",
    },
    DevUser {
        key: "scout6",
        name: "Ola Otherscout",
        email: "scout6@dev.test",
        team: Some(OTHER_TEAM),
        roles: Roles::SCOUT,
        label: "scout of the other team",
    },
    DevUser {
        key: "noteam",
        name: "Nat Noteam",
        email: "noteam@dev.test",
        team: None,
        roles: Roles::SCOUT,
        label: "scout with no team",
    },
];

/// `TEALTEAM_ENV` as dev mode needs it: a production process never runs this.
pub fn refuse_production(env: Option<&str>) -> Result<(), String> {
    match env.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
        Some("prod" | "production") => Err(
            "dev refuses to run with TEALTEAM_ENV=prod in the environment: it makes accounts \
             with a known password. Unset it, or run the server with `serve`."
                .into(),
        ),
        _ => Ok(()),
    }
}

/// Whether `dir` is, or is inside, the production data directory, following
/// symlinks as far as the path exists.
pub fn refuse_production_dir(dir: &Path) -> Result<(), String> {
    let mut resolved = dir.to_path_buf();
    // The directory may not exist yet; resolve its nearest ancestor that does.
    let mut tail = PathBuf::new();
    while !resolved.exists() {
        match (resolved.file_name().map(PathBuf::from), resolved.parent()) {
            (Some(name), Some(parent)) => {
                tail = name.join(&tail);
                resolved = parent.to_path_buf();
            }
            _ => break,
        }
    }
    let resolved = resolved.canonicalize().unwrap_or(resolved).join(tail);
    if resolved.starts_with(PRODUCTION_DIR) {
        return Err(format!(
            "dev refuses to use {}: that is the production data directory",
            resolved.display()
        ));
    }
    Ok(())
}

/// Wipe `dir`, if it is one this command made (or is empty).
fn wipe(dir: &Path) -> anyhow::Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    let ours = dir.join(MARKER).exists();
    let empty = std::fs::read_dir(dir)?.next().is_none();
    if !ours && !empty {
        anyhow::bail!(
            "{} exists but was not made by `dev` (no {MARKER} in it); not deleting it",
            dir.display()
        );
    }
    std::fs::remove_dir_all(dir).with_context(|| format!("removing {}", dir.display()))
}

/// The state over `dir`'s database, seeding it first when it has not been.
pub async fn open(dir: &Path) -> anyhow::Result<AppState> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    std::fs::write(dir.join(MARKER), "made by `tt-web dev`; safe to delete\n")?;

    let db = dir.join("dev.db");
    if !dir.join(SEEDED).exists() {
        // A directory whose seeding stopped halfway is started over.
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", db.display()));
        }
    }
    let url = format!("sqlite://{}?mode=rwc", db.display());
    let repo = SqliteRepo::connect(&url).with_context(|| format!("opening {url}"))?;
    tt_repo_sqlite::migrate::apply(repo.pool())
        .await
        .context("applying migrations")?;
    let state = AppState {
        repo: Arc::new(repo),
        season: Arc::new(season::current_season().context("embedded season schema")?),
        upstream: Arc::new(Upstream::disabled()),
        snapshots: Default::default(),
        tokens: Default::default(),
    };
    if !dir.join(SEEDED).exists() {
        seed::seed(&state).await.context("seeding the dev data")?;
        std::fs::write(dir.join(SEEDED), "")?;
        // A sync serves changes only once they are a moment old (S2); wait, so
        // the first pull and the checks of it are not empty.
        tokio::time::sleep(
            crate::sync::LAG.to_std().unwrap_or_default() + std::time::Duration::from_millis(200),
        )
        .await;
        info!("seeded the dev data");
    }
    Ok(state)
}

/// What a person sees at the top of the terminal.
fn print_users() {
    println!("\nDEV DATA: every password is {PASSWORD:?}\n");
    println!("{:<20} {:<22} {:<8} role", "name", "email", "team");
    for u in USERS {
        let team = u.team.map_or("-".to_string(), |t| t.to_string());
        println!("{:<20} {:<22} {:<8} {}", u.name, u.email, team, u.label);
    }
    println!(
        "\nEvent {} (our team {OUR_TEAM}, the other team {OTHER_TEAM}); the first {} matches are played.\n",
        seed::EVENT,
        seed::PLAYED
    );
}

/// `tt-web dev`.
pub async fn run(options: Options) -> anyhow::Result<()> {
    startup::init_tracing();
    refuse_production(std::env::var("TEALTEAM_ENV").ok().as_deref()).map_err(anyhow::Error::msg)?;
    let dir = std::env::current_dir()?.join(DATA_DIR);
    refuse_production_dir(&dir).map_err(anyhow::Error::msg)?;
    if options.fresh {
        wipe(&dir)?;
    }

    let state = open(&dir).await?;
    print_users();

    let report = checks::run(&state).await;
    if options.check {
        report.print_all();
        println!("\n{}", report.summary());
        anyhow::ensure!(report.failed() == 0, "{} check(s) failed", report.failed());
        return Ok(());
    }
    report.print_failures();
    println!("{}\n", report.summary());
    if report.failed() > 0 {
        warn!("serving anyway; `cargo run -- dev --check` lists every check");
    }

    let (listener, port) = bind(options.port.unwrap_or(DEFAULT_PORT)).await?;
    println!("DEV DATA: serving http://localhost:{port} (Ctrl-C to stop)\n");
    info!("DEV DATA: listening on http://127.0.0.1:{port}");
    axum::serve(listener, banner(startup::router(state)))
        .await
        .context("server error")
}

/// 127.0.0.1 on `start`, or the next free port after it.
async fn bind(start: u16) -> anyhow::Result<(tokio::net::TcpListener, u16)> {
    for port in start..=start.saturating_add(100) {
        match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
            Ok(listener) => return Ok((listener, port)),
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => continue,
            Err(e) => return Err(e).with_context(|| format!("binding port {port}")),
        }
    }
    anyhow::bail!("no free port from {start} to {}", start.saturating_add(100))
}

/// Say DEV DATA at the top of every page, and in a header on every response.
fn banner(router: Router) -> Router {
    router.layer(axum::middleware::from_fn(stamp))
}

const STAMP: &str = r#"<div style="background:#b45309;color:#fff;text-align:center;padding:4px;font:600 13px sans-serif;position:sticky;top:0;z-index:9999">DEV DATA &mdash; fake accounts, throwaway database</div>"#;

async fn stamp(request: Request<Body>, next: Next) -> Response {
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert("x-dev-data", HeaderValue::from_static("true"));
    let is_html = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("text/html"));
    if !is_html {
        return response;
    }
    let (mut parts, body) = response.into_parts();
    let Ok(bytes) = axum::body::to_bytes(body, 16 * 1024 * 1024).await else {
        return Response::from_parts(parts, Body::empty());
    };
    let html = String::from_utf8_lossy(&bytes);
    let stamped = match html
        .find("<body")
        .and_then(|at| html[at..].find('>').map(|end| at + end + 1))
    {
        Some(at) => format!("{}{STAMP}{}", &html[..at], &html[at..]),
        None => html.into_owned(),
    };
    parts.headers.remove(header::CONTENT_LENGTH);
    Response::from_parts(parts, Body::from(stamped))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Options, String> {
        Options::parse(args.iter().map(|a| a.to_string()))
    }

    #[test]
    fn the_flags_parse_and_a_typo_is_an_error() {
        assert_eq!(parse(&[]), Ok(Options::default()));
        assert_eq!(
            parse(&["--fresh", "--check", "--port", "9100"]),
            Ok(Options {
                fresh: true,
                check: true,
                port: Some(9100)
            })
        );
        assert!(parse(&["--port"]).is_err());
        assert!(parse(&["--port", "0"]).is_err());
        assert!(parse(&["--port", "eighty"]).is_err());
        assert!(parse(&["--fresh", "now"]).unwrap_err().contains("now"));
    }

    #[test]
    fn a_production_environment_is_refused() {
        assert!(refuse_production(None).is_ok());
        assert!(refuse_production(Some("dev")).is_ok());
        assert!(refuse_production(Some("")).is_ok());
        for prod in ["prod", " PROD ", "production"] {
            assert!(refuse_production(Some(prod)).is_err(), "{prod}");
        }
    }

    #[test]
    fn the_production_data_directory_is_refused_even_if_it_does_not_exist_yet() {
        assert!(refuse_production_dir(Path::new("/srv/tealteam")).is_err());
        assert!(refuse_production_dir(Path::new("/srv/tealteam/dev-data")).is_err());
        assert!(refuse_production_dir(Path::new("/srv/tealteam-other/dev-data")).is_ok());
        assert!(refuse_production_dir(&std::env::temp_dir().join("x")).is_ok());
    }

    #[test]
    fn wipe_only_removes_a_directory_dev_made() {
        let dir = std::env::temp_dir().join(format!("tt-dev-wipe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("precious.txt"), "x").unwrap();
        assert!(wipe(&dir).is_err(), "a folder dev did not make");
        assert!(dir.join("precious.txt").exists());
        std::fs::write(dir.join(MARKER), "").unwrap();
        wipe(&dir).unwrap();
        assert!(!dir.exists());
        wipe(&dir).unwrap();
    }
}
