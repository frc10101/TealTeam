//! Outbox entries the Pi refused (C10), kept for the lead scout.
//!
//! A push (C7) answers each entry it cannot record with a reason, and the
//! device keeps the entry. That alone leaves it on one tablet, in the pocket
//! of a scout who cannot fix the schedule. So the Pi keeps its own copy here,
//! and the lead scout's page lists the ones nobody has dealt with yet.
//!
//! Server-only: no change log, no snapshot. What a lead does with one comes
//! back to the device the ordinary way, as an observation under the same
//! record id, which clears the device's entry.

use chrono::{DateTime, Utc};
use sqlx::Row;
use tt_core::outbox::QueuedObservation;
use tt_core::season::{parse_payload, payload_to_json};
use tt_repo::Result;

use crate::SqliteRepo;
use crate::users::{from_sql, query_err, to_sql};

/// A refusal to keep: the entry as pushed, and what the push knew.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewRefusal {
    /// `observed_at` already on the Pi's clock.
    pub entry: QueuedObservation,
    pub scouter_id: Option<i64>,
    pub device_id: Option<i64>,
    pub submitting_team: Option<i32>,
    pub reason: String,
}

/// What a lead did with a refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    /// Recorded as this observation.
    Recorded(i64),
    Dismissed,
}

/// A kept refusal, with names for the page.
#[derive(Debug, Clone, PartialEq)]
pub struct Refusal {
    pub id: i64,
    pub entry: QueuedObservation,
    pub scouter_id: Option<i64>,
    /// `None` when the account is gone.
    pub scouter_name: Option<String>,
    pub device_id: Option<i64>,
    /// The tablet's name, or "Device" and the start of its id.
    pub device_name: Option<String>,
    pub submitting_team: Option<i32>,
    pub reason: String,
    pub refused_at: DateTime<Utc>,
    /// `None` while it waits for a lead.
    pub resolution: Option<Resolution>,
    pub resolved_by: Option<String>,
    pub resolved_at: Option<DateTime<Utc>>,
}

const SELECT: &str = "SELECT r.*, u.name AS scouter_name, d.name AS device_name, \
         d.device_uuid, b.name AS resolver_name \
     FROM refused_entries r \
     LEFT JOIN users u ON u.id = r.scouter_id \
     LEFT JOIN devices d ON d.id = r.device_id \
     LEFT JOIN users b ON b.id = r.resolved_by";

fn from_row(row: &sqlx::sqlite::SqliteRow) -> Refusal {
    let id: i64 = row.get("id");
    let ts = |column: &str| {
        row.get::<Option<String>, _>(column)
            .as_deref()
            .and_then(from_sql)
    };
    let device_name = match (
        row.get::<Option<String>, _>("device_name")
            .filter(|n| !n.trim().is_empty()),
        row.get::<Option<String>, _>("device_uuid"),
    ) {
        (Some(name), _) => Some(name.trim().to_string()),
        (None, Some(uuid)) => Some(format!(
            "Device {}",
            uuid.chars().take(8).collect::<String>()
        )),
        (None, None) => None,
    };
    let resolution = match row.get::<Option<String>, _>("resolution").as_deref() {
        Some("recorded") => Some(Resolution::Recorded(
            row.get::<Option<i64>, _>("observation_id")
                .unwrap_or_default(),
        )),
        Some(_) => Some(Resolution::Dismissed),
        None => None,
    };
    Refusal {
        id,
        entry: QueuedObservation {
            client_record_id: row.get("client_record_id"),
            match_key: row.get("match_key"),
            team_number: row.get("team_number"),
            payload: parse_payload(&row.get::<String, _>("payload")).unwrap_or_else(|e| {
                tracing::warn!("refused entry {id} has an unreadable payload: {e}");
                Default::default()
            }),
            schema_version: row.get("schema_version"),
            observed_at: ts("observed_at").unwrap_or_default(),
        },
        scouter_id: row.get("scouter_id"),
        scouter_name: row.get("scouter_name"),
        device_id: row.get("device_id"),
        device_name,
        submitting_team: row.get("submitting_team"),
        reason: row.get("reason"),
        refused_at: ts("refused_at").unwrap_or_default(),
        resolution,
        resolved_by: row.get("resolver_name"),
        resolved_at: ts("resolved_at"),
    }
}

impl SqliteRepo {
    /// Keep a refusal. The same entry refused again, say from an exported
    /// outbox pushed later, gets the newer reason while it still waits, and
    /// is left alone once a lead has dealt with it.
    pub async fn keep_refusal(&self, refusal: &NewRefusal, now: DateTime<Utc>) -> Result<()> {
        let entry = &refusal.entry;
        sqlx::query(
            "INSERT INTO refused_entries (client_record_id, match_key, team_number, payload, \
                 schema_version, observed_at, scouter_id, device_id, submitting_team, reason, \
                 refused_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT (client_record_id) DO UPDATE SET \
                 reason = excluded.reason, refused_at = excluded.refused_at \
             WHERE resolution IS NULL",
        )
        .bind(&entry.client_record_id)
        .bind(&entry.match_key)
        .bind(entry.team_number)
        .bind(payload_to_json(&entry.payload))
        .bind(entry.schema_version)
        .bind(to_sql(entry.observed_at))
        .bind(refusal.scouter_id)
        .bind(refusal.device_id)
        .bind(refusal.submitting_team)
        .bind(&refusal.reason)
        .bind(to_sql(now))
        .execute(self.pool())
        .await
        .map_err(|e| query_err("keeping a refused entry", e))?;
        Ok(())
    }

    /// Refusals no lead has dealt with, oldest first.
    pub async fn open_refusals(&self) -> Result<Vec<Refusal>> {
        let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
            "{SELECT} WHERE r.resolution IS NULL ORDER BY r.id"
        )))
        .fetch_all(self.pool())
        .await
        .map_err(|e| query_err("reading refused entries", e))?;
        Ok(rows.iter().map(from_row).collect())
    }

    pub async fn refusal(&self, id: i64) -> Result<Option<Refusal>> {
        let row = sqlx::query(sqlx::AssertSqlSafe(format!("{SELECT} WHERE r.id = ?")))
            .bind(id)
            .fetch_optional(self.pool())
            .await
            .map_err(|e| query_err("reading a refused entry", e))?;
        Ok(row.as_ref().map(from_row))
    }

    /// Mark a refusal dealt with. `false` when someone already had.
    pub async fn resolve_refusal(
        &self,
        id: i64,
        resolution: Resolution,
        by: i64,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        let (word, observation) = match resolution {
            Resolution::Recorded(observation) => ("recorded", Some(observation)),
            Resolution::Dismissed => ("dismissed", None),
        };
        let done = sqlx::query(
            "UPDATE refused_entries \
             SET resolution = ?, observation_id = ?, resolved_by = ?, resolved_at = ? \
             WHERE id = ? AND resolution IS NULL",
        )
        .bind(word)
        .bind(observation)
        .bind(by)
        .bind(to_sql(now))
        .bind(id)
        .execute(self.pool())
        .await
        .map_err(|e| query_err("resolving a refused entry", e))?;
        Ok(done.rows_affected() == 1)
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use tt_core::season::Value;
    use tt_repo::{NewUser, Repo};

    use super::*;

    fn at(minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 3, 14, 10, minute, 0).unwrap()
    }

    fn refusal(record_id: &str, reason: &str) -> NewRefusal {
        NewRefusal {
            entry: QueuedObservation {
                client_record_id: record_id.into(),
                match_key: "2026now_qm99".into(),
                team_number: 254,
                payload: [("teleop_scored".to_string(), Value::Count(9))].into(),
                schema_version: 1,
                observed_at: at(1),
            },
            scouter_id: Some(1),
            device_id: None,
            submitting_team: Some(10101),
            reason: reason.into(),
        }
    }

    #[tokio::test]
    async fn a_refusal_waits_until_a_lead_deals_with_it_and_then_stays() {
        let repo = SqliteRepo::connect("sqlite::memory:").unwrap();
        crate::migrate::apply(repo.pool()).await.unwrap();
        let scout = NewUser {
            email: "sam@example.com".into(),
            password_hash: "x".into(),
            name: "Sam".into(),
            team_number: Some(10101),
            roles: Default::default(),
        };
        repo.create_user(scout, at(0)).await.unwrap();

        repo.keep_refusal(&refusal("r1", "not on the schedule"), at(2))
            .await
            .unwrap();
        repo.keep_refusal(&refusal("r2", "not in that match"), at(3))
            .await
            .unwrap();
        // Pushed again from an export: one row, the newer reason.
        repo.keep_refusal(&refusal("r1", "still not on the schedule"), at(4))
            .await
            .unwrap();

        let open = repo.open_refusals().await.unwrap();
        assert_eq!(open.len(), 2);
        let first = &open[0];
        assert_eq!(first.entry, refusal("r1", "").entry);
        assert_eq!(first.reason, "still not on the schedule");
        assert_eq!(first.refused_at, at(4));
        assert_eq!(first.scouter_name.as_deref(), Some("Sam"));
        assert_eq!(first.device_name, None);
        assert_eq!(first.resolution, None);

        assert!(
            repo.resolve_refusal(first.id, Resolution::Dismissed, 1, at(5))
                .await
                .unwrap()
        );
        // A second lead pressing at once finds it done.
        assert!(
            !repo
                .resolve_refusal(first.id, Resolution::Recorded(7), 1, at(6))
                .await
                .unwrap()
        );
        // And pushing it again does not reopen it.
        repo.keep_refusal(&refusal("r1", "again"), at(7))
            .await
            .unwrap();

        let open = repo.open_refusals().await.unwrap();
        assert_eq!(
            open.iter()
                .map(|r| r.entry.client_record_id.as_str())
                .collect::<Vec<_>>(),
            ["r2"]
        );
        let done = repo.refusal(first.id).await.unwrap().unwrap();
        assert_eq!(done.resolution, Some(Resolution::Dismissed));
        assert_eq!(done.resolved_by.as_deref(), Some("Sam"));
        assert_eq!(done.resolved_at, Some(at(5)));
        assert_eq!(done.reason, "still not on the schedule");
        assert_eq!(repo.refusal(999).await.unwrap(), None);
    }
}
