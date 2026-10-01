//! Which event a page is about (U2, U3): `?event=` and nothing else.
//!
//! A page without one shows a sensible default
//! ([`tt_core::records::default_event`]). `tt-web`'s `EventParam` reads the
//! query with [`requested`], and so does the service worker's dispatch.

use chrono::{DateTime, Utc};
use tracing::warn;
use tt_core::records::{Event, default_event};
use tt_core::user::User;
use tt_repo::LocalRepo;
use tt_templates::EventSwitcher;

/// `?event=`'s value, lowercased, if it is not blank.
///
/// A mangled query means "no event named", never an error: someone mistyping
/// a bookmark should land on a working page.
pub fn requested(raw: Option<&str>) -> Option<String> {
    raw.map(|key| key.trim().to_ascii_lowercase())
        .filter(|key| !key.is_empty())
}

/// The events a viewer can switch between, and the one this page shows.
#[derive(Debug, Default)]
pub struct EventContext {
    /// In date order.
    pub options: Vec<Event>,
    pub selected: Option<Event>,
    /// The URL named an event this database does not have.
    pub unknown: Option<String>,
}

impl EventContext {
    pub fn switcher(&self) -> EventSwitcher {
        EventSwitcher::new(&self.options, self.selected.as_ref())
    }
}

/// Decide which events to offer and which to show.
///
/// Offered: the viewer's team's events; every event when they have no team, or
/// when their team is on no roster yet -- a scout whose team's registration has
/// not synced should still see something (REBUILD_SPEC.md 5.1).
///
/// An event named in the URL is shown even if it is not on that list, so a
/// lead scout can look at the event their next opponent just came from.
///
/// Storage errors degrade to an empty context: the page renders without a
/// switcher rather than failing.
pub async fn resolve<R: LocalRepo>(
    repo: &R,
    viewer: Option<&User>,
    requested: Option<&str>,
    now: DateTime<Utc>,
) -> EventContext {
    let mut options = match viewer.and_then(|u| u.team_number) {
        Some(team) => match repo.events_for_team(team).await {
            Ok(events) if !events.is_empty() => events,
            Ok(_) => all_events(repo).await,
            Err(e) => {
                warn!("listing events for team {team}: {e}");
                Vec::new()
            }
        },
        None => all_events(repo).await,
    };

    let mut unknown = None;
    let mut selected = None;
    if let Some(key) = requested {
        if let Some(event) = options.iter().find(|e| e.key == key) {
            selected = Some(event.clone());
        } else {
            match repo.event(key).await {
                Ok(Some(event)) => {
                    options.push(event.clone());
                    // Keep the list in date order, matching the repo's ordering.
                    options.sort_by(|a, b| {
                        (a.start_date.is_none(), a.start_date, &a.name).cmp(&(
                            b.start_date.is_none(),
                            b.start_date,
                            &b.name,
                        ))
                    });
                    selected = Some(event);
                }
                Ok(None) => unknown = Some(key.to_string()),
                Err(e) => warn!("looking up event {key}: {e}"),
            }
        }
    }

    let selected = selected.or_else(|| default_event(&options, now).cloned());
    EventContext {
        options,
        selected,
        unknown,
    }
}

async fn all_events<R: LocalRepo>(repo: &R) -> Vec<Event> {
    repo.list_events().await.unwrap_or_else(|e| {
        warn!("listing events: {e}");
        Vec::new()
    })
}
