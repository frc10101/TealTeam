//! The pick list (U20).
//!
//! `GET /pick-list?event=` shows the viewer's team's list; `POST
//! /api/pick-list?event=` makes one change to it. What a change does is
//! `tt_core::picklist::apply`, and the write only lands on the list the change
//! was applied to (`Repo::replace_pick_list`). If someone else changed the list
//! in between, the change is simply applied again to what is there now.

use std::collections::HashMap;

use chrono::Utc;
use tracing::{info, warn};
use tt_core::picklist::{self, Edit, Entry, Tag};
use tt_core::user::User;
use tt_repo::Repo;
use tt_templates::{Candidate, Nav, PickListPage, PickRow, TagOption, team_href};

use crate::events::EventContext;
use crate::scouting::new_record_id;
use crate::startup::AppState;

const NOT_SAVED: &str = "Not saved: the server's storage did not answer. Try again.";

/// Enough for any real collision: each retry is one small transaction.
const ATTEMPTS: usize = 3;

pub fn href(event_key: &str) -> String {
    format!("/pick-list?event={event_key}")
}

/// The page. `typed_team` is what a refused add sent, shown again.
pub async fn page(
    state: &AppState,
    nav: Nav,
    user: &User,
    context: &EventContext,
    mut errors: Vec<String>,
    typed_team: String,
    removed: Option<i32>,
) -> PickListPage {
    let storage_ready = nav.storage_ready;
    let mut page = PickListPage {
        title: "Pick list".into(),
        nav,
        event_name: String::new(),
        unavailable: String::new(),
        errors: Vec::new(),
        notice: removed
            .map(|team| format!("Took {team} off the list."))
            .unwrap_or_default(),
        rows: Vec::new(),
        crossed: 0,
        candidates: Vec::new(),
        typed_team,
        tags: TagOption::all(),
        post_href: String::new(),
    };

    let Some(owning_team) = user.team_number else {
        page.unavailable = "Your account has no team, and a pick list belongs to a team.".into();
        return page;
    };
    let Some(event) = &context.selected else {
        page.unavailable = if storage_ready {
            "No events have been loaded yet, so there is no one to pick.".into()
        } else {
            "The server's storage is unavailable, so the pick list cannot be shown.".into()
        };
        return page;
    };
    page.event_name = event.name.clone();
    page.post_href = format!("/api/pick-list?event={}", event.key);
    if let Some(unknown) = &context.unknown {
        errors.push(format!(
            "There is no event “{unknown}” on this server. Showing {} instead.",
            event.name
        ));
    }

    let list = match state.repo.pick_list(owning_team, &event.key).await {
        Ok(list) => list,
        Err(e) => {
            warn!("pick list for {owning_team} at {}: {e}", event.key);
            page.unavailable = "The pick list could not be read. Try again.".into();
            page.errors = errors;
            return page;
        }
    };
    let roster = state
        .repo
        .event_teams(&event.key)
        .await
        .inspect_err(|e| warn!("roster for {}: {e}", event.key))
        .unwrap_or_default();
    // Ranks are a nicety here: without them the list still works.
    let ranks: HashMap<i32, i32> = state
        .repo
        .event_stats(&event.key)
        .await
        .inspect_err(|e| warn!("rankings for {}: {e}", event.key))
        .unwrap_or_default()
        .into_iter()
        .filter_map(|s| Some((s.team_number, s.rank?)))
        .collect();
    let name_of = |team: i32| {
        roster
            .iter()
            .find(|t| t.number == team)
            .map(|t| t.name.clone())
            .unwrap_or_default()
    };
    let rank_of = |team: i32| {
        ranks
            .get(&team)
            .map(|r| format!("Rank {r}"))
            .unwrap_or_default()
    };

    let last = list.len();
    page.rows = list
        .iter()
        .enumerate()
        .map(|(index, entry)| PickRow {
            place: index + 1,
            team: entry.team_number,
            name: name_of(entry.team_number),
            rank: rank_of(entry.team_number),
            tag: entry.tag.map(Tag::key).unwrap_or_default().into(),
            tag_label: entry.tag.map(Tag::label).unwrap_or_default().into(),
            crossed: entry.crossed,
            first: index == 0,
            last: index + 1 == last,
            profile_href: team_href(&event.key, entry.team_number),
        })
        .collect();
    page.crossed = list.iter().filter(|e| e.crossed).count();

    let mut candidates: Vec<&tt_core::records::Team> = roster
        .iter()
        .filter(|t| !list.iter().any(|e| e.team_number == t.number))
        .collect();
    // Ranked teams first, best first; then the rest by number.
    candidates.sort_by_key(|t| (ranks.get(&t.number).copied().unwrap_or(i32::MAX), t.number));
    page.candidates = candidates
        .into_iter()
        .map(|t| Candidate {
            team: t.number,
            name: t.name.clone(),
            rank: rank_of(t.number),
        })
        .collect();
    page.errors = errors;
    page
}

/// Read one change from the posted form.
fn read_edit(form: &[(String, String)]) -> Result<Edit, String> {
    let field = |name: &str| {
        form.iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.trim())
            .unwrap_or_default()
    };
    let raw_team = field("team");
    if raw_team.is_empty() {
        return Err("Type a team number.".into());
    }
    let team = raw_team
        .strip_prefix("frc")
        .unwrap_or(raw_team)
        .parse::<i32>()
        .ok()
        .filter(|n| (1..=99_999).contains(n))
        .ok_or_else(|| format!("“{raw_team}” is not a team number."))?;

    Ok(match field("op") {
        "add" => Edit::Add {
            team,
            record_id: new_record_id(),
        },
        "remove" => Edit::Remove { team },
        "up" => Edit::Up { team },
        "down" => Edit::Down { team },
        "move" => Edit::MoveTo {
            team,
            place: field("place")
                .parse()
                .ok()
                .filter(|p| *p >= 1)
                .ok_or_else(|| format!("Type the place to move {team} to, from 1."))?,
        },
        "cross" => Edit::Cross {
            team,
            crossed: true,
        },
        "uncross" => Edit::Cross {
            team,
            crossed: false,
        },
        "tag" => Edit::Tag {
            team,
            tag: match field("tag") {
                "" => None,
                key => Some(Tag::parse(key).ok_or_else(|| format!("“{key}” is not a colour."))?),
            },
        },
        _ => return Err("That change was not recognised. Reload the page and try again.".into()),
    })
}

/// One change: where to go next, or why it was refused.
pub async fn change(
    state: &AppState,
    user: &User,
    requested: Option<&str>,
    context: &EventContext,
    form: &[(String, String)],
) -> Result<String, String> {
    let Some(owning_team) = user.team_number else {
        return Err("Your account has no team, and a pick list belongs to a team.".into());
    };
    // Only ever the event the form named, never the default in its place.
    let event = match (&context.selected, requested) {
        (Some(event), Some(key)) if event.key == key => event,
        _ => return Err("The event was not recognised. Choose it again.".into()),
    };
    let edit = read_edit(form)?;

    let roster: Vec<i32> = match state.repo.event_teams(&event.key).await {
        Ok(roster) => roster.iter().map(|t| t.number).collect(),
        Err(e) => {
            warn!("roster for {}: {e}", event.key);
            return Err(NOT_SAVED.into());
        }
    };

    for _ in 0..ATTEMPTS {
        let before: Vec<Entry> = match state.repo.pick_list(owning_team, &event.key).await {
            Ok(list) => list,
            Err(e) => {
                warn!("pick list for {owning_team} at {}: {e}", event.key);
                return Err(NOT_SAVED.into());
            }
        };
        let mut after = before.clone();
        picklist::apply(&mut after, &edit, &roster)?;

        let written = after == before
            || match state
                .repo
                .replace_pick_list(owning_team, &event.key, &before, &after, Utc::now())
                .await
            {
                Ok(written) => written,
                Err(e) => {
                    warn!("storing pick list for {owning_team} at {}: {e}", event.key);
                    return Err(NOT_SAVED.into());
                }
            };
        if written {
            info!(user = %user.email, event = %event.key, ?edit, "pick list changed");
            let team = edit.team();
            return Ok(match edit {
                Edit::Remove { .. } => format!("{}&removed={team}", href(&event.key)),
                _ => format!("{}#team-{team}", href(&event.key)),
            });
        }
    }
    Err("Someone else was changing the list at the same moment. Try again.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(n, v)| (n.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn a_change_is_read_from_its_form() {
        assert_eq!(
            read_edit(&form(&[("op", "move"), ("team", "254"), ("place", "3")])),
            Ok(Edit::MoveTo {
                team: 254,
                place: 3
            })
        );
        assert_eq!(
            read_edit(&form(&[("op", "tag"), ("team", "frc254"), ("tag", "")])),
            Ok(Edit::Tag {
                team: 254,
                tag: None
            })
        );
        assert!(matches!(
            read_edit(&form(&[("op", "add"), ("team", " 1678 ")])),
            Ok(Edit::Add { team: 1678, .. })
        ));
    }

    #[test]
    fn a_bad_change_says_what_was_wrong() {
        assert_eq!(
            read_edit(&form(&[("op", "add"), ("team", "")])),
            Err("Type a team number.".into())
        );
        assert_eq!(
            read_edit(&form(&[("op", "add"), ("team", "12a")])),
            Err("“12a” is not a team number.".into())
        );
        assert_eq!(
            read_edit(&form(&[("op", "move"), ("team", "254"), ("place", "0")])),
            Err("Type the place to move 254 to, from 1.".into())
        );
        assert_eq!(
            read_edit(&form(&[("op", "tag"), ("team", "254"), ("tag", "mauve")])),
            Err("“mauve” is not a colour.".into())
        );
    }
}
