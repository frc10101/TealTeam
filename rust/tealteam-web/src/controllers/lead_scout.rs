//! Lead scout panel: the submission review queue, team rankings, pick list,
//! and point weight settings.
//!
//! Everything here requires [`crate::models::User::can_lead`]; a signed-in
//! user without the role is redirected home rather than shown a 403, matching
//! the other ports.
//!
//! Rankings combine two independent sources — qualification rank synced from
//! TBA and scouting points computed from approved submissions — which is why
//! the panel loads the roster, the scouting metrics and the weight config
//! before handing all three to the view.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum_extra::extract::cookie::CookieJar;
use std::collections::HashMap;
use tracing::{error, warn};

use crate::models::{scouting, scouting_points, team};
use crate::services::first_sync;
use crate::state::SharedState;
use crate::views::lead_scout::{
    AdminViewerTemplate, PendingSubmissionRow, PickListTeamRow, SubmissionDetail,
    SubmissionDetailTemplate, TeamPointSummary, WeightsTemplate,
};
use crate::views::{render, Nav};
use crate::web::*;

/// `GET /lead-scout` — review queue, rankings and pick list for the selected
/// event. `team_sort` in the query string picks the rankings column.
pub async fn admin_viewer(
    State(state): State<SharedState>,
    jar: CookieJar,
    Query(query): Query<HashMap<String, String>>,
) -> HandlerResult {
    let Some(user) = current_user(&state.pool, &jar).await else {
        return Ok(redirect("/sign-in"));
    };
    if !user.can_lead() {
        return Ok(redirect("/"));
    }

    let team_sort = query.get("team_sort").cloned().unwrap_or_default();
    let selected_event_id = selected_event_id(&state.pool, &jar).await;

    let mut team_rankings = Vec::new();
    let mut pick_list_teams = Vec::new();
    if let Some(event_id) = selected_event_id {
        let roster = team::roster_with_rank(&state.pool, event_id).await;
        let metrics = scouting::metrics_for_event(&state.pool, event_id).await;
        let cfg = scouting_points::load_effective_config(&state.pool).await;
        let totals = scouting_points::totals_by_team(&metrics, &cfg);

        team_rankings =
            TeamPointSummary::build(roster.clone(), &totals.points, &totals.matches, &team_sort);
        pick_list_teams = PickListTeamRow::from_roster(roster);
    }

    let pending_submissions = scouting::pending_submissions(&state.pool)
        .await
        .into_iter()
        .map(PendingSubmissionRow::from_model)
        .collect();

    Ok(render(&AdminViewerTemplate::new(
        Nav::from_user(Some(&user)),
        selected_event_id.is_some(),
        pending_submissions,
        team_rankings,
        pick_list_teams,
        &team_sort,
    )))
}

/// `GET /lead-scout/submissions/:id` — one submission in full.
pub async fn view_submission(
    State(state): State<SharedState>,
    jar: CookieJar,
    Path(id): Path<i32>,
) -> HandlerResult {
    let Some(user) = current_user(&state.pool, &jar).await else {
        return Ok(redirect("/"));
    };
    if !user.can_lead() {
        return Ok(redirect("/"));
    }

    let submission = scouting::submission_detail(&state.pool, id)
        .await
        .map(SubmissionDetail::from_row);

    Ok(render(&SubmissionDetailTemplate {
        title: "Submission Details".to_string(),
        nav: Nav::from_user(Some(&user)),
        submission,
    }))
}

/// `POST /hx/lead-scout/submissions/:id/approve` — accepts a submission into
/// `scouting_data`.
///
/// Legacy submissions predate `submitting_team_id`, so it is resolved from the
/// scouter before the move; without it the notes would be visible to nobody.
/// Approval also refreshes the team from FIRST in the background.
pub async fn approve_submission(
    State(state): State<SharedState>,
    jar: CookieJar,
    Path(id): Path<i32>,
) -> HandlerResult {
    let Some(user) = current_user(&state.pool, &jar).await else {
        return Ok(redirect("/"));
    };
    if !user.can_lead() {
        return Ok(redirect("/"));
    }

    let Some(mut submission) = scouting::find_submission(&state.pool, id).await else {
        return Ok((StatusCode::NOT_FOUND, "Submission not found").into_response());
    };

    let Some(team_number) = team::number_by_id(&state.pool, submission.team_id).await else {
        return Ok((StatusCode::INTERNAL_SERVER_ERROR, "Team not found").into_response());
    };

    // Carry team attribution into scouting_data (drives the notes privacy
    // filter). Resolve from the scouter for legacy submissions.
    if submission.submitting_team_id.is_none() {
        if let Some(scouter_id) = submission.scouter_id {
            submission.submitting_team_id = team::id_for_user(&state.pool, scouter_id).await;
        }
    }

    if let Err(e) = scouting::approve_submission(&state.pool, &submission).await {
        error!("failed to approve submission {id}: {e}");
        return Ok((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to approve submission",
        )
            .into_response());
    }

    // Refresh the team's info from FIRST in the background after approval.
    let pool = state.pool.clone();
    tokio::spawn(async move {
        if let Err(e) = first_sync::sync_team_for_user(&pool, team_number).await {
            warn!("failed to sync team after approval (team {team_number}): {e}");
        }
    });

    Ok(up_navigate("/lead-scout"))
}

/// `POST /hx/lead-scout/submissions/:id/decline` — discards a submission.
pub async fn decline_submission(
    State(state): State<SharedState>,
    jar: CookieJar,
    Path(id): Path<i32>,
) -> HandlerResult {
    let Some(user) = current_user(&state.pool, &jar).await else {
        return Ok(redirect("/"));
    };
    if !user.can_lead() {
        return Ok(redirect("/"));
    }

    if let Err(e) = scouting::delete_submission(&state.pool, id).await {
        error!("failed to decline submission {id}: {e}");
        return Ok((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to decline submission",
        )
            .into_response());
    }

    Ok(up_navigate("/lead-scout"))
}

/// `GET /lead-scout/weights` — the point weight form, sorted by label.
pub async fn weights_page(
    State(state): State<SharedState>,
    jar: CookieJar,
    Query(query): Query<HashMap<String, String>>,
) -> HandlerResult {
    let Some(user) = current_user(&state.pool, &jar).await else {
        return Ok(redirect("/sign-in"));
    };
    if !user.can_lead() {
        return Ok(redirect("/"));
    }

    let mut sections = scouting_points::load_sections(&state.pool).await;
    sections.sort_by(|a, b| a.label.cmp(&b.label));

    Ok(render(&WeightsTemplate::new(
        Nav::from_user(Some(&user)),
        sections,
        query.get("updated").map(|v| v == "1").unwrap_or(false),
        query.get("error").cloned().unwrap_or_default(),
    )))
}

/// `POST /lead-scout/weights` — saves the weights and returns to the rankings
/// sorted by points, so the effect of the change is immediately visible.
///
/// An invalid or unsavable value redirects back to the form with the reason in
/// the query string.
pub async fn weights_update(
    State(state): State<SharedState>,
    jar: CookieJar,
    body: String,
) -> HandlerResult {
    let Some(user) = current_user(&state.pool, &jar).await else {
        return Ok(redirect("/sign-in"));
    };
    if !user.can_lead() {
        return Ok(redirect("/"));
    }

    let base_sections = scouting_points::load_sections(&state.pool).await;
    let form = form_map(&body);
    let Some(parsed) = scouting_points::parse_sections_from_form(&form, &base_sections) else {
        return Ok(redirect("/lead-scout/weights?error=Invalid+weight+value"));
    };

    if let Err(e) = scouting_points::persist_sections(&state.pool, &parsed).await {
        error!("failed to persist scouting point weights: {e}");
        return Ok(redirect("/lead-scout/weights?error=Failed+to+save+weights"));
    }

    Ok(redirect("/lead-scout?team_sort=points"))
}
