//! Live match schedule fragment on the home page.
//!
//! The panel is deliberately narrow: it shows only matches starting within
//! [`WINDOW_MINUTES`] either side of now, so the screen in the pit answers
//! "what is happening right now" rather than listing the whole day.
//!
//! Two sources feed it. [`MatchDisplay::from_schedule`] presents a live FIRST
//! schedule; [`MatchDisplay::from_cached`] presents the `matches` rows synced
//! from The Blue Alliance, used when FIRST is unreachable — the cached form
//! has times but no line-ups, which is why its team lists are empty.

use askama::Template;
use chrono::{DateTime, Local, Utc};
use tracing::warn;

use crate::services::first_api::ScheduleMatch;

/// Matches are only shown within this many minutes either side of now.
const WINDOW_MINUTES: i64 = 15;

/// One match card: when it starts, how soon, and who is in it.
pub struct MatchDisplay {
    pub description: String,
    pub time_display: String,
    pub time_status: String,
    pub minutes_until: i64,
    pub red_teams: Vec<i32>,
    pub blue_teams: Vec<i32>,
}

impl MatchDisplay {
    /// Live schedule from the FIRST API, filtered to the current window and
    /// sorted by start time.
    pub fn from_schedule(raw_matches: &[ScheduleMatch]) -> Vec<Self> {
        let now = Local::now();
        let window_start = now - chrono::Duration::minutes(WINDOW_MINUTES);
        let window_end = now + chrono::Duration::minutes(WINDOW_MINUTES);

        let mut display: Vec<(DateTime<Local>, Self)> = Vec::new();
        for m in raw_matches {
            let Ok(start_time) = DateTime::parse_from_rfc3339(&m.start_time) else {
                warn!(
                    "failed to parse match start time: match {} time {}",
                    m.match_number, m.start_time
                );
                continue;
            };
            let start_time = start_time.with_timezone(&Local);
            if start_time < window_start || start_time > window_end {
                continue;
            }

            let mut red = Vec::new();
            let mut blue = Vec::new();
            for mt in &m.teams {
                if mt.station.starts_with("Red") {
                    red.push(mt.team_number);
                } else if mt.station.starts_with("Blue") {
                    blue.push(mt.team_number);
                }
            }

            let minutes_until = (start_time - now).num_minutes();
            display.push((
                start_time,
                Self {
                    description: m.description.clone(),
                    time_display: start_time.format("%a %b %-d, %-I:%M %p").to_string(),
                    time_status: time_status(minutes_until).to_string(),
                    minutes_until,
                    red_teams: red,
                    blue_teams: blue,
                },
            ));
        }

        display.sort_by_key(|(t, _)| *t);
        display.into_iter().map(|(_, m)| m).collect()
    }

    /// Cached schedule from the local DB, used when FIRST is unreachable.
    pub fn from_cached(rows: Vec<(i32, Option<DateTime<Utc>>)>) -> Vec<Self> {
        let now = Utc::now();
        rows.into_iter()
            .map(|(match_number, scheduled_time)| {
                let mut item = Self {
                    description: format!("Match {match_number}"),
                    time_display: String::new(),
                    time_status: "upcoming".to_string(),
                    minutes_until: 0,
                    red_teams: Vec::new(),
                    blue_teams: Vec::new(),
                };
                if let Some(scheduled) = scheduled_time {
                    item.time_display = scheduled
                        .with_timezone(&Local)
                        .format("%a %b %-d, %-I:%M %p")
                        .to_string();
                    let minutes_until = (scheduled - now).num_minutes();
                    item.minutes_until = minutes_until;
                    item.time_status = time_status(minutes_until).to_string();
                }
                item
            })
            .collect()
    }
}

/// Bucket a match into `past` / `current` / `upcoming`, which the template
/// turns into colour.
fn time_status(minutes_until: i64) -> &'static str {
    if minutes_until < -WINDOW_MINUTES {
        "past"
    } else if minutes_until <= 5 {
        "current"
    } else {
        "upcoming"
    }
}

/// The schedule panel: a status line, a list of matches, or both.
#[derive(Template)]
#[template(path = "partials/match_schedule.html")]
pub struct MatchScheduleFragment {
    pub message: String,
    pub matches: Vec<MatchDisplay>,
}

impl MatchScheduleFragment {
    /// A schedule panel showing only a status line.
    pub fn message(message: &str) -> Self {
        Self {
            message: message.to_string(),
            matches: Vec::new(),
        }
    }

    /// Matches, optionally under a note explaining where they came from
    /// (cached data, offline mode).
    pub fn with_matches(message: &str, matches: Vec<MatchDisplay>) -> Self {
        Self {
            message: message.to_string(),
            matches,
        }
    }
}
