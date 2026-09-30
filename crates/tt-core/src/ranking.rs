//! Ranking teams by what scouts recorded (L11), and the weights behind it (L12).
//!
//! A team's scouting score is the **average** of its observations' scores, shown
//! with how many there were. The retired app summed them, so a team scouted
//! twelve times outranked a better one scouted four times on volume alone
//! (REBUILD_SPEC.md 12.9).

use std::cmp::Ordering;
use std::collections::BTreeMap;

use crate::season::{COUNTER_UNIT, FieldKind, Payload, SeasonSchema, TOGGLE_ON, WeightOverrides};

/// The range a lead scout may set a weight to.
pub const WEIGHT_MIN: i64 = -100;
pub const WEIGHT_MAX: i64 = 100;

// ── Scores ──────────────────────────────────────────────────────────────────

/// One approved observation, as the ranking needs it.
#[derive(Debug, Clone, Copy)]
pub struct Scored<'a> {
    pub team_number: i32,
    pub payload: &'a Payload,
    pub schema_version: i64,
}

/// A team's scouting score.
#[derive(Debug, Clone, PartialEq)]
pub struct TeamScore {
    pub team_number: i32,
    /// Observations that counted.
    pub n: usize,
    /// Their mean score.
    pub average: f64,
}

/// Every team's average score, by team number.
///
/// Observations recorded on another version of the form are left out: their
/// fields mean different things, and averaging them in would be quietly wrong.
pub fn team_scores(
    schema: &SeasonSchema,
    overrides: &WeightOverrides,
    observations: &[Scored],
) -> Vec<TeamScore> {
    let mut totals: BTreeMap<i32, (i64, usize)> = BTreeMap::new();
    for o in observations
        .iter()
        .filter(|o| o.schema_version == schema.version)
    {
        let entry = totals.entry(o.team_number).or_default();
        entry.0 += schema.score(o.payload, overrides);
        entry.1 += 1;
    }
    totals
        .into_iter()
        .map(|(team_number, (total, n))| TeamScore {
            team_number,
            n,
            average: total as f64 / n as f64,
        })
        .collect()
}

// ── Sorting ─────────────────────────────────────────────────────────────────

/// How the rankings table is ordered.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SortKey {
    /// The event's official rank, unranked teams last. The default.
    #[default]
    Rank,
    /// Scouting score, highest first; unscouted teams last.
    Points,
    Number,
    Name,
}

impl SortKey {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "rank" => Some(Self::Rank),
            "points" => Some(Self::Points),
            "number" => Some(Self::Number),
            "name" => Some(Self::Name),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rank => "rank",
            Self::Points => "points",
            Self::Number => "number",
            Self::Name => "name",
        }
    }
}

/// One team's line in the rankings.
#[derive(Debug, Clone, PartialEq)]
pub struct RankingRow {
    pub team_number: i32,
    pub name: String,
    pub rank: Option<i32>,
    pub score: Option<TeamScore>,
}

/// Order `rows` by `key`, breaking ties by team number then name, so the
/// order never depends on how the rows arrived.
pub fn sort(rows: &mut [RankingRow], key: SortKey) {
    // `None` last in both directions.
    fn nulls_last<T>(a: Option<T>, b: Option<T>, cmp: impl Fn(T, T) -> Ordering) -> Ordering {
        match (a, b) {
            (Some(a), Some(b)) => cmp(a, b),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        }
    }
    rows.sort_by(|a, b| {
        let primary = match key {
            SortKey::Rank => nulls_last(a.rank, b.rank, |a, b| a.cmp(&b)),
            SortKey::Points => nulls_last(
                a.score.as_ref().map(|s| s.average),
                b.score.as_ref().map(|s| s.average),
                |a, b| b.total_cmp(&a),
            ),
            SortKey::Number => Ordering::Equal,
            SortKey::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        };
        primary
            .then(a.team_number.cmp(&b.team_number))
            .then_with(|| a.name.cmp(&b.name))
    });
}

// ── Weights (L12) ───────────────────────────────────────────────────────────

/// One point value a lead scout can change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeightSlot {
    pub field_key: String,
    /// The option's key, or [`COUNTER_UNIT`] / [`TOGGLE_ON`].
    pub option_key: String,
    pub field_label: String,
    /// `"Center"`, `"per piece"`, or `"when ticked"`.
    pub option_label: String,
    /// What the season schema says.
    pub default: i64,
}

impl WeightSlot {
    /// The form input's name: `weight_{field}__{option}`, as the retired
    /// editor used.
    pub fn input_name(&self) -> String {
        format!("weight_{}__{}", self.field_key, self.option_key)
    }
}

/// Every scored value in the schema, in form order. Free text is never
/// scored, so it has none.
pub fn weight_slots(schema: &SeasonSchema) -> Vec<WeightSlot> {
    let slot =
        |field: &crate::season::Field, option_key: &str, option_label: &str, default| WeightSlot {
            field_key: field.key.clone(),
            option_key: option_key.to_string(),
            field_label: field.label.clone(),
            option_label: option_label.to_string(),
            default,
        };
    schema
        .fields()
        .flat_map(|field| match &field.kind {
            FieldKind::Select { options } => options
                .iter()
                .map(|o| slot(field, &o.key, &o.label, o.points))
                .collect(),
            FieldKind::Counter { points_each, .. } => {
                vec![slot(field, COUNTER_UNIT, "per piece", *points_each)]
            }
            FieldKind::Toggle { points } => vec![slot(field, TOGGLE_ON, "when ticked", *points)],
            FieldKind::Text { .. } => Vec::new(),
        })
        .collect()
}

/// A weights form that could not be saved: a message per input name.
pub type WeightErrors = BTreeMap<String, String>;

/// Read a posted weights form. All or nothing: one bad value and nothing is
/// saved, so a typo cannot leave half the rubric changed.
///
/// Only values that differ from the schema's default are returned: a stored
/// override then always means "changed on purpose", and resetting is emptying
/// the table. A slot missing from the post keeps its default; posted names
/// that match no slot are ignored.
pub fn read_weights(
    schema: &SeasonSchema,
    pairs: &[(String, String)],
) -> Result<WeightOverrides, WeightErrors> {
    let mut overrides = WeightOverrides::new();
    let mut errors = WeightErrors::new();
    for slot in weight_slots(schema) {
        let name = slot.input_name();
        let Some((_, raw)) = pairs.iter().rev().find(|(k, _)| *k == name) else {
            continue;
        };
        match raw.trim().parse::<i64>() {
            Ok(points) if (WEIGHT_MIN..=WEIGHT_MAX).contains(&points) => {
                if points != slot.default {
                    overrides.set(&slot.field_key, &slot.option_key, points);
                }
            }
            _ => {
                errors.insert(
                    name,
                    format!("A whole number from {WEIGHT_MIN} to {WEIGHT_MAX}."),
                );
            }
        }
    }
    if errors.is_empty() {
        Ok(overrides)
    } else {
        Err(errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::season::{Value, current_season};

    fn payload(pairs: &[(&str, Value)]) -> Payload {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn a_score_is_an_average_not_a_sum() {
        let schema = current_season().unwrap();
        // teleop is 2 points a piece.
        let heavy: Vec<Payload> = (0..4)
            .map(|_| payload(&[("teleop_scored", Value::Count(5))]))
            .collect();
        let light = payload(&[("teleop_scored", Value::Count(10))]);
        let mut observations: Vec<Scored> = heavy
            .iter()
            .map(|p| Scored {
                team_number: 1,
                payload: p,
                schema_version: schema.version,
            })
            .collect();
        observations.push(Scored {
            team_number: 2,
            payload: &light,
            schema_version: schema.version,
        });

        let scores = team_scores(&schema, &WeightOverrides::new(), &observations);
        assert_eq!(
            scores,
            [
                TeamScore {
                    team_number: 1,
                    n: 4,
                    average: 10.0
                },
                TeamScore {
                    team_number: 2,
                    n: 1,
                    average: 20.0
                },
            ],
            "scouted four times does not beat scoring more"
        );
    }

    #[test]
    fn another_form_version_is_left_out_and_overrides_apply() {
        let schema = current_season().unwrap();
        let p = payload(&[("teleop_scored", Value::Count(5))]);
        let observations = [
            Scored {
                team_number: 1,
                payload: &p,
                schema_version: schema.version,
            },
            Scored {
                team_number: 1,
                payload: &p,
                schema_version: schema.version + 1,
            },
        ];
        let mut overrides = WeightOverrides::new();
        overrides.set("teleop_scored", COUNTER_UNIT, 3);

        let scores = team_scores(&schema, &overrides, &observations);
        assert_eq!(scores[0].n, 1);
        assert_eq!(scores[0].average, 15.0);
    }

    #[test]
    fn a_team_seen_only_on_another_form_has_no_score_rather_than_zero() {
        let schema = current_season().unwrap();
        let p = payload(&[("teleop_scored", Value::Count(5))]);
        let observations = [Scored {
            team_number: 254,
            payload: &p,
            schema_version: schema.version - 1,
        }];
        assert_eq!(
            team_scores(&schema, &WeightOverrides::new(), &observations),
            [],
            "unscouted, so it sorts last rather than as a zero"
        );
    }

    #[test]
    fn an_average_keeps_its_fraction_and_can_go_negative() {
        let schema = current_season().unwrap();
        let (three, four) = (
            payload(&[("teleop_scored", Value::Count(3))]),
            payload(&[("teleop_scored", Value::Count(4))]),
        );
        let broke = payload(&[("broke_down", Value::Flag(true))]);
        let seen = |team_number, payload| Scored {
            team_number,
            payload,
            schema_version: schema.version,
        };
        let observations = [seen(1, &three), seen(1, &four), seen(2, &broke)];

        let scores = team_scores(&schema, &WeightOverrides::new(), &observations);
        assert_eq!(scores[0].average, 7.0, "(6 + 8) / 2");
        assert!(scores[1].average < 0.0, "a breakdown costs points");

        let odd = [seen(3, &three), seen(3, &four), seen(3, &four)];
        let scores = team_scores(&schema, &WeightOverrides::new(), &odd);
        assert!(
            (scores[0].average - 22.0 / 3.0).abs() < 1e-9,
            "not integer division: {}",
            scores[0].average
        );
    }

    fn row(team: i32, name: &str, rank: Option<i32>, average: Option<f64>) -> RankingRow {
        RankingRow {
            team_number: team,
            name: name.into(),
            rank,
            score: average.map(|average| TeamScore {
                team_number: team,
                n: 1,
                average,
            }),
        }
    }

    fn order(rows: &[RankingRow]) -> Vec<i32> {
        rows.iter().map(|r| r.team_number).collect()
    }

    #[test]
    fn sorting_puts_the_unknown_last_and_breaks_ties_by_number() {
        let mut rows = vec![
            row(254, "Cheesy Poofs", None, Some(12.0)),
            row(1678, "Citrus Circuits", Some(2), None),
            row(118, "Robonauts", Some(1), Some(12.0)),
            row(971, "spartan", Some(3), Some(20.0)),
        ];
        sort(&mut rows, SortKey::Rank);
        assert_eq!(order(&rows), [118, 1678, 971, 254], "unranked last");
        sort(&mut rows, SortKey::Points);
        assert_eq!(
            order(&rows),
            [971, 118, 254, 1678],
            "ties by number; unscouted last"
        );
        sort(&mut rows, SortKey::Number);
        assert_eq!(order(&rows), [118, 254, 971, 1678]);
        sort(&mut rows, SortKey::Name);
        assert_eq!(order(&rows), [254, 1678, 118, 971], "ignoring case");

        assert_eq!(SortKey::parse("points"), Some(SortKey::Points));
        assert_eq!(SortKey::parse("oops"), None);
    }

    #[test]
    fn every_scored_value_has_a_weight_and_text_has_none() {
        let schema = current_season().unwrap();
        let slots = weight_slots(&schema);
        let teleop = slots
            .iter()
            .find(|s| s.field_key == "teleop_scored")
            .expect("counter");
        assert_eq!(
            (teleop.option_label.as_str(), teleop.default),
            ("per piece", 2)
        );
        assert_eq!(teleop.input_name(), "weight_teleop_scored____each");
        assert!(slots.iter().any(|s| s.option_key == TOGGLE_ON));
        assert!(!slots.iter().any(|s| s.field_key == "notes"));
    }

    fn form(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn a_weights_form_stores_only_what_changed() {
        let schema = current_season().unwrap();
        let overrides = read_weights(
            &schema,
            &form(&[
                ("weight_teleop_scored____each", " 3 "),
                ("weight_auto_scored____each", "4"),
                ("weight_nonsense__x", "99"),
            ]),
        )
        .unwrap();
        let stored: Vec<_> = overrides.iter().collect();
        assert_eq!(
            stored,
            [("teleop_scored", COUNTER_UNIT, 3)],
            "auto kept its default of 4"
        );
    }

    #[test]
    fn one_bad_weight_rejects_the_whole_form() {
        let schema = current_season().unwrap();
        for bad in ["101", "-101", "2.5", "lots", ""] {
            let errors = read_weights(
                &schema,
                &form(&[
                    ("weight_teleop_scored____each", "3"),
                    ("weight_auto_scored____each", bad),
                ]),
            )
            .unwrap_err();
            assert_eq!(errors.len(), 1, "{bad:?}");
            assert!(errors.contains_key("weight_auto_scored____each"));
        }
        assert!(read_weights(&schema, &form(&[("weight_auto_scored____each", "-100")])).is_ok());
    }
}
