//! The courier (S7): a lead scout's tablet fetching from TBA with its own
//! signal, and handing the Pi what it fetched.
//!
//! TBA is a stub on localhost that honours `If-None-Match`. The Pi is the
//! server's own repo behind a transport the test can unplug: its key answer
//! is whatever the test says, and a pushed bundle goes through the Pi's real
//! import (S5). `tt-web`'s push tests run the courier against the real
//! router.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::Router;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use serde_json::{Value as JsonValue, json};
use tt_client::ClientRepo;
use tt_client::courier::{Courier, VIA};
use tt_client::sync::{Reply, Transport};
use tt_core::records::Event;
use tt_core::user::Roles;
use tt_repo::{LocalRepo, NewUpstream, NewUser};
use tt_repo_sqlite::SqliteRepo;
use tt_repo_sqlite::bundle::Pusher;
use tt_repo_sqlite::snapshot::{self, Audience};

const MATCHES: &str = r#"[
  {"key":"2026here_qm1","comp_level":"qm","set_number":1,"match_number":1,
   "alliances":{"red":{"score":88,"team_keys":["frc10101","frc254","frc1678"]},
                "blue":{"score":74,"team_keys":["frc4","frc5","frc6"]}},
   "winning_alliance":"red"}
]"#;

fn at(minute: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 14, 10, minute, 0).unwrap()
}

// ── TBA ─────────────────────────────────────────────────────────────────────

/// What the stub serves, and what it has sent.
#[derive(Default)]
struct Tba {
    /// Bumped to change the rankings.
    rankings: AtomicUsize,
    /// 200s: whole bodies over the tablet's data plan.
    full: AtomicUsize,
    not_modified: AtomicUsize,
}

async fn resource(
    State(tba): State<Arc<Tba>>,
    Path((event, what)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    if headers.get("x-tba-auth-key").and_then(|k| k.to_str().ok()) != Some("the-key") {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if event != "2026here" {
        return StatusCode::NOT_FOUND.into_response();
    }
    let version = tba.rankings.load(Ordering::SeqCst);
    let body = match what.as_str() {
        "matches" => MATCHES.to_string(),
        "oprs" => r#"{"oprs":{"frc254":51.5},"dprs":{},"ccwms":{}}"#.into(),
        "rankings" => format!(r#"{{"rankings":[],"sort_order_info":[{{"name":"v{version}"}}]}}"#),
        "coprs" => "{}".into(),
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    let changed = if what == "rankings" { version } else { 0 };
    let etag = format!("W/\"{what}-{changed}\"");
    if headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        == Some(etag.as_str())
    {
        tba.not_modified.fetch_add(1, Ordering::SeqCst);
        return StatusCode::NOT_MODIFIED.into_response();
    }
    tba.full.fetch_add(1, Ordering::SeqCst);
    ([(header::ETAG, etag)], body).into_response()
}

async fn tba() -> (String, Arc<Tba>) {
    let tba = Arc::new(Tba::default());
    let app = Router::new()
        .route("/event/{event}/{what}", get(resource))
        .with_state(tba.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://{addr}"), tba)
}

// ── The Pi ──────────────────────────────────────────────────────────────────

struct Everyone;

impl Audience for Everyone {
    fn team(&self) -> Option<i32> {
        Some(10101)
    }
    fn has_event(&self, _: &str) -> bool {
        true
    }
    fn has_upstream(&self, _: &str) -> bool {
        true
    }
    fn answers(&self, _: Option<i32>, _: &str) -> Option<String> {
        None
    }
}

/// The server's repo, a lead scout on it, and what it answers at the key.
struct Pi {
    repo: SqliteRepo,
    dir: PathBuf,
    lead: i64,
    reachable: Cell<bool>,
    key: RefCell<(u16, JsonValue)>,
    asked: RefCell<Vec<String>>,
}

impl Drop for Pi {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Pi {
    /// A Pi with today's event, and the tablet's copy of it.
    async fn new(test: &str) -> (Self, ClientRepo) {
        let dir =
            std::env::temp_dir().join(format!("tt-client-courier-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let repo =
            SqliteRepo::connect(&format!("sqlite://{}", dir.join("pi.db").display())).unwrap();
        tt_repo_sqlite::migrate::apply(repo.pool()).await.unwrap();
        let today = NaiveDate::from_ymd_opt(2026, 3, 14);
        let event = Event {
            key: "2026here".into(),
            name: "Here".into(),
            location: None,
            timezone: None,
            start_date: today,
            end_date: today,
            event_code: None,
            event_type: None,
            district_key: None,
            week: None,
        };
        repo.upsert_event(&event, at(0)).await.unwrap();
        let lead = repo
            .create_user(
                NewUser {
                    email: "lee@x".into(),
                    name: "Lee".into(),
                    password_hash: "hash".into(),
                    team_number: Some(10101),
                    roles: Roles {
                        is_lead_scout: true,
                        ..Roles::default()
                    },
                },
                at(0),
            )
            .await
            .unwrap();
        let device =
            ClientRepo::from_bytes(&snapshot::build(&repo, &Everyone).await.unwrap().bytes)
                .unwrap();
        let pi = Pi {
            repo,
            dir,
            lead: lead.id,
            reachable: Cell::new(true),
            key: RefCell::new((200, JsonValue::Null)),
            asked: RefCell::default(),
        };
        (pi, device)
    }

    fn answers_key(&self, status: u16, body: JsonValue) {
        *self.key.borrow_mut() = (status, body);
    }

    fn asked(&self) -> Vec<String> {
        std::mem::take(&mut *self.asked.borrow_mut())
    }
}

impl Transport for &Pi {
    async fn get(&self, path: &str) -> Result<Reply, String> {
        if !self.reachable.get() {
            return Err("no route to the Pi".into());
        }
        self.asked.borrow_mut().push(path.into());
        let (status, body) = self.key.borrow().clone();
        Ok(Reply {
            status,
            body: body.to_string(),
        })
    }

    async fn post_json(&self, path: &str, _: String) -> Result<Reply, String> {
        panic!("the courier posts no JSON, but asked {path}");
    }

    /// What `POST /api/sync/bundle` does with it, and answers.
    async fn post_file(&self, path: &str, body: Vec<u8>) -> Result<Reply, String> {
        if !self.reachable.get() {
            return Err("no route to the Pi".into());
        }
        self.asked.borrow_mut().push(path.into());
        let file = self.dir.join("pushed.sqlite");
        std::fs::write(&file, body).unwrap();
        let pusher = Pusher {
            user_id: self.lead,
            device: None,
        };
        let imported = self.repo.import_bundle(&file, pusher, Utc::now()).await;
        let imported = imported.unwrap();
        Ok(Reply {
            status: 200,
            body: json!({
                "status": "imported",
                "log": imported.log,
                "cursor": imported.to_seq,
                "appended": imported.appended.len(),
            })
            .to_string(),
        })
    }
}

fn key(base: &str, uplink_online: bool) -> JsonValue {
    json!({ "tba": "the-key", "base": base, "uplink_online": uplink_online })
}

/// A log id, and how many were made.
fn log_ids() -> (Rc, impl Fn() -> String) {
    let made = Rc::default();
    let count: Rc = std::rc::Rc::clone(&made);
    (made, move || {
        count.set(count.get() + 1);
        format!("tablet-{}", count.get())
    })
}

type Rc = std::rc::Rc<Cell<usize>>;

// ── Tests ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_lead_scouts_tablet_fetches_with_its_own_signal_and_hands_it_over_on_reconnect() {
    let (base, tba) = tba().await;
    let (pi, device) = Pi::new("reconnect").await;
    pi.answers_key(200, key(&base, false));
    // Something the tablet pulled from the Pi: never sent back.
    device
        .append_upstream(&NewUpstream {
            api: "tba".into(),
            path: "/event/2026here/oprs".into(),
            etag: None,
            body: r#"{"oprs":{"frc254":40.0}}"#.into(),
            fetched_at: at(0),
            via: "pi".into(),
        })
        .await
        .unwrap();
    let (made, fresh_log) = log_ids();
    let courier = Courier::new(&pi, vec!["2026here".into()]);

    // On the venue's wifi, which has internet today: the Pi gives the key,
    // the tablet fetches all four, and hands them over at once.
    let tick = courier.tick(&device, at(10), &fresh_log).await.unwrap();
    assert!(tick.pi && tick.key, "{tick:?}");
    assert_eq!(
        (tick.fetched, tick.pushed, tick.waiting),
        (4, 4, 0),
        "{tick:?}"
    );
    assert_eq!(tick.skipped, None);
    assert_eq!(tba.full.load(Ordering::SeqCst), 4);
    let schema = tt_repo_sqlite::migrate::latest();
    assert_eq!(
        pi.asked(),
        [
            "/api/upstream/key".to_string(),
            format!("/api/sync/bundle?schema={schema}")
        ]
    );
    let on_pi = pi
        .repo
        .latest_upstream("tba", "/event/2026here/matches")
        .await
        .unwrap()
        .expect("the matches reached the Pi");
    assert_eq!(on_pi.entry.body, MATCHES);
    assert!(
        on_pi.entry.via.starts_with("bundle:"),
        "{}",
        on_pi.entry.via
    );

    // Out in the lobby: no Pi, but signal. Only the rankings changed, so
    // that is the one whole body; the rest are 304s from what it holds.
    pi.reachable.set(false);
    tba.rankings.store(1, Ordering::SeqCst);
    let tick = courier.tick(&device, at(13), &fresh_log).await.unwrap();
    assert!(!tick.pi && tick.key, "the key is kept for this: {tick:?}");
    assert_eq!(
        (tick.fetched, tick.pushed, tick.waiting),
        (1, 0, 1),
        "{tick:?}"
    );
    assert_eq!(tba.full.load(Ordering::SeqCst), 5);
    assert_eq!(tba.not_modified.load(Ordering::SeqCst), 3);
    let held = device
        .latest_upstream("tba", "/event/2026here/rankings")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(held.entry.via, VIA);

    // Back in the venue a minute later: too soon to fetch, but the Pi gets
    // the new rankings.
    pi.reachable.set(true);
    let tick = courier.tick(&device, at(14), &fresh_log).await.unwrap();
    assert_eq!(
        tick.skipped.as_deref(),
        Some("fetched less than two minutes ago")
    );
    assert_eq!(
        (tick.fetched, tick.pushed, tick.waiting),
        (0, 1, 0),
        "{tick:?}"
    );
    let on_pi = pi
        .repo
        .latest_upstream("tba", "/event/2026here/rankings")
        .await
        .unwrap()
        .unwrap();
    assert!(on_pi.entry.body.contains("v1"), "{}", on_pi.entry.body);

    // Nothing waits, so nothing is pushed; and one log for the file.
    pi.asked();
    let tick = courier.tick(&device, at(15), &fresh_log).await.unwrap();
    assert_eq!(tick.waiting, 0);
    assert_eq!(pi.asked(), ["/api/upstream/key"]);
    assert_eq!(made.get(), 1);
    let imports = pi.repo.bundle_imports(10).await.unwrap();
    assert_eq!(imports.len(), 2);
    assert_eq!(imports[0].user.as_deref(), Some("Lee"));
}

#[tokio::test]
async fn the_tablet_leaves_fetching_to_a_pi_with_its_own_uplink_and_stops_without_the_role() {
    let (base, tba) = tba().await;
    let (pi, device) = Pi::new("no-fetch").await;
    let (_, fresh_log) = log_ids();
    let courier = Courier::new(&pi, vec!["2026here".into()]);

    pi.answers_key(200, key(&base, true));
    let tick = courier.tick(&device, at(10), &fresh_log).await.unwrap();
    assert_eq!(
        tick.skipped.as_deref(),
        Some("the Pi has its own connection")
    );
    assert!(tick.key);
    assert_eq!(tba.full.load(Ordering::SeqCst), 0);

    // No longer a lead scout: the key goes, and the lobby fetches nothing.
    pi.answers_key(
        403,
        json!({ "error": "only a lead scout may fetch upstream data" }),
    );
    let tick = courier.tick(&device, at(20), &fresh_log).await.unwrap();
    assert!(!tick.key, "{tick:?}");
    pi.reachable.set(false);
    let tick = courier.tick(&device, at(30), &fresh_log).await.unwrap();
    assert_eq!(
        tick.skipped.as_deref(),
        Some("this device has no key to fetch with")
    );

    // A Pi with no key of its own takes the tablet's too.
    pi.reachable.set(true);
    pi.answers_key(200, key(&base, false));
    courier.tick(&device, at(40), &fresh_log).await.unwrap();
    pi.answers_key(
        200,
        json!({ "tba": null, "base": null, "uplink_online": false }),
    );
    let tick = courier.tick(&device, at(50), &fresh_log).await.unwrap();
    assert!(!tick.key, "{tick:?}");

    // A lapsed session (a sign-in redirect) keeps it.
    pi.answers_key(200, key(&base, false));
    courier.tick(&device, at(52), &fresh_log).await.unwrap();
    pi.answers_key(0, JsonValue::Null);
    let tick = courier.tick(&device, at(54), &fresh_log).await.unwrap();
    assert!(tick.key, "{tick:?}");
}

#[tokio::test]
async fn with_no_signal_nothing_is_queued_and_the_next_tick_tries_again() {
    let (pi, device) = Pi::new("no-signal").await;
    let (_, fresh_log) = log_ids();
    // Nothing listens there: the venue's wifi with no internet behind it.
    pi.answers_key(200, key("http://127.0.0.1:9", false));
    let courier = Courier::new(&pi, vec!["2026here".into()]);

    let tick = courier.tick(&device, at(10), &fresh_log).await.unwrap();
    assert_eq!(tick.skipped.as_deref(), Some("no signal"));
    assert_eq!((tick.fetched, tick.waiting), (0, 0));
    assert!(tick.problems.is_empty(), "{tick:?}");
    // Not "fetched less than two minutes ago": it never reached TBA.
    let tick = courier.tick(&device, at(10), &fresh_log).await.unwrap();
    assert_eq!(tick.skipped.as_deref(), Some("no signal"));
}

#[tokio::test]
async fn with_no_events_named_the_tablet_fetches_those_on_today_and_says_what_tba_refused() {
    let (base, tba) = tba().await;
    let (pi, device) = Pi::new("today").await;
    let (_, fresh_log) = log_ids();
    pi.answers_key(200, key(&base, false));
    pi.reachable.set(false);

    // 2026here is on today, so it is fetched.
    let tick = Courier::new(&pi, vec![])
        .tick(&device, at(10), &fresh_log)
        .await
        .unwrap();
    assert_eq!(tick.fetched, 0, "no key yet: the Pi was never asked");
    pi.reachable.set(true);
    let tick = Courier::new(&pi, vec![])
        .tick(&device, at(10), &fresh_log)
        .await
        .unwrap();
    assert_eq!(tick.fetched, 4, "{tick:?}");
    assert_eq!(tba.full.load(Ordering::SeqCst), 4);

    // An event TBA does not know is said, not taken for no signal.
    let tick = Courier::new(&pi, vec!["2026nowhere".into()])
        .tick(&device, at(20), &fresh_log)
        .await
        .unwrap();
    assert_eq!(tick.skipped, None);
    assert_eq!(tick.problems.len(), 4, "{tick:?}");
    assert!(tick.problems[0].contains("404"), "{:?}", tick.problems);
}
