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
//! first (REBUILD_SPEC.md 5.8).
//!
//! The list itself is a [`PickDoc`], a yrs (Yjs) document (L14), so two copies
//! edited apart -- the server and a tablet that lost the network, or two
//! requests at the same moment -- merge into one list that keeps both sets of
//! changes, whatever order they arrive in. The rows in storage are only what
//! the document reads as.
//!
//! In the document each team is a map of its own, under its number, holding
//! its record id, its tag, whether it is crossed off, and an order key. The
//! list is the teams sorted by key, then by number. A move gives one team a
//! key between its new neighbours' and touches nobody else, so moves of two
//! different teams both land, as do a cross and a move of the same team. What
//! cannot both land is settled the same way on every copy:
//!
//! - Two moves of one team: one of them.
//! - Two adds of one team: one row, with one of the record ids.
//! - A removal, and a change to that team made without seeing it: the removal.

use std::collections::HashSet;

use yrs::updates::decoder::Decode;
use yrs::{Any, Doc, In, Map, MapPrelim, MapRef, Out, ReadTxn, StateVector, Transact, Update};

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

/// The root map: each team's number to that team's map.
const PICKS: &str = "picks";
const ID: &str = "id";
const KEY: &str = "at";
const TAG: &str = "tag";
const CROSSED: &str = "crossed";

/// A team's pick list as a yrs document: what every copy of the list merges.
pub struct PickDoc {
    doc: Doc,
    picks: MapRef,
}

/// An entry with its order key.
struct Placed {
    key: String,
    entry: Entry,
}

impl PickDoc {
    /// An empty list. Each `PickDoc` writes under a random client id of its
    /// own, which yrs needs of two copies written at once, so make one per
    /// change rather than keeping one around.
    pub fn new() -> PickDoc {
        let doc = Doc::new();
        let picks = doc.get_or_insert_map(PICKS);
        PickDoc { doc, picks }
    }

    /// A list from its stored state ([`PickDoc::state`]).
    pub fn load(state: &[u8]) -> Result<PickDoc, String> {
        let mut list = PickDoc::new();
        list.merge(state)?;
        Ok(list)
    }

    /// A document for a list stored only as rows: one written before the
    /// document existed. Make it once and keep it. Made twice, the two would
    /// merge as two adds of every team.
    pub fn from_entries(entries: &[Entry]) -> PickDoc {
        let list = PickDoc::new();
        {
            let mut txn = list.doc.transact_mut();
            let mut key = String::new();
            for entry in entries {
                key = between(&key, None, None);
                let mut pick = MapPrelim::from([
                    (ID, In::from(entry.record_id.as_str())),
                    (KEY, In::from(key.as_str())),
                    (CROSSED, In::from(entry.crossed)),
                ]);
                if let Some(tag) = entry.tag {
                    pick.insert(TAG.into(), In::from(tag.key()));
                }
                list.picks
                    .insert(&mut txn, entry.team_number.to_string(), pick);
            }
        }
        list
    }

    /// The whole document as one update: to store, or to send a copy.
    pub fn state(&self) -> Vec<u8> {
        self.doc
            .transact()
            .encode_state_as_update_v1(&StateVector::default())
    }

    /// Merge an update from another copy: a change it made, or its whole
    /// state. Merging the same one twice changes nothing.
    pub fn merge(&mut self, update: &[u8]) -> Result<(), String> {
        let update =
            Update::decode_v1(update).map_err(|e| format!("not a pick list update: {e}"))?;
        self.doc
            .transact_mut()
            .apply_update(update)
            .map_err(|e| format!("not a pick list update: {e}"))
    }

    /// The list, best first.
    pub fn entries(&self) -> Vec<Entry> {
        self.placed().into_iter().map(|p| p.entry).collect()
    }

    fn placed(&self) -> Vec<Placed> {
        let txn = self.doc.transact();
        let text = |pick: &MapRef, field: &str| match pick.get(&txn, field) {
            Some(Out::Any(Any::String(s))) => Some(s.to_string()),
            _ => None,
        };
        let mut list: Vec<Placed> = self
            .picks
            .iter(&txn)
            .filter_map(|(team, pick)| {
                let team_number = team.parse().ok()?;
                let Out::YMap(pick) = pick else { return None };
                // What this build did not write is read as best it can be: a
                // pick with no record id cannot be stored, and is left out,
                // and a key it cannot order by sorts first.
                let record_id = text(&pick, ID)?;
                let key = text(&pick, KEY)
                    .filter(|k| valid_key(k))
                    .unwrap_or_default();
                let crossed = matches!(pick.get(&txn, CROSSED), Some(Out::Any(Any::Bool(true))));
                Some(Placed {
                    key,
                    entry: Entry {
                        record_id,
                        team_number,
                        tag: text(&pick, TAG).as_deref().and_then(Tag::parse),
                        crossed,
                    },
                })
            })
            .collect();
        list.sort_by(|a, b| (&a.key, a.entry.team_number).cmp(&(&b.key, b.entry.team_number)));
        // A record id is one row's, never two.
        let mut seen = HashSet::new();
        list.retain(|p| seen.insert(p.entry.record_id.clone()));
        list
    }

    /// Apply one edit, or say why it cannot be. Returns the update it made,
    /// for the other copies, or `None` when it changed nothing.
    ///
    /// `roster` is the event's teams. Adding a team not on it is refused, as
    /// a typo almost always -- unless the roster is empty, when there is
    /// nothing to check against.
    pub fn apply(&mut self, edit: &Edit, roster: &[i32]) -> Result<Option<Vec<u8>>, String> {
        let list = self.placed();
        let team = edit.team();
        let found = list.iter().position(|p| p.entry.team_number == team);
        let jitter = Some(self.doc.client_id().get());
        let mut txn = self.doc.transact_mut();

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
            let last = list.last().map(|p| p.key.as_str()).unwrap_or_default();
            let pick = MapPrelim::from([
                (ID, In::from(record_id.as_str())),
                (KEY, In::from(between(last, None, jitter))),
                (CROSSED, In::from(false)),
            ]);
            self.picks.insert(&mut txn, team.to_string(), pick);
            return Ok(Some(txn.encode_update_v1()));
        }

        // Another lead may have taken it off a moment ago.
        let Some(at) = found else {
            return Err(format!("Team {team} is not on the list."));
        };
        let name = team.to_string();
        let Some(Out::YMap(pick)) = self.picks.get(&txn, &name) else {
            unreachable!("placed() read it from this map");
        };
        let to = match *edit {
            Edit::Add { .. } => unreachable!("handled above"),
            Edit::Remove { .. } => {
                self.picks.remove(&mut txn, &name);
                return Ok(Some(txn.encode_update_v1()));
            }
            Edit::Up { .. } => at.saturating_sub(1),
            Edit::Down { .. } => at + 1,
            Edit::MoveTo { place, .. } => place.saturating_sub(1),
            Edit::Cross { crossed, .. } => {
                if list[at].entry.crossed == crossed {
                    return Ok(None);
                }
                pick.insert(&mut txn, CROSSED, crossed);
                return Ok(Some(txn.encode_update_v1()));
            }
            Edit::Tag { tag, .. } => {
                if list[at].entry.tag == tag {
                    return Ok(None);
                }
                match tag {
                    Some(tag) => {
                        pick.insert(&mut txn, TAG, tag.key());
                    }
                    None => {
                        pick.remove(&mut txn, TAG);
                    }
                }
                return Ok(Some(txn.encode_update_v1()));
            }
        };

        // A move, clamped to the list: a key between the neighbours it will
        // have. Nobody else's key changes.
        let to = to.min(list.len() - 1);
        if to == at {
            return Ok(None);
        }
        let others: Vec<&str> = list
            .iter()
            .enumerate()
            .filter(|&(i, _)| i != at)
            .map(|(_, p)| p.key.as_str())
            .collect();
        let before = if to == 0 { "" } else { others[to - 1] };
        let key = between(before, others.get(to).copied(), jitter);
        pick.insert(&mut txn, KEY, key);
        Ok(Some(txn.encode_update_v1()))
    }
}

impl Default for PickDoc {
    fn default() -> PickDoc {
        PickDoc::new()
    }
}

// ── Order keys ──────────────────────────────────────────────────────────────
//
// Fractions in base 36, written without the "0.": "i" is a half, and "0i" a
// little under a tenth. There is always room between two of them, so a move
// never renumbers anyone else, which would undo whatever another copy did to
// them meanwhile. No key ends in "0", or there would be no room before it
// ("a" < "a0", with nothing between).

const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";

fn digit(c: u8) -> usize {
    DIGITS.iter().position(|&d| d == c).unwrap_or(0)
}

fn valid_key(key: &str) -> bool {
    !key.is_empty() && !key.ends_with('0') && key.bytes().all(|c| DIGITS.contains(&c))
}

/// A key after `a` and before `b`, or anywhere after `a` when `b` is `None`.
/// `""` is before every key.
///
/// `jitter`, a copy's client id, adds two digits where they fit, so two
/// copies putting a team into the same gap at once do not pick the same key.
/// If they did, the tie would be broken by team number, and nothing could
/// later be put between those two. When `a` is not before `b` -- such a tie
/// -- the key goes after `a`, and so after both.
fn between(a: &str, b: Option<&str>, jitter: Option<u64>) -> String {
    if let Some(b) = b
        && a >= b
    {
        return format!("{a}{}", between("", None, jitter));
    }
    let mid = midpoint(a.as_bytes(), b.map(str::as_bytes));
    let key = String::from_utf8(mid).expect("digits are ascii");
    let Some(jitter) = jitter else { return key };
    let mut jittered = key.clone();
    jittered.push(DIGITS[(jitter % 36) as usize] as char);
    jittered.push(DIGITS[1 + ((jitter / 36) % 35) as usize] as char);
    if b.is_none_or(|b| jittered.as_str() < b) {
        jittered
    } else {
        key
    }
}

/// The digits of a key halfway between `a` and `b`, given `a < b`.
fn midpoint(a: &[u8], b: Option<&[u8]>) -> Vec<u8> {
    if let Some(b) = b {
        // A shared start, reading a short `a` as followed by zeros.
        let n = b
            .iter()
            .enumerate()
            .take_while(|&(i, &d)| a.get(i).copied().unwrap_or(b'0') == d)
            .count();
        if n > 0 {
            let mut key = b[..n].to_vec();
            key.extend(midpoint(a.get(n..).unwrap_or_default(), Some(&b[n..])));
            return key;
        }
    }
    let low = a.first().map_or(0, |&c| digit(c));
    let high = b.map_or(DIGITS.len(), |b| digit(b[0]));
    if high - low > 1 {
        return vec![DIGITS[(low + high) / 2]];
    }
    // Next-door digits. `b`'s first alone is between, if `b` goes on past it;
    // otherwise keep `a`'s and look further along.
    if let Some(b) = b.filter(|b| b.len() > 1) {
        return vec![b[0]];
    }
    let mut key = vec![DIGITS[low]];
    key.extend(midpoint(a.get(1..).unwrap_or_default(), None));
    key
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

    fn doc(teams: &[i32]) -> PickDoc {
        PickDoc::from_entries(&list(teams))
    }

    /// Another copy of the same list, as a tablet or a request would load it.
    fn copy(list: &PickDoc) -> PickDoc {
        PickDoc::load(&list.state()).expect("loads")
    }

    fn order(list: &PickDoc) -> Vec<i32> {
        list.entries().iter().map(|e| e.team_number).collect()
    }

    fn edit(list: &mut PickDoc, edit: Edit) -> Vec<u8> {
        list.apply(&edit, ROSTER)
            .expect("applies")
            .expect("changes something")
    }

    /// Each copy takes the other's update; both must then read the same.
    fn exchange(a: &mut PickDoc, from_a: &[u8], b: &mut PickDoc, from_b: &[u8]) {
        a.merge(from_b).expect("merges");
        b.merge(from_a).expect("merges");
        assert_eq!(a.entries(), b.entries(), "the copies agree");
    }

    const ROSTER: &[i32] = &[254, 1678, 118, 10101, 971];

    #[test]
    fn a_team_is_added_to_the_bottom_once() {
        let mut picks = doc(&[254]);
        let add = |team| Edit::Add {
            team,
            record_id: "new".into(),
        };
        picks.apply(&add(1678), ROSTER).expect("added");
        assert_eq!(order(&picks), [254, 1678]);
        assert_eq!(picks.entries()[1].record_id, "new");

        assert_eq!(
            picks.apply(&add(254), ROSTER),
            Err("Team 254 is already on the list, at 1.".into())
        );
        assert_eq!(
            picks.apply(&add(4414), ROSTER),
            Err("Team 4414 is not at this event.".into())
        );
        // No roster yet: nothing to check against.
        picks.apply(&add(4414), &[]).expect("taken at its word");
    }

    #[test]
    fn moves_are_clamped_to_the_list() {
        let mut picks = doc(&[254, 1678, 118]);
        assert_eq!(picks.apply(&Edit::Up { team: 254 }, ROSTER), Ok(None));
        assert_eq!(order(&picks), [254, 1678, 118], "the top stays the top");
        assert_eq!(picks.apply(&Edit::Down { team: 118 }, ROSTER), Ok(None));
        assert_eq!(
            order(&picks),
            [254, 1678, 118],
            "the bottom stays the bottom"
        );

        edit(&mut picks, Edit::Up { team: 118 });
        assert_eq!(order(&picks), [254, 118, 1678]);
        edit(
            &mut picks,
            Edit::MoveTo {
                team: 254,
                place: 99,
            },
        );
        assert_eq!(order(&picks), [118, 1678, 254]);
        edit(
            &mut picks,
            Edit::MoveTo {
                team: 254,
                place: 0,
            },
        );
        assert_eq!(order(&picks), [254, 118, 1678], "place 0 means the top");
        edit(
            &mut picks,
            Edit::MoveTo {
                team: 1678,
                place: 2,
            },
        );
        assert_eq!(order(&picks), [254, 1678, 118]);
        edit(&mut picks, Edit::Down { team: 254 });
        assert_eq!(order(&picks), [1678, 254, 118]);
    }

    #[test]
    fn a_list_reads_back_from_its_state_and_its_rows() {
        let mut rows = list(&[254, 1678, 118]);
        rows[0].tag = Some(Tag::Green);
        rows[2].crossed = true;
        let picks = PickDoc::from_entries(&rows);
        assert_eq!(picks.entries(), rows);
        assert_eq!(copy(&picks).entries(), rows);
        assert!(PickDoc::load(b"not yrs").is_err());
    }

    #[test]
    fn two_leads_editing_at_once_both_land() {
        // Both saw [254, 1678, 118]. One moves 118 up; the other crosses off
        // 254 and tags 118.
        let mut one = doc(&[254, 1678, 118]);
        let mut other = copy(&one);
        let up = edit(&mut one, Edit::Up { team: 118 });
        edit(
            &mut other,
            Edit::Cross {
                team: 254,
                crossed: true,
            },
        );
        edit(
            &mut other,
            Edit::Tag {
                team: 118,
                tag: Some(Tag::Red),
            },
        );
        let theirs = other.state();
        exchange(&mut one, &up, &mut other, &theirs);
        assert_eq!(order(&one), [254, 118, 1678]);
        let entries = one.entries();
        assert!(entries[0].crossed);
        assert_eq!(entries[1].tag, Some(Tag::Red));
    }

    #[test]
    fn two_leads_reordering_at_once_both_land() {
        // The retired app's bug (REBUILD_SPEC.md 5.8): the second reorder
        // undid the first. Here 971 goes to the top on one copy while 1678
        // goes below 118 on the other, and both moves stand.
        let mut one = doc(&[254, 1678, 118, 971]);
        let mut other = copy(&one);
        let top = edit(
            &mut one,
            Edit::MoveTo {
                team: 971,
                place: 1,
            },
        );
        let down = edit(&mut other, Edit::Down { team: 1678 });
        exchange(&mut one, &top, &mut other, &down);
        assert_eq!(order(&one), [971, 254, 118, 1678]);
    }

    #[test]
    fn two_moves_into_one_gap_both_land_and_leave_room() {
        let mut one = doc(&[254, 1678, 118, 971]);
        let mut other = copy(&one);
        let first = edit(
            &mut one,
            Edit::MoveTo {
                team: 118,
                place: 2,
            },
        );
        let second = edit(
            &mut other,
            Edit::MoveTo {
                team: 971,
                place: 2,
            },
        );
        exchange(&mut one, &first, &mut other, &second);
        let merged = order(&one);
        assert_eq!(merged[0], 254);
        assert_eq!(merged[3], 1678);

        // The two landed on different keys, so there is room between them.
        let lower = merged[2];
        edit(
            &mut one,
            Edit::MoveTo {
                team: 1678,
                place: 3,
            },
        );
        assert_eq!(order(&one), [254, merged[1], 1678, lower]);
    }

    #[test]
    fn one_team_moved_two_ways_ends_up_in_one_place() {
        let mut one = doc(&[254, 1678, 118]);
        let mut other = copy(&one);
        let top = edit(
            &mut one,
            Edit::MoveTo {
                team: 118,
                place: 1,
            },
        );
        let middle = edit(&mut other, Edit::Up { team: 118 });
        exchange(&mut one, &top, &mut other, &middle);
        let merged = order(&one);
        assert_eq!(merged.len(), 3);
        assert!(merged == [118, 254, 1678] || merged == [254, 118, 1678]);
    }

    #[test]
    fn a_team_added_on_two_copies_is_on_the_list_once() {
        let mut one = doc(&[254]);
        let mut other = copy(&one);
        let add = |record_id: &str| Edit::Add {
            team: 1678,
            record_id: record_id.into(),
        };
        let a = edit(&mut one, add("from-one"));
        let b = edit(&mut other, add("from-other"));
        exchange(&mut one, &a, &mut other, &b);
        assert_eq!(order(&one), [254, 1678]);
    }

    #[test]
    fn a_removal_beats_a_change_made_without_seeing_it() {
        let mut one = doc(&[254, 1678]);
        let mut other = copy(&one);
        let removed = edit(&mut one, Edit::Remove { team: 1678 });
        let crossed = edit(
            &mut other,
            Edit::Cross {
                team: 1678,
                crossed: true,
            },
        );
        exchange(&mut one, &removed, &mut other, &crossed);
        assert_eq!(order(&one), [254]);
    }

    #[test]
    fn a_copy_edited_offline_merges_later_and_twice_is_once() {
        // A tablet takes the list, loses the network, and makes three
        // changes; meanwhile the server takes two from someone else.
        let mut server = doc(&[254, 1678, 118]);
        let mut tablet = copy(&server);
        let offline = [
            Edit::Down { team: 254 },
            Edit::Add {
                team: 971,
                record_id: "tablet-971".into(),
            },
            Edit::Tag {
                team: 971,
                tag: Some(Tag::Blue),
            },
        ]
        .map(|change| edit(&mut tablet, change));
        edit(
            &mut server,
            Edit::Cross {
                team: 118,
                crossed: true,
            },
        );
        edit(
            &mut server,
            Edit::MoveTo {
                team: 118,
                place: 1,
            },
        );

        // Back online, the tablet sends its whole state (or each change; the
        // same thing merged), and takes the server's back.
        server.merge(&tablet.state()).unwrap();
        for update in &offline {
            server.merge(update).unwrap();
        }
        tablet.merge(&server.state()).unwrap();
        assert_eq!(server.entries(), tablet.entries());
        assert_eq!(order(&server), [118, 1678, 254, 971]);
        let entries = server.entries();
        assert!(entries[0].crossed);
        assert_eq!(entries[3].tag, Some(Tag::Blue));
        assert_eq!(entries[3].record_id, "tablet-971");
    }

    #[test]
    fn an_edit_to_a_team_already_removed_says_so() {
        let mut picks = doc(&[254, 1678]);
        edit(&mut picks, Edit::Remove { team: 1678 });
        assert_eq!(order(&picks), [254]);
        assert_eq!(
            picks.apply(
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
    fn an_edit_that_changes_nothing_sends_nothing() {
        let mut picks = doc(&[254]);
        let untag = Edit::Tag {
            team: 254,
            tag: None,
        };
        let uncross = Edit::Cross {
            team: 254,
            crossed: false,
        };
        assert_eq!(picks.apply(&untag, ROSTER), Ok(None));
        assert_eq!(picks.apply(&uncross, ROSTER), Ok(None));
    }

    #[test]
    fn there_is_always_room_between_two_keys() {
        // Squeeze a hundred keys in just after the first of two, the worst
        // case, and a hundred more at the top.
        for jitter in [None, Some(0), Some(u64::MAX)] {
            let (low, mut high) = ("i".to_string(), "j".to_string());
            for _ in 0..100 {
                let key = between(&low, Some(&high), jitter);
                assert!(low < key && key < high, "{low} < {key} < {high}");
                assert!(valid_key(&key), "{key}");
                high = key;
            }
            let mut top = "1".to_string();
            for _ in 0..100 {
                let key = between("", Some(&top), jitter);
                assert!(key.as_str() > "" && key < top, "{key} < {top}");
                assert!(valid_key(&key), "{key}");
                top = key;
            }
        }
        assert!(between("z", None, None).as_str() > "z");
        // A tie leaves no room between, so the key goes after both.
        assert!(between("h", Some("h"), None).as_str() > "h");
    }

    #[test]
    fn tags_round_trip_through_their_keys() {
        for tag in Tag::ALL {
            assert_eq!(Tag::parse(tag.key()), Some(tag));
        }
        assert_eq!(Tag::parse("purple"), None);
    }
}
