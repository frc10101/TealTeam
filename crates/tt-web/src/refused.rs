//! Refused outbox entries (C10): the lead scout's list, one in full, and
//! what a lead makes of it.
//!
//! A push (C7, `crate::push`) keeps every entry it refuses, with the reason
//! it gave the tablet. Most are the schedule's fault, not the scout's: a
//! playoff match not posted yet, or a robot the scout put in the wrong
//! match. So a lead can record one again, against the same match or
//! another, and the push's own checks run on it, as the scout who saved it,
//! from the tablet it came from. It keeps its record id, so when it comes
//! back to that tablet through the change log, the tablet's refused entry
//! clears like any other. Or a lead dismisses it, for answers nobody can
//! place; the tablet still has it, to export or discard.

use std::collections::HashMap;
use std::convert::Infallible;

use axum::extract::{FromRequestParts, Query};
use axum::http::request::Parts;
use chrono::{DateTime, Utc};
use tracing::{info, warn};
use tt_core::connectivity::describe_age;
use tt_core::notes::Notes;
use tt_core::records::MatchRecord;
use tt_core::review;
use tt_core::user::User;
use tt_repo::{Recorded, Repo};
use tt_repo_sqlite::refused::{Refusal, Resolution};
use tt_templates::{MatchChoice, Nav, RefusedItem, RefusedList, RefusedPage};

use crate::events::EventContext;
use crate::push::{Author, record_as};
use crate::startup::AppState;

/// `?refused=`: what a lead just did with one, for the lead-scout page's
/// confirmation. Only a message.
#[derive(Debug, Default)]
pub struct ResolvedParam(Option<bool>);

impl<S: Send + Sync> FromRequestParts<S> for ResolvedParam {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Infallible> {
        let recorded = Query::<HashMap<String, String>>::from_request_parts(parts, state)
            .await
            .ok()
            .and_then(|Query(q)| match q.get("refused").map(String::as_str) {
                Some("recorded") => Some(true),
                Some("dismissed") => Some(false),
                _ => None,
            });
        Ok(Self(recorded))
    }
}

impl ResolvedParam {
    pub fn message(&self) -> String {
        match self.0 {
            Some(true) => {
                "Recorded. It is waiting for review with the rest, and the scout's tablet will clear it when it next syncs.".into()
            }
            Some(false) => "Dismissed. The scout's tablet still has it.".into(),
            None => String::new(),
        }
    }
}

/// `"Q14 · Team 254"`, or the match key as sent when it is not on the
/// schedule.
fn heading(refusal: &Refusal, record: Option<&MatchRecord>) -> String {
    let label = record
        .map(MatchRecord::label)
        .unwrap_or_else(|| refusal.entry.match_key.clone());
    format!("{label} · Team {}", refusal.entry.team_number)
}

fn ago(at: DateTime<Utc>, now: DateTime<Utc>) -> String {
    describe_age(now - at)
}

/// `"Sam on Red tablet, 20 minutes ago"`: who saved it, where, and when
/// they watched.
fn byline(refusal: &Refusal, now: DateTime<Utc>) -> String {
    let scout = refusal
        .scouter_name
        .clone()
        .unwrap_or_else(|| "a scout whose account is gone".into());
    let tablet = refusal
        .device_name
        .as_ref()
        .map(|d| format!(" on {d}"))
        .unwrap_or_default();
    format!("{scout}{tablet}, {}", ago(refusal.entry.observed_at, now))
}

/// The lead-scout page's list: every refusal nobody has dealt with, from
/// any event. Links keep the page's event.
pub async fn list(state: &AppState, context: &EventContext) -> RefusedList {
    let open = match state.repo.open_refusals().await {
        Ok(open) => open,
        Err(e) => {
            warn!("refused entries: {e}");
            return RefusedList {
                items: Vec::new(),
                unavailable: true,
            };
        }
    };
    let query = context
        .selected
        .as_ref()
        .map(|e| format!("?event={}", e.key))
        .unwrap_or_default();
    let now = Utc::now();
    let mut items = Vec::with_capacity(open.len());
    for refusal in &open {
        let record = state
            .repo
            .match_by_key(&refusal.entry.match_key)
            .await
            .ok()
            .flatten();
        items.push(RefusedItem {
            heading: heading(refusal, record.as_ref()),
            byline: byline(refusal, now),
            reason: refusal.reason.clone(),
            href: format!("/lead-scout/refused/{}{query}", refusal.id),
        });
    }
    RefusedList {
        items,
        unavailable: false,
    }
}

/// `"Q14 · Red 254 1678 971 · Blue 2 3 4"`.
fn choice(m: &MatchRecord) -> MatchChoice {
    let side = |slots: &[Option<i32>; 3]| {
        slots
            .iter()
            .flatten()
            .map(i32::to_string)
            .collect::<Vec<_>>()
            .join(" ")
    };
    MatchChoice {
        key: m.key.clone(),
        label: format!(
            "{} · Red {} · Blue {}",
            m.label(),
            side(&m.red),
            side(&m.blue)
        ),
    }
}

/// What the form holds: the entry's own match and team, or what a lead
/// just tried.
pub struct Draft {
    pub match_key: String,
    pub team_number: String,
}

/// One refusal in full, as `viewer` may see it: another team's notes are
/// held back (U13). `None` when there is no such refusal, or storage could
/// not say.
pub async fn page(
    state: &AppState,
    nav: Nav,
    context: &EventContext,
    viewer: &User,
    id: i64,
    draft: Option<Draft>,
    errors: Vec<String>,
) -> Option<RefusedPage> {
    let refusal = match state.repo.refusal(id).await {
        Ok(found) => found?,
        Err(e) => {
            warn!("loading refused entry {id}: {e}");
            return None;
        }
    };
    let record = state
        .repo
        .match_by_key(&refusal.entry.match_key)
        .await
        .ok()
        .flatten();
    let matches = match &context.selected {
        Some(event) => state
            .repo
            .event_matches(&event.key)
            .await
            .unwrap_or_else(|e| {
                warn!("matches for {}: {e}", event.key);
                Vec::new()
            }),
        None => Vec::new(),
    };
    let now = Utc::now();

    let by = refusal
        .resolved_by
        .clone()
        .unwrap_or_else(|| "a lead scout".into());
    let when = refusal
        .resolved_at
        .map(|t| ago(t, now))
        .unwrap_or_else(|| "at an unknown time".into());
    let (resolution, observation_href) = match refusal.resolution {
        None => (String::new(), String::new()),
        Some(Resolution::Recorded(observation)) => (
            format!("Recorded by {by}, {when}."),
            match state.repo.observation(observation).await {
                Ok(Some(o)) => format!("/lead-scout/submissions/{}?event={}", o.id, o.event_key),
                _ => String::new(),
            },
        ),
        Some(Resolution::Dismissed) => (format!("Dismissed by {by}, {when}."), String::new()),
    };
    let other_version = if refusal.entry.schema_version == state.season.version {
        String::new()
    } else {
        format!(
            "Saved on version {} of the form; this is version {}. Answers are shown against the current form.",
            refusal.entry.schema_version, state.season.version
        )
    };
    let draft = draft.unwrap_or_else(|| Draft {
        match_key: refusal.entry.match_key.clone(),
        team_number: refusal.entry.team_number.to_string(),
    });

    Some(RefusedPage {
        title: format!("Refused: {}", heading(&refusal, record.as_ref())),
        nav,
        id,
        heading: heading(&refusal, record.as_ref()),
        byline: byline(&refusal, now),
        reason: refusal.reason.clone(),
        resolution,
        observation_href,
        answers: review::answers(
            &state.season,
            &refusal.entry.payload,
            Notes::for_viewer(viewer.team_number, refusal.submitting_team),
        ),
        hidden_notes: match refusal.submitting_team {
            Some(team) => format!("Only scouts on team {team} can read these notes."),
            None => "Saved without a team, so nobody can read these notes.".into(),
        },
        other_version,
        match_listed: matches.iter().any(|m| m.key == draft.match_key),
        matches: matches.iter().map(choice).collect(),
        match_key: draft.match_key,
        team_number: draft.team_number,
        errors,
        back_href: format!(
            "/lead-scout{}#refused",
            context
                .selected
                .as_ref()
                .map(|e| format!("?event={}", e.key))
                .unwrap_or_default()
        ),
    })
}

/// Something a lead did that did not happen, and what to show instead.
pub enum Refused {
    /// No such refusal.
    Missing,
    /// Back to it, with why and what was in the form.
    Again {
        errors: Vec<String>,
        draft: Option<Draft>,
    },
}

const STORAGE: &str = "Not done: the server's storage did not answer. Try again.";

async fn load(state: &AppState, id: i64) -> Result<Refusal, Refused> {
    let refusal = match state.repo.refusal(id).await {
        Ok(Some(refusal)) => refusal,
        Ok(None) => return Err(Refused::Missing),
        Err(e) => {
            warn!("loading refused entry {id}: {e}");
            return Err(Refused::Again {
                errors: vec![STORAGE.into()],
                draft: None,
            });
        }
    };
    if refusal.resolution.is_some() {
        return Err(Refused::Again {
            errors: vec!["Someone already dealt with this one.".into()],
            draft: None,
        });
    }
    Ok(refusal)
}

fn lead_page(event_key: Option<&str>, what: &str) -> String {
    match event_key {
        Some(key) => format!("/lead-scout?event={key}&refused={what}#refused"),
        None => format!("/lead-scout?refused={what}#refused"),
    }
}

/// Record a refusal against the posted match and team. `Ok` is where to go
/// next: the lead-scout page for the event it landed in.
pub async fn record(
    state: &AppState,
    user: &User,
    id: i64,
    pairs: &[(String, String)],
) -> Result<String, Refused> {
    let posted = |name: &str| {
        pairs
            .iter()
            .rev()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.trim().to_string())
            .unwrap_or_default()
    };
    let draft = Draft {
        match_key: posted("match"),
        team_number: posted("team"),
    };
    let again = |error: String, draft: Draft| Refused::Again {
        errors: vec![error],
        draft: Some(draft),
    };

    let refusal = load(state, id).await?;
    let Ok(team_number) = draft.team_number.parse::<i32>() else {
        return Err(again(
            "Not recorded: the team must be a number.".into(),
            draft,
        ));
    };
    let mut entry = refusal.entry.clone();
    entry.match_key = draft.match_key.clone();
    entry.team_number = team_number;
    let author = Author {
        scouter_id: refusal.scouter_id,
        device_id: refusal.device_id,
        submitting_team: refusal.submitting_team,
    };
    let now = Utc::now();
    let observation = match record_as(state, &entry, author, now).await {
        Ok(Ok(Recorded::Created(o) | Recorded::Duplicate(o))) => o,
        Ok(Err(reason)) => return Err(again(reason, draft)),
        Err(e) => {
            warn!("recording refused entry {id}: {e}");
            return Err(again(STORAGE.into(), draft));
        }
    };
    // Recorded either way. Should this fail, the next press finds the
    // observation already there under the same record id, and marks it.
    match state
        .repo
        .resolve_refusal(id, Resolution::Recorded(observation), user.id, now)
        .await
    {
        Ok(_) => {}
        Err(e) => {
            warn!("resolving refused entry {id}: {e}");
            return Err(again(STORAGE.into(), draft));
        }
    }
    info!(
        user = %user.email,
        refused = id,
        observation,
        "refused entry recorded"
    );
    let event = state
        .repo
        .match_by_key(&entry.match_key)
        .await
        .ok()
        .flatten()
        .map(|m| m.event_key);
    Ok(lead_page(event.as_deref(), "recorded"))
}

/// Dismiss a refusal. `Ok` is the lead-scout page, for `event`.
pub async fn dismiss(
    state: &AppState,
    user: &User,
    id: i64,
    event: Option<&str>,
) -> Result<String, Refused> {
    load(state, id).await?;
    match state
        .repo
        .resolve_refusal(id, Resolution::Dismissed, user.id, Utc::now())
        .await
    {
        Ok(true) => {}
        Ok(false) => {
            return Err(Refused::Again {
                errors: vec!["Someone already dealt with this one.".into()],
                draft: None,
            });
        }
        Err(e) => {
            warn!("dismissing refused entry {id}: {e}");
            return Err(Refused::Again {
                errors: vec![STORAGE.into()],
                draft: None,
            });
        }
    }
    info!(user = %user.email, refused = id, "refused entry dismissed");
    Ok(lead_page(event, "dismissed"))
}
