//! Pick lists (U20, L14): `tt_repo_sqlite::picklist`, on the device.
//!
//! The same yrs document and the same row matching. A snapshot carries the
//! viewer's team's rows but no `pick_list_docs` (it is server-only), so the
//! first merge here builds the document from the rows, exactly as the server
//! does for a list stored before L14.

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use tt_core::picklist::{Entry, PickDoc, Tag};
use tt_repo::{RepoError, Result};

use crate::ClientRepo;
use crate::sql::{Context, to_sql};

fn load(conn: &Connection, owning_team: i32, event_key: &str) -> Result<Vec<Entry>> {
    let mut statement = conn
        .prepare(
            "SELECT client_record_id, picked_team, color, crossed FROM pick_list_entries \
             WHERE owning_team = ? AND event_key = ? ORDER BY position, id",
        )
        .ctx("loading a pick list")?;
    statement
        .query_map(params![owning_team, event_key], |row| {
            Ok(Entry {
                record_id: row.get(0)?,
                team_number: row.get(1)?,
                // A colour this build does not know shows as untagged.
                tag: row
                    .get::<_, Option<String>>(2)?
                    .as_deref()
                    .and_then(Tag::parse),
                crossed: row.get::<_, i64>(3)? != 0,
            })
        })
        .and_then(|rows| rows.collect())
        .ctx("loading a pick list")
}

fn doc(conn: &Connection, owning_team: i32, event_key: &str, ts: &str) -> Result<PickDoc> {
    let state: Option<Vec<u8>> = conn
        .query_row(
            "SELECT state FROM pick_list_docs WHERE owning_team = ? AND event_key = ?",
            params![owning_team, event_key],
            |row| row.get(0),
        )
        .optional()
        .ctx("reading a pick list document")?;
    if let Some(state) = state {
        return PickDoc::load(&state)
            .map_err(|e| RepoError::Query(format!("reading a pick list document: {e}")));
    }
    let doc = PickDoc::from_entries(&load(conn, owning_team, event_key)?);
    store_doc(conn, owning_team, event_key, &doc, ts)?;
    Ok(doc)
}

fn store_doc(
    conn: &Connection,
    owning_team: i32,
    event_key: &str,
    doc: &PickDoc,
    ts: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO pick_list_docs (owning_team, event_key, state, updated_at) \
         VALUES (?, ?, ?, ?) \
         ON CONFLICT (owning_team, event_key) DO UPDATE SET \
            state = excluded.state, updated_at = excluded.updated_at",
        params![owning_team, event_key, doc.state(), ts],
    )
    .ctx("storing a pick list document")?;
    Ok(())
}

impl ClientRepo {
    pub(crate) fn pick_list_impl(&self, owning_team: i32, event_key: &str) -> Result<Vec<Entry>> {
        load(&self.conn, owning_team, event_key)
    }

    pub(crate) fn pick_list_doc_impl(
        &self,
        owning_team: i32,
        event_key: &str,
        now: DateTime<Utc>,
    ) -> Result<Vec<u8>> {
        let tx = self
            .conn
            .unchecked_transaction()
            .ctx("starting pick list read")?;
        let doc = doc(&tx, owning_team, event_key, &to_sql(now))?;
        tx.commit().ctx("committing pick list read")?;
        Ok(doc.state())
    }

    pub(crate) fn merge_pick_list_impl(
        &self,
        owning_team: i32,
        event_key: &str,
        update: &[u8],
        now: DateTime<Utc>,
    ) -> Result<Vec<Entry>> {
        let ts = to_sql(now);
        let tx = self
            .conn
            .unchecked_transaction()
            .ctx("starting pick list write")?;

        let mut doc = doc(&tx, owning_team, event_key, &ts)?;
        doc.merge(update).map_err(RepoError::Query)?;
        let before = load(&tx, owning_team, event_key)?;
        let list = doc.entries();

        for gone in before.iter().filter(|e| {
            !list
                .iter()
                .any(|n| n.team_number == e.team_number && n.record_id == e.record_id)
        }) {
            tx.execute(
                "DELETE FROM pick_list_entries WHERE client_record_id = ?",
                [&gone.record_id],
            )
            .ctx("removing a pick")?;
        }

        for (index, entry) in list.iter().enumerate() {
            tx.execute(
                "INSERT INTO pick_list_entries (client_record_id, owning_team, event_key, \
                     picked_team, color, crossed, position, created_at, updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) \
                 ON CONFLICT (owning_team, event_key, picked_team) DO UPDATE SET \
                    color = excluded.color, crossed = excluded.crossed, \
                    position = excluded.position, updated_at = excluded.updated_at \
                 WHERE color IS NOT excluded.color OR crossed != excluded.crossed \
                    OR position != excluded.position",
                params![
                    entry.record_id,
                    owning_team,
                    event_key,
                    entry.team_number,
                    entry.tag.map(Tag::key),
                    entry.crossed as i64,
                    index as i64 + 1,
                    ts,
                    ts,
                ],
            )
            .ctx("storing a pick")?;
        }

        store_doc(&tx, owning_team, event_key, &doc, &ts)?;
        tx.commit().ctx("committing pick list")?;
        Ok(list)
    }
}
