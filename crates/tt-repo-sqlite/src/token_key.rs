//! The Pi's token signing key (C9), in the database it signs for.

use chrono::{DateTime, Utc};
use tt_repo::{RepoError, Result};

use crate::SqliteRepo;
use crate::users::{query_err, to_sql};

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
}
