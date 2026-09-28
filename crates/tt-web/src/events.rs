//! Which event a page is about (U2, U3).
//!
//! The retired app stored the choice on the session (`selected_event_id`), so
//! every render needed a session read, nothing was bookmarkable, two tabs could
//! not show two events, and nothing could work offline (REBUILD_SPEC.md 12.12).
//!
//! Here the event is in the URL, `?event=2026mabil`, and nowhere else. A page
//! without one shows a sensible default ([`tt_core::records::default_event`]),
//! and the header switcher is a GET form -- so choosing an event is a plain
//! navigation, and a retired `POST /api/events/select` has nothing left to do.

use std::collections::HashMap;
use std::convert::Infallible;

use axum::extract::{FromRequestParts, Query};
use axum::http::request::Parts;
use chrono::NaiveDate;
use tracing::warn;
use tt_core::records::{Event, default_event};
use tt_core::user::User;
use tt_repo::Repo;
use tt_templates::{EventPanel, EventSummary, EventSwitcher, StoredCounts};

/// `?event=`, lowercased, if the URL has a non-blank one.
///
/// Never rejects. A mangled query string means "no event named", not a 400:
/// someone mistyping a bookmark should land on a working page.
pub struct EventParam(pub Option<String>);

impl<S: Send + Sync> FromRequestParts<S> for EventParam {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Infallible> {
        let requested = Query::<HashMap<String, String>>::from_request_parts(parts, state)
            .await
            .ok()
            .and_then(|Query(mut query)| query.remove("event"))
            .map(|key| key.trim().to_ascii_lowercase())
            .filter(|key| !key.is_empty());
        Ok(EventParam(requested))
    }
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
pub async fn resolve<R: Repo + Sync>(
    repo: &R,
    viewer: Option<&User>,
    requested: Option<&str>,
    today: NaiveDate,
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

    let selected = selected.or_else(|| default_event(&options, today).cloned());
    EventContext {
        options,
        selected,
        unknown,
    }
}

async fn all_events<R: Repo + Sync>(repo: &R) -> Vec<Event> {
    repo.list_events().await.unwrap_or_else(|e| {
        warn!("listing events: {e}");
        Vec::new()
    })
}

/// The home page's event card (U3).
pub async fn panel<R: Repo + Sync>(
    repo: &R,
    context: &EventContext,
    viewer_team: Option<i32>,
) -> EventPanel {
    let summary = match &context.selected {
        Some(event) => {
            let roster = repo.event_teams(&event.key).await.unwrap_or_else(|e| {
                warn!("roster for {}: {e}", event.key);
                Vec::new()
            });
            let matches = repo.event_matches(&event.key).await.unwrap_or_else(|e| {
                warn!("matches for {}: {e}", event.key);
                Vec::new()
            });
            Some(EventSummary::new(event, &roster, &matches, viewer_team))
        }
        None => None,
    };

    EventPanel {
        none_loaded: summary.is_none() && context.options.is_empty(),
        summary,
        unknown_key: context.unknown.clone().unwrap_or_default(),
    }
}

/// How much of the selected event is stored, for the lead scout's sync card.
///
/// `None` when storage cannot say: a zero there would read as "the sync lost
/// everything", which is worse than showing nothing.
pub async fn stored<R: Repo + Sync>(repo: &R, context: &EventContext) -> Option<StoredCounts> {
    let event = context.selected.as_ref()?;
    let counted = async {
        let teams = repo.event_teams(&event.key).await?.len();
        let matches = repo.event_matches(&event.key).await?;
        tt_repo::Result::Ok((teams, matches))
    };
    match counted.await {
        Ok((teams, matches)) => Some(StoredCounts {
            event_name: event.name.clone(),
            teams,
            matches: matches.len(),
            played: matches.iter().filter(|m| m.played).count(),
        }),
        Err(e) => {
            warn!("counting what is stored for {}: {e}", event.key);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request;
    use chrono::Utc;
    use tt_core::user::Roles;
    use tt_repo_sqlite::SqliteRepo;

    fn day(month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, month, day).unwrap()
    }

    fn event(key: &str, start: NaiveDate, end: NaiveDate) -> Event {
        Event {
            key: key.into(),
            name: key.to_uppercase(),
            location: None,
            timezone: None,
            start_date: Some(start),
            end_date: Some(end),
            event_code: None,
            event_type: None,
            district_key: None,
            week: None,
        }
    }

    fn viewer(team_number: Option<i32>) -> User {
        User {
            id: 1,
            email: "sam@example.com".into(),
            name: "Sam".into(),
            team_number,
            roles: Roles::SCOUT,
        }
    }

    /// Three events; team 10101 attends only the last.
    async fn repo() -> SqliteRepo {
        let repo = SqliteRepo::connect("sqlite::memory:").expect("connect");
        tt_repo_sqlite::migrate::apply(repo.pool())
            .await
            .expect("migrate");
        let now = Utc::now();
        for e in [
            event("2026early", day(3, 1), day(3, 3)),
            event("2026mid", day(3, 12), day(3, 15)),
            event("2026late", day(3, 26), day(3, 29)),
        ] {
            repo.upsert_event(&e, now).await.expect("event");
        }
        let team = tt_core::records::Team {
            number: 10101,
            name: "Teal Team".into(),
            nickname: None,
            school: None,
            city: None,
            state: None,
            country: None,
            rookie_year: None,
            website: None,
        };
        repo.upsert_team(&team, now).await.expect("team");
        repo.link_event_team("2026late", 10101, now)
            .await
            .expect("link");
        repo
    }

    fn keys(events: &[Event]) -> Vec<&str> {
        events.iter().map(|e| e.key.as_str()).collect()
    }

    async fn param(uri: &str) -> Option<String> {
        let (mut parts, ()) = Request::builder().uri(uri).body(()).unwrap().into_parts();
        let EventParam(key) = EventParam::from_request_parts(&mut parts, &())
            .await
            .unwrap();
        key
    }

    #[tokio::test]
    async fn the_event_parameter_is_read_leniently() {
        assert_eq!(
            param("/?event=2026MABIL").await.as_deref(),
            Some("2026mabil")
        );
        assert_eq!(
            param("/?event=%202026mabil%20").await.as_deref(),
            Some("2026mabil")
        );
        assert_eq!(param("/").await, None);
        assert_eq!(param("/?event=").await, None, "blank is no event");
        // Odd input is never a 400. It passes through as a key, and `resolve`
        // then reports it as an unknown event on a working page.
        assert_eq!(param("/?event=%ZZ").await.as_deref(), Some("%zz"));
        assert_eq!(param("/?event=a&event=b").await.as_deref(), Some("b"));
    }

    #[tokio::test]
    async fn a_viewer_with_a_team_is_offered_that_teams_events() {
        let repo = repo().await;
        let context = resolve(&repo, Some(&viewer(Some(10101))), None, day(3, 13)).await;

        assert_eq!(keys(&context.options), ["2026late"]);
        // Not the event running today: this team is not at it.
        assert_eq!(context.selected.unwrap().key, "2026late");
    }

    #[tokio::test]
    async fn anyone_else_is_offered_everything_and_lands_on_todays_event() {
        let repo = repo().await;
        for who in [None, Some(viewer(None)), Some(viewer(Some(254)))] {
            let context = resolve(&repo, who.as_ref(), None, day(3, 13)).await;
            assert_eq!(keys(&context.options), ["2026early", "2026mid", "2026late"]);
            assert_eq!(context.selected.unwrap().key, "2026mid", "{who:?}");
        }
    }

    #[tokio::test]
    async fn an_event_named_in_the_url_is_shown_even_off_the_teams_list() {
        let repo = repo().await;
        let context = resolve(
            &repo,
            Some(&viewer(Some(10101))),
            Some("2026early"),
            day(3, 13),
        )
        .await;

        assert_eq!(context.selected.as_ref().unwrap().key, "2026early");
        assert_eq!(
            keys(&context.options),
            ["2026early", "2026late"],
            "added to the switcher, in date order"
        );
        assert!(context.unknown.is_none());
    }

    #[tokio::test]
    async fn an_unknown_event_falls_back_and_says_so() {
        let repo = repo().await;
        let context = resolve(&repo, None, Some("2026nope"), day(3, 13)).await;

        assert_eq!(context.unknown.as_deref(), Some("2026nope"));
        assert_eq!(context.selected.unwrap().key, "2026mid");
    }

    #[tokio::test]
    async fn an_empty_database_is_reported_as_such() {
        let repo = SqliteRepo::connect("sqlite::memory:").expect("connect");
        tt_repo_sqlite::migrate::apply(repo.pool())
            .await
            .expect("migrate");

        let context = resolve(&repo, None, None, day(3, 13)).await;
        let panel = panel(&repo, &context, None).await;

        assert!(panel.none_loaded);
        assert!(panel.summary.is_none());
    }
}
