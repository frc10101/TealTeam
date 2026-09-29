//! Startup sequence (F5).
//!
//! The ordering here is a requirement, not a preference. At an event the server
//! is a Raspberry Pi on a folding table and the people restarting it are
//! students in the middle of a competition. A server that boots half-working and
//! says so beats a server that refuses to boot and explains why in a log nobody
//! is reading.
//!
//! So:
//!
//!   1. Load `.env` files, then read config from the environment.
//!   2. Initialise tracing.
//!   3. **Validate config. This is the only fatal step** -- a bad `TEALTEAM_ENV`,
//!      `PORT`, or `FIRST_SYNC_ON_BOOT` is a typo someone can fix, and guessing
//!      at it is how the retired implementation ended up erasing databases.
//!   4. Open the database *lazily*. Does not touch the disk.
//!   5. Probe it. **Failure is logged, not fatal**; the app serves degraded.
//!   6. Start upstream sync in the background, if storage is up. **Never
//!      awaited**: a venue with no internet must not delay the first page.
//!   7. Bind and serve. Failure to bind is fatal -- there is nothing to serve on.

use anyhow::Context;
use axum::Router;
use axum::extract::State;
use axum::response::{Html, IntoResponse};
use axum::routing::get;
use std::sync::Arc;
use tracing::{info, warn};
use tt_core::season::{self, SeasonSchema};
use tt_repo::{Health, Repo};
use tt_repo_sqlite::SqliteRepo;
use tt_templates::{HealthPage, Page};

use crate::config::{self, Config};
use crate::upstream::{self, Upstream};

/// Everything a handler needs. Cheap to clone; the contents are shared.
#[derive(Clone)]
pub struct AppState {
    pub repo: Arc<SqliteRepo>,
    /// Parsed once at startup rather than per request. Immutable for the life of
    /// the process, which is what makes the schema version a deployment fact.
    pub season: Arc<SeasonSchema>,
    /// Shared with the background sync, so a manual sync and the loop agree on
    /// the uplink's state and never run two event syncs at once.
    pub upstream: Arc<Upstream>,
}

/// Steps 1-3, shared by every command.
fn prepare() -> anyhow::Result<Config> {
    // 1-2. Config comes before tracing is configured, so early failures print to
    // stderr rather than vanishing. Tracing then picks up RUST_LOG from the same
    // .env files.
    config::load_dotenv_files();
    init_tracing();

    // 3. The only fatal validation step.
    Config::from_env().context("invalid configuration")
}

pub async fn run() -> anyhow::Result<()> {
    let config = prepare()?;
    info!(
        port = config.port,
        database = %config.database_url,
        schema_reset_allowed = config.allow_schema_reset,
        "starting tealteam"
    );
    if config.allow_schema_reset {
        warn!("running in dev mode: destructive schema resets are PERMITTED");
    }

    // 3b. The season schema is embedded, so a bad one is a build failure, not a
    // boot failure. Parsing here turns it into a value the handlers can share.
    let season = season::current_season().context("embedded season schema is invalid")?;
    info!(
        season = season.season,
        name = %season.name,
        version = season.version,
        fields = season.fields().count(),
        "loaded season schema"
    );

    // 4. Lazy: succeeds even if the storage path is unwritable.
    let repo = SqliteRepo::connect(&config.database_url)
        .with_context(|| format!("opening database {}", config.database_url))?;

    // 5. Degrade, do not abort.
    let storage_ready = match repo.health().await {
        Health::Ready => {
            // Forward-only. Nothing in this path can drop anything, regardless of
            // configuration -- see tt_repo_sqlite::migrate.
            tt_repo_sqlite::migrate::apply(repo.pool())
                .await
                .context("applying migrations")?;

            let expired = repo
                .purge_expired_sessions(chrono::Utc::now())
                .await
                .unwrap_or(0);
            if expired > 0 {
                info!("purged {expired} expired session(s)");
            }
            true
        }
        Health::Down => {
            warn!(
                "database unavailable at {} -- serving degraded pages. \
                 Storage-backed features will not work until this is fixed.",
                config.database_url
            );
            false
        }
    };

    let state = AppState {
        repo: Arc::new(repo),
        season: Arc::new(season),
        upstream: Arc::new(Upstream::from_env()),
    };

    // 6. Only with storage up. When it is down, migrations did not run and a
    // sync would have no tables to write to.
    if storage_ready {
        upstream::spawn(
            state.repo.clone(),
            state.upstream.clone(),
            config.first_sync_on_boot,
        );
    } else {
        warn!("upstream sync not started: storage is down");
    }

    // 7. Bind on 0.0.0.0 so LAN clients reach it. Nothing else is reachable from
    // a scout's phone.
    let addr = format!("0.0.0.0:{}", config.port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| format!("binding {addr}"))?;
    info!("listening on http://{addr}");

    axum::serve(listener, router(state))
        .await
        .context("server error")?;

    Ok(())
}

/// `tt-web bulk-load` (I8).
///
/// Unlike serving, a dead database is fatal here: there is nothing to degrade
/// to, and the point of the command is to find problems before leaving the shop.
pub async fn bulk_load() -> anyhow::Result<()> {
    let config = prepare()?;

    let repo = SqliteRepo::connect(&config.database_url)
        .with_context(|| format!("opening database {}", config.database_url))?;
    if !repo.health().await.is_ready() {
        anyhow::bail!("database unavailable at {}", config.database_url);
    }
    tt_repo_sqlite::migrate::apply(repo.pool())
        .await
        .context("applying migrations")?;

    upstream::bulk_load(&repo, &Upstream::from_env(), &mut std::io::stdout()).await
}

pub fn router(state: AppState) -> Router {
    use crate::handlers;
    use axum::routing::post;

    Router::new()
        // Pages
        .route("/", get(handlers::home))
        .route("/sign-in", get(handlers::sign_in_page))
        .route("/sign-up", get(handlers::sign_up_page))
        .route("/account", get(handlers::account))
        .route("/submission", get(handlers::submission))
        .route("/lead-scout", get(handlers::lead_scout))
        .route("/lead-scout/assignments", get(handlers::assignments))
        .route("/drive-coach", get(handlers::drive_coach))
        // Forms
        .route("/api/auth/login", post(handlers::login))
        .route("/api/auth/signup", post(handlers::signup))
        .route("/api/auth/logout", post(handlers::logout))
        .route(
            "/api/account/change-password",
            post(handlers::change_password),
        )
        .route("/api/submission", post(handlers::submit_observation))
        .route("/api/device/heartbeat", post(handlers::device_heartbeat))
        .route("/api/frc/sync", post(handlers::manual_sync))
        // Operational
        .route("/health", get(health_json))
        .route("/status", get(health_page))
        .nest_service("/static", tower_http::services::ServeDir::new(static_dir()))
        // Every error a browser would otherwise get as a blank or plain-text
        // screen becomes a page with the nav on it (U10).
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::errors::html_errors,
        ))
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .with_state(state)
}

/// Locate `static/`, looking next to the executable first and then up from the
/// working directory.
///
/// Carried over from the retired implementation because it solved a real
/// deployment problem: the binary must find its assets whether it was launched
/// by `cargo run` (cwd = repo root) or straight out of `target/release/` on the
/// Pi (REBUILD_SPEC.md 10).
fn static_dir() -> std::path::PathBuf {
    let mut roots = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        roots.push(dir.to_path_buf());
    }
    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd);
    }

    for root in roots {
        for ancestor in root.ancestors() {
            let candidate = ancestor.join("crates/tt-web/static");
            if candidate.is_dir() {
                return candidate;
            }
            let bare = ancestor.join("static");
            if bare.is_dir() {
                return bare;
            }
        }
    }
    "static".into()
}

/// Human-readable status. Renders whether or not storage is reachable -- that is
/// the point of it.
async fn health_page(State(state): State<AppState>) -> impl IntoResponse {
    let storage_ready = state.repo.health().await.is_ready();
    let schema_version = if storage_ready {
        state.repo.schema_version().await.unwrap_or(None)
    } else {
        None
    };

    let page = HealthPage {
        storage_ready,
        schema_version,
    };
    match page.render_html() {
        Ok(html) => Html(html).into_response(),
        Err(e) => {
            // Templates are checked at build time, so this is close to
            // unreachable -- but rendering is still fallible (formatting, writes).
            tracing::error!("rendering health page: {e}");
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "render error",
            )
                .into_response()
        }
    }
}

/// Machine-readable liveness, for the Pi's autostart supervision.
///
/// Returns 200 even when storage is down: the *process* is alive and serving,
/// which is what a supervisor needs to know. Storage state is in the body.
async fn health_json(State(state): State<AppState>) -> impl IntoResponse {
    let ready = state.repo.health().await.is_ready();
    let body = if ready {
        r#"{"storage":"ready"}"#
    } else {
        r#"{"storage":"down"}"#
    };
    (
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        body,
    )
}

fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    // sqlx logs every statement at INFO, which buries everything else.
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,sqlx=warn,tower_http=info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    fn state_for(url: &str) -> AppState {
        AppState {
            repo: Arc::new(SqliteRepo::connect(url).expect("lazy connect")),
            season: Arc::new(season::current_season().expect("embedded schema")),
            upstream: Arc::new(Upstream::disabled()),
        }
    }

    /// A state backed by a migrated in-memory database, for tests that write.
    pub(super) async fn migrated_state() -> AppState {
        let repo = SqliteRepo::connect("sqlite::memory:").expect("connect");
        tt_repo_sqlite::migrate::apply(repo.pool())
            .await
            .expect("migrate");
        AppState {
            repo: Arc::new(repo),
            season: Arc::new(season::current_season().expect("embedded schema")),
            upstream: Arc::new(Upstream::disabled()),
        }
    }

    async fn body_string(response: axum::response::Response) -> String {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read body");
        String::from_utf8(bytes.to_vec()).expect("utf8")
    }

    #[tokio::test]
    async fn serves_the_status_page_when_storage_is_healthy() {
        let response = router(state_for("sqlite::memory:"))
            .oneshot(
                Request::builder()
                    .uri("/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("request");

        assert_eq!(response.status(), StatusCode::OK);
        assert!(body_string(response).await.contains("ready"));
    }

    #[tokio::test]
    async fn still_serves_pages_when_storage_is_unreachable() {
        // The requirement this whole startup design exists for: a dead database
        // must not take the HTTP surface down with it.
        let response = router(state_for("sqlite:///nonexistent-dir/tealteam.db"))
            .oneshot(
                Request::builder()
                    .uri("/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("request");

        assert_eq!(response.status(), StatusCode::OK);
        assert!(body_string(response).await.contains("unavailable"));
    }

    #[tokio::test]
    async fn health_endpoint_reports_storage_down_but_still_answers_200() {
        // A supervisor asks "is the process alive"; storage state goes in the body.
        let response = router(state_for("sqlite:///nonexistent-dir/tealteam.db"))
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("request");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(body_string(response).await, r#"{"storage":"down"}"#);
    }

    #[tokio::test]
    async fn health_endpoint_reports_ready_storage() {
        let response = router(state_for("sqlite::memory:"))
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("request");

        assert_eq!(body_string(response).await, r#"{"storage":"ready"}"#);
    }
}

#[cfg(test)]
mod flow_tests {
    //! End-to-end tests through the real router, against a migrated in-memory
    //! database. These are the tests that would have caught the retired
    //! implementation's unguarded database viewer.

    use super::tests::migrated_state;
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode, header};
    use axum::response::Response;
    use tower::ServiceExt;

    async fn post(state: &AppState, uri: &str, body: &str, cookie: Option<&str>) -> Response {
        let mut req = Request::builder()
            .method("POST")
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
        if let Some(c) = cookie {
            req = req.header(header::COOKIE, c);
        }
        router(state.clone())
            .oneshot(req.body(Body::from(body.to_string())).unwrap())
            .await
            .expect("request")
    }

    async fn get(state: &AppState, uri: &str, cookie: Option<&str>) -> Response {
        let mut req = Request::builder().method("GET").uri(uri);
        if let Some(c) = cookie {
            req = req.header(header::COOKIE, c);
        }
        router(state.clone())
            .oneshot(req.body(Body::empty()).unwrap())
            .await
            .expect("request")
    }

    async fn text(response: Response) -> String {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        String::from_utf8_lossy(&bytes).into_owned()
    }

    /// Pull the session cookie out of a Set-Cookie header.
    fn session_cookie_from(response: &Response) -> Option<String> {
        let raw = response.headers().get(header::SET_COOKIE)?.to_str().ok()?;
        let pair = raw.split(';').next()?;
        pair.starts_with("tt_session=").then(|| pair.to_string())
    }

    const SIGNUP: &str = "name=Sam&email=sam%40example.com&team_number=10101&password=longenough1&confirm_password=longenough1";

    async fn signed_up(state: &AppState) -> String {
        let response = post(state, "/api/auth/signup", SIGNUP, None).await;
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        session_cookie_from(&response).expect("signup should set a session cookie")
    }

    #[tokio::test]
    async fn a_new_account_can_sign_up_and_lands_signed_in() {
        let state = migrated_state().await;
        let cookie = signed_up(&state).await;

        let body = text(get(&state, "/", Some(&cookie)).await).await;
        assert!(body.contains("Sam"), "home page should greet the user");
        assert!(body.contains("Rebuilt"), "and name the season");
    }

    #[tokio::test]
    async fn the_first_account_becomes_an_admin() {
        // Otherwise a fresh deployment has nobody who can grant anybody anything.
        let state = migrated_state().await;
        let cookie = signed_up(&state).await;

        let body = text(get(&state, "/account", Some(&cookie)).await).await;
        assert!(body.contains("Admin"));
    }

    #[tokio::test]
    async fn the_second_account_is_an_ordinary_scout() {
        let state = migrated_state().await;
        signed_up(&state).await;

        let response = post(
            &state,
            "/api/auth/signup",
            "name=Kim&email=kim%40example.com&password=longenough1&confirm_password=longenough1",
            None,
        )
        .await;
        let cookie = session_cookie_from(&response).expect("session");

        let body = text(get(&state, "/account", Some(&cookie)).await).await;
        assert!(body.contains("Scout"));
        assert!(!body.contains("Admin"));
    }

    #[tokio::test]
    async fn signing_up_twice_with_one_email_is_refused() {
        let state = migrated_state().await;
        signed_up(&state).await;

        let body = text(post(&state, "/api/auth/signup", SIGNUP, None).await).await;
        assert!(body.contains("already exists"));
    }

    #[tokio::test]
    async fn email_uniqueness_ignores_case_end_to_end() {
        let state = migrated_state().await;
        signed_up(&state).await;

        let body = text(post(
            &state,
            "/api/auth/signup",
            "name=Other&email=SAM%40EXAMPLE.COM&password=longenough1&confirm_password=longenough1",
            None,
        )
        .await)
        .await;
        assert!(body.contains("already exists"));
    }

    #[tokio::test]
    async fn signup_rejects_mismatched_passwords_and_keeps_what_was_typed() {
        let state = migrated_state().await;
        let body = text(post(
            &state,
            "/api/auth/signup",
            "name=Sam&email=sam%40example.com&team_number=10101&password=longenough1&confirm_password=different1",
            None,
        )
        .await)
        .await;

        assert!(body.contains("do not match"));
        // Re-rendering with the input intact is the whole point: retyping a form
        // on a phone is how people give up.
        assert!(body.contains("sam@example.com"));
        assert!(body.contains("10101"));
    }

    #[tokio::test]
    async fn signup_rejects_a_short_password() {
        let state = migrated_state().await;
        let body = text(
            post(
                &state,
                "/api/auth/signup",
                "name=Sam&email=sam%40example.com&password=short&confirm_password=short",
                None,
            )
            .await,
        )
        .await;
        assert!(body.contains("at least 8"));
    }

    #[tokio::test]
    async fn sign_in_works_with_the_right_password() {
        let state = migrated_state().await;
        signed_up(&state).await;

        let response = post(
            &state,
            "/api/auth/login",
            "email=sam%40example.com&password=longenough1",
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert!(session_cookie_from(&response).is_some());
    }

    #[tokio::test]
    async fn a_wrong_password_and_a_missing_account_give_the_same_message() {
        // Anything more specific turns the login form into a way to discover
        // which email addresses are registered.
        let state = migrated_state().await;
        signed_up(&state).await;

        let wrong = text(
            post(
                &state,
                "/api/auth/login",
                "email=sam%40example.com&password=wrongpassword",
                None,
            )
            .await,
        )
        .await;
        let missing = text(
            post(
                &state,
                "/api/auth/login",
                "email=nobody%40example.com&password=wrongpassword",
                None,
            )
            .await,
        )
        .await;

        assert!(wrong.contains("Invalid email or password"));
        assert!(missing.contains("Invalid email or password"));
    }

    #[tokio::test]
    async fn signing_out_invalidates_the_session() {
        let state = migrated_state().await;
        let cookie = signed_up(&state).await;

        post(&state, "/api/auth/logout", "", Some(&cookie)).await;

        // The same cookie must no longer authenticate.
        let response = get(&state, "/account", Some(&cookie)).await;
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            response.headers().get(header::LOCATION).unwrap(),
            "/sign-in"
        );
    }

    // ── Guards ──────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn anonymous_visitors_are_sent_to_sign_in() {
        let state = migrated_state().await;
        for path in [
            "/account",
            "/submission",
            "/lead-scout",
            "/lead-scout/assignments",
            "/drive-coach",
        ] {
            let response = get(&state, path, None).await;
            assert_eq!(response.status(), StatusCode::SEE_OTHER, "{path}");
            assert_eq!(
                response.headers().get(header::LOCATION).unwrap(),
                "/sign-in",
                "{path}"
            );
        }
    }

    #[tokio::test]
    async fn a_scout_without_the_role_is_sent_home_not_shown_a_403() {
        let state = migrated_state().await;
        signed_up(&state).await; // first account: admin

        let response = post(
            &state,
            "/api/auth/signup",
            "name=Kim&email=kim%40example.com&password=longenough1&confirm_password=longenough1",
            None,
        )
        .await;
        let scout = session_cookie_from(&response).expect("session");

        for path in ["/lead-scout", "/lead-scout/assignments", "/drive-coach"] {
            let response = get(&state, path, Some(&scout)).await;
            assert_eq!(response.status(), StatusCode::SEE_OTHER, "{path}");
            assert_eq!(
                response.headers().get(header::LOCATION).unwrap(),
                "/",
                "{path}"
            );
        }
    }

    #[tokio::test]
    async fn an_admin_reaches_the_privileged_pages() {
        let state = migrated_state().await;
        let admin = signed_up(&state).await;

        for path in [
            "/lead-scout",
            "/lead-scout/assignments",
            "/drive-coach",
            "/submission",
        ] {
            let response = get(&state, path, Some(&admin)).await;
            assert_eq!(response.status(), StatusCode::OK, "{path}");
        }
    }

    #[tokio::test]
    async fn the_nav_does_not_link_pages_a_scout_cannot_open() {
        let state = migrated_state().await;
        signed_up(&state).await;
        let response = post(
            &state,
            "/api/auth/signup",
            "name=Kim&email=kim%40example.com&password=longenough1&confirm_password=longenough1",
            None,
        )
        .await;
        let scout = session_cookie_from(&response).expect("session");

        let body = text(get(&state, "/", Some(&scout)).await).await;
        assert!(!body.contains("/lead-scout"));
        assert!(!body.contains("/drive-coach"));
    }

    // ── Password change ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn a_password_can_be_changed_and_the_new_one_works() {
        let state = migrated_state().await;
        let cookie = signed_up(&state).await;

        let body = text(
            post(
                &state,
                "/api/account/change-password",
                "current_password=longenough1&new_password=brandnew12&confirm_password=brandnew12",
                Some(&cookie),
            )
            .await,
        )
        .await;
        assert!(body.contains("Password changed"));

        let response = post(
            &state,
            "/api/auth/login",
            "email=sam%40example.com&password=brandnew12",
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
    }

    #[tokio::test]
    async fn changing_a_password_requires_the_current_one() {
        let state = migrated_state().await;
        let cookie = signed_up(&state).await;

        let body = text(
            post(
                &state,
                "/api/account/change-password",
                "current_password=notitatall&new_password=brandnew12&confirm_password=brandnew12",
                Some(&cookie),
            )
            .await,
        )
        .await;
        assert!(body.contains("Current password is incorrect"));
    }

    // ── Devices ─────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn a_heartbeat_registers_a_device() {
        let state = migrated_state().await;
        let response = post(
            &state,
            "/api/device/heartbeat",
            "",
            Some("tt_device=0191f7ac-1234-7000-8000-abcdefabcdef"),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        assert!(text(response).await.contains("\"status\":\"ok\""));

        let devices = state.repo.list_devices().await.expect("list");
        assert_eq!(devices.len(), 1);
        assert!(devices[0].last_seen_at.is_some());
    }

    #[tokio::test]
    async fn a_heartbeat_without_a_device_id_still_answers_200() {
        // A scout's tablet losing its cookie must not produce a console error.
        let state = migrated_state().await;
        let response = post(&state, "/api/device/heartbeat", "", None).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(state.repo.list_devices().await.expect("list").len(), 0);
    }

    #[tokio::test]
    async fn a_borrowed_tablet_keeps_the_team_it_was_first_seen_with() {
        let state = migrated_state().await;
        let cookie_a = signed_up(&state).await; // team 10101

        let device = "tt_device=0191f7ac-1234-7000-8000-abcdefabcdef";
        post(
            &state,
            "/api/device/heartbeat",
            "",
            Some(&format!("{cookie_a}; {device}")),
        )
        .await;

        // Someone from another team picks up the same tablet.
        post(
            &state,
            "/api/auth/signup",
            "name=Kim&email=kim%40example.com&team_number=254&password=longenough1&confirm_password=longenough1",
            None,
        )
        .await;
        let response = post(
            &state,
            "/api/auth/login",
            "email=kim%40example.com&password=longenough1",
            None,
        )
        .await;
        let cookie_b = session_cookie_from(&response).expect("session");
        post(
            &state,
            "/api/device/heartbeat",
            "",
            Some(&format!("{cookie_b}; {device}")),
        )
        .await;

        let devices = state.repo.list_devices().await.expect("list");
        assert_eq!(devices[0].team_number, Some(10101), "first team wins");
    }

    // ── Manual sync (I13) ───────────────────────────────────────────────────

    use crate::upstream::test_support::{EVENTS, first_stub, upstream_at};

    const BROWSER_ACCEPT: &str = "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8";

    async fn with_first_stub(state: AppState) -> AppState {
        let base = first_stub(EVENTS).await;
        AppState {
            upstream: Arc::new(upstream_at(Some(&base), false)),
            ..state
        }
    }

    async fn sync_request(state: &AppState, accept: &str, cookie: Option<&str>) -> Response {
        let mut req = Request::builder()
            .method("POST")
            .uri("/api/frc/sync")
            .header(header::ACCEPT, accept);
        if let Some(c) = cookie {
            req = req.header(header::COOKIE, c);
        }
        router(state.clone())
            .oneshot(req.body(Body::empty()).unwrap())
            .await
            .expect("request")
    }

    #[tokio::test]
    async fn only_leads_and_admins_can_trigger_a_sync() {
        let state = with_first_stub(migrated_state().await).await;

        let anonymous = sync_request(&state, "*/*", None).await;
        assert_eq!(anonymous.status(), StatusCode::SEE_OTHER);
        assert_eq!(anonymous.headers()[header::LOCATION], "/sign-in");

        signed_up(&state).await; // the first account, an admin
        let response = post(
            &state,
            "/api/auth/signup",
            "name=Kim&email=kim%40example.com&password=longenough1&confirm_password=longenough1",
            None,
        )
        .await;
        let scout = session_cookie_from(&response).expect("session");
        let refused = sync_request(&state, "*/*", Some(&scout)).await;
        assert_eq!(refused.status(), StatusCode::SEE_OTHER);
        assert_eq!(refused.headers()[header::LOCATION], "/");

        assert!(
            state.repo.list_events().await.unwrap().is_empty(),
            "neither refused request may have synced anything"
        );
    }

    #[tokio::test]
    async fn a_script_gets_json_counts() {
        let state = with_first_stub(migrated_state().await).await;
        let admin = signed_up(&state).await;

        let response = sync_request(&state, "*/*", Some(&admin)).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body: serde_json::Value = serde_json::from_str(&text(response).await).expect("json");

        assert_eq!(body["ok"], true, "{body}");
        assert_eq!(body["events"], 1);
        assert_eq!(body["teams"], 2);
        assert_eq!(body["problems"], serde_json::json!([]));
        assert_eq!(state.repo.list_events().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn the_sync_button_gets_the_page_back_with_the_outcome() {
        let state = with_first_stub(migrated_state().await).await;
        let admin = signed_up(&state).await;

        let response = sync_request(&state, BROWSER_ACCEPT, Some(&admin)).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = text(response).await;

        assert!(body.contains("Synced 1 event(s) and 2 team(s)."), "{body}");
        assert!(
            body.contains("just now"),
            "the card shows the sync that just ran"
        );
        assert!(body.contains("Sync now"), "and the button, to go again");
    }

    #[tokio::test]
    async fn the_lead_scout_page_says_what_upstream_is_configured() {
        let state = migrated_state().await; // no credentials at all
        let admin = signed_up(&state).await;

        let body = text(get(&state, "/lead-scout", Some(&admin)).await).await;

        assert!(body.contains("FIRST and TBA data"));
        assert!(
            body.contains("Not checked yet"),
            "unknown, not \"No internet\""
        );
        assert!(body.contains("never"));
        assert_eq!(body.matches("not configured").count(), 2, "FIRST and TBA");
        assert!(
            !body.contains("<dt>Stored</dt>"),
            "no event, nothing to count"
        );
    }

    // ── Event selection and summary (U2, U3) ────────────────────────────────

    /// Store an event running from `start` to `end` days from today, with the
    /// given teams on its roster.
    async fn seed_event(state: &AppState, key: &str, name: &str, days: (i64, i64), teams: &[i32]) {
        use tt_core::records::{Event, Team};
        let now = chrono::Utc::now();
        let today = now.date_naive();
        let event = Event {
            key: key.into(),
            name: name.into(),
            location: Some("Boston, MA".into()),
            timezone: None,
            start_date: Some(today + chrono::TimeDelta::days(days.0)),
            end_date: Some(today + chrono::TimeDelta::days(days.1)),
            event_code: None,
            event_type: None,
            district_key: None,
            week: None,
        };
        state.repo.upsert_event(&event, now).await.expect("event");
        for &number in teams {
            let team = Team {
                number,
                name: format!("Team {number}"),
                nickname: None,
                school: None,
                city: None,
                state: None,
                country: None,
                rookie_year: None,
                website: None,
            };
            state.repo.upsert_team(&team, now).await.expect("team");
            state
                .repo
                .link_event_team(key, number, now)
                .await
                .expect("link");
        }
    }

    #[tokio::test]
    async fn home_summarises_the_event_running_today() {
        let state = migrated_state().await;
        seed_event(&state, "2026past", "Last Month", (-30, -28), &[10101]).await;
        seed_event(&state, "2026now", "This Weekend", (-1, 1), &[254, 10101]).await;
        let cookie = signed_up(&state).await; // team 10101

        let body = text(get(&state, "/", Some(&cookie)).await).await;

        assert!(body.contains("<h2>This Weekend</h2>"), "{body}");
        assert!(body.contains("<dt>Teams</dt><dd>2</dd>"));
        assert!(body.contains(r#"<li class="you"><strong>10101</strong>"#));
        assert!(!body.contains("Your team is not listed"));
        assert!(body.contains(r#"<option value="2026now" selected>"#));
    }

    #[tokio::test]
    async fn the_event_in_the_url_wins_and_every_link_keeps_it() {
        let state = migrated_state().await;
        seed_event(&state, "2026past", "Last Month", (-30, -28), &[10101]).await;
        seed_event(&state, "2026now", "This Weekend", (-1, 1), &[10101]).await;
        let cookie = signed_up(&state).await; // an admin, so every link shows

        let body = text(get(&state, "/?event=2026past", Some(&cookie)).await).await;

        assert!(body.contains("<h2>Last Month</h2>"));
        assert!(body.contains(r#"<option value="2026past" selected>"#));
        for link in ["/submission", "/lead-scout", "/drive-coach"] {
            assert!(
                body.contains(&format!(r#"href="{link}?event=2026past""#)),
                "{link} should carry the event"
            );
        }

        // And the next page keeps showing it.
        let next = text(get(&state, "/lead-scout?event=2026past", Some(&cookie)).await).await;
        assert!(next.contains(r#"<option value="2026past" selected>"#));
        assert!(next.contains(r#"action="/api/frc/sync?event=2026past""#));
    }

    #[tokio::test]
    async fn a_team_missing_from_the_roster_is_told_so() {
        let state = migrated_state().await;
        seed_event(&state, "2026now", "This Weekend", (-1, 1), &[254]).await;
        let cookie = signed_up(&state).await; // team 10101, on no roster

        let body = text(get(&state, "/", Some(&cookie)).await).await;

        assert!(
            body.contains("<h2>This Weekend</h2>"),
            "still offered the event"
        );
        assert!(body.contains("Your team is not listed for this event yet."));
    }

    #[tokio::test]
    async fn an_unknown_event_is_named_and_the_page_still_works() {
        let state = migrated_state().await;
        seed_event(&state, "2026now", "This Weekend", (-1, 1), &[]).await;

        let response = get(&state, "/?event=2026nope", None).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = text(response).await;

        assert!(body.contains("There is no event “2026nope” on this server"));
        assert!(
            body.contains("<h2>This Weekend</h2>"),
            "the default instead"
        );
    }

    #[tokio::test]
    async fn an_empty_database_says_how_to_load_events() {
        let state = migrated_state().await;
        let body = text(get(&state, "/", None).await).await;
        assert!(body.contains("No events have been loaded yet."));
        assert!(!body.contains("event-switcher\""), "no empty switcher");
    }

    #[tokio::test]
    async fn sign_in_has_no_event_switcher() {
        let state = migrated_state().await;
        seed_event(&state, "2026now", "This Weekend", (-1, 1), &[]).await;
        let body = text(get(&state, "/sign-in", None).await).await;
        assert!(!body.contains("event-switcher\""));
    }

    // ── Scouting (U4) ───────────────────────────────────────────────────────

    const RECORD_ID: &str = "0191f7ac-1234-7000-8000-000000000001";
    const DEVICE: &str = "tt_device=0191f7ac-1234-7000-8000-abcdefabcdef";

    async fn seed_match(state: &AppState, number: i32, played: bool) {
        use tt_core::matches::CompLevel;
        use tt_core::records::MatchRecord;
        let record = MatchRecord {
            key: format!("2026now_qm{number}"),
            event_key: "2026now".into(),
            comp_level: CompLevel::Qualification,
            set_number: 1,
            match_number: number,
            // Only 10101 and 254 are on the synced roster; 1-4 are not.
            red: [Some(10101), Some(254), Some(1)],
            blue: [Some(2), Some(3), Some(4)],
            red_score: None,
            blue_score: None,
            winner: None,
            played,
            scheduled_at: None,
            actual_at: None,
        };
        state
            .repo
            .upsert_match(&record, chrono::Utc::now())
            .await
            .expect("match");
    }

    /// An event running now with Q1 played and Q2 not, and a signed-in scout on
    /// team 10101 whose tablet has checked in. Returns the scout's cookies.
    async fn scouting() -> (AppState, String) {
        let state = migrated_state().await;
        seed_event(&state, "2026now", "This Weekend", (-1, 1), &[10101, 254]).await;
        seed_match(&state, 1, true).await;
        seed_match(&state, 2, false).await;
        let cookies = format!("{}; {DEVICE}", signed_up(&state).await);
        post(&state, "/api/device/heartbeat", "", Some(&cookies)).await;
        (state, cookies)
    }

    fn observation_form(team: i32, record_id: &str, answers: &str) -> String {
        format!("match=2026now_qm2&team={team}&record_id={record_id}&{answers}")
    }

    /// What a browser posts: every counter (untouched ones at their rendered
    /// 0), ticked boxes only, and the text.
    const GOOD_ANSWERS: &str = "f.starting_position=center&f.auto_scored=0&f.teleop_scored=9\
                                &f.broke_down=on&f.penalties=0&f.notes=tippy+on+the+ramp";

    async fn observations(state: &AppState) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM observations")
            .fetch_one(state.repo.pool())
            .await
            .expect("count")
    }

    #[tokio::test]
    async fn scouting_waits_for_a_match_schedule() {
        let state = migrated_state().await;
        seed_event(&state, "2026now", "This Weekend", (-1, 1), &[10101]).await;
        let cookie = signed_up(&state).await;

        let body = text(get(&state, "/submission", Some(&cookie)).await).await;
        assert!(body.contains("This Weekend has no match schedule yet."));
        assert!(!body.contains("match-picker"));
    }

    #[tokio::test]
    async fn the_scouting_page_opens_on_the_next_unplayed_match() {
        let (state, cookies) = scouting().await;
        let body = text(get(&state, "/submission", Some(&cookies)).await).await;

        assert!(body.contains("Scout Q2"), "{body}");
        assert!(body.contains(r#"<option value="2026now_qm1">Q1 · played</option>"#));
        assert!(
            body.contains(r#"href="/submission?event=2026now&#38;match=2026now_qm2&#38;team=254""#),
            "each robot is a link to its form"
        );
        assert!(!body.contains("scout-form"), "no robot chosen yet");
    }

    #[tokio::test]
    async fn choosing_a_robot_opens_the_season_form() {
        let (state, cookies) = scouting().await;
        let body = text(
            get(
                &state,
                "/submission?event=2026now&match=2026now_qm2&team=254",
                Some(&cookies),
            )
            .await,
        )
        .await;

        assert!(body.contains("Q2 · Team 254 · Red 2"), "{body}");
        assert!(body.contains("Team 254"), "the roster's name for it");
        for field in state.season.fields() {
            assert!(
                body.contains(&format!(r#"name="f.{}""#, field.key)),
                "{}",
                field.key
            );
        }
        assert!(body.contains(r#"action="/api/submission?event=2026now""#));
        assert!(body.contains(r#"name="record_id" value="0"#), "a v7 id");
    }

    #[tokio::test]
    async fn a_saved_observation_is_pending_review_with_who_where_and_when() {
        let (state, cookies) = scouting().await;
        let response = post(
            &state,
            "/api/submission?event=2026now",
            &observation_form(254, RECORD_ID, GOOD_ANSWERS),
            Some(&cookies),
        )
        .await;

        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        let next = response.headers()[header::LOCATION]
            .to_str()
            .unwrap()
            .to_string();
        assert_eq!(
            next,
            "/submission?event=2026now&match=2026now_qm2&saved=254"
        );

        use sqlx::Row;
        let row = sqlx::query(
            "SELECT client_record_id, event_key, alliance, payload, schema_version, \
                    scouter_id, device_id, submitting_team, review_state \
             FROM observations",
        )
        .fetch_one(state.repo.pool())
        .await
        .expect("one row");
        assert_eq!(row.get::<String, _>("client_record_id"), RECORD_ID);
        assert_eq!(row.get::<String, _>("event_key"), "2026now");
        assert_eq!(
            row.get::<String, _>("alliance"),
            "red",
            "read off the match"
        );
        assert_eq!(
            row.get::<String, _>("payload"),
            r#"{"auto_scored":0,"broke_down":true,"no_show":false,"notes":"tippy on the ramp","penalties":0,"starting_position":"center","teleop_scored":9}"#,
            "untouched counters are zeros and unticked boxes are noes"
        );
        assert_eq!(row.get::<i64, _>("schema_version"), state.season.version);
        assert_eq!(row.get::<Option<i64>, _>("scouter_id"), Some(1));
        assert!(
            row.get::<Option<i64>, _>("device_id").is_some(),
            "the tablet"
        );
        assert_eq!(
            row.get::<Option<i32>, _>("submitting_team"),
            Some(10101),
            "resolved at write time (L7)"
        );
        assert_eq!(row.get::<String, _>("review_state"), "pending");

        let confirmation = text(get(&state, &next, Some(&cookies)).await).await;
        assert!(confirmation.contains("Saved team 254 in Q2."));
        assert!(confirmation.contains("Recorded"));
    }

    #[tokio::test]
    async fn a_double_tapped_save_stores_one_observation() {
        let (state, cookies) = scouting().await;
        for _ in 0..2 {
            let response = post(
                &state,
                "/api/submission",
                &observation_form(254, RECORD_ID, GOOD_ANSWERS),
                Some(&cookies),
            )
            .await;
            assert_eq!(response.status(), StatusCode::SEE_OTHER);
        }
        assert_eq!(observations(&state).await, 1);
    }

    #[tokio::test]
    async fn a_form_with_mistakes_comes_back_with_the_answers_in_it() {
        let (state, cookies) = scouting().await;
        let response = post(
            &state,
            "/api/submission",
            &observation_form(
                254,
                RECORD_ID,
                "f.teleop_scored=9o&f.notes=tippy+on+the+ramp",
            ),
            Some(&cookies),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = text(response).await;
        assert!(body.contains("Not saved yet."));
        assert!(
            body.contains("Choose one."),
            "starting position is required"
        );
        assert!(body.contains("Enter a whole number."));
        assert!(body.contains(r#"value="9o""#));
        assert!(body.contains("tippy on the ramp</textarea>"));
        assert!(
            body.contains(&format!(r#"name="record_id" value="{RECORD_ID}""#)),
            "the same id, so fixing and resaving cannot store it twice"
        );
        assert_eq!(observations(&state).await, 0);
    }

    #[tokio::test]
    async fn a_second_observation_of_one_robot_is_refused_and_says_why() {
        let (state, cookies) = scouting().await;
        post(
            &state,
            "/api/submission",
            &observation_form(254, RECORD_ID, GOOD_ANSWERS),
            Some(&cookies),
        )
        .await;

        let other_id = "0191f7ac-1234-7000-8000-000000000002";
        let body = text(
            post(
                &state,
                "/api/submission",
                &observation_form(254, other_id, GOOD_ANSWERS),
                Some(&cookies),
            )
            .await,
        )
        .await;
        assert!(body.contains("Not saved: each scout keeps one observation per robot per match."));
        assert!(body.contains("You have already recorded team 254 in Q2."));
        assert!(!body.contains("scout-form"));
        assert_eq!(observations(&state).await, 1);

        // Choosing the robot again says so up front, before any typing.
        let body = text(
            get(
                &state,
                "/submission?event=2026now&match=2026now_qm2&team=254",
                Some(&cookies),
            )
            .await,
        )
        .await;
        assert!(body.contains("You have already recorded team 254 in Q2."));
        assert!(!body.contains("scout-form"));
    }

    #[tokio::test]
    async fn the_robot_must_be_in_the_match() {
        let (state, cookies) = scouting().await;
        let body = text(
            post(
                &state,
                "/api/submission",
                &observation_form(9999, RECORD_ID, GOOD_ANSWERS),
                Some(&cookies),
            )
            .await,
        )
        .await;
        assert!(body.contains("Not saved: pick the robot you watched from this match."));
        assert!(body.contains("Team 9999 is not in Q2."));
        assert_eq!(observations(&state).await, 0);
    }

    #[tokio::test]
    async fn a_match_that_is_not_scheduled_is_refused() {
        let (state, cookies) = scouting().await;
        let body = text(
            post(
                &state,
                "/api/submission",
                &format!("match=2026now_qm99&team=254&record_id={RECORD_ID}&{GOOD_ANSWERS}"),
                Some(&cookies),
            )
            .await,
        )
        .await;
        assert!(body.contains("Not saved: that match is not on the schedule."));
        assert_eq!(observations(&state).await, 0);
    }

    #[tokio::test]
    async fn a_robot_the_roster_has_not_synced_can_be_scouted() {
        // Team 3 is in the TBA schedule but not yet in any FIRST roster.
        let (state, cookies) = scouting().await;
        let response = post(
            &state,
            "/api/submission",
            &observation_form(3, RECORD_ID, GOOD_ANSWERS),
            Some(&cookies),
        )
        .await;
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(observations(&state).await, 1);
    }

    #[tokio::test]
    async fn a_saved_link_confirms_nothing_that_was_not_saved() {
        let (state, cookies) = scouting().await;
        let body = text(
            get(
                &state,
                "/submission?event=2026now&match=2026now_qm2&saved=254",
                Some(&cookies),
            )
            .await,
        )
        .await;
        assert!(!body.contains("Saved team"));
    }

    #[tokio::test]
    async fn only_a_signed_in_scout_can_save() {
        let (state, _) = scouting().await;
        let response = post(
            &state,
            "/api/submission",
            &observation_form(254, RECORD_ID, GOOD_ANSWERS),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(response.headers()[header::LOCATION], "/sign-in");
        assert_eq!(observations(&state).await, 0);
    }

    // ── Assignment grid (L1) ────────────────────────────────────────────────

    /// Assign `team` in `2026now_qm{number}` to a scout or a tablet.
    async fn assign(
        state: &AppState,
        number: i32,
        team: i32,
        scouter: Option<i64>,
        device: Option<i64>,
    ) {
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO scout_assignments \
                 (match_key, team_number, event_key, scouter_id, device_id, created_at, updated_at) \
             VALUES (?, ?, '2026now', ?, ?, ?, ?)",
        )
        .bind(format!("2026now_qm{number}"))
        .bind(team)
        .bind(scouter)
        .bind(device)
        .bind(&now)
        .bind(&now)
        .execute(state.repo.pool())
        .await
        .expect("assign");
    }

    #[tokio::test]
    async fn the_assignment_grid_shows_who_watches_each_robot() {
        let (state, admin) = scouting().await;
        // Sam (the admin, user 1) on 254 and Sam's tablet (device 1) on 10101.
        assign(&state, 2, 254, Some(1), None).await;
        assign(&state, 2, 10101, None, Some(1)).await;

        let response = get(&state, "/lead-scout/assignments", Some(&admin)).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = text(response).await;

        assert!(body.contains("Assignments · This Weekend"), "{body}");
        assert!(body.contains("<strong>2 of 6</strong> robots in upcoming matches"));
        let q2 = &body[body.find(r#"id="2026now_qm2""#).expect("Q2 row")..];
        let q2 = &q2[..q2.find("</tr>").unwrap()];
        assert!(q2.contains("Device 0191f7ac"), "an unnamed tablet: {q2}");
        assert!(q2.contains(r#"<span class="slot-kind">tablet</span>"#));
        assert!(q2.contains(">Sam<"));
        assert!(q2.contains(r#"<span class="slot-name">Team 254</span>"#));
        assert_eq!(q2.matches("Unassigned").count(), 4);

        // Q1 is played: still listed, folded away below.
        let played = body.find(r#"id="played""#).expect("played section");
        assert!(body.find(r#"id="2026now_qm1""#).unwrap() > played);
    }

    #[tokio::test]
    async fn the_grid_waits_for_a_match_schedule() {
        let state = migrated_state().await;
        seed_event(&state, "2026now", "This Weekend", (-1, 1), &[10101]).await;
        let admin = signed_up(&state).await;

        let body = text(get(&state, "/lead-scout/assignments", Some(&admin)).await).await;
        assert!(body.contains("This Weekend has no match schedule yet."));
        assert!(!body.contains("<table"));
    }

    #[tokio::test]
    async fn the_grid_is_one_tap_from_the_lead_scout_page_and_keeps_the_event() {
        let (state, admin) = scouting().await;
        let body = text(get(&state, "/lead-scout?event=2026now", Some(&admin)).await).await;
        assert!(body.contains(r#"href="/lead-scout/assignments?event=2026now""#));

        let grid = text(
            get(
                &state,
                "/lead-scout/assignments?event=2026now",
                Some(&admin),
            )
            .await,
        )
        .await;
        assert!(
            grid.contains(r#"href="/lead-scout?event=2026now""#),
            "and back"
        );
    }

    // ── No Unpoly: plain pages and live regions (U8) ────────────────────────

    #[tokio::test]
    async fn every_script_and_stylesheet_a_page_loads_is_served() {
        let (state, cookie) = scouting().await;
        let body = text(
            get(
                &state,
                "/submission?match=2026now_qm2&team=254",
                Some(&cookie),
            )
            .await,
        )
        .await;

        let assets: Vec<&str> = body
            .split(['"', '\''])
            .filter(|s| s.starts_with("/static/"))
            .collect();
        assert!(assets.len() >= 5, "{assets:?}");
        for asset in assets {
            let response = get(&state, asset, None).await;
            assert_eq!(response.status(), StatusCode::OK, "{asset}");
        }
    }

    /// Every live region on `body`, as the `(id, url)` static/js/live.js reads
    /// off its tag.
    fn live_regions(body: &str) -> Vec<(String, String)> {
        body.match_indices(" data-live=\"")
            .map(|(at, _)| {
                let start = body[..at].rfind('<').expect("inside a tag");
                let end = at + body[at..].find('>').expect("the tag ends");
                let tag = &body[start..end];
                let attr = |name: &str| {
                    let from = tag.find(&format!(" {name}=\""))? + name.len() + 3;
                    let len = tag[from..].find('"')?;
                    Some(tag[from..from + len].replace("&#38;", "&"))
                };
                (
                    attr("id").expect("a live region needs an id"),
                    attr("data-live").expect("and a page to refresh from"),
                )
            })
            .collect()
    }

    /// What live.js relies on: each region's URL answers with a page holding
    /// exactly one element with the region's id. Returns those pages.
    async fn assert_live_regions_resolve(
        state: &AppState,
        body: &str,
        cookie: &str,
    ) -> Vec<String> {
        let regions = live_regions(body);
        assert!(!regions.is_empty(), "no live region on the page");
        let mut pages = Vec::new();
        for (id, url) in regions {
            let response = get(state, &url, Some(cookie)).await;
            assert_eq!(response.status(), StatusCode::OK, "{url}");
            let page = text(response).await;
            let found = page.matches(&format!(" id=\"{id}\"")).count();
            assert_eq!(found, 1, "#{id} on {url}");
            pages.push(page);
        }
        pages
    }

    #[tokio::test]
    async fn the_sync_card_refreshes_itself_from_the_lead_scout_page() {
        let (state, admin) = scouting().await;
        let body = text(get(&state, "/lead-scout?event=2026now", Some(&admin)).await).await;

        let expected = (
            "upstream-status".to_string(),
            "/lead-scout?event=2026now".to_string(),
        );
        assert_eq!(live_regions(&body), [expected]);
        assert_live_regions_resolve(&state, &body, &admin).await;

        // Once the session is gone the refresh lands on sign-in, which has no
        // such element, so live.js keeps the card instead of swapping in a form.
        let signed_out = get(&state, "/lead-scout?event=2026now", None).await;
        assert_eq!(signed_out.headers()[header::LOCATION], "/sign-in");
        let sign_in = text(get(&state, "/sign-in", None).await).await;
        assert!(!sign_in.contains(r#"id="upstream-status""#));
    }

    #[tokio::test]
    async fn after_a_sync_press_the_card_refreshes_from_the_page_not_the_post() {
        let state = with_first_stub(migrated_state().await).await;
        let admin = signed_up(&state).await;

        // The address bar now says /api/frc/sync; the region must not.
        let body = text(sync_request(&state, BROWSER_ACCEPT, Some(&admin)).await).await;
        let refreshed = assert_live_regions_resolve(&state, &body, &admin).await;

        // The outcome of the press sits outside the region, so a refresh
        // cannot take it away while the lead scout is reading it.
        let outcome = body.find("Synced 1 event(s)").expect("outcome shown");
        let region = body.find(r#"id="upstream-status""#).expect("region");
        assert!(outcome < region);
        assert!(!refreshed[0].contains("Synced 1 event(s)"));
    }

    #[tokio::test]
    async fn the_sync_card_counts_what_is_stored_for_the_event() {
        let (state, admin) = scouting().await; // two teams; Q1 played, Q2 not
        let page = || get(&state, "/lead-scout?event=2026now", Some(&admin));

        let body = text(page().await).await;
        assert!(
            body.contains("This Weekend: 2 teams · 2 matches, 1 played"),
            "{body}"
        );

        // What a background pass landing looks like on the next refresh.
        seed_match(&state, 3, false).await;
        let body = text(page().await).await;
        assert!(body.contains("This Weekend: 2 teams · 3 matches, 1 played"));
    }

    // ── Error pages (U10) ───────────────────────────────────────────────────

    /// Send `request` to `app` with the given `Accept`, and read back the status,
    /// the content type, and the body.
    async fn fetch(
        app: Router,
        request: axum::http::request::Builder,
        accept: &str,
        body: &str,
    ) -> (StatusCode, String, String) {
        let response = app
            .oneshot(
                request
                    .header(header::ACCEPT, accept)
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .expect("request");
        let status = response.status();
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .map(|v| v.to_str().unwrap().to_string())
            .unwrap_or_default();
        (status, content_type, text(response).await)
    }

    fn form_post(uri: &str) -> axum::http::request::Builder {
        Request::builder()
            .method("POST")
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
    }

    #[tokio::test]
    async fn every_dead_end_a_browser_reaches_is_a_page_with_the_nav() {
        let state = migrated_state().await;
        let cases = [
            // A mistyped link or an old bookmark.
            (
                Request::get("/submision"),
                "",
                StatusCode::NOT_FOUND,
                "Page not found",
            ),
            // Reopening the address a failed sign-in left in the address bar.
            (
                Request::get("/api/auth/login"),
                "",
                StatusCode::METHOD_NOT_ALLOWED,
                "Nothing to show here",
            ),
            // A form from before a field was added.
            (
                form_post("/api/auth/login"),
                "email=a%40b.c",
                StatusCode::UNPROCESSABLE_ENTITY,
                "That did not work",
            ),
            // A body that is not a form at all.
            (
                Request::post("/api/auth/signup").header(header::CONTENT_TYPE, "text/plain"),
                "x",
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "That did not work",
            ),
            // A missing file opened straight in the address bar.
            (
                Request::get("/static/js/gone.js"),
                "",
                StatusCode::NOT_FOUND,
                "Page not found",
            ),
        ];

        for (request, body, status, heading) in cases {
            let (got, content_type, page) =
                fetch(router(state.clone()), request, BROWSER_ACCEPT, body).await;
            assert_eq!(got, status, "{page}");
            assert!(
                content_type.starts_with("text/html"),
                "{status}: {content_type}"
            );
            assert!(
                page.contains(&format!("<h1>{heading}</h1>")),
                "{status}: {page}"
            );
            assert!(page.contains(r#"<header class="nav">"#), "{status}");
            assert!(
                page.contains(r#"href="/sign-in""#),
                "{status}: signed out, so offer sign-in"
            );
            assert!(
                !page.contains("Failed to deserialize"),
                "axum's text goes to the log"
            );
            assert!(!page.contains("Form requests must"));
        }
    }

    #[tokio::test]
    async fn a_405_page_keeps_the_allow_header() {
        let state = migrated_state().await;
        let response = router(state)
            .oneshot(
                Request::get("/api/auth/login")
                    .header(header::ACCEPT, BROWSER_ACCEPT)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("request");
        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(response.headers()[header::ALLOW], "POST");
    }

    #[tokio::test]
    async fn a_signed_in_scout_keeps_their_nav_on_an_error_page() {
        let state = migrated_state().await;
        let cookie = signed_up(&state).await;
        // Where a failed save leaves the address bar.
        let request = Request::get("/api/submission").header(header::COOKIE, &cookie);

        let (status, _, page) = fetch(router(state), request, BROWSER_ACCEPT, "").await;
        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
        assert!(page.contains("Sign out"), "{page}");
        assert!(
            page.contains(r#"href="/submission""#),
            "the Scout tab, to go back"
        );
        assert!(page.contains("405 · /api/submission"));
    }

    #[tokio::test]
    async fn scripts_and_asset_loads_get_errors_as_they_were() {
        let state = migrated_state().await;

        let (status, content_type, body) =
            fetch(router(state.clone()), Request::get("/nope"), "*/*", "").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(!content_type.starts_with("text/html"));
        assert!(body.is_empty());

        // What a <script src> for a missing file sends.
        let (status, _, body) = fetch(
            router(state.clone()),
            Request::get("/static/js/gone.js"),
            "*/*",
            "",
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(body.is_empty());

        // A script can still read why its post was refused.
        let (status, _, body) = fetch(
            router(state),
            form_post("/api/auth/login"),
            "*/*",
            "email=a%40b.c",
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(body.contains("missing field `password`"), "{body}");
    }

    /// Three routes that fail in ways no real route does yet, behind the same
    /// layer as the real router.
    fn app_with_faults(state: AppState) -> Router {
        async fn boom() -> impl IntoResponse {
            (StatusCode::INTERNAL_SERVER_ERROR, "pool timed out")
        }
        async fn own() -> impl IntoResponse {
            (StatusCode::CONFLICT, Html("<p>already saved</p>"))
        }
        async fn sized() -> impl IntoResponse {
            (
                StatusCode::BAD_REQUEST,
                [(header::CONTENT_LENGTH, "4")],
                "nope",
            )
        }
        Router::new()
            .route("/boom", axum::routing::get(boom))
            .route("/own", axum::routing::get(own))
            .route("/sized", axum::routing::get(sized))
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                crate::errors::html_errors,
            ))
            .with_state(state)
    }

    #[tokio::test]
    async fn a_fault_in_a_handler_is_the_error_page_and_internals_stay_in_the_log() {
        let app = app_with_faults(migrated_state().await);
        let (status, content_type, page) =
            fetch(app, Request::get("/boom"), BROWSER_ACCEPT, "").await;

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(content_type.starts_with("text/html"));
        assert!(page.contains("<h1>Something went wrong</h1>"), "{page}");
        assert!(!page.contains("pool timed out"));
    }

    #[tokio::test]
    async fn an_error_that_is_already_a_page_is_left_alone() {
        let app = app_with_faults(migrated_state().await);
        let (status, _, page) = fetch(app, Request::get("/own"), BROWSER_ACCEPT, "").await;

        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(page, "<p>already saved</p>");
    }

    #[tokio::test]
    async fn a_length_set_for_the_replaced_body_is_not_sent_with_the_page() {
        // Hyper trusts an explicit Content-Length, so a stale one would cut the
        // page off after four bytes.
        let app = app_with_faults(migrated_state().await);
        let response = app
            .oneshot(
                Request::get("/sized")
                    .header(header::ACCEPT, BROWSER_ACCEPT)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("request");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let length = response.headers().get(header::CONTENT_LENGTH).cloned();
        let page = text(response).await;
        assert!(page.contains("<h1>That did not work</h1>"));
        // axum fills in the length of whatever body leaves the layer.
        assert_eq!(length.expect("length"), page.len().to_string().as_str());
    }
}
