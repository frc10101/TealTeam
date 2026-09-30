//! Pick lists (U20).
//!
//! A list is written whole, but only over the list the edit was made to: the
//! read, the comparison, and the write share one transaction, and the pool's
//! single connection means nothing else can write in between. A list that
//! changed since it was read is left alone and reported, so the caller redoes
//! its edit on the new one rather than writing a stale order over it.
//!
//! Rows are matched by team, not deleted and re-inserted, so each keeps its
//! `client_record_id` and `created_at`, and only rows that changed get a new
//! `updated_at`.

use chrono::{DateTime, Utc};
use sqlx::{Row, Sqlite, SqliteConnection};
use tt_core::picklist::{Entry, Tag};
use tt_repo::Result;

use crate::SqliteRepo;
use crate::users::{query_err, to_sql};

async fn load(
    conn: &mut SqliteConnection,
    owning_team: i32,
    event_key: &str,
) -> Result<Vec<Entry>> {
    let rows = sqlx::query::<Sqlite>(
        "SELECT client_record_id, picked_team, color, crossed FROM pick_list_entries \
         WHERE owning_team = ? AND event_key = ? ORDER BY position, id",
    )
    .bind(owning_team)
    .bind(event_key)
    .fetch_all(conn)
    .await
    .map_err(|e| query_err("loading a pick list", e))?;
    Ok(rows
        .iter()
        .map(|row| Entry {
            record_id: row.get("client_record_id"),
            team_number: row.get("picked_team"),
            // A colour this build does not know shows as untagged rather than
            // failing the whole list.
            tag: row
                .get::<Option<String>, _>("color")
                .as_deref()
                .and_then(Tag::parse),
            crossed: row.get::<i64, _>("crossed") != 0,
        })
        .collect())
}

impl SqliteRepo {
    pub(crate) async fn pick_list_impl(
        &self,
        owning_team: i32,
        event_key: &str,
    ) -> Result<Vec<Entry>> {
        let mut conn = self
            .pool
            .acquire()
            .await
            .map_err(|e| query_err("reading a pick list", e))?;
        load(&mut conn, owning_team, event_key).await
    }

    pub(crate) async fn replace_pick_list_impl(
        &self,
        owning_team: i32,
        event_key: &str,
        expected: &[Entry],
        list: &[Entry],
        now: DateTime<Utc>,
    ) -> Result<bool> {
        let ts = to_sql(now);
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| query_err("starting pick list write", e))?;

        if load(&mut tx, owning_team, event_key).await? != expected {
            return Ok(false);
        }

        for gone in expected
            .iter()
            .filter(|e| !list.iter().any(|n| n.team_number == e.team_number))
        {
            sqlx::query(
                "DELETE FROM pick_list_entries \
                 WHERE owning_team = ? AND event_key = ? AND picked_team = ?",
            )
            .bind(owning_team)
            .bind(event_key)
            .bind(gone.team_number)
            .execute(&mut *tx)
            .await
            .map_err(|e| query_err("removing a pick", e))?;
        }

        for (index, entry) in list.iter().enumerate() {
            sqlx::query(
                "INSERT INTO pick_list_entries (client_record_id, owning_team, event_key, \
                     picked_team, color, crossed, position, created_at, updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) \
                 ON CONFLICT (owning_team, event_key, picked_team) DO UPDATE SET \
                    color = excluded.color, crossed = excluded.crossed, \
                    position = excluded.position, updated_at = excluded.updated_at \
                 WHERE color IS NOT excluded.color OR crossed != excluded.crossed \
                    OR position != excluded.position",
            )
            .bind(&entry.record_id)
            .bind(owning_team)
            .bind(event_key)
            .bind(entry.team_number)
            .bind(entry.tag.map(Tag::key))
            .bind(entry.crossed as i64)
            .bind(index as i64 + 1)
            .bind(&ts)
            .bind(&ts)
            .execute(&mut *tx)
            .await
            .map_err(|e| query_err("storing a pick", e))?;
        }

        tx.commit()
            .await
            .map_err(|e| query_err("committing pick list", e))?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tt_core::picklist::{Edit, apply};
    use tt_core::records::Event;
    use tt_repo::Repo;

    const EVENT: &str = "2026mabil";

    async fn repo() -> SqliteRepo {
        let repo = SqliteRepo::connect("sqlite::memory:").expect("connect");
        crate::migrate::apply(repo.pool()).await.expect("migrate");
        for key in [EVENT, "2026nhgrs"] {
            let event = Event {
                key: key.into(),
                name: key.into(),
                location: None,
                timezone: None,
                start_date: None,
                end_date: None,
                event_code: None,
                event_type: None,
                district_key: None,
                week: None,
            };
            repo.upsert_event(&event, Utc::now()).await.expect("event");
        }
        repo
    }

    /// Read, edit, and write back, as the web layer does.
    async fn edit(repo: &SqliteRepo, owning_team: i32, edit: Edit) -> bool {
        let before = repo.pick_list(owning_team, EVENT).await.expect("read");
        let mut after = before.clone();
        apply(&mut after, &edit, &[]).expect("edit");
        repo.replace_pick_list(owning_team, EVENT, &before, &after, Utc::now())
            .await
            .expect("write")
    }

    fn add(team: i32) -> Edit {
        Edit::Add {
            team,
            record_id: format!("0191f7ac-0000-7000-8000-{team:012}"),
        }
    }

    fn order(list: &[Entry]) -> Vec<i32> {
        list.iter().map(|e| e.team_number).collect()
    }

    #[tokio::test]
    async fn a_list_reads_back_in_order_with_its_tags() {
        let repo = repo().await;
        for team in [254, 1678, 118] {
            assert!(edit(&repo, 10101, add(team)).await);
        }
        assert!(edit(&repo, 10101, Edit::Up { team: 118 }).await);
        assert!(
            edit(
                &repo,
                10101,
                Edit::Tag {
                    team: 254,
                    tag: Some(Tag::Green)
                }
            )
            .await
        );
        assert!(
            edit(
                &repo,
                10101,
                Edit::Cross {
                    team: 1678,
                    crossed: true
                }
            )
            .await
        );
        assert!(edit(&repo, 10101, Edit::Remove { team: 254 }).await);

        let list = repo.pick_list(10101, EVENT).await.expect("read");
        assert_eq!(order(&list), [118, 1678]);
        assert_eq!(list[1].record_id, "0191f7ac-0000-7000-8000-000000001678");
        assert!(list[1].crossed);

        let positions: Vec<i64> = sqlx::query_scalar(
            "SELECT position FROM pick_list_entries WHERE owning_team = 10101 ORDER BY position",
        )
        .fetch_all(repo.pool())
        .await
        .unwrap();
        assert_eq!(positions, [1, 2], "no gap where 254 was");
    }

    #[tokio::test]
    async fn a_list_changed_since_it_was_read_is_not_written_over() {
        let repo = repo().await;
        assert!(edit(&repo, 10101, add(254)).await);

        // Two leads read the same list...
        let seen = repo.pick_list(10101, EVENT).await.unwrap();
        // ...one adds a team...
        assert!(edit(&repo, 10101, add(1678)).await);
        // ...and the other's write, made to the list it saw, is refused.
        let mut stale = seen.clone();
        apply(&mut stale, &add(118), &[]).unwrap();
        let written = repo
            .replace_pick_list(10101, EVENT, &seen, &stale, Utc::now())
            .await
            .unwrap();
        assert!(!written);
        assert_eq!(
            order(&repo.pick_list(10101, EVENT).await.unwrap()),
            [254, 1678]
        );
    }

    #[tokio::test]
    async fn each_team_has_its_own_list_per_event() {
        let repo = repo().await;
        assert!(edit(&repo, 10101, add(254)).await);
        assert!(edit(&repo, 1678, add(118)).await);
        let other_event = repo.pick_list(10101, "2026nhgrs").await.unwrap();

        assert_eq!(order(&repo.pick_list(10101, EVENT).await.unwrap()), [254]);
        assert_eq!(order(&repo.pick_list(1678, EVENT).await.unwrap()), [118]);
        assert!(other_event.is_empty());
    }
}
