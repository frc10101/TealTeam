//! Team lookup page: the team identity card, its events, and the per-event
//! stats and scouting summary.
//!
//! [`TeamDataView::build`] is where scouting rows become a profile. Structured
//! fields are summarised by taking the most common value across every scouted
//! match (one bad observation does not move the picture), while the free-text
//! ratings shown as "latest" come from the newest row.
//!
//! Notes are filtered by team: only rows whose `submitting_team_id` matches
//! the viewer's team are shown, so scouting commentary stays private to the
//! team that wrote it while the structured summary is shared.

use askama::Template;
use std::collections::HashMap;

use super::Nav;
use crate::models::event::EventOption;
use crate::models::{ScoutingData, Team, TeamEventStats};

// ── View models ───────────────────────────────────────────────────────────

/// A team's identity card.
pub struct TeamView {
    pub team_number: i32,
    pub name: String,
    pub school_name: String,
    pub city_line: String,
    pub rookie_year: String,
    pub nickname: String,
    pub website: String,
    pub motto: String,
}

impl TeamView {
    /// Presents a team record, blanking out whatever FIRST did not provide.
    pub fn from_team(team: &Team) -> Self {
        Self {
            team_number: team.team_number,
            name: team.name.clone(),
            school_name: team.school_name.clone().unwrap_or_default(),
            city_line: city_line(team),
            rookie_year: team.rookie_year.map(|y| y.to_string()).unwrap_or_default(),
            nickname: team.nickname.clone().unwrap_or_default(),
            website: team.website.clone().unwrap_or_default(),
            motto: team.motto.clone().unwrap_or_default(),
        }
    }
}

/// "City, State • Country", skipping whichever parts are missing.
fn city_line(team: &Team) -> String {
    let mut line = team
        .city
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or_default()
        .to_string();

    if let Some(state) = team.state.as_deref().filter(|s| !s.is_empty()) {
        line = if line.is_empty() {
            state.to_string()
        } else {
            format!("{line}, {state}")
        };
    }
    if let Some(country) = team.country.as_deref().filter(|s| !s.is_empty()) {
        line = if line.is_empty() {
            country.to_string()
        } else {
            format!("{line} • {country}")
        };
    }
    line
}

/// One scouting note, shown only to the team that wrote it.
pub struct NoteEntry {
    pub note: String,
    pub scouted_display: String,
    pub match_index: usize,
}

/// Pre-formatted stats for display (empty string = value absent).
#[derive(Default)]
pub struct StatsView {
    pub present: bool,
    pub rank: String,
    pub record: String,
    pub matches: String,
    pub qual_average: String,
    pub avg_match_points: String,
    pub dq_count: String,
    pub opr: String,
    pub dpr: String,
    pub ccwm: String,
    pub auto_opr: String,
    pub teleop_opr: String,
    pub endgame_opr: String,
    pub qual_points: String,
    pub elim_points: String,
    pub award_points: String,
    pub alliance_points: String,
}

impl StatsView {
    /// Formats synced stats for display: OPR-style values to two decimals,
    /// component OPRs to one, and zero-valued counters blanked rather than
    /// shown as `0`.
    pub fn from_stats(s: TeamEventStats) -> Self {
        let f1 = |v: Option<f64>| v.map(|x| format!("{x:.1}")).unwrap_or_default();
        let f2 = |v: Option<f64>| v.map(|x| format!("{x:.2}")).unwrap_or_default();
        let int = |v: Option<i32>| v.map(|x| x.to_string()).unwrap_or_default();

        let matches_played = s.matches_played.unwrap_or(0);
        let record = {
            let w = s.wins.unwrap_or(0);
            let l = s.losses.unwrap_or(0);
            let t = s.ties.unwrap_or(0);
            if t > 0 {
                format!("{w}W {l}L {t}T")
            } else {
                format!("{w}W {l}L")
            }
        };
        let dq = s.dq_count.unwrap_or(0);

        Self {
            present: true,
            rank: int(s.rank),
            record,
            matches: if matches_played > 0 { matches_played.to_string() } else { String::new() },
            qual_average: f1(s.qual_average),
            avg_match_points: f2(s.avg_match_points),
            dq_count: if dq > 0 { dq.to_string() } else { String::new() },
            opr: f2(s.opr),
            dpr: f2(s.dpr),
            ccwm: f2(s.ccwm),
            auto_opr: f1(s.auto_opr),
            teleop_opr: f1(s.teleop_opr),
            endgame_opr: f1(s.endgame_opr),
            qual_points: int(s.qual_points),
            elim_points: int(s.elim_points),
            award_points: int(s.award_points),
            alliance_points: int(s.alliance_points),
        }
    }
}

/// A team's performance at one event: synced stats plus a summary of what
/// scouts observed.
#[derive(Default)]
pub struct TeamDataView {
    pub has_data: bool,
    pub event_name: String,
    pub stats: Option<StatsView>,
    pub scouting_count: usize,
    pub most_common_start_pos: String,
    pub most_common_defense: String,
    pub most_common_traversal: String,
    pub most_common_scoring_strategy: String,
    pub most_common_hang_level: String,
    pub most_common_hang_position: String,
    pub most_common_accuracy: String,
    pub shooting_speed: String,
    pub capacity: String,
    pub defendability: String,
    pub scoring_strategy: String,
    pub notes: Vec<NoteEntry>,
    pub recent_alliances: Vec<String>,
}

impl TeamDataView {
    /// Summarises a team's approved scouting rows at one event. Notes are only
    /// included when they came from the viewer's own team.
    pub fn build(
        event_name: String,
        stats: Option<TeamEventStats>,
        scouting_data: Vec<ScoutingData>,
        viewer_team_id: i32,
    ) -> Self {
        let mut v = TeamDataView {
            event_name,
            stats: stats.map(StatsView::from_stats),
            scouting_count: scouting_data.len(),
            ..Default::default()
        };

        if scouting_data.is_empty() {
            v.has_data = v.stats.is_some();
            return v;
        }
        v.has_data = true;

        let most_common = |f: &dyn Fn(&ScoutingData) -> Option<String>| -> String {
            let mut counts: HashMap<String, i32> = HashMap::new();
            for sd in &scouting_data {
                if let Some(value) = f(sd).filter(|s| !s.is_empty()) {
                    *counts.entry(value).or_insert(0) += 1;
                }
            }
            counts.into_iter().max_by_key(|(_, c)| *c).map(|(v, _)| v).unwrap_or_default()
        };

        v.most_common_start_pos = most_common(&|sd| sd.starting_position.clone());
        v.most_common_defense = most_common(&|sd| sd.defense_rating.clone());
        v.most_common_traversal = most_common(&|sd| sd.traversal.clone());
        v.most_common_scoring_strategy = most_common(&|sd| sd.scoring_strategy.clone());
        v.most_common_hang_level = most_common(&|sd| sd.hang_level.clone());
        v.most_common_hang_position = most_common(&|sd| sd.hang_position.clone());
        v.most_common_accuracy = most_common(&|sd| sd.accuracy_rating.clone());

        let latest = &scouting_data[0];
        v.shooting_speed = latest.shooting_speed.clone().unwrap_or_default();
        v.capacity = latest.capacity.clone().unwrap_or_default();
        v.defendability = latest.defendability.clone().unwrap_or_default();
        v.scoring_strategy = latest.scoring_strategy.clone().unwrap_or_default();

        for (i, sd) in scouting_data.iter().enumerate() {
            if let Some(notes) = sd.notes.as_deref().filter(|n| !n.is_empty()) {
                if viewer_team_id > 0 && sd.submitting_team_id == Some(viewer_team_id) {
                    v.notes.push(NoteEntry {
                        note: notes.to_string(),
                        scouted_display: sd
                            .scouted_at
                            .map(|t| t.with_timezone(&chrono::Local).format("%b %-d %-I:%M %p").to_string())
                            .unwrap_or_default(),
                        match_index: i + 1,
                    });
                }
            }
        }

        v.recent_alliances = scouting_data.iter().take(5).map(|sd| sd.alliance_color.clone()).collect();

        v
    }
}

// ── Templates ─────────────────────────────────────────────────────────────

/// The team lookup page: a search box and, once a team is found, its card.
#[derive(Template)]
#[template(path = "pages/team.html")]
pub struct TeamPageTemplate {
    pub title: String,
    pub nav: Nav,
    pub team_search_value: String,
    pub team_error: String,
    pub team_info_html: String,
}

/// A team's card and event picker, swapped in on search.
#[derive(Template)]
#[template(path = "partials/team_info.html")]
pub struct TeamInfoFragment {
    pub signed_in: bool,
    pub team: TeamView,
    pub events: Vec<EventOption>,
    pub selected_event_id: Option<i32>,
}

/// The stats and scouting summary for one team at one event.
#[derive(Template)]
#[template(path = "partials/team_data.html")]
pub struct TeamDataFragment {
    pub v: TeamDataView,
}

/// Error card swapped into the team-info slot when a lookup fails.
pub fn team_info_error_html(message: &str) -> String {
    format!(
        r#"<div id="team-info-container"><div class="card"><div class="card-body text-center text-red-400 py-8">{}</div></div></div>"#,
        html_escape::encode_text(message)
    )
}
