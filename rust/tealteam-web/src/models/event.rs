//! Events: the entity, the events a user may select, and the per-event
//! summary shown on the home page.
//!
//! Events are synced from the FIRST API (see
//! [`crate::services::first_sync`]); the app never creates them from user
//! input. Which events a user may select depends on their team: a user with a
//! team number sees the events that team is registered for, and a user without
//! one sees everything.
//!
//! `tba_key` (`2026mndu`) is the join key to both external APIs — The Blue
//! Alliance uses it directly, and [`crate::web::extract_event_code`] turns it
//! into the FIRST event code.

use chrono::NaiveDate;
use sqlx::{FromRow, PgPool};

use super::user::User;

/// An `events` row.
#[derive(Debug, Clone, FromRow)]
pub struct Event {
    pub id: i32,
    pub name: String,
    pub location: Option<String>,
    pub timezone: Option<String>,
    pub start_date: Option<NaiveDate>,
    pub end_date: Option<NaiveDate>,
    pub tba_key: Option<String>,
    pub event_type: Option<String>,
    pub district_key: Option<String>,
    pub week: Option<i32>,
}

/// One entry of the event `<select>`.
#[derive(Debug, Clone, FromRow)]
pub struct EventOption {
    pub id: i32,
    pub name: String,
}

/// A team on an event roster, as listed in the summary panel.
#[derive(Debug, Clone, FromRow)]
pub struct EventTeamRow {
    pub team_number: i32,
    pub name: String,
}

/// Everything the home-page event summary needs about one event.
#[derive(Debug, Clone, Default)]
pub struct EventSummary {
    pub name: String,
    pub teams_count: i64,
    pub matches_count: i64,
    pub teams: Vec<EventTeamRow>,
    /// True when the viewer's own team is not on the event roster.
    pub viewer_team_missing: bool,
}

/// What the FIRST/TBA schedule lookups need to identify an event.
#[derive(Debug, Clone, FromRow)]
pub struct EventScheduleSource {
    pub name: String,
    pub tba_key: Option<String>,
    pub timezone: Option<String>,
}

/// Events available to a user: their team's events, or all events for
/// team-less users.
pub async fn available_ids(pool: &PgPool, user: &User) -> anyhow::Result<Vec<i32>> {
    if let Some(team_number) = user.active_team_number() {
        return ids_for_team(pool, team_number).await;
    }

    Ok(sqlx::query_scalar("SELECT id FROM events ORDER BY start_date")
        .fetch_all(pool)
        .await?)
}

/// Ids of the events a team is registered for, via `event_teams`.
pub async fn ids_for_team(pool: &PgPool, team_number: i32) -> anyhow::Result<Vec<i32>> {
    Ok(sqlx::query_scalar(
        "SELECT DISTINCT event_teams.event_id
         FROM event_teams
         JOIN teams ON teams.id = event_teams.team_id
         WHERE teams.team_number = $1",
    )
    .bind(team_number)
    .fetch_all(pool)
    .await?)
}

/// Picker entries for the given ids, in start-date order. An empty id list
/// short-circuits without a query.
pub async fn options(pool: &PgPool, event_ids: &[i32]) -> Vec<EventOption> {
    if event_ids.is_empty() {
        return Vec::new();
    }
    sqlx::query_as("SELECT id, name FROM events WHERE id = ANY($1) ORDER BY start_date")
        .bind(event_ids)
        .fetch_all(pool)
        .await
        .unwrap_or_default()
}

/// Display name of one event.
pub async fn find_name(pool: &PgPool, event_id: i32) -> Option<String> {
    sqlx::query_scalar("SELECT name FROM events WHERE id = $1")
        .bind(event_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
}

/// TBA key of one event; `None` when the column is null (the double flatten
/// collapses "no row" and "null column" into the same answer).
pub async fn find_tba_key(pool: &PgPool, event_id: i32) -> Option<String> {
    sqlx::query_scalar::<_, Option<String>>("SELECT tba_key FROM events WHERE id = $1")
        .bind(event_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .flatten()
}

/// Everything the schedule lookups need in one query.
pub async fn find_schedule_source(pool: &PgPool, event_id: i32) -> Option<EventScheduleSource> {
    sqlx::query_as("SELECT name, tba_key, timezone FROM events WHERE id = $1")
        .bind(event_id)
        .fetch_optional(pool)
        .await
        .unwrap_or(None)
}

/// Counts and roster for the home-page summary panel.
///
/// Each query degrades independently (a failure shows as zero or an empty
/// list) so one bad column cannot blank the whole panel. When `viewer` has a
/// team number, the result also records whether that team is on the roster.
pub async fn summary(pool: &PgPool, event_id: i32, viewer: Option<&User>) -> EventSummary {
    let mut summary = EventSummary {
        name: find_name(pool, event_id).await.unwrap_or_default(),
        ..Default::default()
    };

    summary.teams_count = sqlx::query_scalar("SELECT COUNT(*) FROM event_teams WHERE event_id = $1")
        .bind(event_id)
        .fetch_one(pool)
        .await
        .unwrap_or(0);

    summary.matches_count = sqlx::query_scalar("SELECT COUNT(*) FROM matches WHERE event_id = $1")
        .bind(event_id)
        .fetch_one(pool)
        .await
        .unwrap_or(0);

    summary.teams = sqlx::query_as(
        "SELECT teams.team_number, teams.name
         FROM event_teams
         JOIN teams ON teams.id = event_teams.team_id
         WHERE event_teams.event_id = $1
         ORDER BY teams.team_number",
    )
    .bind(event_id)
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    if let Some(team_number) = viewer.and_then(|u| u.team_number) {
        let user_team_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM event_teams
             JOIN teams ON teams.id = event_teams.team_id
             WHERE event_teams.event_id = $1 AND teams.team_number = $2",
        )
        .bind(event_id)
        .bind(team_number)
        .fetch_one(pool)
        .await
        .unwrap_or(0);
        summary.viewer_team_missing = user_team_count == 0;
    }

    summary
}
