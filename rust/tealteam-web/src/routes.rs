//! URL table: every route maps to one controller action.
//!
//! Paths and methods match the Go and .NET ports exactly, so the same
//! templates, JavaScript and bookmarked links work against any of them.
//! Three prefixes, by convention:
//!
//! - bare paths (`/teams`, `/lead-scout`) render full HTML pages;
//! - `/hx/*` returns HTML fragments swapped in by Unpoly (the prefix is a
//!   leftover from the HTMX version the UI was ported from);
//! - `/api/*` returns JSON, or performs an action and redirects.
//!
//! Authorization is not expressed here — each controller action checks the
//! session itself, so an unauthorized request can redirect to the sign-in page
//! or return the right shape of error for its route.

use axum::routing::{delete, get, post};
use axum::Router;

use crate::controllers::{
    api, assignments, auth, coach, db_viewer, home, lead_scout, matches, submission, teams,
};
use crate::state::SharedState;

/// Builds the application router with the shared state attached.
///
/// `/static` is mounted by [`crate::main`] rather than here, since it is a
/// file service rather than a controller.
pub fn router(state: SharedState) -> Router {
    Router::new()
        // Pages
        .route("/", get(home::index))
        .route("/help", get(home::help))
        .route("/api/events/select", post(home::select_event))
        .route("/hx/events/summary", get(home::event_summary))
        // Auth
        .route("/sign-in", get(auth::sign_in_page))
        .route("/sign-up", get(auth::sign_up_page))
        .route("/api/auth/login", post(auth::login))
        .route("/api/auth/signup", post(auth::signup))
        .route("/api/auth/logout", post(auth::logout))
        .route("/account", get(auth::account_page))
        .route("/api/account/change-password", post(auth::change_password))
        // Scouting submission
        .route("/submission", get(submission::submission_page).post(submission::submit))
        .route("/submission/event-teams", get(submission::event_teams))
        // Lead scout
        .route("/lead-scout", get(lead_scout::admin_viewer))
        .route("/lead-scout/submissions/:id", get(lead_scout::view_submission))
        .route("/hx/lead-scout/submissions/:id/approve", post(lead_scout::approve_submission))
        .route("/hx/lead-scout/submissions/:id/decline", post(lead_scout::decline_submission))
        .route("/lead-scout/weights", get(lead_scout::weights_page).post(lead_scout::weights_update))
        // Assignments
        .route("/lead-scout/assignments", get(assignments::assignments_page))
        .route("/hx/assignments/set", post(assignments::set_assignment))
        .route("/hx/assignments/auto", post(assignments::auto_distribute))
        .route("/hx/assignments/clear-all", post(assignments::clear_all))
        .route("/hx/assignments/clear-match/:match_id", post(assignments::clear_match))
        .route("/hx/devices/:id/rename", post(assignments::rename_device))
        .route("/api/device/heartbeat", post(assignments::heartbeat))
        // Teams
        .route("/teams", get(teams::team_page))
        .route("/hx/teams/search", get(teams::team_search))
        .route("/hx/teams/fetch-past-events", post(teams::fetch_past_events))
        .route("/hx/teams/data", get(teams::team_event_data))
        // Matches / coach
        .route("/hx/matches/schedule", get(matches::match_schedule))
        .route("/drive-coach", get(coach::coach_viewer))
        .route("/hx/drive-coach/matches", get(coach::drive_coach_matches))
        // DB viewer
        .route("/development/db", get(db_viewer::db_viewer))
        .route("/hx/development/db/table/:name", get(db_viewer::table_content))
        // JSON APIs
        .route("/api/frc/sync", post(api::frc_sync))
        .route("/api/pick-list", get(api::get_pick_list))
        .route("/api/pick-list/entry", post(api::save_pick_list_entry))
        .route("/api/pick-list/entry", delete(api::delete_pick_list_entry))
        .route("/hx/network/status", get(api::network_status_badge))
        .route("/api/network/status", get(api::network_status))
        .with_state(state)
}
