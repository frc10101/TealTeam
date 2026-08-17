//! Teams: the entity and the lookups used by the team page, the submission
//! form and the assignment grid.
//!
//! Teams are synced from the FIRST API. Two identifiers are in play and are
//! easy to confuse: `team_number` is the FRC number everyone says out loud
//! (2530) and is what `matches` rows store in their robot slots, while `id` is
//! the local surrogate key that `scouting_data`, `event_teams` and
//! `scout_assignments` reference. Functions here are named for which one they
//! take or return.

use sqlx::{FromRow, PgPool};

/// A `teams` row.
#[derive(Debug, Clone, FromRow)]
pub struct Team {
    pub id: i32,
    pub team_number: i32,
    pub name: String,
    pub school: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub tba_key: Option<String>,
    pub nickname: Option<String>,
    pub school_name: Option<String>,
    pub country: Option<String>,
    pub rookie_year: Option<i32>,
    pub motto: Option<String>,
    pub website: Option<String>,
}

/// One entry of a team `<select>` on the submission form.
#[derive(Debug, Clone, FromRow)]
pub struct TeamOption {
    pub id: i32,
    pub team_number: i32,
    pub name: String,
}

/// A team on an event roster, keyed by team number for the assignment grid.
#[derive(Debug, Clone, FromRow)]
pub struct EventTeamLookup {
    pub team_number: i32,
    pub team_id: i32,
    pub team_name: String,
}

/// Full team record by FRC number.
pub async fn find_by_number(pool: &PgPool, team_number: i32) -> Option<Team> {
    sqlx::query_as("SELECT * FROM teams WHERE team_number = $1 LIMIT 1")
        .bind(team_number)
        .fetch_optional(pool)
        .await
        .unwrap_or(None)
}

/// Local id for an FRC number, for foreign keys.
pub async fn id_by_number(pool: &PgPool, team_number: i32) -> Option<i32> {
    sqlx::query_scalar("SELECT id FROM teams WHERE team_number = $1 LIMIT 1")
        .bind(team_number)
        .fetch_optional(pool)
        .await
        .unwrap_or(None)
}

/// FRC number for a local id, for display and API lookups.
pub async fn number_by_id(pool: &PgPool, team_id: i32) -> Option<i32> {
    sqlx::query_scalar("SELECT team_number FROM teams WHERE id = $1")
        .bind(team_id)
        .fetch_optional(pool)
        .await
        .unwrap_or(None)
}

/// The team id of the team a user belongs to, resolved through their profile.
pub async fn id_for_user(pool: &PgPool, user_id: i32) -> Option<i32> {
    sqlx::query_scalar(
        "SELECT t.id FROM users u JOIN teams t ON t.team_number = u.team_number WHERE u.id = $1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .unwrap_or(None)
}

/// Teams on an event roster, for the submission form's team picker.
pub async fn options_for_event(pool: &PgPool, event_id: i32) -> Vec<TeamOption> {
    sqlx::query_as(
        "SELECT teams.id, teams.team_number, teams.name
         FROM teams
         JOIN event_teams ON teams.id = event_teams.team_id
         WHERE event_teams.event_id = $1
         ORDER BY teams.team_number",
    )
    .bind(event_id)
    .fetch_all(pool)
    .await
    .unwrap_or_default()
}

/// Event roster for resolving the team numbers in `matches` robot slots to
/// local ids and names.
pub async fn lookup_for_event(pool: &PgPool, event_id: i32) -> Vec<EventTeamLookup> {
    sqlx::query_as(
        "SELECT teams.team_number, teams.id AS team_id, teams.name AS team_name
         FROM teams
         JOIN event_teams ON teams.id = event_teams.team_id
         WHERE event_teams.event_id = $1",
    )
    .bind(event_id)
    .fetch_all(pool)
    .await
    .unwrap_or_default()
}

/// Teams on an event roster with their qualification rank, if stats are synced.
pub async fn roster_with_rank(pool: &PgPool, event_id: i32) -> Vec<(i32, i32, String, Option<i32>)> {
    sqlx::query_as(
        "SELECT teams.id AS team_id, teams.team_number, teams.name AS team_name, team_event_stats.rank
         FROM teams
         JOIN event_teams ON event_teams.team_id = teams.id
         LEFT JOIN team_event_stats ON team_event_stats.team_id = teams.id
             AND team_event_stats.event_id = event_teams.event_id
         WHERE event_teams.event_id = $1
         ORDER BY teams.team_number",
    )
    .bind(event_id)
    .fetch_all(pool)
    .await
    .unwrap_or_default()
}
