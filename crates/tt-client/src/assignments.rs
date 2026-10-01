//! Scout assignments, rankings typed in by hand, and point weights:
//! `tt_repo_sqlite::{assignments, standings, weights}`, on the device.

use chrono::{DateTime, Utc};
use rusqlite::{Row, params};
use tt_core::assignments::{Assignee, AssigneeKey, Assignment};
use tt_core::season::WeightOverrides;
use tt_core::standings::Standing;
use tt_repo::{Device, NewAssignment, Result};

use crate::ClientRepo;
use crate::sql::{Context, to_sql, ts_column};

fn assignment_from_row(row: &Row) -> rusqlite::Result<Option<Assignment>> {
    // A snapshot keeps the assignments but none of `users` or `devices`
    // (S10), so the name is usually missing here, where on the server it
    // never is. The grid still reads, with the id in place of the name.
    let scout = match row.get::<_, Option<i64>>("scouter_id")? {
        Some(id) => Some(Assignee::Scout {
            id,
            name: row
                .get::<_, Option<String>>("scouter_name")?
                .unwrap_or_else(|| format!("Scout {id}")),
        }),
        None => None,
    };
    let device = match row.get::<_, Option<i64>>("device_id")? {
        Some(id) => {
            let name = match row.get::<_, Option<String>>("device_uuid")? {
                // Named as the device list names it.
                Some(device_uuid) => Device {
                    id,
                    device_uuid,
                    name: row.get("device_name")?,
                    team_number: None,
                    last_seen_at: ts_column(row.get("last_seen_at")?),
                    last_user_id: None,
                    clock_offset_ms: None,
                }
                .display_name(),
                None => format!("Device {id}"),
            };
            Some(Assignee::Device { id, name })
        }
        None => None,
    };
    // A scout is the more specific instruction; a row with neither is
    // skipped, as on the server.
    let Some(assignee) = scout.or(device) else {
        return Ok(None);
    };
    Ok(Some(Assignment {
        match_key: row.get("match_key")?,
        team_number: row.get("team_number")?,
        assignee,
    }))
}

/// A robot the schedule names before any roster sync made its row. The
/// next roster sync replaces the placeholder name.
fn ensure_team(tx: &rusqlite::Transaction, team_number: i32, ts: &str) -> rusqlite::Result<usize> {
    tx.execute(
        "INSERT INTO teams (team_number, name, created_at, updated_at) \
         VALUES (?, ?, ?, ?) ON CONFLICT (team_number) DO NOTHING",
        params![team_number, format!("Team {team_number}"), ts, ts],
    )
}

impl ClientRepo {
    pub(crate) fn event_assignments_impl(&self, event_key: &str) -> Result<Vec<Assignment>> {
        let rows = self.all(
            "SELECT a.match_key, a.team_number, \
                    a.scouter_id, u.name AS scouter_name, \
                    a.device_id, d.device_uuid, d.name AS device_name, d.last_seen_at \
             FROM scout_assignments a \
             LEFT JOIN users u ON u.id = a.scouter_id \
             LEFT JOIN devices d ON d.id = a.device_id \
             WHERE a.event_key = ? \
             ORDER BY a.match_key, a.team_number",
            [event_key],
            assignment_from_row,
            "listing assignments",
        )?;
        Ok(rows.into_iter().flatten().collect())
    }

    pub(crate) fn set_assignments_impl(
        &self,
        assignments: &[NewAssignment],
        assigned_by: i64,
        now: DateTime<Utc>,
    ) -> Result<()> {
        let ts = to_sql(now);
        let tx = self
            .conn
            .unchecked_transaction()
            .ctx("starting assignment write")?;
        for a in assignments {
            ensure_team(&tx, a.team_number, &ts).ctx("ensuring assigned team exists")?;
            // Exactly one of the two, always.
            let (scouter, device) = match a.assignee {
                AssigneeKey::Scout(id) => (Some(id), None),
                AssigneeKey::Device(id) => (None, Some(id)),
            };
            tx.execute(
                "INSERT INTO scout_assignments (match_key, team_number, event_key, scouter_id, \
                     device_id, assigned_by, created_at, updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?) \
                 ON CONFLICT (match_key, team_number) DO UPDATE SET \
                    scouter_id  = excluded.scouter_id, \
                    device_id   = excluded.device_id, \
                    assigned_by = excluded.assigned_by, \
                    updated_at  = excluded.updated_at",
                params![
                    a.match_key,
                    a.team_number,
                    a.event_key,
                    scouter,
                    device,
                    assigned_by,
                    ts,
                    ts
                ],
            )
            .ctx("assigning a robot")?;
        }
        tx.commit().ctx("committing assignments")
    }

    pub(crate) fn unassign_impl(&self, match_key: &str, team_number: i32) -> Result<()> {
        self.conn
            .execute(
                "DELETE FROM scout_assignments WHERE match_key = ? AND team_number = ?",
                params![match_key, team_number],
            )
            .ctx("removing an assignment")?;
        Ok(())
    }

    pub(crate) fn clear_assignments_impl(
        &self,
        event_key: &str,
        match_key: Option<&str>,
    ) -> Result<u64> {
        let gone = self
            .conn
            .execute(
                "DELETE FROM scout_assignments \
                 WHERE event_key = ?1 AND (?2 IS NULL OR match_key = ?2)",
                params![event_key, match_key],
            )
            .ctx("clearing assignments")?;
        Ok(gone as u64)
    }

    // ── Rankings typed in by hand (I14) ─────────────────────────────────────

    /// The server's replace-outright rule: listed teams get their typed rank
    /// and record, everyone else at the event loses theirs, OPRs stay.
    pub(crate) fn record_standings_impl(
        &self,
        event_key: &str,
        standings: &[Standing],
        now: DateTime<Utc>,
    ) -> Result<()> {
        let ts = to_sql(now);
        let tx = self
            .conn
            .unchecked_transaction()
            .ctx("starting standings write")?;
        tx.execute(
            "UPDATE team_event_stats SET rank = NULL WHERE event_key = ?",
            [event_key],
        )
        .ctx("clearing ranks")?;
        for s in standings {
            ensure_team(&tx, s.team_number, &ts).ctx("ensuring ranked team exists")?;
            let record = s.record;
            tx.execute(
                "INSERT INTO team_event_stats (team_number, event_key, rank, qual_average, \
                     wins, losses, ties, matches_played, synced_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) \
                 ON CONFLICT (team_number, event_key) DO UPDATE SET \
                    rank = excluded.rank, qual_average = excluded.qual_average, \
                    wins = excluded.wins, losses = excluded.losses, ties = excluded.ties, \
                    matches_played = excluded.matches_played, \
                    avg_match_points = NULL, dq_count = NULL, qual_points = NULL, \
                    total_points = NULL, synced_at = excluded.synced_at",
                params![
                    s.team_number,
                    event_key,
                    s.rank,
                    s.ranking_score,
                    record.map(|r| r.wins),
                    record.map(|r| r.losses),
                    record.map(|r| r.ties),
                    record.map(|r| r.played()),
                    ts,
                ],
            )
            .ctx("storing a typed rank")?;
        }
        tx.commit().ctx("committing standings")
    }

    // ── Point weights (L12) ─────────────────────────────────────────────────

    pub(crate) fn weight_overrides_impl(&self) -> Result<WeightOverrides> {
        let rows = self.all(
            "SELECT field_key, option_key, points FROM scouting_point_weights",
            [],
            |row| Ok(((row.get(0)?, row.get(1)?), row.get(2)?)),
            "loading point weights",
        )?;
        Ok(rows.into_iter().collect())
    }

    pub(crate) fn replace_weight_overrides_impl(
        &self,
        overrides: &WeightOverrides,
        now: DateTime<Utc>,
    ) -> Result<()> {
        let ts = to_sql(now);
        let tx = self
            .conn
            .unchecked_transaction()
            .ctx("starting weights write")?;
        tx.execute("DELETE FROM scouting_point_weights", [])
            .ctx("clearing point weights")?;
        for (field, option, points) in overrides.iter() {
            tx.execute(
                "INSERT INTO scouting_point_weights (field_key, option_key, points, updated_at) \
                 VALUES (?, ?, ?, ?)",
                params![field, option, points, ts],
            )
            .ctx("saving a point weight")?;
        }
        tx.commit().ctx("committing point weights")
    }
}
