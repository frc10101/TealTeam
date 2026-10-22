//! The scouting page and the save behind it (U4), behind axum.
//!
//! The page and the check of a post are `tt_pages::scouting`, shared with
//! the service worker, which makes them over the device's copy when the
//! server cannot be reached (OF1). What is left here is the server's own:
//! the query extractor, the tablet a request came from, the scout's declined
//! records and the live stream, and recording a save in the Pi's tables.

use std::collections::HashMap;
use std::convert::Infallible;

use axum::extract::{FromRequestParts, Query};
use axum::http::request::Parts;
use chrono::Utc;
use rand::RngCore;
use tracing::{info, warn};
use tt_core::form::FormErrors;
use tt_core::record_id;
use tt_core::user::User;
use tt_pages::scouting::{Saved, ServerOnly};
use tt_repo::{Recorded, Repo, RepoError};
use tt_templates::{Draft, Nav, SubmissionPage};

pub use tt_pages::scouting::{Rejected, ScoutParams};

use crate::events::EventContext;
use crate::startup::AppState;

/// `?match=`, `&team=`, `&choose=`, and `&saved=` ([`ScoutParams`]), as an
/// extractor. Never rejects, like [`crate::events::EventParam`].
pub struct ScoutQuery(pub ScoutParams);

impl<S: Send + Sync> FromRequestParts<S> for ScoutQuery {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Infallible> {
        let query = Query::<HashMap<String, String>>::from_request_parts(parts, state)
            .await
            .map(|Query(query)| query)
            .unwrap_or_default();
        Ok(Self(ScoutParams::from_lookup(|name| {
            query.get(name).map(String::as_str)
        })))
    }
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

/// Assemble the scouting page for `context`'s event, from the Pi's tables:
/// `tt_pages::scouting::page`, with the declined records and the live stream
/// only the server has.
#[allow(clippy::too_many_arguments)]
pub async fn page(
    state: &AppState,
    user: &User,
    device_id: Option<i64>,
    nav: Nav,
    context: &EventContext,
    params: &ScoutParams,
    draft: Option<Draft>,
    errors: Vec<String>,
) -> SubmissionPage {
    let mut server = ServerOnly::default();
    if let Some(event) = &context.selected {
        let matches = state
            .repo
            .event_matches(&event.key)
            .await
            .unwrap_or_default();
        server.declined =
            tt_pages::scouting::declined_notices(&*state.repo, user, context, &matches).await;
        server.stream = crate::sync::stream_href(state, &event.key).await;
    }
    tt_pages::scouting::page(
        &*state.repo,
        &state.season,
        user,
        device_id,
        nav,
        context,
        params,
        draft,
        errors,
        Saved::OnServer,
        server,
        new_record_id,
    )
    .await
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
    let device_id = device_id(state, device_uuid).await;
    let now = Utc::now();
    let observation = tt_pages::scouting::read_post(
        &*state.repo,
        &state.season,
        user,
        device_id,
        pairs,
        now,
        new_record_id,
    )
    .await?;

    match state.repo.record_observation(&observation, now).await {
        Ok(recorded) => {
            let outcome = match recorded {
                Recorded::Created(_) => "recorded",
                Recorded::Duplicate(_) => "already recorded, repeat ignored",
            };
            info!(
                user = %user.email,
                r#match = %observation.match_key,
                team = observation.team_number,
                "observation {outcome}"
            );
            Ok(tt_pages::scouting::saved_href(&observation))
        }
        Err(RepoError::Conflict { .. }) => Err(tt_pages::scouting::one_per_robot(&observation)),
        Err(e) => {
            warn!("recording observation: {e}");
            let mut rejected = tt_pages::scouting::one_per_robot(&observation);
            rejected.draft = Some(Draft {
                record_id: observation.client_record_id.clone(),
                answers: tt_core::form::RawAnswers::from_pairs(pairs),
                errors: FormErrors::default(),
            });
            rejected.errors =
                vec!["Could not save. Your answers are still here — try again.".into()];
            Err(rejected)
        }
    }
}
