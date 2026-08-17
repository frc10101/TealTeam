//! Per-match assignment grid: one row per match, six robot slots per row,
//! each with a picker listing the team's scouts and the registered devices.
//!
//! [`AssignmentData::build`] is the interesting part. Matches store team
//! *numbers* while assignments reference team *ids*, so building the grid
//! means joining three loaded lists in memory: the schedule, the event roster
//! (number -> id and name) and the existing assignments (match + team id ->
//! assignee). A slot whose team is not on the roster renders as `TBD`, which
//! is normal before elimination alliances are set.
//!
//! The table is re-rendered whole after every mutation rather than patched,
//! which keeps the counts, the online dots and the pickers consistent.

use askama::Template;
use chrono::{DateTime, Utc};
use std::collections::HashMap;

use super::Nav;
use crate::models::assignment::{AssignmentRow, MatchRow};
use crate::models::device::DeviceRow;
use crate::models::team::EventTeamLookup;
use crate::models::user::ScoutRow;

/// One match: its label, time, and both alliances' slots.
pub struct MatchAssignmentRow {
    pub match_id: i32,
    pub match_number: i32,
    pub match_type: String,
    pub played: bool,
    pub scheduled_time: Option<DateTime<Utc>>,
    pub red_slots: Vec<SlotAssignment>,
    pub blue_slots: Vec<SlotAssignment>,
}

impl MatchAssignmentRow {
    /// Short match label: `Q12`, `SF3`, `F1`.
    pub fn label(&self) -> String {
        match self.match_type.as_str() {
            "qm" | "" => format!("Q{}", self.match_number),
            "sf" => format!("SF{}", self.match_number),
            "f" => format!("F{}", self.match_number),
            _ => format!("M{}", self.match_number),
        }
    }

    /// Scheduled start in the server's local time, blank if unscheduled.
    pub fn scheduled_display(&self) -> String {
        self.scheduled_time
            .map(|t| t.with_timezone(&chrono::Local).format("%-I:%M %p").to_string())
            .unwrap_or_default()
    }

    /// Slots with someone assigned, for the "3/6" coverage badge.
    pub fn assigned_count(&self) -> usize {
        self.red_slots.iter().chain(&self.blue_slots).filter(|s| s.assignment_id.is_some()).count()
    }

    /// Slots with a known team — the denominator of that badge.
    pub fn total_count(&self) -> usize {
        self.red_slots.iter().chain(&self.blue_slots).filter(|s| s.team_id.is_some()).count()
    }
}

/// One robot slot and who is scouting it.
#[derive(Clone)]
pub struct SlotAssignment {
    pub position: i32,
    pub team_id: Option<i32>,
    pub team_number: i32,
    pub team_name: String,
    pub assignment_id: Option<i32>,
    pub scouter_id: Option<i32>,
    pub device_id: Option<i32>,
}

impl SlotAssignment {
    /// True when nobody holds this slot.
    pub fn is_unassigned(&self) -> bool {
        self.scouter_id.is_none() && self.device_id.is_none()
    }

    /// Marks the selected option for a scout. Askama passes loop variables by
    /// reference, hence `&i32`.
    pub fn is_scouter(&self, id: &i32) -> bool {
        self.scouter_id.as_ref() == Some(id)
    }

    /// Marks the selected option for a device.
    pub fn is_device(&self, id: &i32) -> bool {
        self.device_id.as_ref() == Some(id)
    }
}

/// The whole grid: matches, the pickers' contents, and an optional notice.
#[derive(Default)]
pub struct AssignmentData {
    pub selected_event_name: String,
    pub match_rows: Vec<MatchAssignmentRow>,
    pub scouts: Vec<ScoutRow>,
    pub devices: Vec<DeviceRow>,
    pub info: String,
}

impl AssignmentData {
    /// Joins matches, the event roster and existing assignments into the grid.
    pub fn build(
        selected_event_name: String,
        matches: Vec<MatchRow>,
        roster: Vec<EventTeamLookup>,
        assignments: Vec<AssignmentRow>,
        scouts: Vec<ScoutRow>,
        devices: Vec<DeviceRow>,
    ) -> Self {
        let team_lookup: HashMap<i32, EventTeamLookup> =
            roster.into_iter().map(|r| (r.team_number, r)).collect();

        let mut by_match: HashMap<i32, HashMap<i32, AssignmentRow>> = HashMap::new();
        for a in assignments {
            by_match.entry(a.match_id).or_default().insert(a.team_id, a);
        }

        let match_rows = matches
            .into_iter()
            .map(|m| {
                let empty = HashMap::new();
                let by_team = by_match.get(&m.id).unwrap_or(&empty);
                MatchAssignmentRow {
                    match_id: m.id,
                    match_number: m.match_number,
                    played: m.played,
                    scheduled_time: m.scheduled_time,
                    red_slots: build_slots(&m.red_slots(), &team_lookup, by_team),
                    blue_slots: build_slots(&m.blue_slots(), &team_lookup, by_team),
                    match_type: m.match_type,
                }
            })
            .collect();

        Self {
            selected_event_name,
            match_rows,
            scouts,
            devices,
            info: String::new(),
        }
    }
}

/// Resolves three `(station, team number)` slots against the roster and the
/// existing assignments.
fn build_slots(
    slots: &[(i32, Option<i32>)],
    team_lookup: &HashMap<i32, EventTeamLookup>,
    by_team: &HashMap<i32, AssignmentRow>,
) -> Vec<SlotAssignment> {
    slots
        .iter()
        .map(|(pos, team_number)| {
            let (team_id, team_name) = team_number
                .and_then(|n| team_lookup.get(&n))
                .map(|t| (Some(t.team_id), t.team_name.clone()))
                .unwrap_or((None, "TBD".to_string()));

            let a = team_id.and_then(|id| by_team.get(&id));
            SlotAssignment {
                position: *pos,
                team_id,
                team_number: team_number.unwrap_or(0),
                team_name,
                assignment_id: a.map(|a| a.id),
                scouter_id: a.and_then(|a| a.scouter_id),
                device_id: a.and_then(|a| a.device_id),
            }
        })
        .collect()
}

// ── Templates ─────────────────────────────────────────────────────────────

const PAGE_DESCRIPTION: &str =
    "Assign scouts or devices to each robot slot per match. Assignees get a pre-filled scouting form.";

/// The assignments page, wrapping the device list and the grid.
#[derive(Template)]
#[template(path = "pages/assignments.html")]
pub struct AssignmentsTemplate {
    pub title: String,
    pub description: String,
    pub nav: Nav,
    pub selected_event_id: Option<i32>,
    pub info: String,
    pub selected_event_name: String,
    pub device_list_html: String,
    pub assignment_table_html: String,
}

impl AssignmentsTemplate {
    /// Page shell around already-rendered fragments.
    pub fn new(
        nav: Nav,
        event_id: i32,
        selected_event_name: String,
        device_list_html: String,
        assignment_table_html: String,
    ) -> Self {
        Self {
            title: "Match Assignments".to_string(),
            description: PAGE_DESCRIPTION.to_string(),
            nav,
            selected_event_id: Some(event_id),
            info: String::new(),
            selected_event_name,
            device_list_html,
            assignment_table_html,
        }
    }

    /// Shown when the lead scout has not picked an event yet.
    pub fn without_event(nav: Nav) -> Self {
        Self {
            title: "Match Assignments".to_string(),
            description: PAGE_DESCRIPTION.to_string(),
            nav,
            selected_event_id: None,
            info: "Select an event on the home page first.".to_string(),
            selected_event_name: String::new(),
            device_list_html: String::new(),
            assignment_table_html: String::new(),
        }
    }
}

/// The grid on its own, re-served after every mutation.
#[derive(Template)]
#[template(path = "partials/assignment_table.html")]
pub struct AssignmentTableFragment {
    pub data: AssignmentData,
}

/// Registered devices with their online state and rename forms.
#[derive(Template)]
#[template(path = "partials/device_list.html")]
pub struct DeviceListFragment {
    pub devices: Vec<DeviceRow>,
}
