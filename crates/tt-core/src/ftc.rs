//! FTC Events API payloads for the one optional pull (FTC_BETA_PLAN.md, F7).
//!
//! One event, `USALATQ`, season 2026; `docs/FTC_BETA_API.md` has the mapping.
//! Events and teams have FRC's field names, so they reuse
//! [`FirstEvent`](crate::upstream::FirstEvent) and
//! [`FirstTeam`](crate::upstream::FirstTeam) inside FTC's lowercase
//! envelopes. The hybrid schedule becomes the TBA `/event/{key}/matches` body
//! the CSV import (F3) writes, so both paths store matches the same way: two
//! robots an alliance by station, `-1` for an unplayed score, and keys
//! `{event}_qm{n}`. Rankings fill the `TeamEventStats` columns they match.

use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use serde::Deserialize;
use serde_json::json;

use crate::matches::{ALLIANCE_SIZE, slot_names};
use crate::schedule::Surrogate;
use crate::upstream::{FirstEvent, FirstTeam, or_default, team_key};

/// `/{season}/events?eventCode=`
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Events {
    #[serde(default, deserialize_with = "or_default")]
    pub events: Vec<FirstEvent>,
}

/// `/{season}/teams?eventCode=`, one page.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Teams {
    #[serde(default, deserialize_with = "or_default")]
    pub teams: Vec<FirstTeam>,
    #[serde(default)]
    pub page_total: Option<u32>,
}

/// `/{season}/schedule/{code}/qual/hybrid`
#[derive(Debug, Clone, Default, Deserialize)]
pub struct HybridSchedule {
    #[serde(default, deserialize_with = "or_default")]
    pub schedule: Vec<HybridMatch>,
}

/// One match of the hybrid schedule: the schedule with results merged in,
/// nulls until it is played.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HybridMatch {
    #[serde(default, deserialize_with = "or_default")]
    pub match_number: i32,
    #[serde(default)]
    pub start_time: Option<String>,
    #[serde(default)]
    pub actual_start_time: Option<String>,
    #[serde(default)]
    pub score_red_final: Option<i64>,
    #[serde(default)]
    pub score_blue_final: Option<i64>,
    #[serde(default, deserialize_with = "or_default")]
    pub teams: Vec<HybridTeam>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HybridTeam {
    #[serde(default)]
    pub team_number: Option<i32>,
    /// `Red1`, `Red2`, `Blue1`, `Blue2`.
    #[serde(default, deserialize_with = "or_default")]
    pub station: String,
    #[serde(default, deserialize_with = "or_default")]
    pub surrogate: bool,
}

/// `/{season}/rankings/{code}`
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Rankings {
    #[serde(default, deserialize_with = "or_default")]
    pub rankings: Vec<Ranking>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ranking {
    #[serde(default, deserialize_with = "or_default")]
    pub rank: i32,
    #[serde(default, deserialize_with = "or_default")]
    pub team_number: i32,
    #[serde(default)]
    pub wins: Option<i32>,
    #[serde(default)]
    pub losses: Option<i32>,
    #[serde(default)]
    pub ties: Option<i32>,
    #[serde(default)]
    pub qual_average: Option<f64>,
    #[serde(default)]
    pub dq: Option<i32>,
    #[serde(default)]
    pub matches_played: Option<i32>,
}

/// A hybrid schedule as the CSV import would have stored it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mapped {
    /// The TBA `/event/{key}/matches` body.
    pub body: String,
    /// `{event}_qm{n}`, in schedule order.
    pub keys: Vec<String>,
    /// Every team the schedule names, sorted.
    pub teams: Vec<i32>,
    pub surrogates: Vec<Surrogate>,
    pub played: usize,
    /// Rows left out or odd, in words a lead scout can check.
    pub problems: Vec<String>,
}

/// FTC's times, as unix seconds. With no offset given they are read as the
/// event's local time, as FIRST's APIs send them.
fn unix(raw: Option<&str>, zone: Tz) -> Option<i64> {
    let raw = raw?.trim();
    if raw.is_empty() {
        return None;
    }
    if let Ok(at) = DateTime::parse_from_rfc3339(raw) {
        return Some(at.timestamp());
    }
    let naive = NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S%.f").ok()?;
    zone.from_local_datetime(&naive)
        .earliest()
        .map(|at| at.with_timezone(&Utc).timestamp())
}

/// Map the qualification hybrid schedule onto F3's matches body.
pub fn map_schedule(event_key: &str, schedule: &HybridSchedule, zone: Tz) -> Mapped {
    let slots = slot_names();
    let mut mapped = Mapped::default();
    let mut matches = Vec::new();
    let mut teams = std::collections::BTreeSet::new();

    for m in &schedule.schedule {
        let n = m.match_number;
        if n <= 0 {
            mapped
                .problems
                .push(format!("A match with no number ({n}) was left out."));
            continue;
        }
        let key = format!("{event_key}_qm{n}");
        if mapped.keys.contains(&key) {
            mapped
                .problems
                .push(format!("Q{n} is listed twice; the first was kept."));
            continue;
        }
        let mut red: [Option<i32>; ALLIANCE_SIZE] = [None; ALLIANCE_SIZE];
        let mut blue: [Option<i32>; ALLIANCE_SIZE] = [None; ALLIANCE_SIZE];
        for team in &m.teams {
            let Some(number) = team.team_number.filter(|t| *t > 0) else {
                continue;
            };
            let station = team.station.trim().to_ascii_lowercase();
            let Some(index) = slots.iter().position(|s| *s == station) else {
                mapped.problems.push(format!(
                    "Q{n}: team {number} is at station “{}”, which is not one of {}.",
                    team.station,
                    slots.join(", ")
                ));
                continue;
            };
            let (alliance, i) = if index < ALLIANCE_SIZE {
                (&mut red, index)
            } else {
                (&mut blue, index - ALLIANCE_SIZE)
            };
            alliance[i] = Some(number);
            teams.insert(number);
            if team.surrogate {
                mapped.surrogates.push(Surrogate {
                    match_number: n,
                    slot: station,
                    team: number,
                });
            }
        }
        if red.iter().chain(&blue).all(Option::is_none) {
            mapped
                .problems
                .push(format!("Q{n} has no teams yet and was left out."));
            continue;
        }

        let played = m.score_red_final.is_some() && m.score_blue_final.is_some();
        mapped.played += usize::from(played);
        // TBA's keys, with an empty key for an empty station so the robots
        // stay in their slots.
        let keys = |alliance: &[Option<i32>]| {
            alliance
                .iter()
                .map(|t| t.map(team_key).unwrap_or_default())
                .collect::<Vec<_>>()
        };
        let actual = unix(m.actual_start_time.as_deref(), zone);
        matches.push(json!({
            "key": key,
            "comp_level": "qm",
            "set_number": 1,
            "match_number": n,
            "alliances": {
                "red": { "score": m.score_red_final.unwrap_or(-1), "team_keys": keys(&red) },
                "blue": { "score": m.score_blue_final.unwrap_or(-1), "team_keys": keys(&blue) },
            },
            "time": unix(m.start_time.as_deref(), zone).unwrap_or(0),
            "actual_time": actual.unwrap_or(0),
            // Marks a scored match as played when FTC sent no start time.
            "score_breakdown": if played { json!({ "source": "ftc" }) } else { serde_json::Value::Null },
        }));
        mapped.keys.push(key);
    }
    mapped.body = serde_json::Value::Array(matches).to_string();
    mapped.teams = teams.into_iter().collect();
    mapped
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::upstream::Match;

    const SCHEDULE: &str = r#"{"schedule":[
      {"matchNumber":1,"startTime":"2026-10-24T09:00:00","actualStartTime":"2026-10-24T09:02:10",
       "scoreRedFinal":51,"scoreBlueFinal":40,"redWins":true,"blueWins":false,
       "teams":[{"teamNumber":7842,"station":"Red1","surrogate":false},
                {"teamNumber":18664,"station":"Red2","surrogate":false},
                {"teamNumber":19459,"station":"Blue1","surrogate":false},
                {"teamNumber":21830,"station":"Blue2","surrogate":true}]},
      {"matchNumber":2,"startTime":"2026-10-24T09:07:00","actualStartTime":null,
       "scoreRedFinal":null,"scoreBlueFinal":null,
       "teams":[{"teamNumber":24148,"station":"Blue2"},{"teamNumber":24378,"station":"Red1"}]},
      {"matchNumber":3,"teams":[]}
    ]}"#;

    #[test]
    fn the_hybrid_schedule_becomes_the_imports_matches_body() {
        let parsed: HybridSchedule = serde_json::from_str(SCHEDULE).unwrap();
        let mapped = map_schedule("2026usalatq", &parsed, chrono_tz::America::Chicago);
        assert_eq!(mapped.keys, ["2026usalatq_qm1", "2026usalatq_qm2"]);
        assert_eq!(mapped.played, 1);
        assert_eq!(mapped.surrogates.len(), 1);
        assert_eq!(mapped.surrogates[0].describe(), "Q1 blue2: 21830");
        assert_eq!(mapped.problems, ["Q3 has no teams yet and was left out."]);
        assert_eq!(mapped.teams.len(), 6);

        let matches: Vec<Match> = serde_json::from_str(&mapped.body).unwrap();
        assert!(matches[0].played());
        assert_eq!(matches[0].red_score(), Some(51));
        assert_eq!(matches[0].blue_teams(), [Some(19459), Some(21830)]);
        // 09:00 in Athens, Alabama, is 14:00 UTC in October.
        assert_eq!(matches[0].time, 1_792_850_400);
        assert!(!matches[1].played());
        assert_eq!(matches[1].red_score(), None);
        assert_eq!(matches[1].red_teams(), [Some(24378), None]);
        assert_eq!(matches[1].blue_teams(), [None, Some(24148)]);
    }

    #[test]
    fn rankings_and_teams_read_ftcs_envelopes() {
        let rankings: Rankings = serde_json::from_str(
            r#"{"rankings":[{"rank":1,"teamNumber":7842,"wins":2,"losses":0,"ties":1,
                "qualAverage":55.5,"dq":0,"matchesPlayed":3,"sortOrder1":2.0}]}"#,
        )
        .unwrap();
        assert_eq!(rankings.rankings[0].qual_average, Some(55.5));
        let teams: Teams = serde_json::from_str(
            r#"{"teams":[{"teamNumber":7842,"nameShort":"Bots"}],"pageTotal":1}"#,
        )
        .unwrap();
        assert_eq!(teams.teams[0].display_name(), "Bots");
        let events: Events = serde_json::from_str(r#"{"events":null}"#).unwrap();
        assert!(events.events.is_empty());
    }
}
