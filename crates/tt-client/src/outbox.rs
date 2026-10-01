//! The outbox (C7): what this device recorded and the Pi has not yet echoed.
//!
//! An observation recorded here goes into `observations`, as on the server,
//! and into `outbox` with what the Pi needs to record it
//! ([`QueuedObservation`]). Both are in this one file, and the file is only
//! ever written whole, so a save has both or neither. The sync client
//! ([`crate::sync`]) pushes the queue and clears an entry when the change log
//! brings the observation back: the Pi's echo, not its receipt, is "saved".
//!
//! A refused entry stays, with the Pi's reason, and is not sent again. Its
//! observation is taken off this device's tables, since the Pi will never
//! have it; the answers are still in the entry, and [`ClientRepo::export_outbox`]
//! hands the lot over as a file.
//!
//! The table is this device's own: the server has none, so a snapshot (S10)
//! never carries one, and it is made here the first time it is wanted.
//! Replacing the file with a new snapshot would lose it, which is why
//! `snapshot.js` will not do that unasked.

use chrono::{DateTime, Utc};
use rusqlite::{OptionalExtension, params};
use serde::Serialize;
use tt_core::outbox::QueuedObservation;
use tt_core::record_id;
use tt_repo::{NewObservation, Recorded, RepoError, Result};

use crate::ClientRepo;
use crate::sql::{Context, from_sql, to_sql, ts_column};

const OUTBOX: &str = "CREATE TABLE IF NOT EXISTS outbox (
    record_id TEXT    PRIMARY KEY,
    kind      TEXT    NOT NULL CHECK (kind IN ('observation')),
    -- The QueuedObservation, as JSON: what is pushed, and what is exported.
    body      TEXT    NOT NULL,
    queued_at TEXT    NOT NULL,
    attempts  INTEGER NOT NULL DEFAULT 0,
    tried_at  TEXT,
    -- The Pi's reason, once it has said no; NULL while it is still to send.
    refused   TEXT
) STRICT";

/// One entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Queued {
    #[serde(flatten)]
    pub observation: QueuedObservation,
    pub queued_at: DateTime<Utc>,
    pub attempts: i64,
    pub tried_at: Option<DateTime<Utc>>,
    pub refused: Option<String>,
}

/// What [`ClientRepo::export_outbox`] writes: a [`tt_core::outbox::Push`]
/// with more on each entry, so the file can be pushed as it is.
#[derive(Serialize)]
struct Export<'a> {
    format: &'static str,
    version: u32,
    exported_at: DateTime<Utc>,
    schema: Option<i64>,
    observations: &'a [Queued],
}

impl ClientRepo {
    pub(crate) fn ensure_outbox(&self) -> Result<()> {
        self.conn
            .execute_batch(OUTBOX)
            .ctx("making the device's outbox")
    }

    /// Record an observation on this device and queue it for the Pi.
    ///
    /// Queued only when it is new here: a duplicate is already queued, or
    /// already came from the Pi.
    pub async fn queue_observation(
        &self,
        observation: &NewObservation,
        now: DateTime<Utc>,
    ) -> Result<Recorded> {
        self.saved(self.queue_observation_impl(observation, now))
            .await
    }

    fn queue_observation_impl(
        &self,
        observation: &NewObservation,
        now: DateTime<Utc>,
    ) -> Result<Recorded> {
        // The echo is matched by this id, so it must be the Pi's spelling.
        let Some(id) = record_id::normalize(&observation.client_record_id) else {
            return Err(RepoError::Refused(
                "an observation needs a record id".into(),
            ));
        };
        self.ensure_outbox()?;
        let observation = NewObservation {
            client_record_id: id.clone(),
            ..observation.clone()
        };
        let recorded = self.record_observation_impl(&observation, now)?;
        let Recorded::Created(row) = recorded else {
            return Ok(recorded);
        };
        let body = QueuedObservation {
            client_record_id: id.clone(),
            match_key: observation.match_key,
            team_number: observation.team_number,
            payload: observation.payload,
            schema_version: observation.schema_version,
            observed_at: observation.observed_at,
        };
        let queued = self.conn.execute(
            "INSERT INTO outbox (record_id, kind, body, queued_at) \
             VALUES (?, 'observation', ?, ?) ON CONFLICT (record_id) DO NOTHING",
            params![id, to_json(&body), to_sql(now)],
        );
        if let Err(e) = queued {
            // Nothing is saved until this returns: take the row back, so the
            // file never holds an observation that will not be sent.
            let _ = self
                .conn
                .execute("DELETE FROM observations WHERE id = ?", [row]);
            return Err(crate::sql::query_err("queueing an observation", e));
        }
        Ok(recorded)
    }

    /// Everything in the outbox, oldest first: waiting, and refused.
    pub fn outbox(&self) -> Result<Vec<Queued>> {
        self.ensure_outbox()?;
        let rows = self.all(
            "SELECT body, queued_at, attempts, tried_at, refused FROM outbox \
             ORDER BY queued_at, record_id",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            },
            "reading the outbox",
        )?;
        rows.into_iter()
            .map(|(body, queued_at, attempts, tried_at, refused)| {
                Ok(Queued {
                    observation: from_json(&body)?,
                    queued_at: from_sql(&queued_at).unwrap_or_default(),
                    attempts,
                    tried_at: ts_column(tried_at),
                    refused,
                })
            })
            .collect()
    }

    /// The outbox as a file to hand to the lead scout: for when it cannot
    /// be pushed, such as a device on an old build (S11).
    pub fn export_outbox(&self, now: DateTime<Utc>) -> Result<String> {
        let observations = self.outbox()?;
        let export = Export {
            format: "tealteam-outbox",
            version: 1,
            exported_at: now,
            schema: self.schema()?,
            observations: &observations,
        };
        serde_json::to_string_pretty(&export)
            .map_err(|e| RepoError::Query(format!("writing the outbox out: {e}")))
    }

    /// Drop a refused entry, once someone has dealt with it. One still
    /// waiting is not dropped: `false`.
    pub async fn discard_refused(&self, record_id: &str) -> Result<bool> {
        self.ensure_outbox()?;
        let gone = self
            .conn
            .execute(
                "DELETE FROM outbox WHERE record_id = ? AND refused IS NOT NULL",
                [record_id],
            )
            .ctx("discarding an outbox entry")?;
        self.saved(Ok(gone > 0)).await
    }

    /// Up to `limit` entries still to send, after `after` in id order.
    pub(crate) fn waiting(
        &self,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<QueuedObservation>> {
        self.ensure_outbox()?;
        let bodies = self.all(
            "SELECT body FROM outbox WHERE refused IS NULL AND record_id > ? \
             ORDER BY record_id LIMIT ?",
            params![after.unwrap_or(""), limit as i64],
            |row| row.get::<_, String>(0),
            "reading the outbox",
        )?;
        bodies.iter().map(|body| from_json(body)).collect()
    }

    pub(crate) fn mark_tried(&self, record_ids: &[&str], now: DateTime<Utc>) -> Result<()> {
        for id in record_ids {
            self.conn
                .execute(
                    "UPDATE outbox SET attempts = attempts + 1, tried_at = ? WHERE record_id = ?",
                    params![to_sql(now), id],
                )
                .ctx("noting an outbox attempt")?;
        }
        Ok(())
    }

    /// The Pi said no: keep the entry with why, and take its observation off
    /// this device's tables, unless the Pi's own copy has already arrived.
    pub(crate) fn mark_refused(&self, record_id: &str, reason: &str) -> Result<()> {
        let tx = self.conn.unchecked_transaction().ctx("noting a refusal")?;
        let waiting = tx
            .execute(
                "UPDATE outbox SET refused = ? WHERE record_id = ?",
                params![reason, record_id],
            )
            .ctx("noting a refusal")?;
        if waiting > 0 {
            tx.execute(
                "DELETE FROM observations WHERE client_record_id = ?",
                [record_id],
            )
            .ctx("noting a refusal")?;
        }
        tx.commit().ctx("noting a refusal")
    }

    /// Whether `record_id` is still waiting to be sent.
    pub fn is_queued(&self, record_id: &str) -> Result<bool> {
        self.ensure_outbox()?;
        self.conn
            .query_row(
                "SELECT 1 FROM outbox WHERE record_id = ? AND refused IS NULL",
                [record_id],
                |_| Ok(()),
            )
            .optional()
            .map(|found| found.is_some())
            .ctx("reading the outbox")
    }
}

fn to_json(body: &QueuedObservation) -> String {
    // A map of strings, numbers, and booleans: cannot fail.
    serde_json::to_string(body).unwrap_or_default()
}

fn from_json(body: &str) -> Result<QueuedObservation> {
    serde_json::from_str(body)
        .map_err(|e| RepoError::Query(format!("reading an outbox entry: {e}")))
}
