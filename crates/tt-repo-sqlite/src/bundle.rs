//! Upstream bundles (S5): FIRST and TBA responses a client fetched with its
//! own signal, pushed to the Pi as a small SQLite file.
//!
//! # The format
//!
//! A bundle is a SQLite database with two tables:
//!
//! ```sql
//! CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
//! -- 'format' = '1'
//! -- 'log'    = the id of the client's upstream log: random, made once,
//! --            and new whenever the client's storage is wiped
//! CREATE TABLE upstream (
//!     seq        INTEGER PRIMARY KEY,  -- the client's own log order
//!     api        TEXT NOT NULL,        -- 'first' or 'tba'
//!     path       TEXT NOT NULL,        -- as the S1 log writes it
//!     etag       TEXT,
//!     body       TEXT NOT NULL,        -- the JSON as received
//!     fetched_at TEXT NOT NULL         -- RFC 3339, UTC
//! );
//! ```
//!
//! The `upstream` table is S1's, so a client keeping that log can send it
//! as it is. Other columns and tables are ignored.
//!
//! # The import
//!
//! The file is `ATTACH`ed, and in one transaction:
//!
//!   - Rows past this log's cursor in `sync_state` are read, oldest first.
//!   - Each is appended to the Pi's `upstream` log, through the same append
//!     as the Pi's own fetches, unless its body is already the newest for its
//!     path, or it was fetched no later than that newest. Upstream is
//!     last-write-wins by when it was fetched, so an old phone's bundle can
//!     never roll a ranking back. A time in the future is taken as now.
//!   - A row that is not a FIRST or TBA JSON response is refused and counted.
//!   - The cursor moves to the bundle's newest row, so the same bundle again
//!     imports nothing.
//!   - A `bundle_imports` row says who pushed it, from which tablet, and what
//!     it did. Appended rows carry `via = 'bundle:<that row's id>'`.
//!
//! Turning the appended responses into matches and stats is `tt-upstream`'s
//! job, with the same parsers as the Pi's own sync.

use std::path::Path;

use chrono::{DateTime, Utc};
use sqlx::{Connection, Row, SqliteConnection};
use tracing::warn;
use tt_repo::{NewUpstream, RepoError, Result};

use crate::SqliteRepo;
use crate::upstream::{Appended, append_in};
use crate::users::{from_sql, query_err, to_sql};

/// The bundle format this build reads.
pub const FORMAT: &str = "1";

/// Every SQLite file starts with this.
const MAGIC: &[u8] = b"SQLite format 3\0";

/// Who pushed a bundle.
#[derive(Debug, Clone, Copy)]
pub struct Pusher<'a> {
    pub user_id: i64,
    /// The pushing tablet's device id, when it sent one.
    pub device: Option<&'a str>,
}

/// What an import did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Imported {
    /// The `bundle_imports` row.
    pub id: i64,
    pub log: String,
    /// The cursor before and after: rows in `(from_seq, to_seq]` were read.
    pub from_seq: i64,
    pub to_seq: i64,
    /// `(api, path)` of every response with new content, in bundle order.
    pub appended: Vec<(String, String)>,
    pub unchanged: usize,
    pub stale: usize,
    pub refused: usize,
}

impl Imported {
    /// Rows past the cursor, whatever became of them.
    pub fn read(&self) -> usize {
        self.appended.len() + self.unchanged + self.stale + self.refused
    }
}

/// A row of the audit trail, for the lead scout's page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleImport {
    pub imported_at: DateTime<Utc>,
    /// The pusher's name, if the account still exists.
    pub user: Option<String>,
    /// The tablet's name, or the start of its id, when it sent one.
    pub device: Option<String>,
    pub appended: i64,
    pub unchanged: i64,
    pub stale: i64,
    pub refused: i64,
}

fn refused(what: impl Into<String>) -> RepoError {
    RepoError::Refused(what.into())
}

/// An error reading the attached file is the file's fault.
fn unreadable(e: sqlx::Error) -> RepoError {
    refused(format!("the bundle could not be read: {e}"))
}

impl SqliteRepo {
    /// Import the bundle at `file`. A file that is not a bundle is
    /// [`RepoError::Refused`], with the reason; nothing is written then.
    pub async fn import_bundle(
        &self,
        file: &Path,
        pusher: Pusher<'_>,
        now: DateTime<Utc>,
    ) -> Result<Imported> {
        let mut head = [0u8; MAGIC.len()];
        let looks_right = std::fs::File::open(file)
            .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut head))
            .is_ok()
            && head == MAGIC;
        if !looks_right {
            return Err(refused("that is not a SQLite file"));
        }
        let path = file
            .to_str()
            .ok_or_else(|| refused("the bundle's file name is not UTF-8"))?;

        let mut conn = self
            .pool()
            .acquire()
            .await
            .map_err(|e| query_err("importing a bundle", e))?;
        // ATTACH cannot run inside a transaction, so it brackets one. On an
        // in-memory database it would attach an empty in-memory one instead
        // of the file, so the tests use a real file too.
        sqlx::query("ATTACH DATABASE ? AS bundle")
            .bind(path)
            .execute(&mut *conn)
            .await
            .map_err(unreadable)?;
        let result = import_attached(&mut conn, pusher, now).await;
        if let Err(e) = sqlx::query("DETACH DATABASE bundle")
            .execute(&mut *conn)
            .await
        {
            // The pool's one connection must not keep a stranger's file
            // attached, or the next import could not attach its own.
            warn!("detaching a bundle: {e}; closing that connection");
            conn.close_on_drop();
        }
        result
    }

    /// The newest `limit` imports, newest first.
    pub async fn bundle_imports(&self, limit: i64) -> Result<Vec<BundleImport>> {
        let rows = sqlx::query(
            "SELECT b.imported_at, u.name AS user_name, b.device, d.name AS device_name, \
                    b.appended, b.unchanged, b.stale, b.refused \
             FROM bundle_imports b \
             LEFT JOIN users u ON u.id = b.user_id \
             LEFT JOIN devices d ON d.device_uuid = b.device \
             ORDER BY b.id DESC LIMIT ?",
        )
        .bind(limit)
        .fetch_all(self.pool())
        .await
        .map_err(|e| query_err("reading bundle imports", e))?;
        Ok(rows
            .iter()
            .map(|row| {
                let uuid: Option<String> = row.get("device");
                BundleImport {
                    imported_at: from_sql(&row.get::<String, _>("imported_at")).unwrap_or_default(),
                    user: row.get("user_name"),
                    device: row
                        .get::<Option<String>, _>("device_name")
                        .or_else(|| uuid.map(|u| u.chars().take(8).collect())),
                    appended: row.get("appended"),
                    unchanged: row.get("unchanged"),
                    stale: row.get("stale"),
                    refused: row.get("refused"),
                }
            })
            .collect())
    }
}

async fn meta(conn: &mut SqliteConnection, key: &str) -> Result<Option<String>> {
    sqlx::query_scalar("SELECT value FROM bundle.meta WHERE key = ?")
        .bind(key)
        .fetch_optional(&mut *conn)
        .await
        .map_err(unreadable)
}

async fn import_attached(
    conn: &mut SqliteConnection,
    pusher: Pusher<'_>,
    now: DateTime<Utc>,
) -> Result<Imported> {
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM bundle.sqlite_master \
         WHERE type = 'table' AND name IN ('meta', 'upstream')",
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(unreadable)?;
    if tables.len() != 2 {
        return Err(refused(
            "that is a SQLite file, but not a bundle: it needs `meta` and `upstream` tables",
        ));
    }
    match meta(conn, "format").await?.as_deref() {
        Some(FORMAT) => {}
        Some(other) => {
            return Err(refused(format!(
                "the bundle is format {other:?}; this server reads format {FORMAT:?}"
            )));
        }
        None => return Err(refused("the bundle does not say its format")),
    }
    let log = meta(conn, "log")
        .await?
        .filter(|l| (1..=64).contains(&l.len()) && l.chars().all(|c| c.is_ascii_graphic()))
        .ok_or_else(|| refused("the bundle does not name its log"))?;
    let source = format!("bundle:{log}");

    let mut tx = conn
        .begin()
        .await
        .map_err(|e| query_err("importing a bundle", e))?;

    let from_seq: i64 = sqlx::query_scalar("SELECT cursor FROM sync_state WHERE source = ?")
        .bind(&source)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| query_err("reading the bundle cursor", e))?
        .unwrap_or(0);

    let rows = sqlx::query(
        "SELECT seq, api, path, etag, body, fetched_at, json_valid(body) AS is_json \
         FROM bundle.upstream WHERE seq > ? ORDER BY seq",
    )
    .bind(from_seq)
    .fetch_all(&mut *tx)
    .await
    .map_err(unreadable)?;

    let id: i64 = sqlx::query_scalar(
        "INSERT INTO bundle_imports (imported_at, user_id, device, log, from_seq, to_seq, \
                                     appended, unchanged, stale, refused) \
         VALUES (?, ?, ?, ?, ?, ?, 0, 0, 0, 0) RETURNING id",
    )
    .bind(to_sql(now))
    .bind(pusher.user_id)
    .bind(pusher.device)
    .bind(&log)
    .bind(from_seq)
    .bind(from_seq)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| query_err("recording a bundle import", e))?;

    let mut imported = Imported {
        id,
        log,
        from_seq,
        to_seq: from_seq,
        ..Default::default()
    };
    let via = format!("bundle:{id}");
    for row in &rows {
        let seq: i64 = row.try_get("seq").map_err(unreadable)?;
        imported.to_seq = imported.to_seq.max(seq);
        let Some(entry) = readable(row, &via, now) else {
            imported.refused += 1;
            continue;
        };
        match append_in(&mut tx, &entry, true).await? {
            Appended::New(_) => imported.appended.push((entry.api, entry.path)),
            Appended::Unchanged => imported.unchanged += 1,
            Appended::Stale => imported.stale += 1,
        }
    }

    sqlx::query(
        "INSERT INTO sync_state (source, cursor, applied_at) VALUES (?, ?, ?) \
         ON CONFLICT (source) DO UPDATE SET cursor = excluded.cursor, \
                                            applied_at = excluded.applied_at",
    )
    .bind(&source)
    .bind(imported.to_seq)
    .bind(to_sql(now))
    .execute(&mut *tx)
    .await
    .map_err(|e| query_err("advancing the bundle cursor", e))?;

    sqlx::query(
        "UPDATE bundle_imports SET to_seq = ?, appended = ?, unchanged = ?, stale = ?, \
                                   refused = ? WHERE id = ?",
    )
    .bind(imported.to_seq)
    .bind(imported.appended.len() as i64)
    .bind(imported.unchanged as i64)
    .bind(imported.stale as i64)
    .bind(imported.refused as i64)
    .bind(id)
    .execute(&mut *tx)
    .await
    .map_err(|e| query_err("recording a bundle import", e))?;

    tx.commit()
        .await
        .map_err(|e| query_err("importing a bundle", e))?;
    Ok(imported)
}

/// A bundle row as a log entry, or `None` when it is not a FIRST or TBA JSON
/// response with a time this server can read.
fn readable(row: &sqlx::sqlite::SqliteRow, via: &str, now: DateTime<Utc>) -> Option<NewUpstream> {
    let api: String = row.try_get("api").ok()?;
    let path: String = row.try_get("path").ok()?;
    let body: String = row.try_get("body").ok()?;
    let is_json: bool = row.try_get("is_json").ok()?;
    let fetched_at = from_sql(&row.try_get::<String, _>("fetched_at").ok()?)?;
    let sound = matches!(api.as_str(), "first" | "tba")
        && path.starts_with('/')
        && path.len() <= 512
        && is_json;
    sound.then(|| NewUpstream {
        api,
        path,
        etag: row.try_get("etag").ok().flatten(),
        body,
        // A tablet's clock can be ahead (S12); a future time would make the
        // row outrank every fetch until then.
        fetched_at: fetched_at.min(now),
        via: via.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use chrono::{TimeDelta, TimeZone};
    use sqlx::ConnectOptions;
    use tt_repo::Repo;

    use super::*;

    /// A test's own directory, removed when it ends.
    struct Scratch(PathBuf);

    impl Scratch {
        fn file(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A file-backed Pi: attached to an in-memory database, a file would be
    /// opened in memory too.
    async fn repo(test: &str) -> (Scratch, SqliteRepo) {
        let dir = std::env::temp_dir().join(format!("tt-bundle-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let scratch = Scratch(dir);
        let url = format!("sqlite://{}", scratch.file("pi.db").display());
        let repo = SqliteRepo::connect(&url).expect("connect");
        crate::migrate::apply(repo.pool()).await.expect("migrate");
        sqlx::query(
            "INSERT INTO users (id, email, name, password_hash, created_at, updated_at) \
             VALUES (1, 'lead@example.com', 'Ana', 'x', '2026-03-14T00:00:00Z', '2026-03-14T00:00:00Z')",
        )
        .execute(repo.pool())
        .await
        .expect("user");
        (scratch, repo)
    }

    fn at(minute: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 3, 14, 12, 0, 0).unwrap() + TimeDelta::minutes(minute)
    }

    const LEAD: Pusher<'static> = Pusher {
        user_id: 1,
        device: Some("tablet-0001"),
    };

    /// A plain single-file database, as `VACUUM INTO` writes one.
    async fn open(file: &Path) -> SqliteConnection {
        sqlx::sqlite::SqliteConnectOptions::new()
            .filename(file)
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Delete)
            .connect()
            .await
            .unwrap()
    }

    /// A bundle file holding `rows` of `(seq, api, path, body, minute)`.
    async fn bundle(
        scratch: &Scratch,
        name: &str,
        log: &str,
        rows: &[(i64, &str, &str, &str, i64)],
    ) -> PathBuf {
        let file = scratch.file(&format!("{name}.sqlite"));
        {
            let mut conn = open(&file).await;
            sqlx::raw_sql(
                "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL); \
                 CREATE TABLE upstream (seq INTEGER PRIMARY KEY, api TEXT NOT NULL, \
                   path TEXT NOT NULL, etag TEXT, body TEXT NOT NULL, fetched_at TEXT NOT NULL);",
            )
            .execute(&mut conn)
            .await
            .unwrap();
            sqlx::query("INSERT INTO meta VALUES ('format', '1'), ('log', ?)")
                .bind(log)
                .execute(&mut conn)
                .await
                .unwrap();
            for (seq, api, path, body, minute) in rows {
                sqlx::query("INSERT INTO upstream VALUES (?, ?, ?, NULL, ?, ?)")
                    .bind(seq)
                    .bind(api)
                    .bind(path)
                    .bind(body)
                    .bind(to_sql(at(*minute)))
                    .execute(&mut conn)
                    .await
                    .unwrap();
            }
            conn.close().await.unwrap();
        }
        file
    }

    const RANKINGS: &str = "/event/2026mslr/rankings";

    #[tokio::test]
    async fn a_bundle_lands_in_the_log_once_with_who_pushed_it() {
        let (scratch, repo) = repo("once").await;
        let bundle = bundle(
            &scratch,
            "once",
            "phone-a",
            &[
                (1, "tba", RANKINGS, "[1]", 0),
                (2, "tba", "/event/2026mslr/matches", "[]", 0),
            ],
        )
        .await;

        let first = repo.import_bundle(&bundle, LEAD, at(5)).await.unwrap();
        assert_eq!(first.appended.len(), 2);
        assert_eq!((first.from_seq, first.to_seq), (0, 2));
        let log = repo.upstream_since(0, 10).await.unwrap();
        assert_eq!(log.len(), 2);
        assert_eq!(log[0].entry.via, format!("bundle:{}", first.id));
        assert_eq!(
            log[0].entry.fetched_at,
            at(0),
            "the phone's time, not the push's"
        );

        let again = repo.import_bundle(&bundle, LEAD, at(6)).await.unwrap();
        assert_eq!(again.read(), 0, "the cursor is past everything");
        assert_eq!((again.from_seq, again.to_seq), (2, 2));
        assert_eq!(repo.upstream_since(0, 10).await.unwrap().len(), 2);

        let audit = repo.bundle_imports(10).await.unwrap();
        assert_eq!(audit.len(), 2, "every push is recorded, even an empty one");
        assert_eq!(audit[1].user.as_deref(), Some("Ana"));
        assert_eq!(audit[1].device.as_deref(), Some("tablet-0"));
        assert_eq!(audit[1].appended, 2);
    }

    #[tokio::test]
    async fn an_older_fetch_never_replaces_a_newer_one() {
        let (scratch, repo) = repo("stale").await;
        repo.append_upstream(&NewUpstream {
            api: "tba".into(),
            path: RANKINGS.into(),
            etag: None,
            body: "[\"pi\"]".into(),
            fetched_at: at(10),
            via: "pi".into(),
        })
        .await
        .unwrap();
        let bundle = bundle(
            &scratch,
            "stale",
            "phone-b",
            &[
                (1, "tba", RANKINGS, "[\"phone, earlier\"]", 5),
                (2, "tba", RANKINGS, "[\"pi\"]", 12),
                (3, "tba", RANKINGS, "[\"phone, later\"]", 15),
            ],
        )
        .await;

        let imported = repo.import_bundle(&bundle, LEAD, at(20)).await.unwrap();
        assert_eq!(imported.stale, 1);
        assert_eq!(imported.unchanged, 1);
        assert_eq!(imported.appended.len(), 1);
        let newest = repo
            .latest_upstream("tba", RANKINGS)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(newest.entry.body, "[\"phone, later\"]");
    }

    #[tokio::test]
    async fn rows_that_are_not_responses_are_refused_and_the_rest_imported() {
        let (scratch, repo) = repo("refused").await;
        let bundle = bundle(
            &scratch,
            "refused",
            "phone-c",
            &[
                (1, "espn", "/scores", "{}", 0),
                (2, "tba", RANKINGS, "not json", 0),
                (3, "tba", "no-slash", "{}", 0),
                (4, "tba", RANKINGS, "{}", 60 * 24),
            ],
        )
        .await;

        let imported = repo.import_bundle(&bundle, LEAD, at(1)).await.unwrap();
        assert_eq!(imported.refused, 3);
        assert_eq!(imported.appended.len(), 1);
        assert_eq!(imported.to_seq, 4);
        let newest = repo
            .latest_upstream("tba", RANKINGS)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(newest.entry.fetched_at, at(1), "tomorrow is taken as now");
    }

    #[tokio::test]
    async fn each_log_has_its_own_cursor() {
        let (scratch, repo) = repo("cursors").await;
        let a = bundle(
            &scratch,
            "log-a",
            "phone-a",
            &[(7, "tba", RANKINGS, "[1]", 0)],
        )
        .await;
        let b = bundle(
            &scratch,
            "log-b",
            "phone-b",
            &[(1, "tba", RANKINGS, "[2]", 1)],
        )
        .await;
        assert_eq!(repo.import_bundle(&a, LEAD, at(2)).await.unwrap().to_seq, 7);
        let other = repo.import_bundle(&b, LEAD, at(2)).await.unwrap();
        assert_eq!(other.from_seq, 0, "a wiped phone's new log starts over");
        assert_eq!(other.appended.len(), 1);
    }

    #[tokio::test]
    async fn a_file_that_is_not_a_bundle_is_refused_and_writes_nothing() {
        let (scratch, repo) = repo("not-a-bundle").await;
        let text = scratch.file("text.sqlite");
        std::fs::write(&text, "hello").unwrap();
        let err = repo.import_bundle(&text, LEAD, at(0)).await.unwrap_err();
        assert!(matches!(err, RepoError::Refused(_)), "{err}");

        let plain = scratch.file("plain.sqlite");
        let mut conn = open(&plain).await;
        sqlx::query("CREATE TABLE notes (x)")
            .execute(&mut conn)
            .await
            .unwrap();
        conn.close().await.unwrap();
        let err = repo.import_bundle(&plain, LEAD, at(0)).await.unwrap_err();
        assert!(err.to_string().contains("not a bundle"), "{err}");

        let future = bundle(&scratch, "future", "phone-d", &[]).await;
        sqlx::query("UPDATE meta SET value = '2' WHERE key = 'format'")
            .execute(&mut open(&future).await)
            .await
            .unwrap();
        let err = repo.import_bundle(&future, LEAD, at(0)).await.unwrap_err();
        assert!(err.to_string().contains("format \"2\""), "{err}");

        assert!(repo.bundle_imports(10).await.unwrap().is_empty());
        // And the one connection is still usable, with nothing left attached.
        let good = bundle(
            &scratch,
            "after",
            "phone-e",
            &[(1, "tba", RANKINGS, "[]", 0)],
        )
        .await;
        assert_eq!(
            repo.import_bundle(&good, LEAD, at(1))
                .await
                .unwrap()
                .appended
                .len(),
            1
        );
    }
}
