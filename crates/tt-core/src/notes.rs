//! Who may read a scout's notes (U13).
//!
//! Notes are the one part of an observation written in prose -- "tippy on the
//! ramp", "their driver panics under defense" -- and several teams can share
//! one server. The rule is that notes are read only by the team whose scout
//! wrote them: the observation's `submitting_team`, fixed when it was saved
//! (L7). A viewer with no team reads none, and nor does anyone read notes
//! saved with no team; there is no team for them to belong to.
//!
//! The numbers are shared. Only the prose is held back.

use crate::season::{FieldKind, Payload, SeasonSchema, Value};

/// Whether a viewer sees the notes on an observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notes {
    Shown,
    Hidden,
}

impl Notes {
    /// The rule: shown only when the viewer's team is the team the notes were
    /// written for.
    pub fn for_viewer(viewer_team: Option<i32>, submitting_team: Option<i32>) -> Self {
        match (viewer_team, submitting_team) {
            (Some(viewer), Some(owner)) if viewer == owner => Self::Shown,
            _ => Self::Hidden,
        }
    }

    pub fn shown(self) -> bool {
        self == Self::Shown
    }
}

/// The notes written on one observation: each of `schema`'s text fields that
/// was filled in, in form order, as `(label, text)`. Empty when none were.
/// Whether the viewer may read them is [`Notes::for_viewer`]'s to say.
pub fn written(schema: &SeasonSchema, payload: &Payload) -> Vec<(String, String)> {
    schema
        .fields()
        .filter(|f| matches!(f.kind, FieldKind::Text { .. }))
        .filter_map(|f| {
            let text = payload.get(&f.key).and_then(Value::as_text)?.trim();
            (!text.is_empty()).then(|| (f.label.clone(), text.to_string()))
        })
        .collect()
}

/// Take the notes out of `payload`, for a viewer [`Notes::for_viewer`] hides
/// them from (S2). Removes `schema`'s text fields, and any text answer that is
/// not one of its choice fields: a field dropped from the form cannot be told
/// apart from notes, so it is treated as notes (U13).
pub fn redact(schema: &SeasonSchema, payload: &mut Payload) {
    payload.retain(|key, value| match value {
        Value::Text(_) => schema
            .fields()
            .any(|f| f.key == *key && matches!(f.kind, FieldKind::Select { .. })),
        Value::Count(_) | Value::Flag(_) => true,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::season::current_season;

    #[test]
    fn only_the_writing_team_reads_its_notes() {
        assert_eq!(Notes::for_viewer(Some(10101), Some(10101)), Notes::Shown);
        assert_eq!(Notes::for_viewer(Some(254), Some(10101)), Notes::Hidden);
    }

    #[test]
    fn nobody_reads_notes_without_a_team_on_both_sides() {
        assert_eq!(Notes::for_viewer(None, Some(10101)), Notes::Hidden);
        assert_eq!(Notes::for_viewer(Some(10101), None), Notes::Hidden);
        assert_eq!(
            Notes::for_viewer(None, None),
            Notes::Hidden,
            "two teamless people are not a team"
        );
    }

    #[test]
    fn written_notes_are_the_filled_in_text_fields() {
        let schema = current_season().unwrap();
        let mut payload = Payload::new();
        payload.insert("teleop_scored".into(), Value::Count(9));
        assert!(written(&schema, &payload).is_empty(), "no text at all");

        payload.insert("notes".into(), Value::Text("   ".into()));
        assert!(written(&schema, &payload).is_empty(), "blank is not a note");

        payload.insert("notes".into(), Value::Text(" tippy on the ramp\n".into()));
        assert_eq!(
            written(&schema, &payload),
            [(
                "Anything the numbers miss".to_string(),
                "tippy on the ramp".to_string()
            )]
        );
    }

    #[test]
    fn redacting_keeps_numbers_and_choices_and_drops_every_kind_of_prose() {
        let schema = current_season().unwrap();
        let mut payload: Payload = [
            ("starting_position", Value::Text("left".into())),
            ("teleop_scored", Value::Count(4)),
            ("broke_down", Value::Flag(true)),
            ("notes", Value::Text("tippy on the ramp".into())),
            (
                "old_comment_field",
                Value::Text("from last year's form".into()),
            ),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
        redact(&schema, &mut payload);
        let kept: Vec<&str> = payload.keys().map(String::as_str).collect();
        assert_eq!(kept, ["broke_down", "starting_position", "teleop_scored"]);
    }
}
