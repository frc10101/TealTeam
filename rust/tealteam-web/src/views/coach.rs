//! Drive coach panel: the team's own matches with alliance partners and their
//! OPR/DPR, plus a status summary.
//!
//! Unlike the home-page schedule this shows the team's whole day, tagged by
//! status so the current match stands out: completed once it is more than 15
//! minutes past, "Current Match" within 15 minutes either side, upcoming after
//! that. Each card also names the coach's alliance and partners, which is the
//! question actually being asked between matches.
//!
//! FIRST returns schedule times without an offset, so [`parse_match_time`]
//! interprets them in the event's timezone when one is known.

use askama::Template;
use chrono::{DateTime, FixedOffset, Local, Utc};
use chrono_tz::Tz;
use std::collections::HashMap;
use tracing::error;

use super::Nav;
use crate::services::first_api::ScheduleMatch;

/// One robot on a match card, with its synced strength estimates.
pub struct DriveCoachTeam {
    pub team_number: i32,
    pub opr: Option<f64>,
    pub dpr: Option<f64>,
}

impl DriveCoachTeam {
    /// OPR to one decimal, `0.0` before stats are synced.
    pub fn opr_display(&self) -> String {
        self.opr.map(|v| format!("{v:.1}")).unwrap_or_else(|| "0.0".to_string())
    }
    /// DPR to one decimal, `0.0` before stats are synced.
    pub fn dpr_display(&self) -> String {
        self.dpr.map(|v| format!("{v:.1}")).unwrap_or_else(|| "0.0".to_string())
    }
}

/// One match card: both alliances, the viewer's side, and status styling.
pub struct DriveCoachMatch {
    pub description: String,
    pub match_number: i32,
    pub time_display: String,
    pub status_label: String,
    pub status_class: String,
    pub badge_class: String,
    pub red_teams: Vec<DriveCoachTeam>,
    pub blue_teams: Vec<DriveCoachTeam>,
    pub our_alliance: String,
    pub our_partners: Vec<i32>,
    pub start_time_sort: i64,
}

/// The counts above the schedule. `present` is false until a schedule loads,
/// which hides the strip rather than showing zeros.
#[derive(Default)]
pub struct DriveCoachSummary {
    pub present: bool,
    pub event_name: String,
    pub team_number: i32,
    pub current_matches: i32,
    pub upcoming_matches: i32,
    pub completed_matches: i32,
}

/// Turns a FIRST schedule into match cards ordered by start time, tagging each
/// with its status and the viewer's alliance.
pub fn build_schedule(
    raw_matches: &[ScheduleMatch],
    stats_by_team: &HashMap<i32, (Option<f64>, Option<f64>)>,
    event_tz: Option<Tz>,
    event_name: String,
    user_team_number: i32,
) -> (Vec<DriveCoachMatch>, DriveCoachSummary) {
    let now = Utc::now();
    let mut results = Vec::new();
    let (mut current, mut upcoming, mut completed) = (0, 0, 0);

    for m in raw_matches {
        let start_time = parse_match_time(&m.start_time, event_tz);

        let mut entry = DriveCoachMatch {
            description: m.description.clone(),
            match_number: m.match_number,
            time_display: String::new(),
            status_label: "Upcoming".to_string(),
            status_class: "border-teal-600 bg-teal-900/20".to_string(),
            badge_class: "bg-teal-700 text-white".to_string(),
            red_teams: Vec::new(),
            blue_teams: Vec::new(),
            our_alliance: String::new(),
            our_partners: Vec::new(),
            start_time_sort: i64::MAX,
        };

        if let Some(start) = start_time {
            entry.time_display = start.with_timezone(&Local).format("%a %b %-d, %-I:%M %p").to_string();
            entry.start_time_sort = start.timestamp();
            let minutes_until = (start.with_timezone(&Utc) - now).num_minutes();
            if minutes_until < -15 {
                entry.status_label = "Completed".to_string();
                entry.status_class = "border-gray-700 bg-gray-900/40".to_string();
                entry.badge_class = "bg-gray-700 text-gray-200".to_string();
                completed += 1;
            } else if minutes_until <= 15 {
                entry.status_label = "Current Match".to_string();
                entry.status_class = "border-yellow-500 bg-yellow-900/20 ring-2 ring-yellow-500/40".to_string();
                entry.badge_class = "bg-yellow-600 text-white".to_string();
                current += 1;
            } else {
                upcoming += 1;
            }
        } else {
            error!("failed to parse match start time: match {} raw {}", m.match_number, m.start_time);
        }

        for mt in &m.teams {
            let (opr, dpr) = stats_by_team.get(&mt.team_number).copied().unwrap_or((None, None));
            let team = DriveCoachTeam { team_number: mt.team_number, opr, dpr };
            if mt.station.starts_with("Red") {
                entry.red_teams.push(team);
                if mt.team_number == user_team_number {
                    entry.our_alliance = "Red".to_string();
                }
            } else if mt.station.starts_with("Blue") {
                entry.blue_teams.push(team);
                if mt.team_number == user_team_number {
                    entry.our_alliance = "Blue".to_string();
                }
            }
        }

        let our_teams = match entry.our_alliance.as_str() {
            "Red" => Some(&entry.red_teams),
            "Blue" => Some(&entry.blue_teams),
            _ => None,
        };
        if let Some(our_teams) = our_teams {
            entry.our_partners = our_teams
                .iter()
                .filter(|t| t.team_number != user_team_number)
                .map(|t| t.team_number)
                .collect();
        }

        results.push(entry);
    }

    results.sort_by_key(|m| m.start_time_sort);

    let summary = DriveCoachSummary {
        present: true,
        event_name,
        team_number: user_team_number,
        current_matches: current,
        upcoming_matches: upcoming,
        completed_matches: completed,
    };

    (results, summary)
}

/// Parse a FIRST schedule time. RFC3339 with offset wins; otherwise interpret
/// a naive datetime in the event timezone.
fn parse_match_time(raw: &str, event_tz: Option<Tz>) -> Option<DateTime<FixedOffset>> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(raw) {
        return Some(dt);
    }
    if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S") {
        if let Some(tz) = event_tz {
            use chrono::TimeZone;
            if let chrono::LocalResult::Single(dt) = tz.from_local_datetime(&naive) {
                return Some(dt.fixed_offset());
            }
        }
        // Fall back to local time interpretation.
        use chrono::TimeZone;
        if let chrono::LocalResult::Single(dt) = Local.from_local_datetime(&naive) {
            return Some(dt.fixed_offset());
        }
    }
    None
}

// ── Templates ─────────────────────────────────────────────────────────────

/// The drive coach page.
#[derive(Template)]
#[template(path = "pages/coach_viewer.html")]
pub struct CoachViewerTemplate {
    pub title: String,
    pub description: String,
    pub nav: Nav,
    pub summary: DriveCoachSummary,
    pub event_selection_html: String,
    pub matches_html: String,
}

impl CoachViewerTemplate {
    /// Page shell around an already-rendered schedule.
    pub fn new(
        nav: Nav,
        summary: DriveCoachSummary,
        event_selection_html: String,
        matches_html: String,
    ) -> Self {
        Self {
            title: "Drive Coach Panel".to_string(),
            description: "Quick match schedule and alliance partners.".to_string(),
            nav,
            summary,
            event_selection_html,
            matches_html,
        }
    }
}

/// The match list, re-fetched on a timer so the current match keeps up.
#[derive(Template)]
#[template(path = "partials/drive_coach_matches.html")]
pub struct DriveCoachMatchesFragment {
    pub updated_at: String,
    pub error: String,
    pub info: String,
    pub matches: Vec<DriveCoachMatch>,
}

impl DriveCoachMatchesFragment {
    /// A neutral notice instead of matches.
    pub fn info(updated_at: String, info: &str) -> Self {
        Self {
            updated_at,
            error: String::new(),
            info: info.to_string(),
            matches: Vec::new(),
        }
    }

    /// A failure notice: no schedule, and why.
    pub fn error(updated_at: String, error: String) -> Self {
        Self {
            updated_at,
            error,
            info: String::new(),
            matches: Vec::new(),
        }
    }

    /// The loaded schedule, or a notice when the team has no matches yet.
    pub fn matches(updated_at: String, matches: Vec<DriveCoachMatch>) -> Self {
        let info = if matches.is_empty() {
            "No matches were returned for your team at the selected event yet.".to_string()
        } else {
            String::new()
        };
        Self {
            updated_at,
            error: String::new(),
            info,
            matches,
        }
    }
}

/// Shown on both the page and the fragment when no event is selected yet.
pub const NO_EVENT_INFO: &str = "Select an event above to load your team schedule.";
