// Pulls FIRST Events API data into events, teams, and event_teams.
// Port of internal/frc/sync.go and Services/FirstSyncService.cs.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use chrono::NaiveDate;
use sqlx::PgPool;
use tracing::{info, warn};

use crate::connectivity;
use crate::first_api::{FirstApiClient, FirstEvent, FirstTeam};
use crate::tba::TbaClient;
use crate::tba_stats_sync;

const DEFAULT_COUNTRY: &str = "USA";

#[derive(Debug)]
pub struct SyncSkipped;

impl std::fmt::Display for SyncSkipped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "first sync skipped")
    }
}
impl std::error::Error for SyncSkipped {}

#[derive(Debug, Clone, Copy)]
pub struct SyncResult {
    pub season: i32,
    pub events: usize,
    pub teams: usize,
    pub event_teams: usize,
}

pub async fn sync_on_boot(pool: &PgPool) {
    let skip = std::env::var("FIRST_SYNC_ON_BOOT")
        .map(|v| v.eq_ignore_ascii_case("false"))
        .unwrap_or(false);
    if skip {
        info!("FIRST sync skipped (FIRST_SYNC_ON_BOOT=false)");
        return;
    }

    match tokio::time::timeout(Duration::from_secs(60), sync_now(pool)).await {
        Ok(Ok(result)) => info!(
            "FIRST sync complete: events={} teams={} event_teams={}",
            result.events, result.teams, result.event_teams
        ),
        Ok(Err(e)) if e.is::<SyncSkipped>() => {
            info!("FIRST sync skipped (missing FIRST_API_USERNAME or FIRST_API_KEY)");
        }
        Ok(Err(e)) => warn!("FIRST sync failed: {e}"),
        Err(_) => warn!("FIRST sync failed: timed out"),
    }
}

pub async fn sync_now(pool: &PgPool) -> anyhow::Result<SyncResult> {
    let client = FirstApiClient::from_environment().ok_or(SyncSkipped)?;
    let season = FirstApiClient::season_from_environment();

    let event_code_filter = env_trimmed("FIRST_EVENT_CODE");
    let team_filter = env_trimmed("FIRST_TEAM_NUMBER");
    let mut country_filter = env_trimmed("FIRST_COUNTRY");
    if country_filter.is_empty() {
        country_filter = DEFAULT_COUNTRY.to_string();
    }

    let mut filters = HashMap::new();
    if !event_code_filter.is_empty() {
        filters.insert("eventCode".to_string(), event_code_filter.clone());
    }
    if !team_filter.is_empty() {
        filters.insert("teamNumber".to_string(), team_filter.clone());
    }

    info!("FIRST sync starting (season {season})");
    let mut events = client.get_season_events(season, &filters).await?;

    if event_code_filter.is_empty() && team_filter.is_empty() && !country_filter.is_empty() {
        events.retain(|e| e.country.eq_ignore_ascii_case(&country_filter));
    }

    let event_ids = upsert_events(pool, &events).await;
    info!("FIRST events synced: {}", event_ids.len());

    let mut unique_teams = HashSet::new();
    let mut event_team_count = 0;
    for (event_code, event_id) in &event_ids {
        let teams = match client.get_event_teams(season, event_code).await {
            Ok(t) => t,
            Err(e) => {
                warn!("teams fetch failed ({event_code}): {e}");
                continue;
            }
        };

        let mut team_ids = Vec::new();
        for team in &teams {
            match upsert_team(pool, team).await {
                Ok(id) => {
                    team_ids.push(id);
                    unique_teams.insert(id);
                }
                Err(e) => warn!("team upsert failed (team {}): {e}", team.team_number),
            }
        }

        for team_id in &team_ids {
            if let Err(e) = upsert_event_team(pool, *event_id, *team_id).await {
                warn!("event_teams upsert failed (event {event_id}, team {team_id}): {e}");
            }
        }

        event_team_count += team_ids.len();
        info!("FIRST teams synced for {event_code}: {}", team_ids.len());
    }

    connectivity::record_successful_sync();
    Ok(SyncResult {
        season,
        events: event_ids.len(),
        teams: unique_teams.len(),
        event_teams: event_team_count,
    })
}

/// Pulls FIRST Events API data for a specific team (called on sign-in/sign-up),
/// then launches a background TBA stats sync for that team's events.
pub async fn sync_team_for_user(pool: &PgPool, team_number: i32) -> anyhow::Result<SyncResult> {
    let client = FirstApiClient::from_environment().ok_or(SyncSkipped)?;
    let season = FirstApiClient::season_from_environment();

    let mut filters = HashMap::new();
    filters.insert("teamNumber".to_string(), team_number.to_string());

    info!("FIRST team sync starting for team {team_number} (season {season})");
    let events = client.get_season_events(season, &filters).await?;
    if events.is_empty() {
        warn!("no events found for team {team_number} in season {season}");
        return Ok(SyncResult { season, events: 0, teams: 0, event_teams: 0 });
    }

    let event_ids = upsert_events(pool, &events).await;
    info!("FIRST events synced for team {team_number}: {}", event_ids.len());

    let mut team_id = 0;
    let mut unique_teams = HashSet::new();
    let mut event_team_count = 0;

    for (event_code, event_id) in &event_ids {
        let teams = match client.get_event_teams(season, event_code).await {
            Ok(t) => t,
            Err(e) => {
                warn!("teams fetch failed ({event_code}): {e}");
                continue;
            }
        };

        for team in &teams {
            let id = match upsert_team(pool, team).await {
                Ok(id) => id,
                Err(e) => {
                    warn!("team upsert failed (team {}): {e}", team.team_number);
                    continue;
                }
            };

            unique_teams.insert(id);
            if team.team_number == team_number {
                team_id = id;
            }

            match upsert_event_team(pool, *event_id, id).await {
                Ok(()) => event_team_count += 1,
                Err(e) => warn!("event_teams upsert failed (event {event_id}, team {id}): {e}"),
            }
        }
    }

    if team_id == 0 {
        anyhow::bail!("team {team_number} not found in any events");
    }

    info!(
        "FIRST team sync complete for team {team_number}: events={} teams={} event_teams={}",
        event_ids.len(),
        unique_teams.len(),
        event_team_count
    );

    // Sync TBA stats for the team's events in the background (detached).
    let ids_snapshot: Vec<i32> = event_ids.values().copied().collect();
    let pool_clone = pool.clone();
    tokio::spawn(async move {
        sync_team_tba_stats(&pool_clone, ids_snapshot).await;
    });

    connectivity::record_successful_sync();
    Ok(SyncResult {
        season,
        events: event_ids.len(),
        teams: 1,
        event_teams: event_team_count,
    })
}

async fn sync_team_tba_stats(pool: &PgPool, event_ids: Vec<i32>) {
    let tba_key = env_trimmed("TBA_AUTH_KEY");
    if tba_key.is_empty() {
        info!("TBA_AUTH_KEY not configured, skipping TBA stats sync");
        return;
    }

    let tba = TbaClient::new(&tba_key);
    for event_id in event_ids {
        let event_tba_key: Option<Option<String>> =
            sqlx::query_scalar("SELECT tba_key FROM events WHERE id = $1")
                .bind(event_id)
                .fetch_optional(pool)
                .await
                .unwrap_or(None);

        let Some(Some(event_tba_key)) = event_tba_key else {
            warn!("event {event_id} has no TBA key, skipping stats sync");
            continue;
        };
        if event_tba_key.trim().is_empty() {
            warn!("event {event_id} has no TBA key, skipping stats sync");
            continue;
        }

        let sync = tba_stats_sync::sync_team_stats_for_event(pool, &tba, event_id, &event_tba_key);
        match tokio::time::timeout(Duration::from_secs(30), sync).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => warn!("TBA stats sync failed for event {event_id}: {e}"),
            Err(_) => warn!("TBA stats sync timed out for event {event_id}"),
        }
    }
}

async fn upsert_events(pool: &PgPool, events: &[FirstEvent]) -> HashMap<String, i32> {
    let mut event_ids = HashMap::new();
    for evt in events {
        let event_code = evt.effective_code();
        if event_code.is_empty() {
            warn!("event missing code: {}", evt.name);
            continue;
        }

        match upsert_event(pool, evt).await {
            Ok(id) => {
                event_ids.insert(event_code, id);
            }
            Err(e) => warn!("event upsert failed ({event_code}): {e}"),
        }
    }
    event_ids
}

async fn upsert_event(pool: &PgPool, evt: &FirstEvent) -> anyhow::Result<i32> {
    let start_date = parse_event_date(&evt.date_start)?;
    let end_date = parse_event_date(&evt.date_end)?;
    let event_code = evt.effective_code();
    if event_code.is_empty() {
        anyhow::bail!("missing event code");
    }

    let location = [&evt.venue, &evt.city, &evt.stateprov, &evt.country]
        .iter()
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join(", ");

    // TBA key: {year}{event_code_lowercase}, e.g. 2026mabil
    let tba_key = format!("{}{}", start_date.format("%Y"), event_code.to_lowercase());

    // events.tba_key has no unique constraint, so mirror GORM's FirstOrCreate:
    // select first, then update or insert.
    let existing_id: Option<i32> =
        sqlx::query_scalar("SELECT id FROM events WHERE tba_key = $1 LIMIT 1")
            .bind(&tba_key)
            .fetch_optional(pool)
            .await?;

    if let Some(id) = existing_id {
        sqlx::query(
            "UPDATE events SET name = $1, location = $2, start_date = $3, end_date = $4,
                event_type = $5, district_key = $6, week = $7, updated_at = CURRENT_TIMESTAMP
             WHERE id = $8",
        )
        .bind(&evt.name)
        .bind(&location)
        .bind(start_date)
        .bind(end_date)
        .bind(&evt.event_type)
        .bind(&evt.district_code)
        .bind(evt.week_number)
        .bind(id)
        .execute(pool)
        .await?;
        return Ok(id);
    }

    let id: i32 = sqlx::query_scalar(
        "INSERT INTO events (name, location, start_date, end_date, tba_key, event_type, district_key, week)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         RETURNING id",
    )
    .bind(&evt.name)
    .bind(&location)
    .bind(start_date)
    .bind(end_date)
    .bind(&tba_key)
    .bind(&evt.event_type)
    .bind(&evt.district_code)
    .bind(evt.week_number)
    .fetch_one(pool)
    .await?;
    Ok(id)
}

pub async fn upsert_team(pool: &PgPool, team: &FirstTeam) -> anyhow::Result<i32> {
    let name = team.display_name();

    let existing_id: Option<i32> =
        sqlx::query_scalar("SELECT id FROM teams WHERE team_number = $1 LIMIT 1")
            .bind(team.team_number)
            .fetch_optional(pool)
            .await?;

    let tba_key = format!("frc{}", team.team_number);

    if let Some(id) = existing_id {
        sqlx::query(
            "UPDATE teams SET name = $1, school = $2, city = $3, state = $4, tba_key = $5,
                nickname = $6, school_name = $7, country = $8, rookie_year = $9, website = $10,
                updated_at = CURRENT_TIMESTAMP
             WHERE id = $11",
        )
        .bind(&name)
        .bind(&team.school_name)
        .bind(&team.city)
        .bind(&team.state_prov)
        .bind(&tba_key)
        .bind(&team.name_short)
        .bind(&team.school_name)
        .bind(&team.country)
        .bind(team.rookie_year)
        .bind(&team.website)
        .bind(id)
        .execute(pool)
        .await?;
        return Ok(id);
    }

    let id: i32 = sqlx::query_scalar(
        "INSERT INTO teams (team_number, name, school, city, state, tba_key, nickname,
            school_name, country, rookie_year, website)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
         RETURNING id",
    )
    .bind(team.team_number)
    .bind(&name)
    .bind(&team.school_name)
    .bind(&team.city)
    .bind(&team.state_prov)
    .bind(&tba_key)
    .bind(&team.name_short)
    .bind(&team.school_name)
    .bind(&team.country)
    .bind(team.rookie_year)
    .bind(&team.website)
    .fetch_one(pool)
    .await?;
    Ok(id)
}

async fn upsert_event_team(pool: &PgPool, event_id: i32, team_id: i32) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO event_teams (event_id, team_id) VALUES ($1, $2)
         ON CONFLICT (event_id, team_id) DO NOTHING",
    )
    .bind(event_id)
    .bind(team_id)
    .execute(pool)
    .await?;
    Ok(())
}

fn parse_event_date(value: &str) -> anyhow::Result<NaiveDate> {
    let trimmed = value.trim().trim_matches('"');
    if trimmed.is_empty() {
        anyhow::bail!("empty date");
    }

    if let Ok(d) = NaiveDate::parse_from_str(trimmed, "%Y-%m-%d") {
        return Ok(d);
    }
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%dT%H:%M:%S") {
        return Ok(dt.date());
    }
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(trimmed) {
        return Ok(dt.date_naive());
    }

    anyhow::bail!("unparsable date: {trimmed}")
}

fn env_trimmed(key: &str) -> String {
    std::env::var(key).unwrap_or_default().trim().to_string()
}
