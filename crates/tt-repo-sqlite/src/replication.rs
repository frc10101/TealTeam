//! Which tables reach clients, and how (S3).
//!
//! Every table is in exactly one list, and a test fails the build if a new
//! one is not, or if a table outside [`REPLICATED`] gets a trigger writing to
//! `changes`. Replicating a table is a decision made here, by name, never a
//! side effect of a migration.
//!
//! A fresh device's snapshot (S10, [`crate::snapshot`]) is cut by the same
//! lists: what is not [`REPLICATED`] or [`FROM_UPSTREAM`] is emptied.

/// Written by scouts and leads; every change goes to clients through the
/// `changes` log (S2), filtered by `sync::visible`.
pub const REPLICATED: &[&str] = &["observations", "scout_assignments", "pick_list_entries"];

/// Never to leave the server: password hashes, session tokens, which person
/// holds which tablet, and the key that signs offline tokens (C9).
pub const NEVER_REPLICATED: &[&str] = &["users", "sessions", "devices", "token_key"];

/// Derived from FIRST and TBA. Clients get the responses they came from,
/// through the `upstream` log (S1), and derive the same rows.
pub const FROM_UPSTREAM: &[&str] = &[
    "events",
    "teams",
    "event_teams",
    "matches",
    "team_event_stats",
];

/// The server's own: the two logs themselves, settings not replicated yet,
/// and the bundle cursors and audit trail (S5). The point weights (L12) will
/// want to join [`REPLICATED`] once clients compute rankings.
///
/// The pick list documents (L14) do reach clients, but not as rows: a lead's
/// copy is exchanged whole through `/api/pick-list/doc`, which checks whose
/// list it is, since a yrs update is merged, not replayed. Their rows, in
/// `pick_list_entries`, replicate as usual.
///
/// Refused outbox entries (C10) are for the lead scout's page. Whatever a lead
/// makes of one reaches devices as an ordinary observation.
pub const SERVER_ONLY: &[&str] = &[
    "changes",
    "upstream",
    "scouting_point_weights",
    "pick_list_docs",
    "sync_state",
    "bundle_imports",
    "refused_entries",
];

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::SqliteRepo;

    #[tokio::test]
    async fn every_table_is_classified_and_only_the_replicated_ones_feed_changes() {
        let repo = SqliteRepo::connect("sqlite::memory:").expect("connect");
        crate::migrate::apply(repo.pool()).await.expect("migrate");

        let tables: BTreeSet<String> = sqlx::query_scalar(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' \
             AND name NOT LIKE '\\_%' ESCAPE '\\'",
        )
        .fetch_all(repo.pool())
        .await
        .unwrap()
        .into_iter()
        .collect();
        let lists = [REPLICATED, NEVER_REPLICATED, FROM_UPSTREAM, SERVER_ONLY];
        for table in &tables {
            let homes = lists.iter().filter(|l| l.contains(&table.as_str())).count();
            assert_eq!(
                homes, 1,
                "{table} must be in exactly one list in replication.rs, not {homes}"
            );
        }
        for name in lists.concat() {
            assert!(tables.contains(name), "{name} is listed but does not exist");
        }

        let fed: BTreeSet<String> = sqlx::query_scalar(
            "SELECT DISTINCT tbl_name FROM sqlite_master \
             WHERE type = 'trigger' AND sql LIKE '%INSERT INTO changes%'",
        )
        .fetch_all(repo.pool())
        .await
        .unwrap()
        .into_iter()
        .collect();
        let replicated: BTreeSet<String> = REPLICATED.iter().map(|t| t.to_string()).collect();
        assert_eq!(
            fed, replicated,
            "only REPLICATED tables may write to changes"
        );
    }
}
