//! Scouting data: queued submissions awaiting lead-scout review, and the
//! approved rows they become.
//!
//! Scouts do not write to `scouting_data` directly. A submission lands in
//! `scouting_submissions` ([`queue_submission`]), a lead scout reviews it
//! ([`pending_submissions`], [`submission_detail`]), and approving it moves
//! the row into `scouting_data` in one transaction ([`approve_submission`]);
//! declining just deletes it ([`delete_submission`]). Only approved rows feed
//! rankings and the team page.
//!
//! `submitting_team_id` records which team collected the observation. Free-text
//! notes are only ever shown back to that team — see
//! [`crate::views::teams::TeamDataView::build`] — while the structured fields
//! are shared with everyone, so alliance partners can see capability without
//! reading another team's private commentary.
//!
//! The structured fields are stored as lowercase keyword strings (`"high"`,
//! `"l3"`, `"trench"`) rather than enums, matching the Go schema and letting
//! [`super::scouting_points`] score them against a configurable weight table.

use chrono::{DateTime, Utc};
use sqlx::{FromRow, PgPool};

use super::scouting_points::ScoutingMetricRow;

/// An approved `scouting_data` row.
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

/// A `scouting_submissions` row: submitted, not yet reviewed.
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

/// A queued submission joined with the names shown in the review list.
#[derive(Debug, Clone, FromRow)]
pub struct PendingSubmission {
    pub id: i32,
    pub event_name: String,
    pub team_number: i32,
    pub team_name: String,
    pub scout_name: Option<String>,
    pub notes: Option<String>,
}

/// One queued submission with every field resolved for the detail page.
#[derive(Debug, Clone, FromRow)]
pub struct SubmissionDetailRow {
    pub id: i32,
    pub event_name: String,
    pub team_number: i32,
    pub team_name: String,
    pub scout_name: Option<String>,
    pub alliance_color: String,
    pub notes: String,
    pub starting_position: String,
    pub defense_rating: String,
    pub traversal: String,
    pub scoring_strategy: String,
    pub shooting_speed: String,
    pub capacity: String,
    pub defendability: String,
    pub hang_level: String,
    pub auto_hang: String,
    pub hang_position: String,
    pub created_at: String,
}

/// Values a scout enters on the submission form.
pub struct NewSubmission<'a> {
    pub event_id: i32,
    pub team_id: i32,
    pub alliance_color: &'a str,
    pub notes: &'a str,
    pub starting_position: &'a str,
    pub defense_rating: String,
    pub traversal: String,
    pub scoring_strategy: String,
    pub shooting_speed: String,
    pub capacity: String,
    pub defendability: &'a str,
    pub hang_level: String,
    pub auto_hang: String,
    pub hang_position: String,
    pub scouter_id: i32,
    pub submitting_team_id: Option<i32>,
}

/// Queues a scout's submission for review.
pub async fn queue_submission(
    pool: &PgPool,
    submission: &NewSubmission<'_>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO scouting_submissions (event_id, team_id, alliance_color, notes, starting_position,
            defense_rating, traversal, scoring_strategy, shooting_speed, capacity, defendability,
            hang_level, auto_hang, hang_position, scouted_at, scouter_id, submitting_team_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)",
    )
    .bind(submission.event_id)
    .bind(submission.team_id)
    .bind(submission.alliance_color)
    .bind(submission.notes)
    .bind(submission.starting_position)
    .bind(&submission.defense_rating)
    .bind(&submission.traversal)
    .bind(&submission.scoring_strategy)
    .bind(&submission.shooting_speed)
    .bind(&submission.capacity)
    .bind(submission.defendability)
    .bind(&submission.hang_level)
    .bind(&submission.auto_hang)
    .bind(&submission.hang_position)
    .bind(Utc::now())
    .bind(submission.scouter_id)
    .bind(submission.submitting_team_id)
    .execute(pool)
    .await
    .map(|_| ())
}

/// The review queue, oldest first. Not scoped to an event: the panel shows
/// everything outstanding, as in the Go app.
pub async fn pending_submissions(pool: &PgPool) -> Vec<PendingSubmission> {
    sqlx::query_as(
        "SELECT scouting_submissions.id, events.name AS event_name, teams.team_number,
                teams.name AS team_name, users.name AS scout_name, scouting_submissions.notes
         FROM scouting_submissions
         JOIN events ON events.id = scouting_submissions.event_id
         JOIN teams ON teams.id = scouting_submissions.team_id
         LEFT JOIN users ON users.id = scouting_submissions.scouter_id
         ORDER BY scouting_submissions.created_at",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default()
}

/// One submission with names resolved and nulls coalesced for display.
pub async fn submission_detail(pool: &PgPool, id: i32) -> Option<SubmissionDetailRow> {
    sqlx::query_as(
        "SELECT scouting_submissions.id,
            events.name AS event_name,
            teams.team_number,
            teams.name AS team_name,
            users.name AS scout_name,
            COALESCE(scouting_submissions.alliance_color, '') AS alliance_color,
            COALESCE(scouting_submissions.notes, '') AS notes,
            COALESCE(scouting_submissions.starting_position, '') AS starting_position,
            COALESCE(scouting_submissions.defense_rating, '') AS defense_rating,
            COALESCE(scouting_submissions.traversal, '') AS traversal,
            COALESCE(scouting_submissions.scoring_strategy, '') AS scoring_strategy,
            COALESCE(scouting_submissions.shooting_speed, '') AS shooting_speed,
            COALESCE(scouting_submissions.capacity, '') AS capacity,
            COALESCE(scouting_submissions.defendability, '') AS defendability,
            COALESCE(scouting_submissions.hang_level, '') AS hang_level,
            COALESCE(scouting_submissions.auto_hang, '') AS auto_hang,
            COALESCE(scouting_submissions.hang_position, '') AS hang_position,
            TO_CHAR(scouting_submissions.created_at, 'YYYY-MM-DD HH24:MI:SS') AS created_at
         FROM scouting_submissions
         JOIN events ON events.id = scouting_submissions.event_id
         JOIN teams ON teams.id = scouting_submissions.team_id
         LEFT JOIN users ON users.id = scouting_submissions.scouter_id
         WHERE scouting_submissions.id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .unwrap_or(None)
}

/// The raw submission entity, as needed to copy it into `scouting_data`.
pub async fn find_submission(pool: &PgPool, id: i32) -> Option<ScoutingSubmission> {
    sqlx::query_as("SELECT * FROM scouting_submissions WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await
        .unwrap_or(None)
}

/// Promotes a queued submission into `scouting_data` and removes it.
///
/// Both statements share a transaction, so a submission can never be both
/// approved and still queued, nor lost without being recorded.
pub async fn approve_submission(
    pool: &PgPool,
    submission: &ScoutingSubmission,
) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;

    sqlx::query(
        "INSERT INTO scouting_data (event_id, team_id, alliance_color, notes, starting_position,
            defense_rating, traversal, scoring_strategy, shooting_speed, capacity, defendability,
            hang_level, auto_hang, hang_position, scouted_at, scouter_id, submitting_team_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)",
    )
    .bind(submission.event_id)
    .bind(submission.team_id)
    .bind(&submission.alliance_color)
    .bind(&submission.notes)
    .bind(&submission.starting_position)
    .bind(&submission.defense_rating)
    .bind(&submission.traversal)
    .bind(&submission.scoring_strategy)
    .bind(&submission.shooting_speed)
    .bind(&submission.capacity)
    .bind(&submission.defendability)
    .bind(&submission.hang_level)
    .bind(&submission.auto_hang)
    .bind(&submission.hang_position)
    .bind(submission.scouted_at)
    .bind(submission.scouter_id)
    .bind(submission.submitting_team_id)
    .execute(&mut *tx)
    .await?;

    sqlx::query("DELETE FROM scouting_submissions WHERE id = $1")
        .bind(submission.id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(())
}

/// Declines a submission by deleting it.
pub async fn delete_submission(pool: &PgPool, id: i32) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM scouting_submissions WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await
        .map(|_| ())
}

/// Approved rows for one team at one event, newest first — the order the
/// team page relies on to pick "latest" values.
pub async fn data_for_team_event(pool: &PgPool, team_id: i32, event_id: i32) -> Vec<ScoutingData> {
    sqlx::query_as(
        "SELECT * FROM scouting_data WHERE team_id = $1 AND event_id = $2 ORDER BY scouted_at DESC",
    )
    .bind(team_id)
    .bind(event_id)
    .fetch_all(pool)
    .await
    .unwrap_or_default()
}

/// The scored fields of every approved row at an event, for point rankings.
///
/// Nulls are coalesced to empty strings so scoring never has to unwrap; an
/// unrecognised or empty value is simply worth zero points.
pub async fn metrics_for_event(pool: &PgPool, event_id: i32) -> Vec<ScoutingMetricRow> {
    sqlx::query_as(
        "SELECT scouting_data.team_id,
                COALESCE(scouting_data.defense_rating, '') AS defense_rating,
                COALESCE(scouting_data.traversal, '') AS traversal,
                COALESCE(scouting_data.shooting_speed, '') AS shooting_speed,
                COALESCE(scouting_data.capacity, '') AS capacity,
                COALESCE(scouting_data.scoring_strategy, '') AS scoring_strategy,
                COALESCE(scouting_data.hang_level, '') AS hang_level,
                COALESCE(scouting_data.auto_hang, '') AS auto_hang,
                COALESCE(scouting_data.hang_position, '') AS hang_position,
                COALESCE(scouting_data.starting_position, '') AS starting_position
         FROM scouting_data
         WHERE scouting_data.event_id = $1",
    )
    .bind(event_id)
    .fetch_all(pool)
    .await
    .unwrap_or_default()
}
