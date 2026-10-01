//! The upstream log (S1) and the change log (S2): `tt_repo_sqlite::upstream`,
//! on the device.
//!
//! A snapshot keeps the newest response per path in scope, and empties
//! `changes`. Local writes refill it through the same triggers, so what this
//! device changed since its snapshot is what `changes` holds.

use chrono::{DateTime, Utc};
use rusqlite::{OptionalExtension, Row, params};
use tt_repo::{Change, NewUpstream, Result, UPSTREAM_KEEP_PER_PATH, UpstreamEntry};

use crate::ClientRepo;
use crate::sql::{Context, from_sql, to_sql};

fn entry_from(row: &Row) -> rusqlite::Result<UpstreamEntry> {
    Ok(UpstreamEntry {
        seq: row.get("seq")?,
        entry: NewUpstream {
            api: row.get("api")?,
            path: row.get("path")?,
            etag: row.get("etag")?,
            body: row.get("body")?,
            fetched_at: from_sql(&row.get::<_, String>("fetched_at")?).unwrap_or_default(),
            via: row.get("via")?,
        },
    })
}

impl ClientRepo {
    /// The server's append: skip a body the same as the newest for its path,
    /// then prune the path, in one transaction.
    pub(crate) fn append_upstream_impl(&self, entry: &NewUpstream) -> Result<Option<i64>> {
        let tx = self
            .conn
            .unchecked_transaction()
            .ctx("appending to the upstream log")?;
        let newest: Option<String> = tx
            .query_row(
                "SELECT body FROM upstream WHERE api = ? AND path = ? \
                 ORDER BY seq DESC LIMIT 1",
                params![entry.api, entry.path],
                |row| row.get(0),
            )
            .optional()
            .ctx("reading the upstream log")?;
        if newest.as_deref() == Some(entry.body.as_str()) {
            return Ok(None);
        }
        let seq: i64 = tx
            .query_row(
                "INSERT INTO upstream (api, path, etag, body, fetched_at, via) \
                 VALUES (?, ?, ?, ?, ?, ?) RETURNING seq",
                params![
                    entry.api,
                    entry.path,
                    entry.etag,
                    entry.body,
                    to_sql(entry.fetched_at),
                    entry.via,
                ],
                |row| row.get(0),
            )
            .ctx("appending to the upstream log")?;
        tx.execute(
            "DELETE FROM upstream WHERE api = ?1 AND path = ?2 AND seq NOT IN \
             (SELECT seq FROM upstream WHERE api = ?1 AND path = ?2 ORDER BY seq DESC LIMIT ?3)",
            params![entry.api, entry.path, UPSTREAM_KEEP_PER_PATH],
        )
        .ctx("pruning the upstream log")?;
        tx.commit().ctx("appending to the upstream log")?;
        Ok(Some(seq))
    }

    pub(crate) fn latest_upstream_impl(
        &self,
        api: &str,
        path: &str,
    ) -> Result<Option<UpstreamEntry>> {
        self.conn
            .query_row(
                "SELECT seq, api, path, etag, body, fetched_at, via FROM upstream \
                 WHERE api = ? AND path = ? ORDER BY seq DESC LIMIT 1",
                params![api, path],
                entry_from,
            )
            .optional()
            .ctx("reading the upstream log")
    }

    pub(crate) fn upstream_since_impl(&self, after: i64, limit: i64) -> Result<Vec<UpstreamEntry>> {
        self.all(
            "SELECT seq, api, path, etag, body, fetched_at, via FROM upstream \
             WHERE seq > ? ORDER BY seq LIMIT ?",
            params![after, limit],
            entry_from,
            "reading the upstream log",
        )
    }

    pub(crate) fn log_heads_impl(&self) -> Result<(i64, i64)> {
        self.conn
            .query_row(
                "SELECT (SELECT COALESCE(MAX(seq), 0) FROM changes), \
                        (SELECT COALESCE(MAX(seq), 0) FROM upstream)",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .ctx("reading the log heads")
    }

    pub(crate) fn changes_since_impl(
        &self,
        after: i64,
        limit: i64,
        settled_before: DateTime<Utc>,
    ) -> Result<Vec<Change>> {
        self.all(
            "SELECT seq, entity, entity_pk, op, payload, event_key, team_scope, created_at \
             FROM changes WHERE seq > ? AND created_at < ? ORDER BY seq LIMIT ?",
            params![after, to_sql(settled_before), limit],
            |row| {
                Ok(Change {
                    seq: row.get("seq")?,
                    entity: row.get("entity")?,
                    entity_pk: row.get("entity_pk")?,
                    op: row.get("op")?,
                    payload: row.get("payload")?,
                    event_key: row.get("event_key")?,
                    team_scope: row.get::<_, Option<i64>>("team_scope")?.map(|t| t as i32),
                    created_at: from_sql(&row.get::<_, String>("created_at")?).unwrap_or_default(),
                })
            },
            "reading the change log",
        )
    }
}
