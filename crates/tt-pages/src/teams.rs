//! The team profile page (U11): `/teams?event=…&team=…`.
//!
//! Everything comes from storage. The retired page ran a blocking FIRST sync
//! when a team had no local events, so a page render could wait many seconds on
//! a gymnasium's internet (REBUILD_SPEC.md 12.7). A team the server does not
//! know yet is said so, and the background sync brings it in.
//!
//! The first page a device can make for itself (C5): with no server, the
//! service worker runs this over the device's copy.

use chrono::{DateTime, Utc};
use tracing::warn;
use tt_core::connectivity::{Freshness, describe_age};
use tt_core::notes::{self, Notes};
use tt_core::profile;
use tt_core::records::MatchRecord;
use tt_core::season::{FieldKind, Payload, SeasonSchema};
use tt_repo::{LocalRepo, StoredObservation};
use tt_templates::{
    EventLink, Nav, NoteLine, RosterEntry, TeamAtEvent, TeamCard, TeamMatchLine, TeamPage,
    stat_lines, summary_sections,
};

use crate::events::EventContext;

/// `?team=`'s value, as typed, if it is not blank. Never rejects: the page
/// says what is wrong with it.
pub fn requested(raw: Option<&str>) -> Option<String> {
    raw.map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
}

/// The page for the team `requested` names, at the event `context` selected.
///
/// `viewer_team` decides whose notes are shown (U13): only that team's.
pub async fn page<R: LocalRepo>(
    repo: &R,
    season: &SeasonSchema,
    nav: Nav,
    viewer_team: Option<i32>,
    context: &EventContext,
    requested: Option<&str>,
    now: DateTime<Utc>,
) -> TeamPage {
    let mut page = TeamPage {
        title: "Teams".into(),
        nav,
        query: requested.unwrap_or_default().to_string(),
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
        Some(event) => repo.event_teams(&event.key).await.unwrap_or_else(|e| {
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

    let Some(raw) = requested else {
        return page;
    };
    let Some(number) = raw.parse::<i32>().ok().filter(|n| *n > 0) else {
        page.errors.push(format!(
            "“{raw}” is not a team number. Type just the digits, like 1678."
        ));
        return page;
    };
    let team = match repo.team(number).await {
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

    let events = repo.events_for_team(number).await.unwrap_or_else(|e| {
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
        let matches = repo.team_matches(&event.key, number).await?;
        let stats = repo.team_stats(&event.key, number).await?;
        let approved = repo.approved_observations(&event.key).await?;
        let pending = repo.pending_observations(&event.key).await?;
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

    let observed: Vec<&StoredObservation> = approved
        .iter()
        .filter(|o| o.team_number == number && o.schema_version == season.version)
        .collect();
    let payloads: Vec<&Payload> = observed.iter().map(|o| &o.payload).collect();
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

    let synced = stats
        .as_ref()
        .and_then(|s| s.synced_at)
        .map(|at| Freshness::of(at, now, event.is_running(now)));
    page.at_event = Some(TeamAtEvent {
        event_name: event.name.clone(),
        stats: stat_lines(stats.as_ref()),
        synced: synced.as_ref().map(|f| f.age.clone()).unwrap_or_default(),
        stale: synced.is_some_and(|f| f.stale),
        observed: payloads.len(),
        latest_scouted: latest_scouted(&observed, now),
        waiting: pending.iter().filter(|o| o.team_number == number).count(),
        sections: summary_sections(&profile::summarize(season, &payloads), payloads.len()),
        notes: note_lines(season, viewer_team, &observed, &matches),
        notes_team: viewer_team,
        notes_href: crate::notes_href(&event.key, number),
        graph_href: crate::graph_href(&event.key, number),
        matches: matches
            .iter()
            .filter_map(|m| TeamMatchLine::new(m, number))
            .collect(),
    });
    page
}

/// When the newest of `observed` was recorded (U14): `"12 minutes ago"`,
/// empty when there are none.
pub fn latest_scouted(observed: &[&StoredObservation], now: DateTime<Utc>) -> String {
    observed
        .iter()
        .filter_map(|o| o.observed_at.or(o.created_at))
        .max()
        .map(|at| describe_age(now - at))
        .unwrap_or_default()
}

/// The notes `viewer_team` may read on `observed`, in match order: their own
/// team's and no one else's (U13).
fn note_lines(
    season: &SeasonSchema,
    viewer_team: Option<i32>,
    observed: &[&StoredObservation],
    matches: &[MatchRecord],
) -> Vec<NoteLine> {
    let mut readable: Vec<&StoredObservation> = observed
        .iter()
        .copied()
        .filter(|o| Notes::for_viewer(viewer_team, o.submitting_team).shown())
        .collect();
    // Stable, and `None` sorts first: a match gone from the schedule leads.
    readable.sort_by_key(|o| matches.iter().position(|m| m.key == o.match_key));
    // With one text field on the form, its label says nothing the heading
    // does not.
    let one_field = season
        .fields()
        .filter(|f| matches!(f.kind, FieldKind::Text { .. }))
        .count()
        <= 1;
    readable
        .iter()
        .flat_map(|o| {
            let label = matches
                .iter()
                .find(|m| m.key == o.match_key)
                .map(MatchRecord::label)
                .unwrap_or_else(|| o.match_key.clone());
            let scout = o
                .scouter_name
                .clone()
                .unwrap_or_else(|| "a scout whose account is gone".into());
            notes::written(season, &o.payload)
                .into_iter()
                .map(move |(field, text)| NoteLine {
                    heading: format!("{label} · {scout}"),
                    label: if one_field { String::new() } else { field },
                    text,
                })
        })
        .collect()
}
