//! The pick list (U20): one team's ranking of who it would pick at an event.
//!
//! Each owning team has one list per event, best first, and each entry can be
//! tagged with a colour and crossed off -- crossed off being what happens,
//! live, as alliance selection takes teams.
//!
//! Every change is an [`Edit`] applied to the list **as it is when the change
//! arrives**, not a new order computed on someone's screen. "Move 254 up one"
//! and "cross off 1678" from two leads at once then both land, where the
//! retired app's client-sent positions let the second save silently undo the
//! first (REBUILD_SPEC.md 5.8). Genuinely merging two people's reorders is the
//! CRDT's job (L14); this only makes sure nobody's change is thrown away.

/// A colour tag. What each colour means is the team's own convention.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tag {
    Green,
    Yellow,
    Red,
    Blue,
}

impl Tag {
    pub const ALL: [Tag; 4] = [Tag::Green, Tag::Yellow, Tag::Red, Tag::Blue];

    /// What is stored, and what a form sends.
    pub fn key(self) -> &'static str {
        match self {
            Tag::Green => "green",
            Tag::Yellow => "yellow",
            Tag::Red => "red",
            Tag::Blue => "blue",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Tag::Green => "Green",
            Tag::Yellow => "Yellow",
            Tag::Red => "Red",
            Tag::Blue => "Blue",
        }
    }

    pub fn parse(key: &str) -> Option<Tag> {
        Tag::ALL.into_iter().find(|t| t.key() == key)
    }
}

/// One team on the list. Its place is its index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Minted when the team is added and kept for the row's life (D7).
    pub record_id: String,
    pub team_number: i32,
    pub tag: Option<Tag>,
    pub crossed: bool,
}

/// One change to a list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Edit {
    /// Onto the bottom of the list.
    Add {
        team: i32,
        record_id: String,
    },
    Remove {
        team: i32,
    },
    /// One place towards the top. Relative, so it means the same thing
    /// whatever else moved since the page was drawn.
    Up {
        team: i32,
    },
    Down {
        team: i32,
    },
    /// To a place, counted from 1. Past the end means the bottom.
    MoveTo {
        team: i32,
        place: usize,
    },
    Cross {
        team: i32,
        crossed: bool,
    },
    Tag {
        team: i32,
        tag: Option<Tag>,
    },
}

impl Edit {
    pub fn team(&self) -> i32 {
        match *self {
            Edit::Add { team, .. }
            | Edit::Remove { team }
            | Edit::Up { team }
            | Edit::Down { team }
            | Edit::MoveTo { team, .. }
            | Edit::Cross { team, .. }
            | Edit::Tag { team, .. } => team,
        }
    }
}

/// Apply one edit, or say why it cannot be.
///
/// `roster` is the event's teams. Adding a team not on it is refused, as a
/// typo almost always -- unless the roster is empty, when there is nothing to
/// check against.
pub fn apply(list: &mut Vec<Entry>, edit: &Edit, roster: &[i32]) -> Result<(), String> {
    let team = edit.team();
    let found = list.iter().position(|e| e.team_number == team);

    if let Edit::Add { record_id, .. } = edit {
        if let Some(at) = found {
            return Err(format!(
                "Team {team} is already on the list, at {}.",
                at + 1
            ));
        }
        if !roster.is_empty() && !roster.contains(&team) {
            return Err(format!("Team {team} is not at this event."));
        }
        list.push(Entry {
            record_id: record_id.clone(),
            team_number: team,
            tag: None,
            crossed: false,
        });
        return Ok(());
    }

    // Another lead may have taken it off a moment ago.
    let Some(at) = found else {
        return Err(format!("Team {team} is not on the list."));
    };
    match *edit {
        Edit::Add { .. } => unreachable!("handled above"),
        Edit::Remove { .. } => {
            list.remove(at);
        }
        Edit::Up { .. } => move_to(list, at, at.saturating_sub(1)),
        Edit::Down { .. } => move_to(list, at, at + 1),
        Edit::MoveTo { place, .. } => move_to(list, at, place.saturating_sub(1)),
        Edit::Cross { crossed, .. } => list[at].crossed = crossed,
        Edit::Tag { tag, .. } => list[at].tag = tag,
    }
    Ok(())
}

/// Move the entry at `from` to index `to`, clamped to the list.
fn move_to(list: &mut Vec<Entry>, from: usize, to: usize) {
    let entry = list.remove(from);
    let to = to.min(list.len());
    list.insert(to, entry);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(teams: &[i32]) -> Vec<Entry> {
        teams
            .iter()
            .map(|&team_number| Entry {
                record_id: format!("id-{team_number}"),
                team_number,
                tag: None,
                crossed: false,
            })
            .collect()
    }

    fn order(list: &[Entry]) -> Vec<i32> {
        list.iter().map(|e| e.team_number).collect()
    }

    const ROSTER: &[i32] = &[254, 1678, 118, 10101, 971];

    #[test]
    fn a_team_is_added_to_the_bottom_once() {
        let mut picks = list(&[254]);
        let add = |team| Edit::Add {
            team,
            record_id: "new".into(),
        };
        apply(&mut picks, &add(1678), ROSTER).expect("added");
        assert_eq!(order(&picks), [254, 1678]);
        assert_eq!(picks[1].record_id, "new");

        assert_eq!(
            apply(&mut picks, &add(254), ROSTER),
            Err("Team 254 is already on the list, at 1.".into())
        );
        assert_eq!(
            apply(&mut picks, &add(4414), ROSTER),
            Err("Team 4414 is not at this event.".into())
        );
        // No roster yet: nothing to check against.
        apply(&mut picks, &add(4414), &[]).expect("taken at its word");
    }

    #[test]
    fn moves_are_clamped_to_the_list() {
        let mut picks = list(&[254, 1678, 118]);
        apply(&mut picks, &Edit::Up { team: 254 }, ROSTER).unwrap();
        assert_eq!(order(&picks), [254, 1678, 118], "the top stays the top");
        apply(&mut picks, &Edit::Down { team: 118 }, ROSTER).unwrap();
        assert_eq!(
            order(&picks),
            [254, 1678, 118],
            "the bottom stays the bottom"
        );

        apply(&mut picks, &Edit::Up { team: 118 }, ROSTER).unwrap();
        assert_eq!(order(&picks), [254, 118, 1678]);
        apply(
            &mut picks,
            &Edit::MoveTo {
                team: 254,
                place: 99,
            },
            ROSTER,
        )
        .unwrap();
        assert_eq!(order(&picks), [118, 1678, 254]);
        apply(
            &mut picks,
            &Edit::MoveTo {
                team: 254,
                place: 0,
            },
            ROSTER,
        )
        .unwrap();
        assert_eq!(order(&picks), [254, 118, 1678], "place 0 means the top");
        apply(
            &mut picks,
            &Edit::MoveTo {
                team: 1678,
                place: 2,
            },
            ROSTER,
        )
        .unwrap();
        assert_eq!(order(&picks), [254, 1678, 118]);
    }

    #[test]
    fn two_leads_editing_at_once_both_land() {
        // Both saw [254, 1678, 118]. One moves 118 up; the other crosses off
        // 254. Applied in either order, neither change is lost.
        let up = Edit::Up { team: 118 };
        let cross = Edit::Cross {
            team: 254,
            crossed: true,
        };
        for edits in [[&up, &cross], [&cross, &up]] {
            let mut picks = list(&[254, 1678, 118]);
            for edit in edits {
                apply(&mut picks, edit, ROSTER).unwrap();
            }
            assert_eq!(order(&picks), [254, 118, 1678]);
            assert!(picks[0].crossed);
        }
    }

    #[test]
    fn an_edit_to_a_team_already_removed_says_so() {
        let mut picks = list(&[254, 1678]);
        apply(&mut picks, &Edit::Remove { team: 1678 }, ROSTER).unwrap();
        assert_eq!(order(&picks), [254]);
        assert_eq!(
            apply(
                &mut picks,
                &Edit::Tag {
                    team: 1678,
                    tag: Some(Tag::Red)
                },
                ROSTER
            ),
            Err("Team 1678 is not on the list.".into())
        );
    }

    #[test]
    fn tags_round_trip_through_their_keys() {
        for tag in Tag::ALL {
            assert_eq!(Tag::parse(tag.key()), Some(tag));
        }
        assert_eq!(Tag::parse("purple"), None);
    }
}
