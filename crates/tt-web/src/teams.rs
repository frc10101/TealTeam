//! The team profile page (U11): `/teams?event=…&team=…`.
//!
//! Everything comes from storage. The retired page ran a blocking FIRST sync
//! when a team had no local events, so a page render could wait many seconds on
//! a gymnasium's internet (REBUILD_SPEC.md 12.7). A team the server does not
//! know yet is said so, and the background sync brings it in.

use std::collections::HashMap;
use std::convert::Infallible;

use axum::extract::{FromRequestParts, Query};
use axum::http::request::Parts;
use chrono::Utc;
use tracing::warn;
use tt_core::connectivity::{describe_age, is_stale};
use tt_core::profile;
use tt_core::season::Payload;
use tt_repo::Repo;
use tt_templates::{
    EventLink, Nav, RosterEntry, TeamAtEvent, TeamCard, TeamMatchLine, TeamPage, stat_lines,
    summary_sections,
};

use crate::events::EventContext;
use crate::startup::AppState;

/// `?team=`, as typed. Never rejects.
#[derive(Debug, Default)]
pub struct TeamParam(pub Option<String>);

impl<S: Send + Sync> FromRequestParts<S> for TeamParam {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Infallible> {
        let raw = Query::<HashMap<String, String>>::from_request_parts(parts, state)
            .await
            .ok()
            .and_then(|Query(mut q)| q.remove("team"))
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty());
        Ok(Self(raw))
    }
}

pub async fn page(
    state: &AppState,
    nav: Nav,
    context: &EventContext,
    requested: &TeamParam,
) -> TeamPage {
    let mut page = TeamPage {
        title: "Teams".into(),
        nav,
        query: requested.0.clone().unwrap_or_default(),
        roster: Vec::new(),
        errors: Vec::new(),
        team: None,
        at_event: None,
        not_at_event: String::new(),
        other_events: Vec::new(),
    };
    let event = context.selected.as_ref();
    if let Some(unknown) = &context.unknown {
        page.errors
            .push(format!("There is no event “{unknown}” on this server."));
    }

    let roster = match event {
        Some(event) => state
            .repo
            .event_teams(&event.key)
            .await
            .unwrap_or_else(|e| {
                warn!("roster for {}: {e}", event.key);
                Vec::new()
            }),
        None => Vec::new(),
    };
    page.roster = roster
        .iter()
        .map(|t| RosterEntry {
            number: t.number,
            name: t.name.clone(),
            is_viewer: false,
        })
        .collect();

    let Some(raw) = &requested.0 else {
        return page;
    };
    let Some(number) = raw.parse::<i32>().ok().filter(|n| *n > 0) else {
        page.errors.push(format!(
            "“{raw}” is not a team number. Type just the digits, like 1678."
        ));
        return page;
    };
    let team = match state.repo.team(number).await {
        Ok(Some(team)) => team,
        Ok(None) => {
            page.errors.push(format!(
                "There is no team {number} on this server yet. Teams arrive with each event's \
                 roster; the background sync keeps them current."
            ));
            return page;
        }
        Err(e) => {
            warn!("loading team {number}: {e}");
            page.errors
                .push("Could not read that team. Reload to try again.".into());
            return page;
        }
    };
    page.title = format!("{} · {}", team.number, team.name);
    page.team = Some(TeamCard::new(&team));

    let events = state
        .repo
        .events_for_team(number)
        .await
        .unwrap_or_else(|e| {
            warn!("events for team {number}: {e}");
            Vec::new()
        });
    page.other_events = events
        .iter()
        .filter(|e| Some(&e.key) != event.map(|ev| &ev.key))
        .map(|e| EventLink::new(e, number))
        .collect();

    let Some(event) = event else {
        return page;
    };
    let loaded = async {
        let matches = state.repo.team_matches(&event.key, number).await?;
        let stats = state.repo.team_stats(&event.key, number).await?;
        let approved = state.repo.approved_observations(&event.key).await?;
        let pending = state.repo.pending_observations(&event.key).await?;
        tt_repo::Result::Ok((matches, stats, approved, pending))
    };
    let (matches, stats, approved, pending) = match loaded.await {
        Ok(loaded) => loaded,
        Err(e) => {
            warn!("team {number} at {}: {e}", event.key);
            page.errors.push(format!(
                "Could not read team {number} at {}. Reload to try again.",
                event.name
            ));
            return page;
        }
    };

    let payloads: Vec<&Payload> = approved
        .iter()
        .filter(|o| o.team_number == number && o.schema_version == state.season.version)
        .map(|o| &o.payload)
        .collect();
    let on_roster = roster.iter().any(|t| t.number == number);
    if !on_roster && matches.is_empty() && stats.is_none() && payloads.is_empty() {
        page.not_at_event = if page.other_events.is_empty() {
            format!("Team {number} is not at {}.", event.name)
        } else {
            format!(
                "Team {number} is not at {}. Its other events are below.",
                event.name
            )
        };
        return page;
    }

    let now = Utc::now();
    let synced_at = stats.as_ref().and_then(|s| s.synced_at);
    page.at_event = Some(TeamAtEvent {
        event_name: event.name.clone(),
        stats: stat_lines(stats.as_ref()),
        synced: synced_at
            .map(|at| describe_age(now - at))
            .unwrap_or_default(),
        stale: synced_at.is_some() && is_stale(synced_at, now),
        observed: payloads.len(),
        waiting: pending.iter().filter(|o| o.team_number == number).count(),
        sections: summary_sections(&profile::summarize(&state.season, &payloads)),
        matches: matches
            .iter()
            .filter_map(|m| TeamMatchLine::new(m, number))
            .collect(),
    });
    page
}
