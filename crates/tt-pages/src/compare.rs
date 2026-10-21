//! Compare teams (DR1): `/compare?event=…&team=…&team=…&last=5&ref=…`.
//!
//! The numbers are shared, as on the team page and the graph: every approved
//! observation at the event on the current form, whoever's scouts recorded
//! it. Notes are not compared; each column links to its team's page.

use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, Utc};
use tracing::warn;
use tt_core::compare::{self, Better, Cell, DEFAULT_TEAMS, MAX_TEAMS, THIN, TeamMatches, Window};
use tt_core::connectivity::describe_age;
use tt_core::graph::{self, POINTS};
use tt_core::ranking::{self, Scored};
use tt_core::season::{Payload, SeasonSchema};
use tt_core::upstream;
use tt_repo::LocalRepo;
use tt_templates::{
    BoxPlot, Chip, Choice, CompareCell, CompareColumn, CompareGroup, ComparePage, CompareRow, Nav,
    team_href,
};

use crate::events::EventContext;

/// `/compare` for `event_key`, with `teams` chosen.
pub fn compare_href(event_key: &str, teams: &[i32]) -> String {
    let mut href = format!("/compare?event={event_key}&chosen=1");
    for team in teams {
        href.push_str(&format!("&team={team}"));
    }
    href
}

/// The comparison for the event `context` selected. `query` is a list, not
/// a map: `team` repeats. Unless the URL says otherwise, the others are
/// measured against `viewer_team` when it is among those chosen.
#[allow(clippy::too_many_arguments)]
pub async fn page<R: LocalRepo>(
    repo: &R,
    schema: &SeasonSchema,
    nav: Nav,
    viewer_team: Option<i32>,
    context: &EventContext,
    query: &[(String, String)],
    now: DateTime<Utc>,
) -> ComparePage {
    let storage_ready = nav.storage_ready;
    let all = |name: &str| -> Vec<&str> {
        query
            .iter()
            .filter(|(k, _)| k == name)
            .map(|(_, v)| v.trim())
            .collect()
    };
    let chosen = !all("chosen").is_empty() || !all("team").is_empty();
    let window = Window::parse(all("last").first().copied().unwrap_or_default());
    let asked_ref: Option<i32> = all("ref").first().and_then(|r| r.parse().ok());
    let mut page = ComparePage {
        title: "Compare".into(),
        nav,
        event_name: String::new(),
        unavailable: String::new(),
        errors: Vec::new(),
        notices: Vec::new(),
        teams: Vec::new(),
        more_teams: Vec::new(),
        max_teams: MAX_TEAMS,
        windows: Window::OFFERED
            .iter()
            .map(|w| Choice {
                value: w.as_str().into(),
                label: w.label(),
                selected: *w == window,
            })
            .collect(),
        references: Vec::new(),
        columns: Vec::new(),
        groups: Vec::new(),
        graph_href: String::new(),
        source: String::new(),
    };

    let Some(selected) = &context.selected else {
        page.unavailable = if storage_ready {
            "No events have been loaded yet, so there is nothing to compare.".into()
        } else {
            "The server's storage is unavailable, so nothing can be compared.".into()
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
        let approved = repo.approved_observations(&selected.key).await?;
        let matches = repo.event_matches(&selected.key).await?;
        let stats = repo.event_stats(&selected.key).await?;
        let pending = repo.pending_observations(&selected.key).await?;
        let overrides = repo.weight_overrides().await?;
        tt_repo::Result::Ok((approved, matches, stats, pending.len(), overrides))
    };
    let (approved, matches, stats, waiting, overrides) = match loaded.await {
        Ok(loaded) => loaded,
        Err(e) => {
            warn!("compare at {}: {e}", selected.key);
            page.unavailable = "The numbers could not be read. Reload to try again.".into();
            return page;
        }
    };
    let roster = repo
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

    // Each team's matches in schedule order, each with its observations.
    let place: HashMap<&str, usize> = matches
        .iter()
        .enumerate()
        .map(|(i, m)| (m.key.as_str(), i))
        .collect();
    let counted: Vec<_> = approved
        .iter()
        .filter(|o| o.schema_version == schema.version)
        .collect();
    let mut by_team: BTreeMap<i32, BTreeMap<(usize, &str), Vec<&Payload>>> = BTreeMap::new();
    for o in &counted {
        let at = place
            .get(o.match_key.as_str())
            .copied()
            .unwrap_or(usize::MAX);
        by_team
            .entry(o.team_number)
            .or_default()
            .entry((at, o.match_key.as_str()))
            .or_default()
            .push(&o.payload);
    }

    // Offered: scouted teams by scout estimate, then the rest of the roster.
    let scored: Vec<Scored> = counted
        .iter()
        .map(|o| Scored {
            team_number: o.team_number,
            payload: &o.payload,
            schema_version: o.schema_version,
        })
        .collect();
    let mut best = ranking::team_scores(schema, &overrides, &scored);
    best.sort_by(|a, b| {
        b.average
            .total_cmp(&a.average)
            .then(a.team_number.cmp(&b.team_number))
    });
    let mut offered: Vec<i32> = best.iter().map(|s| s.team_number).collect();
    let mut unscouted: Vec<i32> = roster
        .iter()
        .map(|t| t.number)
        .filter(|n| !offered.contains(n))
        .collect();
    unscouted.sort_unstable();
    offered.extend(unscouted);
    if offered.is_empty() {
        page.unavailable = format!(
            "Nobody at {} has been scouted or put on the roster yet, so there is nothing to compare.",
            selected.name
        );
        return page;
    }

    let asked: Vec<i32> = all("team").iter().filter_map(|t| t.parse().ok()).collect();
    let teams: Vec<i32> = if chosen {
        let (teams, over) = graph::choose(&asked, &offered, MAX_TEAMS);
        if over > 0 {
            page.notices.push(format!(
                "Up to {MAX_TEAMS} teams fit side by side, so {over} more {} left off.",
                if over == 1 { "was" } else { "were" }
            ));
        }
        teams
    } else {
        // `ref` alone, from a team page: that team against the best here.
        let first = asked_ref.filter(|r| offered.contains(r));
        let top: Vec<i32> = first
            .into_iter()
            .chain(
                best.iter()
                    .map(|s| s.team_number)
                    .filter(|t| Some(*t) != first),
            )
            .take(DEFAULT_TEAMS)
            .collect();
        if !top.is_empty() {
            page.notices.push(format!(
                "Showing the {} with the best scout estimate here. Tap teams to choose others.",
                match top.len() {
                    1 => "team".to_string(),
                    n => format!("{n} teams"),
                }
            ));
        }
        top
    };
    // `ref=` (Nobody) is a choice too; only no `ref` at all means ours.
    let reference = if all("ref").is_empty() {
        viewer_team
    } else {
        asked_ref
    }
    .filter(|r| teams.contains(r));

    let chip = |team: i32| {
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
    page.teams = teams.iter().map(|t| chip(*t)).collect();
    page.more_teams = offered
        .iter()
        .filter(|t| !teams.contains(t))
        .map(|t| chip(*t))
        .collect();
    page.references = std::iter::once(Choice {
        value: String::new(),
        label: "Nobody".into(),
        selected: reference.is_none(),
    })
    .chain(teams.iter().map(|t| Choice {
        value: t.to_string(),
        label: t.to_string(),
        selected: reference == Some(*t),
    }))
    .collect();
    page.source = source(
        counted.len(),
        approved.len() - counted.len(),
        waiting,
        &stats,
        now,
    );
    if teams.is_empty() {
        return page;
    }
    page.graph_href = {
        let mut href = format!("/graph?event={}&chosen=1", selected.key);
        for t in &teams {
            href.push_str(&format!("&team={t}"));
        }
        href.push_str(&format!("&metric={POINTS}"));
        href
    };

    let empty = BTreeMap::new();
    let columns: Vec<TeamMatches> = teams
        .iter()
        .map(|t| TeamMatches {
            team_number: *t,
            matches: by_team.get(t).unwrap_or(&empty).values().cloned().collect(),
            stats: stats.iter().find(|s| s.team_number == *t),
        })
        .collect();
    let table = compare::table(schema, &overrides, &columns, window);
    let ref_index = reference.and_then(|r| teams.iter().position(|t| *t == r));

    page.columns = teams
        .iter()
        .zip(&table.matches)
        .enumerate()
        .map(|(i, (t, n))| CompareColumn {
            number: *t,
            name: name_of(*t),
            slot: i + 1,
            team_href: team_href(&selected.key, *t),
            matches: format!("{n} match{}", if *n == 1 { "" } else { "es" }),
            thin: *n < THIN,
            reference: ref_index == Some(i),
        })
        .collect();
    page.groups = table
        .groups
        .iter()
        .map(|group| CompareGroup {
            label: group.label.clone(),
            rows: group
                .rows
                .iter()
                .map(|row| view_row(row, ref_index))
                .collect(),
        })
        .collect();
    page
}

/// One row as the page reads it.
fn view_row(row: &compare::Row, reference: Option<usize>) -> CompareRow {
    let best = row.best();
    // One scale for the row, so the boxes line up across teams.
    let spreads: Vec<_> = row
        .cells
        .iter()
        .filter_map(|c| match c {
            Cell::Spread(Some(s)) => Some(*s),
            _ => None,
        })
        .collect();
    let lo = spreads
        .iter()
        .map(|s| s.min)
        .reduce(f64::min)
        .unwrap_or(0.0);
    let hi = spreads
        .iter()
        .map(|s| s.max)
        .reduce(f64::max)
        .unwrap_or(0.0);
    let at = |v: f64| ((v - lo) / (hi - lo) * 1000.0).round() / 10.0;
    let ref_value = reference.and_then(|i| row.cells[i].value());

    let cells = row
        .cells
        .iter()
        .enumerate()
        .map(|(i, cell)| {
            let mut view = CompareCell {
                best: best[i],
                ..CompareCell::default()
            };
            match cell {
                Cell::Spread(None) | Cell::Official(None) => view.main = "—".into(),
                Cell::Spread(Some(s)) => {
                    view.main = number(s.mean);
                    let mut detail = vec![format!("median {}", number(s.median))];
                    if s.max > s.min {
                        detail.push(format!("{}–{}", number(s.min), number(s.max)));
                    }
                    if let Some(sd) = s.sd {
                        detail.push(format!("±{}", number(sd)));
                    }
                    detail.push(format!("{} match{}", s.n, if s.n == 1 { "" } else { "es" }));
                    view.detail = detail.join(" · ");
                    if hi > lo {
                        view.plot = Some(BoxPlot {
                            min: at(s.min),
                            q1: at(s.q1),
                            median: at(s.median),
                            q3: at(s.q3),
                            max: at(s.max),
                        });
                    }
                }
                Cell::Rate { of: 0, .. } => view.main = "—".into(),
                Cell::Rate { yes, of } => {
                    view.main = format!("{}%", (*yes as f64 / *of as f64 * 100.0).round());
                    view.detail = format!("{yes} of {of}");
                }
                Cell::Tally(tally) if tally.is_empty() => view.main = "—".into(),
                Cell::Tally(tally) => {
                    view.main = tally
                        .iter()
                        .map(|(label, n)| format!("{label} {n}"))
                        .collect::<Vec<_>>()
                        .join(" · ");
                }
                Cell::Official(Some(v)) => view.main = number(*v),
            }
            if let (Some(r), Some(v)) = (ref_value, cell.value())
                && Some(i) != reference
            {
                let rate = matches!(cell, Cell::Rate { .. });
                let d = if rate { (v - r) * 100.0 } else { v - r };
                if d.abs() < 0.05 {
                    view.delta = "same".into();
                } else {
                    let sign = if d > 0.0 { "+" } else { "−" };
                    view.delta = format!(
                        "{sign}{}{}",
                        number(d.abs()),
                        if rate { " pts" } else { "" }
                    );
                    let ahead = match row.better {
                        Better::Higher => Some(d > 0.0),
                        Better::Lower => Some(d < 0.0),
                        Better::Neither => None,
                    };
                    view.delta_class = match ahead {
                        Some(true) => "ahead".into(),
                        Some(false) => "behind".into(),
                        None => String::new(),
                    };
                }
            }
            view
        })
        .collect();
    CompareRow {
        label: row.label.clone(),
        hint: if row.better == Better::Lower {
            "lower is better".into()
        } else {
            String::new()
        },
        cells,
    }
}

/// `8`, `8.3`: one decimal, none when it is a whole number.
fn number(v: f64) -> String {
    let rounded = (v * 10.0).round() / 10.0;
    if rounded.fract() == 0.0 {
        format!("{rounded:.0}")
    } else {
        format!("{rounded:.1}")
    }
}

fn source(
    counted: usize,
    older: usize,
    waiting: usize,
    stats: &[tt_core::records::TeamEventStats],
    now: DateTime<Utc>,
) -> String {
    let mut said = format!(
        "Scouting: {counted} approved observation{} at this event.",
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
    match stats.iter().filter_map(|s| s.synced_at).max() {
        Some(at) => said.push_str(&format!(
            " Official numbers: {}, {}.",
            upstream::STATS_SOURCE,
            describe_age(now - at)
        )),
        None => said.push_str(" No official numbers synced yet."),
    }
    said
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_keep_one_decimal_only_when_needed() {
        assert_eq!(number(8.0), "8");
        assert_eq!(number(8.26), "8.3");
        assert_eq!(number(0.04), "0");
    }

    #[test]
    fn a_reference_says_who_is_ahead_by_which_way_is_better() {
        let s = |v: f64| Cell::Spread(compare::spread(&[v, v + 2.0]));
        let row = compare::Row {
            key: "k".into(),
            label: "Fouls".into(),
            better: Better::Lower,
            cells: vec![s(1.0), s(3.0), Cell::Spread(None)],
        };
        let view = view_row(&row, Some(0));
        assert_eq!(view.hint, "lower is better");
        assert_eq!(view.cells[0].delta, "", "the reference itself");
        assert_eq!(view.cells[1].delta, "+2");
        assert_eq!(view.cells[1].delta_class, "behind", "more fouls is worse");
        assert_eq!(view.cells[2].main, "—");
        assert!(view.cells[0].best);
        let plot = view.cells[0].plot.unwrap();
        assert_eq!(
            (plot.min, plot.max),
            (0.0, 50.0),
            "on the row's shared scale"
        );
        assert_eq!(view.cells[0].detail, "median 2 · 1–3 · ±1.4 · 2 matches");
    }

    #[test]
    fn a_rate_is_a_percentage_and_its_lead_is_in_points() {
        let row = compare::Row {
            key: "k".into(),
            label: "Parked".into(),
            better: Better::Higher,
            cells: vec![Cell::Rate { yes: 1, of: 2 }, Cell::Rate { yes: 3, of: 4 }],
        };
        let view = view_row(&row, Some(0));
        assert_eq!(
            (view.cells[1].main.as_str(), view.cells[1].detail.as_str()),
            ("75%", "3 of 4")
        );
        assert_eq!(view.cells[1].delta, "+25 pts");
        assert_eq!(view.cells[1].delta_class, "ahead");
    }
}
