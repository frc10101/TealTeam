//! The pages this device can make for itself (C5): the service worker's
//! answer when a page cannot be fetched from the server.
//!
//! Each address runs its `tt_pages` function, the one the server runs, over
//! this device's copy. An address not listed here is `None`, and the worker
//! shows the offline shell (C1) as before. C6 adds addresses, read-only first.
//!
//! A device cannot know who is holding it: the session is the server's to
//! check, and signing in offline is C9. So a page made here has no account
//! links, offers every event the copy has, and shows no one's notes. The copy
//! itself holds only what the person who fetched it could see (S10).

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use tracing::warn;
use tt_core::season::SeasonSchema;
use tt_pages::events::{self, EventContext};
use tt_repo::LocalRepo;
use tt_templates::{Nav, Page};

use crate::ClientRepo;

/// The page at `path`, with the query string `query` (no `?`), made from
/// this device's copy. `None` when only the server can answer it.
pub async fn render(
    repo: &ClientRepo,
    season: &SeasonSchema,
    path: &str,
    query: &str,
    now: DateTime<Utc>,
) -> Option<String> {
    // A repeated key keeps its last value, as axum's `Query` does.
    let query: HashMap<String, String> = form_urlencoded::parse(query.as_bytes())
        .into_owned()
        .collect();
    let arg = |key: &str| query.get(key).map(String::as_str);
    let event = events::requested(arg("event"));

    let page = match path {
        "/teams" => {
            let context = events::resolve(repo, None, event.as_deref(), now).await;
            let team = tt_pages::teams::requested(arg("team"));
            tt_pages::teams::page(
                repo,
                season,
                nav(repo, &context).await,
                None,
                &context,
                team.as_deref(),
                now,
            )
            .await
            .render_html()
        }
        _ => return None,
    };
    page.inspect_err(|e| warn!("making {path} on this device: {e}"))
        .ok()
}

/// No one signed in, as far as the device can tell, and made here.
async fn nav(repo: &ClientRepo, context: &EventContext) -> Nav {
    Nav {
        from_device: true,
        event: context.switcher(),
        ..Nav::anonymous(repo.health().await.is_ready())
    }
}
