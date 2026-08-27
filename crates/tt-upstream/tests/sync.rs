//! End-to-end sync: a stub upstream, the real clients, the real SQLite repo.
//!
//! These are the tests that prove the whole path — HTTP, deserialization,
//! fallback extraction, and the upsert — rather than each half in isolation.
//! Every payload here is shaped like the real thing, including the parts that
//! cost the retired implementation real data: dynamically named component OPRs
//! and rankings whose primitives are null.

use axum::Router;
use axum::routing::get;
use tt_repo::Repo;
use tt_repo_sqlite::SqliteRepo;
use tt_upstream::Uplink;
use tt_upstream::first::{EventFilters, FirstClient};
use tt_upstream::sync;
use tt_upstream::tba::TbaClient;

// ── Canned upstream payloads ────────────────────────────────────────────────

const FIRST_EVENTS: &str = r#"{"Events":[{
  "code":"MABIL","name":"Greater Boston Regional","venue":"Reggie Lewis Center",
  "city":"Boston","stateprov":"MA","country":"USA","timezone":"America/New_York",
  "dateStart":"2026-03-12T00:00:00","dateEnd":"2026-03-15T00:00:00","weekNumber":3
}]}"#;

const FIRST_TEAMS: &str = r#"{"teams":[
  {"teamNumber":10101,"nameShort":"Teal Team","nameFull":"Sponsors & School",
   "schoolName":"Example High","city":"Boston","stateProv":"MA","country":"USA","rookieYear":2024},
  {"teamNumber":254,"nameShort":"The Cheesy Poofs","city":"San Jose","stateProv":"CA","country":"USA"}
]}"#;

/// 2026 shape: component names are season-specific, not fixed fields.
const TBA_COPRS: &str = r#"{
  "totalAutoPoints":    {"frc10101": 20.1572, "frc254": 31.0},
  "totalTeleopPoints":  {"frc10101": 84.3,    "frc254": 96.5},
  "totalEndgamePoints": {"frc10101": 12.9,    "frc254": 15.25}
}"#;

const TBA_OPRS: &str = r#"{
  "oprs":  {"frc10101": 55.5, "frc254": 88.25},
  "dprs":  {"frc10101": 20.1, "frc254": 15.0},
  "ccwms": {"frc10101": 35.4, "frc254": 73.25}
}"#;

/// 2026 shape: qual_points / total_points are null; the numbers live in the
/// sort_orders and extra_stats arrays.
const TBA_RANKINGS: &str = r#"{"rankings":[
  {"team_key":"frc254","rank":1,"matches_played":12,"dq":0,
   "record":{"wins":11,"losses":1,"ties":0},
   "qual_average":null,"qual_points":null,"total_points":null,
   "sort_orders":[18.0,171.0],"extra_stats":[18.0]},
  {"team_key":"frc10101","rank":7,"matches_played":12,"dq":1,
   "record":{"wins":7,"losses":4,"ties":1},
   "qual_average":null,"qual_points":null,"total_points":null,
   "sort_orders":[12.0,143.5],"extra_stats":[12.0]}
]}"#;

const TBA_MATCHES: &str = r#"[
  {"key":"2026mabil_qm1","comp_level":"qm","set_number":1,"match_number":1,
   "time":1773500000,"actual_time":1773500400,"score_breakdown":{"red":{},"blue":{}},
   "alliances":{"red":{"score":88,"team_keys":["frc10101","frc254","frc1"]},
                "blue":{"score":74,"team_keys":["frc2","frc3","frc4"]}}},
  {"key":"2026mabil_qm2","comp_level":"qm","set_number":1,"match_number":2,
   "time":1773600000,
   "alliances":{"red":{"score":-1,"team_keys":["frc2","frc10101","frc5"]},
                "blue":{"score":-1,"team_keys":["frc6","frc7","frc8"]}}},
  {"key":"2026mabil_sf2m1","comp_level":"sf","set_number":2,"match_number":1,
   "alliances":{"red":{"score":-1,"team_keys":[]},"blue":{"score":-1,"team_keys":[]}}}
]"#;

/// Start a stub upstream on an ephemeral port. Returns its base URL.
async fn stub_server() -> String {
    let app = Router::new()
        .route("/2026/events", get(|| async { FIRST_EVENTS }))
        .route("/2026/teams", get(|| async { FIRST_TEAMS }))
        .route("/event/{key}/oprs", get(|| async { TBA_OPRS }))
        .route("/event/{key}/coprs", get(|| async { TBA_COPRS }))
        .route("/event/{key}/rankings", get(|| async { TBA_RANKINGS }))
        .route("/event/{key}/matches", get(|| async { TBA_MATCHES }));

    // 127.0.0.1 is deliberate: the client skips its internet probe for local
    // addresses, so these tests never touch the network.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    format!("http://{addr}")
}

async fn repo() -> SqliteRepo {
    let repo = SqliteRepo::connect("sqlite::memory:").expect("connect");
    tt_repo_sqlite::migrate::apply(repo.pool())
        .await
        .expect("migrate");
    repo
}

fn clients(base: &str, uplink: &Uplink) -> (FirstClient, TbaClient) {
    let first = FirstClient::new("user", "token", 2026, uplink.clone())
        .expect("first client")
        .with_base_url(base);
    let tba = TbaClient::new("key", uplink.clone())
        .expect("tba client")
        .with_base_url(base);
    (first, tba)
}

#[tokio::test]
async fn a_bulk_load_lands_events_teams_matches_and_stats() {
    let base = stub_server().await;
    let repo = repo().await;
    let uplink = Uplink::new();
    let (first, tba) = clients(&base, &uplink);

    let report = sync::bulk_load(&repo, &first, Some(&tba), &EventFilters::all(), &uplink)
        .await
        .expect("bulk load");

    assert!(
        report.problems.is_empty(),
        "unexpected: {:?}",
        report.problems
    );
    assert_eq!(report.events, 1);
    assert_eq!(report.teams, 2);
    assert_eq!(report.event_teams, 2);
    assert_eq!(report.matches, 3);
    assert_eq!(report.stats, 2);

    // The sync is recorded, which is what the freshness badges read.
    assert!(uplink.snapshot().last_sync.is_some());
}

#[tokio::test]
async fn the_event_is_stored_with_its_derived_key_and_location() {
    let base = stub_server().await;
    let repo = repo().await;
    let uplink = Uplink::new();
    let (first, _) = clients(&base, &uplink);

    sync::sync_events(&repo, &first, &EventFilters::all())
        .await
        .expect("sync");

    let event = repo
        .event("2026mabil")
        .await
        .expect("query")
        .expect("event");
    assert_eq!(event.name, "Greater Boston Regional");
    assert_eq!(
        event.location.as_deref(),
        Some("Reggie Lewis Center, Boston, MA, USA")
    );
    assert_eq!(event.timezone.as_deref(), Some("America/New_York"));
    assert_eq!(
        event.start_date.map(|d| d.to_string()).as_deref(),
        Some("2026-03-12")
    );
    assert_eq!(event.week, Some(3));
}

#[tokio::test]
async fn component_oprs_survive_their_dynamic_names() {
    // Reading fixed field names here is what left auto_opr null for every team
    // in the retired implementation.
    let base = stub_server().await;
    let repo = repo().await;
    let uplink = Uplink::new();
    let (first, tba) = clients(&base, &uplink);

    sync::sync_events(&repo, &first, &EventFilters::all())
        .await
        .expect("events");
    sync::sync_stats(&repo, &tba, "2026mabil")
        .await
        .expect("stats");

    let stats = repo
        .team_stats("2026mabil", 10101)
        .await
        .expect("query")
        .expect("stats");

    assert_eq!(stats.auto_opr, Some(20.1572));
    assert_eq!(stats.teleop_opr, Some(84.3));
    assert_eq!(stats.endgame_opr, Some(12.9));
    assert_eq!(stats.opr, Some(55.5));
    assert_eq!(stats.dpr, Some(20.1));
}

#[tokio::test]
async fn ranking_points_come_out_of_the_arrays_when_the_primitives_are_null() {
    // The 2026 schema leaves qual_points and total_points null. Direct access
    // yields zeros; a team ranked 7th showing qual_points=0 is the symptom.
    let base = stub_server().await;
    let repo = repo().await;
    let uplink = Uplink::new();
    let (first, tba) = clients(&base, &uplink);

    sync::sync_events(&repo, &first, &EventFilters::all())
        .await
        .expect("events");
    sync::sync_stats(&repo, &tba, "2026mabil")
        .await
        .expect("stats");

    let stats = repo.team_stats("2026mabil", 10101).await.unwrap().unwrap();
    assert_eq!(stats.rank, Some(7));
    assert_eq!(stats.qual_average, Some(12.0)); // sort_orders[0]
    assert_eq!(stats.avg_match_points, Some(143.5)); // sort_orders[1]
    assert_eq!(stats.qual_points, Some(12));
    assert_eq!(stats.total_points, Some(12)); // extra_stats[0]
    assert_eq!(stats.record_line(), "7W 4L 1T");
    assert_eq!(stats.dq_count, Some(1));
}

#[tokio::test]
async fn matches_record_scores_only_when_played() {
    let base = stub_server().await;
    let repo = repo().await;
    let uplink = Uplink::new();
    let (first, tba) = clients(&base, &uplink);

    sync::sync_events(&repo, &first, &EventFilters::all())
        .await
        .expect("events");
    sync::sync_matches(&repo, &tba, "2026mabil")
        .await
        .expect("matches");

    let matches = repo.event_matches("2026mabil").await.expect("query");
    assert_eq!(matches.len(), 3);

    let played = &matches[0];
    assert_eq!(played.label(), "Q1");
    assert!(played.played);
    assert_eq!(played.red_score, Some(88));
    assert_eq!(played.winner.as_deref(), Some("red"));
    assert_eq!(played.red, [Some(10101), Some(254), Some(1)]);

    let upcoming = &matches[1];
    assert!(!upcoming.played);
    // TBA's -1 sentinel must never reach storage as a score.
    assert_eq!(upcoming.red_score, None);
    assert_eq!(upcoming.winner, None);
}

#[tokio::test]
async fn playoff_matches_keep_their_set_number_rather_than_being_folded() {
    let base = stub_server().await;
    let repo = repo().await;
    let uplink = Uplink::new();
    let (first, tba) = clients(&base, &uplink);

    sync::sync_events(&repo, &first, &EventFilters::all())
        .await
        .expect("events");
    sync::sync_matches(&repo, &tba, "2026mabil")
        .await
        .expect("matches");

    let matches = repo.event_matches("2026mabil").await.expect("query");
    // Ordering puts qualifications before playoffs.
    let sf = matches.last().expect("a semifinal");
    assert_eq!(sf.label(), "SF1");
    assert_eq!(sf.set_number, 2);
    assert_eq!(sf.match_number, 1);
}

#[tokio::test]
async fn a_team_schedule_contains_only_that_teams_matches() {
    let base = stub_server().await;
    let repo = repo().await;
    let uplink = Uplink::new();
    let (first, tba) = clients(&base, &uplink);

    sync::sync_events(&repo, &first, &EventFilters::all())
        .await
        .expect("events");
    sync::sync_matches(&repo, &tba, "2026mabil")
        .await
        .expect("matches");

    let ours = repo.team_matches("2026mabil", 10101).await.expect("query");
    assert_eq!(ours.len(), 2);
    assert_eq!(ours[0].alliance_of(10101), Some("red"));
    assert_eq!(ours[0].partners_of(10101), vec![254, 1]);
    // Q2 puts us in a different slot; partners must follow.
    assert_eq!(ours[1].partners_of(10101), vec![2, 5]);
}

#[tokio::test]
async fn syncing_twice_updates_rather_than_duplicating() {
    // Real upserts, which the retired schema could not do because it lacked the
    // unique constraints.
    let base = stub_server().await;
    let repo = repo().await;
    let uplink = Uplink::new();
    let (first, tba) = clients(&base, &uplink);

    for _ in 0..2 {
        sync::sync_events(&repo, &first, &EventFilters::all())
            .await
            .expect("events");
        sync::sync_matches(&repo, &tba, "2026mabil")
            .await
            .expect("matches");
        sync::sync_stats(&repo, &tba, "2026mabil")
            .await
            .expect("stats");
    }

    assert_eq!(repo.list_events().await.unwrap().len(), 1);
    assert_eq!(repo.event_teams("2026mabil").await.unwrap().len(), 2);
    assert_eq!(repo.event_matches("2026mabil").await.unwrap().len(), 3);
    assert_eq!(repo.event_stats("2026mabil").await.unwrap().len(), 2);
}

#[tokio::test]
async fn event_stats_come_back_ranked() {
    let base = stub_server().await;
    let repo = repo().await;
    let uplink = Uplink::new();
    let (first, tba) = clients(&base, &uplink);

    sync::sync_events(&repo, &first, &EventFilters::all())
        .await
        .expect("events");
    sync::sync_stats(&repo, &tba, "2026mabil")
        .await
        .expect("stats");

    let stats = repo.event_stats("2026mabil").await.expect("query");
    assert_eq!(stats[0].team_number, 254, "rank 1 first");
    assert_eq!(stats[1].team_number, 10101);
}

#[tokio::test]
async fn active_events_pick_up_a_running_event() {
    let base = stub_server().await;
    let repo = repo().await;
    let uplink = Uplink::new();
    let (first, _) = clients(&base, &uplink);
    sync::sync_events(&repo, &first, &EventFilters::all())
        .await
        .expect("events");

    let during = chrono::NaiveDate::from_ymd_opt(2026, 3, 13).unwrap();
    assert_eq!(repo.active_events(during, 1).await.unwrap().len(), 1);

    let day_before = chrono::NaiveDate::from_ymd_opt(2026, 3, 11).unwrap();
    assert_eq!(
        repo.active_events(day_before, 1).await.unwrap().len(),
        1,
        "an event starting tomorrow is imminent"
    );

    let long_before = chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    assert!(repo.active_events(long_before, 1).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_missing_tba_key_degrades_to_events_only() {
    let base = stub_server().await;
    let repo = repo().await;
    let uplink = Uplink::new();
    let (first, _) = clients(&base, &uplink);

    let report = sync::bulk_load(&repo, &first, None, &EventFilters::all(), &uplink)
        .await
        .expect("bulk load");

    assert_eq!(report.events, 1);
    assert_eq!(report.matches, 0);
    assert!(
        report
            .problems
            .iter()
            .any(|p| p.contains("TBA key not configured")),
        "the operator should be told why there are no matches"
    );
}

#[tokio::test]
async fn upstream_failures_are_reported_not_swallowed() {
    // A server that answers events but 500s everything else.
    let app = Router::new()
        .route("/2026/events", get(|| async { FIRST_EVENTS }))
        .route(
            "/2026/teams",
            get(|| async { (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "boom") }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let repo = repo().await;
    let uplink = Uplink::new();
    let first = FirstClient::new("u", "t", 2026, uplink.clone())
        .unwrap()
        .with_base_url(format!("http://{addr}"));

    let report = sync::sync_events(&repo, &first, &EventFilters::all())
        .await
        .expect("the run continues past a roster failure");

    // The event still landed; only its roster did not.
    assert_eq!(report.events, 1);
    assert_eq!(report.teams, 0);
    assert!(report.problems.iter().any(|p| p.contains("fetching teams")));
    assert!(uplink.snapshot().last_api_error.is_some());
}
