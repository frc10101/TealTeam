//! The scouting assistant (AI1): a question, answered from the data sheet,
//! with the sheet underneath so the answer can be checked.

use askama::Template;

use crate::Nav;

#[derive(Template)]
#[template(path = "pages/ask.html")]
pub struct AskPage {
    pub title: String,
    pub nav: Nav,
    pub event_name: String,
    /// Why nothing can be asked: no event, no storage, or no assistant set
    /// up on this server.
    pub unavailable: String,
    /// What answers: `"a local model at http://10.101.0.20:8080"`.
    pub backend: String,
    /// Where the form posts, with the event in it.
    pub post_href: String,
    pub question: String,
    pub answer: String,
    /// Why there is no answer: busy, timed out, unreachable.
    pub error: String,
    /// The sheet the answer was given, verbatim.
    pub sheet: String,
    /// `"Built 10:32 from 41 observations of 12 teams."`
    pub sheet_about: String,
}

impl AskPage {
    pub fn new(nav: Nav) -> Self {
        Self {
            title: "Ask".into(),
            nav,
            event_name: String::new(),
            unavailable: String::new(),
            backend: String::new(),
            post_href: String::new(),
            question: String::new(),
            answer: String::new(),
            error: String::new(),
            sheet: String::new(),
            sheet_about: String::new(),
        }
    }
}
