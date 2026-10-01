//! The graph view (U21): `/graph?event=…&team=…&metric=…&span=season`.
//!
//! The page carries every team with something to draw, not only the chosen
//! ones, so `static/js/graph.js` redraws on a tap with no round trip. The
//! chosen ones are in the URL, which the script keeps current, so a chart can
//! be bookmarked or sent. Without the script the chips are a GET form and the
//! numbers are tables.
//!
//! The numbers are shared, as on the team profile: every approved observation
//! at the event, whoever's scouts recorded it. Notes are not numbers and are
//! not here; they are U22's view.

use std::collections::HashMap;

use chrono::Utc;
use serde_json::json;
use tracing::warn;
use tt_core::connectivity::describe_age;
use tt_core::graph::{self, DEFAULT_TEAMS, MAX_METRICS, MAX_TEAMS, Metric, POINTS, Point, Sample};
use tt_core::ranking::{self, Scored};
use tt_core::records::{Event, TeamEventStats};
use tt_repo::{Repo, StoredObservation};
use tt_templates::{Chip, GraphPage, Nav, TableRow, TeamTable};

use crate::events::EventContext;
use crate::startup::AppState;

/// What one event contributes.
struct Loaded {
    event: Event,
    approved: Vec<StoredObservation>,
    /// Match key → `"Q14"`, and its place in the schedule.
    matches: HashMap<String, (String, usize)>,
    stats: Vec<TeamEventStats>,
}

pub async fn page(
    state: &AppState,
    nav: Nav,
    context: &EventContext,
    query: &[(String, String)],
) -> GraphPage {
    let storage_ready = nav.storage_ready;
    let all = |name: &str| -> Vec<&str> {
        query
            .iter()
            .filter(|(k, _)| k == name)
            .map(|(_, v)| v.trim())
            .collect()
    };
    let chosen = !all("chosen").is_empty() || !all("team").is_empty();
    let season = all("span").contains(&"season");
    let mut page = GraphPage {
        title: "Graph".into(),
        nav,
        event_name: String::new(),
        unavailable: String::new(),
        errors: Vec::new(),
        notices: Vec::new(),
        teams: Vec::new(),
        more_teams: Vec::new(),
        metrics: Vec::new(),
        prompt: String::new(),
        season,
        data: String::new(),
        headings: Vec::new(),
        tables: Vec::new(),
        source: String::new(),
        max_teams: MAX_TEAMS,
        max_metrics: MAX_METRICS,
    };

    let Some(selected) = &context.selected else {
        page.unavailable = if storage_ready {
            "No events have been loaded yet, so there is nothing to draw.".into()
        } else {
            "The server's storage is unavailable, so nothing can be drawn.".into()
        };
        return page;
    };
    page.event_name = selected.name.clone();
    if let Some(unknown) = &context.unknown {
        page.errors.push(format!(
            "There is no event “{unknown}” on this server. Showing {} instead.",
            selected.name
        ));
    }

    let loaded = async {
        // The selected event first, so a failure there is the one reported.
        let mut events = vec![selected.clone()];
        if season {
            let year = &selected.key[..selected.key.len().min(4)];
            let mut others: Vec<Event> = state
                .repo
                .list_events()
                .await?
                .into_iter()
                .filter(|e| e.key.starts_with(year) && e.key != selected.key)
                .collect();
            events.append(&mut others);
            events.sort_by(|a, b| (a.start_date, &a.key).cmp(&(b.start_date, &b.key)));
        }
        let mut loaded = Vec::new();
        for event in events {
            let approved = state.repo.approved_observations(&event.key).await?;
            // An event nobody scouted adds nothing, so skip its schedule.
            if approved.is_empty() && event.key != selected.key {
                continue;
            }
            let matches = state
                .repo
                .event_matches(&event.key)
                .await?
                .into_iter()
                .enumerate()
                .map(|(i, m)| (m.key.clone(), (m.label(), i)))
                .collect();
            let stats = state.repo.event_stats(&event.key).await?;
            loaded.push(Loaded {
                event,
                approved,
                matches,
                stats,
            });
        }
        let pending = state.repo.pending_observations(&selected.key).await?;
        let overrides = state.repo.weight_overrides().await?;
        tt_repo::Result::Ok((loaded, pending.len(), overrides))
    };
    let (loaded, waiting, overrides) = match loaded.await {
        Ok(loaded) => loaded,
        Err(e) => {
            warn!("graph at {}: {e}", selected.key);
            page.unavailable = "The numbers could not be read. Reload to try again.".into();
            return page;
        }
    };
    // Names are a nicety: without them the chart still reads.
    let roster = state
        .repo
        .event_teams(&selected.key)
        .await
        .inspect_err(|e| warn!("roster for {}: {e}", selected.key))
        .unwrap_or_default();
    let name_of = |team: i32| {
        roster
            .iter()
            .find(|t| t.number == team)
            .map(|t| t.name.clone())
            .unwrap_or_default()
    };

    let schema = &state.season;
    let at_selected = &loaded[loaded
        .iter()
        .position(|l| l.event.key == selected.key)
        .expect("the selected event is always loaded")];
    let any_stats = loaded.iter().any(|l| {
        l.stats
            .iter()
            .any(|s| s.opr.is_some() || s.dpr.is_some() || s.ccwm.is_some())
    });
    // TBA's metrics are offered only once something has been synced: a chip
    // that can never draw anything is a trap.
    let metrics: Vec<Metric> = graph::metrics(schema)
        .into_iter()
        .filter(|m| any_stats || !m.from_tba())
        .collect();

    let samples: Vec<Sample> = loaded
        .iter()
        .enumerate()
        .flat_map(|(place, l)| {
            l.approved.iter().map(move |o| Sample {
                team_number: o.team_number,
                event_key: &l.event.key,
                match_key: &o.match_key,
                place: (
                    place,
                    l.matches.get(&o.match_key).map_or(usize::MAX, |m| m.1),
                ),
                payload: &o.payload,
                schema_version: o.schema_version,
            })
        })
        .collect();
    let mut lines = graph::series(schema, &overrides, &metrics, &samples, |event, team| {
        loaded
            .iter()
            .find(|l| l.event.key == event)?
            .stats
            .iter()
            .find(|s| s.team_number == team)
    });
    // Across events, the chips are the selected event's teams: its roster and
    // whoever was scouted there.
    if season {
        lines.retain(|team, _| {
            roster.iter().any(|t| t.number == *team)
                || at_selected.approved.iter().any(|o| o.team_number == *team)
        });
    }
    let offered: Vec<i32> = lines.keys().copied().collect();
    if offered.is_empty() {
        page.unavailable = if season {
            format!(
                "Nobody at {} has been scouted this season yet, so there is nothing to draw.",
                selected.name
            )
        } else {
            format!(
                "Nothing has been scouted and approved at {} yet, so there is nothing to draw.",
                selected.name
            )
        };
        return page;
    }

    // Which teams and metrics are on. Before anyone chooses, the three best
    // scouted at this event, by their scouting points.
    let asked_teams: Vec<i32> = all("team").iter().filter_map(|t| t.parse().ok()).collect();
    let (teams, over) = if chosen {
        graph::choose(&asked_teams, &offered, MAX_TEAMS)
    } else {
        let scored: Vec<Scored> = at_selected
            .approved
            .iter()
            .map(|o| Scored {
                team_number: o.team_number,
                payload: &o.payload,
                schema_version: o.schema_version,
            })
            .collect();
        let mut best = ranking::team_scores(schema, &overrides, &scored);
        best.sort_by(|a, b| b.average.total_cmp(&a.average));
        let best: Vec<i32> = best
            .into_iter()
            .map(|s| s.team_number)
            .filter(|t| offered.contains(t))
            .take(DEFAULT_TEAMS)
            .collect();
        if !best.is_empty() {
            page.notices.push(format!(
                "Showing the {} with the most scouting points here. Tap teams to choose others.",
                match best.len() {
                    1 => "team".to_string(),
                    n => format!("{n} teams"),
                }
            ));
        }
        (best, 0)
    };
    if over > 0 {
        page.notices.push(format!(
            "Only {MAX_TEAMS} teams fit on one chart, so {over} more {} left off.",
            if over == 1 { "was" } else { "were" }
        ));
    }
    let metric_keys: Vec<String> = metrics.iter().map(|m| m.key.clone()).collect();
    let asked_metrics: Vec<String> = all("metric").iter().map(|m| m.to_string()).collect();
    let (shown, over) = if chosen {
        graph::choose(&asked_metrics, &metric_keys, MAX_METRICS)
    } else {
        (vec![POINTS.to_string()], 0)
    };
    if over > 0 {
        page.notices.push(format!(
            "Only {MAX_METRICS} metrics fit on one chart, so {over} more {} left off.",
            if over == 1 { "was" } else { "were" }
        ));
    }

    let team_chip = |team: i32| {
        let name = name_of(team);
        let slot = teams.iter().position(|t| *t == team).map_or(0, |i| i + 1);
        Chip {
            value: team.to_string(),
            label: team.to_string(),
            title: if name.is_empty() {
                format!("Team {team}")
            } else {
                format!("{team} · {name}")
            },
            on: slot > 0,
            slot,
        }
    };
    page.teams = teams.iter().map(|t| team_chip(*t)).collect();
    page.more_teams = offered
        .iter()
        .filter(|t| !teams.contains(t))
        .map(|t| team_chip(*t))
        .collect();
    page.metrics = metrics
        .iter()
        .map(|m| {
            let slot = shown.iter().position(|k| *k == m.key).map_or(0, |i| i + 1);
            Chip {
                value: m.key.clone(),
                label: m.label.clone(),
                title: m.label.clone(),
                on: slot > 0,
                slot,
            }
        })
        .collect();

    if teams.is_empty() {
        page.prompt = "Tap a team to draw it.".into();
    } else if shown.is_empty() {
        page.prompt = "Tap a metric to draw it.".into();
    }

    // A point's name: its match, and its event's code once there are several.
    let several = loaded.len() > 1;
    let label = |p: &Point| {
        let in_event = loaded.iter().find(|l| l.event.key == p.event_key);
        let name = in_event
            .and_then(|l| l.matches.get(&p.match_key))
            .map(|m| m.0.clone())
            .unwrap_or_else(|| p.match_key.clone());
        if several {
            let code = p.event_key.get(4..).unwrap_or(&p.event_key).to_uppercase();
            format!("{code} {name}")
        } else {
            name
        }
    };
    let round = |v: Option<f64>| v.map(|v| (v * 100.0).round() / 100.0);

    page.data = json!({
        "metrics": metrics.iter().map(|m| json!({"key": m.key, "label": m.label})).collect::<Vec<_>>(),
        "teams": lines.iter().map(|(team, points)| json!({
            "number": team,
            "name": name_of(*team),
            "points": points.iter().map(|p| json!({
                "match": label(p),
                "n": p.observed,
                "v": p.values.iter().map(|v| round(*v)).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })
    .to_string();

    let columns: Vec<usize> = shown
        .iter()
        .filter_map(|k| metrics.iter().position(|m| m.key == *k))
        .collect();
    page.headings = columns.iter().map(|i| metrics[*i].label.clone()).collect();
    page.tables = page
        .teams
        .iter()
        .map(|chip| {
            let number: i32 = chip.value.parse().unwrap_or_default();
            TeamTable {
                number,
                name: name_of(number),
                slot: chip.slot,
                rows: lines
                    .get(&number)
                    .map(Vec::as_slice)
                    .unwrap_or_default()
                    .iter()
                    .map(|p| TableRow {
                        label: label(p),
                        values: columns
                            .iter()
                            .map(|i| {
                                round(p.values[*i])
                                    .map_or_else(|| "—".to_string(), |v| v.to_string())
                            })
                            .collect(),
                    })
                    .collect(),
            }
        })
        .collect();

    page.source = source(schema.version, &loaded, waiting, any_stats);
    page
}

/// Where the numbers come from: always said (RI 2B).
fn source(version: i64, loaded: &[Loaded], waiting: usize, any_stats: bool) -> String {
    let approved = loaded.iter().flat_map(|l| &l.approved);
    let counted = approved
        .clone()
        .filter(|o| o.schema_version == version)
        .count();
    let older = approved.count() - counted;
    let places = if loaded.len() > 1 {
        format!("{} events", loaded.len())
    } else {
        "this event".to_string()
    };
    let mut said = format!(
        "Scouting: {counted} approved observation{} at {places}.",
        if counted == 1 { "" } else { "s" }
    );
    if older > 0 {
        said.push_str(&format!(
            " {older} on an older form {} left out.",
            if older == 1 { "is" } else { "are" }
        ));
    }
    if waiting > 0 {
        said.push_str(&format!(
            " {waiting} more {} waiting for review.",
            if waiting == 1 { "is" } else { "are" }
        ));
    }
    let synced = loaded
        .iter()
        .flat_map(|l| &l.stats)
        .filter_map(|s| s.synced_at)
        .max();
    said.push_str(&match (any_stats, synced) {
        (true, Some(at)) => format!(
            " OPR, DPR, and CCWM: The Blue Alliance, synced {}.",
            describe_age(Utc::now() - at)
        ),
        (true, None) => " OPR, DPR, and CCWM: The Blue Alliance.".to_string(),
        (false, _) => " Nothing from The Blue Alliance has been synced yet.".to_string(),
    });
    said
}
