//! Typing the rankings in (I14).
//!
//! `GET /lead-scout/rankings/enter?event=` shows the stored ranking as text to
//! correct; `POST /api/rankings/manual?event=` replaces it. The parsing and
//! every rule about what may be typed live in `tt_core::standings`, and what a
//! save does to the table is `Repo::record_standings`. This is the glue.

use chrono::Utc;
use tracing::{info, warn};
use tt_core::connectivity::describe_age;
use tt_core::standings;
use tt_core::user::User;
use tt_repo::Repo;
use tt_templates::{Nav, RankingsEntryPage};

use crate::events::EventContext;
use crate::startup::AppState;

const NOT_SAVED: &str = "The server's storage did not answer. Try again.";

/// The entry page's own address, for one event.
pub fn href(event_key: &str) -> String {
    format!("/lead-scout/rankings/enter?event={event_key}")
}

/// The page. `typed` is the text a refused save sent, which is shown again as
/// it was rather than replaced by what is stored.
pub async fn page(
    state: &AppState,
    nav: Nav,
    context: &EventContext,
    typed: Option<String>,
    errors: Vec<String>,
    saved: Option<usize>,
) -> RankingsEntryPage {
    let storage_ready = nav.storage_ready;
    let back_href = format!("/lead-scout{}", nav.event.query());
    let mut page = RankingsEntryPage {
        title: "Type rankings in".into(),
        nav,
        event_name: String::new(),
        unavailable: String::new(),
        errors,
        refused: typed.is_some(),
        notice: String::new(),
        text: String::new(),
        as_of: String::new(),
        roster_size: 0,
        post_href: String::new(),
        back_href,
    };

    let Some(event) = &context.selected else {
        page.unavailable = if storage_ready {
            "No events have been loaded yet, so there is nothing to rank.".into()
        } else {
            "The server's storage is unavailable, so rankings cannot be entered.".into()
        };
        return page;
    };
    page.event_name = event.name.clone();
    page.post_href = format!("/api/rankings/manual?event={}", event.key);
    if let Some(unknown) = &context.unknown {
        page.errors.push(format!(
            "There is no event “{unknown}” on this server. Showing {} instead.",
            event.name
        ));
    }
    if let Some(count) = saved {
        page.notice = format!(
            "Saved {count} {}.",
            if count == 1 { "rank" } else { "ranks" }
        );
    }

    page.roster_size = match state.repo.event_teams(&event.key).await {
        Ok(roster) => roster.len(),
        Err(e) => {
            warn!("roster for {}: {e}", event.key);
            0
        }
    };

    match state.repo.event_stats(&event.key).await {
        Ok(stats) => {
            let newest = stats
                .iter()
                .filter(|s| s.rank.is_some())
                .filter_map(|s| s.synced_at)
                .max();
            match typed {
                // The box holds what was just sent, not the stored ranking,
                // so its age would describe the wrong thing.
                Some(typed) => page.text = typed,
                None => {
                    if let Some(at) = newest {
                        page.as_of = describe_age(Utc::now() - at);
                    }
                    page.text = standings::format(&stats);
                }
            }
        }
        Err(e) => {
            warn!("rankings for {}: {e}", event.key);
            // Typing over a ranking that could not be read is still the point
            // of the page; say so rather than hiding the form.
            page.errors
                .push("The stored ranking could not be read, so the box starts empty.".into());
            page.text = typed.unwrap_or_default();
        }
    }
    page
}

/// A save: where to go next, or the reasons it was refused.
pub async fn save(
    state: &AppState,
    user: &User,
    requested: Option<&str>,
    context: &EventContext,
    text: &str,
) -> Result<String, Vec<String>> {
    // Only ever the event the form named. Falling back to the default event
    // would write one event's standings over another's.
    let event = match (&context.selected, requested) {
        (Some(event), Some(key)) if event.key == key => event,
        _ => {
            return Err(vec![
                "The event was not recognised. Choose it again.".into(),
            ]);
        }
    };

    let roster: Vec<i32> = match state.repo.event_teams(&event.key).await {
        Ok(roster) => roster.iter().map(|t| t.number).collect(),
        Err(e) => {
            warn!("roster for {}: {e}", event.key);
            return Err(vec![NOT_SAVED.into()]);
        }
    };
    let parsed = standings::parse(text, &roster)?;

    match state
        .repo
        .record_standings(&event.key, &parsed, Utc::now())
        .await
    {
        Ok(()) => {
            info!(user = %user.email, event = %event.key, ranks = parsed.len(), "rankings typed in");
            Ok(format!("{}&saved={}", href(&event.key), parsed.len()))
        }
        Err(e) => {
            warn!("storing typed rankings for {}: {e}", event.key);
            Err(vec![NOT_SAVED.into()])
        }
    }
}
