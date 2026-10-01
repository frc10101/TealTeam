//! The pages this device can make for itself (C5): the service worker's
//! answer when a page cannot be fetched from the server.
//!
//! Each address runs its `tt_pages` function, the one the server runs, over
//! this device's copy. An address not listed here is `None`, and the worker
//! shows the offline shell (C1) as before. C6 moved what a scout reads at an
//! event: home, a team, the notes, and the graph.
//!
//! A device cannot know who is holding it: the session is the server's to
//! check, and signing in offline is C9. So a page made here has no account
//! links. It knows whose copy it holds, though: the snapshot names the team
//! it was cut for ([`TEAM_SOURCE`]), and holds no other team's notes (S10).
//! Pages made here are that team's: its events offered, its notes shown.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use tracing::warn;
use tt_core::season::SeasonSchema;
use tt_pages::events::{self, EventContext};
use tt_repo::LocalRepo;
use tt_templates::{Nav, Page};

use crate::ClientRepo;

/// The page at `path`, with the query string `raw` (no `?`), made from
/// this device's copy. `None` when only the server can answer it.
pub async fn render(
    repo: &ClientRepo,
    season: &SeasonSchema,
    path: &str,
    raw: &str,
    now: DateTime<Utc>,
) -> Option<String> {
    // A repeated key keeps its last value, as axum's `Query` does.
    let query: HashMap<String, String> = form_urlencoded::parse(raw.as_bytes())
        .into_owned()
        .collect();
    let arg = |key: &str| query.get(key).map(String::as_str);
    let event = events::requested(arg("event"));
    if !["/", "/teams", "/notes", "/graph"].contains(&path) {
        return None;
    }
    let ours = viewer_team(repo);
    let context = events::resolve(repo, ours, event.as_deref(), now).await;
    let nav = nav(repo, &context).await;

    let page = match path {
        "/" => tt_pages::home::page(repo, season, nav, ours, &context)
            .await
            .render_html(),
        "/teams" => {
            let team = tt_pages::teams::requested(arg("team"));
            tt_pages::teams::page(repo, season, nav, ours, &context, team.as_deref(), now)
                .await
                .render_html()
        }
        "/notes" => tt_pages::notes::page(repo, season, nav, ours, &context, &query, now)
            .await
            .render_html(),
        "/graph" => {
            // `team` and `metric` repeat: every pair, in order.
            let pairs: Vec<(String, String)> = form_urlencoded::parse(raw.as_bytes())
                .into_owned()
                .collect();
            tt_pages::graph::page(repo, season, nav, &context, &pairs, now)
                .await
                .render_html()
        }
        _ => return None,
    };
    page.inspect_err(|e| warn!("making {path} on this device: {e}"))
        .ok()
}

/// The `sync_state` row the server's snapshot names its team in
/// (`tt_repo_sqlite::snapshot::TEAM_SOURCE`).
pub const TEAM_SOURCE: &str = "snapshot:team";

/// The team this device's copy was cut for, if it was cut for one.
pub fn viewer_team(repo: &ClientRepo) -> Option<i32> {
    repo.cursor(TEAM_SOURCE)
        .inspect_err(|e| warn!("reading whose copy this is: {e}"))
        .ok()
        .and_then(|team| i32::try_from(team).ok())
        .filter(|team| *team > 0)
}

/// No one signed in, as far as the device can tell, and made here.
async fn nav(repo: &ClientRepo, context: &EventContext) -> Nav {
    Nav {
        from_device: true,
        event: context.switcher(),
        ..Nav::anonymous(repo.health().await.is_ready())
    }
}
