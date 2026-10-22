//! FTC Events API payloads for the one optional pull (FTC_BETA_PLAN.md, F7).
//!
//! One event, `USALATQ`, season 2026; `docs/ftc/FTC_BETA_API.md` has the mapping.
//! Events and teams have FRC's field names, so they reuse
//! [`FirstEvent`](crate::upstream::FirstEvent) and
//! [`FirstTeam`](crate::upstream::FirstTeam) inside FTC's lowercase
//! envelopes. The hybrid schedule becomes the TBA `/event/{key}/matches` body
//! the CSV import (F3) writes, so both paths store matches the same way: two
//! robots an alliance by station, `-1` for an unplayed score, and keys
//! `{event}_qm{n}`. Rankings fill the `TeamEventStats` columns they match.
//! FTC publishes no OPRs, so [`ratings`] works them out from the played
//! matches (F10), in TBA's shapes, for the columns TBA's fill in FRC.

use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use serde::Deserialize;
use serde_json::json;

use crate::matches::{ALLIANCE_SIZE, slot_names};
use crate::opr;
use crate::schedule::Surrogate;
use crate::upstream::{ComponentOprs, FirstEvent, FirstTeam, Oprs, or_default, team_key};

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
    /// Foul points each alliance was given, inside its final score.
    #[serde(default)]
    pub score_red_foul: Option<i64>,
    #[serde(default)]
    pub score_blue_foul: Option<i64>,
    #[serde(default)]
    pub score_red_auto: Option<i64>,
    #[serde(default)]
    pub score_blue_auto: Option<i64>,
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

/// `/{season}/scores/{code}/qual`: a score sheet per match, a side per
/// alliance. Only the phase totals are read here; the log keeps the rest.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScoreSheets {
    #[serde(default, deserialize_with = "or_default")]
    pub match_scores: Vec<ScoreSheet>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScoreSheet {
    #[serde(default, deserialize_with = "or_default")]
    pub match_number: i32,
    #[serde(default, deserialize_with = "or_default")]
    pub alliances: Vec<SheetSide>,
}

/// One alliance's side of a score sheet.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SheetSide {
    /// `Red` or `Blue`.
    #[serde(default, deserialize_with = "or_default")]
    pub alliance: String,
    #[serde(default)]
    pub auto_points: Option<f64>,
    /// TELEOP, with the PARK at the end of the match in it.
    #[serde(default)]
    pub teleop_points: Option<f64>,
    #[serde(default)]
    pub teleop_park_points: Option<f64>,
}

impl ScoreSheets {
    fn side(&self, match_number: i32, red: bool) -> Option<&SheetSide> {
        let color = if red { "red" } else { "blue" };
        self.match_scores
            .iter()
            .find(|s| s.match_number == match_number)?
            .alliances
            .iter()
            .find(|a| a.alliance.trim().eq_ignore_ascii_case(color))
    }
}

/// OPR, DPR, and CCWM, and the phase OPRs, worked out from the qualification
/// results (F10) in TBA's `/oprs` and `/coprs` shapes, so they are stored and
/// shown as FRC's are.
#[derive(Debug, Clone, Default)]
pub struct Worked {
    pub oprs: Oprs,
    pub components: ComponentOprs,
    /// The played matches they were worked out from.
    pub played: usize,
}

/// The phase components' names, as TBA's `/coprs` would name them, so
/// [`ComponentOprs::phase_opr`] finds them.
pub const AUTO_POINTS: &str = "autoPoints";
pub const TELEOP_POINTS: &str = "teleopPoints";
pub const ENDGAME_POINTS: &str = "endgamePoints";

/// Work the ratings out from the hybrid schedule's played matches, surrogate
/// robots included: a surrogate's match still counts for its robot.
///
/// Each alliance's score is its final score less the foul points it was
/// given, as TBA leaves fouls out: a foul is the other alliance's doing. A
/// match with no foul figure counts its final score. AUTO is the schedule's;
/// TELEOP and the end-of-match PARK (the endgame) come from the score
/// sheets, for the matches that have one. A fit the matches do not determine
/// is left out, so too few matches leave every rating empty.
pub fn ratings(schedule: &HybridSchedule, sheets: &ScoreSheets) -> Worked {
    let slots = slot_names();
    let mut played = Vec::new();
    let mut auto = Vec::new();
    let mut teleop = Vec::new();
    let mut endgame = Vec::new();
    for m in &schedule.schedule {
        let (Some(red_final), Some(blue_final)) = (m.score_red_final, m.score_blue_final) else {
            continue;
        };
        let (mut red, mut blue) = (Vec::new(), Vec::new());
        for team in &m.teams {
            let Some(number) = team.team_number.filter(|t| *t > 0) else {
                continue;
            };
            let station = team.station.trim().to_ascii_lowercase();
            match slots.iter().position(|s| *s == station) {
                Some(i) if i < ALLIANCE_SIZE => red.push(number),
                Some(_) => blue.push(number),
                None => {}
            }
        }
        if red.is_empty() || blue.is_empty() {
            continue;
        }
        let net = |total: i64, foul: Option<i64>| (total - foul.unwrap_or(0).max(0)) as f64;
        played.push(opr::Played {
            red: red.clone(),
            blue: blue.clone(),
            red_points: net(red_final, m.score_red_foul),
            blue_points: net(blue_final, m.score_blue_foul),
        });
        for (teams, is_red, hybrid_auto) in [
            (&red, true, m.score_red_auto),
            (&blue, false, m.score_blue_auto),
        ] {
            let side = sheets.side(m.match_number, is_red);
            let row = |points: Option<f64>| {
                points.map(|points| opr::Row {
                    teams: teams.clone(),
                    points,
                })
            };
            auto.extend(row(hybrid_auto
                .map(|a| a as f64)
                .or_else(|| side.and_then(|s| s.auto_points))));
            let park = side.and_then(|s| s.teleop_park_points);
            teleop.extend(row(side.and_then(|s| Some(s.teleop_points? - park?))));
            endgame.extend(row(park));
        }
    }

    let mut worked = Worked {
        played: played.len(),
        ..Worked::default()
    };
    if let Some(rated) = opr::event(&played) {
        for (team, r) in rated {
            let key = team_key(team);
            worked.oprs.oprs.insert(key.clone(), r.opr);
            worked.oprs.dprs.insert(key.clone(), r.dpr);
            worked.oprs.ccwms.insert(key, r.ccwm);
        }
    }
    for (name, rows) in [
        (AUTO_POINTS, auto),
        (TELEOP_POINTS, teleop),
        (ENDGAME_POINTS, endgame),
    ] {
        if let Some(fit) = opr::solve(&rows) {
            worked.components.components.insert(
                name.to_string(),
                fit.into_iter().map(|(t, v)| (team_key(t), v)).collect(),
            );
        }
    }
    worked
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

    /// Four teams, each with each other once (`opr`'s hand-worked case):
    /// A 7842, B 18664, C 19459, D 21830 make 10, 20, 30, 40 a match.
    fn round_robin() -> (HybridSchedule, ScoreSheets) {
        let team = |n: i32, station: &str, surrogate: bool| json!({ "teamNumber": n, "station": station, "surrogate": surrogate });
        // (number, red, blue, final, foul, auto) per alliance.
        let played = [
            (1, [7842, 18664], [19459, 21830], (35, 70), (5, 0), (3, 7)),
            (2, [7842, 19459], [18664, 21830], (40, 60), (0, 0), (4, 6)),
            (3, [7842, 21830], [18664, 19459], (50, 58), (0, 8), (5, 5)),
        ];
        let mut schedule: Vec<_> = played
            .iter()
            .map(|(n, red, blue, finals, fouls, autos)| {
                json!({
                    "matchNumber": n,
                    "scoreRedFinal": finals.0, "scoreBlueFinal": finals.1,
                    "scoreRedFoul": fouls.0, "scoreBlueFoul": fouls.1,
                    "scoreRedAuto": autos.0, "scoreBlueAuto": autos.1,
                    "teams": [team(red[0], "Red1", false), team(red[1], "Red2", false),
                              // Q3's blue2 is a surrogate: it still counts.
                              team(blue[0], "Blue1", false), team(blue[1], "Blue2", *n == 3)],
                })
            })
            .collect();
        // Not played yet: left out.
        schedule.push(json!({ "matchNumber": 4, "scoreRedFinal": null,
            "teams": [team(7842, "Red1", false), team(18664, "Blue1", false)] }));
        let schedule = serde_json::from_value(json!({ "schedule": schedule })).unwrap();

        // TELEOP without PARK: 2, 4, 6, 8. PARK: 5, 5, 0, 10. Q2 has no sheet.
        let side = |alliance: &str, teleop: f64, park: f64| {
            json!({ "alliance": alliance, "autoPoints": 99, "teleopPoints": teleop + park,
                    "teleopParkPoints": park })
        };
        let sheets = serde_json::from_value(json!({ "matchScores": [
            { "matchNumber": 1, "alliances": [side("Red", 6.0, 10.0), side("Blue", 14.0, 10.0)] },
            { "matchNumber": 3, "alliances": [side("Red", 10.0, 15.0), side("Blue", 10.0, 5.0)] },
            { "matchNumber": 5, "alliances": [side("Red", 1.0, 1.0)] },
        ]}))
        .unwrap();
        (schedule, sheets)
    }

    #[test]
    fn ratings_come_from_the_played_matches_less_fouls() {
        let (schedule, sheets) = round_robin();
        let worked = ratings(&schedule, &sheets);
        assert_eq!(worked.played, 3);
        let near = |got: Option<&f64>, want: f64| {
            let got = *got.expect("rated");
            assert!((got - want).abs() < 1e-9, "{got} is not {want}");
        };
        for (team, opr, dpr) in [
            (7842, 10.0, 40.0),
            (18664, 20.0, 30.0),
            (19459, 30.0, 20.0),
            (21830, 40.0, 10.0),
        ] {
            let key = team_key(team);
            near(worked.oprs.oprs.get(&key), opr);
            near(worked.oprs.dprs.get(&key), dpr);
            near(worked.oprs.ccwms.get(&key), opr - dpr);
        }
        // AUTO from the schedule: 1, 2, 3, 4.
        let c = &worked.components;
        let phase = |team: i32, p| c.phase_opr(&team_key(team), p);
        use crate::upstream::Phase;
        near(phase(21830, Phase::Auto).as_ref(), 4.0);
        // Two sheets are 4 equations for 4 teams, but Q1 and Q3 alone do not
        // separate them: TELEOP and the endgame stay empty.
        assert_eq!(phase(7842, Phase::Teleop), None);
        assert_eq!(phase(7842, Phase::Endgame), None);
    }

    #[test]
    fn a_sheet_for_every_match_gives_teleop_and_the_endgame() {
        let (schedule, mut sheets) = round_robin();
        let q2: ScoreSheet = serde_json::from_value(json!({ "matchNumber": 2, "alliances": [
            { "alliance": "Red", "teleopPoints": 13.0, "teleopParkPoints": 5.0 },
            { "alliance": "Blue", "teleopPoints": 27.0, "teleopParkPoints": 15.0 },
        ]}))
        .unwrap();
        sheets.match_scores.push(q2);
        let worked = ratings(&schedule, &sheets);
        use crate::upstream::Phase;
        for (team, teleop, park) in [(7842, 2.0, 5.0), (19459, 6.0, 0.0), (21830, 8.0, 10.0)] {
            let key = team_key(team);
            let t = worked.components.phase_opr(&key, Phase::Teleop).unwrap();
            let e = worked.components.phase_opr(&key, Phase::Endgame).unwrap();
            assert!(
                (t - teleop).abs() < 1e-9 && (e - park).abs() < 1e-9,
                "{team}: {t} {e}"
            );
        }
    }

    #[test]
    fn one_played_match_rates_nobody() {
        let parsed: HybridSchedule = serde_json::from_str(SCHEDULE).unwrap();
        let worked = ratings(&parsed, &ScoreSheets::default());
        assert_eq!(worked.played, 1);
        assert!(worked.oprs.oprs.is_empty() && worked.components.components.is_empty());
    }
}
