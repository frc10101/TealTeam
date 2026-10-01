//! Pick lists (U20, L14).
//!
//! A list is a yrs document (`tt_core::picklist::PickDoc`), stored whole in
//! `pick_list_docs`, and its rows in `pick_list_entries` are what it reads as.
//! A change is an update merged into the stored document. The read, the
//! merge, and both writes share one transaction, and the pool's single
//! connection means nothing else writes in between. Merges commute, so
//! nothing is compared or retried: two changes made to the same reading of
//! the list both land, as does a tablet's batch made offline.
//!
//! Rows are matched by team and record id, not deleted and re-inserted, so
//! each keeps its `created_at`, and only rows that changed get a new
//! `updated_at`. A team whose record id changed -- added on two copies at
//! once, and the other copy's id won -- is a new row.

use chrono::{DateTime, Utc};
use sqlx::{Row, Sqlite, SqliteConnection};
use tt_core::picklist::{Entry, PickDoc, Tag};
use tt_repo::{RepoError, Result};

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

/// The list's document. A list stored before L14 has none, and gets one
/// from its rows, stored at once so it is only ever made once.
async fn doc(
    conn: &mut SqliteConnection,
    owning_team: i32,
    event_key: &str,
    ts: &str,
) -> Result<PickDoc> {
    let state: Option<Vec<u8>> = sqlx::query_scalar(
        "SELECT state FROM pick_list_docs WHERE owning_team = ? AND event_key = ?",
    )
    .bind(owning_team)
    .bind(event_key)
    .fetch_optional(&mut *conn)
    .await
    .map_err(|e| query_err("reading a pick list document", e))?;
    if let Some(state) = state {
        return PickDoc::load(&state)
            .map_err(|e| RepoError::Query(format!("reading a pick list document: {e}")));
    }
    let doc = PickDoc::from_entries(&load(conn, owning_team, event_key).await?);
    store_doc(conn, owning_team, event_key, &doc, ts).await?;
    Ok(doc)
}

async fn store_doc(
    conn: &mut SqliteConnection,
    owning_team: i32,
    event_key: &str,
    doc: &PickDoc,
    ts: &str,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO pick_list_docs (owning_team, event_key, state, updated_at) \
         VALUES (?, ?, ?, ?) \
         ON CONFLICT (owning_team, event_key) DO UPDATE SET \
            state = excluded.state, updated_at = excluded.updated_at",
    )
    .bind(owning_team)
    .bind(event_key)
    .bind(doc.state())
    .bind(ts)
    .execute(conn)
    .await
    .map_err(|e| query_err("storing a pick list document", e))?;
    Ok(())
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

    pub(crate) async fn pick_list_doc_impl(
        &self,
        owning_team: i32,
        event_key: &str,
        now: DateTime<Utc>,
    ) -> Result<Vec<u8>> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| query_err("starting pick list read", e))?;
        let doc = doc(&mut tx, owning_team, event_key, &to_sql(now)).await?;
        tx.commit()
            .await
            .map_err(|e| query_err("committing pick list read", e))?;
        Ok(doc.state())
    }

    pub(crate) async fn merge_pick_list_impl(
        &self,
        owning_team: i32,
        event_key: &str,
        update: &[u8],
        now: DateTime<Utc>,
    ) -> Result<Vec<Entry>> {
        let ts = to_sql(now);
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| query_err("starting pick list write", e))?;

        let mut doc = doc(&mut tx, owning_team, event_key, &ts).await?;
        doc.merge(update).map_err(RepoError::Query)?;
        let before = load(&mut tx, owning_team, event_key).await?;
        let list = doc.entries();

        for gone in before.iter().filter(|e| {
            !list
                .iter()
                .any(|n| n.team_number == e.team_number && n.record_id == e.record_id)
        }) {
            sqlx::query("DELETE FROM pick_list_entries WHERE client_record_id = ?")
                .bind(&gone.record_id)
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

        store_doc(&mut tx, owning_team, event_key, &doc, &ts).await?;
        tx.commit()
            .await
            .map_err(|e| query_err("committing pick list", e))?;
        Ok(list)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tt_core::picklist::Edit;
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

    /// Read, edit, and merge back, as the web layer does. The update, for
    /// merging again.
    async fn edit(repo: &SqliteRepo, owning_team: i32, edit: Edit) -> Vec<u8> {
        let state = repo
            .pick_list_doc(owning_team, EVENT, Utc::now())
            .await
            .expect("read");
        let update = PickDoc::load(&state)
            .unwrap()
            .apply(&edit, &[])
            .expect("edit")
            .expect("a change");
        repo.merge_pick_list(owning_team, EVENT, &update, Utc::now())
            .await
            .expect("write");
        update
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
            edit(&repo, 10101, add(team)).await;
        }
        edit(&repo, 10101, Edit::Up { team: 118 }).await;
        edit(
            &repo,
            10101,
            Edit::Tag {
                team: 254,
                tag: Some(Tag::Green),
            },
        )
        .await;
        edit(
            &repo,
            10101,
            Edit::Cross {
                team: 1678,
                crossed: true,
            },
        )
        .await;
        edit(&repo, 10101, Edit::Remove { team: 254 }).await;

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
    async fn two_changes_to_one_reading_both_land_and_twice_is_once() {
        let repo = repo().await;
        edit(&repo, 10101, add(254)).await;
        edit(&repo, 10101, add(1678)).await;

        // Two leads read the same list...
        let seen = repo.pick_list_doc(10101, EVENT, Utc::now()).await.unwrap();
        let up = PickDoc::load(&seen)
            .unwrap()
            .apply(&Edit::Up { team: 1678 }, &[])
            .unwrap()
            .unwrap();
        let tag = PickDoc::load(&seen)
            .unwrap()
            .apply(
                &Edit::Tag {
                    team: 254,
                    tag: Some(Tag::Blue),
                },
                &[],
            )
            .unwrap()
            .unwrap();
        // ...and both their changes are kept, whichever lands second.
        repo.merge_pick_list(10101, EVENT, &tag, Utc::now())
            .await
            .unwrap();
        let list = repo
            .merge_pick_list(10101, EVENT, &up, Utc::now())
            .await
            .unwrap();
        assert_eq!(order(&list), [1678, 254]);
        assert_eq!(list[1].tag, Some(Tag::Blue));
        assert_eq!(repo.pick_list(10101, EVENT).await.unwrap(), list);

        // A tablet resending what it already sent changes nothing.
        let again = repo
            .merge_pick_list(10101, EVENT, &up, Utc::now())
            .await
            .unwrap();
        assert_eq!(again, list);

        let bad = repo
            .merge_pick_list(10101, EVENT, b"garbage", Utc::now())
            .await;
        assert!(bad.is_err());
        assert_eq!(repo.pick_list(10101, EVENT).await.unwrap(), list);
    }

    #[tokio::test]
    async fn a_list_from_before_the_document_gets_one_from_its_rows() {
        let repo = repo().await;
        for (position, team) in [(1, 118), (2, 254)] {
            sqlx::query(
                "INSERT INTO pick_list_entries (client_record_id, owning_team, event_key, \
                     picked_team, color, crossed, position, created_at, updated_at) \
                 VALUES (?, 10101, ?, ?, 'red', 0, ?, '2026-03-01T00:00:00.000Z', \
                     '2026-03-01T00:00:00.000Z')",
            )
            .bind(format!("old-{team}"))
            .bind(EVENT)
            .bind(team)
            .bind(position)
            .execute(repo.pool())
            .await
            .unwrap();
        }

        // Made once: reading twice gives the same document, not a second
        // copy of every team.
        let first = repo.pick_list_doc(10101, EVENT, Utc::now()).await.unwrap();
        let second = repo.pick_list_doc(10101, EVENT, Utc::now()).await.unwrap();
        assert_eq!(first, second);

        edit(&repo, 10101, Edit::Down { team: 118 }).await;
        let list = repo.pick_list(10101, EVENT).await.unwrap();
        assert_eq!(order(&list), [254, 118]);
        assert_eq!(list[0].record_id, "old-254");
        assert_eq!(list[0].tag, Some(Tag::Red));
    }

    #[tokio::test]
    async fn a_team_whose_record_id_changed_is_a_new_row() {
        let repo = repo().await;
        let seen = repo.pick_list_doc(10101, EVENT, Utc::now()).await.unwrap();
        // Two copies add 1678 with ids of their own.
        let ids: Vec<_> = ["a", "b"]
            .into_iter()
            .map(|id| {
                PickDoc::load(&seen)
                    .unwrap()
                    .apply(
                        &Edit::Add {
                            team: 1678,
                            record_id: id.into(),
                        },
                        &[],
                    )
                    .unwrap()
                    .unwrap()
            })
            .collect();
        for update in &ids {
            repo.merge_pick_list(10101, EVENT, update, Utc::now())
                .await
                .unwrap();
        }
        let state = repo.pick_list_doc(10101, EVENT, Utc::now()).await.unwrap();
        let winner = PickDoc::load(&state).unwrap().entries();
        let rows = repo.pick_list(10101, EVENT).await.unwrap();
        assert_eq!(rows, winner, "one row, with the id that won");
    }

    #[tokio::test]
    async fn each_team_has_its_own_list_per_event() {
        let repo = repo().await;
        edit(&repo, 10101, add(254)).await;
        edit(&repo, 1678, add(118)).await;
        let other_event = repo.pick_list(10101, "2026nhgrs").await.unwrap();

        assert_eq!(order(&repo.pick_list(10101, EVENT).await.unwrap()), [254]);
        assert_eq!(order(&repo.pick_list(1678, EVENT).await.unwrap()), [118]);
        assert!(other_event.is_empty());
    }
}
