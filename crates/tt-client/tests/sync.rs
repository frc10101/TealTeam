//! The outbox and the sync client (C7), against a Pi that answers whatever
//! the test says. `tt-web`'s push tests run the same client against the real
//! router; these cover what a real Pi rarely does on cue: refusing, being
//! unreachable, running another schema, and sending rows that clash.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::PathBuf;

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use serde_json::{Value as JsonValue, json};
use tt_client::ClientRepo;
use tt_client::sync::{CHANGES_SOURCE, Reply, Stop, SyncClient, Transport, UPSTREAM_SOURCE};
use tt_core::matches::CompLevel;
use tt_core::outbox::Push;
use tt_core::records::{Event, MatchRecord, Team};
use tt_core::season::{Payload, Value};
use tt_repo::{LocalRepo, NewObservation, Recorded};
use tt_repo_sqlite::SqliteRepo;
use tt_repo_sqlite::snapshot::{self, Audience};

const MINE: &str = "0191f7ac-1234-7000-8000-000000000001";
const OTHER: &str = "0191f7ac-1234-7000-8000-000000000002";
const PIS: &str = "0191f7ac-1234-7000-8000-0000000000aa";

fn at(minute: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 14, 10, minute, 0).unwrap()
}

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

struct Dir(PathBuf);

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A device opened from a snapshot of a Pi with one event and one match:
/// 2026here Q1, 10101 254 1678 on red.
async fn device(test: &str) -> ClientRepo {
    let dir =
        Dir(std::env::temp_dir().join(format!("tt-client-sync-{}-{test}", std::process::id())));
    let _ = std::fs::remove_dir_all(&dir.0);
    std::fs::create_dir_all(&dir.0).unwrap();
    let pi = SqliteRepo::connect(&format!("sqlite://{}", dir.0.join("pi.db").display())).unwrap();
    tt_repo_sqlite::migrate::apply(pi.pool()).await.unwrap();
    let date = NaiveDate::from_ymd_opt(2026, 3, 13);
    let event = Event {
        key: "2026here".into(),
        name: "Here".into(),
        location: None,
        timezone: None,
        start_date: date,
        end_date: date,
        event_code: None,
        event_type: None,
        district_key: None,
        week: None,
    };
    pi.upsert_event(&event, at(0)).await.unwrap();
    for number in [10101, 254, 1678, 4, 5, 6] {
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
        pi.upsert_team(&team, at(0)).await.unwrap();
    }
    let game = MatchRecord {
        key: "2026here_qm1".into(),
        event_key: "2026here".into(),
        comp_level: CompLevel::Qualification,
        set_number: 1,
        match_number: 1,
        red: [Some(10101), Some(254), Some(1678)],
        blue: [Some(4), Some(5), Some(6)],
        red_score: None,
        blue_score: None,
        winner: None,
        played: false,
        scheduled_at: None,
        actual_at: None,
    };
    pi.upsert_match(&game, at(0)).await.unwrap();
    let snap = snapshot::build(&pi, &Everyone).await.unwrap();
    ClientRepo::from_bytes(&snap.bytes).unwrap()
}

fn observation(id: &str, team: i32, scouter: i64) -> NewObservation {
    let mut payload = Payload::new();
    payload.insert("teleop_scored".into(), Value::Count(9));
    payload.insert("notes".into(), Value::Text("tippy".into()));
    NewObservation {
        client_record_id: id.into(),
        match_key: "2026here_qm1".into(),
        event_key: "2026here".into(),
        team_number: team,
        alliance: "red",
        payload,
        schema_version: 1,
        scouter_id: Some(scouter),
        device_id: None,
        submitting_team: Some(10101),
        observed_at: at(5),
    }
}

/// Answers in the order given, and remembers what was asked.
#[derive(Default)]
struct Pi {
    replies: RefCell<VecDeque<Result<Reply, String>>>,
    asked: RefCell<Vec<(String, Option<String>)>>,
}

impl Pi {
    fn answers(replies: impl IntoIterator<Item = Result<(u16, JsonValue), &'static str>>) -> Self {
        let replies = replies
            .into_iter()
            .map(|r| {
                r.map(|(status, body)| Reply {
                    status,
                    body: body.to_string(),
                })
                .map_err(String::from)
            })
            .collect();
        Self {
            replies: RefCell::new(replies),
            asked: RefCell::default(),
        }
    }

    fn reply(&self, path: &str, body: Option<String>) -> Result<Reply, String> {
        self.asked.borrow_mut().push((path.into(), body));
        self.replies
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| panic!("nothing left to answer {path}"))
    }

    fn paths(&self) -> Vec<String> {
        self.asked.borrow().iter().map(|(p, _)| p.clone()).collect()
    }
}

impl Transport for &Pi {
    async fn get(&self, path: &str) -> Result<Reply, String> {
        self.reply(path, None)
    }
    async fn post_json(&self, path: &str, body: String) -> Result<Reply, String> {
        self.reply(path, Some(body))
    }
    async fn post_file(&self, path: &str, _: Vec<u8>) -> Result<Reply, String> {
        self.reply(path, Some("<a bundle>".into()))
    }
}

fn recorded(schema: i64, ids: &[&str]) -> Result<(u16, JsonValue), &'static str> {
    let receipts: Vec<JsonValue> = ids
        .iter()
        .map(|id| json!({ "client_record_id": id, "outcome": "recorded" }))
        .collect();
    Ok((200, json!({ "schema": schema, "receipts": receipts })))
}

fn pulled(schema: i64, changes: JsonValue, cursor: i64) -> Result<(u16, JsonValue), &'static str> {
    Ok((
        200,
        json!({
            "schema": schema, "build": "b", "form": 1,
            "changes": changes, "changes_cursor": cursor, "changes_more": false,
            "upstream": [], "upstream_cursor": 0, "upstream_more": false,
        }),
    ))
}

/// The observation's row as the Pi's change log writes it.
fn echo(seq: i64, id: &str, team: i32, scouter: i64, state: &str) -> JsonValue {
    json!({
        "seq": seq, "entity": "observation", "entity_pk": id, "op": "upsert",
        "event_key": "2026here", "at": "2026-03-14T10:06:00.000Z",
        "row": {
            "client_record_id": id, "match_key": "2026here_qm1", "team_number": team,
            "event_key": "2026here", "alliance": "red",
            "payload": { "notes": "tippy", "teleop_scored": 9 }, "schema_version": 1,
            "scouter_id": scouter, "device_id": 3, "submitting_team": 10101,
            "review_state": state, "review_note": null, "reviewed_by": null,
            "reviewed_at": null, "observed_at": "2026-03-14T10:05:00.000Z",
            "updated_at": "2026-03-14T10:06:00.000Z",
        },
    })
}

#[tokio::test]
async fn an_observation_recorded_offline_waits_until_the_pi_echoes_it() {
    let device = device("echo").await;
    let schema = device
        .schema()
        .unwrap()
        .expect("a snapshot has its migrations");
    let recorded_here = device
        .queue_observation(&observation(MINE, 254, 7), at(5))
        .await
        .unwrap();
    assert!(matches!(recorded_here, Recorded::Created(_)));
    assert!(device.is_queued(MINE).unwrap());
    assert_eq!(
        device.observed_teams("2026here_qm1", 7).await.unwrap(),
        vec![254],
        "the device's own pages see it at once"
    );
    // The same form posted twice is one entry.
    let again = device
        .queue_observation(&observation(MINE, 254, 7), at(6))
        .await
        .unwrap();
    assert!(matches!(again, Recorded::Duplicate(_)));
    assert_eq!(device.outbox().unwrap().len(), 1);

    // Recorded, but the change has not settled into the log yet.
    let pi = Pi::answers([recorded(schema, &[MINE]), pulled(schema, json!([]), 0)]);
    let client = SyncClient::new(&pi, vec!["2026here".into()]);
    let report = client.sync(&device, at(10)).await.unwrap();
    assert_eq!((report.sent, report.echoed, report.stopped), (1, 0, None));
    assert!(device.is_queued(MINE).unwrap(), "a receipt is not an echo");
    let paths = pi.paths();
    assert_eq!(paths[0], format!("/api/sync/push?schema={schema}"));
    assert_eq!(
        paths[1],
        format!("/api/sync/pull?schema={schema}&changes=0&upstream=0&event=2026here")
    );
    let sent: Push = serde_json::from_str(pi.asked.borrow()[0].1.as_deref().unwrap()).unwrap();
    assert_eq!(sent.observations.len(), 1);
    assert_eq!(sent.observations[0].client_record_id, MINE);
    assert_eq!(sent.observations[0].observed_at, at(5));
    let entry = &device.outbox().unwrap()[0];
    assert_eq!((entry.attempts, entry.tried_at), (1, Some(at(10))));

    // Sent again, harmlessly, and this time it comes back.
    let pi = Pi::answers([
        recorded(schema, &[MINE]),
        pulled(schema, json!([echo(41, MINE, 254, 7, "approved")]), 41),
    ]);
    let client = SyncClient::new(&pi, vec!["2026here".into()]);
    let report = client.sync(&device, at(11)).await.unwrap();
    assert_eq!((report.sent, report.echoed, report.applied), (1, 1, 1));
    assert!(device.outbox().unwrap().is_empty());
    assert_eq!(device.cursor(CHANGES_SOURCE).unwrap(), 41);
    let approved = device.approved_observations("2026here").await.unwrap();
    assert_eq!(approved.len(), 1, "the Pi's row, review and all");
    assert_eq!(approved[0].team_number, 254);

    // Nothing waiting: no push at all.
    let pi = Pi::answers([pulled(schema, json!([]), 41)]);
    SyncClient::new(&pi, vec![])
        .sync(&device, at(12))
        .await
        .unwrap();
    assert_eq!(
        pi.paths(),
        vec![format!(
            "/api/sync/pull?schema={schema}&changes=41&upstream=0"
        )]
    );
}

#[tokio::test]
async fn a_refusal_is_kept_with_its_reason_and_can_be_handed_over() {
    let device = device("refused").await;
    let schema = device.schema().unwrap().unwrap();
    device
        .queue_observation(&observation(MINE, 254, 7), at(5))
        .await
        .unwrap();
    device
        .queue_observation(&observation(OTHER, 1678, 7), at(6))
        .await
        .unwrap();

    let pi = Pi::answers([
        Ok((
            200,
            json!({ "schema": schema, "receipts": [
                { "client_record_id": MINE, "outcome": "recorded" },
                { "client_record_id": OTHER, "outcome": "refused", "reason": "Not saved: no." },
            ]}),
        )),
        pulled(schema, json!([]), 0),
    ]);
    let report = SyncClient::new(&pi, vec![])
        .sync(&device, at(10))
        .await
        .unwrap();
    assert_eq!((report.sent, report.refused), (2, 1));
    assert!(!device.is_queued(OTHER).unwrap());
    let outbox = device.outbox().unwrap();
    assert_eq!(outbox.len(), 2, "kept, not dropped");
    assert_eq!(outbox[1].refused.as_deref(), Some("Not saved: no."));
    assert_eq!(
        device.observed_teams("2026here_qm1", 7).await.unwrap(),
        vec![254],
        "the refused one is off the device's tables"
    );

    // Not sent again.
    let pi = Pi::answers([recorded(schema, &[MINE]), pulled(schema, json!([]), 0)]);
    SyncClient::new(&pi, vec![])
        .sync(&device, at(11))
        .await
        .unwrap();
    let sent: Push = serde_json::from_str(pi.asked.borrow()[0].1.as_deref().unwrap()).unwrap();
    assert_eq!(sent.observations.len(), 1);
    assert_eq!(sent.observations[0].client_record_id, MINE);

    // The file a lead scout is handed pushes as it is.
    let file = device.export_outbox(at(12)).unwrap();
    let exported: JsonValue = serde_json::from_str(&file).unwrap();
    assert_eq!(exported["format"], "tealteam-outbox");
    assert_eq!(exported["schema"], schema);
    assert_eq!(exported["observations"][1]["refused"], "Not saved: no.");
    assert_eq!(exported["observations"][1]["payload"]["notes"], "tippy");
    let push: Push = serde_json::from_str(&file).unwrap();
    assert_eq!(push.observations.len(), 2);

    assert!(
        !device.discard_refused(MINE).await.unwrap(),
        "still waiting"
    );
    assert!(device.discard_refused(OTHER).await.unwrap());
    assert_eq!(device.outbox().unwrap().len(), 1);
}

#[tokio::test]
async fn nothing_but_an_echo_empties_the_outbox() {
    let device = device("stops").await;
    let schema = device.schema().unwrap().unwrap();
    device
        .queue_observation(&observation(MINE, 254, 7), at(5))
        .await
        .unwrap();
    let stop = |reply: Result<(u16, JsonValue), &'static str>| {
        let device = &device;
        async move {
            let pi = Pi::answers([reply]);
            let report = SyncClient::new(&pi, vec![])
                .sync(device, at(10))
                .await
                .unwrap();
            assert_eq!(pi.paths().len(), 1, "stopped at the push");
            report.stopped
        }
    };

    assert_eq!(
        stop(Ok((
            409,
            json!({ "error": "schema mismatch", "action": "reload" })
        )))
        .await,
        Some(Stop::Reload)
    );
    assert_eq!(
        stop(Ok((409, json!({ "action": "server-behind" })))).await,
        Some(Stop::ServerBehind)
    );
    assert_eq!(stop(Ok((303, json!(null)))).await, Some(Stop::SignedOut));
    assert_eq!(stop(Ok((0, json!(null)))).await, Some(Stop::SignedOut));
    assert_eq!(
        stop(Ok((503, json!({ "error": "storage unavailable" })))).await,
        Some(Stop::Offline("storage unavailable".into()))
    );
    assert_eq!(
        stop(Err("no connection to the Pi")).await,
        Some(Stop::Offline("no connection to the Pi".into()))
    );
    assert_eq!(
        stop(Ok((422, json!({ "error": "not an outbox" })))).await,
        Some(Stop::Refused("not an outbox".into()))
    );
    let outbox = device.outbox().unwrap();
    assert_eq!(outbox.len(), 1);
    assert_eq!(outbox[0].attempts, 7);
    assert!(device.is_queued(MINE).unwrap());

    // A pull that finds the Pi on another schema stops too.
    let pi = Pi::answers([
        recorded(schema, &[MINE]),
        Ok((409, json!({ "action": "reload" }))),
    ]);
    let report = SyncClient::new(&pi, vec![])
        .sync(&device, at(11))
        .await
        .unwrap();
    assert_eq!(report.stopped, Some(Stop::Reload));
    // And a sign-in page where JSON should be is a lost session.
    let pi = Pi::answers([recorded(schema, &[MINE])]);
    pi.replies.borrow_mut().push_back(Ok(Reply {
        status: 200,
        body: "<!doctype html><title>Sign in</title>".into(),
    }));
    let report = SyncClient::new(&pi, vec![])
        .sync(&device, at(12))
        .await
        .unwrap();
    assert_eq!(report.stopped, Some(Stop::SignedOut));
    assert!(device.is_queued(MINE).unwrap());
}

#[tokio::test]
async fn the_pis_rows_win_and_deletions_arrive() {
    let device = device("apply").await;
    let schema = device.schema().unwrap().unwrap();
    // Scout 7 saved 254 here; the Pi already had another of theirs.
    device
        .queue_observation(&observation(MINE, 254, 7), at(5))
        .await
        .unwrap();

    let pick = |seq: i64, id: &str, picked: i32, position: i64| {
        json!({
            "seq": seq, "entity": "pick_list_entry", "entity_pk": id, "op": "upsert",
            "event_key": "2026here",
            "row": {
                "client_record_id": id, "owning_team": 10101, "event_key": "2026here",
                "picked_team": picked, "color": null, "crossed": 0, "position": position,
                "updated_at": "2026-03-14T10:07:00.000Z",
            },
        })
    };
    let changes = json!([
        echo(50, PIS, 254, 7, "pending"),
        {
            "seq": 51, "entity": "assignment", "entity_pk": "2026here_qm1:1678", "op": "upsert",
            "event_key": "2026here",
            "row": {
                "match_key": "2026here_qm1", "team_number": 1678, "event_key": "2026here",
                "scouter_id": 7, "device_id": null, "assigned_by": 2,
                "updated_at": "2026-03-14T10:07:00.000Z",
            },
        },
        pick(52, "p-1", 254, 1),
        // The same team again under another id: the Pi deleted and re-added.
        pick(53, "p-2", 254, 2),
        { "seq": 54, "entity": "pick_list_entry", "entity_pk": "p-9", "op": "delete",
          "event_key": "2026here", "row": null },
        { "seq": 55, "entity": "mystery", "entity_pk": "x", "op": "upsert",
          "event_key": null, "row": {} },
    ]);
    let pull = json!({
        "schema": schema, "build": "b", "form": 1,
        "changes": changes, "changes_cursor": 55, "changes_more": false,
        "upstream": [{
            "seq": 9, "api": "tba", "path": "/event/2026here/rankings", "etag": "W/\"9\"",
            "body": "{\"rankings\":[]}", "fetched_at": "2026-03-14T10:08:00Z",
        }],
        "upstream_cursor": 9, "upstream_more": false,
    });
    // Push first; the Pi refuses it, since it has Scout 7's already.
    let pi = Pi::answers([
        Ok((
            200,
            json!({ "schema": schema, "receipts": [
                { "client_record_id": MINE, "outcome": "refused", "reason": "Not saved: you already have one." },
            ]}),
        )),
        Ok((200, pull)),
    ]);
    let report = SyncClient::new(&pi, vec![])
        .sync(&device, at(10))
        .await
        .unwrap();
    assert_eq!(
        report.applied, 5,
        "the unknown entity is skipped: {report:?}"
    );
    assert_eq!(report.upstream, 1);
    assert_eq!(device.cursor(CHANGES_SOURCE).unwrap(), 55);
    assert_eq!(device.cursor(UPSTREAM_SOURCE).unwrap(), 9);

    let pending = device.pending_observations("2026here").await.unwrap();
    assert_eq!(pending.len(), 1, "one live observation per scout and robot");
    let assignments = device.event_assignments("2026here").await.unwrap();
    assert_eq!(assignments.len(), 1);
    assert_eq!(assignments[0].team_number, 1678);
    let list = device.pick_list(10101, "2026here").await.unwrap();
    assert_eq!(
        list.len(),
        1,
        "the Pi's latest row for 254 replaced the first"
    );
    assert!(
        device
            .latest_upstream("tba", "/event/2026here/rankings")
            .await
            .unwrap()
            .is_some()
    );

    // And a deletion of the assignment.
    let pi = Pi::answers([pulled(
        schema,
        json!([{ "seq": 56, "entity": "assignment", "entity_pk": "2026here_qm1:1678",
                 "op": "delete", "event_key": "2026here", "row": null }]),
        56,
    )]);
    SyncClient::new(&pi, vec![])
        .sync(&device, at(11))
        .await
        .unwrap();
    assert!(
        device
            .event_assignments("2026here")
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        pi.paths()[0].contains("changes=55&upstream=9"),
        "{:?}",
        pi.paths()
    );
}
