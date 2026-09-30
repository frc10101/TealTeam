//! Real upstream payloads, recorded, through the real clients (Q2).
//!
//! `tests/fixtures/` holds responses recorded from FIRST and TBA on
//! 2026-09-30, bodies only. Match lists keep Q1-Q3 and every playoff match;
//! nothing else is edited. Three seasons' shapes:
//!
//!   * **2026 Magnolia Regional** (`2026mslr`, FIRST `MSLR`): team 10101's
//!     own event, the double-elimination bracket, and component OPRs with
//!     two auto points components.
//!   * **2019 Bayou Regional** (`2019lake`): quarterfinals, best-of-three
//!     sets, and ranking columns for cargo and hatch panels. FIRST's 2019
//!     events endpoint answered 500 on the day, so FIRST's prior season is
//!     2025 (`LAKE`).
//!   * **2026 Arizona Robotics League Championship** (`2026azrl`), before it
//!     had happened: every list empty.
//!   * **2026 Milstein Division** (FIRST `MILSTEIN`, TBA `2026mil`): 74
//!     teams, over two pages.
//!   * **TBA's 2026 event list** (`tba/events_2026.json`), cut to a dozen
//!     events whose TBA key and FIRST code disagree, or agree (I15).
//!
//! Upstream drifts every season. When a sync breaks on a new shape, record
//! it here next to these.

use std::collections::HashMap;

use axum::Router;
use axum::extract::{Path, Query};
use axum::http::StatusCode;
use axum::routing::get;
use tt_core::matches::CompLevel;
use tt_core::records::{Event, Team};
use tt_repo::Repo;
use tt_repo_sqlite::SqliteRepo;
use tt_upstream::Uplink;
use tt_upstream::first::{EventFilters, FirstClient};
use tt_upstream::sync;
use tt_upstream::tba::TbaClient;

fn fixture(path: &str) -> Option<String> {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/{path}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .ok()
}

fn served(path: String) -> Result<String, StatusCode> {
    fixture(&path).ok_or(StatusCode::NOT_FOUND)
}

/// FIRST and TBA on one loopback port, answering from `fixtures/`.
/// `2026null` is an event TBA has nothing for: every body is `null`.
async fn recorded_upstream() -> String {
    let app = Router::new()
        .route(
            "/event/{key}/{what}",
            get(|Path((key, what)): Path<(String, String)>| async move {
                if key == "2026null" {
                    return Ok("null".to_string());
                }
                served(format!("tba/{key}_{what}.json"))
            }),
        )
        .route(
            "/{season}/{what}",
            get(
                |Path((season, what)): Path<(String, String)>,
                 Query(q): Query<HashMap<String, String>>| async move {
                    // TBA's `/events/{year}` shares FIRST's two-segment shape.
                    if season == "events" {
                        return served(format!("tba/events_{what}.json"));
                    }
                    let code = q.get("eventCode").cloned().unwrap_or_default();
                    let page = match q.get("page") {
                        Some(n) => format!("_p{n}"),
                        None if code == "MILSTEIN" && what == "teams" => "_p1".into(),
                        None => String::new(),
                    };
                    served(format!("first/{season}_{what}_{code}{page}.json"))
                },
            ),
        );
    // Loopback, so the clients skip their internet probe.
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

fn first(base: &str, season: i32) -> FirstClient {
    FirstClient::new("user", "token", season, Uplink::new())
        .expect("client")
        .with_base_url(base)
}

fn tba(base: &str) -> TbaClient {
    TbaClient::new("key", Uplink::new())
        .expect("client")
        .with_base_url(base)
}

fn only(code: &str) -> EventFilters {
    EventFilters {
        event_code: Some(code.into()),
        ..EventFilters::default()
    }
}

/// An event FIRST could not give us, stored by hand, with `teams` on it.
async fn stored_event(repo: &SqliteRepo, key: &str, teams: &[i32]) {
    let now = chrono::Utc::now();
    let event = Event {
        key: key.into(),
        name: key.into(),
        location: None,
        timezone: None,
        start_date: None,
        end_date: None,
        event_code: None,
        event_type: None,
        district_key: None,
        week: None,
    };
    repo.upsert_event(&event, now).await.expect("event");
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
        repo.upsert_team(&team, now).await.expect("team");
        repo.link_event_team(key, number, now).await.expect("link");
    }
}

fn close(actual: Option<f64>, expected: f64) -> bool {
    actual.is_some_and(|a| (a - expected).abs() < 1e-6)
}

// ── 2026, this season ───────────────────────────────────────────────────────

#[tokio::test]
async fn this_seasons_event_syncs_whole_from_both_apis() {
    let base = recorded_upstream().await;
    let repo = repo().await;

    let events = sync::sync_events(&repo, &first(&base, 2026), Some(&tba(&base)), &only("MSLR"))
        .await
        .expect("events");
    assert!(events.problems.is_empty(), "{:?}", events.problems);
    let event = repo
        .event("2026mslr")
        .await
        .unwrap()
        .expect("keyed for TBA");
    assert_eq!(event.name, "Magnolia Regional");
    assert_eq!(event.event_type.as_deref(), Some("Regional"));
    assert_eq!(event.week, Some(3));
    assert_eq!(
        event.timezone.as_deref(),
        Some("America/Chicago"),
        "FIRST sent \"Central Standard Time\""
    );
    assert_eq!(repo.event_teams("2026mslr").await.unwrap().len(), 46);

    let matches = sync::sync_matches(&repo, &tba(&base), "2026mslr")
        .await
        .expect("matches");
    assert!(matches.problems.is_empty(), "{:?}", matches.problems);
    let stored = repo.event_matches("2026mslr").await.unwrap();
    let labels: Vec<String> = stored.iter().map(|m| m.label()).collect();
    assert_eq!(
        labels,
        [
            "Q1", "Q2", "Q3", "SF1", "SF2", "SF3", "SF4", "SF5", "SF6", "SF7", "SF8", "SF9",
            "SF10", "SF11", "SF12", "SF13", "F1", "F2"
        ],
        "in bracket order, each its own"
    );
    assert!(stored.iter().all(|m| m.played && m.actual_at.is_some()));

    let stats = sync::sync_stats(&repo, &tba(&base), "2026mslr")
        .await
        .expect("stats");
    assert!(stats.problems.is_empty(), "{:?}", stats.problems);
    let teal = repo
        .team_stats("2026mslr", 10101)
        .await
        .unwrap()
        .expect("10101 was there");
    assert_eq!(teal.rank, Some(18));
    assert_eq!(
        (teal.wins, teal.losses, teal.ties),
        (Some(5), Some(4), Some(0))
    );
    assert!(close(teal.qual_average, 2.0));
    assert!(close(teal.avg_match_points, 81.33), "the Avg Match column");
    assert_eq!(teal.total_points, Some(18), "Total Ranking Points");
    assert!(close(teal.opr, 5.645113090337809));
    assert!(
        close(teal.auto_opr, 2.504123619261337),
        "totalAutoPoints, not autoTowerPoints (0.0): {:?}",
        teal.auto_opr
    );
    assert!(close(teal.teleop_opr, 1.0981084638870051));
    assert!(close(teal.endgame_opr, 0.6232793978391646));
}

#[tokio::test]
async fn a_championship_division_roster_is_every_page() {
    let base = recorded_upstream().await;
    let teams = first(&base, 2026)
        .event_teams("MILSTEIN")
        .await
        .expect("teams");
    assert_eq!(teams.len(), 74, "65 on the first page, 9 on the second");
    let mut numbers: Vec<i32> = teams.iter().map(|t| t.team_number).collect();
    numbers.sort_unstable();
    numbers.dedup();
    assert_eq!(numbers.len(), 74);
}

#[tokio::test]
async fn a_championship_division_is_stored_under_tbas_key_not_firsts_code() {
    let base = recorded_upstream().await;
    let repo = repo().await;

    let report = sync::sync_events(
        &repo,
        &first(&base, 2026),
        Some(&tba(&base)),
        &only("MILSTEIN"),
    )
    .await
    .expect("events");
    assert!(report.problems.is_empty(), "{:?}", report.problems);
    let events = repo.list_events().await.unwrap();
    let keys: Vec<&str> = events.iter().map(|e| e.key.as_str()).collect();
    assert_eq!(keys, ["2026mil"], "not 2026milstein, which TBA 404s");
    assert_eq!(
        events[0].event_code.as_deref(),
        Some("milstein"),
        "FIRST's, for its own calls"
    );
    assert_eq!(repo.event_teams("2026mil").await.unwrap().len(), 74);
}

#[tokio::test]
async fn without_tba_the_key_is_built_from_firsts_code() {
    let base = recorded_upstream().await;
    let repo = repo().await;
    sync::sync_events(&repo, &first(&base, 2026), None, &only("MILSTEIN"))
        .await
        .expect("events");
    assert!(repo.event("2026milstein").await.unwrap().is_some());
}

#[tokio::test]
async fn a_tba_event_list_that_fails_is_a_problem_not_a_failed_sync() {
    let base = recorded_upstream().await;
    let repo = repo().await;
    // No fixture for 2025's list: the stub answers 404.
    let report = sync::sync_events(&repo, &first(&base, 2025), Some(&tba(&base)), &only("LAKE"))
        .await
        .expect("events");
    assert!(
        repo.event("2025lake").await.unwrap().is_some(),
        "FIRST's data still lands"
    );
    assert_eq!(report.problems.len(), 1, "{:?}", report.problems);
    assert!(report.problems[0].contains("TBA's event list is unavailable"));
}

#[tokio::test]
async fn every_response_lands_in_the_upstream_log_once() {
    // S1: the sync writes the tables as before; the log gets what it read.
    let base = recorded_upstream().await;
    let repo = std::sync::Arc::new(repo().await);
    let (recorder, journal) = tt_upstream::journal::journal(repo.clone());
    let first = first(&base, 2026).with_recorder(recorder.clone());
    let tba = tba(&base).with_recorder(recorder);

    for _ in 0..2 {
        sync::sync_events(&*repo, &first, Some(&tba), &only("MSLR"))
            .await
            .expect("events");
        sync::sync_event(&*repo, &tba, "2026mslr").await;
    }
    drop((first, tba));
    journal.await.expect("journal");

    let log = repo.upstream_since(0, 100).await.unwrap();
    let paths: Vec<(&str, &str)> = log
        .iter()
        .map(|e| (e.entry.api.as_str(), e.entry.path.as_str()))
        .collect();
    for expected in [
        ("first", "/2026/events?eventCode=MSLR"),
        ("first", "/2026/teams?eventCode=MSLR"),
        ("tba", "/events/2026"),
        ("tba", "/event/2026mslr/matches"),
        ("tba", "/event/2026mslr/rankings"),
        ("tba", "/event/2026mslr/oprs"),
        ("tba", "/event/2026mslr/coprs"),
    ] {
        assert_eq!(
            paths.iter().filter(|p| **p == expected).count(),
            1,
            "{expected:?} once, though synced twice: {paths:?}"
        );
    }
    let matches = log
        .iter()
        .find(|e| e.entry.path == "/event/2026mslr/matches")
        .unwrap();
    assert_eq!(matches.entry.via, "pi");
    assert_eq!(
        matches.entry.body.trim(),
        fixture("tba/2026mslr_matches.json").unwrap().trim(),
        "the body as received, to replay or pass on"
    );
    assert_eq!(
        repo.event_matches("2026mslr").await.unwrap().len(),
        18,
        "and the pages' tables as before"
    );
}

// ── Prior seasons ───────────────────────────────────────────────────────────

#[tokio::test]
async fn a_2019_bracket_keeps_its_quarterfinals_in_order() {
    let base = recorded_upstream().await;
    let repo = repo().await;
    stored_event(&repo, "2019lake", &[]).await;

    let report = sync::sync_matches(&repo, &tba(&base), "2019lake")
        .await
        .expect("matches");
    assert!(report.problems.is_empty(), "{:?}", report.problems);
    let stored = repo.event_matches("2019lake").await.unwrap();
    assert_eq!(stored.len(), 18);
    let labels: Vec<String> = stored.iter().map(|m| m.label()).collect();
    assert_eq!(
        labels,
        [
            "Q1", "Q2", "Q3", "QF1", "QF1-2", "QF2", "QF2-2", "QF3", "QF3-2", "QF4", "QF4-2",
            "QF4-3", "SF1", "SF1-2", "SF2", "SF2-2", "F1", "F2"
        ]
    );
    assert_eq!(stored[3].comp_level, CompLevel::QuarterFinal);
}

#[tokio::test]
async fn a_2019_ranking_has_no_average_match_points_rather_than_cargo() {
    let base = recorded_upstream().await;
    let repo = repo().await;
    stored_event(&repo, "2019lake", &[364]).await;

    let report = sync::sync_stats(&repo, &tba(&base), "2019lake")
        .await
        .expect("stats");
    assert!(report.problems.is_empty(), "{:?}", report.problems);
    let stats = repo
        .team_stats("2019lake", 364)
        .await
        .unwrap()
        .expect("364");
    assert_eq!(stats.rank, Some(1));
    assert!(
        close(stats.qual_average, 2.77),
        "Ranking Score is first every year"
    );
    assert_eq!(
        stats.avg_match_points, None,
        "sort_orders[1] is Cargo (219) in 2019"
    );
    assert_eq!(stats.total_points, Some(25));
    assert!(close(stats.auto_opr, 5.95529889668504), "autoPoints");
    assert!(close(stats.teleop_opr, 29.46221704758401));
    assert_eq!(stats.endgame_opr, None, "2019 had no endgame component");
}

#[tokio::test]
async fn last_seasons_first_payloads_still_parse() {
    let base = recorded_upstream().await;
    let client = first(&base, 2025);
    let events = client.events(&only("LAKE")).await.expect("events");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].tba_key().as_deref(), Some("2025lake"));
    assert_eq!(
        events[0].location(),
        "Pontchartrain Center, Kenner, LA, USA"
    );
    assert_eq!(events[0].week_number, Some(6));
    let teams = client.event_teams("LAKE").await.expect("teams");
    assert_eq!(teams.len(), 55);
    assert!(teams.iter().all(|t| !t.display_name().is_empty()));
}

// ── Nothing yet ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn an_event_that_has_not_happened_is_empty_not_an_error() {
    let base = recorded_upstream().await;
    let client = tba(&base);
    // Recorded: `[]`, `{}`, and rankings with column names but no rows.
    assert!(
        client
            .matches("2026azrl")
            .await
            .expect("matches")
            .is_empty()
    );
    assert!(client.oprs("2026azrl").await.expect("oprs").oprs.is_empty());
    assert!(
        client
            .rankings("2026azrl")
            .await
            .expect("rankings")
            .rankings
            .is_empty()
    );
    // Not recorded -- no event answered it on the day -- but what TBA sends
    // for an event it has nothing for at all.
    assert!(
        client
            .matches("2026null")
            .await
            .expect("matches")
            .is_empty()
    );
    assert!(client.oprs("2026null").await.expect("oprs").oprs.is_empty());
    assert!(
        client
            .component_oprs("2026null")
            .await
            .expect("coprs")
            .components
            .is_empty()
    );
    assert!(
        client
            .rankings("2026null")
            .await
            .expect("rankings")
            .rankings
            .is_empty()
    );
}
