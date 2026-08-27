//! Migration runner (D11).
//!
//! # The bug this is designed against
//!
//! The retired implementation defaulted `TEALTEAM_ENV` to `test`, and `test`
//! dropped the migration-history table on boot while the first migration opened
//! with `DROP TABLE ... CASCADE` on every table. One unset environment variable
//! erased an event's scouting data (REBUILD_SPEC.md 12.10).
//!
//! So the rules here are:
//!
//!   * [`apply`] only ever moves **forward**. There is no code path in it that
//!     can drop anything. It is what runs at startup, always.
//!   * [`reset`] destroys data, is never called by startup, and requires the
//!     caller to have already checked an explicit opt-in.
//!
//! Migrations are embedded at compile time, so the deployed binary carries its
//! own schema and there is no `migrations/` directory to lose on the Pi.

use sqlx::SqlitePool;
use sqlx::migrate::Migrator;
use tracing::{info, warn};
use tt_repo::{RepoError, Result};

/// Embedded migrations, applied in filename order and checksummed by sqlx --
/// editing an already-applied migration is detected rather than silently
/// diverging between machines.
pub static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

/// Apply any migrations the database has not seen. Forward only.
///
/// Safe to call on every boot, including when nothing has changed.
pub async fn apply(pool: &SqlitePool) -> Result<()> {
    let before = applied_count(pool).await;

    MIGRATOR.run(pool).await.map_err(|e| {
        // The most common cause by far is an edited migration failing its
        // checksum, so say that out loud rather than surfacing sqlx's wording.
        RepoError::Query(format!(
            "migration failed ({e}). If a migration file was edited after being \
             applied, revert it and add a new one instead."
        ))
    })?;

    let after = applied_count(pool).await;
    match after.saturating_sub(before) {
        0 => info!("schema up to date ({after} migration(s) applied)"),
        n => info!("applied {n} migration(s), schema now at {after}"),
    }
    Ok(())
}

/// Drop every table and re-apply from scratch. **Destroys all data.**
///
/// Never called by startup. The caller must have verified an explicit opt-in
/// (`TEALTEAM_ENV=dev`); this function does not check, it only warns loudly, so
/// that the decision lives at the call site where a reviewer will see it.
pub async fn reset(pool: &SqlitePool) -> Result<()> {
    warn!("RESETTING DATABASE -- all data will be destroyed");

    MIGRATOR
        .undo(pool, 0)
        .await
        .map_err(|e| RepoError::Query(format!("reverting migrations: {e}")))?;

    apply(pool).await
}

async fn applied_count(pool: &SqlitePool) -> i64 {
    // Returns 0 before the history table exists, which is the honest answer.
    sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations")
        .fetch_one(pool)
        .await
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SqliteRepo;

    async fn fresh() -> SqliteRepo {
        let repo = SqliteRepo::connect("sqlite::memory:").expect("connect");
        apply(repo.pool()).await.expect("migrate");
        repo
    }

    #[tokio::test]
    async fn migrations_apply_to_an_empty_database() {
        let repo = fresh().await;
        let tables: Vec<String> = sqlx::query_scalar(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' \
             AND name NOT LIKE '\\_%' ESCAPE '\\' ORDER BY name",
        )
        .fetch_all(repo.pool())
        .await
        .expect("list tables");

        assert_eq!(
            tables,
            vec![
                "devices",
                "event_teams",
                "events",
                "matches",
                "observations",
                "pick_list_entries",
                "scout_assignments",
                "scouting_point_weights",
                "sessions",
                "team_event_stats",
                "teams",
                "users",
            ]
        );
    }

    #[tokio::test]
    async fn dead_tables_from_the_retired_schema_are_not_recreated() {
        // awards and zebra_data existed for years and nothing ever wrote them.
        let repo = fresh().await;
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE name IN ('awards', 'zebra_data')",
        )
        .fetch_one(repo.pool())
        .await
        .expect("query");
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn applying_twice_is_a_no_op() {
        let repo = fresh().await;
        apply(repo.pool()).await.expect("second apply must succeed");
    }

    #[tokio::test]
    async fn tables_are_strict() {
        // STRICT rejects a string in an INTEGER column instead of silently
        // storing it, which is worth having on a schema students will extend.
        let repo = fresh().await;
        let now = "2026-03-14T12:00:00Z";
        let result = sqlx::query(
            "INSERT INTO teams (team_number, name, created_at, updated_at) \
             VALUES ('not a number', 'X', ?, ?)",
        )
        .bind(now)
        .bind(now)
        .execute(repo.pool())
        .await;
        assert!(result.is_err(), "STRICT should reject a non-integer key");
    }

    #[tokio::test]
    async fn email_uniqueness_ignores_case() {
        let repo = fresh().await;
        let now = "2026-03-14T12:00:00Z";
        let insert = |email: &'static str| {
            sqlx::query(
                "INSERT INTO users (email, name, password_hash, created_at, updated_at) \
                 VALUES (?, 'Scout', 'x', ?, ?)",
            )
            .bind(email)
            .bind(now)
            .bind(now)
            .execute(repo.pool())
        };

        insert("Scout@example.com").await.expect("first insert");
        assert!(
            insert("scout@example.com").await.is_err(),
            "differing only in case must collide"
        );
    }

    #[tokio::test]
    async fn an_assignment_needs_a_scout_or_a_device() {
        let repo = fresh().await;
        seed_match(&repo).await;
        let now = "2026-03-14T12:00:00Z";

        let result = sqlx::query(
            "INSERT INTO scout_assignments \
               (match_key, team_number, event_key, created_at, updated_at) \
             VALUES ('2026test_qm1', 10101, '2026test', ?, ?)",
        )
        .bind(now)
        .bind(now)
        .execute(repo.pool())
        .await;

        assert!(result.is_err(), "CHECK should reject an unassigned row");
    }

    #[tokio::test]
    async fn observations_reject_an_unknown_review_state() {
        let repo = fresh().await;
        seed_match(&repo).await;
        assert!(
            insert_observation(&repo, "rejected", Some(1))
                .await
                .is_err()
        );
        assert!(insert_observation(&repo, "declined", Some(1)).await.is_ok());
    }

    #[tokio::test]
    async fn a_scout_cannot_double_submit_for_one_robot_in_one_match() {
        let repo = fresh().await;
        seed_match(&repo).await;

        insert_observation(&repo, "pending", Some(1))
            .await
            .expect("first");
        assert!(
            insert_observation(&repo, "pending", Some(1)).await.is_err(),
            "second live observation from the same scout must collide"
        );
    }

    #[tokio::test]
    async fn a_declined_observation_does_not_block_a_corrected_resubmission() {
        // The coverage index excludes declined rows precisely so this works.
        let repo = fresh().await;
        seed_match(&repo).await;

        insert_observation(&repo, "declined", Some(1))
            .await
            .expect("declined");
        insert_observation(&repo, "pending", Some(1))
            .await
            .expect("resubmission after a decline must be allowed");
    }

    #[tokio::test]
    async fn deleting_an_event_cascades_to_its_matches_and_observations() {
        let repo = fresh().await;
        seed_match(&repo).await;
        insert_observation(&repo, "approved", Some(1))
            .await
            .expect("observation");

        sqlx::query("DELETE FROM events WHERE tba_key = '2026test'")
            .execute(repo.pool())
            .await
            .expect("delete event");

        let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM observations")
            .fetch_one(repo.pool())
            .await
            .expect("count");
        assert_eq!(remaining, 0, "foreign keys must actually cascade");
    }

    // ── Fixtures ────────────────────────────────────────────────────────────

    const NOW: &str = "2026-03-14T12:00:00Z";

    async fn seed_match(repo: &SqliteRepo) {
        let pool = repo.pool();
        sqlx::query("INSERT INTO teams (team_number, name, created_at, updated_at) VALUES (10101, 'Teal Team', ?, ?)")
            .bind(NOW).bind(NOW).execute(pool).await.expect("team");
        sqlx::query("INSERT INTO events (tba_key, name, created_at, updated_at) VALUES ('2026test', 'Test Event', ?, ?)")
            .bind(NOW).bind(NOW).execute(pool).await.expect("event");
        sqlx::query(
            "INSERT INTO matches (tba_key, event_key, comp_level, set_number, match_number, created_at, updated_at) \
             VALUES ('2026test_qm1', '2026test', 'qm', 1, 1, ?, ?)",
        ).bind(NOW).bind(NOW).execute(pool).await.expect("match");
        sqlx::query("INSERT INTO users (id, email, name, password_hash, created_at, updated_at) VALUES (1, 'a@b.c', 'Scout', 'x', ?, ?)")
            .bind(NOW).bind(NOW).execute(pool).await.expect("user");
    }

    async fn insert_observation(
        repo: &SqliteRepo,
        state: &str,
        scouter: Option<i64>,
    ) -> std::result::Result<sqlx::sqlite::SqliteQueryResult, sqlx::Error> {
        sqlx::query(
            "INSERT INTO observations \
               (client_record_id, match_key, team_number, event_key, alliance, payload, \
                schema_version, scouter_id, review_state, observed_at, created_at, updated_at) \
             VALUES (?, '2026test_qm1', 10101, '2026test', 'red', '{}', 1, ?, ?, ?, ?, ?)",
        )
        .bind(uuid_like())
        .bind(scouter)
        .bind(state)
        .bind(NOW)
        .bind(NOW)
        .bind(NOW)
        .execute(repo.pool())
        .await
    }

    /// Unique-enough stand-in; real ids are UUIDv7 from the client.
    fn uuid_like() -> String {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        format!("test-{:016x}", N.fetch_add(1, Ordering::Relaxed))
    }
}
