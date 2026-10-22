//! Rankings (L11) and the point values behind them (L12).

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::convert::Infallible;

use axum::extract::{FromRequestParts, Query};
use axum::http::request::Parts;
use chrono::Utc;
use tracing::{info, warn};
use tt_core::compare::{self, Better, Cell, TeamMatches, Window};
use tt_core::connectivity::Freshness;
use tt_core::graph;
use tt_core::ranking::{self, RankingRow, Scored, SortKey, WeightErrors};
use tt_core::review::ReviewState;
use tt_core::user::User;
use tt_pages::teams::latest_scouted;
use tt_repo::{Repo, StoredObservation};
use tt_templates::{
    MetricChoice, MetricColumn, Nav, RankingView, RankingsPage, SCORE_LABEL, SortLink, THIN_BELOW,
    WeightGroup, WeightInput, WeightsPage,
};

use crate::events::EventContext;
use crate::startup::AppState;

/// `?sort=`, `?col=` for each metric column, and `?saved=` after a weights
/// save. Never rejects: an unknown sort is the default one.
#[derive(Debug, Default)]
pub struct RankingParams {
    pub sort: SortKey,
    /// `?sort=m.<field key>`: by a metric column, best first.
    pub metric_sort: Option<String>,
    /// The metric columns asked for; `None` for the defaults.
    pub cols: Option<Vec<String>>,
    pub saved: Option<String>,
}

impl<S: Send + Sync> FromRequestParts<S> for RankingParams {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Infallible> {
        let pairs = Query::<Vec<(String, String)>>::from_request_parts(parts, state)
            .await
            .map(|Query(q)| q)
            .unwrap_or_default();
        let query: HashMap<&str, &str> = pairs
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let sort = query.get("sort").map(|s| s.trim()).unwrap_or_default();
        // `cols=1` marks a chosen set, so ticking none is not the defaults.
        let cols =
            (query.contains_key("cols") || pairs.iter().any(|(k, _)| k == "col")).then(|| {
                pairs
                    .iter()
                    .filter(|(k, _)| k == "col")
                    .map(|(_, v)| v.trim().to_string())
                    .filter(|v| !v.is_empty())
                    .collect()
            });
        Ok(Self {
            sort: SortKey::parse(sort).unwrap_or_default(),
            metric_sort: sort.strip_prefix(METRIC_SORT).map(str::to_string),
            cols,
            saved: query.get("saved").map(|s| s.to_string()),
        })
    }
}

/// A metric column's sort, `m.<field key>`.
const METRIC_SORT: &str = "m.";

/// The rankings table for `context`'s event, ordered and with the columns
/// `params` asks for.
pub async fn page(
    state: &AppState,
    nav: Nav,
    context: &EventContext,
    params: &RankingParams,
) -> RankingsPage {
    let sort = params.sort;
    let storage_ready = nav.storage_ready;
    let mut page = RankingsPage {
        title: "Rankings".into(),
        nav,
        event_name: String::new(),
        unavailable: String::new(),
        errors: Vec::new(),
        rows: Vec::new(),
        columns: Vec::new(),
        pending: 0,
        weights_changed: 0,
        latest_scouted: String::new(),
        ranks_updated: String::new(),
        ranks_stale: false,
        metrics: Vec::new(),
        metric_choices: Vec::new(),
        sort: String::new(),
        shown_cols: Vec::new(),
        cols_chosen: params.cols.is_some(),
    };
    let Some(event) = &context.selected else {
        page.unavailable = if storage_ready {
            "No events have been loaded yet, so there is nothing to rank.".into()
        } else {
            "The server's storage is unavailable, so rankings cannot be shown.".into()
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

    let loaded = async {
        let approved = state.repo.approved_observations(&event.key).await?;
        let pending = state.repo.pending_observations(&event.key).await?;
        let overrides = state.repo.weight_overrides().await?;
        let roster = state.repo.event_teams(&event.key).await?;
        let stats = state.repo.event_stats(&event.key).await?;
        tt_repo::Result::Ok((approved, pending, overrides, roster, stats))
    };
    let (approved, pending, overrides, roster, stats) = match loaded.await {
        Ok(loaded) => loaded,
        Err(e) => {
            warn!("rankings for {}: {e}", event.key);
            page.unavailable = "Could not read the rankings. Reload to try again.".into();
            return page;
        }
    };
    page.pending = pending.len();
    page.weights_changed = overrides.iter().count();

    // Where each column's numbers come from, and how old they are (U14, I12).
    let now = Utc::now();
    let counted: Vec<&StoredObservation> = approved
        .iter()
        .filter(|o| o.schema_version == state.season.version)
        .collect();
    page.latest_scouted = latest_scouted(&counted, now);
    let ranks = stats
        .iter()
        .filter(|s| s.rank.is_some())
        .filter_map(|s| s.synced_at)
        .max()
        .map(|at| Freshness::of(at, now, event.is_running(now)));
    page.ranks_updated = ranks.as_ref().map(|f| f.age.clone()).unwrap_or_default();
    page.ranks_stale = ranks.is_some_and(|f| f.stale);

    let scored: Vec<Scored> = approved
        .iter()
        .filter(|o| o.review_state == ReviewState::Approved)
        .map(|o| Scored {
            team_number: o.team_number,
            payload: &o.payload,
            schema_version: o.schema_version,
        })
        .collect();
    let scores = ranking::team_scores(&state.season, &overrides, &scored);

    // Everyone on the roster, plus anyone ranked or scouted who is not on it
    // yet: rosters and schedules arrive from different feeds.
    let teams: BTreeSet<i32> = roster
        .iter()
        .map(|t| t.number)
        .chain(stats.iter().map(|s| s.team_number))
        .chain(scores.iter().map(|s| s.team_number))
        .collect();
    let mut rows = Vec::with_capacity(teams.len());
    for number in teams {
        let name = match roster.iter().find(|t| t.number == number) {
            Some(team) => team.name.clone(),
            None => state
                .repo
                .team(number)
                .await
                .ok()
                .flatten()
                .map(|t| t.name)
                .unwrap_or_default(),
        };
        rows.push(RankingRow {
            team_number: number,
            name,
            rank: stats
                .iter()
                .find(|s| s.team_number == number)
                .and_then(|s| s.rank),
            score: scores.iter().find(|s| s.team_number == number).cloned(),
        });
    }
    ranking::sort(&mut rows, sort);

    // What scouts recorded, per team: the rows Compare shows, for everyone
    // here at once (DR1). Counts and point-carrying choices are an average
    // per match; yes/no fields are how often. Tallies have no order to sort.
    let mut by_team: BTreeMap<i32, BTreeMap<&str, Vec<&tt_core::season::Payload>>> =
        BTreeMap::new();
    for o in approved
        .iter()
        .filter(|o| o.review_state == ReviewState::Approved)
        .filter(|o| o.schema_version == state.season.version)
    {
        by_team
            .entry(o.team_number)
            .or_default()
            .entry(o.match_key.as_str())
            .or_default()
            .push(&o.payload);
    }
    let team_matches: Vec<TeamMatches> = rows
        .iter()
        .map(|r| TeamMatches {
            team_number: r.team_number,
            matches: by_team
                .get(&r.team_number)
                .map(|m| m.values().cloned().collect())
                .unwrap_or_default(),
            stats: None,
        })
        .collect();
    let table = compare::table(&state.season, &overrides, &team_matches, Window::All);
    let offered: Vec<&compare::Row> = table
        .groups
        .iter()
        .flat_map(|g| &g.rows)
        .filter(|r| r.key != graph::POINTS)
        .filter(|r| matches!(r.cells.first(), Some(Cell::Spread(_) | Cell::Rate { .. })))
        .collect();
    // By default every count: what a robot does, in the order of the form.
    let shown: Vec<&compare::Row> = match &params.cols {
        Some(asked) => offered
            .iter()
            .filter(|r| asked.contains(&r.key))
            .copied()
            .collect(),
        None => offered
            .iter()
            .filter(|r| matches!(r.cells.first(), Some(Cell::Spread(_))))
            .copied()
            .collect(),
    };
    let by_metric = params
        .metric_sort
        .as_deref()
        .and_then(|key| shown.iter().position(|r| r.key == key));
    // Rows are in table order here; a metric sort reorders both together.
    let mut order: Vec<usize> = (0..rows.len()).collect();
    if let Some(m) = by_metric {
        let row = shown[m];
        order.sort_by(|a, b| {
            let (x, y) = (row.cells[*a].value(), row.cells[*b].value());
            match (x, y) {
                (Some(x), Some(y)) => match row.better {
                    Better::Lower => x.total_cmp(&y),
                    _ => y.total_cmp(&x),
                },
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            }
            .then(rows[*a].team_number.cmp(&rows[*b].team_number))
        });
    }
    let cols_query: String = shown
        .iter()
        .map(|r| format!("&col={}", r.key))
        .collect::<String>()
        + if params.cols.is_some() { "&cols=1" } else { "" };
    let sort_href = |key: &str| {
        format!(
            "/lead-scout/rankings?event={}&sort={key}{cols_query}",
            event.key
        )
    };
    page.sort = match by_metric {
        Some(m) => format!("{METRIC_SORT}{}", shown[m].key),
        None => sort.as_str().to_string(),
    };
    page.metrics = shown
        .iter()
        .enumerate()
        .map(|(i, r)| MetricColumn {
            value: format!("{METRIC_SORT}{}", r.key),
            label: r.label.clone(),
            href: sort_href(&format!("{METRIC_SORT}{}", r.key)),
            current: by_metric == Some(i),
            hint: match r.better {
                Better::Lower => "fewer is better".into(),
                _ => String::new(),
            },
        })
        .collect();
    page.shown_cols = shown.iter().map(|r| r.key.clone()).collect();
    page.metric_choices = offered
        .iter()
        .map(|r| MetricChoice {
            key: r.key.clone(),
            label: match r.cells.first() {
                Some(Cell::Rate { .. }) => format!("{} (how often)", r.label),
                _ => r.label.clone(),
            },
            checked: shown.iter().any(|s| s.key == r.key),
        })
        .collect();
    let metric_text = |cell: &Cell| match cell {
        Cell::Spread(Some(s)) => tenths(s.mean),
        Cell::Rate { yes, of } if *of > 0 => {
            format!("{}%", (100.0 * *yes as f64 / *of as f64).round())
        }
        _ => String::new(),
    };
    let metric_cells: Vec<Vec<String>> = (0..rows.len())
        .map(|i| shown.iter().map(|r| metric_text(&r.cells[i])).collect())
        .collect();
    let mut slots: Vec<Option<(RankingRow, Vec<String>)>> =
        rows.into_iter().zip(metric_cells).map(Some).collect();
    let rows: Vec<(RankingRow, Vec<String>)> =
        order.iter().filter_map(|i| slots[*i].take()).collect();

    page.rows = rows
        .into_iter()
        .map(|(row, metrics)| {
            let n = row.score.as_ref().map_or(0, |s| s.n);
            RankingView {
                rank: row.rank.map(|r| r.to_string()).unwrap_or_default(),
                team: row.team_number,
                name: row.name,
                score: row
                    .score
                    .map(|s| tenths(s.average))
                    .unwrap_or_default(),
                n,
                thin: n < THIN_BELOW,
                metrics,
            }
        })
        .collect();
    page.columns = [
        ("Rank", SortKey::Rank, true),
        ("Team", SortKey::Number, false),
        ("Name", SortKey::Name, false),
        (SCORE_LABEL, SortKey::Points, true),
    ]
    .into_iter()
    .map(|(label, key, numeric)| SortLink {
        value: key.as_str(),
        label,
        href: sort_href(key.as_str()),
        current: by_metric.is_none() && key == sort,
        numeric,
    })
    .collect();
    page
}

/// One decimal, a half rounded up, as Compare shows it: `format!("{:.1}")`
/// rounds an exact half to even, so 6.25 read 6.2 here and 6.3 there.
fn tenths(v: f64) -> String {
    format!("{:.1}", (v * 10.0).round() / 10.0)
}

/// The point-values form. `returned` is a refused post: what was typed, and
/// why it was refused.
pub async fn weights_page(
    state: &AppState,
    nav: Nav,
    notice: String,
    returned: Option<(&[(String, String)], WeightErrors)>,
) -> WeightsPage {
    let overrides = state.repo.weight_overrides().await.unwrap_or_else(|e| {
        warn!("loading point weights: {e}");
        Default::default()
    });
    let mut groups: Vec<WeightGroup> = Vec::new();
    for slot in ranking::weight_slots(&state.season) {
        let name = slot.input_name();
        let stored = overrides.points_for(&slot.field_key, &slot.option_key, slot.default);
        let (value, error) = match &returned {
            Some((pairs, errors)) => (
                pairs
                    .iter()
                    .rev()
                    .find(|(k, _)| *k == name)
                    .map(|(_, v)| v.clone())
                    .unwrap_or_else(|| stored.to_string()),
                errors.get(&name).cloned().unwrap_or_default(),
            ),
            None => (stored.to_string(), String::new()),
        };
        let input = WeightInput {
            changed: value.trim() != slot.default.to_string(),
            name,
            option_label: slot.option_label.clone(),
            value,
            default: slot.default,
            error,
        };
        match groups.last_mut() {
            Some(group) if group.field_label == slot.field_label => group.inputs.push(input),
            _ => groups.push(WeightGroup {
                field_label: slot.field_label.clone(),
                inputs: vec![input],
            }),
        }
    }
    WeightsPage {
        title: "Point values".into(),
        nav,
        rejected: returned
            .as_ref()
            .is_some_and(|(_, errors)| !errors.is_empty()),
        errors: Vec::new(),
        changed: overrides.iter().count(),
        groups,
        notice,
    }
}

/// Why a weights save did not happen.
pub enum WeightsRefused {
    Invalid(WeightErrors),
    Storage,
}

/// `POST /api/weights`: save the whole form, or nothing.
pub async fn save_weights(
    state: &AppState,
    user: &User,
    pairs: &[(String, String)],
) -> Result<(), WeightsRefused> {
    let overrides = ranking::read_weights(&state.season, pairs).map_err(WeightsRefused::Invalid)?;
    state
        .repo
        .replace_weight_overrides(&overrides, Utc::now())
        .await
        .map_err(|e| {
            warn!("saving point weights: {e}");
            WeightsRefused::Storage
        })?;
    info!(user = %user.email, changed = overrides.iter().count(), "point weights saved");
    Ok(())
}

/// `POST /api/weights/reset`: every value back to the schema's.
pub async fn reset_weights(state: &AppState, user: &User) -> bool {
    match state
        .repo
        .replace_weight_overrides(&Default::default(), Utc::now())
        .await
    {
        Ok(()) => {
            info!(user = %user.email, "point weights reset");
            true
        }
        Err(e) => {
            warn!("resetting point weights: {e}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::tenths;

    #[test]
    fn a_half_rounds_up_as_compare_shows_it() {
        assert_eq!(tenths(6.25), "6.3");
        assert_eq!(tenths(6.0), "6.0");
        assert_eq!(tenths(0.04), "0.0");
    }
}
