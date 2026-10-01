//! Observations and their review: `tt_repo_sqlite::observations`, on the
//! device. A scout with no signal records through the outbox
//! (`ClientRepo::queue_observation`, C7), which calls this and queues the
//! push; the rows fire the same `changes` triggers as on the server.

use chrono::{DateTime, Utc};
use rusqlite::{OptionalExtension, Row, params};
use tt_core::assignments::Sighting;
use tt_core::review::{Decision, ReviewState};
use tt_core::season::{parse_payload, payload_to_json};
use tt_repo::{NewObservation, Recorded, RepoError, Result, StoredObservation};

use crate::ClientRepo;
use crate::sql::{Context, is_unique_violation, query_err, to_sql, ts_column};

fn stored_from_row(row: &Row) -> rusqlite::Result<StoredObservation> {
    let id: i64 = row.get("id")?;
    Ok(StoredObservation {
        id,
        match_key: row.get("match_key")?,
        event_key: row.get("event_key")?,
        team_number: row.get("team_number")?,
        alliance: row.get("alliance")?,
        payload: parse_payload(&row.get::<_, String>("payload")?).unwrap_or_else(|e| {
            tracing::warn!("observation {id} has an unreadable payload: {e}");
            Default::default()
        }),
        schema_version: row.get("schema_version")?,
        scouter_id: row.get("scouter_id")?,
        scouter_name: row.get("scouter_name")?,
        submitting_team: row.get("submitting_team")?,
        review_state: ReviewState::parse(&row.get::<_, String>("review_state")?)
            .unwrap_or(ReviewState::Pending),
        review_note: row.get("review_note")?,
        reviewer_name: row.get("reviewer_name")?,
        reviewed_at: ts_column(row.get("reviewed_at")?),
        observed_at: ts_column(row.get("observed_at")?),
        created_at: ts_column(row.get("created_at")?),
    })
}

/// With the names resolved. A snapshot keeps the names its rows use
/// (S10b); one it lacks reads as `None`, like a deleted account.
const STORED: &str = "SELECT o.*, s.name AS scouter_name, r.name AS reviewer_name \
                      FROM observations o \
                      LEFT JOIN users s ON s.id = o.scouter_id \
                      LEFT JOIN users r ON r.id = o.reviewed_by";

impl ClientRepo {
    pub(crate) fn record_observation_impl(
        &self,
        observation: &NewObservation,
        now: DateTime<Utc>,
    ) -> Result<Recorded> {
        let ts = to_sql(now);
        let tx = self
            .conn
            .unchecked_transaction()
            .ctx("starting observation write")?;
        tx.execute(
            "INSERT INTO teams (team_number, name, created_at, updated_at) VALUES (?, ?, ?, ?) \
             ON CONFLICT (team_number) DO NOTHING",
            params![
                observation.team_number,
                format!("Team {}", observation.team_number),
                ts,
                ts
            ],
        )
        .ctx("ensuring observed team exists")?;

        // DO NOTHING covers only client_record_id; the per-scout coverage
        // index still fails, and is a conflict.
        let inserted = tx
            .query_row(
                "INSERT INTO observations (client_record_id, match_key, team_number, event_key, \
                     alliance, payload, schema_version, scouter_id, device_id, submitting_team, \
                     observed_at, created_at, updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
                 ON CONFLICT (client_record_id) DO NOTHING \
                 RETURNING id",
                params![
                    observation.client_record_id,
                    observation.match_key,
                    observation.team_number,
                    observation.event_key,
                    observation.alliance,
                    payload_to_json(&observation.payload),
                    observation.schema_version,
                    observation.scouter_id,
                    observation.device_id,
                    observation.submitting_team,
                    to_sql(observation.observed_at),
                    ts,
                    ts,
                ],
                |row| row.get::<_, i64>(0),
            )
            .optional();
        let recorded = match inserted {
            Ok(Some(id)) => Recorded::Created(id),
            Ok(None) => Recorded::Duplicate(
                tx.query_row(
                    "SELECT id FROM observations WHERE client_record_id = ?",
                    [&observation.client_record_id],
                    |row| row.get(0),
                )
                .ctx("loading duplicate observation")?,
            ),
            Err(e) if is_unique_violation(&e) => {
                return Err(RepoError::Conflict {
                    what: "An observation of this robot in this match by this scout",
                });
            }
            Err(e) => return Err(query_err("recording observation", e)),
        };
        tx.commit().ctx("committing observation")?;
        Ok(recorded)
    }

    pub(crate) fn observed_teams_impl(&self, match_key: &str, scouter_id: i64) -> Result<Vec<i32>> {
        self.all(
            "SELECT team_number FROM observations \
             WHERE match_key = ? AND scouter_id = ? AND review_state <> 'declined' \
             ORDER BY team_number",
            params![match_key, scouter_id],
            |row| row.get(0),
            "listing observed teams",
        )
    }

    pub(crate) fn recorded_by_impl(
        &self,
        event_key: &str,
        scouter_id: i64,
    ) -> Result<Vec<(String, i32)>> {
        self.all(
            "SELECT match_key, team_number FROM observations \
             WHERE event_key = ? AND scouter_id = ? AND review_state <> 'declined' \
             ORDER BY match_key, team_number",
            params![event_key, scouter_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
            "listing recorded robots",
        )
    }

    pub(crate) fn observations_in_state_impl(
        &self,
        event_key: &str,
        state: ReviewState,
    ) -> Result<Vec<StoredObservation>> {
        self.all(
            &format!(
                "{STORED} WHERE o.event_key = ? AND o.review_state = ? \
                 ORDER BY o.created_at, o.id"
            ),
            params![event_key, state.as_str()],
            stored_from_row,
            "listing observations",
        )
    }

    pub(crate) fn observation_impl(&self, id: i64) -> Result<Option<StoredObservation>> {
        self.conn
            .query_row(&format!("{STORED} WHERE o.id = ?"), [id], stored_from_row)
            .optional()
            .ctx("loading observation")
    }

    pub(crate) fn review_observation_impl(
        &self,
        id: i64,
        decision: &Decision,
        reviewer_id: i64,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        let note = match decision {
            Decision::Approve => None,
            Decision::Decline(reason) => Some(reason.as_str()),
        };
        let ts = to_sql(now);
        // Guarded on the state, as on the server: a second review changes
        // nothing and says so.
        let done = self
            .conn
            .execute(
                "UPDATE observations SET \
                    review_state = ?, review_note = ?, reviewed_by = ?, reviewed_at = ?, \
                    updated_at = ?, \
                    submitting_team = COALESCE(submitting_team, \
                        (SELECT team_number FROM users WHERE id = observations.scouter_id)) \
                 WHERE id = ? AND review_state = 'pending'",
                params![decision.state().as_str(), note, reviewer_id, ts, ts, id],
            )
            .ctx("reviewing observation")?;
        Ok(done == 1)
    }

    pub(crate) fn declined_for_impl(
        &self,
        event_key: &str,
        scouter_id: i64,
    ) -> Result<Vec<StoredObservation>> {
        self.all(
            &format!(
                "{STORED} \
                 WHERE o.event_key = ? AND o.scouter_id = ? AND o.review_state = 'declined' \
                   AND NOT EXISTS (SELECT 1 FROM observations again \
                       WHERE again.match_key = o.match_key \
                         AND again.team_number = o.team_number \
                         AND again.scouter_id = o.scouter_id \
                         AND again.review_state <> 'declined') \
                 ORDER BY o.reviewed_at DESC, o.id DESC"
            ),
            params![event_key, scouter_id],
            stored_from_row,
            "listing declined observations",
        )
    }

    pub(crate) fn event_sightings_impl(&self, event_key: &str) -> Result<Vec<Sighting>> {
        self.all(
            "SELECT match_key, team_number, scouter_id, device_id FROM observations \
             WHERE event_key = ? AND review_state <> 'declined' \
             ORDER BY match_key, team_number, id",
            [event_key],
            |row| {
                Ok(Sighting {
                    match_key: row.get(0)?,
                    team_number: row.get(1)?,
                    scouter_id: row.get(2)?,
                    device_id: row.get(3)?,
                })
            },
            "listing sightings",
        )
    }
}
