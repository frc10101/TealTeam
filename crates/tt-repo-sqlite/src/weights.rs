//! The lead scout's point-value overrides (L12).
//!
//! Only values that differ from the season schema are stored, so a row always
//! means "changed on purpose" and resetting is emptying the table. A row naming
//! a field the schema no longer has is harmless: scoring ignores it.

use chrono::{DateTime, Utc};
use sqlx::Row;
use tt_core::season::WeightOverrides;
use tt_repo::Result;

use crate::SqliteRepo;
use crate::users::{query_err, to_sql};

impl SqliteRepo {
    pub(crate) async fn weight_overrides_impl(&self) -> Result<WeightOverrides> {
        let rows = sqlx::query("SELECT field_key, option_key, points FROM scouting_point_weights")
            .fetch_all(&self.pool)
            .await
            .map_err(|e| query_err("loading point weights", e))?;
        Ok(rows
            .iter()
            .map(|row| {
                (
                    (
                        row.get::<String, _>("field_key"),
                        row.get::<String, _>("option_key"),
                    ),
                    row.get::<i64, _>("points"),
                )
            })
            .collect())
    }

    pub(crate) async fn replace_weight_overrides_impl(
        &self,
        overrides: &WeightOverrides,
        now: DateTime<Utc>,
    ) -> Result<()> {
        let ts = to_sql(now);
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| query_err("starting weights write", e))?;
        sqlx::query("DELETE FROM scouting_point_weights")
            .execute(&mut *tx)
            .await
            .map_err(|e| query_err("clearing point weights", e))?;
        for (field, option, points) in overrides.iter() {
            sqlx::query(
                "INSERT INTO scouting_point_weights (field_key, option_key, points, updated_at) \
                 VALUES (?, ?, ?, ?)",
            )
            .bind(field)
            .bind(option)
            .bind(points)
            .bind(&ts)
            .execute(&mut *tx)
            .await
            .map_err(|e| query_err("saving a point weight", e))?;
        }
        tx.commit()
            .await
            .map_err(|e| query_err("committing point weights", e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tt_repo::Repo;

    #[tokio::test]
    async fn overrides_are_replaced_whole() {
        let repo = SqliteRepo::connect("sqlite::memory:").expect("connect");
        crate::migrate::apply(repo.pool()).await.expect("migrate");
        assert!(repo.weight_overrides().await.unwrap().is_empty());

        let mut first = WeightOverrides::new();
        first.set("teleop_scored", "__each", 3);
        first.set("endgame", "full", 10);
        repo.replace_weight_overrides(&first, Utc::now())
            .await
            .unwrap();
        assert_eq!(repo.weight_overrides().await.unwrap(), first);

        // Saving the form again replaces everything: endgame went back to its
        // default, so it is no longer stored.
        let mut second = WeightOverrides::new();
        second.set("teleop_scored", "__each", 5);
        repo.replace_weight_overrides(&second, Utc::now())
            .await
            .unwrap();
        assert_eq!(repo.weight_overrides().await.unwrap(), second);

        repo.replace_weight_overrides(&WeightOverrides::new(), Utc::now())
            .await
            .unwrap();
        assert!(repo.weight_overrides().await.unwrap().is_empty());
    }
}
