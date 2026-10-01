//! Rankings (L11) and the point values behind them (L12).

use std::collections::{BTreeSet, HashMap};
use std::convert::Infallible;

use axum::extract::{FromRequestParts, Query};
use axum::http::request::Parts;
use chrono::Utc;
use tracing::{info, warn};
use tt_core::connectivity::Freshness;
use tt_core::ranking::{self, RankingRow, Scored, SortKey, WeightErrors};
use tt_core::review::ReviewState;
use tt_core::user::User;
use tt_pages::teams::latest_scouted;
use tt_repo::{Repo, StoredObservation};
use tt_templates::{
    Nav, RankingView, RankingsPage, SortLink, THIN_BELOW, WeightGroup, WeightInput, WeightsPage,
};

use crate::events::EventContext;
use crate::startup::AppState;

/// `?sort=`, and `?saved=` after a weights save. Never rejects: an unknown
/// sort is the default one.
#[derive(Debug, Default)]
pub struct RankingParams {
    pub sort: SortKey,
    pub saved: Option<String>,
}

impl<S: Send + Sync> FromRequestParts<S> for RankingParams {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Infallible> {
        let query = Query::<HashMap<String, String>>::from_request_parts(parts, state)
            .await
            .map(|Query(q)| q)
            .unwrap_or_default();
        Ok(Self {
            sort: query
                .get("sort")
                .and_then(|s| SortKey::parse(s.trim()))
                .unwrap_or_default(),
            saved: query.get("saved").cloned(),
        })
    }
}

/// The rankings table for `context`'s event, ordered by `sort`.
pub async fn page(
    state: &AppState,
    nav: Nav,
    context: &EventContext,
    sort: SortKey,
) -> RankingsPage {
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

    page.rows = rows
        .into_iter()
        .map(|row| {
            let n = row.score.as_ref().map_or(0, |s| s.n);
            RankingView {
                rank: row.rank.map(|r| r.to_string()).unwrap_or_default(),
                team: row.team_number,
                name: row.name,
                score: row
                    .score
                    .map(|s| format!("{:.1}", s.average))
                    .unwrap_or_default(),
                n,
                thin: n < THIN_BELOW,
            }
        })
        .collect();
    page.columns = [
        ("Rank", SortKey::Rank, true),
        ("Team", SortKey::Number, false),
        ("Name", SortKey::Name, false),
        ("Score", SortKey::Points, true),
    ]
    .into_iter()
    .map(|(label, key, numeric)| SortLink {
        label,
        href: format!(
            "/lead-scout/rankings?event={}&sort={}",
            event.key,
            key.as_str()
        ),
        current: key == sort,
        numeric,
    })
    .collect();
    page
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
