//! The lead scout's review of observations (L8-L10).
//!
//! Every observation arrives pending. A lead scout approves it, or declines it
//! with a reason the scout is shown. Neither destroys anything: the retired app
//! deleted a declined submission outright, with no record and no word to the
//! scout (REBUILD_SPEC.md 12.5). Here the row stays, with who declined it, when,
//! and why, and the scout can record the robot again.

use serde::{Deserialize, Serialize};

use crate::error::{DomainError, Result};
use crate::season::{FieldKind, Payload, SeasonSchema, Value};

/// Longest decline reason kept. A sentence or two for a teenager on a phone.
pub const REASON_MAX: usize = 500;

/// Where an observation stands. Stored as its lowercase name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReviewState {
    Pending,
    Approved,
    Declined,
}

impl ReviewState {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "pending" => Some(Self::Pending),
            "approved" => Some(Self::Approved),
            "declined" => Some(Self::Declined),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Declined => "declined",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Pending => "Waiting for review",
            Self::Approved => "Approved",
            Self::Declined => "Declined",
        }
    }
}

/// A lead scout's verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Approve,
    /// With the reason the scout will be shown. Build it with
    /// [`Decision::decline`], which checks the reason.
    Decline(String),
}

impl Decision {
    /// A decline needs a reason: a scout told only "declined" cannot do
    /// better next time, which was the whole complaint about the retired
    /// app's silent delete.
    pub fn decline(reason: &str) -> Result<Self> {
        let reason = reason.trim();
        if reason.is_empty() {
            return Err(DomainError::Invalid {
                field: "reason",
                value: "Say why, so the scout can fix it.".into(),
            });
        }
        if reason.chars().count() > REASON_MAX {
            return Err(DomainError::Invalid {
                field: "reason",
                value: format!("Keep the reason under {REASON_MAX} characters."),
            });
        }
        Ok(Self::Decline(reason.to_string()))
    }

    pub fn state(&self) -> ReviewState {
        match self {
            Self::Approve => ReviewState::Approved,
            Self::Decline(_) => ReviewState::Declined,
        }
    }
}

/// One answer, ready to read: the field's label and what the scout said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    pub label: String,
    /// The option's label, the count, "Yes" or "No", or the text. `"—"` when
    /// the scout left it out.
    pub value: String,
    /// A free-text field, so the page can give it room.
    pub is_text: bool,
}

/// A section's worth of answers, in form order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnswerGroup {
    pub label: String,
    pub answers: Vec<Answer>,
}

/// A payload as the lead scout reads it: every field of `schema` in form order,
/// then anything the payload holds that the schema does not declare -- from an
/// older form version -- so nothing a scout recorded is hidden.
pub fn answers(schema: &SeasonSchema, payload: &Payload) -> Vec<AnswerGroup> {
    let mut groups: Vec<AnswerGroup> = schema
        .sections
        .iter()
        .map(|section| AnswerGroup {
            label: section.label.clone(),
            answers: section
                .fields
                .iter()
                .map(|field| {
                    let value = match (&field.kind, payload.get(&field.key)) {
                        (_, None) => "—".to_string(),
                        (FieldKind::Select { options }, Some(Value::Text(key))) => options
                            .iter()
                            .find(|o| o.key == *key)
                            .map(|o| o.label.clone())
                            .unwrap_or_else(|| key.clone()),
                        (_, Some(value)) => show(value),
                    };
                    Answer {
                        label: field.label.clone(),
                        value,
                        is_text: matches!(field.kind, FieldKind::Text { .. }),
                    }
                })
                .collect(),
        })
        .collect();

    let extra: Vec<Answer> = payload
        .iter()
        .filter(|(key, _)| schema.field(key).is_none())
        .map(|(key, value)| Answer {
            label: key.clone(),
            value: show(value),
            is_text: false,
        })
        .collect();
    if !extra.is_empty() {
        groups.push(AnswerGroup {
            label: "Not on the current form".into(),
            answers: extra,
        });
    }
    groups
}

fn show(value: &Value) -> String {
    match value {
        Value::Text(text) => text.clone(),
        Value::Count(n) => n.to_string(),
        Value::Flag(true) => "Yes".into(),
        Value::Flag(false) => "No".into(),
    }
}

/// Whether every free-text field is blank: the retired queue's "Missing notes"
/// flag, generalised so it survives next season's schema.
///
/// A schema with no text fields has nothing to miss.
pub fn missing_notes(schema: &SeasonSchema, payload: &Payload) -> bool {
    let mut text_fields = schema
        .fields()
        .filter(|f| matches!(f.kind, FieldKind::Text { .. }))
        .peekable();
    text_fields.peek().is_some()
        && text_fields.all(|f| {
            payload
                .get(&f.key)
                .and_then(Value::as_text)
                .is_none_or(|t| t.trim().is_empty())
        })
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

    #[test]
    fn a_decline_needs_a_reason_and_keeps_it_short() {
        assert_eq!(
            Decision::decline("  swapped auto and teleop  ").unwrap(),
            Decision::Decline("swapped auto and teleop".into())
        );
        assert!(Decision::decline("   ").is_err());
        assert!(Decision::decline(&"x".repeat(REASON_MAX + 1)).is_err());
        assert!(
            Decision::decline(&"é".repeat(REASON_MAX)).is_ok(),
            "characters, not bytes"
        );
        assert_eq!(Decision::Approve.state(), ReviewState::Approved);
    }

    #[test]
    fn review_states_round_trip_through_storage() {
        for state in [
            ReviewState::Pending,
            ReviewState::Approved,
            ReviewState::Declined,
        ] {
            assert_eq!(ReviewState::parse(state.as_str()), Some(state));
        }
        assert_eq!(ReviewState::parse("deleted"), None);
    }

    #[test]
    fn answers_read_as_the_form_did() {
        let schema = current_season().unwrap();
        let groups = answers(
            &schema,
            &payload(&[
                ("starting_position", Value::Text("center".into())),
                ("teleop_scored", Value::Count(9)),
                ("no_show", Value::Flag(false)),
                ("notes", Value::Text("tippy".into())),
            ]),
        );
        let all: Vec<(&str, &str)> = groups
            .iter()
            .flat_map(|g| &g.answers)
            .map(|a| (a.label.as_str(), a.value.as_str()))
            .collect();
        assert!(
            all.contains(&("Starting position", "Center")),
            "the option's label"
        );
        assert!(all.contains(&("Pieces scored in teleop", "9")));
        assert!(all.contains(&("Did not show / disabled all match", "No")));
        assert!(all.contains(&("Pieces scored in auto", "—")), "left out");
        assert_eq!(groups.len(), schema.sections.len());
        assert!(groups.iter().flat_map(|g| &g.answers).any(|a| a.is_text));
    }

    #[test]
    fn answers_from_an_older_form_are_still_shown() {
        let schema = current_season().unwrap();
        let groups = answers(
            &schema,
            &payload(&[("hang_level", Value::Text("l3".into()))]),
        );
        let last = groups.last().unwrap();
        assert_eq!(last.label, "Not on the current form");
        assert_eq!(last.answers[0].value, "l3");
    }

    #[test]
    fn notes_are_missing_when_every_text_field_is_blank() {
        let schema = current_season().unwrap();
        assert!(missing_notes(&schema, &payload(&[])));
        assert!(missing_notes(
            &schema,
            &payload(&[("notes", Value::Text("   ".into()))])
        ));
        assert!(!missing_notes(
            &schema,
            &payload(&[("notes", Value::Text("fast".into()))])
        ));
    }
}
