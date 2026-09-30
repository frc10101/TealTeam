//! The scouting page and the save behind it (U4).
//!
//! A scout picks a match, then one of its six robots, then fills in a form drawn
//! from the season schema. Every step is in the URL -- `?event=&match=&team=` --
//! so a reload, a bookmark, or a second tab lands on the same step.
//!
//! The match and robot are always re-checked against the schedule on save. The
//! alliance is read off the match rather than asked, which removes one more
//! thing a scout could get wrong.

use std::collections::HashMap;
use std::convert::Infallible;

use axum::extract::{FromRequestParts, Query};
use axum::http::request::Parts;
use chrono::Utc;
use rand::RngCore;
use tracing::{info, warn};
use tt_core::assignments::{self, AssigneeKey};
use tt_core::form::{FormErrors, RawAnswers, read_answers};
use tt_core::record_id;
use tt_core::records::MatchRecord;
use tt_core::user::User;
use tt_repo::{NewObservation, Recorded, Repo, RepoError};
use tt_templates::{
    AssignedCard, Draft, Keypad, MatchLink, MatchPicker, Nav, RosterEntry, ScoutForm,
    SubmissionPage, choose_href, draft_key, scout_href,
};

use crate::events::EventContext;
use crate::startup::AppState;

const STORAGE_DOWN: &str =
    "The server's storage is unavailable, so nothing can be recorded right now.";

/// `?match=`, `&team=`, `&choose=`, and `&saved=`: which step of the page to
/// show.
///
/// Never rejects, like [`crate::events::EventParam`]: a mangled link should
/// land on a working page that says what was wrong.
#[derive(Debug, Default)]
pub struct ScoutParams {
    pub match_key: Option<String>,
    pub team: Option<i32>,
    /// Show the robot picker even where the scout has an assignment: the
    /// deliberate override (L5).
    pub choose: bool,
    /// The robot the scout just saved, for the confirmation.
    pub saved: Option<i32>,
}

impl<S: Send + Sync> FromRequestParts<S> for ScoutParams {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Infallible> {
        let query = Query::<HashMap<String, String>>::from_request_parts(parts, state)
            .await
            .map(|Query(query)| query)
            .unwrap_or_default();
        let lookup = |name: &str| query.get(name).map(String::as_str);
        Ok(Self {
            match_key: match_key(lookup("match")),
            team: team_number(lookup("team")),
            choose: lookup("choose").is_some_and(|v| !v.trim().is_empty()),
            saved: team_number(lookup("saved")),
        })
    }
}

fn match_key(raw: Option<&str>) -> Option<String> {
    raw.map(|k| k.trim().to_ascii_lowercase())
        .filter(|k| !k.is_empty())
}

fn team_number(raw: Option<&str>) -> Option<i32> {
    raw.and_then(|t| t.trim().parse().ok()).filter(|t| *t > 0)
}

/// A fresh UUIDv7 for a form about to be shown (D7).
pub fn new_record_id() -> String {
    let mut random = [0u8; 10];
    rand::rng().fill_bytes(&mut random);
    let unix_ms = u64::try_from(Utc::now().timestamp_millis()).unwrap_or(0);
    record_id::uuid_v7(unix_ms, random)
}

/// This browser's tablet, if it has checked in.
pub async fn device_id(state: &AppState, device_uuid: Option<&str>) -> Option<i64> {
    match state.repo.device_by_uuid(device_uuid?).await {
        Ok(device) => device.map(|d| d.id),
        Err(e) => {
            warn!("looking up device: {e}");
            None
        }
    }
}

/// Assemble the scouting page for `context`'s event.
///
/// With no match or robot named, a scout with an assignment (L4) lands on it:
/// "You are scouting 1678", form open, no picker (L5). An assignment is
/// theirs if it names their account or the tablet they are on (`device_id`).
///
/// `draft` is a form coming back after a failed save; `errors` are messages
/// from that save. Storage failures degrade to a page that says so; a failed
/// read of assignments only costs the shortcut.
#[allow(clippy::too_many_arguments)]
pub async fn page(
    state: &AppState,
    user: &User,
    device_id: Option<i64>,
    nav: Nav,
    context: &EventContext,
    params: &ScoutParams,
    draft: Option<Draft>,
    mut errors: Vec<String>,
) -> SubmissionPage {
    let storage_ready = nav.storage_ready;
    let mut page = SubmissionPage {
        title: "Scout".into(),
        nav,
        unavailable: String::new(),
        picker: None,
        saved: String::new(),
        saved_draft: String::new(),
        errors: Vec::new(),
        notice: String::new(),
        form: None,
        assigned: None,
        off_assignment: String::new(),
        next_duty: None,
        missed: Vec::new(),
        keypad: None,
        declined: Vec::new(),
    };

    let Some(event) = &context.selected else {
        page.unavailable = if storage_ready {
            "No events have been loaded yet, so there is nothing to scout.".into()
        } else {
            STORAGE_DOWN.into()
        };
        page.errors = errors;
        return page;
    };
    if let Some(unknown) = &context.unknown {
        errors.push(format!(
            "There is no event “{unknown}” on this server. Showing {} instead.",
            event.name
        ));
    }

    let matches = state
        .repo
        .event_matches(&event.key)
        .await
        .unwrap_or_else(|e| {
            warn!("matches for {}: {e}", event.key);
            Vec::new()
        });
    if matches.is_empty() {
        page.unavailable = format!(
            "{} has no match schedule yet. Scouting opens once it is published.",
            event.name
        );
        page.errors = errors;
        return page;
    }

    page.declined = crate::review::declined_notices(state, user, context, &matches).await;

    let recorded_here = state
        .repo
        .recorded_by(&event.key, user.id)
        .await
        .unwrap_or_else(|e| {
            warn!("robots recorded by {} at {}: {e}", user.id, event.key);
            Vec::new()
        });
    let event_assignments = state
        .repo
        .event_assignments(&event.key)
        .await
        .unwrap_or_else(|e| {
            warn!("assignments for {}: {e}", event.key);
            Vec::new()
        });
    let mut me = vec![AssigneeKey::Scout(user.id)];
    me.extend(device_id.map(AssigneeKey::Device));
    let agenda = assignments::agenda(&matches, &event_assignments, &me, &recorded_here);
    // Storage only, even when empty: the retired page asked FIRST here
    // (REBUILD_SPEC.md 12.7). The robots come from the match; the roster only
    // adds names and the keypad's type-ahead.
    let roster = state
        .repo
        .event_teams(&event.key)
        .await
        .unwrap_or_else(|e| {
            warn!("roster for {}: {e}", event.key);
            Vec::new()
        });
    let index_of = |key: &str| matches.iter().position(|m| m.key == key);

    // Which match, which robot, and whether the robot came from an assignment.
    let mut following = false;
    let (index, team) = match (&params.match_key, params.team) {
        (Some(key), team) => {
            let index = index_of(key).or_else(|| {
                errors.push(format!("There is no match “{key}” at {}.", event.name));
                None
            });
            match index {
                Some(i) => {
                    let assigned = (!params.choose && team.is_none())
                        .then(|| agenda.open_in(key))
                        .flatten();
                    following = assigned.is_some();
                    (i, team.or(assigned))
                }
                None => (next_to_scout(&matches), None),
            }
        }
        // The keypad (L3): a team, no match. Its next unplayed match, or its
        // last one once they are all played.
        (None, Some(team)) => {
            let theirs = |m: &&MatchRecord| m.alliance_of(team).is_some();
            let found = matches
                .iter()
                .filter(theirs)
                .find(|m| !m.played)
                .or_else(|| matches.iter().rfind(|m| m.alliance_of(team).is_some()));
            match found {
                Some(m) => (index_of(&m.key).expect("from this list"), Some(team)),
                None => {
                    errors.push(format!(
                        "Team {team} is not on the schedule at {}.",
                        event.name
                    ));
                    (next_to_scout(&matches), None)
                }
            }
        }
        (None, None) => match (&agenda.next, params.choose) {
            (Some((key, team)), false) => {
                following = true;
                (index_of(key).expect("from this schedule"), Some(*team))
            }
            _ => (next_to_scout(&matches), None),
        },
    };
    let record = &matches[index];
    let label = record.label();

    let recorded: Vec<i32> = recorded_here
        .iter()
        .filter(|(key, _)| *key == record.key)
        .map(|(_, team)| *team)
        .collect();

    let team = team.filter(|&team| {
        let in_match = record.alliance_of(team).is_some();
        if !in_match {
            errors.push(format!("Team {team} is not in {label}."));
        }
        in_match
    });

    // Only confirm a save that is really there, not whatever the URL claims.
    if let Some(saved) = params.saved.filter(|t| recorded.contains(t)) {
        page.saved =
            format!("Saved team {saved} in {label}. It is waiting for the lead scout's review.");
        page.saved_draft = draft_key(
            user.id,
            &event.key,
            &record.key,
            saved,
            state.season.version,
        );
    }

    let duty_link = |(key, team): &(String, i32)| {
        let label = matches
            .iter()
            .find(|m| m.key == *key)
            .map(MatchRecord::label)
            .unwrap_or_default();
        MatchLink {
            href: scout_href(&event.key, key, Some(*team)),
            label: format!("team {team} in {label}"),
        }
    };
    page.missed = agenda.missed.iter().map(duty_link).collect();
    // Pointing at the next assignment is noise while the page shows it.
    page.next_duty = agenda
        .next
        .as_ref()
        .filter(|(key, t)| !(*key == record.key && team == Some(*t)))
        .map(duty_link);

    let assigned_here = agenda.open_in(&record.key);
    if let (Some(team), Some(mine)) = (team, assigned_here)
        && team != mine
    {
        page.off_assignment = format!(
            "You are assigned team {mine} in {label}. Make sure {team} is the robot you are watching."
        );
    }

    let team_name = |number: i32| {
        roster
            .iter()
            .find(|t| t.number == number)
            .map(|t| t.name.clone())
    };
    if following && let Some(team) = team {
        let station = [("Red", &record.red), ("Blue", &record.blue)]
            .into_iter()
            .find_map(|(side, slots)| {
                let i = slots.iter().position(|s| *s == Some(team))?;
                Some(format!("{side} {}", i + 1))
            })
            .unwrap_or_default();
        page.assigned = Some(AssignedCard {
            team,
            team_name: team_name(team).unwrap_or_default(),
            where_: format!("{label} · {station}"),
            choose_href: choose_href(&event.key, &record.key),
        });
    } else {
        page.picker = Some(MatchPicker::new(
            &event.key,
            &matches,
            index,
            team,
            &recorded,
            assigned_here,
        ));
        page.keypad = Some(Keypad {
            event_key: event.key.clone(),
            roster: roster
                .iter()
                .map(|t| RosterEntry {
                    number: t.number,
                    name: t.name.clone(),
                    is_viewer: false,
                })
                .collect(),
        });
    }

    if let Some(team) = team {
        if recorded.contains(&team) {
            page.notice = format!("You have already recorded team {team} in {label}.");
        } else {
            let name = match team_name(team) {
                Some(name) => Some(name),
                None => match state.repo.team(team).await {
                    Ok(found) => found.map(|t| t.name),
                    Err(e) => {
                        warn!("loading team {team}: {e}");
                        None
                    }
                },
            };
            let mut form = ScoutForm::new(
                &state.season,
                record,
                team,
                name.as_deref(),
                draft.unwrap_or_else(|| Draft::fresh(new_record_id())),
                user.team_number.is_none(),
            );
            form.draft_key =
                draft_key(user.id, &event.key, &record.key, team, state.season.version);
            page.form = Some(form);
        }
    }

    page.errors = errors;
    page
}

/// The match a scout most likely wants: the first not yet played, or the last
/// one once they all have been.
fn next_to_scout(matches: &[MatchRecord]) -> usize {
    matches
        .iter()
        .position(|m| !m.played)
        .unwrap_or(matches.len().saturating_sub(1))
}

/// A save that did not happen, and what to show instead.
pub struct Rejected {
    /// The event to show: the match's, once the match is known.
    pub event_key: Option<String>,
    pub params: ScoutParams,
    /// The form to show again, answers intact. `None` when there is no form to
    /// go back to.
    pub draft: Option<Draft>,
    pub errors: Vec<String>,
}

/// Save a posted observation. `Ok` is where to send the scout next.
///
/// The rejection is boxed only because it is large; it is the rarer path.
pub async fn submit(
    state: &AppState,
    user: &User,
    device_uuid: Option<&str>,
    pairs: &[(String, String)],
) -> Result<String, Box<Rejected>> {
    let posted = |name: &str| {
        pairs
            .iter()
            .rev()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    };
    let params = ScoutParams {
        match_key: match_key(posted("match")),
        team: team_number(posted("team")),
        choose: false,
        saved: None,
    };
    // A form that somehow lost its id still saves: the per-scout coverage
    // index stops a second copy, so a scout's work is never refused over it.
    let record_id = posted("record_id")
        .and_then(record_id::normalize)
        .unwrap_or_else(new_record_id);
    let answers = RawAnswers::from_pairs(pairs);
    let draft = |errors: FormErrors| Draft {
        record_id: record_id.clone(),
        answers: answers.clone(),
        errors,
    };

    // The schedule decides which match, robot, and alliance this is -- never
    // the post alone.
    let found = match &params.match_key {
        Some(key) => state.repo.match_by_key(key).await,
        None => Ok(None),
    };
    let record = match found {
        Ok(Some(record)) => record,
        Ok(None) => {
            return Err(Box::new(Rejected {
                event_key: None,
                params,
                draft: None,
                errors: vec!["Not saved: that match is not on the schedule.".into()],
            }));
        }
        Err(e) => {
            warn!("loading match for a submission: {e}");
            return Err(Box::new(Rejected {
                event_key: None,
                params,
                draft: None,
                errors: vec![format!("Not saved. {STORAGE_DOWN}")],
            }));
        }
    };
    let event_key = Some(record.event_key.clone());

    let Some((team, alliance)) = params
        .team
        .and_then(|team| Some((team, record.alliance_of(team)?)))
    else {
        return Err(Box::new(Rejected {
            event_key,
            params,
            draft: None,
            errors: vec!["Not saved: pick the robot you watched from this match.".into()],
        }));
    };

    let payload = match read_answers(&state.season, &answers) {
        Ok(payload) => payload,
        Err(errors) => {
            return Err(Box::new(Rejected {
                event_key,
                params,
                draft: Some(draft(errors)),
                errors: Vec::new(),
            }));
        }
    };

    let device_id = device_id(state, device_uuid).await;

    let now = Utc::now();
    let observation = NewObservation {
        client_record_id: record_id.clone(),
        match_key: record.key.clone(),
        event_key: record.event_key.clone(),
        team_number: team,
        alliance,
        payload,
        schema_version: state.season.version,
        scouter_id: Some(user.id),
        device_id,
        submitting_team: user.team_number,
        observed_at: now,
    };

    match state.repo.record_observation(&observation, now).await {
        Ok(recorded) => {
            let outcome = match recorded {
                Recorded::Created(_) => "recorded",
                Recorded::Duplicate(_) => "already recorded, repeat ignored",
            };
            info!(user = %user.email, r#match = %record.key, team, "observation {outcome}");
            Ok(format!(
                "/submission?event={}&match={}&saved={team}",
                record.event_key, record.key
            ))
        }
        Err(RepoError::Conflict { .. }) => Err(Box::new(Rejected {
            event_key,
            params,
            draft: None,
            errors: vec!["Not saved: each scout keeps one observation per robot per match.".into()],
        })),
        Err(e) => {
            warn!("recording observation: {e}");
            Err(Box::new(Rejected {
                event_key,
                params,
                draft: Some(draft(FormErrors::default())),
                errors: vec!["Could not save. Your answers are still here — try again.".into()],
            }))
        }
    }
}
