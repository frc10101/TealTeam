//! The drive coach's view of the schedule (U18).
//!
//! The retired coach panel fetched the FIRST schedule live, so at an event with
//! no internet -- exactly where a coach needs it -- it showed nothing
//! (REBUILD_SPEC.md 12.6). Here it reads the local `matches` table the
//! background sync fills.
//!
//! **The schedule, not the clock, says what has been played.** FRC events run
//! late; judged by the clock alone (`matches::classify`), a match twenty
//! minutes behind reads "Completed" before anyone has driven it. So a match is
//! played when the results feed says so, the next match is the first one that
//! is not, and the clock only describes how far off that is.

use chrono::{DateTime, TimeDelta, Utc};

use crate::matches::{CURRENT_WINDOW, MatchStatus, classify};
use crate::records::MatchRecord;

/// Where one of the coach's matches stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standing {
    /// The results feed has it.
    Played,
    /// The first of the team's matches not played yet.
    Next,
    /// Every unplayed match after that.
    Later,
}

/// The team's matches in playing order, each with where it stands.
pub fn schedule(matches: &[MatchRecord], team: i32) -> Vec<(&MatchRecord, Standing)> {
    let mut next_found = false;
    matches
        .iter()
        .filter(|m| m.alliance_of(team).is_some())
        .map(|m| {
            let standing = if m.played {
                Standing::Played
            } else if !next_found {
                next_found = true;
                Standing::Next
            } else {
                Standing::Later
            };
            (m, standing)
        })
        .collect()
}

/// How far off an unplayed match is, in words: `"in 12 min"`, `"due now"`,
/// `"running 22 min late"`, or `"time not published"`. Relative, so it needs
/// no timezone (docs/reference/TIMEZONE_HANDLING.md).
pub fn timing(scheduled: Option<DateTime<Utc>>, now: DateTime<Utc>) -> String {
    let Some(at) = scheduled else {
        return "time not published".into();
    };
    match classify(Some(at), now) {
        MatchStatus::Upcoming => format!("in {}", span(at - now)),
        MatchStatus::Current if at > now => format!("in {}", span(at - now)),
        MatchStatus::Current => "due now".into(),
        // The clock says it is past; the feed says it has not been played.
        MatchStatus::Completed => format!("running {} late", span(now - at)),
        MatchStatus::Unscheduled => "time not published".into(),
    }
}

/// `"12 min"`, `"1 h 5 min"`, `"3 h"`.
fn span(delta: TimeDelta) -> String {
    let minutes = delta.num_minutes().max(1);
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

/// Whether the ±15-minute window says a scheduled match should be on the
/// field about now. Only ever used to describe, never to mark it played.
pub fn is_due(scheduled: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
    scheduled.is_some_and(|at| (at - now).abs() <= CURRENT_WINDOW)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matches::CompLevel;
    use chrono::TimeZone;

    fn at(hour: u32, minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 3, 14, hour, minute, 0).unwrap()
    }

    fn scheduled(number: i32, red: [i32; 3], played: bool) -> MatchRecord {
        MatchRecord {
            key: format!("2026mabil_qm{number}"),
            event_key: "2026mabil".into(),
            comp_level: CompLevel::Qualification,
            set_number: 1,
            match_number: number,
            red: red.map(Some),
            blue: [Some(4), Some(5), Some(6)],
            red_score: None,
            blue_score: None,
            winner: None,
            played,
            scheduled_at: Some(at(12, number as u32)),
            actual_at: None,
        }
    }

    #[test]
    fn the_next_match_is_the_first_unplayed_whatever_the_clock_says() {
        let matches = [
            scheduled(1, [10101, 2, 3], true),
            scheduled(2, [7, 8, 9], false),
            scheduled(3, [10101, 2, 3], false),
            scheduled(4, [1, 10101, 3], false),
        ];
        let standings: Vec<(i32, Standing)> = schedule(&matches, 10101)
            .into_iter()
            .map(|(m, s)| (m.match_number, s))
            .collect();
        assert_eq!(
            standings,
            [
                (1, Standing::Played),
                (3, Standing::Next),
                (4, Standing::Later)
            ]
        );
        assert!(schedule(&matches, 9999).is_empty());
    }

    #[test]
    fn timing_is_relative_and_a_late_match_is_late_not_done() {
        assert_eq!(timing(Some(at(12, 40)), at(12, 0)), "in 40 min");
        assert_eq!(timing(Some(at(14, 5)), at(12, 0)), "in 2 h 5 min");
        assert_eq!(
            timing(Some(at(12, 10)), at(12, 0)),
            "in 10 min",
            "inside the window, still ahead"
        );
        assert_eq!(timing(Some(at(11, 55)), at(12, 0)), "due now");
        assert_eq!(timing(Some(at(11, 38)), at(12, 0)), "running 22 min late");
        assert_eq!(timing(None, at(12, 0)), "time not published");
    }

    #[test]
    fn due_means_inside_the_window_either_side() {
        assert!(is_due(Some(at(12, 15)), at(12, 0)));
        assert!(is_due(Some(at(11, 45)), at(12, 0)));
        assert!(!is_due(Some(at(12, 16)), at(12, 0)));
        assert!(!is_due(None, at(12, 0)));
    }
}
