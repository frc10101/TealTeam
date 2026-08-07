// Lead scout panel: pending submission review, team rankings, pick list, and
// point weight settings (port of Controllers/LeadScoutController.cs).

use askama::Template;
use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum_extra::extract::cookie::CookieJar;
use sqlx::PgPool;
use std::collections::HashMap;
use tracing::{error, warn};

use super::*;
use crate::scouting_points::{self, ScoutingMetricRow, ScoutingPointSection};
use crate::state::SharedState;

// ── View model types ──────────────────────────────────────────────────────

pub struct PendingSubmissionRow {
    pub id: i32,
    pub event_name: String,
    pub team_number: i32,
    pub team_name: String,
    pub scout_name: String,
    pub flag_label: String,
    pub flag_class: String,
}

pub struct PickListTeamRow {
    pub team_number: i32,
    pub team_name: String,
    pub rank: Option<i32>,
    pub rank_display: String,
}

pub struct TeamPointSummary {
    pub team_number: i32,
    pub team_name: String,
    pub rank: Option<i32>,
    pub rank_display: String,
    pub points: i32,
    pub matches: i32,
}

#[derive(sqlx::FromRow)]
struct SubmissionDetailDbRow {
    id: i32,
    event_name: String,
    team_number: i32,
    team_name: String,
    scout_name: Option<String>,
    alliance_color: String,
    notes: String,
    starting_position: String,
    defense_rating: String,
    traversal: String,
    scoring_strategy: String,
    shooting_speed: String,
    capacity: String,
    defendability: String,
    hang_level: String,
    auto_hang: String,
    hang_position: String,
    created_at: String,
}

pub struct SubmissionDetail {
    pub id: i32,
    pub event_name: String,
    pub team_number: i32,
    pub team_name: String,
    pub scout_name: String,
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
    pub flag_label: String,
    pub flag_class: String,
    pub created_at: String,
}

// ── Templates ─────────────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "pages/admin_viewer.html")]
struct AdminViewerTemplate {
    title: String,
    description: String,
    nav: Nav,
    has_event: bool,
    pending_submissions: Vec<PendingSubmissionRow>,
    team_rankings: Vec<TeamPointSummary>,
    pick_list_teams: Vec<PickListTeamRow>,
    sort_rank_class: String,
    sort_points_class: String,
    sort_number_class: String,
    sort_name_class: String,
}

#[derive(Template)]
#[template(path = "pages/submission_detail.html")]
struct SubmissionDetailTemplate {
    title: String,
    nav: Nav,
    submission: Option<SubmissionDetail>,
}

#[derive(Template)]
#[template(path = "pages/lead_scout_weights.html")]
struct WeightsTemplate {
    title: String,
    description: String,
    nav: Nav,
    sections: Vec<ScoutingPointSection>,
    updated: bool,
    error: String,
}

fn sort_class(active: &str, keys: &[&str]) -> String {
    if keys.contains(&active) {
        "text-teal-300".to_string()
    } else {
        "text-gray-400 hover:text-gray-300".to_string()
    }
}

fn rank_display(rank: Option<i32>) -> String {
    rank.map(|r| format!("#{r}")).unwrap_or_else(|| "-".to_string())
}

// ── Handlers ──────────────────────────────────────────────────────────────

pub async fn admin_viewer(
    State(state): State<SharedState>,
    jar: CookieJar,
    Query(query): Query<HashMap<String, String>>,
) -> HandlerResult {
    let Some(user) = current_user(&state.pool, &jar).await else {
        return Ok(redirect("/sign-in"));
    };
    if !user.is_admin && !user.is_lead_scout {
        return Ok(redirect("/"));
    }

    let team_sort = query.get("team_sort").cloned().unwrap_or_default();
    let session = current_session(&state.pool, &jar).await;
    let selected_event_id = session.and_then(|s| s.selected_event_id);

    let mut team_rankings = Vec::new();
    let mut pick_list_teams = Vec::new();
    if let Some(event_id) = selected_event_id {
        team_rankings = load_team_point_rankings(&state.pool, event_id, &team_sort).await;
        pick_list_teams = load_pick_list_teams(&state.pool, event_id).await;
    }
    let pending_submissions = load_pending_submissions(&state.pool).await;

    Ok(render(&AdminViewerTemplate {
        title: "Lead Scout Panel".to_string(),
        description: "Approve submissions, review rankings, and coordinate match strategy."
            .to_string(),
        nav: Nav::from_user(Some(&user)),
        has_event: selected_event_id.is_some(),
        pending_submissions,
        team_rankings,
        pick_list_teams,
        sort_rank_class: sort_class(&team_sort, &["", "rank"]),
        sort_points_class: sort_class(&team_sort, &["points"]),
        sort_number_class: sort_class(&team_sort, &["number"]),
        sort_name_class: sort_class(&team_sort, &["name"]),
    }))
}

async fn load_pending_submissions(pool: &PgPool) -> Vec<PendingSubmissionRow> {
    let rows: Vec<(i32, String, i32, String, Option<String>, Option<String>)> = sqlx::query_as(
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
    .unwrap_or_default();

    rows.into_iter()
        .map(|(id, event_name, team_number, team_name, scout_name, notes)| {
            let missing = notes.as_deref().map(|n| n.trim().is_empty()).unwrap_or(true);
            PendingSubmissionRow {
                id,
                event_name,
                team_number,
                team_name,
                scout_name: scout_name.filter(|s| !s.trim().is_empty()).unwrap_or_else(|| "Unknown".to_string()),
                flag_label: if missing { "Missing notes" } else { "Clean" }.to_string(),
                flag_class: if missing { "text-yellow-300" } else { "text-teal-300" }.to_string(),
            }
        })
        .collect()
}

async fn load_pick_list_teams(pool: &PgPool, event_id: i32) -> Vec<PickListTeamRow> {
    let rows: Vec<(i32, String, Option<i32>)> = sqlx::query_as(
        "SELECT teams.team_number, teams.name AS team_name, team_event_stats.rank
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
    .unwrap_or_default();

    rows.into_iter()
        .map(|(team_number, team_name, rank)| PickListTeamRow {
            team_number,
            team_name,
            rank,
            rank_display: rank.map(|r| format!("#{r}")).unwrap_or_default(),
        })
        .collect()
}

async fn load_team_point_rankings(
    pool: &PgPool,
    event_id: i32,
    sort_key: &str,
) -> Vec<TeamPointSummary> {
    let teams: Vec<(i32, i32, String, Option<i32>)> = sqlx::query_as(
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
    .unwrap_or_default();

    let metrics: Vec<ScoutingMetricRow> = sqlx::query_as(
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
    .unwrap_or_default();

    let cfg = scouting_points::load_effective_config(pool).await;
    let mut points_by_team: HashMap<i32, i32> = HashMap::new();
    let mut matches_by_team: HashMap<i32, i32> = HashMap::new();
    for row in &metrics {
        *points_by_team.entry(row.team_id).or_insert(0) +=
            scouting_points::calculate_scouting_points(row, &cfg);
        *matches_by_team.entry(row.team_id).or_insert(0) += 1;
    }

    let mut summaries: Vec<TeamPointSummary> = teams
        .into_iter()
        .map(|(team_id, team_number, team_name, rank)| TeamPointSummary {
            team_number,
            team_name,
            rank,
            rank_display: rank_display(rank),
            points: points_by_team.get(&team_id).copied().unwrap_or(0),
            matches: matches_by_team.get(&team_id).copied().unwrap_or(0),
        })
        .collect();

    let sort_key = {
        let k = sort_key.trim().to_lowercase();
        if k.is_empty() { "rank".to_string() } else { k }
    };

    summaries.sort_by(|left, right| {
        use std::cmp::Ordering;
        let primary = match sort_key.as_str() {
            "points" => right.points.cmp(&left.points),
            "name" => left
                .team_name
                .trim()
                .to_lowercase()
                .cmp(&right.team_name.trim().to_lowercase()),
            "number" => left.team_number.cmp(&right.team_number),
            _ => match (left.rank, right.rank) {
                (Some(a), Some(b)) => a.cmp(&b),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            },
        };
        primary
            .then_with(|| left.team_number.cmp(&right.team_number))
            .then_with(|| left.team_name.cmp(&right.team_name))
    });

    summaries
}

pub async fn view_submission(
    State(state): State<SharedState>,
    jar: CookieJar,
    Path(id): Path<i32>,
) -> HandlerResult {
    let Some(user) = current_user(&state.pool, &jar).await else {
        return Ok(redirect("/"));
    };
    if !user.is_admin && !user.is_lead_scout {
        return Ok(redirect("/"));
    }

    let row: Option<SubmissionDetailDbRow> = sqlx::query_as(
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
    .fetch_optional(&state.pool)
    .await
    .unwrap_or(None);

    let submission = row.map(|r| {
        let missing = r.notes.trim().is_empty();
        SubmissionDetail {
            id: r.id,
            event_name: r.event_name,
            team_number: r.team_number,
            team_name: r.team_name,
            scout_name: r.scout_name.filter(|s| !s.trim().is_empty()).unwrap_or_else(|| "Unknown".to_string()),
            alliance_color: r.alliance_color,
            notes: r.notes,
            starting_position: r.starting_position,
            defense_rating: r.defense_rating,
            traversal: r.traversal,
            scoring_strategy: r.scoring_strategy,
            shooting_speed: r.shooting_speed,
            capacity: r.capacity,
            defendability: r.defendability,
            hang_level: r.hang_level,
            auto_hang: r.auto_hang,
            hang_position: r.hang_position,
            flag_label: if missing { "Missing notes" } else { "Clean" }.to_string(),
            flag_class: if missing { "text-yellow-300" } else { "text-teal-300" }.to_string(),
            created_at: r.created_at,
        }
    });

    Ok(render(&SubmissionDetailTemplate {
        title: "Submission Details".to_string(),
        nav: Nav::from_user(Some(&user)),
        submission,
    }))
}

pub async fn approve_submission(
    State(state): State<SharedState>,
    jar: CookieJar,
    Path(id): Path<i32>,
) -> HandlerResult {
    let Some(user) = current_user(&state.pool, &jar).await else {
        return Ok(redirect("/"));
    };
    if !user.is_admin && !user.is_lead_scout {
        return Ok(redirect("/"));
    }

    let submission: Option<crate::models::ScoutingSubmission> =
        sqlx::query_as("SELECT * FROM scouting_submissions WHERE id = $1")
            .bind(id)
            .fetch_optional(&state.pool)
            .await
            .unwrap_or(None);
    let Some(mut submission) = submission else {
        return Ok((axum::http::StatusCode::NOT_FOUND, "Submission not found").into_response());
    };

    let team_number: Option<i32> =
        sqlx::query_scalar("SELECT team_number FROM teams WHERE id = $1")
            .bind(submission.team_id)
            .fetch_optional(&state.pool)
            .await
            .unwrap_or(None);
    let Some(team_number) = team_number else {
        return Ok((axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Team not found").into_response());
    };

    // Carry team attribution into scouting_data (drives the notes privacy
    // filter). Resolve from the scouter for legacy submissions.
    if submission.submitting_team_id.is_none() {
        if let Some(scouter_id) = submission.scouter_id {
            submission.submitting_team_id = sqlx::query_scalar(
                "SELECT t.id FROM users u JOIN teams t ON t.team_number = u.team_number WHERE u.id = $1",
            )
            .bind(scouter_id)
            .fetch_optional(&state.pool)
            .await
            .unwrap_or(None);
        }
    }

    let mut tx = match state.pool.begin().await {
        Ok(tx) => tx,
        Err(e) => {
            error!("failed to approve submission {id}: {e}");
            return Ok((axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Failed to approve submission").into_response());
        }
    };

    let insert = sqlx::query(
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
    .await;

    if let Err(e) = insert {
        error!("failed to approve submission {id}: {e}");
        return Ok((axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Failed to approve submission").into_response());
    }

    if let Err(e) = sqlx::query("DELETE FROM scouting_submissions WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await
    {
        error!("failed to approve submission {id}: {e}");
        return Ok((axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Failed to approve submission").into_response());
    }

    if let Err(e) = tx.commit().await {
        error!("failed to approve submission {id}: {e}");
        return Ok((axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Failed to approve submission").into_response());
    }

    // Refresh the team's info from FIRST in the background after approval.
    let pool = state.pool.clone();
    tokio::spawn(async move {
        if let Err(e) = crate::first_sync::sync_team_for_user(&pool, team_number).await {
            warn!("failed to sync team after approval (team {team_number}): {e}");
        }
    });

    Ok(hx_redirect_header(().into_response(), "/lead-scout"))
}

pub async fn decline_submission(
    State(state): State<SharedState>,
    jar: CookieJar,
    Path(id): Path<i32>,
) -> HandlerResult {
    let Some(user) = current_user(&state.pool, &jar).await else {
        return Ok(redirect("/"));
    };
    if !user.is_admin && !user.is_lead_scout {
        return Ok(redirect("/"));
    }

    if let Err(e) = sqlx::query("DELETE FROM scouting_submissions WHERE id = $1")
        .bind(id)
        .execute(&state.pool)
        .await
    {
        error!("failed to decline submission {id}: {e}");
        return Ok((axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Failed to decline submission").into_response());
    }

    Ok(hx_redirect_header(().into_response(), "/lead-scout"))
}

pub async fn weights_page(
    State(state): State<SharedState>,
    jar: CookieJar,
    Query(query): Query<HashMap<String, String>>,
) -> HandlerResult {
    let Some(user) = current_user(&state.pool, &jar).await else {
        return Ok(redirect("/sign-in"));
    };
    if !user.is_admin && !user.is_lead_scout {
        return Ok(redirect("/"));
    }

    let mut sections = scouting_points::load_sections(&state.pool).await;
    sections.sort_by(|a, b| a.label.cmp(&b.label));

    Ok(render(&WeightsTemplate {
        title: "Point Weight Settings".to_string(),
        description: "Adjust category point values used for team ranking scores.".to_string(),
        nav: Nav::from_user(Some(&user)),
        sections,
        updated: query.get("updated").map(|v| v == "1").unwrap_or(false),
        error: query.get("error").cloned().unwrap_or_default(),
    }))
}

pub async fn weights_update(
    State(state): State<SharedState>,
    jar: CookieJar,
    body: String,
) -> HandlerResult {
    let Some(user) = current_user(&state.pool, &jar).await else {
        return Ok(redirect("/sign-in"));
    };
    if !user.is_admin && !user.is_lead_scout {
        return Ok(redirect("/"));
    }

    let base_sections = scouting_points::load_sections(&state.pool).await;
    let form = form_map(&body);
    let parsed = scouting_points::parse_sections_from_form(&form, &base_sections);
    let Some(parsed) = parsed else {
        return Ok(redirect("/lead-scout/weights?error=Invalid+weight+value"));
    };

    if let Err(e) = scouting_points::persist_sections(&state.pool, &parsed).await {
        error!("failed to persist scouting point weights: {e}");
        return Ok(redirect("/lead-scout/weights?error=Failed+to+save+weights"));
    }

    Ok(redirect("/lead-scout?team_sort=points"))
}
