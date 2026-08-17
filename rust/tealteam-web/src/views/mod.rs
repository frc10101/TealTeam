//! View layer: the Askama template structs, the view models they render, and
//! the presentation logic that turns model data into display strings.
//!
//! Views never touch the database — controllers load data and hand it over.
//! What belongs here is anything a designer would recognise as a decision:
//! date and number formatting, CSS class selection, sort order of a table,
//! which of several messages to show.
//!
//! # Templates
//!
//! Markup lives in `templates/`, configured by `askama.toml`, and is compiled
//! into the binary: a malformed template or a field a template references but
//! the struct does not have is a build error, not a runtime 500. Pages extend
//! `templates/layout.html`; fragments in `templates/partials/` are rendered
//! standalone and swapped in by Unpoly.
//!
//! # Pages and fragments
//!
//! A page struct is rendered with [`render`]. A fragment that is embedded in a
//! page *and* served on its own — the event picker, the submission panel, the
//! assignment table — is rendered to a string with [`render_html`] and passed
//! into the page struct as a `_html` field, so the same partial produces the
//! same markup on first paint and on every later swap.
//!
//! # Escaping
//!
//! Askama HTML-escapes interpolations by default. The `_html` fields above are
//! the exception: they are marked `|safe` in the templates because they hold
//! markup this crate rendered. Anything user-supplied that bypasses a template
//! (a handful of small error fragments built with `format!`) is escaped
//! explicitly with `html_escape`.

pub mod assignments;
pub mod auth;
pub mod coach;
pub mod db_viewer;
pub mod events;
pub mod home;
pub mod lead_scout;
pub mod matches;
pub mod network;
pub mod submission;
pub mod teams;

use askama::Template;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};

use crate::models::User;

/// Renders a page template into an HTML response.
///
/// A rendering failure logs and returns a 500; because templates are checked
/// at compile time, that generally means a formatting error rather than a
/// missing field.
pub fn render<T: Template>(t: &T) -> Response {
    match t.render() {
        Ok(html) => Html(html).into_response(),
        Err(e) => {
            tracing::error!("template render failed: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, format!("template error: {e}")).into_response()
        }
    }
}

/// Renders a fragment to a string for embedding in a parent template.
///
/// Failures degrade to an empty string rather than blanking the whole page —
/// the surrounding page still renders without that panel.
pub fn render_html<T: Template>(t: &T) -> String {
    t.render().unwrap_or_else(|e| {
        tracing::error!("template render failed: {e}");
        String::new()
    })
}

// ── Nav (layout chrome shared by every page) ──────────────────────────────

/// Which navigation links the layout shows.
///
/// Derived from the signed-in user, so the chrome matches what the controllers
/// will actually let them do. Hiding a link is cosmetic: every protected
/// action re-checks the session server-side.
#[derive(Debug, Clone, Default)]
pub struct Nav {
    pub signed_in: bool,
    pub is_admin: bool,
    pub is_lead_scout: bool,
    pub is_coach: bool,
}

impl Nav {
    /// Nav for the current viewer; the default (nothing but public links) for
    /// a signed-out one.
    pub fn from_user(user: Option<&User>) -> Self {
        match user {
            Some(u) => Self {
                signed_in: true,
                is_admin: u.is_admin,
                is_lead_scout: u.is_lead_scout,
                is_coach: u.is_coach,
            },
            None => Self::default(),
        }
    }

    /// Show the lead scout links.
    pub fn show_lead(&self) -> bool {
        self.signed_in && (self.is_admin || self.is_lead_scout)
    }

    /// Show the drive coach link.
    pub fn show_coach(&self) -> bool {
        self.signed_in && (self.is_admin || self.is_coach)
    }
}
