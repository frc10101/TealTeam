//! Reading a submitted scouting form into a [`Payload`] (U4).
//!
//! The form is rendered from the season schema, so this is the other half: an
//! HTML form post is a flat list of strings, and this turns it back into typed
//! answers -- or into one message per field saying what is wrong, so the page
//! can re-render with everything the scout typed still in place.
//!
//! It lives here rather than in the server so the same reading runs in a
//! service worker once submission works offline (C5).
//!
//! # What an untouched input means
//!
//! A scout who watched a match and left "Broke down" unticked is saying it did
//! not break down, so a missing checkbox is recorded as `false`. Counters are
//! rendered starting at their minimum, so an untouched one arrives as a real
//! zero. Only a select nobody chose, a counter somebody cleared, or blank notes
//! are left out of the payload -- that is "not recorded", which is different
//! from zero.

use std::collections::BTreeMap;

use crate::season::{Field, FieldKind, Payload, SeasonSchema, Value};

/// Prefix on every schema field's input name, so a season can name a field
/// `match` or `team` without colliding with the form's own inputs.
pub const FIELD_PREFIX: &str = "f.";

/// The input name for a schema field: `f.auto_scored`.
pub fn input_name(field_key: &str) -> String {
    format!("{FIELD_PREFIX}{field_key}")
}

/// A form's answers as the browser sent them, keyed by field key with the
/// prefix removed. Inputs without the prefix are not answers and are dropped.
///
/// Kept as raw strings, not parsed, because a re-rendered form must show what
/// the scout typed -- including the "7o" that failed to parse.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RawAnswers(BTreeMap<String, String>);

impl RawAnswers {
    /// Collect answers from form pairs. A repeated name keeps its last value.
    pub fn from_pairs(pairs: &[(String, String)]) -> Self {
        Self(
            pairs
                .iter()
                .filter_map(|(name, value)| {
                    let key = name.strip_prefix(FIELD_PREFIX)?;
                    Some((key.to_string(), value.clone()))
                })
                .collect(),
        )
    }

    /// A saved payload as the form would post it, to show it in the form
    /// again (C10b): only `schema`'s fields, a ticked toggle as `on`, and a
    /// counter left out as blank rather than its minimum, so it stays left
    /// out.
    pub fn from_payload(schema: &SeasonSchema, payload: &Payload) -> Self {
        Self(
            schema
                .fields()
                .filter_map(|field| {
                    let raw = match (payload.get(&field.key), &field.kind) {
                        (None, FieldKind::Counter { .. }) => String::new(),
                        (None, _) | (Some(Value::Flag(false)), _) => return None,
                        (Some(Value::Flag(true)), _) => "on".into(),
                        (Some(Value::Count(n)), _) => n.to_string(),
                        (Some(Value::Text(text)), _) => text.clone(),
                    };
                    Some((field.key.clone(), raw))
                })
                .collect(),
        )
    }

    /// Answer `field_key` with `value`, replacing what was posted.
    pub fn set(&mut self, field_key: &str, value: Option<&str>) {
        match value {
            Some(value) => self.0.insert(field_key.to_string(), value.to_string()),
            None => self.0.remove(field_key),
        };
    }

    /// Whether two sets of answers say the same thing: equal once each is
    /// trimmed, line breaks are `\n`, and blanks are dropped -- the
    /// differences between a payload shown in a form and that form posted
    /// back untouched.
    pub fn same_as(&self, other: &RawAnswers) -> bool {
        fn said(raw: &RawAnswers) -> BTreeMap<&str, String> {
            raw.0
                .iter()
                .map(|(key, value)| (key.as_str(), value.replace("\r\n", "\n").trim().to_string()))
                .filter(|(_, value)| !value.is_empty())
                .collect()
        }
        said(self) == said(other)
    }

    pub fn get(&self, field_key: &str) -> Option<&str> {
        self.0.get(field_key).map(String::as_str)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// What is wrong with a submitted form.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FormErrors {
    /// By field key, one message each, written for the scout.
    pub fields: BTreeMap<String, String>,
    /// Problems that belong to no single field.
    pub form: Vec<String>,
}

impl FormErrors {
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty() && self.form.is_empty()
    }

    pub fn field(&self, field_key: &str) -> Option<&str> {
        self.fields.get(field_key).map(String::as_str)
    }
}

/// Shown when a post carries answers for fields this schema does not have.
pub const STALE_FORM: &str = "This page was loaded with a different version of the form. \
                              Check your answers and save again.";

/// Read a form post against a schema.
///
/// A successful result always passes [`SeasonSchema::validate_payload`]. On
/// failure every field's problem is reported at once, so the scout fixes them
/// in one pass rather than discovering them one Save at a time.
pub fn read_answers(schema: &SeasonSchema, raw: &RawAnswers) -> Result<Payload, FormErrors> {
    let mut payload = Payload::new();
    let mut errors = FormErrors::default();

    for field in schema.fields() {
        match read_field(field, raw.get(&field.key)) {
            Ok(Some(value)) => {
                payload.insert(field.key.clone(), value);
            }
            Ok(None) if field.required => {
                errors
                    .fields
                    .insert(field.key.clone(), missing_message(field).into());
            }
            Ok(None) => {}
            Err(message) => {
                errors.fields.insert(field.key.clone(), message);
            }
        }
    }

    // Answers for fields that do not exist mean the page came from another
    // schema version. Dropping them silently would lose work without a word.
    if raw.0.keys().any(|key| schema.field(key).is_none()) {
        errors.form.push(STALE_FORM.into());
    }

    if !errors.is_empty() {
        return Err(errors);
    }

    // Belt and braces: the reading above should make this unreachable, and if
    // it ever is not, the scout gets a message rather than a bad row.
    if let Err(e) = schema.validate_payload(&payload) {
        errors
            .form
            .push(format!("These answers could not be checked: {e}"));
        return Err(errors);
    }
    Ok(payload)
}

/// Answers that came by another road than the form's own post -- a tablet's
/// unsent form, carried by QR (S13) -- typed as far as they go, for the
/// push's rules to judge. On `schema`, the form it was typed into, a field
/// reads as the form reads it, and one that does not read is kept as its
/// text, so a refusal shows a lead what to correct (C10b). With no schema
/// (another version of the form), and for a name the schema does not have,
/// the answer is guessed: `on` a ticked box, a whole number a count, the
/// rest text.
pub fn read_answers_as_given(schema: Option<&SeasonSchema>, raw: &RawAnswers) -> Payload {
    if let Some(schema) = schema
        && let Ok(payload) = read_answers(schema, raw)
    {
        return payload;
    }
    let guess = |text: &str| {
        if text.trim().eq_ignore_ascii_case("on") {
            Value::Flag(true)
        } else if let Ok(n) = text.trim().parse::<i64>() {
            Value::Count(n)
        } else {
            Value::Text(text.trim().to_string())
        }
    };
    let mut payload = Payload::new();
    for (key, text) in &raw.0 {
        let field = schema.and_then(|s| s.field(key));
        let value = match field.map(|f| read_field(f, Some(text))) {
            Some(Ok(value)) => value,
            Some(Err(_)) => Some(Value::Text(text.clone())),
            None => Some(guess(text)).filter(|v| v != &Value::Text(String::new())),
        };
        if let Some(value) = value {
            payload.insert(key.clone(), value);
        }
    }
    // A box left unticked is not posted, and on this form means no.
    if let Some(schema) = schema {
        for field in schema.fields() {
            if matches!(field.kind, FieldKind::Toggle { .. }) {
                payload
                    .entry(field.key.clone())
                    .or_insert(Value::Flag(false));
            }
        }
    }
    payload
}

/// One field's answer: `Ok(None)` when nothing was recorded.
fn read_field(field: &Field, raw: Option<&str>) -> Result<Option<Value>, String> {
    let given = raw.map(str::trim).filter(|s| !s.is_empty());

    match &field.kind {
        FieldKind::Toggle { .. } => Ok(Some(Value::Flag(given.is_some_and(is_on)))),

        FieldKind::Select { options } => match given {
            None => Ok(None),
            Some(chosen) if options.iter().any(|o| o.key == chosen) => {
                Ok(Some(Value::Text(chosen.to_string())))
            }
            Some(_) => Err("Choose one of the options.".into()),
        },

        FieldKind::Counter { min, max, .. } => match given {
            None => Ok(None),
            Some(text) => match text.parse::<i64>() {
                Ok(n) if (*min..=*max).contains(&n) => Ok(Some(Value::Count(n))),
                Ok(_) => Err(format!("Must be between {min} and {max}.")),
                Err(_) => Err("Enter a whole number.".into()),
            },
        },

        FieldKind::Text { max_len } => {
            // Browsers submit a textarea's line breaks as CRLF but count them
            // as one character against `maxlength`. Normalise, so the limit
            // here is the limit the scout saw.
            let text = raw.unwrap_or_default().replace("\r\n", "\n");
            let text = text.trim();
            let length = text.chars().count();
            if text.is_empty() {
                Ok(None)
            } else if length > *max_len {
                Err(format!("{length} characters; the limit is {max_len}."))
            } else {
                Ok(Some(Value::Text(text.to_string())))
            }
        }
    }
}

/// Whether a checkbox's submitted value means ticked. Browsers send `on`
/// unless told otherwise; the rest are for scripts and the phase 3 client.
pub fn is_on(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "on" | "true" | "1" | "yes"
    )
}

fn missing_message(field: &Field) -> &'static str {
    match field.kind {
        FieldKind::Select { .. } => "Choose one.",
        FieldKind::Counter { .. } => "Enter a number.",
        FieldKind::Toggle { .. } | FieldKind::Text { .. } => "Required.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::season::current_season;

    fn schema() -> SeasonSchema {
        SeasonSchema::parse(
            r#"{
              "season": 2026, "version": 1,
              "sections": [{"key": "s", "label": "S", "fields": [
                {"key": "defense", "label": "Defense", "required": true, "type": "select",
                 "options": [{"key": "high", "label": "High"}, {"key": "low", "label": "Low"}]},
                {"key": "pieces", "label": "Pieces", "type": "counter", "min": 0, "max": 30},
                {"key": "climbed", "label": "Climbed", "type": "toggle"},
                {"key": "notes", "label": "Notes", "type": "text", "max_len": 10}
              ]}]
            }"#,
        )
        .expect("valid schema")
    }

    fn raw(pairs: &[(&str, &str)]) -> RawAnswers {
        let pairs: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        RawAnswers::from_pairs(&pairs)
    }

    #[test]
    fn a_complete_form_reads_into_typed_answers() {
        let payload = read_answers(
            &schema(),
            &raw(&[
                ("f.defense", "high"),
                ("f.pieces", " 7 "),
                ("f.climbed", "on"),
                ("f.notes", "  fast  "),
            ]),
        )
        .expect("valid");

        assert_eq!(payload["defense"], Value::Text("high".into()));
        assert_eq!(payload["pieces"], Value::Count(7));
        assert_eq!(payload["climbed"], Value::Flag(true));
        assert_eq!(payload["notes"], Value::Text("fast".into()), "trimmed");
    }

    #[test]
    fn inputs_without_the_prefix_are_not_answers() {
        let answers = raw(&[("match", "2026mabil_qm1"), ("f.defense", "low")]);
        assert_eq!(answers.get("defense"), Some("low"));
        assert_eq!(answers.get("match"), None);
        assert!(read_answers(&schema(), &answers).is_ok());
    }

    #[test]
    fn an_unticked_box_is_a_recorded_no() {
        // The scout watched the match; not ticking "climbed" means it did not.
        let payload = read_answers(&schema(), &raw(&[("f.defense", "low")])).expect("valid");
        assert_eq!(payload["climbed"], Value::Flag(false));
    }

    #[test]
    fn blank_optional_answers_are_left_out_rather_than_zeroed() {
        let payload = read_answers(
            &schema(),
            &raw(&[("f.defense", "low"), ("f.pieces", ""), ("f.notes", "   ")]),
        )
        .expect("valid");
        assert!(!payload.contains_key("pieces"), "cleared is not zero");
        assert!(!payload.contains_key("notes"));
    }

    #[test]
    fn every_problem_is_reported_at_once() {
        let errors = read_answers(
            &schema(),
            &raw(&[("f.pieces", "31"), ("f.notes", "far too long for ten")]),
        )
        .expect_err("invalid");

        assert_eq!(errors.field("defense"), Some("Choose one."));
        assert_eq!(errors.field("pieces"), Some("Must be between 0 and 30."));
        assert_eq!(
            errors.field("notes"),
            Some("20 characters; the limit is 10.")
        );
        assert!(errors.form.is_empty());
    }

    #[test]
    fn a_counter_that_is_not_a_whole_number_says_so() {
        for bad in ["7o", "2.5", "seven"] {
            let errors = read_answers(&schema(), &raw(&[("f.defense", "low"), ("f.pieces", bad)]))
                .expect_err(bad);
            assert_eq!(
                errors.field("pieces"),
                Some("Enter a whole number."),
                "{bad}"
            );
        }
    }

    #[test]
    fn a_select_only_accepts_its_own_options() {
        let errors = read_answers(&schema(), &raw(&[("f.defense", "medium")])).expect_err("bad");
        assert_eq!(errors.field("defense"), Some("Choose one of the options."));
    }

    #[test]
    fn line_breaks_count_once_toward_the_text_limit() {
        // Ten characters as the scout typed them; eleven bytes of CRLF on the wire.
        let payload = read_answers(
            &schema(),
            &raw(&[("f.defense", "low"), ("f.notes", "fast\r\nslow!")]),
        )
        .expect("within the limit once normalised");
        assert_eq!(payload["notes"], Value::Text("fast\nslow!".into()));
    }

    #[test]
    fn answers_for_unknown_fields_are_reported_not_dropped() {
        let errors = read_answers(
            &schema(),
            &raw(&[("f.defense", "low"), ("f.hang_level", "l3")]),
        )
        .expect_err("stale form");
        assert_eq!(errors.form, vec![STALE_FORM.to_string()]);
    }

    #[test]
    fn a_repeated_input_keeps_its_last_value() {
        assert_eq!(
            raw(&[("f.defense", "high"), ("f.defense", "low")]).get("defense"),
            Some("low")
        );
    }

    #[test]
    fn checkbox_values_from_browsers_and_scripts_all_count() {
        for on in ["on", "true", "1", "YES"] {
            let payload = read_answers(&schema(), &raw(&[("f.defense", "low"), ("f.climbed", on)]))
                .expect("valid");
            assert_eq!(payload["climbed"], Value::Flag(true), "{on}");
        }
        let payload = read_answers(
            &schema(),
            &raw(&[("f.defense", "low"), ("f.climbed", "false")]),
        )
        .expect("valid");
        assert_eq!(payload["climbed"], Value::Flag(false));
    }

    #[test]
    fn the_shipped_schema_reads_a_form_a_scout_would_submit() {
        let season = current_season().expect("schema");
        let payload = read_answers(
            &season,
            &raw(&[
                ("f.starting_position", "center"),
                ("f.auto_scored", "2"),
                ("f.teleop_scored", "9"),
                ("f.endgame", "full"),
                ("f.penalties", "0"),
                ("f.notes", "tippy on the ramp"),
            ]),
        )
        .expect("valid");

        assert!(season.validate_payload(&payload).is_ok());
        assert_eq!(payload["broke_down"], Value::Flag(false));
        assert_eq!(payload["no_show"], Value::Flag(false));
    }

    #[test]
    fn a_saved_payload_shown_in_the_form_and_posted_back_untouched_is_the_same() {
        let saved: Payload = [
            ("defense".to_string(), Value::Text("high".into())),
            ("notes".to_string(), Value::Text("fast\nslow".into())),
            ("climbed".to_string(), Value::Flag(false)),
            ("hang_level".to_string(), Value::Text("l3".into())),
        ]
        .into();
        let shown = RawAnswers::from_payload(&schema(), &saved);
        assert_eq!(shown.get("pieces"), Some(""), "left out stays left out");
        assert_eq!(shown.get("climbed"), None, "an unticked box");
        assert_eq!(shown.get("hang_level"), None, "not this form's");
        assert_eq!(
            read_answers(&schema(), &shown).unwrap()["climbed"],
            Value::Flag(false)
        );

        // What a browser sends back: CRLF, every input, no unticked box.
        let posted = raw(&[
            ("f.defense", "high"),
            ("f.pieces", ""),
            ("f.notes", "fast\r\nslow"),
        ]);
        assert!(posted.same_as(&shown));
        let mut changed = posted.clone();
        changed.set("pieces", Some("4"));
        assert!(!changed.same_as(&shown));
        changed.set("pieces", None);
        assert!(changed.same_as(&shown));
    }

    #[test]
    fn answers_from_another_road_are_typed_as_far_as_they_go() {
        // Every answer fits: just as the form reads them.
        let good = raw(&[("f.defense", "low"), ("f.pieces", "4")]);
        assert_eq!(
            read_answers_as_given(Some(&schema()), &good),
            read_answers(&schema(), &good).unwrap()
        );

        // One does not: it is kept as typed, for a lead to correct, and the
        // rest still read.
        let bad = raw(&[("f.defense", "low"), ("f.pieces", "7o"), ("f.notes", " ")]);
        let payload = read_answers_as_given(Some(&schema()), &bad);
        assert_eq!(payload["pieces"], Value::Text("7o".into()));
        assert_eq!(payload["defense"], Value::Text("low".into()));
        assert_eq!(payload["climbed"], Value::Flag(false));
        assert!(!payload.contains_key("notes"));
        assert!(schema().validate_payload(&payload).is_err());

        // Another version's form: guessed.
        let old = raw(&[
            ("f.climbed", "on"),
            ("f.cones", "3"),
            ("f.who", "fast"),
            ("f.x", ""),
        ]);
        let payload = read_answers_as_given(None, &old);
        assert_eq!(payload["climbed"], Value::Flag(true));
        assert_eq!(payload["cones"], Value::Count(3));
        assert_eq!(payload["who"], Value::Text("fast".into()));
        assert!(!payload.contains_key("x"));
    }

    #[test]
    fn input_names_carry_the_prefix() {
        assert_eq!(input_name("auto_scored"), "f.auto_scored");
    }
}
