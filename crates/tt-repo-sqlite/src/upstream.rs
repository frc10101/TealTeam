//! The upstream log (S1): FIRST and TBA responses, as received.
//!
//! An append skips a body identical to the newest for its path, and prunes the
//! path to its newest [`UPSTREAM_KEEP_PER_PATH`], in the same transaction.
//! Both keep the log's size proportional to the number of distinct requests,
//! not to how long the server has been polling them.

use chrono::{DateTime, Utc};
use sqlx::Row;
use tt_repo::{Change, NewUpstream, Result, UPSTREAM_KEEP_PER_PATH, UpstreamEntry};

use crate::SqliteRepo;
use crate::users::{from_sql, query_err, to_sql};

impl SqliteRepo {
    pub(crate) async fn append_upstream_impl(&self, entry: &NewUpstream) -> Result<Option<i64>> {
        let mut tx = self
            .pool()
            .begin()
            .await
            .map_err(|e| query_err("appending to the upstream log", e))?;

        let newest: Option<String> = sqlx::query_scalar(
            "SELECT body FROM upstream WHERE api = ? AND path = ? ORDER BY seq DESC LIMIT 1",
        )
        .bind(&entry.api)
        .bind(&entry.path)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| query_err("reading the upstream log", e))?;
        if newest.as_deref() == Some(entry.body.as_str()) {
            return Ok(None);
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
        .fetch_one(&mut *tx)
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
        .execute(&mut *tx)
        .await
        .map_err(|e| query_err("pruning the upstream log", e))?;

        tx.commit()
            .await
            .map_err(|e| query_err("appending to the upstream log", e))?;
        Ok(Some(seq))
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
        Ok(rows
            .iter()
            .map(|row| UpstreamEntry {
                seq: row.get("seq"),
                entry: NewUpstream {
                    api: row.get("api"),
                    path: row.get("path"),
                    etag: row.get("etag"),
                    body: row.get("body"),
                    fetched_at: from_sql(&row.get::<String, _>("fetched_at")).unwrap_or_default(),
                    via: row.get("via"),
                },
            })
            .collect())
    }
}

impl SqliteRepo {
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
