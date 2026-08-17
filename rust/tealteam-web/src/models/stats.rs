//! Per-event team statistics synced from The Blue Alliance and FIRST.
//!
//! Written by [`crate::services::tba_stats_sync`] on a background loop and
//! read here; the app never computes these itself. Everything is optional
//! because a fresh event has rankings before it has OPR, and an event with no
//! `TBA_AUTH_KEY` configured has neither.
//!
//! OPR/DPR/CCWM are The Blue Alliance's alliance-contribution estimates;
//! `rank`, `wins`/`losses`/`ties` and the point columns come from the
//! qualification standings.

use sqlx::{FromRow, PgPool};

/// team_event_stats row; NUMERIC columns must be selected with ::float8 casts.
#[derive(Debug, Clone, FromRow)]
pub struct TeamEventStats {
    pub team_id: i32,
    pub event_id: i32,
    pub opr: Option<f64>,
    pub dpr: Option<f64>,
    pub ccwm: Option<f64>,
    pub auto_opr: Option<f64>,
    pub teleop_opr: Option<f64>,
    pub endgame_opr: Option<f64>,
    pub rank: Option<i32>,
    pub matches_played: Option<i32>,
    pub qual_average: Option<f64>,
    pub avg_match_points: Option<f64>,
    pub wins: Option<i32>,
    pub losses: Option<i32>,
    pub ties: Option<i32>,
    pub dq_count: Option<i32>,
    pub qual_points: Option<i32>,
    pub elim_points: Option<i32>,
    pub award_points: Option<i32>,
    pub alliance_points: Option<i32>,
    pub total_points: Option<i32>,
}

impl TeamEventStats {
    /// Base `SELECT` for this table.
    ///
    /// The numeric columns are `NUMERIC` in PostgreSQL, which sqlx will not
    /// decode into `f64` without an explicit cast, so every read goes through
    /// this string with a `WHERE` clause appended rather than `SELECT *`.
    pub const SELECT: &'static str = "SELECT team_id, event_id,
        opr::float8 AS opr, dpr::float8 AS dpr, ccwm::float8 AS ccwm,
        auto_opr::float8 AS auto_opr, teleop_opr::float8 AS teleop_opr,
        endgame_opr::float8 AS endgame_opr, rank, matches_played,
        qual_average::float8 AS qual_average, avg_match_points::float8 AS avg_match_points,
        wins, losses, ties, dq_count, qual_points, elim_points, award_points,
        alliance_points, total_points
        FROM team_event_stats";
}

/// Stats for one team at one event, `None` until a sync has run.
pub async fn find(pool: &PgPool, team_id: i32, event_id: i32) -> Option<TeamEventStats> {
    sqlx::query_as::<_, TeamEventStats>(&format!(
        "{} WHERE team_id = $1 AND event_id = $2 LIMIT 1",
        TeamEventStats::SELECT
    ))
    .bind(team_id)
    .bind(event_id)
    .fetch_optional(pool)
    .await
    .unwrap_or(None)
}

/// OPR/DPR by team number for the drive coach's match cards.
pub async fn opr_dpr_by_team_number(
    pool: &PgPool,
    event_id: i32,
    team_numbers: &[i32],
) -> Vec<(i32, Option<f64>, Option<f64>)> {
    if team_numbers.is_empty() {
        return Vec::new();
    }
    sqlx::query_as(
        "SELECT teams.team_number, team_event_stats.opr::float8, team_event_stats.dpr::float8
         FROM teams
         LEFT JOIN team_event_stats ON team_event_stats.team_id = teams.id
             AND team_event_stats.event_id = $1
         WHERE teams.team_number = ANY($2)",
    )
    .bind(event_id)
    .bind(team_numbers)
    .fetch_all(pool)
    .await
    .unwrap_or_default()
}
