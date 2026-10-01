//! The graph view (U21): what each team did, match by match, as numbers a
//! chart can draw.
//!
//! # What a line is
//!
//! One team, one metric. Its points are the team's scouted matches in order:
//! events by date, then the schedule. The x axis is "the team's first match,
//! second, third", not the event's match numbers, so two teams' trends line up
//! however the schedule fell, and a team's events run on into one another.
//! Two scouts on one robot in one match make one point, their average.
//!
//! # What a metric is
//!
//! Anything a scout counts, read off the season file, plus the scouting score
//! and three of The Blue Alliance's numbers. Free text is never a metric, and
//! neither is a choice whose options carry no points (where a robot started
//! has no "higher"). TBA's numbers are one per event, so their line is flat
//! across that event's matches.

use std::collections::BTreeMap;

use crate::records::TeamEventStats;
use crate::season::{FieldKind, Payload, SeasonSchema, Value, WeightOverrides};

/// Teams on the chart at once: the categorical palette's eight colours. A
/// ninth would need a colour nobody can tell from the others.
pub const MAX_TEAMS: usize = 8;
/// Metrics on the chart at once: one line style each, solid, dashed, dotted.
pub const MAX_METRICS: usize = 3;
/// Teams shown before the viewer chooses: the best scouted.
pub const DEFAULT_TEAMS: usize = 3;
/// The scouting score's key, and the metric shown before the viewer chooses.
pub const POINTS: &str = "points";

/// Where a metric's numbers come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The observation's score, with the lead scout's weights.
    Points,
    /// One field of the form.
    Field,
    Opr,
    Dpr,
    Ccwm,
}

/// One thing the chart can draw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Metric {
    /// In the URL: `points`, `f.<field key>`, `opr`, `dpr`, `ccwm`.
    pub key: String,
    pub label: String,
    pub source: Source,
}

impl Metric {
    /// From The Blue Alliance rather than from scouts.
    pub fn from_tba(&self) -> bool {
        matches!(self.source, Source::Opr | Source::Dpr | Source::Ccwm)
    }

    fn field_key(&self) -> &str {
        self.key.strip_prefix("f.").unwrap_or(&self.key)
    }
}

/// Every metric the chart offers, scouting first in form order, then TBA's.
pub fn metrics(schema: &SeasonSchema) -> Vec<Metric> {
    let mut all = vec![Metric {
        key: POINTS.into(),
        label: "Scouting points".into(),
        source: Source::Points,
    }];
    for field in schema.fields() {
        let label = match &field.kind {
            FieldKind::Counter { .. } => field.label.clone(),
            FieldKind::Toggle { .. } => format!("{} (1 = yes)", field.label),
            FieldKind::Select { options } if options.iter().any(|o| o.points != 0) => {
                format!("{} (points)", field.label)
            }
            _ => continue,
        };
        all.push(Metric {
            key: format!("f.{}", field.key),
            label,
            source: Source::Field,
        });
    }
    for (key, label, source) in [
        ("opr", "OPR (TBA, per event)", Source::Opr),
        ("dpr", "DPR (TBA, per event)", Source::Dpr),
        ("ccwm", "CCWM (TBA, per event)", Source::Ccwm),
    ] {
        all.push(Metric {
            key: key.into(),
            label: label.into(),
            source,
        });
    }
    all
}

/// `metric`'s value in one observation. `None` when the scout left it out,
/// and for TBA's metrics, which no observation holds.
pub fn value(
    schema: &SeasonSchema,
    overrides: &WeightOverrides,
    metric: &Metric,
    payload: &Payload,
) -> Option<f64> {
    match metric.source {
        Source::Points => Some(schema.score(payload, overrides) as f64),
        Source::Field => {
            let field = schema.field(metric.field_key())?;
            match (&field.kind, payload.get(&field.key)?) {
                (FieldKind::Counter { .. }, Value::Count(n)) => Some(*n as f64),
                (FieldKind::Toggle { .. }, Value::Flag(on)) => Some(if *on { 1.0 } else { 0.0 }),
                (FieldKind::Select { options }, Value::Text(chosen)) => options
                    .iter()
                    .find(|o| &o.key == chosen)
                    .map(|o| overrides.points_for(&field.key, &o.key, o.points) as f64),
                _ => None,
            }
        }
        Source::Opr | Source::Dpr | Source::Ccwm => None,
    }
}

fn tba(metric: &Metric, stats: &TeamEventStats) -> Option<f64> {
    match metric.source {
        Source::Opr => stats.opr,
        Source::Dpr => stats.dpr,
        Source::Ccwm => stats.ccwm,
        Source::Points | Source::Field => None,
    }
}

// ── Series ──────────────────────────────────────────────────────────────────

/// One approved observation, placed in time.
#[derive(Debug, Clone, Copy)]
pub struct Sample<'a> {
    pub team_number: i32,
    pub event_key: &'a str,
    pub match_key: &'a str,
    /// Its event's place among the events drawn, then its match's place in
    /// that event's schedule. A match gone from the schedule sorts last.
    pub place: (usize, usize),
    pub payload: &'a Payload,
    pub schema_version: i64,
}

/// One of a team's matches on the chart.
#[derive(Debug, Clone, PartialEq)]
pub struct Point {
    pub event_key: String,
    pub match_key: String,
    /// How many approved observations it averages.
    pub observed: usize,
    /// One per metric, in the order asked for.
    pub values: Vec<Option<f64>>,
}

/// A team in one match: the team, where the match falls, and its key. Sorts
/// into the order a line is drawn in.
type InMatch<'a> = (i32, (usize, usize), &'a str);

/// Each team's points, in match order, one value per metric in `metrics`.
///
/// Observations on another version of the form are left out, as everywhere:
/// their fields mean different things. `stats` gives TBA's numbers for a team
/// at an event.
pub fn series<'s>(
    schema: &SeasonSchema,
    overrides: &WeightOverrides,
    metrics: &[Metric],
    samples: &[Sample],
    stats: impl Fn(&str, i32) -> Option<&'s TeamEventStats>,
) -> BTreeMap<i32, Vec<Point>> {
    let mut matches: BTreeMap<InMatch, Vec<&Sample>> = BTreeMap::new();
    for s in samples
        .iter()
        .filter(|s| s.schema_version == schema.version)
    {
        matches
            .entry((s.team_number, s.place, s.match_key))
            .or_default()
            .push(s);
    }
    let mut out: BTreeMap<i32, Vec<Point>> = BTreeMap::new();
    for ((team, _, match_key), observed) in matches {
        let event_key = observed[0].event_key;
        let team_stats = stats(event_key, team);
        let values = metrics
            .iter()
            .map(|metric| {
                if metric.from_tba() {
                    return team_stats.and_then(|s| tba(metric, s));
                }
                let found: Vec<f64> = observed
                    .iter()
                    .filter_map(|s| value(schema, overrides, metric, s.payload))
                    .collect();
                (!found.is_empty()).then(|| found.iter().sum::<f64>() / found.len() as f64)
            })
            .collect();
        out.entry(team).or_default().push(Point {
            event_key: event_key.to_string(),
            match_key: match_key.to_string(),
            observed: observed.len(),
            values,
        });
    }
    out
}

// ── Choosing ────────────────────────────────────────────────────────────────

/// What the URL asked for, kept to what is on offer, without repeats, and at
/// most `max`. Also says how many were dropped for the limit, so the page can
/// say so rather than lose them quietly.
pub fn choose<T: PartialEq + Clone>(asked: &[T], offered: &[T], max: usize) -> (Vec<T>, usize) {
    let mut kept: Vec<T> = Vec::new();
    for item in asked {
        if offered.contains(item) && !kept.contains(item) {
            kept.push(item.clone());
        }
    }
    let over = kept.len().saturating_sub(max);
    kept.truncate(max);
    (kept, over)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::season::current_season;

    fn payload(pairs: &[(&str, Value)]) -> Payload {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    fn metric(schema: &SeasonSchema, key: &str) -> Metric {
        metrics(schema)
            .into_iter()
            .find(|m| m.key == key)
            .unwrap_or_else(|| panic!("{key}"))
    }

    #[test]
    fn metrics_are_what_scouts_count_then_tbas() {
        let schema = current_season().unwrap();
        let keys: Vec<String> = metrics(&schema).into_iter().map(|m| m.key).collect();
        assert_eq!(
            keys,
            [
                "points",
                "f.no_show",
                "f.auto_scored",
                "f.teleop_scored",
                "f.endgame",
                "f.defense_rating",
                "f.driver_skill",
                "f.speed",
                "f.broke_down",
                "f.penalties",
                "opr",
                "dpr",
                "ccwm",
            ],
            "no starting position (no points), no notes"
        );
        assert_eq!(
            metric(&schema, "f.endgame").label,
            "Endgame result (points)"
        );
        assert_eq!(
            metric(&schema, "f.broke_down").label,
            "Broke down or tipped (1 = yes)"
        );
    }

    #[test]
    fn a_value_is_a_count_a_flag_or_a_choices_weighted_points() {
        let schema = current_season().unwrap();
        let mut weights = WeightOverrides::new();
        weights.set("endgame", "full", 10);
        let p = payload(&[
            ("teleop_scored", Value::Count(7)),
            ("broke_down", Value::Flag(true)),
            ("endgame", Value::Text("full".into())),
        ]);
        let v = |key: &str| value(&schema, &weights, &metric(&schema, key), &p);
        assert_eq!(v("f.teleop_scored"), Some(7.0));
        assert_eq!(v("f.broke_down"), Some(1.0));
        assert_eq!(
            v("f.endgame"),
            Some(10.0),
            "the lead's weight, not the file's"
        );
        assert_eq!(v("f.auto_scored"), None, "left out is not zero");
        assert_eq!(v("points"), Some((7 * 2 - 5 + 10) as f64));
        assert_eq!(v("opr"), None, "no observation holds OPR");
    }

    #[test]
    fn a_line_is_a_teams_matches_in_order_with_two_scouts_averaged() {
        let schema = current_season().unwrap();
        let none = WeightOverrides::new();
        let wanted = [metric(&schema, "f.teleop_scored"), metric(&schema, "opr")];
        let (a, b, c, d, old) = (
            payload(&[("teleop_scored", Value::Count(4))]),
            payload(&[("teleop_scored", Value::Count(6))]),
            payload(&[("teleop_scored", Value::Count(9))]),
            payload(&[]),
            payload(&[("teleop_scored", Value::Count(60))]),
        );
        let sample = |match_key, place: (usize, usize), payload, schema_version| Sample {
            team_number: 254,
            event_key: if place.0 == 0 { "2026one" } else { "2026two" },
            match_key,
            place,
            payload,
            schema_version,
        };
        let samples = [
            // Out of order, as storage may hand them over.
            sample("2026two_qm1", (1, 0), &d, schema.version),
            sample("2026one_qm3", (0, 2), &c, schema.version),
            sample("2026one_qm1", (0, 0), &a, schema.version),
            sample("2026one_qm1", (0, 0), &b, schema.version),
            sample("2026one_qm2", (0, 1), &old, schema.version + 1),
        ];
        let stats = TeamEventStats {
            team_number: 254,
            event_key: "2026one".into(),
            opr: Some(41.5),
            ..Default::default()
        };
        let lines = series(&schema, &none, &wanted, &samples, |event, team| {
            (event == "2026one" && team == 254).then_some(&stats)
        });
        let points = &lines[&254];
        let keys: Vec<&str> = points.iter().map(|p| p.match_key.as_str()).collect();
        assert_eq!(
            keys,
            ["2026one_qm1", "2026one_qm3", "2026two_qm1"],
            "events in order, then the schedule; another form version left out"
        );
        assert_eq!(points[0].observed, 2);
        assert_eq!(
            points[0].values,
            [Some(5.0), Some(41.5)],
            "two scouts averaged"
        );
        assert_eq!(
            points[1].values,
            [Some(9.0), Some(41.5)],
            "OPR is flat across the event"
        );
        assert_eq!(
            points[2].values,
            [None, None],
            "nothing counted, no OPR synced"
        );
    }

    #[test]
    fn a_choice_keeps_what_is_offered_once_and_says_what_the_limit_cut() {
        let offered = [1, 2, 3, 4, 5];
        assert_eq!(choose(&[3, 9, 3, 1], &offered, 8), (vec![3, 1], 0));
        assert_eq!(choose(&[5, 4, 3, 2, 1], &offered, 3), (vec![5, 4, 3], 2));
        assert_eq!(choose::<i32>(&[], &offered, 3), (vec![], 0));
    }
}
