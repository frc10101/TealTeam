//! The home page (U3): `/?event=…`. Who is signed in, and the selected
//! event's card.
//!
//! The device makes it too (C6), for the team its copy was cut for. Made
//! there, it says whose copy it is and links to the pages the device can
//! make, rather than offering a sign-in the device cannot do.

use tt_core::season::SeasonSchema;
use tt_repo::LocalRepo;
use tt_templates::{HomePage, Nav};

use crate::events::{self, EventContext};

pub async fn page<R: LocalRepo>(
    repo: &R,
    season: &SeasonSchema,
    nav: Nav,
    viewer_team: Option<i32>,
    context: &EventContext,
) -> HomePage {
    HomePage {
        title: "Home".into(),
        nav,
        team_display: viewer_team.map(|n| n.to_string()).unwrap_or_default(),
        season_name: season.name.clone(),
        season_year: season.season,
        event: events::panel(repo, context, viewer_team).await,
    }
}
