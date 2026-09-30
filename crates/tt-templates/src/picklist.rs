//! The pick list page (U20): the team's list for the selected event, best
//! first, with every change a small form of its own.

use askama::Template;
use tt_core::picklist::Tag;

use crate::Nav;

#[derive(Template)]
#[template(path = "pages/pick_list.html")]
pub struct PickListPage {
    pub title: String,
    pub nav: Nav,
    pub event_name: String,
    /// Why there is no list, when there is not.
    pub unavailable: String,
    pub errors: Vec<String>,
    /// What the change that led here did.
    pub notice: String,
    pub rows: Vec<PickRow>,
    /// How many of `rows` are crossed off.
    pub crossed: usize,
    /// The event's teams not on the list yet, best ranked first.
    pub candidates: Vec<Candidate>,
    /// What is in the add box: empty, or what a refused add sent.
    pub typed_team: String,
    pub tags: Vec<TagOption>,
    /// Where every change posts.
    pub post_href: String,
}

#[derive(Debug, Clone)]
pub struct PickRow {
    /// 1 for the top of the list.
    pub place: usize,
    pub team: i32,
    pub name: String,
    /// `"Rank 3"`, or empty when the event has no ranking for the team.
    pub rank: String,
    /// The tag's key, for its swatch; empty when untagged.
    pub tag: String,
    pub tag_label: String,
    pub crossed: bool,
    pub first: bool,
    pub last: bool,
    pub profile_href: String,
}

#[derive(Debug, Clone)]
pub struct Candidate {
    pub team: i32,
    pub name: String,
    pub rank: String,
}

#[derive(Debug, Clone)]
pub struct TagOption {
    pub key: &'static str,
    pub label: &'static str,
}

impl TagOption {
    pub fn all() -> Vec<TagOption> {
        Tag::ALL
            .into_iter()
            .map(|t| TagOption {
                key: t.key(),
                label: t.label(),
            })
            .collect()
    }
}
