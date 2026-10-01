//! Pages built from a [`LocalRepo`], by the same code on both sides (C5).
//!
//! `tt-web` calls these behind axum, over the Pi's database. `tt-client` calls
//! them behind the service worker, over the device's copy, when the server
//! cannot be reached. One function, one template, so the page a scout sees
//! offline is the page the server would have sent from the same rows.
//!
//! What lives here is only what a page is made of: the rows it reads and how
//! it turns them into a view model. Who may see it (the session, the role
//! guards) and how the request arrives (extractors, the worker's fetch event)
//! stay with each side. A page moves here when it is wanted offline (C6), one
//! at a time; the rest stay in `tt-web`.
//!
//! [`LocalRepo`]: tt_repo::LocalRepo

pub mod events;
pub mod teams;

/// `/notes` for `event_key`, filtered to `team` (U22).
pub fn notes_href(event_key: &str, team: i32) -> String {
    format!("/notes?event={event_key}&team={team}")
}

/// `/graph` for `event_key`, showing `team` (U21).
pub fn graph_href(event_key: &str, team: i32) -> String {
    format!(
        "/graph?event={event_key}&chosen=1&team={team}&metric={}",
        tt_core::graph::POINTS
    )
}
