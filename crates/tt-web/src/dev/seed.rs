//! The fake event `tt-web dev` serves (F13): who is in it, and what they did.
//!
//! Everything goes in through the calls the app itself makes: the schedule
//! through `schedule::import` (the F3 import the lead's page runs), accounts
//! and sessions through the `Repo`, observations and reviews through the
//! `Repo`, a refusal through `push::record`. Nothing is raw SQL. The data is
//! fixed (a small xorshift, no clock in any answer), so two seeds hold the
//! same event; only the timestamps differ.
//!
//! The event: `DEVEVT`, 14 teams, 21 qualification matches of 4 robots. The
//! first 12 are played, with scores, rankings and OPR worked out as an FTC
//! pull would leave them (F7, F10). The other 9 are unplayed and assigned.

use anyhow::Context;
use chrono::{TimeDelta, Utc};
use tt_core::assignments::AssigneeKey;
use tt_core::opr::{self, Played};
use tt_core::outbox::QueuedObservation;
use tt_core::picklist::{Edit, PickDoc, Tag};
use tt_core::records::{MatchRecord, TeamEventStats};
use tt_core::review::Decision;
use tt_core::season::{Payload, Value};
use tt_core::user::{Roles, User};
use tt_repo::{NewAssignment, NewObservation, NewUser, Recorded, Repo};

use crate::schedule::{self, Posted};
use crate::startup::AppState;

/// Every account's password.
pub const PASSWORD: &str = "dev";
/// Our team, and the second team that scouts the same event.
pub const OURS: i32 = 10101;
pub const OTHER: i32 = 10202;
pub const EVENT_CODE: &str = "DEVEVT";
/// The tablet registered to scout one (its device cookie).
pub const TABLET: &str = "dev00000-0000-4000-8000-0000000000a1";
/// A tablet nobody has registered.
pub const SPARE_TABLET: &str = "dev00000-0000-4000-8000-0000000000b2";
/// Text only one team may read, in its scouts' notes.
pub const NOTE_OURS: &str = "DEVNOTE-10101";
pub const NOTE_OTHER: &str = "DEVNOTE-10202";
pub const NOTE_NONE: &str = "DEVNOTE-NOTEAM";
/// In a decline's reason.
pub const DECLINE_OURS: &str = "DEVDECLINE-10101";
pub const DECLINE_OTHER: &str = "DEVDECLINE-10202";

const PLAYED: i32 = 12;
const MATCHES: i32 = 21;

/// One account.
pub struct DevUser {
    pub email: &'static str,
    pub name: &'static str,
    /// What the table says about them.
    pub role: &'static str,
    pub team: Option<i32>,
    pub roles: Roles,
}

const fn roles(admin: bool, lead: bool, coach: bool) -> Roles {
    Roles {
        is_admin: admin,
        is_lead_scout: lead,
        is_coach: coach,
    }
}

/// Every account, in the order they are made (so their ids are 1..).
pub const USERS: &[DevUser] = &[
    DevUser {
        email: "admin@dev.test",
        name: "Ada Admin",
        role: "admin",
        team: Some(OURS),
        roles: roles(true, false, false),
    },
    DevUser {
        email: "lead@dev.test",
        name: "Lena Lead",
        role: "lead scout",
        team: Some(OURS),
        roles: roles(false, true, false),
    },
    DevUser {
        email: "coach@dev.test",
        name: "Cole Coach",
        role: "drive coach",
        team: Some(OURS),
        roles: roles(false, false, true),
    },
    DevUser {
        email: "scout1@dev.test",
        name: "Sam Scout",
        role: "scout (has the tablet)",
        team: Some(OURS),
        roles: Roles::SCOUT,
    },
    DevUser {
        email: "scout2@dev.test",
        name: "Priya Scout",
        role: "scout",
        team: Some(OURS),
        roles: Roles::SCOUT,
    },
    DevUser {
        email: "scout3@dev.test",
        name: "Quinn Scout",
        role: "scout",
        team: Some(OURS),
        roles: Roles::SCOUT,
    },
    DevUser {
        email: "scout4@dev.test",
        name: "Rae Scout",
        role: "scout",
        team: Some(OURS),
        roles: Roles::SCOUT,
    },
    DevUser {
        email: "signedout@dev.test",
        name: "Sid Signedout",
        role: "scout, signed out everywhere",
        team: Some(OURS),
        roles: Roles::SCOUT,
    },
    DevUser {
        email: "lead2@dev.test",
        name: "Lou Otherlead",
        role: "lead scout",
        team: Some(OTHER),
        roles: roles(false, true, false),
    },
    DevUser {
        email: "scoutb1@dev.test",
        name: "Bea Otherscout",
        role: "scout",
        team: Some(OTHER),
        roles: Roles::SCOUT,
    },
    DevUser {
        email: "scoutb2@dev.test",
        name: "Bo Otherscout",
        role: "scout",
        team: Some(OTHER),
        roles: Roles::SCOUT,
    },
    DevUser {
        email: "noteam@dev.test",
        name: "Nat Noteam",
        role: "scout, no team",
        team: None,
        roles: Roles::SCOUT,
    },
    DevUser {
        email: "noteamlead@dev.test",
        name: "Nell Noteamlead",
        role: "lead scout, no team",
        team: None,
        roles: roles(false, true, false),
    },
];

/// An account by email.
pub fn user(email: &str) -> &'static DevUser {
    USERS
        .iter()
        .find(|u| u.email == email)
        .unwrap_or_else(|| panic!("no dev user {email}"))
}

/// The 14 teams: ours, the other scouting team, and 12 more.
fn teams() -> Vec<(i32, &'static str)> {
    vec![
        (OURS, "Teal Team"),
        (OTHER, "Orange Circuit"),
        (10001, "Quantum Hornets"),
        (10002, "Gear Gnomes"),
        (10003, "Velocity Voles"),
        (10004, "Pollen Patrol"),
        (10005, "Nectar Ninjas"),
        (10006, "Hive Minds"),
        (10007, "Torque Tigers"),
        (10008, "Bolt Bandits"),
        (10009, "Servo Swarm"),
        (10010, "Flux Foxes"),
        (10011, "Drift Drones"),
        (10012, "Cobalt Cranes"),
    ]
}

/// The quals: each match takes the four teams that have played least so
/// far (a fixed shuffle breaks ties), so every team plays about six times
/// and partners vary enough for OPR to be worked out from the played half.
fn schedule_csv() -> String {
    let roster = teams();
    let mut rng = Rng(0x2545_f491_4f6c_dd1d);
    let mut played = vec![0u32; roster.len()];
    let mut csv = tt_core::schedule::header();
    for n in 1..=MATCHES {
        let mut order: Vec<usize> = (0..roster.len()).collect();
        for i in (1..order.len()).rev() {
            order.swap(i, rng.below(i as u64 + 1) as usize);
        }
        order.sort_by_key(|i| played[*i]);
        let four = &order[..4];
        for i in four {
            played[*i] += 1;
        }
        // Q5's blue2 is a surrogate: it plays, and the match is not its own.
        let star = if n == 5 { "*" } else { "" };
        csv.push_str(&format!(
            "\n{n},{},{},{},{}{star}",
            roster[four[0]].0, roster[four[1]].0, roster[four[2]].0, roster[four[3]].0
        ));
    }
    csv
}

/// xorshift64: the same answers on every seed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn pick<'a>(&mut self, of: &[&'a str]) -> &'a str {
        of[self.below(of.len() as u64) as usize]
    }
}

fn answers(rng: &mut Rng, notes: Option<String>) -> Payload {
    let text = |s: &str| Value::Text(s.into());
    let mut p = Payload::new();
    p.insert("robot_status".into(), text("played"));
    p.insert("auto_leave".into(), Value::Flag(rng.below(2) == 1));
    p.insert("auto_launched".into(), Value::Count(rng.below(6) as i64));
    p.insert("launched".into(), Value::Count(rng.below(30) as i64));
    p.insert(
        "launch_accuracy".into(),
        text(rng.pick(&["most", "about_half", "few"])),
    );
    p.insert(
        "garden_delivered".into(),
        Value::Count(rng.below(12) as i64),
    );
    p.insert(
        "flower_role".into(),
        text(rng.pick(&["none", "attempted", "placed", "placed_bottom_nectar"])),
    );
    p.insert("teleop_park".into(), Value::Flag(rng.below(2) == 1));
    p.insert(
        "defense_played".into(),
        text(rng.pick(&["none", "some", "primary_role"])),
    );
    p.insert(
        "driver_skill".into(),
        text(rng.pick(&["struggling", "average", "strong", "excellent"])),
    );
    p.insert("fouls_observed".into(), Value::Count(rng.below(3) as i64));
    if let Some(notes) = notes {
        p.insert("notes".into(), Value::Text(notes));
    }
    p
}

/// A record id that is a UUID and the same on every seed.
fn record_id(n: u64) -> String {
    format!("0191f7ac-1234-7000-8000-{n:012x}")
}

/// What a review should leave an observation as.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Verdict {
    Approved,
    Waiting,
    Declined,
    /// Approved, then undone (G3): waiting again, with a log to show it.
    Undone,
}

struct Watch {
    scout: &'static str,
    match_number: i32,
    slot: usize,
    verdict: Verdict,
    notes: Option<String>,
}

fn watches() -> Vec<Watch> {
    let ours = [
        "scout1@dev.test",
        "scout2@dev.test",
        "scout3@dev.test",
        "scout4@dev.test",
    ];
    let mut out = Vec::new();
    // Our four scouts, one robot each, in Q1-Q8. Q1-Q6 are reviewed; Q7-Q8
    // wait. Two exceptions: a decline (Q3, slot 2) and an undone approval
    // (Q5, slot 3).
    for m in 1..=8 {
        for (slot, scout) in ours.iter().enumerate() {
            let verdict = match (m, slot) {
                (3, 1) => Verdict::Declined,
                (5, 2) => Verdict::Undone,
                (7..=8, _) => Verdict::Waiting,
                _ => Verdict::Approved,
            };
            out.push(Watch {
                scout,
                match_number: m,
                slot,
                verdict,
                notes: Some(format!("{NOTE_OURS}: Q{m} robot {}", slot + 1)),
            });
        }
    }
    // The other team's two scouts, red 1 and red 2 of Q1-Q6.
    for m in 1..=6 {
        for (slot, scout) in ["scoutb1@dev.test", "scoutb2@dev.test"].iter().enumerate() {
            let verdict = match (m, slot) {
                (2, 1) => Verdict::Declined,
                (5..=6, _) => Verdict::Waiting,
                _ => Verdict::Approved,
            };
            out.push(Watch {
                scout,
                match_number: m,
                slot,
                verdict,
                notes: Some(format!("{NOTE_OTHER}: Q{m} robot {}", slot + 1)),
            });
        }
    }
    // Sid watched one before being signed out; Nat, with no team, another.
    out.push(Watch {
        scout: "signedout@dev.test",
        match_number: 1,
        slot: 3,
        verdict: Verdict::Approved,
        notes: Some(format!("{NOTE_OURS}: Sid, Q1")),
    });
    out.push(Watch {
        scout: "noteam@dev.test",
        match_number: 2,
        slot: 0,
        verdict: Verdict::Approved,
        notes: Some(format!("{NOTE_NONE}: Q2")),
    });
    out
}

fn match_key(event: &str, n: i32) -> String {
    format!("{event}_qm{n}")
}

/// Make the event on `state`'s (migrated, empty) database.
pub async fn seed(state: &AppState) -> anyhow::Result<()> {
    let repo = &*state.repo;
    let now = Utc::now();
    let hash = crate::auth::hash_password(PASSWORD)
        .map_err(|e| anyhow::anyhow!("hashing the dev password: {e}"))?;

    // ── Accounts ────────────────────────────────────────────────────────────
    let mut users: Vec<User> = Vec::new();
    for u in USERS {
        users.push(
            repo.create_user(
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
            .with_context(|| format!("creating {}", u.email))?,
        );
    }
    let by = |email: &str| -> &User {
        let at = USERS
            .iter()
            .position(|u| u.email == email)
            .expect("dev user");
        &users[at]
    };
    let lead = by("lead@dev.test");
    let lead2 = by("lead2@dev.test");

    // ── The event: the F3 import, as the lead's page posts it ───────────────
    let roster: String = std::iter::once("team,name".to_string())
        .chain(teams().into_iter().map(|(n, name)| format!("{n},{name}")))
        .collect::<Vec<_>>()
        .join("\n");
    let imported = schedule::import(
        state,
        lead,
        &Posted {
            code: EVENT_CODE.into(),
            name: "Dev Qualifier".into(),
            timezone: "America/Chicago".into(),
            date: String::new(),
            schedule: schedule_csv(),
            roster,
        },
    )
    .await
    .map_err(|errors| anyhow::anyhow!("the dev schedule was refused: {}", errors.join("; ")))?;
    let event = imported.event_key;

    // ── Results, rankings and OPR: what an FTC pull leaves (F7, F10) ────────
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let mut played = Vec::new();
    let mut table: std::collections::BTreeMap<i32, (i32, i32, i32, i64, i32)> = Default::default();
    for n in 1..=PLAYED {
        let mut m: MatchRecord = repo
            .match_by_key(&match_key(&event, n))
            .await?
            .with_context(|| format!("Q{n} was not stored"))?;
        let (red, blue) = (40 + rng.below(110) as i64, 40 + rng.below(110) as i64);
        m.red_score = Some(red);
        m.blue_score = Some(blue);
        m.winner = match red.cmp(&blue) {
            std::cmp::Ordering::Greater => Some("red".into()),
            std::cmp::Ordering::Less => Some("blue".into()),
            std::cmp::Ordering::Equal => None,
        };
        m.played = true;
        m.actual_at = Some(now - TimeDelta::minutes(((PLAYED - n) * 9 + 30) as i64));
        repo.upsert_match(&m, now).await?;
        played.push(Played {
            red: m.red.iter().flatten().copied().collect(),
            blue: m.blue.iter().flatten().copied().collect(),
            red_points: red as f64,
            blue_points: blue as f64,
        });
        for (side, own, other) in [(&m.red, red, blue), (&m.blue, blue, red)] {
            for team in side.iter().flatten() {
                let row = table.entry(*team).or_default();
                row.4 += 1;
                row.3 += own;
                match own.cmp(&other) {
                    std::cmp::Ordering::Greater => row.0 += 1,
                    std::cmp::Ordering::Less => row.1 += 1,
                    std::cmp::Ordering::Equal => row.2 += 1,
                }
            }
        }
    }
    let ratings = opr::event(&played);
    anyhow::ensure!(
        ratings.is_some(),
        "the dev results do not determine OPR: play more matches"
    );
    let mut order: Vec<_> = table.iter().collect();
    order.sort_by_key(|(team, (w, _, t, points, _))| (-(w * 2 + t), -points, **team));
    for (rank, (team, (w, l, t, points, n))) in order.into_iter().enumerate() {
        let rating = ratings.as_ref().and_then(|r| r.get(team));
        repo.upsert_team_stats(
            &TeamEventStats {
                team_number: *team,
                event_key: event.clone(),
                rank: Some(rank as i32 + 1),
                matches_played: Some(*n),
                wins: Some(*w),
                losses: Some(*l),
                ties: Some(*t),
                qual_average: Some(*points as f64 / f64::from(*n)),
                opr: rating.map(|r| r.opr),
                dpr: rating.map(|r| r.dpr),
                ccwm: rating.map(|r| r.ccwm),
                synced_at: Some(now),
                ..TeamEventStats::default()
            },
            now,
        )
        .await?;
    }

    // ── Observations, and the lead's verdicts on them ───────────────────────
    let matches = repo.event_matches(&event).await?;
    let mut seen = 0u64;
    for w in watches() {
        let scout = by(w.scout);
        let record = matches
            .iter()
            .find(|m| m.key == match_key(&event, w.match_number))
            .context("a watched match")?;
        let team = record.teams().nth(w.slot).context("a watched slot")?;
        seen += 1;
        let payload = answers(&mut rng, w.notes);
        state
            .season
            .validate_payload(&payload)
            .with_context(|| format!("dev answers for {team} do not fit the form"))?;
        let at = record.actual_at.unwrap_or(now) + TimeDelta::minutes(5);
        let recorded = repo
            .record_observation(
                &NewObservation {
                    client_record_id: record_id(seen),
                    match_key: record.key.clone(),
                    event_key: event.clone(),
                    team_number: team,
                    alliance: record.alliance_of(team).context("alliance")?,
                    payload,
                    schema_version: state.season.version,
                    scouter_id: Some(scout.id),
                    device_id: None,
                    submitting_team: scout.team_number,
                    observed_at: at.min(now),
                },
                now,
            )
            .await?;
        let (Recorded::Created(id) | Recorded::Duplicate(id)) = recorded;
        let (reviewer, reason) = if scout.team_number == Some(OTHER) {
            (lead2, DECLINE_OTHER)
        } else {
            (lead, DECLINE_OURS)
        };
        match w.verdict {
            Verdict::Waiting => {}
            Verdict::Approved | Verdict::Undone => {
                repo.review_observation(id, &Decision::Approve, reviewer.id, now)
                    .await?;
                if w.verdict == Verdict::Undone {
                    repo.reopen_observation(id, reviewer.id, now).await?;
                }
            }
            Verdict::Declined => {
                let decision = Decision::decline(&format!(
                    "{reason}: the launches were counted from the wrong side. Watch the HIVE next time."
                ))?;
                repo.review_observation(id, &decision, reviewer.id, now)
                    .await?;
            }
        }
    }

    // ── A tablet registered to a scout ──────────────────────────────────────
    let tablet = repo
        .touch_device(TABLET, Some(by("scout1@dev.test")), now)
        .await?;
    repo.rename_device(tablet.id, "Tablet A", now).await?;
    repo.touch_device(SPARE_TABLET, None, now).await?;

    // ── Refused entries waiting for the lead, one from each team ────────────
    for (scout, tablet_id, key, team) in [
        // A robot that is not in the match.
        (
            by("scout2@dev.test"),
            Some(tablet.id),
            match_key(&event, 7),
            99_999,
        ),
        // A match that is not on the schedule.
        (by("scoutb1@dev.test"), None, match_key(&event, 99), OTHER),
    ] {
        seen += 1;
        let entry = QueuedObservation {
            client_record_id: record_id(seen),
            match_key: key,
            team_number: team,
            payload: answers(
                &mut rng,
                Some(format!(
                    "DEVNOTE-{}: refused",
                    scout.team_number.unwrap_or(0)
                )),
            ),
            schema_version: state.season.version,
            observed_at: now - TimeDelta::minutes(3),
        };
        let device = match tablet_id {
            Some(_) => repo.device_by_uuid(TABLET).await?,
            None => None,
        };
        let outcome = crate::push::record(state, scout, device.as_ref(), entry, now).await?;
        anyhow::ensure!(
            matches!(outcome, tt_core::outbox::Outcome::Refused { .. }),
            "the dev refusal for {} was accepted",
            scout.email
        );
    }

    // ── Assignments for the unplayed matches ────────────────────────────────
    let ids = |emails: &[&str]| -> Vec<i64> { emails.iter().map(|e| by(e).id).collect() };
    let ours = ids(&[
        "scout1@dev.test",
        "scout2@dev.test",
        "scout3@dev.test",
        "scout4@dev.test",
    ]);
    let mixed = ids(&[
        "scoutb1@dev.test",
        "scoutb2@dev.test",
        "scout1@dev.test",
        "scout2@dev.test",
    ]);
    let mut planned = Vec::new();
    for n in (PLAYED + 1)..=MATCHES {
        let record = matches
            .iter()
            .find(|m| m.key == match_key(&event, n))
            .context("an unplayed match")?;
        for (slot, team) in record.teams().enumerate() {
            let assignee = if n == MATCHES && slot == 3 {
                AssigneeKey::Device(tablet.id)
            } else if n > PLAYED + 5 {
                AssigneeKey::Scout(mixed[slot])
            } else {
                AssigneeKey::Scout(ours[slot])
            };
            planned.push(NewAssignment {
                match_key: record.key.clone(),
                event_key: event.clone(),
                team_number: team,
                assignee,
            });
        }
    }
    repo.set_assignments(&planned, lead.id, now).await?;

    // ── Pick lists, one per team ────────────────────────────────────────────
    let roster: Vec<i32> = teams().into_iter().map(|(n, _)| n).collect();
    for (team, picks, crossed, tagged) in [
        (OURS, [10003, 10007, 10011, 10002], 10011, 10007),
        (OTHER, [10005, 10009, 10001, 10012], 10012, 10005),
    ] {
        let mut doc = PickDoc::new();
        for (i, pick) in picks.into_iter().enumerate() {
            doc.apply(
                &Edit::Add {
                    team: pick,
                    record_id: record_id(0x5000 + u64::from(team as u32) + i as u64),
                },
                &roster,
            )
            .map_err(anyhow::Error::msg)?;
        }
        doc.apply(
            &Edit::Cross {
                team: crossed,
                crossed: true,
            },
            &roster,
        )
        .map_err(anyhow::Error::msg)?;
        doc.apply(
            &Edit::Tag {
                team: tagged,
                tag: Some(Tag::Green),
            },
            &roster,
        )
        .map_err(anyhow::Error::msg)?;
        repo.merge_pick_list(team, &event, &doc.state(), now)
            .await?;
    }

    // ── Sid is signed out everywhere (T1): sessions gone, epoch bumped ──────
    repo.sign_out_everywhere(by("signedout@dev.test").id, lead.id, now)
        .await?;

    Ok(())
}

/// The event's key, as the import makes it.
pub fn event_key(season: i32) -> String {
    tt_core::upstream::normalize_event_key(EVENT_CODE, season).expect("a code")
}

/// Match `n`'s key in the dev event.
pub fn qual_key(season: i32, n: i32) -> String {
    match_key(&event_key(season), n)
}
