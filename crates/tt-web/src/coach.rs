//! The drive coach panel (U18): `/drive-coach`.
//!
//! Reads the local schedule and statistics only. The retired panel fetched the
//! FIRST schedule live and was empty at every event without internet
//! (REBUILD_SPEC.md 12.6).

use chrono::Utc;
use tracing::warn;
use tt_core::connectivity::Freshness;
use tt_core::user::User;
use tt_repo::Repo;
use tt_templates::{DriveCoachPage, Nav};

use crate::events::EventContext;
use crate::startup::AppState;

pub async fn page(
    state: &AppState,
    user: &User,
    nav: Nav,
    context: &EventContext,
) -> DriveCoachPage {
    let storage_ready = nav.storage_ready;
    let mut page = DriveCoachPage {
        title: "Drive Coach".into(),
        nav,
        event_name: String::new(),
        team: user.team_number.map(|n| n.to_string()).unwrap_or_default(),
        unavailable: String::new(),
        errors: Vec::new(),
        next: None,
        later: Vec::new(),
        played: Vec::new(),
        live_href: String::new(),
        stats_synced: String::new(),
        stats_stale: false,
    };
    let Some(team) = user.team_number else {
        page.unavailable = "Your account has no team number, so there is no schedule to \
            follow. An admin can add it."
            .into();
        return page;
    };
    let Some(event) = &context.selected else {
        page.unavailable = if storage_ready {
            "No events have been loaded yet.".into()
        } else {
            "The server's storage is unavailable, so the schedule cannot be shown.".into()
        };
        return page;
    };
    page.event_name = event.name.clone();
    page.live_href = format!("/drive-coach?event={}", event.key);
    if let Some(unknown) = &context.unknown {
        page.errors.push(format!(
            "There is no event “{unknown}” on this server. Showing {} instead.",
            event.name
        ));
    }

    let loaded = async {
        let matches = state.repo.event_matches(&event.key).await?;
        let stats = state.repo.event_stats(&event.key).await?;
        tt_repo::Result::Ok((matches, stats))
    };
    let (matches, stats) = match loaded.await {
        Ok(loaded) => loaded,
        Err(e) => {
            warn!("coach schedule for {}: {e}", event.key);
            page.unavailable = "Could not read the schedule. Reload to try again.".into();
            return page;
        }
    };
    if matches.is_empty() {
        page.unavailable = format!(
            "{} has no match schedule yet. It appears here as soon as it is published.",
            event.name
        );
        return page;
    }
    let now = Utc::now();
    page.schedule(event, &matches, team, &stats, now);
    let synced = stats
        .iter()
        .filter(|s| s.opr.is_some() || s.dpr.is_some())
        .filter_map(|s| s.synced_at)
        .max()
        .map(|at| Freshness::of(at, now, event.is_running(now)));
    page.stats_synced = synced.as_ref().map(|f| f.age.clone()).unwrap_or_default();
    page.stats_stale = synced.is_some_and(|f| f.stale);
    if page.next.is_none() && page.later.is_empty() && page.played.is_empty() {
        page.unavailable = format!("Team {team} is not on the schedule at {}.", event.name);
    }
    page
}
