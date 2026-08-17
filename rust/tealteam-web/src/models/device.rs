//! Scouting devices: tablets identified by a permanent UUID cookie.
//!
//! `static/js/device.js` plants a `device_uuid` cookie on first visit and
//! posts a periodic heartbeat; [`heartbeat`] registers the device on first
//! contact and refreshes `last_seen_at` after. That timestamp is the whole
//! point — it lets the assignments page show which tablets are actually alive
//! in the pit right now (see
//! [`crate::models::assignment::ONLINE_WINDOW_MINUTES`]) so robots can be
//! assigned to a device rather than to a person.
//!
//! Devices are never deleted; a stale one simply stops being online.

use chrono::{DateTime, Utc};
use sqlx::{FromRow, PgPool};

/// A device with its display name and current online state.
#[derive(Debug, Clone, FromRow)]
pub struct DeviceRow {
    pub id: i32,
    pub name: String,
    pub online: bool,
}

/// Devices with a display name, most recently seen first.
pub async fn list(pool: &PgPool, online_cutoff: DateTime<Utc>) -> Vec<DeviceRow> {
    sqlx::query_as(
        "SELECT id, COALESCE(NULLIF(name, ''), 'Device ' || SUBSTRING(device_uuid, 1, 8)) AS name,
                (last_seen_at >= $1) IS TRUE AS online
         FROM devices ORDER BY last_seen_at DESC NULLS LAST",
    )
    .bind(online_cutoff)
    .fetch_all(pool)
    .await
    .unwrap_or_default()
}

/// Ids of devices seen since the cutoff, used by auto-distribute.
pub async fn online_ids(pool: &PgPool, online_cutoff: DateTime<Utc>) -> Vec<i32> {
    sqlx::query_scalar("SELECT id FROM devices WHERE last_seen_at >= $1")
        .bind(online_cutoff)
        .fetch_all(pool)
        .await
        .unwrap_or_default()
}

/// Gives a device a human name ("Pit tablet 2") in place of its UUID prefix.
pub async fn rename(pool: &PgPool, id: i32, name: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE devices SET name = $1, updated_at = CURRENT_TIMESTAMP WHERE id = $2")
        .bind(name)
        .bind(id)
        .execute(pool)
        .await
        .map(|_| ())
}

/// Registers a device on first contact and refreshes `last_seen_at` after.
pub async fn heartbeat(
    pool: &PgPool,
    device_uuid: &str,
    team_number: Option<i32>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO devices (device_uuid, team_number, last_seen_at)
         VALUES ($1, $2, $3)
         ON CONFLICT (device_uuid) DO UPDATE SET
            last_seen_at = EXCLUDED.last_seen_at,
            team_number = COALESCE(devices.team_number, EXCLUDED.team_number),
            updated_at = CURRENT_TIMESTAMP",
    )
    .bind(device_uuid)
    .bind(team_number)
    .bind(Utc::now())
    .execute(pool)
    .await
    .map(|_| ())
}
