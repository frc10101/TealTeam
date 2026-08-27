//! The competition records: events, teams, matches, and derived statistics.
//!
//! Plain data. These mirror the schema closely because they are what crosses the
//! [`Repo`](tt_repo) boundary, but they are not tied to any storage engine --
//! which is what lets the same structs come out of SQLite on a Pi and out of
//! SQLite-WASM in a browser tab.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::matches::CompLevel;

/// A competition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    /// `"2026mabil"`. The natural key: stable, meaningful, and agreed on by both
    /// upstream APIs.
    pub key: String,
    pub name: String,
    pub location: Option<String>,
    /// IANA identifier. Match times are rendered in the event's zone, never the
    /// server's — see docs/TIMEZONE_HANDLING.md.
    pub timezone: Option<String>,
    pub start_date: Option<NaiveDate>,
    pub end_date: Option<NaiveDate>,
    pub event_code: Option<String>,
    pub event_type: Option<String>,
    pub district_key: Option<String>,
    pub week: Option<i32>,
}

impl Event {
    /// Whether `date` falls within the event, inclusive of both ends.
    pub fn is_active_on(&self, date: NaiveDate) -> bool {
        match (self.start_date, self.end_date) {
            (Some(start), Some(end)) => start <= date && date <= end,
            _ => false,
        }
    }

    /// Days until the event starts. Negative once it has begun.
    pub fn days_until(&self, date: NaiveDate) -> Option<i64> {
        self.start_date.map(|start| (start - date).num_days())
    }
}

/// An FRC team.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Team {
    pub number: i32,
    pub name: String,
    pub nickname: Option<String>,
    pub school: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub country: Option<String>,
    pub rookie_year: Option<i32>,
    pub website: Option<String>,
}

impl Team {
    /// `"Boston, MA · USA"`, skipping whatever is missing.
    pub fn location_line(&self) -> String {
        let local = [self.city.as_deref(), self.state.as_deref()]
            .into_iter()
            .flatten()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(", ");

        match self
            .country
            .as_deref()
            .map(str::trim)
            .filter(|c| !c.is_empty())
        {
            Some(country) if local.is_empty() => country.to_string(),
            Some(country) => format!("{local} · {country}"),
            None => local,
        }
    }
}

/// A scheduled or played match.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatchRecord {
    /// `"2026mabil_qm14"`.
    pub key: String,
    pub event_key: String,
    pub comp_level: CompLevel,
    pub set_number: i32,
    pub match_number: i32,

    /// Team numbers by slot. `None` where the schedule has a gap.
    pub red: [Option<i32>; 3],
    pub blue: [Option<i32>; 3],

    pub red_score: Option<i64>,
    pub blue_score: Option<i64>,
    /// `"red"`, `"blue"`, or `None` for a tie or unplayed match.
    pub winner: Option<String>,
    pub played: bool,

    pub scheduled_at: Option<DateTime<Utc>>,
    pub actual_at: Option<DateTime<Utc>>,
}

impl MatchRecord {
    /// `Q14`, `SF3`, `F1`.
    pub fn label(&self) -> String {
        self.comp_level.label(self.match_number)
    }

    /// Every robot in the match, red first, skipping empty slots.
    pub fn teams(&self) -> impl Iterator<Item = i32> + '_ {
        self.red
            .iter()
            .chain(self.blue.iter())
            .filter_map(|slot| *slot)
    }

    /// Which alliance a team is on, if it is in this match at all.
    pub fn alliance_of(&self, team_number: i32) -> Option<&'static str> {
        if self.red.contains(&Some(team_number)) {
            Some("red")
        } else if self.blue.contains(&Some(team_number)) {
            Some("blue")
        } else {
            None
        }
    }

    /// A team's alliance partners in this match.
    pub fn partners_of(&self, team_number: i32) -> Vec<i32> {
        let alliance = match self.alliance_of(team_number) {
            Some("red") => &self.red,
            Some("blue") => &self.blue,
            _ => return Vec::new(),
        };
        alliance
            .iter()
            .filter_map(|slot| *slot)
            .filter(|n| *n != team_number)
            .collect()
    }

    /// Whether a team is fully assigned — every slot the schedule declares has a
    /// team number. Used by the coverage view (L6).
    pub fn is_fully_scheduled(&self) -> bool {
        self.red.iter().chain(self.blue.iter()).all(|s| s.is_some())
    }
}

/// Upstream-derived performance numbers for one team at one event.
///
/// Everything is optional: TBA publishes rankings before OPRs, OPRs before
/// component OPRs, and some of it never for some events.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TeamEventStats {
    pub team_number: i32,
    pub event_key: String,

    pub opr: Option<f64>,
    pub dpr: Option<f64>,
    pub ccwm: Option<f64>,
    pub auto_opr: Option<f64>,
    pub teleop_opr: Option<f64>,
    pub endgame_opr: Option<f64>,

    pub rank: Option<i32>,
    pub matches_played: Option<i32>,
    pub qual_average: Option<f64>,
    pub avg_match_points: Option<f64>,
    pub wins: Option<i32>,
    pub losses: Option<i32>,
    pub ties: Option<i32>,
    pub dq_count: Option<i32>,
    pub qual_points: Option<i32>,
    pub elim_points: Option<i32>,
    pub award_points: Option<i32>,
    pub alliance_points: Option<i32>,
    pub total_points: Option<i32>,

    /// When this was pulled from upstream. Drives the freshness badges (I12).
    pub synced_at: Option<DateTime<Utc>>,
}

impl TeamEventStats {
    /// `"9W 3L"`, or `"9W 3L 1T"` when there were ties.
    pub fn record_line(&self) -> String {
        let (w, l, t) = (
            self.wins.unwrap_or(0),
            self.losses.unwrap_or(0),
            self.ties.unwrap_or(0),
        );
        if t > 0 {
            format!("{w}W {l}L {t}T")
        } else {
            format!("{w}W {l}L")
        }
    }

    /// Whether anything at all was synced, so the UI can distinguish "no data
    /// yet" from "a genuine zero".
    pub fn has_any_data(&self) -> bool {
        self.rank.is_some() || self.opr.is_some() || self.matches_played.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn event() -> Event {
        Event {
            key: "2026mabil".into(),
            name: "Greater Boston".into(),
            location: None,
            timezone: Some("America/New_York".into()),
            start_date: Some(date(2026, 3, 12)),
            end_date: Some(date(2026, 3, 15)),
            event_code: Some("mabil".into()),
            event_type: None,
            district_key: None,
            week: Some(3),
        }
    }

    fn a_match() -> MatchRecord {
        MatchRecord {
            key: "2026mabil_qm14".into(),
            event_key: "2026mabil".into(),
            comp_level: CompLevel::Qualification,
            set_number: 1,
            match_number: 14,
            red: [Some(10101), Some(254), Some(1)],
            blue: [Some(2), Some(3), Some(4)],
            red_score: Some(88),
            blue_score: Some(74),
            winner: Some("red".into()),
            played: true,
            scheduled_at: None,
            actual_at: None,
        }
    }

    #[test]
    fn an_event_is_active_on_both_of_its_end_days() {
        let e = event();
        assert!(e.is_active_on(date(2026, 3, 12)));
        assert!(e.is_active_on(date(2026, 3, 15)));
        assert!(!e.is_active_on(date(2026, 3, 11)));
        assert!(!e.is_active_on(date(2026, 3, 16)));
    }

    #[test]
    fn an_event_without_dates_is_never_active() {
        let e = Event {
            start_date: None,
            ..event()
        };
        assert!(!e.is_active_on(date(2026, 3, 13)));
    }

    #[test]
    fn days_until_goes_negative_once_it_has_started() {
        assert_eq!(event().days_until(date(2026, 3, 10)), Some(2));
        assert_eq!(event().days_until(date(2026, 3, 13)), Some(-1));
    }

    #[test]
    fn match_labels_use_the_scouting_vocabulary() {
        assert_eq!(a_match().label(), "Q14");
    }

    #[test]
    fn a_match_lists_its_robots_red_first() {
        assert_eq!(
            a_match().teams().collect::<Vec<_>>(),
            vec![10101, 254, 1, 2, 3, 4]
        );
    }

    #[test]
    fn empty_alliance_slots_are_skipped_not_reported_as_zero() {
        let m = MatchRecord {
            red: [Some(1), None, None],
            ..a_match()
        };
        assert_eq!(m.teams().collect::<Vec<_>>(), vec![1, 2, 3, 4]);
        assert!(!m.is_fully_scheduled());
    }

    #[test]
    fn alliance_lookup_finds_a_team_on_either_side() {
        let m = a_match();
        assert_eq!(m.alliance_of(10101), Some("red"));
        assert_eq!(m.alliance_of(3), Some("blue"));
        assert_eq!(m.alliance_of(9999), None);
    }

    #[test]
    fn partners_exclude_the_team_itself() {
        assert_eq!(a_match().partners_of(10101), vec![254, 1]);
        assert_eq!(a_match().partners_of(3), vec![2, 4]);
    }

    #[test]
    fn a_team_not_in_the_match_has_no_partners() {
        assert!(a_match().partners_of(9999).is_empty());
    }

    #[test]
    fn a_record_line_only_mentions_ties_when_there_were_some() {
        let s = TeamEventStats {
            wins: Some(9),
            losses: Some(3),
            ..Default::default()
        };
        assert_eq!(s.record_line(), "9W 3L");
        let s = TeamEventStats { ties: Some(1), ..s };
        assert_eq!(s.record_line(), "9W 3L 1T");
    }

    #[test]
    fn empty_stats_are_distinguishable_from_a_genuine_zero() {
        assert!(!TeamEventStats::default().has_any_data());
        let ranked = TeamEventStats {
            rank: Some(1),
            ..Default::default()
        };
        assert!(ranked.has_any_data());
        // A team that played zero matches but has a row still counts as data.
        let zeroed = TeamEventStats {
            matches_played: Some(0),
            ..Default::default()
        };
        assert!(zeroed.has_any_data());
    }

    #[test]
    fn a_team_location_line_degrades_gracefully() {
        let mut t = Team {
            number: 10101,
            name: "Teal Team".into(),
            nickname: None,
            school: None,
            city: Some("Boston".into()),
            state: Some("MA".into()),
            country: Some("USA".into()),
            rookie_year: None,
            website: None,
        };
        assert_eq!(t.location_line(), "Boston, MA · USA");

        t.state = None;
        assert_eq!(t.location_line(), "Boston · USA");

        t.city = None;
        assert_eq!(t.location_line(), "USA");

        t.country = None;
        assert_eq!(t.location_line(), "");
    }
}
