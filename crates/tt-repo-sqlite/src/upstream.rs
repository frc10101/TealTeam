//! The upstream log (S1): FIRST and TBA responses, as received.
//!
//! An append skips a body identical to the newest for its path, and prunes the
//! path to its newest [`UPSTREAM_KEEP_PER_PATH`], in the same transaction.
//! Both keep the log's size proportional to the number of distinct requests,
//! not to how long the server has been polling them.

use chrono::{DateTime, Utc};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqliteConnection};
use tt_repo::{Change, NewUpstream, Result, UPSTREAM_KEEP_PER_PATH, UpstreamEntry};

use crate::SqliteRepo;
use crate::users::{from_sql, query_err, to_sql};

/// What an append did with a response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Appended {
    New(i64),
    /// The same body as the newest for its path.
    Unchanged,
    /// Fetched no later than the newest for its path. Only a bundle's rows are
    /// checked for this: the Pi's own fetches are the newest by definition.
    Stale,
}

/// Append `entry` and prune its path, inside the caller's transaction.
pub(crate) async fn append_in(
    conn: &mut SqliteConnection,
    entry: &NewUpstream,
    refuse_stale: bool,
) -> Result<Appended> {
    let newest: Option<(String, String)> = sqlx::query_as(
        "SELECT body, fetched_at FROM upstream WHERE api = ? AND path = ? \
         ORDER BY seq DESC LIMIT 1",
    )
    .bind(&entry.api)
    .bind(&entry.path)
    .fetch_optional(&mut *conn)
    .await
    .map_err(|e| query_err("reading the upstream log", e))?;
    if let Some((body, fetched_at)) = &newest {
        if *body == entry.body {
            return Ok(Appended::Unchanged);
        }
        if refuse_stale && from_sql(fetched_at).is_some_and(|newest| newest >= entry.fetched_at) {
            return Ok(Appended::Stale);
        }
    }

    let seq: i64 = sqlx::query_scalar(
        "INSERT INTO upstream (api, path, etag, body, fetched_at, via) \
         VALUES (?, ?, ?, ?, ?, ?) RETURNING seq",
    )
    .bind(&entry.api)
    .bind(&entry.path)
    .bind(&entry.etag)
    .bind(&entry.body)
    .bind(to_sql(entry.fetched_at))
    .bind(&entry.via)
    .fetch_one(&mut *conn)
    .await
    .map_err(|e| query_err("appending to the upstream log", e))?;

    sqlx::query(
        "DELETE FROM upstream WHERE api = ? AND path = ? AND seq NOT IN \
         (SELECT seq FROM upstream WHERE api = ? AND path = ? ORDER BY seq DESC LIMIT ?)",
    )
    .bind(&entry.api)
    .bind(&entry.path)
    .bind(&entry.api)
    .bind(&entry.path)
    .bind(UPSTREAM_KEEP_PER_PATH)
    .execute(&mut *conn)
    .await
    .map_err(|e| query_err("pruning the upstream log", e))?;

    Ok(Appended::New(seq))
}

fn entry_from(row: &SqliteRow) -> UpstreamEntry {
    UpstreamEntry {
        seq: row.get("seq"),
        entry: NewUpstream {
            api: row.get("api"),
            path: row.get("path"),
            etag: row.get("etag"),
            body: row.get("body"),
            fetched_at: from_sql(&row.get::<String, _>("fetched_at")).unwrap_or_default(),
            via: row.get("via"),
        },
    }
}

impl SqliteRepo {
    pub(crate) async fn append_upstream_impl(&self, entry: &NewUpstream) -> Result<Option<i64>> {
        let mut tx = self
            .pool()
            .begin()
            .await
            .map_err(|e| query_err("appending to the upstream log", e))?;
        let appended = append_in(&mut tx, entry, false).await?;
        tx.commit()
            .await
            .map_err(|e| query_err("appending to the upstream log", e))?;
        Ok(match appended {
            Appended::New(seq) => Some(seq),
            Appended::Unchanged | Appended::Stale => None,
        })
    }

    pub(crate) async fn latest_upstream_impl(
        &self,
        api: &str,
        path: &str,
    ) -> Result<Option<UpstreamEntry>> {
        let row = sqlx::query(
            "SELECT seq, api, path, etag, body, fetched_at, via FROM upstream \
             WHERE api = ? AND path = ? ORDER BY seq DESC LIMIT 1",
        )
        .bind(api)
        .bind(path)
        .fetch_optional(self.pool())
        .await
        .map_err(|e| query_err("reading the upstream log", e))?;
        Ok(row.as_ref().map(entry_from))
    }

    pub(crate) async fn upstream_since_impl(
        &self,
        after: i64,
        limit: i64,
    ) -> Result<Vec<UpstreamEntry>> {
        let rows = sqlx::query(
            "SELECT seq, api, path, etag, body, fetched_at, via FROM upstream \
             WHERE seq > ? ORDER BY seq LIMIT ?",
        )
        .bind(after)
        .bind(limit)
        .fetch_all(self.pool())
        .await
        .map_err(|e| query_err("reading the upstream log", e))?;
        Ok(rows.iter().map(entry_from).collect())
    }
}

impl SqliteRepo {
    pub(crate) async fn log_heads_impl(&self) -> Result<(i64, i64)> {
        let heads: (i64, i64) = sqlx::query_as(
            "SELECT (SELECT COALESCE(MAX(seq), 0) FROM changes), \
                    (SELECT COALESCE(MAX(seq), 0) FROM upstream)",
        )
        .fetch_one(self.pool())
        .await
        .map_err(|e| query_err("reading the log heads", e))?;
        Ok(heads)
    }

    /// The venue stream (S2). The log is written by triggers
    /// (`migrations/0004_changes.sql`); this only reads it.
    pub(crate) async fn changes_since_impl(
        &self,
        after: i64,
        limit: i64,
        settled_before: DateTime<Utc>,
    ) -> Result<Vec<Change>> {
        let rows = sqlx::query(
            "SELECT seq, entity, entity_pk, op, payload, event_key, team_scope, created_at \
             FROM changes WHERE seq > ? AND created_at < ? ORDER BY seq LIMIT ?",
        )
        .bind(after)
        .bind(to_sql(settled_before))
        .bind(limit)
        .fetch_all(self.pool())
        .await
        .map_err(|e| query_err("reading the change log", e))?;
        Ok(rows
            .iter()
            .map(|row| Change {
                seq: row.get("seq"),
                entity: row.get("entity"),
                entity_pk: row.get("entity_pk"),
                op: row.get("op"),
                payload: row.get("payload"),
                event_key: row.get("event_key"),
                team_scope: row.get::<Option<i64>, _>("team_scope").map(|t| t as i32),
                created_at: from_sql(&row.get::<String, _>("created_at")).unwrap_or_default(),
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use tt_repo::{NewUpstream, Repo, UPSTREAM_KEEP_PER_PATH};

    use crate::SqliteRepo;

    async fn repo() -> SqliteRepo {
        let repo = SqliteRepo::connect("sqlite::memory:").expect("connect");
        crate::migrate::apply(repo.pool()).await.expect("migrate");
        repo
    }

    fn fetched(path: &str, body: &str) -> NewUpstream {
        NewUpstream {
            api: "tba".into(),
            path: path.into(),
            etag: Some("\"v1\"".into()),
            body: body.into(),
            fetched_at: Utc.with_ymd_and_hms(2026, 3, 14, 12, 0, 0).unwrap(),
            via: "pi".into(),
        }
    }

    #[tokio::test]
    async fn a_response_is_appended_once_and_read_back_whole() {
        let repo = repo().await;
        let first = repo
            .append_upstream(&fetched("/event/2026mslr/matches", "[1]"))
            .await
            .unwrap();
        assert!(first.is_some());
        assert_eq!(
            repo.append_upstream(&fetched("/event/2026mslr/matches", "[1]"))
                .await
                .unwrap(),
            None,
            "the same body again is not news"
        );
        let log = repo.upstream_since(0, 10).await.unwrap();
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].entry, fetched("/event/2026mslr/matches", "[1]"));
    }

    #[tokio::test]
    async fn each_path_keeps_its_newest_few_and_seq_only_ever_grows() {
        let repo = repo().await;
        let mut last = 0;
        for n in 0..(UPSTREAM_KEEP_PER_PATH + 3) {
            let seq = repo
                .append_upstream(&fetched("/event/2026mslr/rankings", &format!("[{n}]")))
                .await
                .unwrap()
                .expect("new body");
            assert!(seq > last);
            last = seq;
        }
        repo.append_upstream(&fetched("/event/2026mslr/oprs", "{}"))
            .await
            .unwrap();

        let log = repo.upstream_since(0, 100).await.unwrap();
        let rankings: Vec<&str> = log
            .iter()
            .filter(|e| e.entry.path.ends_with("rankings"))
            .map(|e| e.entry.body.as_str())
            .collect();
        assert_eq!(rankings.len() as i64, UPSTREAM_KEEP_PER_PATH);
        assert_eq!(rankings.last(), Some(&"[7]"), "the newest is always kept");
        assert_eq!(log.last().unwrap().seq, last + 1, "never reused");

        let after = repo.upstream_since(last, 100).await.unwrap();
        assert_eq!(after.len(), 1, "a cursor sees only what came after it");
    }
}
