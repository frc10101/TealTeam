//! Conditional requests to TBA (I9): a stub that honours `If-None-Match` the
//! way TBA does, and counts how many full bodies it had to send.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use tt_repo::Repo;
use tt_repo_sqlite::SqliteRepo;
use tt_upstream::Uplink;
use tt_upstream::sync;
use tt_upstream::tba::TbaClient;

const ONE_MATCH: &str = r#"[
  {"key":"2026mabil_qm1","comp_level":"qm","set_number":1,"match_number":1,
   "alliances":{"red":{"score":-1,"team_keys":["frc10101","frc254","frc1"]},
                "blue":{"score":-1,"team_keys":["frc2","frc3","frc4"]}}}
]"#;

const PLAYED: &str = r#"[
  {"key":"2026mabil_qm1","comp_level":"qm","set_number":1,"match_number":1,
   "alliances":{"red":{"score":88,"team_keys":["frc10101","frc254","frc1"]},
                "blue":{"score":74,"team_keys":["frc2","frc3","frc4"]}}}
]"#;

/// What the stub serves, and what it has seen.
#[derive(Default)]
struct Upstream {
    /// Which payload is current: 0 is `ONE_MATCH`, 1 is `PLAYED`.
    version: AtomicUsize,
    /// 200s sent -- each one a full body over the tether.
    full: AtomicUsize,
    /// 304s sent.
    not_modified: AtomicUsize,
    /// Answer 304 whatever was asked, the way a broken proxy might.
    always_304: std::sync::atomic::AtomicBool,
}

async fn matches(State(up): State<Arc<Upstream>>, headers: HeaderMap) -> Response {
    let version = up.version.load(Ordering::SeqCst);
    let etag = format!("W/\"v{version}\"");
    let asked = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok());
    if up.always_304.load(Ordering::SeqCst) || asked == Some(etag.as_str()) {
        up.not_modified.fetch_add(1, Ordering::SeqCst);
        return StatusCode::NOT_MODIFIED.into_response();
    }
    up.full.fetch_add(1, Ordering::SeqCst);
    let body = if version == 0 { ONE_MATCH } else { PLAYED };
    ([(header::ETAG, etag)], body).into_response()
}

async fn stub() -> (String, Arc<Upstream>) {
    let up = Arc::new(Upstream::default());
    let app = Router::new()
        .route("/event/{key}/matches", get(matches))
        .with_state(up.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://{addr}"), up)
}

fn client(base: &str, uplink: &Uplink) -> TbaClient {
    TbaClient::new("key", uplink.clone())
        .expect("tba client")
        .with_base_url(base)
}

#[tokio::test]
async fn an_unchanged_resource_is_not_downloaded_again() {
    let (base, up) = stub().await;
    let tba = client(&base, &Uplink::new());

    let first = tba.matches("2026mabil").await.expect("first");
    let second = tba.matches("2026mabil").await.expect("second");

    assert_eq!(first.len(), 1);
    assert_eq!(second.len(), 1, "the 304 still answers with the data");
    assert_eq!(up.full.load(Ordering::SeqCst), 1);
    assert_eq!(up.not_modified.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_changed_resource_replaces_what_was_kept() {
    let (base, up) = stub().await;
    let tba = client(&base, &Uplink::new());

    tba.matches("2026mabil").await.expect("first");
    up.version.store(1, Ordering::SeqCst);
    let changed = tba.matches("2026mabil").await.expect("changed");
    let again = tba.matches("2026mabil").await.expect("again");

    assert_eq!(changed[0].red_score(), Some(88));
    assert_eq!(again[0].red_score(), Some(88), "not the stale body");
    assert_eq!(up.full.load(Ordering::SeqCst), 2);
    assert_eq!(up.not_modified.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn clones_share_what_they_have_seen() {
    // The loop and the manual sync hold clones of one client.
    let (base, up) = stub().await;
    let tba = client(&base, &Uplink::new());
    let other = tba.clone();

    tba.matches("2026mabil").await.expect("first");
    other.matches("2026mabil").await.expect("clone");

    assert_eq!(up.full.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_client_starting_cold_revalidates_what_its_log_remembers() {
    // A browser's client lives as long as its page (S4): it seeds the cache
    // from its own upstream log, so the first request is already conditional.
    let (base, up) = stub().await;
    let tba = client(&base, &Uplink::new());
    tba.remember("/event/2026mabil/matches", "W/\"v0\"", ONE_MATCH);

    let matches = tba.matches("2026mabil").await.expect("revalidated");

    assert_eq!(matches.len(), 1, "the remembered body answers the 304");
    assert_eq!(up.full.load(Ordering::SeqCst), 0);
    assert_eq!(up.not_modified.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_remembered_tag_that_is_stale_fetches_the_new_body() {
    let (base, up) = stub().await;
    up.version.store(1, Ordering::SeqCst);
    let tba = client(&base, &Uplink::new());
    tba.remember("/event/2026mabil/matches", "W/\"v0\"", ONE_MATCH);

    let matches = tba.matches("2026mabil").await.expect("fetched");

    assert_eq!(matches[0].red_score(), Some(88), "not the remembered body");
    assert_eq!(up.full.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_tag_that_cannot_be_a_header_is_not_remembered() {
    let (base, up) = stub().await;
    let tba = client(&base, &Uplink::new());
    tba.remember("/event/2026mabil/matches", "bad\ntag", ONE_MATCH);

    tba.matches("2026mabil").await.expect("fetched");

    assert_eq!(up.full.load(Ordering::SeqCst), 1);
    assert_eq!(up.not_modified.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_304_nobody_asked_for_is_an_error_not_empty_data() {
    let (base, up) = stub().await;
    up.always_304.store(true, Ordering::SeqCst);
    let uplink = Uplink::new();
    let tba = client(&base, &uplink);

    let error = tba
        .matches("2026mabil")
        .await
        .expect_err("nothing to reuse");
    assert!(error.to_string().contains("304"), "{error}");
    assert!(uplink.snapshot().last_api_error.is_some());
}

#[tokio::test]
async fn an_unchanged_schedule_is_still_a_sync_and_counts_as_contact() {
    // Freshness (I12) is "when did we last hear from TBA", not "when did the
    // data last change": a 304 is TBA saying the stored copy is current.
    let (base, up) = stub().await;
    let repo = SqliteRepo::connect("sqlite::memory:").expect("connect");
    tt_repo_sqlite::migrate::apply(repo.pool())
        .await
        .expect("migrate");
    let event = tt_core::records::Event {
        key: "2026mabil".into(),
        name: "Greater Boston".into(),
        location: None,
        timezone: None,
        start_date: None,
        end_date: None,
        event_code: None,
        event_type: None,
        district_key: None,
        week: None,
    };
    repo.upsert_event(&event, chrono::Utc::now())
        .await
        .expect("event");
    let uplink = Uplink::new();
    let tba = client(&base, &uplink);

    sync::sync_matches(&repo, &tba, "2026mabil")
        .await
        .expect("first");
    let before = uplink.snapshot().last_api_success.expect("success");
    let report = sync::sync_matches(&repo, &tba, "2026mabil")
        .await
        .expect("second");

    assert_eq!(up.not_modified.load(Ordering::SeqCst), 1);
    assert_eq!(report.matches, 1);
    assert!(uplink.snapshot().last_api_success.expect("success") >= before);
    assert!(report.problems.is_empty(), "{:?}", report.problems);
}
