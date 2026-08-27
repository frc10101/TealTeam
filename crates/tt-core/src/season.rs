//! Season schemas: the fix for the January rewrite treadmill (D4).
//!
//! # The problem this solves
//!
//! The retired implementation had one column per observation, named after the
//! 2022 game: `hang_level`, `traversal`, `hub_auto_count`. Every January the game
//! changes completely and every one of those columns becomes wrong -- so every
//! January somebody rewrote the schema, the form template, the aggregation, and
//! the scoring, under time pressure, days before the first event
//! (REBUILD_SPEC.md 12.1).
//!
//! Here, a season is **data**. `seasons/2026.json` declares the fields; the form
//! renderer, the payload validator, and the scorer all read that declaration.
//! Next January is a new JSON file and a migration, not a rewrite.
//!
//! # What a schema is
//!
//! A `SeasonSchema` is an ordered list of [`Section`]s, each an ordered list of
//! [`Field`]s. Order is display order -- a scout works down the form in the order
//! things happen in a match, so the JSON is authored in that order and preserved
//! exactly.
//!
//! Selectable options carry point values, which is what makes a schema also a
//! scoring rubric. Weights can be overridden at runtime by the lead scout
//! (L11/L12) without touching the schema.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

use crate::error::{DomainError, Result};

/// How long a free-text answer may be, in characters.
///
/// Notes are scouting shorthand typed on a phone between matches, not essays.
/// A cap keeps one stuck key from filling the Pi's disk.
pub const DEFAULT_TEXT_LIMIT: usize = 2_000;

// ── Schema ──────────────────────────────────────────────────────────────────

/// A season's scouting form and scoring rubric.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeasonSchema {
    /// FRC season year, e.g. `2026`.
    pub season: i32,

    /// Bumped whenever the field set changes in a way that makes older payloads
    /// non-comparable. Stored alongside every scouting row so old data stays
    /// readable and aggregation can refuse to mix incompatible versions.
    pub version: i64,

    /// Display name of the game, e.g. "Rebuilt" for 2026.
    #[serde(default)]
    pub name: String,

    pub sections: Vec<Section>,
}

/// A group of fields shown together, e.g. "Autonomous".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Section {
    pub key: String,
    pub label: String,
    pub fields: Vec<Field>,
}

/// One thing a scout records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Field {
    /// Stable identifier. This is the payload key, so **renaming it breaks old
    /// data** -- bump [`SeasonSchema::version`] instead.
    pub key: String,

    pub label: String,

    /// Optional hint shown under the input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,

    #[serde(default)]
    pub required: bool,

    #[serde(flatten)]
    pub kind: FieldKind,
}

/// What sort of input a field is, and what values it accepts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FieldKind {
    /// One of a fixed set. The workhorse: fast to tap, trivial to aggregate.
    Select { options: Vec<Choice> },

    /// A tally, e.g. "game pieces scored". Bounded so a stuck button cannot
    /// record 4 billion.
    Counter {
        #[serde(default)]
        min: i64,
        max: i64,
        /// Points awarded per unit counted.
        #[serde(default)]
        points_each: i64,
    },

    /// Yes/no. Points apply when true.
    Toggle {
        #[serde(default)]
        points: i64,
    },

    /// Free text. Never scored -- prose cannot be ranked.
    Text {
        #[serde(default = "default_text_limit")]
        max_len: usize,
    },
}

fn default_text_limit() -> usize {
    DEFAULT_TEXT_LIMIT
}

/// One option of a [`FieldKind::Select`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Choice {
    pub key: String,
    pub label: String,
    #[serde(default)]
    pub points: i64,
}

impl SeasonSchema {
    /// Parse and validate a schema document.
    ///
    /// Validation is strict on purpose: a schema is loaded once at startup, and
    /// a typo caught there is infinitely cheaper than one discovered by fifty
    /// scouts at an event.
    pub fn parse(json: &str) -> Result<Self> {
        let schema: SeasonSchema =
            serde_json::from_str(json).map_err(|e| DomainError::Invalid {
                field: "season schema",
                value: e.to_string(),
            })?;
        schema.validate()?;
        Ok(schema)
    }

    /// Check internal consistency: unique keys everywhere, sane bounds, no empty
    /// selects.
    pub fn validate(&self) -> Result<()> {
        if self.version < 1 {
            return Err(DomainError::Invalid {
                field: "version",
                value: self.version.to_string(),
            });
        }
        if self.sections.is_empty() {
            return Err(DomainError::Missing { field: "sections" });
        }

        let mut section_keys = HashSet::new();
        let mut field_keys = HashSet::new();

        for section in &self.sections {
            if section.key.trim().is_empty() {
                return Err(DomainError::Missing {
                    field: "section key",
                });
            }
            if !section_keys.insert(section.key.as_str()) {
                return Err(DomainError::Invalid {
                    field: "duplicate section key",
                    value: section.key.clone(),
                });
            }

            for field in &section.fields {
                if field.key.trim().is_empty() {
                    return Err(DomainError::Missing { field: "field key" });
                }
                // Field keys are unique across the WHOLE schema, not per section:
                // the payload is one flat map, so two sections sharing a key
                // would silently overwrite each other.
                if !field_keys.insert(field.key.as_str()) {
                    return Err(DomainError::Invalid {
                        field: "duplicate field key",
                        value: field.key.clone(),
                    });
                }
                field.validate()?;
            }
        }

        if field_keys.is_empty() {
            return Err(DomainError::Missing { field: "fields" });
        }
        Ok(())
    }

    /// Every field, in display order, ignoring section grouping.
    pub fn fields(&self) -> impl Iterator<Item = &Field> {
        self.sections.iter().flat_map(|s| s.fields.iter())
    }

    pub fn field(&self, key: &str) -> Option<&Field> {
        self.fields().find(|f| f.key == key)
    }
}

impl Field {
    fn validate(&self) -> Result<()> {
        match &self.kind {
            FieldKind::Select { options } => {
                if options.is_empty() {
                    return Err(DomainError::Missing {
                        field: "select options",
                    });
                }
                let mut seen = HashSet::new();
                for option in options {
                    if option.key.trim().is_empty() {
                        return Err(DomainError::Missing {
                            field: "option key",
                        });
                    }
                    if !seen.insert(option.key.as_str()) {
                        return Err(DomainError::Invalid {
                            field: "duplicate option key",
                            value: format!("{}.{}", self.key, option.key),
                        });
                    }
                }
                Ok(())
            }
            FieldKind::Counter { min, max, .. } => {
                if max <= min {
                    return Err(DomainError::Invalid {
                        field: "counter range",
                        value: format!("{}: max {max} must exceed min {min}", self.key),
                    });
                }
                Ok(())
            }
            FieldKind::Toggle { .. } => Ok(()),
            FieldKind::Text { max_len } => {
                if *max_len == 0 {
                    return Err(DomainError::Invalid {
                        field: "text max_len",
                        value: self.key.clone(),
                    });
                }
                Ok(())
            }
        }
    }

    /// Whether this field contributes to a team's ranking score.
    pub fn is_scored(&self) -> bool {
        !matches!(self.kind, FieldKind::Text { .. })
    }
}

// ── Payload ─────────────────────────────────────────────────────────────────

/// One recorded answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Value {
    /// A [`FieldKind::Select`] option key, or [`FieldKind::Text`] content.
    Text(String),
    Count(i64),
    Flag(bool),
}

impl Value {
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Value::Text(s) => Some(s),
            _ => None,
        }
    }
}

/// A scout's answers for one robot in one match.
///
/// A `BTreeMap` rather than a `HashMap` so serialisation is deterministic --
/// which matters because these payloads get hashed and compared during sync
/// (Phase 3) and diffed by humans in the meantime.
pub type Payload = BTreeMap<String, Value>;

/// Parse a JSON object into a [`Payload`].
pub fn parse_payload(json: &str) -> Result<Payload> {
    serde_json::from_str(json).map_err(|e| DomainError::Invalid {
        field: "payload",
        value: e.to_string(),
    })
}

pub fn payload_to_json(payload: &Payload) -> String {
    // Only fails on non-string map keys or non-finite floats; we have neither.
    serde_json::to_string(payload).unwrap_or_else(|_| "{}".to_string())
}

impl SeasonSchema {
    /// Check a payload against this schema.
    ///
    /// Rejects unknown keys as well as bad values. That is deliberate: an unknown
    /// key means the client is running a different schema version, and silently
    /// dropping it would lose a scout's work without telling anybody.
    pub fn validate_payload(&self, payload: &Payload) -> Result<()> {
        for key in payload.keys() {
            if self.field(key).is_none() {
                return Err(DomainError::Invalid {
                    field: "unknown field",
                    value: key.clone(),
                });
            }
        }

        for field in self.fields() {
            match payload.get(&field.key) {
                None => {
                    if field.required {
                        return Err(DomainError::Missing {
                            field: leak(&field.key),
                        });
                    }
                }
                Some(value) => field.validate_value(value)?,
            }
        }
        Ok(())
    }

    /// Total ranking points for one payload.
    ///
    /// `overrides` maps `"field_key.option_key"` to a replacement point value,
    /// letting a lead scout retune weights mid-event without editing the schema
    /// (L11/L12). Unknown override keys are ignored rather than erroring -- a
    /// stale override left over from last season must not break scoring.
    pub fn score(&self, payload: &Payload, overrides: &WeightOverrides) -> i64 {
        let mut total = 0i64;

        for field in self.fields() {
            let Some(value) = payload.get(&field.key) else {
                continue;
            };

            total += match (&field.kind, value) {
                (FieldKind::Select { options }, Value::Text(chosen)) => options
                    .iter()
                    .find(|o| &o.key == chosen)
                    .map(|o| overrides.points_for(&field.key, &o.key, o.points))
                    .unwrap_or(0),

                (FieldKind::Counter { points_each, .. }, Value::Count(n)) => {
                    let each = overrides.points_for(&field.key, COUNTER_UNIT, *points_each);
                    each.saturating_mul(*n)
                }

                (FieldKind::Toggle { points }, Value::Flag(true)) => {
                    overrides.points_for(&field.key, TOGGLE_ON, *points)
                }

                _ => 0,
            };
        }

        total
    }
}

/// Override key used for a counter's per-unit value.
pub const COUNTER_UNIT: &str = "__each";
/// Override key used for a toggle's "on" value.
pub const TOGGLE_ON: &str = "__on";

impl Field {
    fn validate_value(&self, value: &Value) -> Result<()> {
        match (&self.kind, value) {
            (FieldKind::Select { options }, Value::Text(chosen)) => {
                if options.iter().any(|o| &o.key == chosen) {
                    Ok(())
                } else {
                    Err(DomainError::Invalid {
                        field: leak(&self.key),
                        value: chosen.clone(),
                    })
                }
            }
            (FieldKind::Counter { min, max, .. }, Value::Count(n)) => {
                if n >= min && n <= max {
                    Ok(())
                } else {
                    Err(DomainError::Invalid {
                        field: leak(&self.key),
                        value: n.to_string(),
                    })
                }
            }
            (FieldKind::Toggle { .. }, Value::Flag(_)) => Ok(()),
            (FieldKind::Text { max_len }, Value::Text(s)) => {
                // Count characters, not bytes: a scout typing emoji should not
                // hit a limit two thirds early.
                if s.chars().count() <= *max_len {
                    Ok(())
                } else {
                    Err(DomainError::Invalid {
                        field: leak(&self.key),
                        value: format!("{} characters, limit {max_len}", s.chars().count()),
                    })
                }
            }
            // Right field, wrong shape of answer.
            _ => Err(DomainError::Invalid {
                field: leak(&self.key),
                value: format!("wrong value type: {value:?}"),
            }),
        }
    }
}

/// `DomainError` holds `&'static str` field names for cheap construction, but
/// schema field names are runtime strings. Leaking is acceptable here because
/// the set of field keys is small, fixed by the schema, and lives for the
/// process lifetime anyway.
fn leak(key: &str) -> &'static str {
    Box::leak(key.to_string().into_boxed_str())
}

// ── Weight overrides ────────────────────────────────────────────────────────

/// Runtime point-value overrides, keyed by `(field_key, option_key)`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WeightOverrides {
    values: HashMap<(String, String), i64>,
}

impl WeightOverrides {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&mut self, field_key: &str, option_key: &str, points: i64) {
        self.values
            .insert((field_key.to_string(), option_key.to_string()), points);
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// The override for this option, or `default` if none is set.
    pub fn points_for(&self, field_key: &str, option_key: &str, default: i64) -> i64 {
        self.values
            .get(&(field_key.to_string(), option_key.to_string()))
            .copied()
            .unwrap_or(default)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str, i64)> {
        self.values
            .iter()
            .map(|((f, o), p)| (f.as_str(), o.as_str(), *p))
    }
}

impl FromIterator<((String, String), i64)> for WeightOverrides {
    fn from_iter<T: IntoIterator<Item = ((String, String), i64)>>(iter: T) -> Self {
        Self {
            values: iter.into_iter().collect(),
        }
    }
}

// ── The shipped schema ──────────────────────────────────────────────────────

/// The current season's schema, embedded in the binary at compile time.
///
/// Embedded rather than read from disk for three reasons: the Pi deploy is a
/// single binary with no data directory to resolve (REBUILD_SPEC.md 10), a
/// malformed schema becomes a failing test rather than an event-day surprise,
/// and the deployed build can never drift from the schema it was tested against
/// -- which is what makes the version handshake in phase 3 (S11) meaningful.
///
/// Editing it is a rebuild. At Kickoff that is the right trade.
pub const CURRENT_SEASON_JSON: &str = include_str!("../seasons/2026.json");

/// Parse the embedded schema.
///
/// Call once at startup and keep the result; parsing is cheap but not free.
pub fn current_season() -> Result<SeasonSchema> {
    SeasonSchema::parse(CURRENT_SEASON_JSON)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A schema exercising every field kind. Small on purpose -- the real
    /// 2026 schema lives in seasons/2026.json and is checked by its own test.
    fn schema() -> SeasonSchema {
        SeasonSchema::parse(
            r#"{
              "season": 2026,
              "version": 1,
              "name": "Test Season",
              "sections": [
                {
                  "key": "teleop",
                  "label": "Teleop",
                  "fields": [
                    {
                      "key": "defense",
                      "label": "Defense",
                      "required": true,
                      "type": "select",
                      "options": [
                        {"key": "high", "label": "High", "points": 5},
                        {"key": "low",  "label": "Low",  "points": 1}
                      ]
                    },
                    {
                      "key": "pieces",
                      "label": "Pieces scored",
                      "type": "counter",
                      "min": 0, "max": 30, "points_each": 2
                    },
                    {"key": "climbed", "label": "Climbed", "type": "toggle", "points": 4},
                    {"key": "notes", "label": "Notes", "type": "text", "max_len": 10}
                  ]
                }
              ]
            }"#,
        )
        .expect("valid schema")
    }

    fn payload(pairs: &[(&str, Value)]) -> Payload {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn parses_every_field_kind() {
        let s = schema();
        assert_eq!(s.season, 2026);
        assert_eq!(s.fields().count(), 4);
        assert!(matches!(
            s.field("defense").unwrap().kind,
            FieldKind::Select { .. }
        ));
        assert!(matches!(
            s.field("pieces").unwrap().kind,
            FieldKind::Counter { max: 30, .. }
        ));
    }

    #[test]
    fn text_fields_are_never_scored() {
        assert!(!schema().field("notes").unwrap().is_scored());
        assert!(schema().field("defense").unwrap().is_scored());
    }

    // ── Schema validation ───────────────────────────────────────────────────

    #[test]
    fn field_keys_must_be_unique_across_the_whole_schema() {
        // Two sections sharing a key would silently overwrite in the flat payload.
        let json = r#"{
          "season": 2026, "version": 1,
          "sections": [
            {"key": "a", "label": "A", "fields": [
              {"key": "dupe", "label": "One", "type": "toggle"}]},
            {"key": "b", "label": "B", "fields": [
              {"key": "dupe", "label": "Two", "type": "toggle"}]}
          ]}"#;
        assert!(matches!(
            SeasonSchema::parse(json),
            Err(DomainError::Invalid {
                field: "duplicate field key",
                ..
            })
        ));
    }

    #[test]
    fn empty_select_is_rejected() {
        let json = r#"{"season":2026,"version":1,"sections":[
          {"key":"a","label":"A","fields":[
            {"key":"f","label":"F","type":"select","options":[]}]}]}"#;
        assert!(SeasonSchema::parse(json).is_err());
    }

    #[test]
    fn inverted_counter_range_is_rejected() {
        let json = r#"{"season":2026,"version":1,"sections":[
          {"key":"a","label":"A","fields":[
            {"key":"f","label":"F","type":"counter","min":10,"max":2}]}]}"#;
        assert!(SeasonSchema::parse(json).is_err());
    }

    #[test]
    fn schema_without_fields_is_rejected() {
        let json = r#"{"season":2026,"version":1,"sections":[
          {"key":"a","label":"A","fields":[]}]}"#;
        assert!(SeasonSchema::parse(json).is_err());
    }

    #[test]
    fn version_must_be_positive() {
        let json = r#"{"season":2026,"version":0,"sections":[
          {"key":"a","label":"A","fields":[
            {"key":"f","label":"F","type":"toggle"}]}]}"#;
        assert!(SeasonSchema::parse(json).is_err());
    }

    #[test]
    fn schema_round_trips_through_json() {
        let original = schema();
        let json = serde_json::to_string(&original).expect("serialize");
        assert_eq!(SeasonSchema::parse(&json).expect("reparse"), original);
    }

    // ── Payload validation ──────────────────────────────────────────────────

    #[test]
    fn accepts_a_complete_valid_payload() {
        let p = payload(&[
            ("defense", Value::Text("high".into())),
            ("pieces", Value::Count(7)),
            ("climbed", Value::Flag(true)),
            ("notes", Value::Text("fast".into())),
        ]);
        assert!(schema().validate_payload(&p).is_ok());
    }

    #[test]
    fn optional_fields_may_be_absent() {
        let p = payload(&[("defense", Value::Text("low".into()))]);
        assert!(schema().validate_payload(&p).is_ok());
    }

    #[test]
    fn required_field_missing_is_rejected() {
        let p = payload(&[("climbed", Value::Flag(false))]);
        assert!(matches!(
            schema().validate_payload(&p),
            Err(DomainError::Missing { .. })
        ));
    }

    #[test]
    fn unknown_keys_are_rejected_rather_than_dropped() {
        // A client on a different schema version must be told, not silently
        // have a scout's work discarded.
        let p = payload(&[
            ("defense", Value::Text("high".into())),
            ("hang_level", Value::Text("l3".into())),
        ]);
        assert!(matches!(
            schema().validate_payload(&p),
            Err(DomainError::Invalid {
                field: "unknown field",
                ..
            })
        ));
    }

    #[test]
    fn select_rejects_an_option_that_is_not_offered() {
        let p = payload(&[("defense", Value::Text("medium".into()))]);
        assert!(schema().validate_payload(&p).is_err());
    }

    #[test]
    fn counter_enforces_both_bounds() {
        let s = schema();
        assert!(
            s.validate_payload(&payload(&[
                ("defense", Value::Text("high".into())),
                ("pieces", Value::Count(0))
            ]))
            .is_ok()
        );
        assert!(
            s.validate_payload(&payload(&[
                ("defense", Value::Text("high".into())),
                ("pieces", Value::Count(30))
            ]))
            .is_ok()
        );
        assert!(
            s.validate_payload(&payload(&[
                ("defense", Value::Text("high".into())),
                ("pieces", Value::Count(31))
            ]))
            .is_err()
        );
        assert!(
            s.validate_payload(&payload(&[
                ("defense", Value::Text("high".into())),
                ("pieces", Value::Count(-1))
            ]))
            .is_err()
        );
    }

    #[test]
    fn text_limit_counts_characters_not_bytes() {
        // Ten emoji are ten characters and forty bytes. The limit is ten.
        let s = schema();
        let ten_emoji = "🤖".repeat(10);
        assert!(
            s.validate_payload(&payload(&[
                ("defense", Value::Text("high".into())),
                ("notes", Value::Text(ten_emoji))
            ]))
            .is_ok()
        );
        assert!(
            s.validate_payload(&payload(&[
                ("defense", Value::Text("high".into())),
                ("notes", Value::Text("🤖".repeat(11)))
            ]))
            .is_err()
        );
    }

    #[test]
    fn wrong_value_shape_for_a_field_is_rejected() {
        let p = payload(&[("defense", Value::Count(3))]);
        assert!(schema().validate_payload(&p).is_err());
    }

    // ── Scoring ─────────────────────────────────────────────────────────────

    #[test]
    fn scores_each_field_kind() {
        let p = payload(&[
            ("defense", Value::Text("high".into())), // 5
            ("pieces", Value::Count(3)),             // 3 * 2 = 6
            ("climbed", Value::Flag(true)),          // 4
            ("notes", Value::Text("ignored".into())),
        ]);
        assert_eq!(schema().score(&p, &WeightOverrides::new()), 15);
    }

    #[test]
    fn a_false_toggle_scores_nothing() {
        let p = payload(&[
            ("defense", Value::Text("low".into())),
            ("climbed", Value::Flag(false)),
        ]);
        assert_eq!(schema().score(&p, &WeightOverrides::new()), 1);
    }

    #[test]
    fn overrides_replace_schema_points() {
        let mut o = WeightOverrides::new();
        o.set("defense", "high", 100);
        o.set("pieces", COUNTER_UNIT, 10);
        o.set("climbed", TOGGLE_ON, 1);

        let p = payload(&[
            ("defense", Value::Text("high".into())),
            ("pieces", Value::Count(2)),
            ("climbed", Value::Flag(true)),
        ]);
        assert_eq!(schema().score(&p, &o), 100 + 20 + 1);
    }

    #[test]
    fn stale_overrides_from_a_past_season_are_ignored_not_fatal() {
        // A leftover row in scouting_point_weights must never break scoring.
        let mut o = WeightOverrides::new();
        o.set("hang_level", "l3", 99);
        let p = payload(&[("defense", Value::Text("high".into()))]);
        assert_eq!(schema().score(&p, &o), 5);
    }

    #[test]
    fn an_empty_payload_scores_zero() {
        assert_eq!(schema().score(&Payload::new(), &WeightOverrides::new()), 0);
    }

    #[test]
    fn counter_scoring_saturates_rather_than_overflowing() {
        let json = r#"{"season":2026,"version":1,"sections":[
          {"key":"a","label":"A","fields":[
            {"key":"n","label":"N","type":"counter","min":0,"max":9223372036854775807,
             "points_each":9223372036854775807}]}]}"#;
        let s = SeasonSchema::parse(json).expect("valid");
        let p = payload(&[("n", Value::Count(i64::MAX))]);
        assert_eq!(s.score(&p, &WeightOverrides::new()), i64::MAX);
    }

    #[test]
    fn payload_json_is_deterministic() {
        // Sync (phase 3) compares payloads; key order must not vary run to run.
        let p = payload(&[
            ("climbed", Value::Flag(true)),
            ("defense", Value::Text("high".into())),
            ("pieces", Value::Count(1)),
        ]);
        let once = payload_to_json(&p);
        let twice = payload_to_json(&parse_payload(&once).expect("reparse"));
        assert_eq!(once, twice);
        assert!(once.starts_with(r#"{"climbed""#), "keys should be sorted");
    }

    // ── The shipped 2026 schema ─────────────────────────────────────────────

    #[test]
    fn shipped_season_schema_is_valid() {
        // seasons/2026.json is the 2026 game, Rebuilt.
        // This is the test that turns a Kickoff-day typo into a red build.
        let s = current_season().expect("seasons/2026.json must parse and validate");
        assert_eq!(s.season, 2026);
        assert_eq!(s.name, "Rebuilt");
        assert!(s.fields().count() >= 5);
    }

    #[test]
    fn shipped_schema_scores_a_realistic_payload() {
        let s = current_season().expect("schema");
        let p = payload(&[
            ("starting_position", Value::Text("center".into())),
            ("auto_scored", Value::Count(2)),        //  2 * 4  =   8
            ("teleop_scored", Value::Count(9)),      //  9 * 2  =  18
            ("endgame", Value::Text("full".into())), //             6
            ("defense_rating", Value::Text("some".into())), //       2
            ("driver_skill", Value::Text("strong".into())), //       4
            ("speed", Value::Text("fast".into())),   //             4
            ("broke_down", Value::Flag(false)),      //             0
            ("penalties", Value::Count(1)),          //  1 * -2 =  -2
            ("notes", Value::Text("tippy".into())),  //  not scored
        ]);
        s.validate_payload(&p).expect("payload should validate");
        assert_eq!(s.score(&p, &WeightOverrides::new()), 40);
    }

    #[test]
    fn shipped_schema_penalises_unreliability() {
        // A robot that scores well but breaks down should rank below one that
        // scores the same and finishes. Reliability is what alliance picks turn on.
        let s = current_season().expect("schema");
        let base = [
            ("starting_position", Value::Text("left".into())),
            ("teleop_scored", Value::Count(10)),
        ];

        let mut reliable = payload(&base);
        reliable.insert("broke_down".into(), Value::Flag(false));

        let mut fragile = payload(&base);
        fragile.insert("broke_down".into(), Value::Flag(true));

        let w = WeightOverrides::new();
        assert!(s.score(&fragile, &w) < s.score(&reliable, &w));
    }

    #[test]
    fn shipped_schema_ignores_the_comment_block() {
        // The file carries a "_comment" array of Kickoff instructions; unknown
        // top-level keys must not break parsing.
        assert!(CURRENT_SEASON_JSON.contains("_comment"));
        assert!(current_season().is_ok());
    }
}
