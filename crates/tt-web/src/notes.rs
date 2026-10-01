//! The notes view (U22): `/notes?event=…&team=…&scout=…&q=…&order=…`.
//!
//! Every note the viewer's team wrote on approved observations at the selected
//! event, newest first or in match order, each with when it was recorded on
//! the event's clock. Which notes are readable is U13's rule and nothing else;
//! the filters only narrow that. Everything is in the URL, so a narrowed view
//! can be bookmarked or linked to, as the team profile does.

use std::collections::HashMap;

use chrono::Utc;
use tracing::warn;
use tt_core::connectivity::describe_age;
use tt_core::notes::{self, Filter, Notes, Order};
use tt_core::records::MatchRecord;
use tt_core::season::FieldKind;
use tt_core::user::User;
use tt_repo::{Repo, StoredObservation};
use tt_templates::{FilterOption, Nav, NoteEntry, NotesPage, team_href};

use crate::events::EventContext;
use crate::startup::AppState;

/// `/notes` for `event_key`, narrowed to `team`.
pub fn href(event_key: &str, team: i32) -> String {
    format!("/notes?event={event_key}&team={team}")
}

pub async fn page(
    state: &AppState,
    nav: Nav,
    viewer: &User,
    context: &EventContext,
    query: &HashMap<String, String>,
) -> NotesPage {
    let storage_ready = nav.storage_ready;
    let typed = |name: &str| query.get(name).map(|v| v.trim()).unwrap_or_default();
    let order = Order::parse(typed("order"));
    let filter = Filter {
        team: typed("team").parse().ok(),
        scout: typed("scout").parse().ok(),
        words: Filter::words(typed("q")),
    };
    let mut page = NotesPage {
        title: "Notes".into(),
        nav,
        event_name: String::new(),
        unavailable: String::new(),
        errors: Vec::new(),
        own_team: 0,
        teams: Vec::new(),
        scouts: Vec::new(),
        query: typed("q").to_string(),
        order: order.key(),
        total: 0,
        filtered: !filter.is_empty(),
        clear_href: String::new(),
        notes: Vec::new(),
        waiting: 0,
    };

    let Some(own_team) = viewer.team_number else {
        page.unavailable = "Notes are read only by the team whose scouts wrote them. Your \
                            account has no team, so none are shown."
            .into();
        return page;
    };
    page.own_team = own_team;
    let Some(event) = &context.selected else {
        page.unavailable = if storage_ready {
            "No events have been loaded yet, so there are no notes.".into()
        } else {
            "The server's storage is unavailable, so notes cannot be shown.".into()
        };
        return page;
    };
    page.event_name = event.name.clone();
    page.clear_href = format!("/notes?event={}&order={}", event.key, order.key());
    if let Some(unknown) = &context.unknown {
        page.errors.push(format!(
            "There is no event “{unknown}” on this server. Showing {} instead.",
            event.name
        ));
    }

    let loaded = async {
        let approved = state.repo.approved_observations(&event.key).await?;
        let pending = state.repo.pending_observations(&event.key).await?;
        let matches = state.repo.event_matches(&event.key).await?;
        tt_repo::Result::Ok((approved, pending, matches))
    };
    let (approved, pending, matches) = match loaded.await {
        Ok(loaded) => loaded,
        Err(e) => {
            warn!("notes at {}: {e}", event.key);
            page.unavailable = "The notes could not be read. Reload to try again.".into();
            return page;
        }
    };
    // Names are a nicety: without them the notes still read.
    let roster = state
        .repo
        .event_teams(&event.key)
        .await
        .inspect_err(|e| warn!("roster for {}: {e}", event.key))
        .unwrap_or_default();

    let readable = |o: &&StoredObservation| {
        Notes::for_viewer(Some(own_team), o.submitting_team).shown()
            && !notes::written(&state.season, &o.payload).is_empty()
    };
    page.waiting = pending.iter().filter(readable).count();
    let mut observed: Vec<&StoredObservation> = approved.iter().filter(readable).collect();
    match order {
        // Unrecorded times sort last; the latest saved breaks a tie.
        Order::Newest => {
            observed.sort_by_key(|o| std::cmp::Reverse((o.observed_at.or(o.created_at), o.id)))
        }
        // Stable, and `None` sorts first: a match gone from the schedule leads.
        Order::Schedule => observed.sort_by_key(|o| {
            (
                matches.iter().position(|m| m.key == o.match_key),
                o.team_number,
            )
        }),
    }

    // The menus offer whoever has notes here, before any filter, plus what the
    // URL asked for, so the choice in force always shows as chosen.
    let mut teams: Vec<i32> = observed.iter().map(|o| o.team_number).collect();
    teams.extend(filter.team);
    teams.sort_unstable();
    teams.dedup();
    let name_of = |team: i32| {
        roster
            .iter()
            .find(|t| t.number == team)
            .map(|t| t.name.clone())
            .unwrap_or_default()
    };
    page.teams = teams
        .into_iter()
        .map(|team| FilterOption {
            value: team.to_string(),
            label: match name_of(team) {
                name if name.is_empty() => team.to_string(),
                name => format!("{team} · {name}"),
            },
            selected: filter.team == Some(team),
        })
        .collect();
    let mut scouts: Vec<(String, i64)> = observed
        .iter()
        .filter_map(|o| Some((o.scouter_name.clone()?, o.scouter_id?)))
        .collect();
    scouts.sort();
    scouts.dedup();
    page.scouts = scouts
        .into_iter()
        .map(|(name, id)| FilterOption {
            value: id.to_string(),
            label: name,
            selected: filter.scout == Some(id),
        })
        .collect();

    // With one text field on the form, its label says nothing the heading
    // does not.
    let one_field = state
        .season
        .fields()
        .filter(|f| matches!(f.kind, FieldKind::Text { .. }))
        .count()
        <= 1;
    let now = Utc::now();
    for o in observed {
        let written = notes::written(&state.season, &o.payload);
        page.total += written.len();
        let recorded = o.observed_at.or(o.created_at);
        for (field, text) in written {
            if !filter.keeps(o.team_number, o.scouter_id, &text) {
                continue;
            }
            page.notes.push(NoteEntry {
                team: o.team_number,
                team_name: name_of(o.team_number),
                match_label: matches
                    .iter()
                    .find(|m| m.key == o.match_key)
                    .map(MatchRecord::label)
                    .unwrap_or_else(|| o.match_key.clone()),
                scout: o
                    .scouter_name
                    .clone()
                    .unwrap_or_else(|| "a scout whose account is gone".into()),
                label: if one_field { String::new() } else { field },
                text,
                when: recorded
                    .map(|at| event.day_and_time(at))
                    .unwrap_or_default(),
                ago: recorded
                    .map(|at| describe_age(now - at))
                    .unwrap_or_default(),
                filter_href: href(&event.key, o.team_number),
                profile_href: team_href(&event.key, o.team_number),
            });
        }
    }
    page
}
