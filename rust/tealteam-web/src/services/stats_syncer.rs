//! Background loop syncing team statistics and match results from The Blue
//! Alliance.
//!
//! A port of `internal/frc/team_stats_sync.go` and
//! `Services/TeamStatsSyncer.cs`.
//!
//! Spawned once at startup and runs for the life of the process. The cadence
//! adapts: every couple of minutes while an event is running or about to start
//! ([`INTERVAL_DURING_EVENT`]), every few hours otherwise
//! ([`INTERVAL_BETWEEN_EVENTS`]) — rankings move constantly during a
//! competition and not at all between them.
//!
//! Without a `TBA_AUTH_KEY` the loop logs and exits, leaving the app to run on
//! FIRST data alone.

use std::time::Duration;

use chrono::Utc;
use sqlx::PgPool;
use tracing::{info, warn};

use super::first_api::FirstApiClient;
use super::tba::TbaClient;
use super::tba_stats_sync;

/// Poll interval while an event is running or imminent.
const INTERVAL_DURING_EVENT: Duration = Duration::from_secs(2 * 60);
/// Poll interval the rest of the time.
const INTERVAL_BETWEEN_EVENTS: Duration = Duration::from_secs(3 * 60 * 60);

/// Runs the sync loop forever. Spawned by [`crate::main`]; returns
/// immediately if TBA is not configured.
pub async fn run(pool: PgPool) {
    let tba_auth_key = std::env::var("TBA_AUTH_KEY").unwrap_or_default();
    let tba_auth_key = tba_auth_key.trim();
    if tba_auth_key.is_empty() {
        warn!("TBA_AUTH_KEY not configured, team stats sync disabled");
        return;
    }

    info!("team stats sync loop started");
    let tba = TbaClient::new(tba_auth_key);

    // Initial sync immediately.
    run_sync_once(&pool, &tba).await;

    let mut current_interval = INTERVAL_BETWEEN_EVENTS;
    loop {
        tokio::time::sleep(current_interval).await;
        current_interval = determine_sync_interval(&pool).await;
        run_sync_once(&pool, &tba).await;
    }
}

/// One pass over every event, logging rather than propagating failures so a
/// bad pass never stops the loop.
async fn run_sync_once(pool: &PgPool, tba: &TbaClient) {
    match tokio::time::timeout(Duration::from_secs(120), sync_all_team_stats(pool, tba)).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => warn!("team stats sync failed: {e}"),
        Err(_) => warn!("team stats sync timed out"),
    }
}

/// Syncs stats and matches for every event that has a TBA key.
async fn sync_all_team_stats(pool: &PgPool, tba: &TbaClient) -> anyhow::Result<()> {
    let season = FirstApiClient::season_from_environment();
    let now = Utc::now().date_naive();

    let mut active_events: Vec<(i32, Option<String>)> = sqlx::query_as(
        "SELECT id, tba_key FROM events WHERE start_date <= $1 AND end_date >= $1",
    )
    .bind(now)
    .fetch_all(pool)
    .await?;

    if active_events.is_empty() {
        info!("no active events found, syncing recent/upcoming events");
        let week_ago = now - chrono::Duration::days(7);
        let next_week = now + chrono::Duration::days(7);
        active_events = sqlx::query_as(
            "SELECT id, tba_key FROM events WHERE end_date >= $1 AND start_date <= $2",
        )
        .bind(week_ago)
        .bind(next_week)
        .fetch_all(pool)
        .await?;
    }

    for (event_id, raw_key) in active_events {
        let Some(raw_key) = raw_key.filter(|k| !k.trim().is_empty()) else {
            warn!("event {event_id} has no TBA key, skipping");
            continue;
        };

        let event_key = tba_stats_sync::normalize_tba_event_key(&raw_key, season);
        if event_key.is_empty() {
            warn!("event {event_id} has invalid TBA key, skipping");
            continue;
        }

        if let Err(e) =
            tba_stats_sync::sync_team_stats_for_event(pool, tba, event_id, &event_key).await
        {
            warn!("failed to sync stats for event {event_id}: {e}");
            continue;
        }

        if let Err(e) = tba_stats_sync::sync_event_matches(pool, tba, event_id, &event_key).await {
            warn!("failed to sync matches for event {event_id}: {e}");
        }
    }

    Ok(())
}

/// Picks the next interval: fast if an event is running today or starts
/// within 24 hours, slow otherwise. A database that will not answer means
/// slow, so a broken query cannot turn into a hot loop.
async fn determine_sync_interval(pool: &PgPool) -> Duration {
    let now = Utc::now().date_naive();

    let active: Result<i64, _> = tokio::time::timeout(
        Duration::from_secs(5),
        sqlx::query_scalar("SELECT COUNT(*) FROM events WHERE start_date <= $1 AND end_date >= $1")
            .bind(now)
            .fetch_one(pool),
    )
    .await
    .unwrap_or_else(|_| Ok(0));

    match active {
        Ok(count) if count > 0 => {
            info!("{count} active events found, using fast sync interval");
            return INTERVAL_DURING_EVENT;
        }
        Ok(_) => {}
        Err(e) => {
            warn!("failed to determine sync interval, using slow interval: {e}");
            return INTERVAL_BETWEEN_EVENTS;
        }
    }

    let next_day = now + chrono::Duration::days(1);
    let upcoming: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events WHERE start_date > $1 AND start_date <= $2",
    )
    .bind(now)
    .bind(next_day)
    .fetch_one(pool)
    .await
    .unwrap_or(0);

    if upcoming > 0 {
        info!("{upcoming} events starting in next 24 hours, using fast sync interval");
        return INTERVAL_DURING_EVENT;
    }

    INTERVAL_BETWEEN_EVENTS
}
