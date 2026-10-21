//! The FTC Events pull (F7) against a stub that answers like the FTC Events
//! API: basic auth, `Last-Modified`, and a 304 for `If-Modified-Since`.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use tt_repo::Repo;
use tt_repo_sqlite::SqliteRepo;
use tt_upstream::ftc::{self, FtcClient};
use tt_upstream::{Uplink, UpstreamError};

const EVENTS: &str = include_str!("fixtures/ftc/2026_events_USALATQ.json");
const TEAMS: &str = include_str!("fixtures/ftc/2026_teams_USALATQ.json");
const SCHEDULE: &str = include_str!("fixtures/ftc/2026_schedule_USALATQ_qual_hybrid.json");
const RANKINGS: &str = include_str!("fixtures/ftc/2026_rankings_USALATQ.json");
const SCORES: &str = include_str!("fixtures/ftc/2026_scores_USALATQ_qual.json");

const LAST_MODIFIED: &str = "Sat, 24 Oct 2026 14:10:00 GMT";

#[derive(Default)]
struct Seen {
    requests: AtomicUsize,
    not_modified: AtomicUsize,
    /// Requests that carried `If-Modified-Since`.
    conditional: AtomicUsize,
    /// Any request that carried `FMS-OnlyModifiedSince`.
    only_modified: AtomicBool,
    paths: Mutex<Vec<String>>,
    /// Answer the next `n` requests whose path contains `part` with a 503
    /// and this `Retry-After`: `(part, retry_after, n)`.
    busy: Mutex<Option<(&'static str, &'static str, usize)>>,
}

async fn answer(seen: &Seen, uri: &Uri, headers: &HeaderMap, body: &'static str) -> Response {
    seen.requests.fetch_add(1, Ordering::SeqCst);
    seen.paths.lock().unwrap().push(uri.to_string());
    {
        let mut busy = seen.busy.lock().unwrap();
        if let Some((part, retry_after, n)) = busy.as_mut()
            && *n > 0
            && uri.path().contains(*part)
        {
            *n -= 1;
            let retry_after = *retry_after;
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                [(header::RETRY_AFTER, retry_after)],
                "busy",
            )
                .into_response();
        }
    }
    if headers.contains_key("FMS-OnlyModifiedSince") {
        seen.only_modified.store(true, Ordering::SeqCst);
    }
    // user:key
    if headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        != Some("Basic dXNlcjprZXk=")
    {
        return (StatusCode::UNAUTHORIZED, "bad key").into_response();
    }
    let since = headers
        .get(header::IF_MODIFIED_SINCE)
        .and_then(|v| v.to_str().ok());
    if since.is_some() {
        seen.conditional.fetch_add(1, Ordering::SeqCst);
    }
    if since == Some(LAST_MODIFIED) {
        seen.not_modified.fetch_add(1, Ordering::SeqCst);
        return StatusCode::NOT_MODIFIED.into_response();
    }
    ([(header::LAST_MODIFIED, LAST_MODIFIED)], body).into_response()
}

async fn stub() -> (String, Arc<Seen>) {
    let seen = Arc::new(Seen::default());
    let app = Router::new()
        .route(
            "/2026/events",
            get(|State(s): State<Arc<Seen>>, uri: Uri, h: HeaderMap| async move {
                answer(&s, &uri, &h, EVENTS).await
            }),
        )
        .route(
            "/2026/teams",
            get(|State(s): State<Arc<Seen>>, uri: Uri, h: HeaderMap| async move {
                answer(&s, &uri, &h, TEAMS).await
            }),
        )
        .route(
            "/2026/schedule/{code}/qual/hybrid",
            get(
                |State(s): State<Arc<Seen>>, Path(_c): Path<String>, uri: Uri, h: HeaderMap| async move {
                    answer(&s, &uri, &h, SCHEDULE).await
                },
            ),
        )
        .route(
            "/2026/rankings/{code}",
            get(
                |State(s): State<Arc<Seen>>, Path(_c): Path<String>, uri: Uri, h: HeaderMap| async move {
                    answer(&s, &uri, &h, RANKINGS).await
                },
            ),
        )
        .route(
            "/2026/scores/{code}/qual",
            get(
                |State(s): State<Arc<Seen>>, Path(_c): Path<String>, uri: Uri, h: HeaderMap| async move {
                    answer(&s, &uri, &h, SCORES).await
                },
            ),
        )
        .with_state(seen.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://{addr}"), seen)
}

async fn repo() -> SqliteRepo {
    let repo = SqliteRepo::connect("sqlite::memory:").expect("connect");
    tt_repo_sqlite::migrate::apply(repo.pool())
        .await
        .expect("migrate");
    repo
}

fn client(base: &str, key: &str) -> FtcClient {
    FtcClient::new("user", key, Uplink::new())
        .expect("client")
        .with_base_url(base)
}

#[tokio::test]
async fn a_pull_imports_the_event_its_teams_matches_and_rankings() {
    let (base, seen) = stub().await;
    let repo = repo().await;

    let pulled = ftc::pull(&repo, &client(&base, "key")).await.expect("pull");

    assert!(pulled.changed);
    assert_eq!(pulled.event_key, "2026usalatq");
    assert_eq!(pulled.event_name, "Athens Qualifier");
    assert_eq!((pulled.teams, pulled.matches, pulled.played), (11, 6, 1));
    assert_eq!(pulled.surrogates.len(), 1);
    assert_eq!(pulled.surrogates[0].describe(), "Q6 blue1: 19459");
    assert_eq!(pulled.rankings, 11);
    assert_eq!(pulled.score_sheets, 1);
    assert!(pulled.problems.is_empty(), "{:?}", pulled.problems);
    let (keys, roster) = pulled.tidy.expect("a schedule to tidy to");
    assert_eq!(keys.len(), 6);
    assert_eq!(roster.len(), 11);

    let event = repo.event("2026usalatq").await.unwrap().expect("event");
    assert_eq!(event.timezone.as_deref(), Some("America/Chicago"));
    assert_eq!(event.start_date.unwrap().to_string(), "2026-10-24");
    assert_eq!(repo.event_teams("2026usalatq").await.unwrap().len(), 11);
    let matches = repo.event_matches("2026usalatq").await.unwrap();
    assert_eq!(matches.len(), 6);
    let q1 = &matches[0];
    assert_eq!(q1.key, "2026usalatq_qm1");
    assert!(q1.played);
    assert_eq!((q1.red_score, q1.blue_score), (Some(51), Some(40)));
    assert_eq!(q1.red, [Some(7842), Some(18664)]);
    assert!(
        matches[1..]
            .iter()
            .all(|m| !m.played && m.red_score.is_none())
    );

    let stats = repo.team_stats("2026usalatq", 7842).await.unwrap().unwrap();
    assert_eq!(
        (stats.rank, stats.wins, stats.losses),
        (Some(1), Some(1), Some(0))
    );
    assert_eq!(stats.qual_average, Some(51.0));
    // One played match is 2 equations for 4 robots: no OPR, rather than one
    // of many that fit.
    assert_eq!(stats.opr, None);
    assert_eq!(stats.dpr, None);
    assert_eq!(
        pulled.rated,
        Some(ftc::Rated {
            teams: 0,
            played: 1
        })
    );

    // Every raw body is in the log as 'ftc', and the matches as the CSV
    // import writes them.
    for path in [
        "/2026/events?eventCode=USALATQ",
        "/2026/teams?eventCode=USALATQ",
        "/2026/schedule/USALATQ/qual/hybrid",
        "/2026/rankings/USALATQ",
        "/2026/scores/USALATQ/qual",
    ] {
        let logged = repo.latest_upstream("ftc", path).await.unwrap();
        let logged = logged.unwrap_or_else(|| panic!("{path} is not logged"));
        assert_eq!(logged.entry.etag.as_deref(), Some(LAST_MODIFIED));
    }
    let matches = repo
        .latest_upstream("tba", "/event/2026usalatq/matches")
        .await
        .unwrap()
        .expect("the matches body");
    assert_eq!(matches.entry.via, "ftc");
    assert!(matches.entry.body.contains("frc35722"));
    assert_eq!(
        seen.conditional.load(Ordering::SeqCst),
        0,
        "nothing to compare with yet"
    );
}

#[tokio::test]
async fn a_second_pull_with_nothing_new_is_all_304s_and_changes_nothing() {
    let (base, seen) = stub().await;
    let repo = repo().await;
    let client = client(&base, "key");
    ftc::pull(&repo, &client).await.expect("first pull");
    let logged = repo.upstream_since(0, 1000).await.unwrap().len();
    let first = seen.requests.load(Ordering::SeqCst);

    let again = ftc::pull(&repo, &client).await.expect("second pull");

    assert!(!again.changed, "{again:?}");
    assert!(again.tidy.is_none());
    let asked = seen.requests.load(Ordering::SeqCst) - first;
    assert_eq!(asked, 5);
    assert_eq!(seen.not_modified.load(Ordering::SeqCst), 5);
    assert_eq!(seen.conditional.load(Ordering::SeqCst), 5);
    assert!(
        !seen.only_modified.load(Ordering::SeqCst),
        "never FMS-OnlyModifiedSince"
    );
    assert_eq!(repo.upstream_since(0, 1000).await.unwrap().len(), logged);
    assert_eq!(repo.event_matches("2026usalatq").await.unwrap().len(), 6);
}

#[tokio::test]
async fn a_refused_key_is_a_401_and_stores_nothing() {
    let (base, seen) = stub().await;
    let repo = repo().await;

    let error = ftc::pull(&repo, &client(&base, "wrong"))
        .await
        .expect_err("401");

    assert!(
        matches!(error, UpstreamError::Status { status: 401, .. }),
        "{error}"
    );
    assert_eq!(
        seen.requests.load(Ordering::SeqCst),
        1,
        "a 401 is not retried"
    );
    assert!(repo.list_events().await.unwrap().is_empty());
    assert!(repo.upstream_since(0, 10).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_503_with_a_short_retry_after_is_waited_out() {
    let (base, seen) = stub().await;
    *seen.busy.lock().unwrap() = Some(("/schedule/", "1", 1));
    let repo = repo().await;

    let pulled = ftc::pull(&repo, &client(&base, "key")).await.expect("pull");

    assert_eq!(pulled.matches, 6);
    assert_eq!(
        seen.requests.load(Ordering::SeqCst),
        6,
        "5 paths and one retry"
    );
}

#[tokio::test]
async fn a_503_asking_for_minutes_stops_the_pull_and_the_next_pull_stores_everything() {
    let (base, seen) = stub().await;
    *seen.busy.lock().unwrap() = Some(("/rankings/", "120", 1));
    let repo = repo().await;
    let client = client(&base, "key");

    let error = ftc::pull(&repo, &client).await.expect_err("busy");

    assert!(
        matches!(
            error,
            UpstreamError::Busy {
                status: 503,
                retry_after: Some(120),
                ..
            }
        ),
        "{error}"
    );
    assert!(repo.list_events().await.unwrap().is_empty());
    assert!(
        repo.upstream_since(0, 10).await.unwrap().is_empty(),
        "nothing logged, so nothing will come back as a 304"
    );

    let pulled = ftc::pull(&repo, &client).await.expect("the next pull");
    assert!(pulled.changed);
    assert_eq!(seen.not_modified.load(Ordering::SeqCst), 0);
    assert_eq!(repo.event_matches("2026usalatq").await.unwrap().len(), 6);
    assert_eq!(pulled.rankings, 11);
}
