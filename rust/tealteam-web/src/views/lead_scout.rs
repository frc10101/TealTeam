//! Lead scout panel: submission review queue, team rankings, pick list, and
//! the point weight settings form.
//!
//! The rankings table is the one piece with real presentation logic: it joins
//! the event roster with scouting point totals and sorts by whichever column
//! the lead scout clicked ([`TeamPointSummary::build`]), falling back to
//! qualification rank.
//!
//! Submissions are flagged rather than validated — a submission with no notes
//! still counts, it is just marked so the reviewer looks at it first.

use askama::Template;
use std::collections::HashMap;

use super::Nav;
use crate::models::scouting::{PendingSubmission, SubmissionDetailRow};
use crate::models::scouting_points::ScoutingPointSection;

// ── View models ───────────────────────────────────────────────────────────

/// One row of the review queue.
pub struct PendingSubmissionRow {
    pub id: i32,
    pub event_name: String,
    pub team_number: i32,
    pub team_name: String,
    pub scout_name: String,
    pub flag_label: String,
    pub flag_class: String,
}

impl PendingSubmissionRow {
    /// Presents a queued submission, flagging empty notes.
    pub fn from_model(row: PendingSubmission) -> Self {
        let missing = row.notes.as_deref().map(|n| n.trim().is_empty()).unwrap_or(true);
        Self {
            id: row.id,
            event_name: row.event_name,
            team_number: row.team_number,
            team_name: row.team_name,
            scout_name: scout_name_or_unknown(row.scout_name),
            flag_label: flag_label(missing),
            flag_class: flag_class(missing),
        }
    }
}

/// One selectable team on the pick list panel.
pub struct PickListTeamRow {
    pub team_number: i32,
    pub team_name: String,
    pub rank: Option<i32>,
    pub rank_display: String,
}

impl PickListTeamRow {
    /// Built from the event roster rows `(team_id, team_number, name, rank)`.
    pub fn from_roster(roster: Vec<(i32, i32, String, Option<i32>)>) -> Vec<Self> {
        roster
            .into_iter()
            .map(|(_, team_number, team_name, rank)| Self {
                team_number,
                team_name,
                rank,
                rank_display: rank.map(|r| format!("#{r}")).unwrap_or_default(),
            })
            .collect()
    }
}

/// One row of the rankings table: a team, its qualification rank, and its
/// scouting point total.
pub struct TeamPointSummary {
    pub team_number: i32,
    pub team_name: String,
    pub rank: Option<i32>,
    pub rank_display: String,
    pub points: i32,
    pub matches: i32,
}

impl TeamPointSummary {
    /// Joins the event roster with scouting point totals and applies the
    /// requested column sort (defaults to qualification rank).
    pub fn build(
        roster: Vec<(i32, i32, String, Option<i32>)>,
        points_by_team: &HashMap<i32, i32>,
        matches_by_team: &HashMap<i32, i32>,
        sort_key: &str,
    ) -> Vec<Self> {
        let mut summaries: Vec<Self> = roster
            .into_iter()
            .map(|(team_id, team_number, team_name, rank)| Self {
                team_number,
                team_name,
                rank,
                rank_display: rank_display(rank),
                points: points_by_team.get(&team_id).copied().unwrap_or(0),
                matches: matches_by_team.get(&team_id).copied().unwrap_or(0),
            })
            .collect();

        let sort_key = {
            let k = sort_key.trim().to_lowercase();
            if k.is_empty() { "rank".to_string() } else { k }
        };

        summaries.sort_by(|left, right| {
            use std::cmp::Ordering;
            let primary = match sort_key.as_str() {
                "points" => right.points.cmp(&left.points),
                "name" => left
                    .team_name
                    .trim()
                    .to_lowercase()
                    .cmp(&right.team_name.trim().to_lowercase()),
                "number" => left.team_number.cmp(&right.team_number),
                _ => match (left.rank, right.rank) {
                    (Some(a), Some(b)) => a.cmp(&b),
                    (Some(_), None) => Ordering::Less,
                    (None, Some(_)) => Ordering::Greater,
                    (None, None) => Ordering::Equal,
                },
            };
            primary
                .then_with(|| left.team_number.cmp(&right.team_number))
                .then_with(|| left.team_name.cmp(&right.team_name))
        });

        summaries
    }
}

/// A single submission shown in full for review.
pub struct SubmissionDetail {
    pub id: i32,
    pub event_name: String,
    pub team_number: i32,
    pub team_name: String,
    pub scout_name: String,
    pub alliance_color: String,
    pub notes: String,
    pub starting_position: String,
    pub defense_rating: String,
    pub traversal: String,
    pub scoring_strategy: String,
    pub shooting_speed: String,
    pub capacity: String,
    pub defendability: String,
    pub hang_level: String,
    pub auto_hang: String,
    pub hang_position: String,
    pub flag_label: String,
    pub flag_class: String,
    pub created_at: String,
}

impl SubmissionDetail {
    /// Presents a submission for the detail page.
    pub fn from_row(row: SubmissionDetailRow) -> Self {
        let missing = row.notes.trim().is_empty();
        Self {
            id: row.id,
            event_name: row.event_name,
            team_number: row.team_number,
            team_name: row.team_name,
            scout_name: scout_name_or_unknown(row.scout_name),
            alliance_color: row.alliance_color,
            notes: row.notes,
            starting_position: row.starting_position,
            defense_rating: row.defense_rating,
            traversal: row.traversal,
            scoring_strategy: row.scoring_strategy,
            shooting_speed: row.shooting_speed,
            capacity: row.capacity,
            defendability: row.defendability,
            hang_level: row.hang_level,
            auto_hang: row.auto_hang,
            hang_position: row.hang_position,
            flag_label: flag_label(missing),
            flag_class: flag_class(missing),
            created_at: row.created_at,
        }
    }
}

// ── Templates ─────────────────────────────────────────────────────────────

/// The lead scout panel.
#[derive(Template)]
#[template(path = "pages/admin_viewer.html")]
pub struct AdminViewerTemplate {
    pub title: String,
    pub description: String,
    pub nav: Nav,
    pub has_event: bool,
    pub pending_submissions: Vec<PendingSubmissionRow>,
    pub team_rankings: Vec<TeamPointSummary>,
    pub pick_list_teams: Vec<PickListTeamRow>,
    pub sort_rank_class: String,
    pub sort_points_class: String,
    pub sort_number_class: String,
    pub sort_name_class: String,
}

impl AdminViewerTemplate {
    /// Assembles the panel and highlights the active sort column.
    pub fn new(
        nav: Nav,
        has_event: bool,
        pending_submissions: Vec<PendingSubmissionRow>,
        team_rankings: Vec<TeamPointSummary>,
        pick_list_teams: Vec<PickListTeamRow>,
        team_sort: &str,
    ) -> Self {
        Self {
            title: "Lead Scout Panel".to_string(),
            description: "Approve submissions, review rankings, and coordinate match strategy."
                .to_string(),
            nav,
            has_event,
            pending_submissions,
            team_rankings,
            pick_list_teams,
            sort_rank_class: sort_class(team_sort, &["", "rank"]),
            sort_points_class: sort_class(team_sort, &["points"]),
            sort_number_class: sort_class(team_sort, &["number"]),
            sort_name_class: sort_class(team_sort, &["name"]),
        }
    }
}

/// One submission in full, with approve/decline actions. `submission` is
/// `None` when the id does not exist — usually because someone already
/// reviewed it.
#[derive(Template)]
#[template(path = "pages/submission_detail.html")]
pub struct SubmissionDetailTemplate {
    pub title: String,
    pub nav: Nav,
    pub submission: Option<SubmissionDetail>,
}

/// The point weight settings form.
#[derive(Template)]
#[template(path = "pages/lead_scout_weights.html")]
pub struct WeightsTemplate {
    pub title: String,
    pub description: String,
    pub nav: Nav,
    pub sections: Vec<ScoutingPointSection>,
    pub updated: bool,
    pub error: String,
}

impl WeightsTemplate {
    /// The weights form, optionally with a saved/failed notice.
    pub fn new(nav: Nav, sections: Vec<ScoutingPointSection>, updated: bool, error: String) -> Self {
        Self {
            title: "Point Weight Settings".to_string(),
            description: "Adjust category point values used for team ranking scores.".to_string(),
            nav,
            sections,
            updated,
            error,
        }
    }
}

// ── Formatting helpers ────────────────────────────────────────────────────

/// Highlights the active sort column's header link.
fn sort_class(active: &str, keys: &[&str]) -> String {
    if keys.contains(&active) {
        "text-teal-300".to_string()
    } else {
        "text-gray-400 hover:text-gray-300".to_string()
    }
}

/// `#4`, or `-` for a team with no rank yet.
fn rank_display(rank: Option<i32>) -> String {
    rank.map(|r| format!("#{r}")).unwrap_or_else(|| "-".to_string())
}

/// Scout name, or "Unknown" for a submission whose scouter was deleted.
fn scout_name_or_unknown(name: Option<String>) -> String {
    name.filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "Unknown".to_string())
}

/// Review flag text: submissions with no notes are worth a closer look.
fn flag_label(missing_notes: bool) -> String {
    if missing_notes { "Missing notes" } else { "Clean" }.to_string()
}

fn flag_class(missing_notes: bool) -> String {
    if missing_notes { "text-yellow-300" } else { "text-teal-300" }.to_string()
}
