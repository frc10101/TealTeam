//! Snapshots for a fresh device (S10): the server's database, cut down to
//! what one viewer may see, as a SQLite file to download and open as is.
//!
//! A device that has never synced should not replay the change log from
//! `seq = 0`. It downloads one of these instead, writes it to OPFS, and pulls
//! from the cursors inside it. No row-by-row inserts, no JSON, and no schema
//! to build on the client: the file *is* this build's schema, migrations
//! table and all.
//!
//! # How one is made
//!
//! 1. `VACUUM INTO` a scratch file, as a backup is taken (Q4). That is one
//!    read transaction, so the tables and both logs' heads agree. SQLite has
//!    one writer, so no change below the head can still be uncommitted, and
//!    the heads are exact cursors: no lag window, unlike the pull.
//! 2. Cut it down, on a connection of its own with foreign keys **off**, so
//!    emptying `users` cascades into nothing:
//!    - [`NEVER_REPLICATED`] tables are emptied but `users`, and so is every
//!      [`SERVER_ONLY`] one but two. `upstream` keeps the newest response per
//!      path in scope, which is the whole current state of it (S1).
//!      `sync_state` holds the snapshot's cursors, as `server:changes` and
//!      `server:upstream`: how far this copy has read the server.
//!    - Rows of every [`REPLICATED`] and [`FROM_UPSTREAM`] table with an
//!      `event_key` outside the scope go. `events` and `teams` stay whole:
//!      season lists go to everyone, as in the pull.
//!    - Pick lists that are not the viewer's team's go.
//!    - `users` keeps the scouts the rows left name, by id and name only,
//!      so an offline grid says "Sam", not "Scout 7" (S10b). The email is
//!      `#<id>`, the password hash empty, the roles off: nobody signs in
//!      to the copy, and nothing else about a person leaves the server.
//!    - Observations keep their answers as [`Audience::answers`] says,
//!      which is notes removed for anyone but the writing team (U13).
//! 3. `VACUUM` the scratch file, so nothing removed survives in a free page,
//!    in rollback-journal mode, since OPFS has no `-wal` beside the file.
//!
//! The rules are the pull's (`tt_web::sync::visible`), applied to rows
//! rather than changes. Which tables are cut how is [`crate::replication`]'s
//! lists, so a table added there is handled here without a second decision.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, Utc};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode};
use sqlx::{ConnectOptions, Connection, Row, SqliteConnection};
use tt_repo::{RepoError, Result};

use crate::SqliteRepo;
use crate::replication::{FROM_UPSTREAM, NEVER_REPLICATED, REPLICATED, SERVER_ONLY};
use crate::users::query_err;

/// The `sync_state` sources a snapshot's cursors are stored under.
pub const CHANGES_SOURCE: &str = "server:changes";
pub const UPSTREAM_SOURCE: &str = "server:upstream";

/// Who a snapshot is for. The rules are the pull's; this is how the caller,
/// which knows the viewer and the season, hands them over.
pub trait Audience {
    /// The viewer's team. Pick lists are that team's only, and none without.
    fn team(&self) -> Option<i32>;
    /// An event the client subscribed to (S3).
    fn has_event(&self, event_key: &str) -> bool;
    /// An upstream response's path, in scope.
    fn has_upstream(&self, path: &str) -> bool;
    /// An observation's answers as this viewer may read them, written by
    /// `writer`'s team; `None` to keep them as stored.
    fn answers(&self, writer: Option<i32>, payload: &str) -> Option<String>;
}

/// A finished snapshot.
#[derive(Debug, Clone)]
pub struct Snapshot {
    /// The database file.
    pub bytes: Vec<u8>,
    /// The `changes` cursor to pull from next.
    pub changes: i64,
    /// The `upstream` cursor to pull from next.
    pub upstream: i64,
    pub taken_at: DateTime<Utc>,
}

/// A file of this process's own, removed when dropped, with its journal.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        Self(std::env::temp_dir().join(format!(
            "tt-snapshot-{}-{}.sqlite",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )))
    }

    fn path(&self) -> Result<&str> {
        self.0.to_str().ok_or_else(|| {
            RepoError::Unavailable(format!("{} is not a UTF-8 path", self.0.display()))
        })
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
        let _ = std::fs::remove_file(self.0.with_extension("sqlite-journal"));
    }
}

/// Make a snapshot of `repo` for `audience`.
pub async fn build(repo: &SqliteRepo, audience: &(impl Audience + Sync)) -> Result<Snapshot> {
    let taken_at = Utc::now();
    let scratch = Scratch::new();
    sqlx::query("VACUUM INTO ?")
        .bind(scratch.path()?)
        .execute(repo.pool())
        .await
        .map_err(|e| query_err("copying the database for a snapshot", e))?;
    // An in-memory database copies into memory too, and leaves no file.
    if !scratch.0.exists() {
        return Err(RepoError::Unavailable(
            "only a database in a file can be snapshotted".into(),
        ));
    }

    let mut conn = open(&scratch.0).await?;
    let cut = cut(&mut conn, audience, taken_at).await;
    let _ = conn.close().await;
    let (changes, upstream) = cut?;

    let bytes = std::fs::read(&scratch.0)
        .map_err(|e| RepoError::Unavailable(format!("reading {}: {e}", scratch.0.display())))?;
    Ok(Snapshot {
        bytes,
        changes,
        upstream,
        taken_at,
    })
}

async fn open(path: &Path) -> Result<SqliteConnection> {
    SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(false)
        // Emptying users must not cascade into the assignments that name them.
        .foreign_keys(false)
        .journal_mode(SqliteJournalMode::Delete)
        .connect()
        .await
        .map_err(|e| RepoError::Unavailable(format!("opening {}: {e}", path.display())))
}

/// Step 2 and 3 of the module docs, on the scratch copy. Returns the cursors.
async fn cut(
    conn: &mut SqliteConnection,
    audience: &(impl Audience + Sync),
    now: DateTime<Utc>,
) -> Result<(i64, i64)> {
    let err = |what: &'static str| move |e: sqlx::Error| query_err(what, e);
    // Read before anything below writes: the redaction's updates fire the
    // change triggers on this copy.
    let (changes, upstream): (i64, i64) = sqlx::query_as(
        "SELECT (SELECT COALESCE(MAX(seq), 0) FROM changes), \
                (SELECT COALESCE(MAX(seq), 0) FROM upstream)",
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(err("reading a snapshot's cursors"))?;

    let mut tx = conn.begin().await.map_err(err("starting a snapshot"))?;

    // Events out of scope, from every table that names one.
    for table in REPLICATED.iter().chain(FROM_UPSTREAM) {
        let by_event: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pragma_table_info(?) WHERE name = 'event_key')",
        )
        .bind(table)
        .fetch_one(&mut *tx)
        .await
        .map_err(err("reading a table's columns"))?;
        if !by_event {
            continue;
        }
        // Table names from replication.rs, constants: nothing from outside.
        let keys: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT DISTINCT event_key FROM {table}"
        )))
        .fetch_all(&mut *tx)
        .await
        .map_err(err("reading a table's events"))?;
        for key in keys.iter().filter(|k| !audience.has_event(k)) {
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "DELETE FROM {table} WHERE event_key = ?"
            )))
            .bind(key)
            .execute(&mut *tx)
            .await
            .map_err(err("cutting an event from a snapshot"))?;
        }
    }

    // Other teams' pick lists, and every one for a viewer with no team.
    sqlx::query("DELETE FROM pick_list_entries WHERE owning_team IS NOT ?")
        .bind(audience.team())
        .execute(&mut *tx)
        .await
        .map_err(err("cutting pick lists from a snapshot"))?;

    // Answers as the viewer may read them.
    let rows = sqlx::query("SELECT id, submitting_team, payload FROM observations")
        .fetch_all(&mut *tx)
        .await
        .map_err(err("reading observations for a snapshot"))?;
    for row in rows {
        let (id, writer, payload): (i64, Option<i32>, String) =
            (row.get(0), row.get(1), row.get(2));
        if let Some(shown) = audience.answers(writer, &payload) {
            sqlx::query("UPDATE observations SET payload = ? WHERE id = ?")
                .bind(shown)
                .bind(id)
                .execute(&mut *tx)
                .await
                .map_err(err("redacting a snapshot's notes"))?;
        }
    }

    // The people the rows left name, by id and name, and nothing else about
    // them. The email must stay unique, so it is the id.
    sqlx::query(
        "DELETE FROM users WHERE id NOT IN ( \
             SELECT scouter_id FROM scout_assignments WHERE scouter_id IS NOT NULL \
             UNION SELECT assigned_by FROM scout_assignments WHERE assigned_by IS NOT NULL \
             UNION SELECT scouter_id FROM observations WHERE scouter_id IS NOT NULL \
             UNION SELECT reviewed_by FROM observations WHERE reviewed_by IS NOT NULL)",
    )
    .execute(&mut *tx)
    .await
    .map_err(err("cutting users from a snapshot"))?;
    sqlx::query(
        "UPDATE users SET email = '#' || id, password_hash = '', team_number = NULL, \
             is_admin = 0, is_lead_scout = 0, is_coach = 0, \
             last_login_at = NULL, last_seen_at = NULL",
    )
    .execute(&mut *tx)
    .await
    .map_err(err("cutting users from a snapshot"))?;

    // The upstream log: the newest response per path, in scope.
    sqlx::query(
        "DELETE FROM upstream WHERE seq NOT IN (SELECT MAX(seq) FROM upstream GROUP BY api, path)",
    )
    .execute(&mut *tx)
    .await
    .map_err(err("cutting the upstream log for a snapshot"))?;
    let paths: Vec<(i64, String)> = sqlx::query_as("SELECT seq, path FROM upstream")
        .fetch_all(&mut *tx)
        .await
        .map_err(err("reading the upstream log for a snapshot"))?;
    for (seq, _) in paths.iter().filter(|(_, p)| !audience.has_upstream(p)) {
        sqlx::query("DELETE FROM upstream WHERE seq = ?")
            .bind(seq)
            .execute(&mut *tx)
            .await
            .map_err(err("cutting the upstream log for a snapshot"))?;
    }

    // Last, so the triggers' rows from the redaction above go too.
    for table in NEVER_REPLICATED
        .iter()
        .chain(SERVER_ONLY)
        .filter(|t| !["users", "upstream"].contains(*t))
    {
        sqlx::query(sqlx::AssertSqlSafe(format!("DELETE FROM {table}")))
            .execute(&mut *tx)
            .await
            .map_err(err("emptying a table in a snapshot"))?;
    }
    for (source, cursor) in [(CHANGES_SOURCE, changes), (UPSTREAM_SOURCE, upstream)] {
        sqlx::query("INSERT INTO sync_state (source, cursor, applied_at) VALUES (?, ?, ?)")
            .bind(source)
            .bind(cursor)
            .bind(now.to_rfc3339())
            .execute(&mut *tx)
            .await
            .map_err(err("writing a snapshot's cursors"))?;
    }
    tx.commit().await.map_err(err("finishing a snapshot"))?;

    // Rewrite the file, so what was deleted is not still in it.
    sqlx::query("VACUUM")
        .execute(&mut *conn)
        .await
        .map_err(err("compacting a snapshot"))?;
    Ok((changes, upstream))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Team 10101 at 2026here, with notes hidden as `{"hidden":true}`.
    struct Ours;

    impl Audience for Ours {
        fn team(&self) -> Option<i32> {
            Some(10101)
        }
        fn has_event(&self, event_key: &str) -> bool {
            event_key == "2026here"
        }
        fn has_upstream(&self, path: &str) -> bool {
            !path.starts_with("/event/2026away")
        }
        fn answers(&self, writer: Option<i32>, _: &str) -> Option<String> {
            (writer != Some(10101)).then(|| r#"{"hidden":true}"#.into())
        }
    }

    /// A file, not memory: `VACUUM INTO` from an in-memory database writes
    /// to memory as well.
    async fn seeded(test: &str) -> SqliteRepo {
        let dir = std::env::temp_dir().join(format!("tt-snapshot-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let repo = SqliteRepo::connect(&format!("sqlite://{}", dir.join("pi.db").display()))
            .expect("connect");
        crate::migrate::apply(repo.pool()).await.expect("migrate");
        let t = "'2026-03-14T10:00:00Z'";
        for sql in [
            format!(
                "INSERT INTO users (id, email, name, password_hash, team_number, created_at, updated_at) \
                 VALUES (1, 'sam@x', 'Sam', 'SECRET-HASH', 10101, {t}, {t}), \
                        (2, 'pat@x', 'UNNAMED-PAT', 'SECRET-HASH', 10101, {t}, {t})"
            ),
            format!("INSERT INTO sessions VALUES ('SECRET-SESSION', 1, {t}, {t})"),
            format!(
                "INSERT INTO token_key VALUES (1, CAST('SECRET-TOKEN-KEY-0123456789abcde' AS BLOB), {t})"
            ),
            format!(
                "INSERT INTO devices (id, device_uuid, created_at, updated_at) VALUES (1, 'SECRET-DEVICE', {t}, {t})"
            ),
            format!(
                "INSERT INTO teams (team_number, name, created_at, updated_at) VALUES (10101, 'Teal', {t}, {t}), (254, 'Poofs', {t}, {t})"
            ),
            format!(
                "INSERT INTO events (tba_key, name, created_at, updated_at) VALUES ('2026here', 'Here', {t}, {t}), ('2026away', 'Away', {t}, {t})"
            ),
            format!(
                "INSERT INTO event_teams VALUES ('2026here', 254, {t}), ('2026away', 254, {t})"
            ),
            format!(
                "INSERT INTO matches (tba_key, event_key, comp_level, match_number, created_at, updated_at) \
                 VALUES ('2026here_qm1', '2026here', 'qm', 1, {t}, {t}), ('2026away_qm1', '2026away', 'qm', 1, {t}, {t})"
            ),
            format!(
                "INSERT INTO scout_assignments (match_key, team_number, event_key, scouter_id, created_at, updated_at) \
                 VALUES ('2026here_qm1', 254, '2026here', 1, {t}, {t})"
            ),
            format!(
                "INSERT INTO observations (client_record_id, match_key, team_number, event_key, alliance, payload, \
                 schema_version, scouter_id, submitting_team, observed_at, created_at, updated_at) VALUES \
                 ('ours', '2026here_qm1', 254, '2026here', 'red', '{{\"notes\":\"OUR-NOTE\"}}', 1, 1, 10101, {t}, {t}, {t}), \
                 ('theirs', '2026here_qm1', 254, '2026here', 'red', '{{\"notes\":\"THEIR-NOTE\"}}', 1, NULL, 254, {t}, {t}, {t}), \
                 ('away', '2026away_qm1', 254, '2026away', 'red', '{{}}', 1, NULL, 10101, {t}, {t}, {t})"
            ),
            format!(
                "INSERT INTO pick_list_entries (client_record_id, owning_team, event_key, picked_team, created_at, updated_at) \
                 VALUES ('our-pick', 10101, '2026here', 254, {t}, {t}), ('THEIR-PICK', 254, '2026here', 10101, {t}, {t})"
            ),
            "INSERT INTO upstream (api, path, body, fetched_at) VALUES \
             ('tba', '/event/2026here/rankings', '{\"old\":1}', 'x'), \
             ('tba', '/event/2026here/rankings', '{\"new\":1}', 'x'), \
             ('tba', '/event/2026away/rankings', '{\"away\":1}', 'x'), \
             ('tba', '/events/2026', '[]', 'x')"
                .into(),
        ] {
            sqlx::query(sqlx::AssertSqlSafe(sql))
                .execute(repo.pool())
                .await
                .expect("seed");
        }
        repo
    }

    async fn rows<T>(conn: &mut SqliteConnection, sql: &'static str) -> Vec<T>
    where
        T: Send + Unpin + for<'r> sqlx::Decode<'r, sqlx::Sqlite> + sqlx::Type<sqlx::Sqlite>,
    {
        sqlx::query_scalar(sql).fetch_all(conn).await.expect(sql)
    }

    #[tokio::test]
    async fn a_snapshot_holds_what_the_viewer_may_see_and_where_to_pull_from() {
        let repo = seeded("cut").await;
        let heads: (i64, i64) = sqlx::query_as(
            "SELECT (SELECT MAX(seq) FROM changes), (SELECT MAX(seq) FROM upstream)",
        )
        .fetch_one(repo.pool())
        .await
        .unwrap();

        let snap = build(&repo, &Ours).await.expect("snapshot");
        assert_eq!(
            (snap.changes, snap.upstream),
            heads,
            "cursors are the heads"
        );
        assert!(snap.bytes.starts_with(b"SQLite format 3\0"));
        assert_eq!(
            (snap.bytes[18], snap.bytes[19]),
            (1, 1),
            "rollback journal, not WAL: OPFS has no -wal file"
        );
        for secret in [
            "SECRET-HASH",
            "sam@x",
            "UNNAMED-PAT",
            "SECRET-SESSION",
            "SECRET-DEVICE",
            "SECRET-TOKEN-KEY",
            "THEIR-NOTE",
            "THEIR-PICK",
            "2026away_qm1",
            "\"old\"",
        ] {
            assert!(
                !snap
                    .bytes
                    .windows(secret.len())
                    .any(|w| w == secret.as_bytes()),
                "{secret} is nowhere in the file, not even a free page"
            );
        }

        let file =
            std::env::temp_dir().join(format!("tt-snapshot-test-{}.sqlite", std::process::id()));
        std::fs::write(&file, &snap.bytes).unwrap();
        let mut conn = SqliteConnectOptions::new()
            .filename(&file)
            .read_only(true)
            .connect()
            .await
            .expect("a client can open it");
        let ok: Vec<String> = rows(&mut conn, "PRAGMA integrity_check").await;
        assert_eq!(ok, ["ok"]);

        for empty in NEVER_REPLICATED.iter().chain(SERVER_ONLY) {
            if ["users", "upstream", "sync_state"].contains(empty) {
                continue;
            }
            let n: i64 =
                sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {empty}")))
                    .fetch_one(&mut conn)
                    .await
                    .unwrap();
            assert_eq!(n, 0, "{empty} is empty");
        }
        let events: Vec<String> = rows(&mut conn, "SELECT tba_key FROM events ORDER BY 1").await;
        assert_eq!(events, ["2026away", "2026here"], "the event list is whole");
        let matches: Vec<String> = rows(&mut conn, "SELECT tba_key FROM matches").await;
        assert_eq!(matches, ["2026here_qm1"]);
        let assigned: Vec<i64> = rows(&mut conn, "SELECT scouter_id FROM scout_assignments").await;
        assert_eq!(assigned, [1], "cutting users did not cascade");
        let users: Vec<String> = rows(
            &mut conn,
            "SELECT id || ' ' || name || ' ' || email || ' [' || password_hash || '] ' \
                    || (team_number IS NULL) || is_admin || is_lead_scout || is_coach \
             FROM users",
        )
        .await;
        assert_eq!(users, ["1 Sam #1 [] 1000"], "a name, and nothing else");
        let observed: Vec<String> = rows(
            &mut conn,
            "SELECT client_record_id || ' ' || payload FROM observations ORDER BY 1",
        )
        .await;
        assert_eq!(
            observed,
            [r#"ours {"notes":"OUR-NOTE"}"#, r#"theirs {"hidden":true}"#]
        );
        let picks: Vec<String> =
            rows(&mut conn, "SELECT client_record_id FROM pick_list_entries").await;
        assert_eq!(picks, ["our-pick"]);
        let upstream: Vec<String> = rows(&mut conn, "SELECT body FROM upstream ORDER BY seq").await;
        assert_eq!(
            upstream,
            [r#"{"new":1}"#, "[]"],
            "newest per path, in scope"
        );

        let cursors: Vec<String> = rows(
            &mut conn,
            "SELECT source || '=' || cursor FROM sync_state ORDER BY 1",
        )
        .await;
        assert_eq!(
            cursors,
            [
                format!("{CHANGES_SOURCE}={}", heads.0),
                format!("{UPSTREAM_SOURCE}={}", heads.1)
            ]
        );
        let _ = conn.close().await;
        let _ = std::fs::remove_file(&file);
    }

    #[tokio::test]
    async fn a_viewer_with_no_team_gets_no_pick_list_and_the_server_is_untouched() {
        struct Teamless;
        impl Audience for Teamless {
            fn team(&self) -> Option<i32> {
                None
            }
            fn has_event(&self, _: &str) -> bool {
                true
            }
            fn has_upstream(&self, _: &str) -> bool {
                true
            }
            fn answers(&self, _: Option<i32>, _: &str) -> Option<String> {
                None
            }
        }
        let repo = seeded("teamless").await;
        let snap = build(&repo, &Teamless).await.expect("snapshot");
        assert!(!snap.bytes.windows(8).any(|w| w == b"our-pick"));

        let left: (i64, i64) = sqlx::query_as(
            "SELECT (SELECT count(*) FROM users), (SELECT count(*) FROM pick_list_entries)",
        )
        .fetch_one(repo.pool())
        .await
        .unwrap();
        assert_eq!(left, (2, 2), "only the copy was cut");
    }
}
