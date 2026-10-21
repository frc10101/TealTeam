//! The fake event and the people at it (DEV1).
//!
//! Everything goes through the `Repo` and the `SqliteRepo` methods the app
//! itself calls, never raw SQL, so the rows are the ones the app would have
//! written: the change log's triggers fire, a review leaves its reviewer, a
//! sign-out bumps the epoch. Nothing here is random. Ids come from insertion
//! order, record ids from a counter, and the times are offsets from now, so a
//! fresh seed always comes out the same shape.

use anyhow::Context;
use chrono::{TimeDelta, Utc};
use tt_core::assignments::AssigneeKey;
use tt_core::matches::CompLevel;
use tt_core::picklist::{Entry, PickDoc, Tag};
use tt_core::record_id::uuid_v7;
use tt_core::records::{Event, MatchRecord, Team, TeamEventStats};
use tt_core::review::Decision;
use tt_core::season::{Payload, Value};
use tt_core::user::User;
use tt_repo::{NewAssignment, NewObservation, NewUser, Repo};
use tt_repo_sqlite::refused::NewRefusal;

use super::{OTHER_TEAM, OUR_TEAM, PASSWORD, USERS};
use crate::auth::hash_password;
use crate::startup::AppState;

pub const EVENT: &str = "2026devevt";
pub const MATCHES: u32 = 21;
/// Qualification matches 1 through this are played; the rest are not.
pub const PLAYED: u32 = 10;
/// The registered tablet's id, as its cookie carries it.
pub const TABLET: &str = "dev-tablet-0001-stands-left";
/// A match nobody scheduled, for the refused entries.
pub const UNSCHEDULED: &str = "2026devevt_qm99";

/// Fourteen teams: ours, the other team, and twelve made-up ones.
pub fn roster() -> Vec<(i32, String)> {
    let mut teams = vec![
        (OUR_TEAM, "Teal Team".to_string()),
        (OTHER_TEAM, "Second Wind".to_string()),
    ];
    let names = [
        "Gear Heads",
        "Byte Force",
        "Circuit Breakers",
        "Torque Titans",
        "Volt Vikings",
        "Rivet Rangers",
        "Bolt Bandits",
        "Servo Squad",
        "Quantum Quokkas",
        "Alloy Aces",
        "Pixel Pirates",
        "Flux Foxes",
    ];
    teams.extend(
        names
            .iter()
            .enumerate()
            .map(|(i, n)| (3101 + i as i32, n.to_string())),
    );
    teams
}

pub fn match_key(n: u32) -> String {
    format!("{EVENT}_qm{n}")
}

/// The six robots in match `n`, red then blue: the roster in turn.
pub fn robots(n: u32) -> [i32; 6] {
    let teams = roster();
    std::array::from_fn(|slot| teams[(n as usize * 6 + slot) % teams.len()].0)
}

/// What a note says, so a check can look for it: whose team wrote it, in
/// which match, about which robot.
pub fn marker(team: i32, n: u32, robot: i32) -> String {
    format!("N{team}-q{n}-t{robot}")
}

/// The reason given when a lead declines, with something to look for.
pub fn decline_reason(scout: &str, n: u32) -> String {
    format!("DECL-{scout}-q{n}: that is not the robot in the match")
}

/// A record id that is the same every seeding.
fn record_id(counter: u64) -> String {
    let mut random = [0u8; 10];
    random[2..].copy_from_slice(&counter.to_be_bytes());
    uuid_v7(1_790_000_000_000 + counter, random)
}

fn payload(n: u32, note: &str) -> Payload {
    let n = i64::from(n);
    let position = ["left", "center", "right"][(n % 3) as usize];
    let endgame = ["none", "partial", "full"][((n / 2) % 3) as usize];
    [
        ("starting_position", Value::Text(position.into())),
        ("no_show", Value::Flag(false)),
        ("auto_scored", Value::Count(n % 4)),
        ("teleop_scored", Value::Count(4 + n % 9)),
        ("endgame", Value::Text(endgame.into())),
        ("broke_down", Value::Flag(n % 7 == 0)),
        ("penalties", Value::Count(n % 2)),
        ("notes", Value::Text(note.into())),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect()
}

pub async fn seed(state: &AppState) -> anyhow::Result<()> {
    let repo = &*state.repo;
    let now = Utc::now();
    let today = now.date_naive();

    // The event and its teams.
    let event = Event {
        key: EVENT.into(),
        name: "Dev Regional".into(),
        location: Some("Nowhere, Testland".into()),
        timezone: None,
        start_date: Some(today - TimeDelta::days(1)),
        end_date: Some(today + TimeDelta::days(1)),
        event_code: None,
        event_type: None,
        district_key: None,
        week: None,
    };
    repo.upsert_event(&event, now).await?;
    let teams = roster();
    for (number, name) in &teams {
        let team = Team {
            number: *number,
            name: name.clone(),
            nickname: Some(name.clone()),
            school: None,
            city: Some("Testville".into()),
            state: None,
            country: None,
            rookie_year: None,
            website: None,
        };
        repo.upsert_team(&team, now).await?;
        repo.link_event_team(EVENT, *number, now).await?;
    }

    // The schedule: played matches have scores.
    for n in 1..=MATCHES {
        let [r1, r2, r3, b1, b2, b3] = robots(n);
        let played = n <= PLAYED;
        let (red, blue) = (40 + (n * 7) % 35, 38 + (n * 11) % 33);
        let when = now + TimeDelta::minutes(8 * (i64::from(n) - i64::from(PLAYED)));
        let record = MatchRecord {
            key: match_key(n),
            event_key: EVENT.into(),
            comp_level: CompLevel::Qualification,
            set_number: 1,
            match_number: n as i32,
            red: [Some(r1), Some(r2), Some(r3)],
            blue: [Some(b1), Some(b2), Some(b3)],
            red_score: played.then_some(i64::from(red)),
            blue_score: played.then_some(i64::from(blue)),
            winner: played.then(|| if red >= blue { "red" } else { "blue" }.to_string()),
            played,
            scheduled_at: Some(when),
            actual_at: played.then_some(when),
        };
        repo.upsert_match(&record, now).await?;
    }
    for (i, (number, _)) in teams.iter().enumerate() {
        let i = i as i32;
        let stats = TeamEventStats {
            team_number: *number,
            event_key: EVENT.into(),
            opr: Some(20.0 + f64::from((i * 37) % 30)),
            rank: Some(i + 1),
            matches_played: Some(PLAYED as i32 * 6 / teams.len() as i32),
            ..Default::default()
        };
        repo.upsert_team_stats(&stats, now).await?;
    }

    // The people. One hash for everyone: the password is the same, and
    // Argon2 is slow on purpose.
    let hash = hash_password(PASSWORD).map_err(|e| anyhow::anyhow!("hashing: {e}"))?;
    let mut users: Vec<User> = Vec::new();
    for u in USERS {
        let user = repo
            .create_user(
                NewUser {
                    email: u.email.into(),
                    name: u.name.into(),
                    password_hash: hash.clone(),
                    team_number: u.team,
                    roles: u.roles,
                },
                now,
            )
            .await
            .with_context(|| format!("creating {}", u.email))?;
        users.push(user);
    }
    let by = |key: &str| -> &User {
        let at = USERS.iter().position(|u| u.key == key).expect("a dev user");
        &users[at]
    };

    // A tablet registered to a scout, and a scout signed out everywhere.
    let device = repo.touch_device(TABLET, Some(by("scout1")), now).await?;
    repo.rename_device(device.id, "Stands Left", now).await?;
    repo.sign_out_everywhere(by("signedout").id, by("admin").id, now)
        .await?;

    // Observations in the played matches, and what the leads made of them.
    let mut counter = 0u64;
    let mut observe = async |scout: &User, n: u32, robot: i32| -> anyhow::Result<i64> {
        counter += 1;
        let record = repo
            .match_by_key(&match_key(n))
            .await?
            .context("a seeded match")?;
        let team = scout.team_number.context("a scout with a team")?;
        let note = format!("{} drove steadily", marker(team, n, robot));
        let observation = NewObservation {
            client_record_id: record_id(counter),
            match_key: record.key.clone(),
            event_key: EVENT.into(),
            team_number: robot,
            alliance: record.alliance_of(robot).context("a robot in the match")?,
            payload: payload(n, &note),
            schema_version: state.season.version,
            scouter_id: Some(scout.id),
            device_id: (scout.id == by("scout1").id).then_some(device.id),
            submitting_team: scout.team_number,
            observed_at: now - TimeDelta::minutes(120 - 8 * i64::from(n)),
        };
        match repo.record_observation(&observation, now).await? {
            tt_repo::Recorded::Created(id) | tt_repo::Recorded::Duplicate(id) => Ok(id),
        }
    };

    let ours = ["scout1", "scout2", "scout3", "scout4"];
    for n in 1..=PLAYED {
        let r = robots(n);
        for (k, key) in ours.iter().enumerate() {
            let id = observe(by(key), n, r[k]).await?;
            let lead = by("lead").id;
            if n <= 6 {
                repo.review_observation(id, &Decision::Approve, lead, now)
                    .await?;
                if n == 6 && k == 3 {
                    // A review undone (G3): back to waiting, with who and when.
                    repo.reopen_observation(id, lead, now).await?;
                }
            } else if n == 8 && k == 2 {
                let reason = Decision::decline(&decline_reason("s3", n))?;
                repo.review_observation(id, &reason, lead, now).await?;
            }
        }
        // The other team scouts the other two robots, and, in the first
        // three matches, one of ours as well: two teams' notes on one robot.
        let lead2 = by("lead2").id;
        let mut theirs = vec![("scout5", r[4]), ("scout6", r[5])];
        if n <= 3 {
            theirs.push(("scout5", r[0]));
        }
        for (key, robot) in theirs {
            let id = observe(by(key), n, robot).await?;
            if n <= 4 {
                repo.review_observation(id, &Decision::Approve, lead2, now)
                    .await?;
            } else if n == 7 && key == "scout6" {
                let reason = Decision::decline(&decline_reason("s6", n))?;
                repo.review_observation(id, &reason, lead2, now).await?;
            }
        }
    }

    // Assignments for the matches still to play: our scouts in turn, the other
    // team's on the last two robots of the later matches, and the tablet once.
    let lead = by("lead").id;
    let mut assignments = Vec::new();
    for n in PLAYED + 1..=MATCHES {
        for (slot, robot) in robots(n).into_iter().enumerate() {
            let assignee = match (n, slot) {
                (11, 5) => AssigneeKey::Device(device.id),
                (16.., 4) => AssigneeKey::Scout(by("scout5").id),
                (16.., 5) => AssigneeKey::Scout(by("scout6").id),
                _ => AssigneeKey::Scout(by(ours[(n as usize + slot) % ours.len()]).id),
            };
            assignments.push(NewAssignment {
                match_key: match_key(n),
                event_key: EVENT.into(),
                team_number: robot,
                assignee,
            });
        }
    }
    repo.set_assignments(&assignments, lead, now).await?;

    // Entries the Pi refused when they were pushed, waiting for a lead.
    for (key, n) in [("scout2", 1u64), ("scout5", 2)] {
        let scout = by(key);
        let team = scout.team_number.unwrap_or_default();
        let refusal = NewRefusal {
            entry: tt_core::outbox::QueuedObservation {
                client_record_id: record_id(1000 + n),
                match_key: UNSCHEDULED.into(),
                team_number: 3101,
                payload: payload(3, &format!("{} refused", marker(team, 99, 3101))),
                schema_version: state.season.version,
                observed_at: now - TimeDelta::minutes(30),
            },
            scouter_id: Some(scout.id),
            device_id: None,
            submitting_team: scout.team_number,
            reason: format!("Not saved: {UNSCHEDULED} is not on the schedule."),
        };
        repo.keep_refusal(&refusal, now).await?;
    }

    // Each team's pick list.
    let list = |team: i32, picks: &[(i32, Option<Tag>, bool)], base: u64| {
        let entries: Vec<Entry> = picks
            .iter()
            .enumerate()
            .map(|(i, (number, tag, crossed))| Entry {
                record_id: record_id(base + i as u64),
                team_number: *number,
                tag: *tag,
                crossed: *crossed,
            })
            .collect();
        (team, PickDoc::from_entries(&entries).state())
    };
    for (team, doc) in [
        list(
            OUR_TEAM,
            &[
                (3103, Some(Tag::Green), false),
                (3107, Some(Tag::Yellow), false),
                (3105, None, false),
                (3110, Some(Tag::Red), true),
            ],
            2000,
        ),
        list(
            OTHER_TEAM,
            &[
                (3102, Some(Tag::Blue), false),
                (3104, None, false),
                (3109, None, false),
            ],
            3000,
        ),
    ] {
        repo.merge_pick_list(team, EVENT, &doc, now).await?;
    }
    Ok(())
}
