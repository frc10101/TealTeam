//! Who scouts which robot in which match (L1-L6).
//!
//! A lead scout gives each robot in each match one assignee -- a person or a
//! tablet -- so that a scout is handed a robot rather than asked to pick one out
//! of a list of fifty. This is the backbone of the whole scouting flow.

use std::collections::HashMap;

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

/// The most matches in a row anyone is handed while somebody else is free
/// (L13). A qualification match comes round every seven or eight minutes, so
/// six is the better part of an hour without looking away.
pub const LONGEST_RUN: usize = 6;

/// How far apart two scouts' loads can drift before the lighter one is
/// suggested for one of the heavier one's robots (L13). With more scouts than
/// robots a fair rota keeps everyone within one match of each other, so three
/// is a real imbalance, not rounding.
pub const UNEVEN: usize = 3;

/// Who is on duty in each match of the schedule, in playing order: every
/// assignment to a robot still in its match. Assignments elsewhere and stale
/// ones are left out, as everywhere else.
fn duties(schedule: &[MatchRecord], assignments: &[Assignment]) -> Vec<Vec<(i32, AssigneeKey)>> {
    schedule
        .iter()
        .map(|m| {
            assignments
                .iter()
                .filter(|a| a.match_key == m.key && m.alliance_of(a.team_number).is_some())
                .map(|a| (a.team_number, a.assignee.key()))
                .collect()
        })
        .collect()
}

/// Matches each assignee is on duty for, played or to come.
fn loads(duties: &[Vec<(i32, AssigneeKey)>]) -> HashMap<AssigneeKey, usize> {
    let mut loads = HashMap::new();
    for (_, key) in duties.iter().flatten() {
        *loads.entry(*key).or_insert(0) += 1;
    }
    loads
}

fn on_duty(duties: &[Vec<(i32, AssigneeKey)>], i: usize, key: AssigneeKey) -> bool {
    duties[i].iter().any(|(_, k)| *k == key)
}

/// How long a run `key` would be in if they also had match `i`: the matches
/// in a row on duty just before it, it, and just after.
fn run_through(duties: &[Vec<(i32, AssigneeKey)>], i: usize, key: AssigneeKey) -> usize {
    let before = (0..i)
        .rev()
        .take_while(|&j| on_duty(duties, j, key))
        .count();
    let after = (i + 1..duties.len())
        .take_while(|&j| on_duty(duties, j, key))
        .count();
    before + 1 + after
}

/// Hand every open robot in the next `limit` unplayed matches of `schedule`
/// (all of them when `None`) to someone in `pool` (L2, L13).
///
/// `schedule` is the whole event in playing order, played matches included:
/// they are the history the rota is kept fair against. `existing` is what is
/// already assigned, which is left alone.
///
/// **Nobody gets two robots in one match.** The retired auto-distribute was
/// `pool[i % pool.len()]` over every open slot, so with four scouts one of them
/// was handed two robots in the same match -- which nobody can watch. With
/// fewer assignees than robots, the rest of the match stays open, and the grid
/// shows it.
///
/// **Each robot goes to whoever has had the fewest matches** at the event so
/// far, counting what is already assigned and what this call has handed out,
/// ties in pool order -- so a scout who sat one out starts the next, and one
/// who joins at lunch catches up. **But nobody is handed a match that would
/// make more than [`LONGEST_RUN`] in a row while somebody else is free**, so
/// catching up never means an afternoon without a break. When there are no
/// more people than robots there is no one to rest, and they are handed it
/// anyway.
pub fn distribute(
    schedule: &[MatchRecord],
    existing: &[Assignment],
    pool: &[AssigneeKey],
    limit: Option<usize>,
) -> Vec<Pick> {
    let mut duties = duties(schedule, existing);
    let mut loads = loads(&duties);
    let fill: Vec<usize> = (0..schedule.len())
        .filter(|&i| !schedule[i].played)
        .take(limit.unwrap_or(usize::MAX))
        .collect();

    let mut picks = Vec::new();
    for i in fill {
        let m = &schedule[i];
        let open: Vec<i32> = m
            .teams()
            .filter(|t| !duties[i].iter().any(|(team, _)| team == t))
            .collect();
        let mut free: Vec<AssigneeKey> = pool
            .iter()
            .copied()
            .filter(|key| !on_duty(&duties, i, *key))
            .collect();
        // A stable sort, so ties keep pool order.
        free.sort_by_key(|key| {
            (
                run_through(&duties, i, *key) > LONGEST_RUN,
                loads.get(key).copied().unwrap_or(0),
            )
        });
        for (team, key) in open.into_iter().zip(free) {
            duties[i].push((team, key));
            *loads.entry(key).or_insert(0) += 1;
            picks.push(Pick {
                match_key: m.key.clone(),
                team_number: team,
                assignee: key,
            });
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

/// A scout's own assignments at an event, as the scouting page sees them
/// (L3-L5).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Agenda {
    /// What to scout now: the first assignment in an unplayed match that the
    /// scout has not recorded. `(match key, team number)`.
    pub next: Option<(String, i32)>,
    /// Assignments in played matches the scout has not recorded, in playing
    /// order -- a match can end mid-form, and an unrecorded robot is a hole
    /// in the data.
    pub missed: Vec<(String, i32)>,
    /// Every assignment in an unplayed match the scout has not recorded, in
    /// playing order, `next` first. The whole list goes onto the page so a
    /// scout who loses the network still knows where to look (C8).
    pub upcoming: Vec<(String, i32)>,
}

impl Agenda {
    /// The robot assigned to this scout in `match_key` that they have not
    /// recorded yet, if any.
    pub fn open_in(&self, match_key: &str) -> Option<i32> {
        self.next
            .iter()
            .chain(&self.missed)
            .find(|(key, _)| key == match_key)
            .map(|(_, team)| *team)
    }
}

/// Build a scout's agenda.
///
/// `me` is everything that identifies the scout: their account and the tablet
/// they are on. An assignment to either counts, which is what lets a lead
/// scout assign "the tablet on the left" without caring who signs in on it
/// (REBUILD_SPEC.md 5.2). `recorded` is what the scout has already recorded,
/// `(match key, team number)`. Assignments to a robot no longer in its match
/// are skipped: following one would mean watching the wrong robot.
pub fn agenda(
    matches: &[MatchRecord],
    assignments: &[Assignment],
    me: &[AssigneeKey],
    recorded: &[(String, i32)],
) -> Agenda {
    let mut agenda = Agenda::default();
    for m in matches {
        for a in assignments.iter().filter(|a| {
            a.match_key == m.key
                && me.contains(&a.assignee.key())
                && m.alliance_of(a.team_number).is_some()
                && !recorded
                    .iter()
                    .any(|(key, team)| *key == a.match_key && *team == a.team_number)
        }) {
            let duty = (a.match_key.clone(), a.team_number);
            if m.played {
                agenda.missed.push(duty);
            } else {
                agenda.next.get_or_insert_with(|| duty.clone());
                agenda.upcoming.push(duty);
            }
        }
    }
    agenda
}

// ── Coverage (L6) ───────────────────────────────────────────────────────────

/// A stored observation, as coverage sees it: which robot, and who recorded
/// it. Declined observations are not sightings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sighting {
    pub match_key: String,
    pub team_number: i32,
    pub scouter_id: Option<i64>,
    pub device_id: Option<i64>,
}

impl Sighting {
    /// Whether this assignee recorded it: the scout by account, the tablet by
    /// the device it was saved from.
    pub fn by(&self, assignee: AssigneeKey) -> bool {
        match assignee {
            AssigneeKey::Scout(id) => self.scouter_id == Some(id),
            AssigneeKey::Device(id) => self.device_id == Some(id),
        }
    }
}

/// Where one driver station in one match stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotState {
    /// No team in the slot yet.
    Empty,
    /// Somebody has recorded this robot -- this many observations. Whoever it
    /// was, the data exists.
    Recorded(usize),
    /// Played, assigned, and nobody recorded it.
    Missed,
    /// Played, never assigned, and nobody recorded it.
    Unscouted,
    /// To come, with somebody on it.
    Assigned,
    /// To come, with nobody on it.
    Open,
}

/// The state of `team`'s slot in `m`.
pub fn slot_state(
    m: &MatchRecord,
    team: Option<i32>,
    assigned: bool,
    sightings: &[Sighting],
) -> SlotState {
    let Some(team) = team else {
        return SlotState::Empty;
    };
    let seen = sightings
        .iter()
        .filter(|s| s.match_key == m.key && s.team_number == team)
        .count();
    match (seen, m.played, assigned) {
        (n, _, _) if n > 0 => SlotState::Recorded(n),
        (_, true, true) => SlotState::Missed,
        (_, true, false) => SlotState::Unscouted,
        (_, false, true) => SlotState::Assigned,
        (_, false, false) => SlotState::Open,
    }
}

/// How one assignee is doing: what the lead scout needs to see who has been
/// submitting and who has not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tally {
    pub assignee: AssigneeKey,
    pub name: String,
    /// Assigned robots they recorded.
    pub recorded: usize,
    /// Assigned robots in played matches they did not record -- even if
    /// somebody else did, which the grid shows separately.
    pub missed: usize,
    /// Assigned robots in matches still to come.
    pub to_come: usize,
}

/// A tally per assignee with anything assigned, in order of first appearance
/// in the schedule. Assignments to robots no longer in their match are left
/// out, as everywhere else.
pub fn tallies(
    matches: &[MatchRecord],
    assignments: &[Assignment],
    sightings: &[Sighting],
) -> Vec<Tally> {
    let mut tallies: Vec<Tally> = Vec::new();
    for m in matches {
        for a in assignments
            .iter()
            .filter(|a| a.match_key == m.key && m.alliance_of(a.team_number).is_some())
        {
            let key = a.assignee.key();
            let i = match tallies.iter().position(|t| t.assignee == key) {
                Some(i) => i,
                None => {
                    tallies.push(Tally {
                        assignee: key,
                        name: a.assignee.name().to_string(),
                        recorded: 0,
                        missed: 0,
                        to_come: 0,
                    });
                    tallies.len() - 1
                }
            };
            let done = sightings
                .iter()
                .any(|s| s.match_key == m.key && s.team_number == a.team_number && s.by(key));
            let tally = &mut tallies[i];
            if done {
                tally.recorded += 1;
            } else if m.played {
                tally.missed += 1;
            } else {
                tally.to_come += 1;
            }
        }
    }
    tallies
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

// ── Rotation (L13) ──────────────────────────────────────────────────────────

/// One robot the lead scout could move to somebody else, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub match_key: String,
    pub team_number: i32,
    /// The scout who has it now.
    pub from: AssigneeKey,
    pub from_name: String,
    /// Who could take it: free in that match, and not pushed past
    /// [`LONGEST_RUN`] in a row by it. `None` when nobody is.
    pub to: Option<(AssigneeKey, String)>,
    pub reason: Reason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    /// `from` is on duty for `len` matches in a row, `first` to `last`
    /// (match keys); the suggestion is the first to come past
    /// [`LONGEST_RUN`].
    LongRun {
        first: String,
        last: String,
        len: usize,
    },
    /// `from` has `from_load` matches at the event and `to` has `to_load`,
    /// [`UNEVEN`] or more fewer.
    Uneven { from_load: usize, to_load: usize },
}

/// Robots to move so that nobody scouts too long without a break or ends up
/// with far more of the event than anyone else (L13), instead of the lead
/// scout keeping count in their head.
///
/// Only scouts are rotated. A tablet does not tire, and who will be holding
/// it in an hour is not known in advance; when tablets are assigned, the
/// people holding them swap among themselves.
///
/// `relief` is who could take over, in order of preference: the web passes
/// scouts online now and scouts with something still to come, so nobody who
/// went home is suggested. A scout's load is every match they are on duty for
/// at the event, played or to come, whether or not they recorded it -- it is
/// the time they were asked to give.
///
/// At most one suggestion per scout, the long run first. The suggestions are
/// worked out one after another as though each were taken, so two of them
/// never hand the same person two robots in one match, or both lean on the
/// one person sitting out. Taking one and reloading shows the next.
pub fn rotation(
    schedule: &[MatchRecord],
    assignments: &[Assignment],
    relief: &[(AssigneeKey, String)],
) -> Vec<Suggestion> {
    let mut duties = duties(schedule, assignments);
    let mut loads = loads(&duties);

    let mut scouts: Vec<(AssigneeKey, String)> = Vec::new();
    for m in schedule {
        for a in assignments.iter().filter(|a| {
            a.match_key == m.key
                && !a.assignee.is_device()
                && m.alliance_of(a.team_number).is_some()
        }) {
            if !scouts.iter().any(|(key, _)| *key == a.assignee.key()) {
                scouts.push((a.assignee.key(), a.assignee.name().to_string()));
            }
        }
    }

    // Who could take match `i` off `from`, the lightest first.
    let reliever = |duties: &[Vec<(i32, AssigneeKey)>],
                    loads: &HashMap<AssigneeKey, usize>,
                    i: usize,
                    from: AssigneeKey| {
        relief
            .iter()
            .filter(|(key, _)| {
                *key != from
                    && !on_duty(duties, i, *key)
                    && run_through(duties, i, *key) <= LONGEST_RUN
            })
            .min_by_key(|(key, _)| loads.get(key).copied().unwrap_or(0))
            .cloned()
    };

    let mut suggestions = Vec::new();
    for (from, from_name) in scouts {
        let found = long_run(schedule, &duties, from)
            .map(|(first, last, at)| {
                let to = reliever(&duties, &loads, at, from);
                let reason = Reason::LongRun {
                    first: schedule[first].key.clone(),
                    last: schedule[last].key.clone(),
                    len: last - first + 1,
                };
                (at, to, reason)
            })
            .or_else(|| {
                let from_load = loads.get(&from).copied().unwrap_or(0);
                (0..schedule.len())
                    .filter(|&i| !schedule[i].played && on_duty(&duties, i, from))
                    .find_map(|i| {
                        let to = reliever(&duties, &loads, i, from)?;
                        let to_load = loads.get(&to.0).copied().unwrap_or(0);
                        (from_load >= to_load + UNEVEN).then_some((
                            i,
                            Some(to),
                            Reason::Uneven { from_load, to_load },
                        ))
                    })
            });
        let Some((i, to, reason)) = found else {
            continue;
        };
        let slot = duties[i]
            .iter()
            .position(|(_, key)| *key == from)
            .expect("on duty");
        let team_number = duties[i][slot].0;
        if let Some((key, _)) = &to {
            duties[i][slot].1 = *key;
            *loads.entry(*key).or_insert(0) += 1;
            *loads.entry(from).or_insert(0) -= 1;
        }
        suggestions.push(Suggestion {
            match_key: schedule[i].key.clone(),
            team_number,
            from,
            from_name,
            to,
            reason,
        });
    }
    suggestions
}

/// `key`'s first run of more than [`LONGEST_RUN`] matches in a row that still
/// has a match to come past the limit: `(first, last, the match to give
/// away)`, as schedule indexes. A long run entirely in the past is history.
fn long_run(
    schedule: &[MatchRecord],
    duties: &[Vec<(i32, AssigneeKey)>],
    key: AssigneeKey,
) -> Option<(usize, usize, usize)> {
    let mut i = 0;
    while i < duties.len() {
        if !on_duty(duties, i, key) {
            i += 1;
            continue;
        }
        let first = i;
        while i < duties.len() && on_duty(duties, i, key) {
            i += 1;
        }
        let last = i - 1;
        if let Some(at) = (first + LONGEST_RUN..=last).find(|&j| !schedule[j].played) {
            return Some((first, last, at));
        }
    }
    None
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
        let picks = distribute(&matches, &[], &pool, None);

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
        let picks = distribute(&matches, &[], &pool, None);

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
        let picks = distribute(&matches, &existing, &[scout(1), scout(2)], None);

        assert_eq!(picked(&picks, 1), [(1, scout(2))]);
    }

    #[test]
    fn a_stale_assignment_frees_neither_its_robot_nor_blocks_its_scout() {
        let matches = [scheduled(1, [1, 2, 3], [4, 5, 6])];
        // Team 99 left the match; Sam is free to take a robot that is in it.
        let existing = [assigned("2026mabil_qm1", 99)];
        let picks = distribute(&matches, &existing, &[scout(1)], None);
        assert_eq!(picked(&picks, 1), [(1, scout(1))]);
    }

    #[test]
    fn empty_slots_and_an_empty_pool_hand_out_nothing() {
        let mut playoff = scheduled(1, [1, 2, 3], [4, 5, 6]);
        playoff.red = [None; 3];
        playoff.blue = [None; 3];
        assert!(distribute(&[playoff], &[], &[scout(1)], None).is_empty());
        assert!(distribute(&[scheduled(2, [1, 2, 3], [4, 5, 6])], &[], &[], None).is_empty());
    }

    fn season(played: i32, to_come: i32) -> Vec<MatchRecord> {
        (1..=played + to_come)
            .map(|n| {
                let mut m = scheduled(n, [1, 2, 3], [4, 5, 6]);
                m.played = n <= played;
                m
            })
            .collect()
    }

    fn named(id: i64, name: &str) -> Assignee {
        Assignee::Scout {
            id,
            name: name.into(),
        }
    }

    /// The longest run of matches in a row `key` is in, and their count.
    fn run_and_load(
        schedule: &[MatchRecord],
        all: &[Assignment],
        key: AssigneeKey,
    ) -> (usize, usize) {
        let duties = duties(schedule, all);
        let (mut run, mut longest, mut load) = (0, 0, 0);
        for i in 0..duties.len() {
            if on_duty(&duties, i, key) {
                run += 1;
                load += 1;
                longest = longest.max(run);
            } else {
                run = 0;
            }
        }
        (longest, load)
    }

    fn with_picks(existing: &[Assignment], picks: &[Pick]) -> Vec<Assignment> {
        let mut all = existing.to_vec();
        all.extend(picks.iter().map(|p| Assignment {
            match_key: p.match_key.clone(),
            team_number: p.team_number,
            assignee: match p.assignee {
                AssigneeKey::Scout(id) => named(id, "x"),
                AssigneeKey::Device(id) => tablet(id),
            },
        }));
        all
    }

    #[test]
    fn only_the_next_unplayed_matches_are_filled() {
        let schedule = season(2, 3);
        let picks = distribute(&schedule, &[], &[scout(1)], Some(2));
        let keys: Vec<_> = picks.iter().map(|p| p.match_key.as_str()).collect();
        assert_eq!(keys, ["2026mabil_qm3", "2026mabil_qm4"]);
    }

    #[test]
    fn whoever_has_had_the_fewest_matches_goes_first() {
        // Sam scouted both played matches; Alex none.
        let schedule = season(2, 1);
        let existing = [to(1, 1, sam()), to(2, 1, sam())];
        let picks = distribute(&schedule, &existing, &[scout(1), scout(2)], None);
        assert_eq!(picked(&picks, 3), [(1, scout(2)), (2, scout(1))]);
    }

    #[test]
    fn a_latecomer_catches_up_without_going_an_hour_without_a_break() {
        // Six scouts did all of Q1-Q10; a seventh arrives for Q11-Q40.
        let schedule = season(10, 30);
        let existing: Vec<_> = (1..=10)
            .flat_map(|m| (1..=6).map(move |s| to(m, s as i32, named(s, "x"))))
            .collect();
        let pool: Vec<_> = (1..=7).map(scout).collect();
        let picks = distribute(&schedule, &existing, &pool, None);
        assert_eq!(picks.len(), 30 * 6);

        let all = with_picks(&existing, &picks);
        let (run, late) = run_and_load(&schedule, &all, scout(7));
        assert!(run <= LONGEST_RUN, "latecomer ran {run} in a row");
        let (_, early) = run_and_load(&schedule, &all, scout(1));
        assert!(
            late > 30 * 6 / 7,
            "the latecomer did more than a seventh: {late}"
        );
        assert!(
            early > late,
            "but not more than the early ones: {early} vs {late}"
        );
        for s in 1..=7 {
            let (run, _) = run_and_load(&schedule[10..], &all, scout(s));
            assert!(run <= LONGEST_RUN, "scout {s} ran {run} in a row");
        }
    }

    #[test]
    fn with_no_one_to_rest_a_long_run_is_still_handed_out() {
        let schedule = season(0, 8);
        let picks = distribute(&schedule, &[], &[scout(1)], None);
        assert_eq!(picks.len(), 8, "an open robot helps nobody");
    }

    #[test]
    fn a_long_run_is_broken_past_the_limit_by_the_lightest_free_scout() {
        let schedule = season(2, 6);
        let mut assignments: Vec<_> = (1..=8).map(|m| to(m, 4, sam())).collect();
        // Alex is busy in Q7; Jo is free but already has two to come.
        assignments.push(to(7, 1, named(2, "Alex")));
        assignments.push(to(4, 1, named(3, "Jo")));
        assignments.push(to(5, 1, named(3, "Jo")));
        let relief = [(scout(2), "Alex".into()), (scout(3), "Jo".into())];

        let suggestions = rotation(&schedule, &assignments, &relief);
        assert_eq!(
            suggestions,
            [Suggestion {
                match_key: "2026mabil_qm7".into(),
                team_number: 4,
                from: scout(1),
                from_name: "Sam".into(),
                to: Some((scout(3), "Jo".into())),
                reason: Reason::LongRun {
                    first: "2026mabil_qm1".into(),
                    last: "2026mabil_qm8".into(),
                    len: 8,
                },
            }]
        );
    }

    #[test]
    fn a_long_run_in_the_past_is_history() {
        let schedule = season(8, 1);
        let assignments: Vec<_> = (1..=8).map(|m| to(m, 4, sam())).collect();
        assert!(rotation(&schedule, &assignments, &[(scout(2), "Alex".into())]).is_empty());
    }

    #[test]
    fn a_long_run_with_nobody_free_is_still_reported() {
        let schedule = season(0, 7);
        let assignments: Vec<_> = (1..=7).map(|m| to(m, 4, sam())).collect();
        let suggestions = rotation(&schedule, &assignments, &[]);
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].match_key, "2026mabil_qm7");
        assert_eq!(suggestions[0].to, None);
    }

    #[test]
    fn a_heavy_scout_hands_a_robot_to_a_light_one() {
        // Sam: Q1, Q3, Q5, Q7 played, Q9 to come. Alex: nothing.
        let schedule = season(8, 2);
        let assignments: Vec<_> = [1, 3, 5, 7, 9].map(|m| to(m, 2, sam())).into();
        let relief = [(scout(2), "Alex".into())];
        let suggestions = rotation(&schedule, &assignments, &relief);
        assert_eq!(suggestions.len(), 1);
        assert_eq!(
            (suggestions[0].match_key.as_str(), &suggestions[0].reason),
            (
                "2026mabil_qm9",
                &Reason::Uneven {
                    from_load: 5,
                    to_load: 0
                }
            )
        );

        // Within the margin, nothing.
        let close: Vec<_> = [7, 9].map(|m| to(m, 2, sam())).into();
        assert!(rotation(&schedule, &close, &relief).is_empty());
    }

    #[test]
    fn two_suggestions_never_lean_on_one_person_in_one_match() {
        let schedule = season(0, 4);
        let mut assignments: Vec<_> = (1..=4).map(|m| to(m, 1, sam())).collect();
        assignments.extend((1..=4).map(|m| to(m, 2, named(2, "Alex"))));
        let relief = [(scout(3), "Jo".into())];
        let suggestions = rotation(&schedule, &assignments, &relief);
        assert_eq!(suggestions.len(), 2);
        assert_ne!(suggestions[0].match_key, suggestions[1].match_key);
        assert!(
            suggestions
                .iter()
                .all(|s| s.to == Some((scout(3), "Jo".into())))
        );
    }

    #[test]
    fn tablets_are_not_rotated() {
        let schedule = season(0, 8);
        let assignments: Vec<_> = (1..=8).map(|m| to(m, 4, tablet(9))).collect();
        assert!(rotation(&schedule, &assignments, &[(scout(2), "Alex".into())]).is_empty());
    }

    #[test]
    fn doubling_up_in_one_match_is_found() {
        let d = AssigneeKey::Device(1);
        assert_eq!(doubled(&[scout(1), d, scout(1), scout(1)]), [scout(1)]);
        assert!(doubled(&[scout(1), d, scout(2)]).is_empty());
    }

    fn to(match_number: i32, team_number: i32, assignee: Assignee) -> Assignment {
        Assignment {
            match_key: format!("2026mabil_qm{match_number}"),
            team_number,
            assignee,
        }
    }

    fn tablet(id: i64) -> Assignee {
        Assignee::Device {
            id,
            name: "Stands Left".into(),
        }
    }

    fn sam() -> Assignee {
        Assignee::Scout {
            id: 1,
            name: "Sam".into(),
        }
    }

    fn key(match_number: i32, team: i32) -> (String, i32) {
        (format!("2026mabil_qm{match_number}"), team)
    }

    /// Q1 played; Q2-Q4 to come.
    fn schedule() -> Vec<MatchRecord> {
        let mut q1 = scheduled(1, [1, 2, 3], [4, 5, 6]);
        q1.played = true;
        vec![
            q1,
            scheduled(2, [1, 2, 3], [4, 5, 6]),
            scheduled(3, [1, 2, 3], [4, 5, 6]),
            scheduled(4, [1, 2, 3], [4, 5, 6]),
        ]
    }

    #[test]
    fn the_next_duty_is_the_first_unplayed_unrecorded_assignment() {
        let assignments = [to(4, 5, sam()), to(3, 2, sam()), to(2, 6, tablet(9))];
        let agenda = agenda(&schedule(), &assignments, &[scout(1)], &[]);
        assert_eq!(
            agenda.next,
            Some(key(3, 2)),
            "Q3 before Q4; Q2 is someone else's tablet"
        );

        assert_eq!(agenda.upcoming, [key(3, 2), key(4, 5)], "the whole list");

        let agenda = super::agenda(&schedule(), &assignments, &[scout(1)], &[key(3, 2)]);
        assert_eq!(agenda.next, Some(key(4, 5)), "once Q3 is recorded, Q4");
        assert_eq!(agenda.upcoming, [key(4, 5)]);
    }

    #[test]
    fn an_assignment_to_the_tablet_counts_whoever_is_signed_in() {
        let assignments = [to(2, 6, tablet(9))];
        let me = [scout(7), AssigneeKey::Device(9)];
        assert_eq!(
            agenda(&schedule(), &assignments, &me, &[]).next,
            Some(key(2, 6))
        );
    }

    #[test]
    fn a_played_match_not_recorded_is_missed_not_next() {
        let assignments = [to(1, 4, sam()), to(2, 1, sam())];
        let agenda = agenda(&schedule(), &assignments, &[scout(1)], &[]);
        assert_eq!(agenda.next, Some(key(2, 1)));
        assert_eq!(agenda.missed, [key(1, 4)]);
        assert_eq!(agenda.upcoming, [key(2, 1)], "missed is not upcoming");
        assert_eq!(agenda.open_in("2026mabil_qm1"), Some(4));
        assert_eq!(agenda.open_in("2026mabil_qm3"), None);

        let done = super::agenda(&schedule(), &assignments, &[scout(1)], &[key(1, 4)]);
        assert!(done.missed.is_empty());
    }

    #[test]
    fn a_stale_assignment_is_never_a_duty() {
        let agenda = agenda(&schedule(), &[to(2, 99, sam())], &[scout(1)], &[]);
        assert_eq!(agenda, Agenda::default());
    }

    fn seen(match_number: i32, team: i32, scouter: Option<i64>, device: Option<i64>) -> Sighting {
        Sighting {
            match_key: format!("2026mabil_qm{match_number}"),
            team_number: team,
            scouter_id: scouter,
            device_id: device,
        }
    }

    #[test]
    fn a_slot_is_recorded_by_anyone_missed_only_by_everyone() {
        let s = schedule();
        let (q1, q2) = (&s[0], &s[1]);
        let sightings = [seen(1, 1, Some(5), None), seen(1, 1, Some(6), None)];

        assert_eq!(
            slot_state(q1, Some(1), true, &sightings),
            SlotState::Recorded(2)
        );
        assert_eq!(slot_state(q1, Some(2), true, &sightings), SlotState::Missed);
        assert_eq!(
            slot_state(q1, Some(3), false, &sightings),
            SlotState::Unscouted
        );
        assert_eq!(
            slot_state(q2, Some(1), true, &sightings),
            SlotState::Assigned
        );
        assert_eq!(slot_state(q2, Some(1), false, &sightings), SlotState::Open);
        assert_eq!(slot_state(q2, None, false, &sightings), SlotState::Empty);
    }

    #[test]
    fn tallies_count_what_each_assignee_did_themselves() {
        let assignments = [
            to(1, 1, sam()),
            to(1, 2, tablet(9)),
            to(1, 3, sam()),
            to(2, 4, sam()),
            // Stale: 99 is not in Q2.
            to(2, 99, sam()),
        ];
        let sightings = [
            seen(1, 1, Some(1), None),
            // The tablet's robot, saved from the tablet by whoever held it.
            seen(1, 2, Some(7), Some(9)),
            // Sam's robot, recorded by somebody else: covered, but not Sam's.
            seen(1, 3, Some(8), None),
        ];

        let tallies = tallies(&schedule(), &assignments, &sightings);
        assert_eq!(tallies.len(), 2);
        assert_eq!(tallies[0].name, "Sam");
        assert_eq!(
            (tallies[0].recorded, tallies[0].missed, tallies[0].to_come),
            (1, 1, 1)
        );
        assert_eq!(tallies[1].assignee, AssigneeKey::Device(9));
        assert_eq!((tallies[1].recorded, tallies[1].missed), (1, 0));
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
