//! The lead scout's assignment grid (L1).
//!
//! Every match at the selected event, its six robots, and who is watching each
//! one. Read-only for now: setting and distributing assignments is L2.

use tracing::warn;
use tt_repo::Repo;
use tt_templates::{AssignmentGrid, AssignmentsPage, Nav};

use crate::events::EventContext;
use crate::startup::AppState;

/// Assemble the grid for `context`'s event.
///
/// A failed read of the schedule or the assignments replaces the grid with a
/// message: a grid reading "Unassigned" everywhere because a query failed would
/// send a lead scout off to redo work that is already done. A failed roster
/// read only costs the team names.
pub async fn page(state: &AppState, nav: Nav, context: &EventContext) -> AssignmentsPage {
    let storage_ready = nav.storage_ready;
    let mut page = AssignmentsPage {
        title: "Assignments".into(),
        nav,
        event_name: String::new(),
        unavailable: String::new(),
        errors: Vec::new(),
        grid: None,
    };

    let Some(event) = &context.selected else {
        page.unavailable = if storage_ready {
            "No events have been loaded yet, so there is nothing to assign.".into()
        } else {
            "The server's storage is unavailable, so assignments cannot be shown.".into()
        };
        return page;
    };
    page.event_name = event.name.clone();
    if let Some(unknown) = &context.unknown {
        page.errors.push(format!(
            "There is no event “{unknown}” on this server. Showing {} instead.",
            event.name
        ));
    }

    let matches = match state.repo.event_matches(&event.key).await {
        Ok(matches) => matches,
        Err(e) => {
            warn!("matches for {}: {e}", event.key);
            page.unavailable = "Could not read the match schedule. Reload to try again.".into();
            return page;
        }
    };
    if matches.is_empty() {
        page.unavailable = format!(
            "{} has no match schedule yet. Assignments open once it is published.",
            event.name
        );
        return page;
    }

    let assignments = match state.repo.event_assignments(&event.key).await {
        Ok(assignments) => assignments,
        Err(e) => {
            warn!("assignments for {}: {e}", event.key);
            page.unavailable = "Could not read the assignments. Reload to try again.".into();
            return page;
        }
    };
    let roster = state
        .repo
        .event_teams(&event.key)
        .await
        .unwrap_or_else(|e| {
            warn!("roster for {}: {e}", event.key);
            Vec::new()
        });

    page.grid = Some(AssignmentGrid::new(&matches, &roster, &assignments));
    page
}
