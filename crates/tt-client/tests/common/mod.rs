//! What the round-trip tests share: one database opened by the server's
//! repo and by the browser's, and the rows they write to both (C4, C11).
#![allow(dead_code)]

use std::path::PathBuf;

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use tt_core::matches::CompLevel;
use tt_core::records::{Event, MatchRecord, Team};
use tt_core::season::{Payload, Value};
use tt_repo::{LocalRepo, NewObservation, NewUpstream};

pub fn at(minute: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 14, 10, minute, 0).unwrap()
}

/// Long after anything here was written: every change has settled.
pub fn later() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2100, 1, 1, 0, 0, 0).unwrap()
}

/// A directory of the test's own, removed on drop.
pub struct Dir(pub PathBuf);

impl Dir {
    pub fn new(test: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("tt-client-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Dir(dir)
    }

    pub fn url(&self, file: &str) -> String {
        format!("sqlite://{}", self.0.join(file).display())
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub fn event(key: &str, start: Option<(i32, u32, u32)>) -> Event {
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

pub fn team(number: i32, name: &str) -> Team {
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

pub fn game(event_key: &str, number: i32, red: [i32; 3], blue: [i32; 3]) -> MatchRecord {
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

pub fn observation(id: &str, match_key: &str, team: i32, scouter: Option<i64>) -> NewObservation {
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

pub fn upstream(path: &str, body: &str) -> NewUpstream {
    NewUpstream {
        api: "tba".into(),
        path: path.into(),
        etag: Some("W/\"1\"".into()),
        body: body.into(),
        fetched_at: at(1),
        via: "pi".into(),
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

/// The change log without its times, which are SQLite's clock: alike on the
/// two sides, not equal.
pub async fn settled(repo: &impl LocalRepo) -> Vec<(i64, String, String, String, Option<String>)> {
    repo.changes_since(0, 100, later())
        .await
        .unwrap()
        .into_iter()
        .map(|c| (c.seq, c.entity, c.entity_pk, c.op, c.payload))
        .collect()
}
