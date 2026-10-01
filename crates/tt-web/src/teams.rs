//! The team profile page (U11): `/teams?event=…&team=…`.
//!
//! Everything comes from storage. The retired page ran a blocking FIRST sync
//! when a team had no local events, so a page render could wait many seconds on
//! a gymnasium's internet (REBUILD_SPEC.md 12.7). A team the server does not
//! know yet is said so, and the background sync brings it in.
//!
//! The page itself is `tt_pages::teams`, which the service worker also runs
//! over the device's copy when the server cannot be reached (C5).

use std::collections::HashMap;
use std::convert::Infallible;

use axum::extract::{FromRequestParts, Query};
use axum::http::request::Parts;
use chrono::Utc;
use tt_core::user::User;
use tt_templates::{Nav, TeamPage};

use crate::events::EventContext;
use crate::startup::AppState;

/// `?team=`, as typed. Never rejects.
#[derive(Debug, Default)]
pub struct TeamParam(pub Option<String>);

impl<S: Send + Sync> FromRequestParts<S> for TeamParam {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Infallible> {
        let raw = Query::<HashMap<String, String>>::from_request_parts(parts, state)
            .await
            .ok()
            .and_then(|Query(mut q)| q.remove("team"));
        Ok(Self(tt_pages::teams::requested(raw.as_deref())))
    }
}

pub async fn page(
    state: &AppState,
    nav: Nav,
    viewer: &User,
    context: &EventContext,
    requested: &TeamParam,
) -> TeamPage {
    tt_pages::teams::page(
        &*state.repo,
        &state.season,
        nav,
        viewer.team_number,
        context,
        requested.0.as_deref(),
        Utc::now(),
    )
    .await
}
