//! The competition records: events, teams, matches, and derived statistics.
//!
//! Plain data. These mirror the schema closely because they are what crosses the
//! [`Repo`](tt_repo) boundary, but they are not tied to any storage engine --
//! which is what lets the same structs come out of SQLite on a Pi and out of
//! SQLite-WASM in a browser tab.

use chrono::{DateTime, Datelike, NaiveDate, Utc};
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
    /// The event's zone. `None` without one, or for a name stored before Q5
    /// mapped FIRST's Windows names; the next sync replaces those.
    pub fn zone(&self) -> Option<chrono_tz::Tz> {
        self.timezone.as_deref()?.parse().ok()
    }

    /// The date at the event at `now`: its own, not UTC's, so a Saturday
    /// evening in Chicago is still Saturday.
    pub fn today(&self, now: DateTime<Utc>) -> NaiveDate {
        crate::timezone::local_date(self.zone(), now)
    }

    /// Whether the event is on at `now`, by its own calendar.
    pub fn is_running(&self, now: DateTime<Utc>) -> bool {
        self.is_active_on(self.today(now))
    }

    /// `"1:30 PM CDT"`: `at` on the event's clock.
    pub fn clock_time(&self, at: DateTime<Utc>) -> String {
        crate::timezone::clock_time(self.zone(), at)
    }

    /// `"Sat 1:30 PM CDT"`: `at` on the event's clock and calendar.
    pub fn day_and_time(&self, at: DateTime<Utc>) -> String {
        crate::timezone::day_and_time(self.zone(), at)
    }

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

    /// `"Mar 12–15"`, `"Mar 30–Apr 2"`, or `"Mar 12"` for a one-day event.
    /// `None` without a start date. No year: the caller adds one where it helps.
    ///
    /// Assembled by hand because chrono's formatter needs its `alloc` feature,
    /// which this crate does not otherwise enable.
    pub fn date_range(&self) -> Option<String> {
        const MONTHS: [&str; 12] = [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ];
        let short = |d: NaiveDate| format!("{} {}", MONTHS[d.month0() as usize], d.day());

        let start = self.start_date?;
        Some(match self.end_date.filter(|end| *end > start) {
            None => short(start),
            Some(end) if (end.year(), end.month()) == (start.year(), start.month()) => {
                format!("{}–{}", short(start), end.day())
            }
            Some(end) => format!("{}–{}", short(start), short(end)),
        })
    }

    /// [`date_range`](Self::date_range) plus the year, for where the event is
    /// shown on its own: `"Mar 12–15, 2026"`.
    pub fn date_range_with_year(&self) -> Option<String> {
        Some(format!(
            "{}, {}",
            self.date_range()?,
            self.start_date?.year()
        ))
    }
}

/// The event a page shows when its URL names none (U2).
///
/// The one running at `now`; else the next to start; else the most recent to
/// finish; else the first listed. Each by its own calendar, so the finals on a
/// US event's last evening do not lose to the next event on UTC's date. So a lead scout opening the app at an event
/// lands on that event, and in the off-season on the one coming up -- with no
/// stored preference, which is what keeps the choice bookmarkable, per-tab, and
/// available offline.
pub fn default_event(events: &[Event], now: DateTime<Utc>) -> Option<&Event> {
    events
        .iter()
        .find(|e| e.is_running(now))
        .or_else(|| {
            events
                .iter()
                .filter(|e| e.start_date.is_some_and(|start| start > e.today(now)))
                .min_by_key(|e| e.start_date)
        })
        .or_else(|| {
            events
                .iter()
                .filter(|e| e.end_date.is_some_and(|end| end < e.today(now)))
                .max_by_key(|e| e.end_date)
        })
        .or_else(|| events.first())
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
        self.comp_level.label(self.set_number, self.match_number)
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

    /// Midday UTC: the same date in every zone the test events use.
    fn noon(y: i32, m: u32, d: u32) -> DateTime<Utc> {
        date(y, m, d).and_hms_opt(12, 0, 0).unwrap().and_utc()
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

    fn dated(key: &str, start: Option<NaiveDate>, end: Option<NaiveDate>) -> Event {
        Event {
            key: key.into(),
            start_date: start,
            end_date: end,
            ..event()
        }
    }

    #[test]
    fn date_ranges_read_the_way_a_schedule_is_printed() {
        let range = |start, end| dated("x", start, end).date_range();
        let (mar12, mar15) = (Some(date(2026, 3, 12)), Some(date(2026, 3, 15)));
        assert_eq!(range(mar12, mar15).as_deref(), Some("Mar 12–15"));
        assert_eq!(
            range(Some(date(2026, 3, 30)), Some(date(2026, 4, 2))).as_deref(),
            Some("Mar 30–Apr 2")
        );
        assert_eq!(range(mar12, mar12).as_deref(), Some("Mar 12"), "one day");
        assert_eq!(range(mar12, None).as_deref(), Some("Mar 12"));
        // An end before the start is bad data, not a range backwards in time.
        assert_eq!(range(mar15, mar12).as_deref(), Some("Mar 15"));
        assert_eq!(range(None, mar15), None);
        assert_eq!(
            dated("x", mar12, mar15).date_range_with_year().as_deref(),
            Some("Mar 12–15, 2026")
        );
    }

    #[test]
    fn the_default_event_is_the_one_running_today() {
        let events = [
            dated("past", Some(date(2026, 3, 1)), Some(date(2026, 3, 3))),
            dated("now", Some(date(2026, 3, 12)), Some(date(2026, 3, 15))),
            dated("next", Some(date(2026, 3, 26)), Some(date(2026, 3, 29))),
        ];
        let pick = |day| default_event(&events, noon(2026, 3, day)).map(|e| e.key.as_str());

        assert_eq!(pick(12), Some("now"), "its first day");
        assert_eq!(pick(15), Some("now"), "its last day");
        assert_eq!(pick(20), Some("next"), "between events, the one coming up");
    }

    #[test]
    fn after_the_season_the_default_is_the_most_recent_event() {
        let events = [
            dated("early", Some(date(2026, 3, 1)), Some(date(2026, 3, 3))),
            dated("late", Some(date(2026, 4, 1)), Some(date(2026, 4, 3))),
        ];
        let chosen = default_event(&events, noon(2026, 6, 1)).map(|e| e.key.as_str());
        assert_eq!(chosen, Some("late"));
    }

    #[test]
    fn a_us_events_last_evening_is_still_running_on_its_own_calendar() {
        // Magnolia ends Saturday 21 March. 8 PM in Laurel is 1 AM Sunday UTC.
        let magnolia = Event {
            timezone: Some("America/Chicago".into()),
            ..dated("2026mslr", Some(date(2026, 3, 18)), Some(date(2026, 3, 21)))
        };
        let next = dated("next", Some(date(2026, 3, 22)), Some(date(2026, 3, 22)));
        let saturday_evening = date(2026, 3, 22).and_hms_opt(1, 0, 0).unwrap().and_utc();
        assert!(magnolia.is_running(saturday_evening));
        assert_eq!(magnolia.clock_time(saturday_evening), "8:00 PM CDT");
        let events = [magnolia, next];
        assert_eq!(
            default_event(&events, saturday_evening).map(|e| e.key.as_str()),
            Some("2026mslr"),
            "not the event starting on UTC's Sunday"
        );
    }

    #[test]
    fn undated_events_are_a_last_resort_and_nothing_is_nothing() {
        let events = [dated("undated", None, None)];
        let chosen = default_event(&events, noon(2026, 3, 1)).map(|e| e.key.as_str());
        assert_eq!(chosen, Some("undated"));
        assert!(default_event(&[], noon(2026, 3, 1)).is_none());
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
