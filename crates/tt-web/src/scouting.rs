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
use tt_core::form::{FormErrors, RawAnswers, read_answers};
use tt_core::record_id;
use tt_core::records::MatchRecord;
use tt_core::user::User;
use tt_repo::{NewObservation, Recorded, Repo, RepoError};
use tt_templates::{Draft, MatchPicker, Nav, ScoutForm, SubmissionPage};

use crate::events::EventContext;
use crate::startup::AppState;

const STORAGE_DOWN: &str =
    "The server's storage is unavailable, so nothing can be recorded right now.";

/// `?match=`, `&team=`, and `&saved=`: which step of the page to show.
///
/// Never rejects, like [`crate::events::EventParam`]: a mangled link should
/// land on a working page that says what was wrong.
#[derive(Debug, Default)]
pub struct ScoutParams {
    pub match_key: Option<String>,
    pub team: Option<i32>,
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

/// Assemble the scouting page for `context`'s event.
///
/// `draft` is a form coming back after a failed save; `errors` are messages
/// from that save. Storage failures degrade to a page that says so.
pub async fn page(
    state: &AppState,
    user: &User,
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
        errors: Vec::new(),
        notice: String::new(),
        form: None,
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

    let index = match &params.match_key {
        Some(key) => matches.iter().position(|m| &m.key == key).or_else(|| {
            errors.push(format!("There is no match “{key}” at {}.", event.name));
            None
        }),
        None => None,
    }
    .unwrap_or_else(|| next_to_scout(&matches));
    let record = &matches[index];
    let label = record.label();

    let recorded = state
        .repo
        .observed_teams(&record.key, user.id)
        .await
        .unwrap_or_else(|e| {
            warn!("observed teams for {}: {e}", record.key);
            Vec::new()
        });

    let team = params.team.filter(|&team| {
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
    }

    page.picker = Some(MatchPicker::new(
        &event.key, &matches, index, team, &recorded,
    ));

    if let Some(team) = team {
        if recorded.contains(&team) {
            page.notice = format!("You have already recorded team {team} in {label}.");
        } else {
            let name = match state.repo.team(team).await {
                Ok(found) => found.map(|t| t.name),
                Err(e) => {
                    warn!("loading team {team}: {e}");
                    None
                }
            };
            page.form = Some(ScoutForm::new(
                &state.season,
                record,
                team,
                name.as_deref(),
                draft.unwrap_or_else(|| Draft::fresh(new_record_id())),
                user.team_number.is_none(),
            ));
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

    let device_id = match device_uuid {
        Some(uuid) => match state.repo.device_by_uuid(uuid).await {
            Ok(device) => device.map(|d| d.id),
            Err(e) => {
                warn!("looking up device for a submission: {e}");
                None
            }
        },
        None => None,
    };

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
