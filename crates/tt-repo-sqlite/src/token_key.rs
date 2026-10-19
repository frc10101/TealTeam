//! The Pi's token signing key (C9), in the database it signs for; each
//! user's token epoch, and the record of handoff scans (T1).

use chrono::{DateTime, Utc};
use tt_repo::{RepoError, Result};

use crate::SqliteRepo;
use crate::users::{from_sql, query_err, to_sql};

impl SqliteRepo {
    /// The signing seed, made from `fresh` if there is none yet. The first
    /// caller's bytes win, so two at once agree on one key.
    pub async fn token_seed(&self, fresh: [u8; 32], now: DateTime<Utc>) -> Result<[u8; 32]> {
        sqlx::query("INSERT OR IGNORE INTO token_key (id, seed, created_at) VALUES (1, ?, ?)")
            .bind(&fresh[..])
            .bind(to_sql(now))
            .execute(self.pool())
            .await
            .map_err(|e| query_err("making the token key", e))?;
        let seed: Vec<u8> = sqlx::query_scalar("SELECT seed FROM token_key WHERE id = 1")
            .fetch_one(self.pool())
            .await
            .map_err(|e| query_err("reading the token key", e))?;
        seed.try_into()
            .map_err(|_| RepoError::Query("the token key is not 32 bytes".into()))
    }

    /// The epoch `user_id`'s tokens must carry: 0 until first bumped.
    pub async fn token_epoch(&self, user_id: i64) -> Result<i64> {
        let epoch: Option<i64> =
            sqlx::query_scalar("SELECT epoch FROM token_epochs WHERE user_id = ?")
                .bind(user_id)
                .fetch_optional(self.pool())
                .await
                .map_err(|e| query_err("reading a token epoch", e))?;
        Ok(epoch.unwrap_or(0))
    }

    /// Sign `user_id` out everywhere: every session deleted, and the token
    /// epoch bumped, so every offline token issued before is refused. One
    /// transaction. The new epoch.
    pub async fn sign_out_everywhere(
        &self,
        user_id: i64,
        by: i64,
        now: DateTime<Utc>,
    ) -> Result<i64> {
        let mut tx = self
            .pool()
            .begin()
            .await
            .map_err(|e| query_err("signing out everywhere", e))?;
        let epoch = end_everywhere(&mut tx, user_id, by, now).await?;
        tx.commit()
            .await
            .map_err(|e| query_err("signing out everywhere", e))?;
        Ok(epoch)
    }

    /// Record that `lead` scanned a tablet's forms in (S13).
    pub async fn record_handoff_scan(&self, scan: &HandoffScan<'_>) -> Result<()> {
        sqlx::query(
            "INSERT INTO handoff_scans \
               (scanned_at, lead_id, scout_id, device_uuid, nonce, sent, recorded) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(to_sql(scan.at))
        .bind(scan.lead_id)
        .bind(scan.scout_id)
        .bind(scan.device_uuid)
        .bind(scan.nonce)
        .bind(scan.sent as i64)
        .bind(scan.recorded as i64)
        .execute(self.pool())
        .await
        .map_err(|e| query_err("recording a handoff scan", e))?;
        Ok(())
    }

    /// The latest handoff scans, newest first, names as they are now.
    pub async fn handoff_scans(&self, limit: i64) -> Result<Vec<HandoffScanRow>> {
        let rows: Vec<ScanRow> = sqlx::query_as(
            "SELECT h.scanned_at, l.name, s.name, h.device_uuid, h.sent, h.recorded \
                 FROM handoff_scans h \
                 LEFT JOIN users l ON l.id = h.lead_id \
                 LEFT JOIN users s ON s.id = h.scout_id \
                 ORDER BY h.id DESC LIMIT ?",
        )
        .bind(limit)
        .fetch_all(self.pool())
        .await
        .map_err(|e| query_err("reading handoff scans", e))?;
        Ok(rows
            .into_iter()
            .map(
                |(at, lead, scout, device_uuid, sent, recorded)| HandoffScanRow {
                    at: from_sql(&at).unwrap_or_default(),
                    lead,
                    scout,
                    device_uuid,
                    sent,
                    recorded,
                },
            )
            .collect())
    }
}

/// `handoff_scans` joined to names, as read.
type ScanRow = (String, Option<String>, Option<String>, String, i64, i64);

/// A handoff scan, to record.
pub struct HandoffScan<'a> {
    pub at: DateTime<Utc>,
    pub lead_id: i64,
    pub scout_id: i64,
    pub device_uuid: &'a str,
    pub nonce: &'a str,
    pub sent: usize,
    pub recorded: usize,
}

/// A handoff scan, as read back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandoffScanRow {
    pub at: DateTime<Utc>,
    pub lead: Option<String>,
    pub scout: Option<String>,
    pub device_uuid: String,
    pub sent: i64,
    pub recorded: i64,
}

/// The inside of [`SqliteRepo::sign_out_everywhere`], for a caller that has
/// more to do in the same transaction -- a password reset (G1) must not
/// leave a window where the new password is set and the old sessions live.
pub(crate) async fn end_everywhere(
    conn: &mut sqlx::SqliteConnection,
    user_id: i64,
    by: i64,
    now: DateTime<Utc>,
) -> Result<i64> {
    sqlx::query("DELETE FROM sessions WHERE user_id = ?")
        .bind(user_id)
        .execute(&mut *conn)
        .await
        .map_err(|e| query_err("deleting a user's sessions", e))?;
    let epoch: i64 = sqlx::query_scalar(
        "INSERT INTO token_epochs (user_id, epoch, bumped_at, bumped_by) VALUES (?, 1, ?, ?) \
         ON CONFLICT (user_id) DO UPDATE SET epoch = epoch + 1, \
           bumped_at = excluded.bumped_at, bumped_by = excluded.bumped_by \
         RETURNING epoch",
    )
    .bind(user_id)
    .bind(to_sql(now))
    .bind(by)
    .fetch_one(&mut *conn)
    .await
    .map_err(|e| query_err("bumping a token epoch", e))?;
    Ok(epoch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_first_key_is_kept() {
        let repo = SqliteRepo::connect("sqlite::memory:").expect("connect");
        crate::migrate::apply(repo.pool()).await.expect("migrate");
        let first = repo.token_seed([1; 32], Utc::now()).await.unwrap();
        assert_eq!(first, [1; 32]);
        assert_eq!(repo.token_seed([2; 32], Utc::now()).await.unwrap(), first);
    }

    #[tokio::test]
    async fn signing_out_everywhere_ends_sessions_and_bumps_the_epoch() {
        let repo = SqliteRepo::connect("sqlite::memory:").expect("connect");
        crate::migrate::apply(repo.pool()).await.expect("migrate");
        let now = Utc::now();
        for (id, name) in [(1, "Sam"), (2, "Kim")] {
            sqlx::query(
                "INSERT INTO users (id, email, name, password_hash, created_at, updated_at) \
                 VALUES (?, ?, ?, 'x', ?, ?)",
            )
            .bind(id)
            .bind(format!("{name}@example.com"))
            .bind(name)
            .bind(to_sql(now))
            .bind(to_sql(now))
            .execute(repo.pool())
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO sessions (id, user_id, created_at, expires_at) VALUES (?, ?, ?, ?)",
            )
            .bind(format!("session-{id}"))
            .bind(id)
            .bind(to_sql(now))
            .bind(to_sql(now + chrono::TimeDelta::hours(1)))
            .execute(repo.pool())
            .await
            .unwrap();
        }
        assert_eq!(repo.token_epoch(2).await.unwrap(), 0);
        assert_eq!(repo.sign_out_everywhere(2, 1, now).await.unwrap(), 1);
        assert_eq!(repo.sign_out_everywhere(2, 1, now).await.unwrap(), 2);
        assert_eq!(repo.token_epoch(2).await.unwrap(), 2);
        assert_eq!(repo.token_epoch(1).await.unwrap(), 0, "only Kim's");
        let sessions: Vec<i64> = sqlx::query_scalar("SELECT user_id FROM sessions")
            .fetch_all(repo.pool())
            .await
            .unwrap();
        assert_eq!(sessions, [1], "Kim's session is gone, Sam's is not");

        repo.record_handoff_scan(&HandoffScan {
            at: now,
            lead_id: 1,
            scout_id: 2,
            device_uuid: "tablet-1",
            nonce: "n1",
            sent: 3,
            recorded: 2,
        })
        .await
        .unwrap();
        let scans = repo.handoff_scans(10).await.unwrap();
        assert_eq!(scans.len(), 1);
        assert_eq!(scans[0].lead.as_deref(), Some("Sam"));
        assert_eq!(scans[0].scout.as_deref(), Some("Kim"));
        assert_eq!((scans[0].sent, scans[0].recorded), (3, 2));
    }
}
