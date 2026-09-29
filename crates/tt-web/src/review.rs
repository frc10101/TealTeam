//! The review pipeline (L8-L10): the queue on the lead-scout page, one
//! observation in full, and the verdict.
//!
//! Approving and declining are updates to the observation's own row -- never a
//! copy into another table, never a delete. A declined observation keeps its
//! answers, the reason, who declined it, and when; the scout is shown the
//! reason with a button to record the robot again (REBUILD_SPEC.md 12.5).

use std::collections::HashMap;
use std::convert::Infallible;

use axum::extract::{FromRequestParts, Query};
use axum::http::request::Parts;
use chrono::{DateTime, Utc};
use tracing::{info, warn};
use tt_core::connectivity::describe_age;
use tt_core::records::MatchRecord;
use tt_core::review::{self, Decision, ReviewState};
use tt_core::user::User;
use tt_repo::{Repo, StoredObservation};
use tt_templates::{DeclinedNotice, Nav, QueueItem, ReviewPage, ReviewQueue, scout_href};

use crate::events::EventContext;
use crate::startup::AppState;

/// `?reviewed=`: the verdict that led to this page, for its confirmation.
/// Only a message, like the grid's `done=`.
#[derive(Debug, Default)]
pub struct ReviewedParam(pub Option<ReviewState>);

impl<S: Send + Sync> FromRequestParts<S> for ReviewedParam {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Infallible> {
        let reviewed = Query::<HashMap<String, String>>::from_request_parts(parts, state)
            .await
            .ok()
            .and_then(|Query(q)| q.get("reviewed").and_then(|v| ReviewState::parse(v)));
        Ok(Self(reviewed))
    }
}

impl ReviewedParam {
    pub fn message(&self) -> String {
        match self.0 {
            Some(ReviewState::Approved) => "Approved.".into(),
            Some(ReviewState::Declined) => {
                "Declined. The scout will see why, with a button to record it again.".into()
            }
            _ => String::new(),
        }
    }
}

/// `"Q14 · 254 · Red 2"`, or the alliance alone when the match is gone.
fn heading(o: &StoredObservation, record: Option<&MatchRecord>, team_word: bool) -> String {
    let team = if team_word {
        format!("Team {}", o.team_number)
    } else {
        o.team_number.to_string()
    };
    let station = record
        .and_then(|m| {
            [("Red", &m.red), ("Blue", &m.blue)]
                .into_iter()
                .find_map(|(side, slots)| {
                    let i = slots.iter().position(|s| *s == Some(o.team_number))?;
                    Some(format!("{side} {}", i + 1))
                })
        })
        .unwrap_or_else(|| capitalised(&o.alliance));
    let label = record
        .map(MatchRecord::label)
        .unwrap_or_else(|| o.match_key.clone());
    format!("{label} · {team} · {station}")
}

fn capitalised(word: &str) -> String {
    let mut chars = word.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

fn ago(at: Option<DateTime<Utc>>, now: DateTime<Utc>) -> String {
    at.map(|t| describe_age(now - t))
        .unwrap_or_else(|| "at an unknown time".into())
}

fn scout_name(o: &StoredObservation) -> String {
    o.scouter_name
        .clone()
        .unwrap_or_else(|| "a scout whose account is gone".into())
}

/// The lead-scout page's queue for `context`'s event. `None` with no event.
pub async fn queue(state: &AppState, context: &EventContext) -> Option<ReviewQueue> {
    let event = context.selected.as_ref()?;
    let live_href = format!("/lead-scout?event={}", event.key);
    let loaded = async {
        let pending = state.repo.pending_observations(&event.key).await?;
        let matches = state.repo.event_matches(&event.key).await?;
        tt_repo::Result::Ok((pending, matches))
    };
    let (pending, matches) = match loaded.await {
        Ok(loaded) => loaded,
        Err(e) => {
            warn!("review queue for {}: {e}", event.key);
            return Some(ReviewQueue {
                items: Vec::new(),
                unavailable: true,
                live_href,
            });
        }
    };
    let now = Utc::now();
    Some(ReviewQueue {
        items: pending
            .iter()
            .map(|o| QueueItem {
                id: o.id,
                heading: heading(o, matches.iter().find(|m| m.key == o.match_key), false),
                scout: scout_name(o),
                ago: ago(o.observed_at, now),
                missing_notes: review::missing_notes(&state.season, &o.payload),
                href: detail_href(o.id, &o.event_key),
            })
            .collect(),
        unavailable: false,
        live_href,
    })
}

fn detail_href(id: i64, event_key: &str) -> String {
    format!("/lead-scout/submissions/{id}?event={event_key}")
}

fn queue_href(event_key: &str, reviewed: ReviewState) -> String {
    format!(
        "/lead-scout?event={event_key}&reviewed={}#review",
        reviewed.as_str()
    )
}

/// One observation in full. `None` when there is no such observation, or
/// storage could not say.
pub async fn page(
    state: &AppState,
    nav: Nav,
    id: i64,
    notice: String,
    errors: Vec<String>,
    reason: String,
) -> Option<ReviewPage> {
    let o = match state.repo.observation(id).await {
        Ok(found) => found?,
        Err(e) => {
            warn!("loading observation {id}: {e}");
            return None;
        }
    };
    let record = state.repo.match_by_key(&o.match_key).await.ok().flatten();
    let team_name = state
        .repo
        .team(o.team_number)
        .await
        .ok()
        .flatten()
        .map(|t| t.name)
        .unwrap_or_default();
    let waiting = state
        .repo
        .pending_observations(&o.event_key)
        .await
        .map(|p| p.iter().filter(|p| p.id != id).count())
        .unwrap_or(0);
    let now = Utc::now();

    let reviewer = o
        .reviewer_name
        .clone()
        .unwrap_or_else(|| "a lead scout".into());
    let when = ago(o.reviewed_at, now);
    let verdict = match o.review_state {
        ReviewState::Pending => String::new(),
        ReviewState::Approved => format!("Approved by {reviewer}, {when}."),
        ReviewState::Declined => format!(
            "Declined by {reviewer}, {when}: {}",
            o.review_note.as_deref().unwrap_or("no reason given")
        ),
    };
    let other_version = if o.schema_version == state.season.version {
        String::new()
    } else {
        format!(
            "Recorded on version {} of the form; this is version {}. Answers are shown against the current form.",
            o.schema_version, state.season.version
        )
    };

    Some(ReviewPage {
        title: format!("Review {}", heading(&o, record.as_ref(), true)),
        nav,
        id,
        heading: heading(&o, record.as_ref(), true),
        team_name,
        byline: format!("{}, {}", scout_name(&o), ago(o.observed_at, now)),
        state: o.review_state.as_str(),
        state_label: o.review_state.label(),
        verdict,
        answers: review::answers(&state.season, &o.payload),
        missing_notes: review::missing_notes(&state.season, &o.payload),
        other_version,
        pending: o.review_state == ReviewState::Pending,
        notice,
        errors,
        reason,
        waiting,
        back_href: format!("/lead-scout?event={}#review", o.event_key),
    })
}

/// A verdict that was not recorded, and what to show instead.
pub enum Refused {
    /// No such observation.
    Missing,
    /// Back to the observation, with why and whatever reason was typed.
    Again { errors: Vec<String>, reason: String },
}

/// Record a verdict. `Ok` is where to go next: the oldest observation still
/// waiting when `then=next`, else the queue.
pub async fn decide(
    state: &AppState,
    user: &User,
    id: i64,
    pairs: &[(String, String)],
    decline: bool,
) -> Result<String, Refused> {
    let posted = |name: &str| {
        pairs
            .iter()
            .rev()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    };
    let reason = posted("reason").unwrap_or_default().to_string();

    let o = match state.repo.observation(id).await {
        Ok(Some(o)) => o,
        Ok(None) => return Err(Refused::Missing),
        Err(e) => {
            warn!("loading observation {id}: {e}");
            return Err(Refused::Again {
                errors: vec![
                    "Not recorded: the server's storage did not answer. Try again.".into(),
                ],
                reason,
            });
        }
    };

    let decision = if decline {
        match Decision::decline(&reason) {
            Ok(d) => d,
            Err(tt_core::DomainError::Invalid { value, .. }) => {
                return Err(Refused::Again {
                    errors: vec![format!("Not declined. {value}")],
                    reason,
                });
            }
            Err(e) => {
                return Err(Refused::Again {
                    errors: vec![format!("Not declined. {e}")],
                    reason,
                });
            }
        }
    } else {
        Decision::Approve
    };

    match state
        .repo
        .review_observation(id, &decision, user.id, Utc::now())
        .await
    {
        Ok(true) => {}
        Ok(false) => {
            // Someone else got there first; the page will say what they did.
            return Err(Refused::Again {
                errors: vec!["This was already reviewed, so your verdict was not recorded.".into()],
                reason,
            });
        }
        Err(e) => {
            warn!("reviewing observation {id}: {e}");
            return Err(Refused::Again {
                errors: vec![
                    "Not recorded: the server's storage did not answer. Try again.".into(),
                ],
                reason,
            });
        }
    }
    info!(
        user = %user.email,
        observation = id,
        verdict = decision.state().as_str(),
        "observation reviewed"
    );

    let reviewed = decision.state();
    if posted("then") == Some("next")
        && let Ok(pending) = state.repo.pending_observations(&o.event_key).await
        && let Some(next) = pending.first()
    {
        return Ok(format!(
            "{}&reviewed={}",
            detail_href(next.id, &o.event_key),
            reviewed.as_str()
        ));
    }
    Ok(queue_href(&o.event_key, reviewed))
}

/// What the scouting page tells a scout about their declined records (L10).
pub async fn declined_notices(
    state: &AppState,
    user: &User,
    context: &EventContext,
    matches: &[MatchRecord],
) -> Vec<DeclinedNotice> {
    let Some(event) = &context.selected else {
        return Vec::new();
    };
    let declined = state
        .repo
        .declined_for(&event.key, user.id)
        .await
        .unwrap_or_else(|e| {
            warn!("declined observations for {}: {e}", user.id);
            Vec::new()
        });
    declined
        .iter()
        .map(|o| {
            let label = matches
                .iter()
                .find(|m| m.key == o.match_key)
                .map(MatchRecord::label)
                .unwrap_or_else(|| o.match_key.clone());
            DeclinedNotice {
                what: format!("team {} in {label}", o.team_number),
                reason: o.review_note.clone().unwrap_or_default(),
                reviewer: o
                    .reviewer_name
                    .clone()
                    .unwrap_or_else(|| "The lead scout".into()),
                href: scout_href(&o.event_key, &o.match_key, Some(o.team_number)),
            }
        })
        .collect()
}
