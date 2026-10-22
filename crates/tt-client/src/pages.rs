//! The pages this device can make for itself (C5): the service worker's
//! answer when a page cannot be fetched from the server.
//!
//! Each address runs its `tt_pages` function, the one the server runs, over
//! this device's copy. An address not listed here is `None`, and the worker
//! shows the offline shell (C1) as before. C6 moved what a scout reads at an
//! event: home, a team, the notes, and the graph. C6b added the drive coach's
//! schedule, for the copy's team. The server keeps that page to coaches; the
//! device knows no roles, so it answers whoever opens the address, and its tab
//! bar does not offer it. What it shows is the published schedule.
//!
//! A device cannot know who is holding it: the session is the server's to
//! check, and signing in offline is C9. So a page made here has no account
//! links. It knows whose copy it holds, though: the snapshot names the team
//! it was cut for ([`TEAM_SOURCE`]), and holds no other team's notes (S10).
//! Pages made here are that team's: its events offered, its notes shown.
//!
//! OF1 added the scouting page and help, and [`save`]: a scout with no server
//! scouts on, as whoever the last signed-in page said scouts here
//! ([`crate::viewer`]). A save is checked as the server checks it
//! (`tt_pages::scouting::read_post`) and goes into this device's outbox (C7),
//! which the sync pushes once the server answers. Without a viewer there is
//! no scouting page to make, and the worker shows the offline shell.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use tracing::warn;
use tt_core::season::SeasonSchema;
use tt_core::token::Claims;
use tt_pages::events::{self, EventContext};
use tt_pages::scouting::{self, Rejected, Saved, ScoutParams, ServerOnly};
use tt_repo::{LocalRepo, RepoError};
use tt_templates::{HelpPage, Nav, Page};

use crate::ClientRepo;

/// What [`save`] did with a posted form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveOutcome {
    /// Queued: go here, the match confirmed and the next robot offered.
    Saved(String),
    /// Not saved, and this page says why: the form again, or the match.
    Page(String),
}

/// Makes a record id for a new form (D7). The caller supplies the device's
/// own randomness, so this compiles off the browser.
pub type FreshId<'a> = &'a dyn Fn() -> String;

/// The page at `path`, with the query string `raw` (no `?`), made from
/// this device's copy. `None` when only the server can answer it.
pub async fn render(
    repo: &ClientRepo,
    season: &SeasonSchema,
    path: &str,
    raw: &str,
    now: DateTime<Utc>,
    fresh_id: FreshId<'_>,
) -> Option<String> {
    // A repeated key keeps its last value, as axum's `Query` does.
    let query: HashMap<String, String> = form_urlencoded::parse(raw.as_bytes())
        .into_owned()
        .collect();
    let arg = |key: &str| query.get(key).map(String::as_str);
    let event = events::requested(arg("event"));
    if ![
        "/",
        "/teams",
        "/notes",
        "/graph",
        "/drive-coach",
        "/submission",
        "/help",
    ]
    .contains(&path)
    {
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
        "/drive-coach" => tt_pages::coach::page(repo, nav, ours, &context, now)
            .await
            .render_html(),
        "/submission" => {
            let claims = scout(repo)?;
            let params = ScoutParams::from_lookup(arg);
            Ok(scouting_page(
                repo, season, &claims, nav, &context, &params, None, fresh_id,
            )
            .await)
        }
        "/help" => HelpPage {
            title: "Help".into(),
            nav,
        }
        .render_html(),
        _ => return None,
    };
    page.inspect_err(|e| warn!("making {path} on this device: {e}"))
        .ok()
}

/// Who scouts here, if a signed-in page has said ([`crate::viewer`]).
fn scout(repo: &ClientRepo) -> Option<Claims> {
    repo.viewer()
        .inspect_err(|e| warn!("reading who scouts here: {e}"))
        .ok()
        .flatten()
}

/// The scouting page for `claims`' scout, from this device's copy: `params`'
/// step, or a save's rejection shown again.
#[allow(clippy::too_many_arguments)]
async fn scouting_page(
    repo: &ClientRepo,
    season: &SeasonSchema,
    claims: &Claims,
    nav: Nav,
    context: &EventContext,
    params: &ScoutParams,
    rejected: Option<Rejected>,
    fresh_id: FreshId<'_>,
) -> String {
    let user = crate::viewer::user_of(claims);
    let device_id = repo
        .device_by_uuid(&claims.device)
        .await
        .ok()
        .flatten()
        .map(|d| d.id);
    let mut server = ServerOnly::default();
    if let Some(event) = &context.selected {
        let matches = repo.event_matches(&event.key).await.unwrap_or_default();
        server.declined = scouting::declined_notices(repo, &user, context, &matches).await;
    }
    let (params, draft, errors) = match rejected {
        Some(r) => (r.params, r.draft, r.errors),
        None => (params.clone(), None, Vec::new()),
    };
    scouting::page(
        repo,
        season,
        &user,
        device_id,
        nav,
        context,
        &params,
        draft,
        errors,
        Saved::Held,
        server,
        fresh_id,
    )
    .await
    .render_html()
    .inspect_err(|e| warn!("making /submission on this device: {e}"))
    .unwrap_or_default()
}

/// A scouting form posted with no server (OF1): checked as the server checks
/// it, then recorded on this device and queued for the Pi (C7).
///
/// `None` when this device cannot hold it: no copy, no scout, or the copy
/// would not take the write. The page's own outbox (C13) holds it then.
pub async fn save(
    repo: &ClientRepo,
    season: &SeasonSchema,
    body: &str,
    now: DateTime<Utc>,
    fresh_id: FreshId<'_>,
) -> Option<SaveOutcome> {
    let claims = scout(repo)?;
    let user = crate::viewer::user_of(&claims);
    let pairs: Vec<(String, String)> = form_urlencoded::parse(body.as_bytes())
        .into_owned()
        .collect();
    let device_id = repo
        .device_by_uuid(&claims.device)
        .await
        .ok()
        .flatten()
        .map(|d| d.id);
    let checked = scouting::read_post(repo, season, &user, device_id, &pairs, now, fresh_id).await;
    let rejected = match checked {
        Ok(observation) => match repo.queue_observation(&observation, now).await {
            Ok(_) => return Some(SaveOutcome::Saved(scouting::saved_href(&observation))),
            Err(RepoError::Conflict { .. }) => scouting::one_per_robot(&observation),
            Err(e) => {
                warn!("queueing an observation on this device: {e}");
                return None;
            }
        },
        Err(rejected) => rejected,
    };
    let event = rejected.event_key.clone();
    let context = events::resolve(repo, viewer_team(repo), event.as_deref(), now).await;
    let nav = nav(repo, &context).await;
    let params = rejected.params.clone();
    let page = scouting_page(
        repo,
        season,
        &claims,
        nav,
        &context,
        &params,
        Some(*rejected),
        fresh_id,
    )
    .await;
    Some(SaveOutcome::Page(page))
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
