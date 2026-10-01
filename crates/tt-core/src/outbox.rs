//! What a device's outbox sends the Pi (C7), in the one shape both read.
//!
//! A scout with no signal records an observation on the device and queues
//! it. Whenever there is a connection, the queue goes to `POST
//! /api/sync/push` as a [`Push`]; the Pi checks each entry with the same
//! rules as the form, records it, and answers with a [`Receipt`] for each.
//!
//! A receipt saying "recorded" does not clear the entry. The device keeps it
//! until the observation comes back to it through the change log (S2), so a
//! reply lost on the way is no different from a push that never arrived:
//! the entry goes again, and its `client_record_id` makes the second copy a
//! no-op (D7). A refusal is final, and the entry is kept with the reason
//! rather than dropped.
//!
//! Only what the scout decided travels. Which event and alliance follow from
//! the match on the Pi's schedule, and who recorded it, from which tablet,
//! for which team, from the session that pushes.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::season::Payload;

/// Most entries in one push; a device with more sends them in turns.
pub const MAX_PER_PUSH: usize = 100;

/// One observation as the device recorded it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueuedObservation {
    pub client_record_id: String,
    pub match_key: String,
    pub team_number: i32,
    pub payload: Payload,
    /// The form the answers were given on. One other than the Pi's is
    /// recorded as it is, and flagged on the review page.
    pub schema_version: i64,
    /// The device's clock. The Pi corrects it by the tablet's measured
    /// offset (S12).
    pub observed_at: DateTime<Utc>,
}

/// The body of a push. An exported outbox has this shape too, with more
/// fields on each entry, so the file can be pushed as it is.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Push {
    #[serde(default)]
    pub observations: Vec<QueuedObservation>,
}

/// What became of one entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receipt {
    pub client_record_id: String,
    #[serde(flatten)]
    pub outcome: Outcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum Outcome {
    /// Stored now or before. The change log will bring it back.
    Recorded,
    /// Will never be stored as it is; `reason` is for a person.
    Refused { reason: String },
}

/// The answer to a push: the Pi's schema (S11), and a receipt per entry, in
/// the order they were sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushReply {
    pub schema: i64,
    pub receipts: Vec<Receipt>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::season::Value;
    use chrono::TimeZone;

    #[test]
    fn a_receipt_reads_as_one_flat_object() {
        let refused = Receipt {
            client_record_id: "r1".into(),
            outcome: Outcome::Refused {
                reason: "not on the schedule".into(),
            },
        };
        let json = serde_json::to_string(&refused).unwrap();
        assert_eq!(
            json,
            r#"{"client_record_id":"r1","outcome":"refused","reason":"not on the schedule"}"#
        );
        assert_eq!(serde_json::from_str::<Receipt>(&json).unwrap(), refused);
        let recorded: Receipt =
            serde_json::from_str(r#"{"client_record_id":"r2","outcome":"recorded"}"#).unwrap();
        assert_eq!(recorded.outcome, Outcome::Recorded);
    }

    #[test]
    fn a_push_survives_the_trip_and_ignores_what_an_export_adds() {
        let push = Push {
            observations: vec![QueuedObservation {
                client_record_id: "r1".into(),
                match_key: "2026now_qm2".into(),
                team_number: 254,
                payload: [
                    ("teleop_scored".to_string(), Value::Count(9)),
                    ("broke_down".to_string(), Value::Flag(true)),
                    ("notes".to_string(), Value::Text("tippy".into())),
                ]
                .into(),
                schema_version: 1,
                observed_at: Utc.with_ymd_and_hms(2026, 3, 14, 10, 0, 0).unwrap(),
            }],
        };
        let json = serde_json::to_value(&push).unwrap();
        assert_eq!(serde_json::from_value::<Push>(json.clone()).unwrap(), push);

        let mut exported = json;
        exported["format"] = "tealteam-outbox".into();
        exported["observations"][0]["refused"] = "a reason".into();
        assert_eq!(serde_json::from_value::<Push>(exported).unwrap(), push);
    }
}
