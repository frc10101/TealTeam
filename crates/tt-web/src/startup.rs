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
//!   3. **Validate config. This is the only fatal step** -- a bad `TEALTEAM_ENV`
//!      or `PORT` is a typo someone can fix, and guessing at it is how the
//!      retired implementation ended up erasing databases.
//!   4. Open the database *lazily*. Does not touch the disk.
//!   5. Probe it. **Failure is logged, not fatal**; the app serves degraded.
//!   6. Bind and serve. Failure to bind is fatal -- there is nothing to serve on.

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

/// Everything a handler needs. Cheap to clone; the contents are shared.
#[derive(Clone)]
pub struct AppState {
    pub repo: Arc<SqliteRepo>,
    /// Parsed once at startup rather than per request. Immutable for the life of
    /// the process, which is what makes the schema version a deployment fact.
    pub season: Arc<SeasonSchema>,
}

pub async fn run() -> anyhow::Result<()> {
    // 1-2. Config comes before tracing is configured, so early failures print to
    // stderr rather than vanishing. Tracing then picks up RUST_LOG from the same
    // .env files.
    config::load_dotenv_files();
    init_tracing();

    // 3. The only fatal validation step.
    let config = Config::from_env().context("invalid configuration")?;
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
    match repo.health().await {
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
        }
        Health::Down => {
            warn!(
                "database unavailable at {} -- serving degraded pages. \
                 Storage-backed features will not work until this is fixed.",
                config.database_url
            );
        }
    }

    let state = AppState {
        repo: Arc::new(repo),
        season: Arc::new(season),
    };

    // 6. Bind on 0.0.0.0 so LAN clients reach it. Nothing else is reachable from
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
        .route("/drive-coach", get(handlers::drive_coach))
        // Forms
        .route("/api/auth/login", post(handlers::login))
        .route("/api/auth/signup", post(handlers::signup))
        .route("/api/auth/logout", post(handlers::logout))
        .route(
            "/api/account/change-password",
            post(handlers::change_password),
        )
        .route("/api/device/heartbeat", post(handlers::device_heartbeat))
        // Operational
        .route("/health", get(health_json))
        .route("/status", get(health_page))
        .nest_service("/static", tower_http::services::ServeDir::new(static_dir()))
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
        for path in ["/account", "/submission", "/lead-scout", "/drive-coach"] {
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

        for path in ["/lead-scout", "/drive-coach"] {
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

        for path in ["/lead-scout", "/drive-coach", "/submission"] {
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
}
