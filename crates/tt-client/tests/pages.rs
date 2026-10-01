//! The pages a device makes for itself (C5), against the server's.
//!
//! A Pi is seeded through the server's own `Repo` and cut into a snapshot for
//! team 10101 at 2026here, as `/api/sync/snapshot` cuts it. Each page made
//! from the device's copy must be the page the Pi makes from its own
//! database for a 10101 scout, byte for byte: the same function, the same
//! template, and rows the snapshot kept.

use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use tt_client::ClientRepo;
use tt_client::pages::render;
use tt_core::matches::CompLevel;
use tt_core::notes::{self, Notes};
use tt_core::records::{Event, MatchRecord, Team, TeamEventStats};
use tt_core::review::Decision;
use tt_core::season::{Payload, SeasonSchema, Value, current_season};
use tt_core::user::Roles;
use tt_pages::events;
use tt_repo::{LocalRepo, NewObservation, NewUser, Recorded};
use tt_repo_sqlite::SqliteRepo;
use tt_repo_sqlite::snapshot::{self, Audience};
use tt_templates::{Nav, Page};

fn at(minute: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 14, 10, minute, 0).unwrap()
}

/// During 2026here.
fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 14, 18, 0, 0).unwrap()
}

/// Team 10101 at 2026here, with other teams' notes removed, as the server's
/// snapshot does it (`tt_web::snapshot`).
struct Ours<'a>(&'a SeasonSchema);

impl Audience for Ours<'_> {
    fn team(&self) -> Option<i32> {
        Some(10101)
    }
    fn has_event(&self, event_key: &str) -> bool {
        event_key == "2026here"
    }
    fn has_upstream(&self, _: &str) -> bool {
        true
    }
    fn answers(&self, writer: Option<i32>, payload: &str) -> Option<String> {
        if Notes::for_viewer(Some(10101), writer).shown() {
            return None;
        }
        let mut answers: Payload = serde_json::from_str(payload).unwrap_or_default();
        notes::redact(self.0, &mut answers);
        Some(serde_json::to_string(&answers).unwrap())
    }
}

struct Dir(PathBuf);

impl Dir {
    fn new(test: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("tt-pages-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Dir(dir)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn event(key: &str, start: (u32, u32)) -> Event {
    let date = NaiveDate::from_ymd_opt(2026, start.0, start.1);
    Event {
        key: key.into(),
        name: format!("Event {key}"),
        location: Some("Somewhere".into()),
        timezone: Some("America/Chicago".into()),
        start_date: date,
        end_date: date.map(|d| d + chrono::TimeDelta::days(2)),
        event_code: Some(key[4..].into()),
        event_type: Some("Regional".into()),
        district_key: None,
        week: Some(2),
    }
}

fn team(number: i32, name: &str) -> Team {
    Team {
        number,
        name: name.into(),
        nickname: Some(name.into()),
        school: None,
        city: Some("Houston".into()),
        state: None,
        country: Some("USA".into()),
        rookie_year: Some(2000),
        website: None,
    }
}

fn game(number: i32, red: [i32; 3], blue: [i32; 3]) -> MatchRecord {
    MatchRecord {
        key: format!("2026here_qm{number}"),
        event_key: "2026here".into(),
        comp_level: CompLevel::Qualification,
        set_number: 1,
        match_number: number,
        red: red.map(Some),
        blue: blue.map(Some),
        red_score: Some(80),
        blue_score: Some(75),
        winner: Some("red".into()),
        played: number == 1,
        scheduled_at: Some(at(number as u32)),
        actual_at: None,
    }
}

fn observation(id: &str, number: i32, writer: i32, scouter: i64) -> NewObservation {
    let mut payload = Payload::new();
    payload.insert("auto_scored".into(), Value::Count(3));
    payload.insert("teleop_scored".into(), Value::Count(7));
    payload.insert("endgame".into(), Value::Text("deep".into()));
    payload.insert("notes".into(), Value::Text(format!("{writer}'s notes")));
    NewObservation {
        client_record_id: id.into(),
        match_key: format!("2026here_qm{number}"),
        event_key: "2026here".into(),
        team_number: 254,
        alliance: "red",
        payload,
        schema_version: current_season().unwrap().version,
        scouter_id: Some(scouter),
        device_id: None,
        submitting_team: Some(writer),
        observed_at: at(5),
    }
}

/// The Pi, seeded through its own writes, and a device holding a snapshot
/// of it for [`Ours`].
async fn fixture(dir: &Dir, season: &SeasonSchema) -> (SqliteRepo, ClientRepo) {
    let url = format!("sqlite://{}", dir.0.join("pi.db").display());
    let pi = SqliteRepo::connect(&url).unwrap();
    tt_repo_sqlite::migrate::apply(pi.pool()).await.unwrap();

    let user = |email: &str, name: &str, team: i32| NewUser {
        email: email.into(),
        name: name.into(),
        password_hash: "SECRET-HASH".into(),
        team_number: Some(team),
        roles: Roles::default(),
    };
    let sam = pi.create_user(user("sam@x", "Sam", 10101), at(0)).await;
    let pat = pi.create_user(user("pat@x", "Pat", 254), at(0)).await;
    let lee = pi.create_user(user("lee@x", "Lee", 10101), at(0)).await;
    let (sam, pat, lee) = (sam.unwrap().id, pat.unwrap().id, lee.unwrap().id);

    pi.upsert_event(&event("2026here", (3, 13)), at(0))
        .await
        .unwrap();
    pi.upsert_event(&event("2026away", (4, 1)), at(0))
        .await
        .unwrap();
    for (number, name) in [(10101, "Teal"), (254, "Poofs"), (1678, "Citrus")] {
        pi.upsert_team(&team(number, name), at(0)).await.unwrap();
        pi.link_event_team("2026here", number, at(0)).await.unwrap();
    }
    pi.link_event_team("2026away", 254, at(0)).await.unwrap();
    pi.upsert_match(&game(1, [254, 10101, 1678], [4, 5, 6]), at(0))
        .await
        .unwrap();
    pi.upsert_match(&game(2, [4, 5, 6], [254, 10101, 1678]), at(0))
        .await
        .unwrap();
    let stats = TeamEventStats {
        team_number: 254,
        event_key: "2026here".into(),
        opr: Some(50.5),
        rank: Some(1),
        wins: Some(3),
        synced_at: Some(at(1)),
        ..TeamEventStats::default()
    };
    pi.upsert_team_stats(&stats, at(1)).await.unwrap();

    // Approved: one by our scout, one by another team's, whose notes the
    // snapshot removes and the Pi's page hides. One still waiting.
    for (id, number, writer, scouter) in [("ours", 1, 10101, sam), ("theirs", 2, 254, pat)] {
        let Recorded::Created(row) = pi
            .record_observation(&observation(id, number, writer, scouter), at(6))
            .await
            .unwrap()
        else {
            panic!("recorded");
        };
        assert!(
            pi.review_observation(row, &Decision::Approve, lee, at(7))
                .await
                .unwrap()
        );
    }
    pi.record_observation(&observation("waiting", 2, 10101, sam), at(8))
        .await
        .unwrap();

    let snap = snapshot::build(&pi, &Ours(season)).await.expect("snapshot");
    let device = ClientRepo::from_bytes(&snap.bytes).expect("the device opens it");
    (pi, device)
}

/// The event and header as `tt-web` makes them for a 10101 scout.
async fn scout_view(repo: &impl LocalRepo) -> (Nav, events::EventContext) {
    let context = events::resolve(repo, Some(10101), Some("2026here"), now()).await;
    let nav = Nav {
        event: context.switcher(),
        ..Nav::anonymous(true)
    };
    (nav, context)
}

/// The team page as `tt-web` makes it for a 10101 scout, over `repo`.
async fn team_page(repo: &impl LocalRepo, season: &SeasonSchema, query: Option<&str>) -> String {
    let (nav, context) = scout_view(repo).await;
    tt_pages::teams::page(repo, season, nav, Some(10101), &context, query, now())
        .await
        .render_html()
        .unwrap()
}

/// The other pages a scout reads at an event, as `tt-web` makes them for a
/// 10101 scout, over `repo`: home, the notes, and the graph.
async fn other_pages(repo: &impl LocalRepo, season: &SeasonSchema) -> Vec<(&'static str, String)> {
    let mut made = Vec::new();
    let (nav, context) = scout_view(repo).await;
    let home = tt_pages::home::page(repo, season, nav, Some(10101), &context).await;
    made.push(("home", home.render_html().unwrap()));
    for (name, query) in [
        ("notes", ""),
        ("notes by schedule", "order=schedule&team=254"),
    ] {
        let query: HashMap<String, String> = form_urlencoded::parse(query.as_bytes())
            .into_owned()
            .collect();
        let (nav, context) = scout_view(repo).await;
        let page =
            tt_pages::notes::page(repo, season, nav, Some(10101), &context, &query, now()).await;
        made.push((name, page.render_html().unwrap()));
    }
    for (name, query) in [
        ("graph", ""),
        ("graph chosen", "chosen=1&team=254&metric=points&metric=opr"),
        ("graph season", "span=season"),
    ] {
        let query: Vec<(String, String)> = form_urlencoded::parse(query.as_bytes())
            .into_owned()
            .collect();
        let (nav, context) = scout_view(repo).await;
        let page = tt_pages::graph::page(repo, season, nav, &context, &query, now()).await;
        made.push((name, page.render_html().unwrap()));
    }
    made
}

/// `page` without its "Other events" card, or the blank lines its
/// template leaves.
fn without_other_events(page: &str) -> String {
    let Some(start) =
        page.find("<section class=\"card\">\n    <div class=\"card-header\"><h2>Other events")
    else {
        return without_blank_lines(page);
    };
    let end = start + page[start..].find("</section>").unwrap() + "</section>".len();
    without_blank_lines(&format!("{}{}", &page[..start], &page[end..]))
}

fn without_blank_lines(page: &str) -> String {
    page.lines()
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn the_device_makes_the_page_the_server_makes() {
    let dir = Dir::new("same");
    let season = current_season().unwrap();
    let (pi, device) = fixture(&dir, &season).await;

    for query in [Some("254"), Some("1678"), Some("9999"), Some("abc"), None] {
        let served = team_page(&pi, &season, query).await;
        let made = team_page(&device, &season, query).await;
        // The one difference: a snapshot keeps no roster outside its events
        // (S10), so the device cannot know where else a team plays.
        assert_eq!(
            without_other_events(&served),
            without_blank_lines(&made),
            "team {query:?}"
        );
    }

    let served = team_page(&pi, &season, Some("254")).await;
    assert!(served.contains("/teams?event=2026away&#38;team=254"));
    let made = team_page(&device, &season, Some("254")).await;
    assert!(!made.contains("Other events"));
    for shown in [
        "Poofs",
        "OPR</dt><dd>50.50",
        "Q1 · Sam",
        "10101&#39;s notes",
        "Q2",
    ] {
        assert!(made.contains(shown), "{shown} missing");
    }
    assert!(!made.contains("254&#39;s notes"), "another team's notes");

    let served = other_pages(&pi, &season).await;
    let made = other_pages(&device, &season).await;
    for ((name, served), (_, made)) in served.iter().zip(&made) {
        assert_eq!(served, made, "{name}");
    }
    let [(_, home), (_, notes), _, (_, graph), ..] = made.as_slice() else {
        panic!("every page made");
    };
    assert!(home.contains("Citrus"), "the roster");
    assert!(notes.contains("10101&#39;s notes") && notes.contains("Q1 · Sam"));
    assert!(!notes.contains("254&#39;s notes"), "another team's notes");
    assert!(graph.contains("Poofs") && graph.contains("Scouting: 2 approved observations"));
}

#[tokio::test]
async fn the_worker_answers_the_addresses_it_knows_and_no_others() {
    let dir = Dir::new("routes");
    let season = current_season().unwrap();
    let (_pi, device) = fixture(&dir, &season).await;

    let page = render(
        &device,
        &season,
        "/teams",
        "event=2026HERE&team=+254+",
        now(),
    )
    .await
    .expect("the team page is made here");
    assert!(
        page.contains(r#"id="device-page""#),
        "says where it came from"
    );
    assert!(page.contains("Poofs") && page.contains("OPR</dt><dd>50.50"));
    // The device cannot know who is holding it: no account links.
    assert!(!page.contains("Sign in") && !page.contains("Sign out"));
    // But it knows whose copy it holds: that team's notes, and its events.
    assert_eq!(
        tt_client::pages::viewer_team(&device),
        Some(10101),
        "the snapshot says"
    );
    assert_eq!(
        tt_client::pages::TEAM_SOURCE,
        tt_repo_sqlite::snapshot::TEAM_SOURCE
    );
    assert!(page.contains("10101&#39;s notes"));
    assert!(!page.contains(r#"value="2026away""#), "10101 is not there");
    // The pages it can make are a tap away; the lead's are not offered.
    assert!(page.contains(r#"href="/notes?event=2026here""#));
    assert!(!page.contains("/lead-scout"));

    let roster = render(&device, &season, "/teams", "", now()).await.unwrap();
    assert!(roster.contains("Citrus"), "the roster at the default event");

    let home = render(&device, &season, "/", "", now()).await.unwrap();
    assert!(home.contains("copy of team 10101's data") && home.contains(r#"id="offline-agenda""#));
    assert!(
        !home.contains("/sign-in"),
        "no sign-in the device cannot do"
    );
    let notes = render(&device, &season, "/notes", "team=254", now())
        .await
        .unwrap();
    assert!(notes.contains("10101&#39;s notes"));
    // `team` and `metric` repeat, and every one counts.
    let graph = render(
        &device,
        &season,
        "/graph",
        "chosen=1&team=254&metric=points&metric=opr",
        now(),
    )
    .await
    .unwrap();
    for on in [
        r#"name="team" value="254" checked"#,
        r#"name="metric" value="points" checked"#,
        r#"name="metric" value="opr" checked"#,
    ] {
        assert!(graph.contains(on), "{on}");
    }

    for elsewhere in [
        "/lead-scout",
        "/submission",
        "/pick-list",
        "/teams/",
        "/api/sync/pull",
    ] {
        assert!(
            render(&device, &season, elsewhere, "event=2026here", now())
                .await
                .is_none(),
            "{elsewhere} is the server's to answer"
        );
    }
}
