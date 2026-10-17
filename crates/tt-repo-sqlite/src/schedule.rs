//! What a re-imported schedule leaves behind (FTC_BETA_PLAN.md, F3).
//!
//! The new matches arrive the way synced ones do, through the upstream log
//! and `upsert_match`. That replaces a match's robots, but not a match the
//! new schedule dropped, an assignment to a robot no longer in its match, or
//! a team no longer at the event. This tidies those, in one transaction,
//! without touching an observation: a dropped match that already has one
//! stays, and is reported, because deleting it would delete them too.

use std::collections::BTreeSet;
use std::sync::LazyLock;

use sqlx::Row;
use tt_core::matches::slot_names;
use tt_repo::Result;

use crate::SqliteRepo;
use crate::users::query_err;

/// What [`SqliteRepo::tidy_schedule`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tidied {
    /// Matches no longer on the schedule, gone.
    pub removed_matches: u64,
    /// Matches no longer on the schedule but kept, with how many
    /// observations each has: `(match key, count)`.
    pub kept_matches: Vec<(String, i64)>,
    /// Assignments to a robot no longer in its match, gone. Tablets hear of
    /// each through the change log.
    pub dropped_assignments: u64,
    /// Teams off the roster, unlinked from the event.
    pub unlinked_teams: u64,
}

static STALE_ASSIGNMENTS: LazyLock<String> = LazyLock::new(|| {
    format!(
        "DELETE FROM scout_assignments WHERE event_key = ? AND NOT EXISTS \
         (SELECT 1 FROM matches m WHERE m.tba_key = scout_assignments.match_key \
          AND scout_assignments.team_number IN ({}))",
        slot_names()
            .iter()
            .map(|s| format!("m.{s}"))
            .collect::<Vec<_>>()
            .join(", ")
    )
});

static STILL_NAMED: LazyLock<String> = LazyLock::new(|| {
    format!(
        "SELECT (SELECT COUNT(*) FROM observations WHERE event_key = ?1 AND team_number = ?2) \
              + (SELECT COUNT(*) FROM matches WHERE event_key = ?1 AND ?2 IN ({}))",
        slot_names().join(", ")
    )
});

impl SqliteRepo {
    /// Bring an event in line with a schedule just stored: `matches` are its
    /// match keys and `roster` its teams.
    pub async fn tidy_schedule(
        &self,
        event_key: &str,
        matches: &[String],
        roster: &[i32],
    ) -> Result<Tidied> {
        let mut tidied = Tidied::default();
        let keep: BTreeSet<&str> = matches.iter().map(String::as_str).collect();
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| query_err("starting a schedule tidy", e))?;

        let stored = sqlx::query(
            "SELECT m.tba_key, (SELECT COUNT(*) FROM observations o WHERE o.match_key = m.tba_key) \
                 AS observed \
             FROM matches m WHERE m.event_key = ? ORDER BY m.tba_key",
        )
        .bind(event_key)
        .fetch_all(&mut *tx)
        .await
        .map_err(|e| query_err("reading an event's matches", e))?;
        for row in &stored {
            let key: String = row.get("tba_key");
            if keep.contains(key.as_str()) {
                continue;
            }
            let observed: i64 = row.get("observed");
            if observed > 0 {
                tidied.kept_matches.push((key, observed));
                continue;
            }
            tidied.removed_matches += sqlx::query("DELETE FROM matches WHERE tba_key = ?")
                .bind(&key)
                .execute(&mut *tx)
                .await
                .map_err(|e| query_err("removing a dropped match", e))?
                .rows_affected();
        }

        tidied.dropped_assignments = sqlx::query(STALE_ASSIGNMENTS.as_str())
            .bind(event_key)
            .execute(&mut *tx)
            .await
            .map_err(|e| query_err("dropping stale assignments", e))?
            .rows_affected();

        // A team off the roster stays linked while anything at the event
        // still names it: an observation, or a match kept above.
        let linked: Vec<i32> =
            sqlx::query_scalar("SELECT team_number FROM event_teams WHERE event_key = ?")
                .bind(event_key)
                .fetch_all(&mut *tx)
                .await
                .map_err(|e| query_err("reading an event's roster", e))?;
        for team in linked.into_iter().filter(|t| !roster.contains(t)) {
            let named: i64 = sqlx::query_scalar(STILL_NAMED.as_str())
                .bind(event_key)
                .bind(team)
                .fetch_one(&mut *tx)
                .await
                .map_err(|e| query_err("checking a team off the roster", e))?;
            if named == 0 {
                tidied.unlinked_teams +=
                    sqlx::query("DELETE FROM event_teams WHERE event_key = ? AND team_number = ?")
                        .bind(event_key)
                        .bind(team)
                        .execute(&mut *tx)
                        .await
                        .map_err(|e| query_err("unlinking a team", e))?
                        .rows_affected();
            }
        }

        tx.commit()
            .await
            .map_err(|e| query_err("finishing a schedule tidy", e))?;
        Ok(tidied)
    }
}
