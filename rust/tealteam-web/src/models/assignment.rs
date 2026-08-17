//! Matches and per-match scout assignments.
//!
//! A match has six robot slots (three red, three blue) holding **team
//! numbers**, mirroring how FIRST and TBA publish schedules. An assignment
//! attaches one scout or one device to one (match, team) pair, so a scout
//! knows which robot to watch in which match and the submission form can
//! pre-fill itself.
//!
//! An assignment points at either a user or a device, never both — see
//! [`Assignee`]. Device assignments exist because a tablet may be passed
//! between students without anyone signing in; the tablet is identified by its
//! `device_uuid` cookie ([`super::device`]).
//!
//! `matches` rows are synced from The Blue Alliance
//! ([`crate::services::tba_stats_sync`]) and also serve as the offline
//! fallback schedule when the FIRST API is unreachable.

use chrono::{DateTime, Utc};
use sqlx::{FromRow, PgPool};

/// A match with its six robot slots, as stored (slots hold team numbers).
#[derive(Debug, Clone, FromRow)]
pub struct MatchRow {
    pub id: i32,
    pub match_number: i32,
    pub match_type: String,
    pub played: bool,
    pub scheduled_time: Option<DateTime<Utc>>,
    pub red1: Option<i32>,
    pub red2: Option<i32>,
    pub red3: Option<i32>,
    pub blue1: Option<i32>,
    pub blue2: Option<i32>,
    pub blue3: Option<i32>,
}

impl MatchRow {
    /// Red slots as `(station number, team number)`, station 1-3.
    pub fn red_slots(&self) -> [(i32, Option<i32>); 3] {
        [(1, self.red1), (2, self.red2), (3, self.red3)]
    }

    /// Blue slots as `(station number, team number)`, station 1-3.
    pub fn blue_slots(&self) -> [(i32, Option<i32>); 3] {
        [(1, self.blue1), (2, self.blue2), (3, self.blue3)]
    }
}

/// A `scout_assignments` row.
#[derive(Debug, Clone, FromRow)]
pub struct AssignmentRow {
    pub id: i32,
    pub match_id: i32,
    pub team_id: i32,
    pub scouter_id: Option<i32>,
    pub device_id: Option<i32>,
}

/// A robot the current scout (or their device) is expected to scout next.
#[derive(Debug, Clone, FromRow)]
pub struct AssignedTeam {
    pub team_id: i32,
    pub team_number: i32,
    pub team_name: String,
    pub event_id: i32,
    pub match_number: Option<i32>,
}

/// Whether a slot is assigned to a scout or to a device.
#[derive(Debug, Clone, Copy)]
pub struct Assignee {
    pub scouter_id: Option<i32>,
    pub device_id: Option<i32>,
}

impl Assignee {
    /// Parses the picker's `u:<user id>` / `d:<device id>` option values.
    ///
    /// `None` for anything else, which controllers treat as a bad request
    /// rather than silently clearing the slot.
    pub fn parse(raw: &str) -> Option<Self> {
        if let Some(rest) = raw.strip_prefix("u:") {
            rest.parse().ok().map(|id| Self {
                scouter_id: Some(id),
                device_id: None,
            })
        } else if let Some(rest) = raw.strip_prefix("d:") {
            rest.parse().ok().map(|id| Self {
                scouter_id: None,
                device_id: Some(id),
            })
        } else {
            None
        }
    }
}

/// Every match at an event, in schedule order.
pub async fn matches_for_event(pool: &PgPool, event_id: i32) -> Vec<MatchRow> {
    sqlx::query_as(
        "SELECT id, match_number, match_type, played, scheduled_time,
                red1, red2, red3, blue1, blue2, blue3
         FROM matches WHERE event_id = $1
         ORDER BY match_number, match_type",
    )
    .bind(event_id)
    .fetch_all(pool)
    .await
    .unwrap_or_default()
}

/// Match numbers and times only — the offline fallback for the schedule panel.
pub async fn scheduled_times(pool: &PgPool, event_id: i32) -> Vec<(i32, Option<DateTime<Utc>>)> {
    sqlx::query_as(
        "SELECT match_number, scheduled_time FROM matches WHERE event_id = $1 ORDER BY scheduled_time",
    )
    .bind(event_id)
    .fetch_all(pool)
    .await
    .unwrap_or_default()
}

/// Every assignment at an event, joined through `matches` since assignments
/// are keyed by match.
pub async fn for_event(pool: &PgPool, event_id: i32) -> Vec<AssignmentRow> {
    sqlx::query_as(
        "SELECT sa.id, sa.match_id, sa.team_id, sa.scouter_id, sa.device_id
         FROM scout_assignments sa
         JOIN matches m ON m.id = sa.match_id
         WHERE m.event_id = $1",
    )
    .bind(event_id)
    .fetch_all(pool)
    .await
    .unwrap_or_default()
}

/// Upcoming robots assigned to this scout, matched by user id OR by the
/// device's permanent UUID.
pub async fn upcoming_for_scout(
    pool: &PgPool,
    event_id: i32,
    scouter_id: i32,
    device_uuid: Option<String>,
) -> Vec<AssignedTeam> {
    sqlx::query_as(
        "SELECT sa.team_id, teams.team_number, teams.name AS team_name,
                m.event_id, m.match_number
         FROM scout_assignments sa
         JOIN matches m ON m.id = sa.match_id
         JOIN teams ON teams.id = sa.team_id
         LEFT JOIN devices ON devices.id = sa.device_id
         WHERE m.event_id = $1
           AND m.played = FALSE
           AND (sa.scouter_id = $2 OR (devices.device_uuid = $3 AND $3 IS NOT NULL))
         ORDER BY m.match_number ASC",
    )
    .bind(event_id)
    .bind(scouter_id)
    .bind(device_uuid)
    .fetch_all(pool)
    .await
    .unwrap_or_default()
}

/// Assigns a slot, replacing whoever held it.
pub async fn set(
    pool: &PgPool,
    match_id: i32,
    team_id: i32,
    event_id: i32,
    assignee: Assignee,
    assigned_by: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO scout_assignments (match_id, team_id, event_id, scouter_id, device_id, assigned_by)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (match_id, team_id) DO UPDATE SET
            scouter_id = EXCLUDED.scouter_id, device_id = EXCLUDED.device_id,
            assigned_by = EXCLUDED.assigned_by, updated_at = CURRENT_TIMESTAMP",
    )
    .bind(match_id)
    .bind(team_id)
    .bind(event_id)
    .bind(assignee.scouter_id)
    .bind(assignee.device_id)
    .bind(assigned_by)
    .execute(pool)
    .await
    .map(|_| ())
}

/// Fills an empty slot without disturbing one that is already assigned.
pub async fn set_if_absent(
    pool: &PgPool,
    match_id: i32,
    team_id: i32,
    event_id: i32,
    assignee: Assignee,
    assigned_by: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO scout_assignments (match_id, team_id, event_id, scouter_id, device_id, assigned_by)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (match_id, team_id) DO NOTHING",
    )
    .bind(match_id)
    .bind(team_id)
    .bind(event_id)
    .bind(assignee.scouter_id)
    .bind(assignee.device_id)
    .bind(assigned_by)
    .execute(pool)
    .await
    .map(|_| ())
}

/// Unassigns one robot slot.
pub async fn clear_slot(pool: &PgPool, match_id: i32, team_id: i32) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM scout_assignments WHERE match_id = $1 AND team_id = $2")
        .bind(match_id)
        .bind(team_id)
        .execute(pool)
        .await
        .map(|_| ())
}

/// Unassigns every slot at an event.
pub async fn clear_event(pool: &PgPool, event_id: i32) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM scout_assignments WHERE event_id = $1")
        .bind(event_id)
        .execute(pool)
        .await
        .map(|_| ())
}

/// Unassigns every slot in one match.
pub async fn clear_match(pool: &PgPool, match_id: i32) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM scout_assignments WHERE match_id = $1")
        .bind(match_id)
        .execute(pool)
        .await
        .map(|_| ())
}

/// Robot slots in upcoming unplayed matches that nobody is assigned to,
/// ordered so auto-distribute spreads assignees evenly.
pub async fn unassigned_slots(pool: &PgPool, event_id: i32) -> Vec<(i32, i32)> {
    sqlx::query_as(
        "SELECT teams_slot.match_id, teams_slot.team_id
         FROM (
            SELECT m.id AS match_id, t.id AS team_id
            FROM matches m
            CROSS JOIN LATERAL (
                SELECT teams.id
                FROM teams
                WHERE teams.team_number IN (m.red1, m.red2, m.red3, m.blue1, m.blue2, m.blue3)
            ) t(id)
            WHERE m.event_id = $1 AND m.played = FALSE
              AND (m.red1 IS NOT NULL OR m.blue1 IS NOT NULL)
         ) teams_slot
         LEFT JOIN scout_assignments sa
            ON sa.match_id = teams_slot.match_id AND sa.team_id = teams_slot.team_id
         WHERE sa.id IS NULL
         ORDER BY teams_slot.match_id, teams_slot.team_id",
    )
    .bind(event_id)
    .fetch_all(pool)
    .await
    .unwrap_or_default()
}

/// A device counts as online if it checked in within this window.
pub const ONLINE_WINDOW_MINUTES: i64 = 3;

/// Timestamp a device must have checked in after to count as online.
pub fn online_cutoff() -> DateTime<Utc> {
    Utc::now() - chrono::Duration::minutes(ONLINE_WINDOW_MINUTES)
}
