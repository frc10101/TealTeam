//! The browser's repo against the server's, on one snapshot (C4).
//!
//! A Pi's database is seeded through the server's own `Repo`, cut into a
//! snapshot for team 10101 at 2026here as `/api/sync/snapshot` would, and
//! then opened twice: by `tt_repo_sqlite` from a file, and by `tt_client`
//! from the bytes, as a phone opens it from OPFS. Every read must answer the
//! same, and so must the writes both sides can make (C11 widens this to
//! every method, on any database).

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use tt_client::ClientRepo;
use tt_core::assignments::{Assignee, AssigneeKey};
use tt_core::matches::CompLevel;
use tt_core::picklist::{Edit, PickDoc};
use tt_core::records::{Event, MatchRecord, Team, TeamEventStats};
use tt_core::review::Decision;
use tt_core::season::{Payload, Value, WeightOverrides};
use tt_core::standings::{Record, Standing};
use tt_core::user::Roles;
use tt_repo::{
    LocalRepo, NewAssignment, NewObservation, NewUpstream, NewUser, Recorded, RepoError,
};
use tt_repo_sqlite::SqliteRepo;
use tt_repo_sqlite::snapshot::{self, Audience};

fn at(minute: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 14, 10, minute, 0).unwrap()
}

/// Long after anything here was written: every change has settled.
fn later() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2100, 1, 1, 0, 0, 0).unwrap()
}

/// Team 10101 at 2026here, with other teams' notes hidden, as the pull
/// hides them.
struct Ours;

impl Audience for Ours {
    fn team(&self) -> Option<i32> {
        Some(10101)
    }
    fn has_event(&self, event_key: &str) -> bool {
        event_key == "2026here"
    }
    fn has_upstream(&self, path: &str) -> bool {
        !path.starts_with("/event/2026away")
    }
    fn answers(&self, writer: Option<i32>, _: &str) -> Option<String> {
        (writer != Some(10101)).then(|| r#"{"hidden":true}"#.into())
    }
}

/// A directory of the test's own, removed on drop.
struct Dir(PathBuf);

impl Dir {
    fn new(test: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("tt-client-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Dir(dir)
    }

    fn url(&self, file: &str) -> String {
        format!("sqlite://{}", self.0.join(file).display())
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn event(key: &str, start: Option<(i32, u32, u32)>) -> Event {
    let date = start.map(|(y, m, d)| NaiveDate::from_ymd_opt(y, m, d).unwrap());
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

fn game(event_key: &str, number: i32, red: [i32; 3], blue: [i32; 3]) -> MatchRecord {
    MatchRecord {
        key: format!("{event_key}_qm{number}"),
        event_key: event_key.into(),
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

fn observation(id: &str, match_key: &str, team: i32, scouter: Option<i64>) -> NewObservation {
    let mut payload = Payload::new();
    payload.insert("notes".into(), Value::Text(format!("notes on {team}")));
    payload.insert("cycles".into(), Value::Count(4));
    NewObservation {
        client_record_id: id.into(),
        match_key: match_key.into(),
        event_key: match_key.split('_').next().unwrap().into(),
        team_number: team,
        alliance: "red",
        payload,
        schema_version: 1,
        scouter_id: scouter,
        device_id: None,
        submitting_team: scouter.map(|_| 10101),
        observed_at: at(5),
    }
}

fn upstream(path: &str, body: &str) -> NewUpstream {
    NewUpstream {
        api: "tba".into(),
        path: path.into(),
        etag: Some("W/\"1\"".into()),
        body: body.into(),
        fetched_at: at(1),
        via: "pi".into(),
    }
}

/// The Pi's database, through the server's own writes. Returns the user ids
/// of Sam (a scout) and Lee (a lead), and the tablet's device id.
async fn seed(pi: &SqliteRepo) -> (i64, i64, i64) {
    let user = |email: &str, name: &str, lead: bool| NewUser {
        email: email.into(),
        name: name.into(),
        password_hash: "SECRET-HASH".into(),
        team_number: Some(10101),
        roles: Roles {
            is_lead_scout: lead,
            ..Roles::default()
        },
    };
    let sam = pi
        .create_user(user("sam@x", "Sam", false), at(0))
        .await
        .unwrap();
    let lee = pi
        .create_user(user("lee@x", "Lee", true), at(0))
        .await
        .unwrap();
    let tablet = pi
        .touch_device("tablet-1", Some(&sam), at(0))
        .await
        .unwrap();

    pi.upsert_event(&event("2026here", Some((2026, 3, 13))), at(0))
        .await
        .unwrap();
    pi.upsert_event(&event("2026away", Some((2026, 4, 1))), at(0))
        .await
        .unwrap();
    for (number, name) in [
        (10101, "Teal"),
        (254, "Poofs"),
        (1678, "Citrus"),
        (4, "Four"),
    ] {
        pi.upsert_team(&team(number, name), at(0)).await.unwrap();
    }
    for number in [10101, 254, 1678] {
        pi.link_event_team("2026here", number, at(0)).await.unwrap();
    }
    pi.link_event_team("2026away", 254, at(0)).await.unwrap();
    pi.upsert_match(&game("2026here", 1, [10101, 254, 1678], [4, 5, 6]), at(0))
        .await
        .unwrap();
    pi.upsert_match(&game("2026here", 2, [4, 5, 6], [10101, 254, 1678]), at(0))
        .await
        .unwrap();
    pi.upsert_match(&game("2026away", 1, [254, 5, 6], [7, 8, 9]), at(0))
        .await
        .unwrap();
    for (number, rank) in [(254, 1), (10101, 2)] {
        let stats = TeamEventStats {
            team_number: number,
            event_key: "2026here".into(),
            opr: Some(50.5),
            rank: Some(rank),
            wins: Some(3),
            synced_at: Some(at(1)),
            ..TeamEventStats::default()
        };
        pi.upsert_team_stats(&stats, at(1)).await.unwrap();
    }

    pi.set_assignments(
        &[
            NewAssignment {
                match_key: "2026here_qm1".into(),
                event_key: "2026here".into(),
                team_number: 254,
                assignee: AssigneeKey::Scout(sam.id),
            },
            NewAssignment {
                match_key: "2026here_qm1".into(),
                event_key: "2026here".into(),
                team_number: 1678,
                assignee: AssigneeKey::Device(tablet.id),
            },
        ],
        lee.id,
        at(1),
    )
    .await
    .unwrap();

    let approved = observation("o-approved", "2026here_qm1", 254, Some(sam.id));
    let Recorded::Created(id) = pi.record_observation(&approved, at(6)).await.unwrap() else {
        panic!("recorded");
    };
    assert!(
        pi.review_observation(id, &Decision::Approve, lee.id, at(7))
            .await
            .unwrap()
    );
    let declined = observation("o-declined", "2026here_qm1", 1678, Some(sam.id));
    let Recorded::Created(id) = pi.record_observation(&declined, at(6)).await.unwrap() else {
        panic!("recorded");
    };
    let decline = Decision::Decline("wrong robot".into());
    assert!(
        pi.review_observation(id, &decline, lee.id, at(8))
            .await
            .unwrap()
    );
    // Another team's scout, whose notes this viewer may not read.
    let mut theirs = observation("o-theirs", "2026here_qm1", 10101, None);
    theirs.submitting_team = Some(254);
    pi.record_observation(&theirs, at(6)).await.unwrap();
    pi.record_observation(
        &observation("o-away", "2026away_qm1", 254, Some(sam.id)),
        at(6),
    )
    .await
    .unwrap();

    for (owner, picks) in [(10101, [254, 1678]), (254, [10101, 1678])] {
        let mut doc = PickDoc::new();
        for team in picks {
            let add = Edit::Add {
                team,
                record_id: format!("pick-{owner}-{team}"),
            };
            let update = doc.apply(&add, &[]).unwrap().unwrap();
            pi.merge_pick_list(owner, "2026here", &update, at(9))
                .await
                .unwrap();
        }
    }

    for (path, body) in [
        ("/event/2026here/rankings", "{\"old\":1}"),
        ("/event/2026here/rankings", "{\"new\":1}"),
        ("/event/2026away/rankings", "{\"away\":1}"),
        ("/events/2026", "[]"),
    ] {
        pi.append_upstream(&upstream(path, body)).await.unwrap();
    }

    let mut weights = WeightOverrides::new();
    weights.set("climb", "high", 15);
    pi.replace_weight_overrides(&weights, at(9)).await.unwrap();

    (sam.id, lee.id, tablet.id)
}

/// The Pi seeded, and a snapshot of it for [`Ours`]: the server reading the
/// snapshot from a file, and the device reading the same bytes.
struct Fixture {
    _dir: Dir,
    server: SqliteRepo,
    device: ClientRepo,
    bytes: Vec<u8>,
    sam: i64,
    lee: i64,
    tablet: i64,
}

async fn fixture(test: &str) -> Fixture {
    let dir = Dir::new(test);
    let pi = SqliteRepo::connect(&dir.url("pi.db")).unwrap();
    tt_repo_sqlite::migrate::apply(pi.pool()).await.unwrap();
    let (sam, lee, tablet) = seed(&pi).await;
    let snap = snapshot::build(&pi, &Ours).await.expect("snapshot");

    std::fs::write(dir.0.join("snapshot.db"), &snap.bytes).unwrap();
    let server = SqliteRepo::connect(&dir.url("snapshot.db")).unwrap();
    let device = ClientRepo::from_bytes(&snap.bytes).expect("the device opens it as is");
    Fixture {
        _dir: dir,
        server,
        device,
        bytes: snap.bytes,
        sam,
        lee,
        tablet,
    }
}

/// The same call on both, answering the same, `Debug` for `Debug`.
macro_rules! same {
    ($f:expr, $method:ident($($arg:expr),* $(,)?)) => {{
        let server = $f.server.$method($($arg),*).await;
        let device = $f.device.$method($($arg),*).await;
        assert_eq!(
            format!("{device:?}"),
            format!("{server:?}"),
            "{} answers differently on the device",
            stringify!($method($($arg),*))
        );
        device
    }};
}

/// Every read but the assignment grid, which the server cannot read from a
/// snapshot at all: see `the_assignment_grid_reads_without_the_names`.
async fn every_read_agrees(f: &Fixture) {
    let date = NaiveDate::from_ymd_opt(2026, 3, 14).unwrap();
    assert_eq!(same!(f, health()), tt_repo::Health::Ready);
    same!(f, schema_version()).unwrap();
    same!(f, has_any_user()).unwrap();
    same!(f, credentials_by_email("sam@x")).unwrap();
    same!(f, user_by_id(f.sam)).unwrap();
    same!(f, password_hash(f.sam)).unwrap();
    same!(f, device_by_uuid("tablet-1")).unwrap();
    same!(f, list_devices()).unwrap();
    same!(f, list_scouts()).unwrap();
    same!(f, list_events()).unwrap();
    same!(f, event("2026here")).unwrap();
    same!(f, event("2026away")).unwrap();
    same!(f, events_for_team(254)).unwrap();
    same!(f, active_events(date, 7)).unwrap();
    same!(f, team(254)).unwrap();
    same!(f, event_teams("2026here")).unwrap();
    same!(f, event_teams("2026away")).unwrap();
    same!(f, match_by_key("2026here_qm1")).unwrap();
    same!(f, match_by_key("2026away_qm1")).unwrap();
    same!(f, event_matches("2026here")).unwrap();
    same!(f, team_matches("2026here", 254)).unwrap();
    same!(f, team_stats("2026here", 254)).unwrap();
    same!(f, event_stats("2026here")).unwrap();
    same!(f, observed_teams("2026here_qm1", f.sam)).unwrap();
    same!(f, recorded_by("2026here", f.sam)).unwrap();
    same!(f, event_sightings("2026here")).unwrap();
    same!(f, weight_overrides()).unwrap();
    let pending = same!(f, pending_observations("2026here")).unwrap();
    let approved = same!(f, approved_observations("2026here")).unwrap();
    for o in pending.iter().chain(&approved) {
        same!(f, observation(o.id)).unwrap();
    }
    same!(f, declined_for("2026here", f.sam)).unwrap();
    same!(f, pick_list(10101, "2026here")).unwrap();
    same!(f, pick_list(254, "2026here")).unwrap();
    same!(f, upstream_since(0, 100)).unwrap();
    same!(f, latest_upstream("tba", "/event/2026here/rankings")).unwrap();
    same!(f, latest_upstream("tba", "/event/2026away/rankings")).unwrap();
    same!(f, log_heads()).unwrap();
    assert_eq!(
        settled(&f.device).await,
        settled(&f.server).await,
        "changes_since answers differently on the device"
    );
}

/// The change log without its times, which are SQLite's clock: alike on the
/// two sides, not equal.
async fn settled(repo: &impl LocalRepo) -> Vec<(i64, String, String, String, Option<String>)> {
    repo.changes_since(0, 100, later())
        .await
        .unwrap()
        .into_iter()
        .map(|c| (c.seq, c.entity, c.entity_pk, c.op, c.payload))
        .collect()
}

#[tokio::test]
async fn a_snapshot_opens_as_is_and_reads_as_the_server_reads_it() {
    let f = fixture("reads").await;
    every_read_agrees(&f).await;

    // What the cut left, as the device sees it.
    let approved = f.device.approved_observations("2026here").await.unwrap();
    assert_eq!(approved.len(), 1, "o-away is another event's");
    assert_eq!(approved[0].scouter_name, None, "no users on a device");
    let pending = f.device.pending_observations("2026here").await.unwrap();
    assert_eq!(pending[0].payload.get("hidden"), Some(&Value::Flag(true)));
    assert!(
        !pending[0].payload.contains_key("notes"),
        "another team's notes are not on the device"
    );
    assert!(f.device.event_matches("2026away").await.unwrap().is_empty());
    assert_eq!(
        f.device.pick_list(10101, "2026here").await.unwrap().len(),
        2
    );
    assert!(
        f.device
            .pick_list(254, "2026here")
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        f.device.schema().unwrap(),
        Some(tt_repo_sqlite::migrate::latest()),
        "the schema the sync compares (S11)"
    );
    assert!(f.device.is_saved(), "opening writes nothing");
}

#[tokio::test]
async fn the_assignment_grid_reads_without_the_names() {
    let f = fixture("grid").await;
    let grid = f.device.event_assignments("2026here").await.unwrap();
    let assignees: Vec<_> = grid.iter().map(|a| (a.team_number, &a.assignee)).collect();
    assert_eq!(
        assignees,
        [
            (
                254,
                &Assignee::Scout {
                    id: f.sam,
                    name: format!("Scout {}", f.sam)
                }
            ),
            (
                1678,
                &Assignee::Device {
                    id: f.tablet,
                    name: format!("Device {}", f.tablet)
                }
            ),
        ]
    );
}

#[tokio::test]
async fn writes_land_on_the_device_as_on_the_server() {
    let f = fixture("writes").await;

    same!(
        f,
        upsert_event(&event("2026next", Some((2026, 3, 20))), at(20))
    )
    .unwrap();
    same!(f, upsert_team(&team(9999, "New"), at(20))).unwrap();
    same!(f, link_event_team("2026here", 9999, at(20))).unwrap();
    same!(
        f,
        upsert_match(&game("2026here", 3, [9999, 254, 1678], [4, 5, 6]), at(20))
    )
    .unwrap();
    let stats = TeamEventStats {
        team_number: 9999,
        event_key: "2026here".into(),
        opr: Some(12.0),
        ..TeamEventStats::default()
    };
    same!(f, upsert_team_stats(&stats, at(20))).unwrap();
    let typed = [Standing {
        rank: 1,
        team_number: 1678,
        ranking_score: Some(2.5),
        record: Some(Record {
            wins: 4,
            losses: 1,
            ties: 0,
        }),
    }];
    same!(f, record_standings("2026here", &typed, at(21))).unwrap();

    // No scout named: the server's copy enforces its keys, and has no users.
    let new = observation("o-new", "2026here_qm2", 4, None);
    assert!(matches!(
        same!(f, record_observation(&new, at(22))),
        Ok(Recorded::Created(_))
    ));
    assert!(matches!(
        same!(f, record_observation(&new, at(23))),
        Ok(Recorded::Duplicate(_))
    ));
    same!(
        f,
        append_upstream(&upstream("/event/2026here/matches", "[1]"))
    )
    .unwrap();
    assert_eq!(
        same!(
            f,
            append_upstream(&upstream("/event/2026here/matches", "[1]"))
        )
        .unwrap(),
        None,
        "the same body again is not appended"
    );
    let mut weights = WeightOverrides::new();
    weights.set("climb", "low", 3);
    same!(f, replace_weight_overrides(&weights, at(24))).unwrap();
    same!(f, unassign("2026here_qm1", 1678)).unwrap();
    same!(f, clear_assignments("2026here", Some("2026here_qm2"))).unwrap();

    assert!(
        !settled(&f.device).await.is_empty(),
        "the triggers came with the file"
    );

    every_read_agrees(&f).await;
}

#[tokio::test]
async fn a_scout_with_no_signal_records_what_the_file_has_no_user_for() {
    // The reason foreign keys are off on the device: Sam's row never left
    // the Pi, and Sam's observation still names Sam.
    let f = fixture("offline").await;
    assert_eq!(f.device.user_by_id(f.sam).await.unwrap(), None);

    let mine = observation("o-mine", "2026here_qm2", 254, Some(f.sam));
    let recorded = f.device.record_observation(&mine, at(30)).await.unwrap();
    assert!(matches!(recorded, Recorded::Created(_)));
    assert!(
        f.device
            .recorded_by("2026here", f.sam)
            .await
            .unwrap()
            .contains(&("2026here_qm2".into(), 254))
    );
    let conflict = observation("o-again", "2026here_qm2", 254, Some(f.sam));
    assert!(matches!(
        f.device.record_observation(&conflict, at(31)).await,
        Err(RepoError::Conflict { .. })
    ));

    // A lead on the device, the same way.
    let Recorded::Created(id) = recorded else {
        unreachable!()
    };
    assert!(
        f.device
            .review_observation(id, &Decision::Approve, f.lee, at(32))
            .await
            .unwrap()
    );
    f.device
        .set_assignments(
            &[NewAssignment {
                match_key: "2026here_qm2".into(),
                event_key: "2026here".into(),
                team_number: 4,
                assignee: AssigneeKey::Scout(f.sam),
            }],
            f.lee,
            at(33),
        )
        .await
        .unwrap();

    let changes = f.device.changes_since(0, 100, later()).await.unwrap();
    let entities: Vec<_> = changes
        .iter()
        .map(|c| (c.entity.as_str(), c.entity_pk.as_str()))
        .collect();
    assert_eq!(
        entities,
        [
            ("observation", "o-mine"),
            ("observation", "o-mine"),
            ("assignment", "2026here_qm2:4"),
        ],
        "what this device changed since its snapshot, ready to push (C7)"
    );
}

#[tokio::test]
async fn a_pick_list_change_on_the_device_reads_back() {
    let f = fixture("picks").await;
    // No document came with the snapshot: the first read makes one from the
    // rows, as the server does for a list stored before L14.
    let state = f
        .device
        .pick_list_doc(10101, "2026here", at(40))
        .await
        .unwrap();
    let mut doc = PickDoc::load(&state).unwrap();
    let update = doc
        .apply(&Edit::Up { team: 1678 }, &[])
        .unwrap()
        .expect("a change");
    let list = f
        .device
        .merge_pick_list(10101, "2026here", &update, at(41))
        .await
        .unwrap();
    let order: Vec<i32> = list.iter().map(|e| e.team_number).collect();
    assert_eq!(order, [1678, 254]);
    let read: Vec<i32> = f
        .device
        .pick_list(10101, "2026here")
        .await
        .unwrap()
        .iter()
        .map(|e| e.team_number)
        .collect();
    assert_eq!(read, order);
}

/// A saver that keeps what it was given, and fails when told to.
#[derive(Clone, Default)]
struct Kept {
    files: Rc<RefCell<Vec<Vec<u8>>>>,
    broken: Rc<RefCell<bool>>,
}

impl Kept {
    fn saver(&self) -> tt_client::Saver {
        let kept = self.clone();
        Box::new(move |bytes| {
            let kept = kept.clone();
            Box::pin(async move {
                if *kept.broken.borrow() {
                    return Err(RepoError::Unavailable("the disk is full".into()));
                }
                kept.files.borrow_mut().push(bytes);
                Ok(())
            })
        })
    }

    fn count(&self) -> usize {
        self.files.borrow().len()
    }
}

#[tokio::test]
async fn every_change_is_saved_and_a_read_is_not() {
    let f = fixture("saving").await;
    let kept = Kept::default();
    let device = ClientRepo::from_bytes(&f.bytes)
        .unwrap()
        .saving_with(kept.saver());

    device.event_matches("2026here").await.unwrap();
    device.pick_list(10101, "2026here").await.unwrap();
    assert_eq!(kept.count(), 0, "reads write nothing");

    let mine = observation("o-mine", "2026here_qm2", 254, Some(f.sam));
    device.record_observation(&mine, at(30)).await.unwrap();
    assert_eq!(kept.count(), 1);
    assert!(device.is_saved());
    // The same post again stores nothing, so saves nothing.
    device.record_observation(&mine, at(31)).await.unwrap();
    assert_eq!(kept.count(), 1);

    // What was saved is a database with the change in it.
    let reopened = ClientRepo::from_bytes(&kept.files.borrow()[0]).unwrap();
    assert_eq!(
        reopened.recorded_by("2026here", f.sam).await.unwrap(),
        [("2026here_qm1".into(), 254), ("2026here_qm2".into(), 254)]
    );

    // A failed save is the write's error, and the next change saves both.
    *kept.broken.borrow_mut() = true;
    let err = device
        .upsert_team(&team(9999, "New"), at(32))
        .await
        .unwrap_err();
    assert!(matches!(err, RepoError::Unavailable(_)), "{err}");
    assert!(!device.is_saved());
    *kept.broken.borrow_mut() = false;
    device.flush().await.unwrap();
    assert!(device.is_saved());
    let reopened = ClientRepo::from_bytes(kept.files.borrow().last().unwrap()).unwrap();
    assert!(reopened.team(9999).await.unwrap().is_some());
}

#[tokio::test]
async fn what_is_not_a_database_is_refused() {
    for junk in [
        &b""[..],
        b"half a file",
        b"SQLite format 3\0 and then nothing",
    ] {
        let Err(err) = ClientRepo::from_bytes(junk) else {
            panic!("{junk:?} opened");
        };
        assert!(
            matches!(err, RepoError::Refused(_) | RepoError::Unavailable(_)),
            "{err}"
        );
    }
}

#[tokio::test]
async fn a_backup_in_wal_mode_opens_too() {
    // The Pi's own file is WAL (P3). Checkpointed, its bytes are a whole
    // database that says WAL in its header, which memory cannot open as is.
    let dir = Dir::new("wal");
    let pi = SqliteRepo::connect(&dir.url("pi.db")).unwrap();
    tt_repo_sqlite::migrate::apply(pi.pool()).await.unwrap();
    pi.upsert_team(&team(254, "Poofs"), at(0)).await.unwrap();
    // Closing the last connection checkpoints and removes the -wal file.
    pi.pool().close().await;
    let bytes = std::fs::read(dir.0.join("pi.db")).unwrap();
    assert_eq!((bytes[18], bytes[19]), (2, 2), "a WAL file");

    let device = ClientRepo::from_bytes(&bytes).expect("opens");
    assert_eq!(device.team(254).await.unwrap().unwrap().name, "Poofs");
}
