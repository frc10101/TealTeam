//! Pick list: each team's private draft ordering of the teams at an event.
//!
//! Rows are keyed by `(team_number, event_id, picked_team_number)`, so every
//! team keeps its own list and never sees another's. Position and colour are
//! client-driven — the page reorders and re-colours entries, then saves each
//! one through [`save_entry`], which upserts.

use sqlx::{FromRow, PgPool};

/// One team on a pick list, with its position and highlight colour.
#[derive(Debug, Clone, FromRow)]
pub struct PickListEntry {
    pub picked_team_number: i32,
    pub color: Option<String>,
    pub crossed: Option<bool>,
    pub position: Option<i32>,
}

/// One team's list for one event, in saved order.
pub async fn entries(pool: &PgPool, team_number: i32, event_id: i32) -> Vec<PickListEntry> {
    sqlx::query_as(
        "SELECT picked_team_number, color, crossed, position FROM pick_list_entries
         WHERE team_number = $1 AND event_id = $2 ORDER BY position",
    )
    .bind(team_number)
    .bind(event_id)
    .fetch_all(pool)
    .await
    .unwrap_or_default()
}

/// Inserts or updates one entry.
pub async fn save_entry(
    pool: &PgPool,
    team_number: i32,
    event_id: i32,
    entry: &PickListEntry,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO pick_list_entries (team_number, event_id, picked_team_number, color, crossed, position)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (team_number, event_id, picked_team_number) DO UPDATE SET
            color = EXCLUDED.color, crossed = EXCLUDED.crossed, position = EXCLUDED.position,
            updated_at = CURRENT_TIMESTAMP",
    )
    .bind(team_number)
    .bind(event_id)
    .bind(entry.picked_team_number)
    .bind(&entry.color)
    .bind(entry.crossed.unwrap_or(false))
    .bind(entry.position.unwrap_or(0))
    .execute(pool)
    .await
    .map(|_| ())
}

/// Removes one team from a pick list.
pub async fn delete_entry(
    pool: &PgPool,
    team_number: i32,
    event_id: i32,
    picked_team_number: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "DELETE FROM pick_list_entries
         WHERE team_number = $1 AND event_id = $2 AND picked_team_number = $3",
    )
    .bind(team_number)
    .bind(event_id)
    .bind(picked_team_number)
    .execute(pool)
    .await
    .map(|_| ())
}
