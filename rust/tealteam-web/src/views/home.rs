//! Home and Help pages.
//!
//! The home page is mostly a shell around the event picker fragment
//! ([`super::events`]) plus the live match schedule, which loads itself.

use askama::Template;

use super::Nav;

/// The home page.
#[derive(Template)]
#[template(path = "pages/index.html")]
pub struct IndexTemplate {
    pub title: String,
    pub nav: Nav,
    pub message: String,
    pub event_selection_html: String,
}

/// Static help page: how to scout, what the fields mean.
#[derive(Template)]
#[template(path = "pages/help.html")]
pub struct HelpTemplate {
    pub title: String,
    pub nav: Nav,
}
