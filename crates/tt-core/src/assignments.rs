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
    /// Which scout or tablet this is, without the name.
    pub fn key(&self) -> AssigneeKey {
        match self {
            Self::Scout { id, .. } => AssigneeKey::Scout(*id),
            Self::Device { id, .. } => AssigneeKey::Device(*id),
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Self::Scout { name, .. } | Self::Device { name, .. } => name,
        }
    }

    pub fn is_device(&self) -> bool {
        matches!(self, Self::Device { .. })
    }
}

/// A scout or a tablet, by id: what a form posts and what storage writes.
///
/// Written `u:<user id>` or `d:<device id>` in forms, as the retired app did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AssigneeKey {
    Scout(i64),
    Device(i64),
}

impl AssigneeKey {
    /// Read a form value. `None` for anything else, including blank, which a
    /// form uses to mean "nobody".
    pub fn parse(raw: &str) -> Option<Self> {
        let (kind, id) = raw.trim().split_once(':')?;
        let id = id.parse().ok().filter(|id: &i64| *id > 0)?;
        match kind {
            "u" => Some(Self::Scout(id)),
            "d" => Some(Self::Device(id)),
            _ => None,
        }
    }
}

impl std::fmt::Display for AssigneeKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Scout(id) => write!(f, "u:{id}"),
            Self::Device(id) => write!(f, "d:{id}"),
        }
    }
}

/// One robot to hand to one assignee.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pick {
    pub match_key: String,
    pub team_number: i32,
    pub assignee: AssigneeKey,
}

/// Hand every open robot in `matches` to someone in `pool` (L2).
///
/// `matches` are the ones to fill, in playing order; `existing` is what is
/// already assigned, which is left alone. The pool is walked round-robin, and
/// the walk carries on from match to match, so with eight scouts the two who
/// sat out one match start the next.
///
/// **Nobody gets two robots in one match.** The retired auto-distribute was
/// `pool[i % pool.len()]` over every open slot, so with four scouts one of them
/// was handed two robots in the same match -- which nobody can watch. With
/// fewer assignees than robots, the rest of the match stays open, and the grid
/// shows it.
pub fn distribute(
    matches: &[MatchRecord],
    existing: &[Assignment],
    pool: &[AssigneeKey],
) -> Vec<Pick> {
    let mut picks = Vec::new();
    let mut cursor = 0;
    for m in matches {
        let current: Vec<&Assignment> = existing
            .iter()
            .filter(|a| a.match_key == m.key && m.alliance_of(a.team_number).is_some())
            .collect();
        let mut busy: Vec<AssigneeKey> = current.iter().map(|a| a.assignee.key()).collect();
        let open: Vec<i32> = m
            .teams()
            .filter(|t| !current.iter().any(|a| a.team_number == *t))
            .collect();

        let mut open = open.into_iter().peekable();
        let mut tried = 0;
        while open.peek().is_some() && tried < pool.len() {
            let candidate = pool[(cursor + tried) % pool.len()];
            tried += 1;
            if busy.contains(&candidate) {
                continue;
            }
            busy.push(candidate);
            picks.push(Pick {
                match_key: m.key.clone(),
                team_number: open.next().expect("peeked"),
                assignee: candidate,
            });
        }
        if !pool.is_empty() {
            cursor = (cursor + tried) % pool.len();
        }
    }
    picks
}

/// Assignee keys that appear on more than one robot in the same match, for a
/// hand-edited match (L2). A person cannot watch two robots at once.
pub fn doubled(chosen: &[AssigneeKey]) -> Vec<AssigneeKey> {
    let mut seen = Vec::new();
    let mut doubled = Vec::new();
    for key in chosen {
        if seen.contains(key) {
            if !doubled.contains(key) {
                doubled.push(*key);
            }
        } else {
            seen.push(*key);
        }
    }
    doubled
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

    fn scout(id: i64) -> AssigneeKey {
        AssigneeKey::Scout(id)
    }

    fn picked(picks: &[Pick], match_number: i32) -> Vec<(i32, AssigneeKey)> {
        let key = format!("2026mabil_qm{match_number}");
        picks
            .iter()
            .filter(|p| p.match_key == key)
            .map(|p| (p.team_number, p.assignee))
            .collect()
    }

    #[test]
    fn assignee_keys_read_and_write_the_form_values() {
        assert_eq!(AssigneeKey::parse("u:3"), Some(scout(3)));
        assert_eq!(AssigneeKey::parse(" d:12 "), Some(AssigneeKey::Device(12)));
        for bad in ["", "u:", "u:x", "u:0", "u:-1", "x:3", "3"] {
            assert_eq!(AssigneeKey::parse(bad), None, "{bad:?}");
        }
        assert_eq!(scout(3).to_string(), "u:3");
        assert_eq!(AssigneeKey::Device(12).to_string(), "d:12");
        assert_eq!(assigned("m", 1).assignee.key(), scout(1), "Sam is user 1");
    }

    #[test]
    fn six_scouts_cover_every_robot_and_keep_rotating() {
        let matches = [
            scheduled(1, [1, 2, 3], [4, 5, 6]),
            scheduled(2, [7, 8, 9], [10, 11, 12]),
        ];
        let pool: Vec<_> = (1..=8).map(scout).collect();
        let picks = distribute(&matches, &[], &pool);

        assert_eq!(picks.len(), 12);
        let first: Vec<_> = picked(&picks, 1).iter().map(|p| p.1).collect();
        assert_eq!(first, (1..=6).map(scout).collect::<Vec<_>>());
        // The two who sat out Q1 start Q2.
        let second: Vec<_> = picked(&picks, 2).iter().map(|p| p.1).collect();
        assert_eq!(second, [7, 8, 1, 2, 3, 4].map(scout));
    }

    #[test]
    fn nobody_is_handed_two_robots_in_one_match() {
        let matches = [scheduled(1, [1, 2, 3], [4, 5, 6])];
        let pool = [scout(1), scout(2), scout(3), scout(4)];
        let picks = distribute(&matches, &[], &pool);

        assert_eq!(
            picked(&picks, 1),
            [(1, scout(1)), (2, scout(2)), (3, scout(3)), (4, scout(4))],
            "blue 2 and blue 3 stay open rather than doubling anyone up"
        );
    }

    #[test]
    fn what_is_already_assigned_is_left_alone_and_its_scout_is_busy() {
        let matches = [scheduled(1, [1, 2, 3], [4, 5, 6])];
        // Sam (scout 1) already has team 5.
        let existing = [assigned("2026mabil_qm1", 5)];
        let picks = distribute(&matches, &existing, &[scout(1), scout(2)]);

        assert_eq!(picked(&picks, 1), [(1, scout(2))]);
    }

    #[test]
    fn a_stale_assignment_frees_neither_its_robot_nor_blocks_its_scout() {
        let matches = [scheduled(1, [1, 2, 3], [4, 5, 6])];
        // Team 99 left the match; Sam is free to take a robot that is in it.
        let existing = [assigned("2026mabil_qm1", 99)];
        let picks = distribute(&matches, &existing, &[scout(1)]);
        assert_eq!(picked(&picks, 1), [(1, scout(1))]);
    }

    #[test]
    fn empty_slots_and_an_empty_pool_hand_out_nothing() {
        let mut playoff = scheduled(1, [1, 2, 3], [4, 5, 6]);
        playoff.red = [None; 3];
        playoff.blue = [None; 3];
        assert!(distribute(&[playoff], &[], &[scout(1)]).is_empty());
        assert!(distribute(&[scheduled(2, [1, 2, 3], [4, 5, 6])], &[], &[]).is_empty());
    }

    #[test]
    fn doubling_up_in_one_match_is_found() {
        let d = AssigneeKey::Device(1);
        assert_eq!(doubled(&[scout(1), d, scout(1), scout(1)]), [scout(1)]);
        assert!(doubled(&[scout(1), d, scout(2)]).is_empty());
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
