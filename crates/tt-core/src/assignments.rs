//! Who scouts which robot in which match (L1-L6).
//!
//! A lead scout gives each robot in each match one assignee -- a person or a
//! tablet -- so that a scout is handed a robot rather than asked to pick one out
//! of a list of fifty. This is the backbone of the whole scouting flow.

use serde::{Deserialize, Serialize};

use crate::records::MatchRecord;

/// One robot in one match, and who is watching it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Assignment {
    pub match_key: String,
    pub team_number: i32,
    pub assignee: Assignee,
}

/// A person, or a physical tablet whoever is holding it.
///
/// Assigning a tablet is what lets a lead scout say "the tablet on the left
/// watches red 1" without caring who signs in on it (REBUILD_SPEC.md 2.6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Assignee {
    Scout {
        id: i64,
        name: String,
    },
    /// `name` is the display name: the lead scout's label for the tablet, or
    /// `Device 0191f7ac` when it has none.
    Device {
        id: i64,
        name: String,
    },
}

impl Assignee {
    pub fn name(&self) -> &str {
        match self {
            Self::Scout { name, .. } | Self::Device { name, .. } => name,
        }
    }

    pub fn is_device(&self) -> bool {
        matches!(self, Self::Device { .. })
    }
}

/// Assignments naming a robot that is no longer in its match.
///
/// TBA revises schedules -- a replay, a surrogate, a playoff slot filled in --
/// and an assignment made against the old one silently points at the wrong
/// robot. Once assignments pre-fill the scouting form (L4) that is a scout
/// recording the wrong robot, which is the exact mistake assignments exist to
/// prevent, so it has to be visible.
///
/// An assignment for a match not in `matches` is not reported: it belongs to
/// some other event's schedule.
pub fn stale<'a>(matches: &[MatchRecord], assignments: &'a [Assignment]) -> Vec<&'a Assignment> {
    assignments
        .iter()
        .filter(|a| {
            matches
                .iter()
                .find(|m| m.key == a.match_key)
                .is_some_and(|m| m.alliance_of(a.team_number).is_none())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matches::CompLevel;

    fn scheduled(number: i32, red: [i32; 3], blue: [i32; 3]) -> MatchRecord {
        MatchRecord {
            key: format!("2026mabil_qm{number}"),
            event_key: "2026mabil".into(),
            comp_level: CompLevel::Qualification,
            set_number: 1,
            match_number: number,
            red: red.map(Some),
            blue: blue.map(Some),
            red_score: None,
            blue_score: None,
            winner: None,
            played: false,
            scheduled_at: None,
            actual_at: None,
        }
    }

    fn assigned(match_key: &str, team_number: i32) -> Assignment {
        Assignment {
            match_key: match_key.into(),
            team_number,
            assignee: Assignee::Scout {
                id: 1,
                name: "Sam".into(),
            },
        }
    }

    #[test]
    fn an_assignee_has_a_name_either_way() {
        let tablet = Assignee::Device {
            id: 4,
            name: "Stands Left".into(),
        };
        assert_eq!(tablet.name(), "Stands Left");
        assert!(tablet.is_device());
        assert!(!assigned("m", 1).assignee.is_device());
    }

    #[test]
    fn an_assignment_to_a_robot_that_left_the_match_is_stale() {
        let matches = [scheduled(1, [1, 2, 3], [4, 5, 6])];
        let assignments = [
            assigned("2026mabil_qm1", 2),
            assigned("2026mabil_qm1", 6),
            // A surrogate replaced 99 after it was assigned.
            assigned("2026mabil_qm1", 99),
        ];

        let stale = stale(&matches, &assignments);
        assert_eq!(stale, [&assignments[2]]);
    }

    #[test]
    fn an_assignment_for_a_match_elsewhere_is_not_this_schedules_business() {
        let matches = [scheduled(1, [1, 2, 3], [4, 5, 6])];
        assert!(stale(&matches, &[assigned("2026nhgrs_qm1", 99)]).is_empty());
    }
}
