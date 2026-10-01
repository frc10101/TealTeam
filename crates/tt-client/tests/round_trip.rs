//! Every `Repo` method, on the server's repo and the browser's, from one
//! empty database (C11, REBUILD_SPEC.md §11).
//!
//! A database is migrated by the server and its file opened twice: by
//! `tt_repo_sqlite`, and by `tt_client` from the bytes. Each test makes the
//! same calls on both, and every answer must be the same, errors included.
//! Each ends by reading everything back from both, the change log too. So a
//! statement changed on one side and not the other fails here, whichever
//! side it was.
//!
//! The one difference meant to be there is foreign keys, off on the device:
//! `the_device_alone_takes_a_row_naming_someone_it_lacks` pins it. The last
//! test fails when the trait gains a method this file does not call.

#[macro_use]
mod common;

use std::collections::BTreeMap;

use chrono::{NaiveDate, TimeDelta};
use common::{Dir, at, event, game, observation, settled, team, upstream};
use tt_client::ClientRepo;
use tt_core::assignments::AssigneeKey;
use tt_core::picklist::{Edit, PickDoc};
use tt_core::records::TeamEventStats;
use tt_core::review::Decision;
use tt_core::season::WeightOverrides;
use tt_core::standings::{Record, Standing};
use tt_core::user::{Roles, Session, User};
use tt_repo::{LocalRepo, NewAssignment, NewUpstream, NewUser, Recorded, RepoError};
use tt_repo_sqlite::SqliteRepo;

/// One freshly migrated database, open on both sides.
struct Pair {
    dir: Dir,
    server: SqliteRepo,
    device: ClientRepo,
}

async fn fresh(test: &str) -> Pair {
    let dir = Dir::new(&format!("round-trip-{test}"));
    let blank = SqliteRepo::connect(&dir.url("blank.db")).unwrap();
    tt_repo_sqlite::migrate::apply(blank.pool()).await.unwrap();
    // Closing the last connection checkpoints the WAL into the file.
    blank.pool().close().await;
    let bytes = std::fs::read(dir.0.join("blank.db")).unwrap();
    let server = SqliteRepo::connect(&dir.url("blank.db")).unwrap();
    let device = ClientRepo::from_bytes(&bytes).expect("the device opens it");
    Pair {
        dir,
        server,
        device,
    }
}

const EVENTS: [&str; 3] = ["2026here", "2026away", "2026none"];
const TEAMS: [i32; 4] = [10101, 254, 1678, 4];

/// Every read, over every key these tests write, and the change log, then
/// every table row for row: a write no read shows, such as `record_login`,
/// must land alike too. Row ids are counted from 1 on a fresh database, so
/// the first few of each kind cover every row made here.
async fn everything_agrees(p: &Pair) {
    let date = NaiveDate::from_ymd_opt(2026, 3, 14).unwrap();
    assert_eq!(same!(p, health()), tt_repo::Health::Ready);
    same!(p, schema_version()).unwrap();
    same!(p, has_any_user()).unwrap();
    for email in ["sam@x", "lee@x", "nobody@x"] {
        same!(p, credentials_by_email(email)).unwrap();
    }
    for id in 1..=4 {
        same!(p, user_by_id(id)).unwrap();
        same!(p, password_hash(id)).unwrap();
        same!(p, observation(id)).unwrap();
    }
    for uuid in ["tablet-1", "tablet-2", "tablet-3"] {
        same!(p, device_by_uuid(uuid)).unwrap();
    }
    same!(p, list_devices()).unwrap();
    same!(p, list_scouts()).unwrap();
    same!(p, list_events()).unwrap();
    same!(p, active_events(date, 7)).unwrap();
    for number in TEAMS {
        same!(p, team(number)).unwrap();
        same!(p, events_for_team(number)).unwrap();
    }
    for key in EVENTS {
        same!(p, event(key)).unwrap();
        same!(p, event_teams(key)).unwrap();
        same!(p, event_matches(key)).unwrap();
        same!(p, event_assignments(key)).unwrap();
        same!(p, event_stats(key)).unwrap();
        same!(p, event_sightings(key)).unwrap();
        same!(p, pending_observations(key)).unwrap();
        same!(p, approved_observations(key)).unwrap();
        for number in TEAMS {
            same!(p, team_matches(key, number)).unwrap();
            same!(p, team_stats(key, number)).unwrap();
            same!(p, pick_list(number, key)).unwrap();
        }
        for scout in 1..=3 {
            same!(p, recorded_by(key, scout)).unwrap();
            same!(p, declined_for(key, scout)).unwrap();
        }
    }
    for game in ["2026here_qm1", "2026here_qm2", "2026away_qm1"] {
        same!(p, match_by_key(game)).unwrap();
        for scout in 1..=3 {
            same!(p, observed_teams(game, scout)).unwrap();
        }
    }
    same!(p, weight_overrides()).unwrap();
    same!(p, upstream_since(0, 100)).unwrap();
    same!(p, log_heads()).unwrap();
    assert_eq!(
        settled(&p.device).await,
        settled(&p.server).await,
        "changes_since answers differently on the device"
    );

    let device = p.dir.0.join("device.db");
    std::fs::write(&device, p.device.to_bytes().unwrap()).unwrap();
    let server = tables(&p.dir.0.join("blank.db"));
    let device = tables(&device);
    assert_eq!(
        device.keys().collect::<Vec<_>>(),
        server.keys().collect::<Vec<_>>()
    );
    for (table, rows) in &server {
        assert_eq!(&device[table], rows, "{table} differs on the device");
    }
}

/// Every table's rows, sorted, as text. `changes.created_at` is left out:
/// it is SQLite's clock, alike on the two sides and not equal.
fn tables(file: &std::path::Path) -> BTreeMap<String, Vec<String>> {
    let conn = rusqlite::Connection::open(file).unwrap();
    let names: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_schema WHERE type = 'table'")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    names
        .into_iter()
        .map(|table| {
            let mut statement = conn.prepare(&format!("SELECT * FROM \"{table}\"")).unwrap();
            let columns: Vec<String> = statement
                .column_names()
                .into_iter()
                .map(str::to_string)
                .collect();
            let mut rows: Vec<String> = statement
                .query_map([], |row| {
                    let mut cells = Vec::new();
                    for (i, column) in columns.iter().enumerate() {
                        if table == "changes" && column == "created_at" {
                            continue;
                        }
                        let cell: rusqlite::types::Value = row.get(i)?;
                        cells.push(format!("{column}={cell:?}"));
                    }
                    Ok(cells.join(", "))
                })
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap();
            rows.sort();
            (table, rows)
        })
        .collect()
}

fn new_user(email: &str, name: &str, team: Option<i32>, lead: bool) -> NewUser {
    NewUser {
        email: email.into(),
        name: name.into(),
        password_hash: format!("hash-of-{name}"),
        team_number: team,
        roles: Roles {
            is_lead_scout: lead,
            ..Roles::default()
        },
    }
}

/// Sam (a scout of 10101), Lee (a lead of 10101), and Ash (no team), on
/// both sides.
async fn people(p: &Pair) -> (User, User, User) {
    let sam = same!(
        p,
        create_user(new_user("sam@x", "Sam", Some(10101), false), at(0))
    )
    .unwrap();
    let lee = same!(
        p,
        create_user(new_user("lee@x", "Lee", Some(10101), true), at(0))
    )
    .unwrap();
    let ash = same!(p, create_user(new_user("ash@x", "Ash", None, false), at(0))).unwrap();
    (sam, lee, ash)
}

/// Two events, their rosters, and two matches at the first.
async fn schedule(p: &Pair) {
    same!(
        p,
        upsert_event(&event("2026here", Some((2026, 3, 13))), at(0))
    )
    .unwrap();
    same!(
        p,
        upsert_event(&event("2026away", Some((2026, 4, 1))), at(0))
    )
    .unwrap();
    for number in TEAMS {
        same!(
            p,
            upsert_team(&team(number, &format!("Team {number}")), at(0))
        )
        .unwrap();
        same!(p, link_event_team("2026here", number, at(0))).unwrap();
    }
    same!(
        p,
        upsert_match(&game("2026here", 1, [10101, 254, 1678], [4, 5, 6]), at(0))
    )
    .unwrap();
    same!(
        p,
        upsert_match(&game("2026here", 2, [4, 5, 6], [10101, 254, 1678]), at(0))
    )
    .unwrap();
}

#[tokio::test]
async fn accounts_sessions_and_devices_round_trip() {
    let p = fresh("accounts").await;
    assert!(!same!(p, has_any_user()).unwrap());
    let (sam, lee, ash) = people(&p).await;
    assert!(same!(p, has_any_user()).unwrap());
    assert!(matches!(
        same!(
            p,
            create_user(new_user("sam@x", "Sam again", None, false), at(1))
        ),
        Err(RepoError::Conflict { .. })
    ));

    same!(p, set_password_hash(sam.id, "new-hash", at(2))).unwrap();
    assert_eq!(
        same!(p, password_hash(sam.id)).unwrap().as_deref(),
        Some("new-hash")
    );
    same!(p, record_login(lee.id, at(3))).unwrap();
    same!(p, credentials_by_email("lee@x")).unwrap();

    let session = |id: &str, user: &User, minutes: i64| Session {
        id: id.into(),
        user_id: user.id,
        expires_at: at(0) + TimeDelta::minutes(minutes),
    };
    same!(p, create_session(&session("s-sam", &sam, 30), at(0))).unwrap();
    same!(p, create_session(&session("s-lee", &lee, 10), at(0))).unwrap();
    same!(p, create_session(&session("s-ash", &ash, 20), at(0))).unwrap();
    assert!(same!(p, session_user("s-sam", at(5))).unwrap().is_some());
    assert!(
        same!(p, session_user("s-lee", at(15))).unwrap().is_none(),
        "expired, and deleted by the read"
    );
    assert_eq!(same!(p, purge_expired_sessions(at(25))).unwrap(), 1);
    same!(p, delete_session("s-sam")).unwrap();
    assert!(same!(p, session_user("s-sam", at(6))).unwrap().is_none());

    let tablet = same!(p, touch_device("tablet-1", Some(&sam), at(4))).unwrap();
    same!(p, touch_device("tablet-1", Some(&ash), at(5))).unwrap();
    same!(p, touch_device("tablet-2", None, at(5))).unwrap();
    same!(p, record_clock_offset("tablet-1", -1500, at(6))).unwrap();
    same!(p, record_clock_offset("tablet-3", 20, at(6))).unwrap();
    same!(p, rename_device(tablet.id, "Stands Left", at(7))).unwrap();

    everything_agrees(&p).await;
}

#[tokio::test]
async fn the_competition_graph_round_trips() {
    let p = fresh("graph").await;
    schedule(&p).await;
    // Upserts again, changed.
    let mut renamed = event("2026here", Some((2026, 3, 13)));
    renamed.name = "Here, renamed".into();
    same!(p, upsert_event(&renamed, at(1))).unwrap();
    same!(p, upsert_team(&team(254, "Poofs"), at(1))).unwrap();
    same!(p, link_event_team("2026here", 254, at(1))).unwrap();
    same!(p, link_event_team("2026away", 254, at(1))).unwrap();
    let mut played = game("2026here", 2, [4, 5, 6], [10101, 254, 1678]);
    played.played = true;
    played.actual_at = Some(at(12));
    same!(p, upsert_match(&played, at(12))).unwrap();
    same!(
        p,
        upsert_match(&game("2026away", 1, [254, 5, 6], [7, 8, 9]), at(12))
    )
    .unwrap();

    for (number, rank) in [(254, 1), (10101, 2), (4, 3)] {
        let stats = TeamEventStats {
            team_number: number,
            event_key: "2026here".into(),
            opr: Some(40.0 + f64::from(rank)),
            rank: Some(rank),
            wins: Some(3),
            synced_at: Some(at(13)),
            ..TeamEventStats::default()
        };
        same!(p, upsert_team_stats(&stats, at(13))).unwrap();
    }
    let typed = [
        Standing {
            rank: 1,
            team_number: 1678,
            ranking_score: Some(2.5),
            record: Some(Record {
                wins: 4,
                losses: 1,
                ties: 0,
            }),
        },
        Standing {
            rank: 2,
            team_number: 254,
            ranking_score: None,
            record: None,
        },
    ];
    same!(p, record_standings("2026here", &typed, at(14))).unwrap();

    everything_agrees(&p).await;
}

#[tokio::test]
async fn assignments_round_trip_with_names_on_both() {
    let p = fresh("assignments").await;
    let (sam, lee, ash) = people(&p).await;
    schedule(&p).await;
    let tablet = same!(p, touch_device("tablet-1", None, at(0))).unwrap();
    let assign = |game: &str, team: i32, assignee: AssigneeKey| NewAssignment {
        match_key: game.into(),
        event_key: "2026here".into(),
        team_number: team,
        assignee,
    };

    let first = [
        assign("2026here_qm1", 254, AssigneeKey::Scout(sam.id)),
        assign("2026here_qm1", 1678, AssigneeKey::Device(tablet.id)),
        assign("2026here_qm1", 4, AssigneeKey::Scout(ash.id)),
        assign("2026here_qm2", 254, AssigneeKey::Scout(sam.id)),
        assign("2026here_qm2", 4, AssigneeKey::Scout(ash.id)),
    ];
    same!(p, set_assignments(&first, lee.id, at(1))).unwrap();
    // Whoever had it is replaced.
    let swap = [assign("2026here_qm1", 254, AssigneeKey::Scout(ash.id))];
    same!(p, set_assignments(&swap, lee.id, at(2))).unwrap();
    same!(p, unassign("2026here_qm1", 1678)).unwrap();
    same!(p, unassign("2026here_qm1", 1678)).unwrap();
    assert_eq!(same!(p, event_assignments("2026here")).unwrap().len(), 4);
    assert_eq!(
        same!(p, clear_assignments("2026here", Some("2026here_qm2"))).unwrap(),
        2
    );
    assert_eq!(same!(p, clear_assignments("2026here", None)).unwrap(), 2);
    same!(p, set_assignments(&first[..2], lee.id, at(3))).unwrap();

    everything_agrees(&p).await;
}

#[tokio::test]
async fn observations_and_review_round_trip() {
    let p = fresh("review").await;
    let (sam, lee, ash) = people(&p).await;
    schedule(&p).await;

    let mine = observation("o-1", "2026here_qm1", 254, Some(sam.id));
    let Recorded::Created(first) = same!(p, record_observation(&mine, at(5))).unwrap() else {
        panic!("recorded");
    };
    assert!(matches!(
        same!(p, record_observation(&mine, at(6))),
        Ok(Recorded::Duplicate(_))
    ));
    let again = observation("o-1b", "2026here_qm1", 254, Some(sam.id));
    assert!(matches!(
        same!(p, record_observation(&again, at(6))),
        Err(RepoError::Conflict { .. })
    ));
    let other = observation("o-2", "2026here_qm1", 1678, Some(sam.id));
    let Recorded::Created(second) = same!(p, record_observation(&other, at(7))).unwrap() else {
        panic!("recorded");
    };
    // No team on the row: approval fills it in from the scout's.
    let mut unteamed = observation("o-3", "2026here_qm2", 4, Some(ash.id));
    unteamed.submitting_team = None;
    let Recorded::Created(third) = same!(p, record_observation(&unteamed, at(8))).unwrap() else {
        panic!("recorded");
    };

    assert!(
        same!(
            p,
            review_observation(first, &Decision::Approve, lee.id, at(10))
        )
        .unwrap()
    );
    assert!(
        !same!(
            p,
            review_observation(first, &Decision::Approve, lee.id, at(11))
        )
        .unwrap(),
        "already reviewed"
    );
    let decline = Decision::Decline("wrong robot".into());
    assert!(same!(p, review_observation(second, &decline, lee.id, at(12))).unwrap());
    assert!(
        same!(
            p,
            review_observation(third, &Decision::Approve, lee.id, at(13))
        )
        .unwrap()
    );
    assert_eq!(same!(p, declined_for("2026here", sam.id)).unwrap().len(), 1);
    // A declined robot may be recorded again, and then it is not owed news.
    let redo = observation("o-2b", "2026here_qm1", 1678, Some(sam.id));
    same!(p, record_observation(&redo, at(14))).unwrap();
    assert!(
        same!(p, declined_for("2026here", sam.id))
            .unwrap()
            .is_empty()
    );

    let mut weights = WeightOverrides::new();
    weights.set("climb", "high", 15);
    weights.set("climb", "low", 3);
    same!(p, replace_weight_overrides(&weights, at(15))).unwrap();
    same!(p, weight_overrides()).unwrap();
    let mut fewer = WeightOverrides::new();
    fewer.set("auto", "leave", 2);
    same!(p, replace_weight_overrides(&fewer, at(16))).unwrap();

    everything_agrees(&p).await;
}

#[tokio::test]
async fn pick_lists_round_trip() {
    let p = fresh("picks").await;
    schedule(&p).await;
    assert!(same!(p, pick_list(10101, "2026here")).unwrap().is_empty());

    // Made on a lead's copy, merged into both.
    let mut copy = PickDoc::new();
    for team in [254, 1678, 4] {
        let add = Edit::Add {
            team,
            record_id: format!("pick-{team}"),
        };
        let update = copy.apply(&add, &[]).unwrap().unwrap();
        same!(p, merge_pick_list(10101, "2026here", &update, at(1))).unwrap();
    }
    let state = same!(p, pick_list_doc(10101, "2026here", at(2))).unwrap();
    let mut read = PickDoc::load(&state).unwrap();
    for edit in [
        Edit::Up { team: 4 },
        Edit::Cross {
            team: 1678,
            crossed: true,
        },
        Edit::MoveTo {
            team: 254,
            place: 3,
        },
    ] {
        let update = read.apply(&edit, &[]).unwrap().unwrap();
        same!(p, merge_pick_list(10101, "2026here", &update, at(3))).unwrap();
        // Merged twice, it lands once.
        same!(p, merge_pick_list(10101, "2026here", &update, at(4))).unwrap();
    }
    // Made on the older copy, which never saw those edits: it still lands.
    let remove = copy
        .apply(&Edit::Remove { team: 254 }, &[])
        .unwrap()
        .unwrap();
    let list = same!(p, merge_pick_list(10101, "2026here", &remove, at(5))).unwrap();
    let order: Vec<_> = list.iter().map(|e| (e.team_number, e.crossed)).collect();
    assert_eq!(order, [(4, false), (1678, true)]);
    assert!(
        same!(
            p,
            merge_pick_list(10101, "2026here", b"not an update", at(6))
        )
        .is_err()
    );
    // Another team's list, kept apart.
    let mut theirs = PickDoc::new();
    let add = Edit::Add {
        team: 10101,
        record_id: "pick-theirs".into(),
    };
    let update = theirs.apply(&add, &[]).unwrap().unwrap();
    same!(p, merge_pick_list(254, "2026here", &update, at(7))).unwrap();

    everything_agrees(&p).await;
}

#[tokio::test]
async fn the_upstream_log_round_trips() {
    let p = fresh("upstream").await;
    let rankings = "/event/2026here/rankings";
    for n in 0..7 {
        let body = format!("{{\"n\":{n}}}");
        assert!(
            same!(p, append_upstream(&upstream(rankings, &body)))
                .unwrap()
                .is_some()
        );
    }
    assert_eq!(
        same!(p, append_upstream(&upstream(rankings, "{\"n\":6}"))).unwrap(),
        None,
        "the same body again is not appended"
    );
    let first = NewUpstream {
        api: "first".into(),
        path: "/2026/schedule/HERE?tournamentLevel=qual".into(),
        etag: None,
        body: "{\"Schedule\":[]}".into(),
        fetched_at: at(2),
        via: "device-7".into(),
    };
    same!(p, append_upstream(&first)).unwrap();

    assert_eq!(
        same!(p, upstream_since(0, 100)).unwrap().len(),
        tt_repo::UPSTREAM_KEEP_PER_PATH as usize + 1,
        "pruned to the newest five of a path"
    );
    same!(p, upstream_since(4, 2)).unwrap();
    same!(p, latest_upstream("tba", rankings)).unwrap();
    same!(p, latest_upstream("first", &first.path)).unwrap();
    assert!(
        same!(p, latest_upstream("first", rankings))
            .unwrap()
            .is_none()
    );

    everything_agrees(&p).await;
}

#[tokio::test]
async fn the_device_alone_takes_a_row_naming_someone_it_lacks() {
    // Foreign keys are off on the device, so a scout who signed up after the
    // snapshot can still record (C4). The server checks the key when the row
    // arrives (C7). This is the only answer the two may give differently.
    let p = fresh("keys").await;
    schedule(&p).await;
    let stranger = observation("o-x", "2026here_qm1", 254, Some(42));
    assert!(p.server.record_observation(&stranger, at(1)).await.is_err());
    assert!(matches!(
        p.device.record_observation(&stranger, at(1)).await,
        Ok(Recorded::Created(_))
    ));
}

#[test]
fn every_repo_method_is_called_on_both_here() {
    let trait_src = include_str!("../../tt-repo/src/lib.rs");
    let ours: String = include_str!("round_trip.rs").split_whitespace().collect();
    let block = &trait_src[trait_src.find("pub trait LocalRepo").unwrap()..];
    let block = &block[..block.find("\n}\n").unwrap()];
    let methods: Vec<&str> = block
        .split("async fn ")
        .skip(1)
        .map(|rest| &rest[..rest.find('(').unwrap()])
        .collect();
    assert!(methods.len() > 50, "found {methods:?}");
    // `changes_since` is compared by `settled`, without SQLite's clock.
    let missing: Vec<_> = methods
        .iter()
        .filter(|&&m| m != "changes_since" && !ours.contains(&format!("same!(p,{m}(")))
        .collect();
    assert!(
        missing.is_empty(),
        "no round trip for {missing:?}: call each on both with `same!`"
    );
}
