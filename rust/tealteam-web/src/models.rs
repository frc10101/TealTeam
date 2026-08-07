// Entity structs mapped by sqlx::FromRow. Field names match the snake_case
// column names (port of Models/Entities.cs). Timestamp columns declared
// without NOT NULL are Option<_> so runtime decoding never panics.

use chrono::{DateTime, NaiveDate, Utc};
use sqlx::FromRow;

#[derive(Debug, Clone, FromRow)]
pub struct User {
    pub id: i32,
    pub email: String,
    pub name: String,
    pub password_hash: String,
    pub team_number: Option<i32>,
    pub role: Option<String>,
    pub is_admin: bool,
    pub is_lead_scout: bool,
    pub is_coach: bool,
    pub last_login: Option<DateTime<Utc>>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, FromRow)]
pub struct Session {
    pub session_id: String,
    pub user_id: i32,
    pub selected_event_id: Option<i32>,
    pub expires_at: DateTime<Utc>,
    pub created_at: Option<DateTime<Utc>>,
}

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

#[derive(Debug, Clone, FromRow)]
pub struct ScoutingData {
    pub id: i32,
    pub event_id: i32,
    pub team_id: i32,
    pub alliance_color: String,
    pub notes: Option<String>,
    pub starting_position: Option<String>,
    pub defense_rating: Option<String>,
    pub scoring_strategy: Option<String>,
    pub shooting_speed: Option<String>,
    pub capacity: Option<String>,
    pub defendability: Option<String>,
    pub traversal: Option<String>,
    pub hang_level: Option<String>,
    pub auto_hang: Option<String>,
    pub hang_position: Option<String>,
    pub accuracy_rating: Option<String>,
    pub scouted_at: Option<DateTime<Utc>>,
    pub scouter_id: Option<i32>,
    pub submitting_team_id: Option<i32>,
}

#[derive(Debug, Clone, FromRow)]
pub struct ScoutingSubmission {
    pub id: i32,
    pub event_id: i32,
    pub team_id: i32,
    pub alliance_color: String,
    pub notes: Option<String>,
    pub starting_position: Option<String>,
    pub defense_rating: Option<String>,
    pub traversal: Option<String>,
    pub scoring_strategy: Option<String>,
    pub shooting_speed: Option<String>,
    pub capacity: Option<String>,
    pub defendability: Option<String>,
    pub hang_level: Option<String>,
    pub auto_hang: Option<String>,
    pub hang_position: Option<String>,
    pub scouted_at: Option<DateTime<Utc>>,
    pub scouter_id: Option<i32>,
    pub submitting_team_id: Option<i32>,
}

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
    pub const SELECT: &'static str = "SELECT team_id, event_id,
        opr::float8 AS opr, dpr::float8 AS dpr, ccwm::float8 AS ccwm,
        auto_opr::float8 AS auto_opr, teleop_opr::float8 AS teleop_opr,
        endgame_opr::float8 AS endgame_opr, rank, matches_played,
        qual_average::float8 AS qual_average, avg_match_points::float8 AS avg_match_points,
        wins, losses, ties, dq_count, qual_points, elim_points, award_points,
        alliance_points, total_points
        FROM team_event_stats";
}

#[derive(Debug, Clone, FromRow)]
pub struct PickListEntry {
    pub picked_team_number: i32,
    pub color: Option<String>,
    pub crossed: Option<bool>,
    pub position: Option<i32>,
}
