//! Match identity and scheduling status.
//!
//! Both of these were bugs in the retired implementation: status classification
//! lived inline in a handler where it could not be tested, and match labels were
//! formatted in three different places that disagreed about playoff rounds.

use chrono::{DateTime, TimeDelta, Utc};

/// How close a match is to happening, relative to a supplied instant.
///
/// The retired implementation used a +/-15 minute window around the scheduled
/// start (REBUILD_SPEC.md 5.7). That window is preserved: FRC schedules slip
/// constantly, and anything tighter flickers between states all afternoon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchStatus {
    /// Finished more than [`CURRENT_WINDOW`] ago.
    Completed,
    /// Within [`CURRENT_WINDOW`] of its scheduled start, either side.
    Current,
    /// More than [`CURRENT_WINDOW`] away.
    Upcoming,
    /// No scheduled time is known yet.
    Unscheduled,
}

/// How far either side of the scheduled start a match counts as "current".
pub const CURRENT_WINDOW: TimeDelta = TimeDelta::minutes(15);

/// Classify a match by its scheduled start.
///
/// `now` is a parameter rather than a call to `Utc::now()` so this crate stays
/// clock-free and wasm-clean -- and so the boundaries are testable.
pub fn classify(scheduled: Option<DateTime<Utc>>, now: DateTime<Utc>) -> MatchStatus {
    let Some(scheduled) = scheduled else {
        return MatchStatus::Unscheduled;
    };

    let delta = scheduled - now;
    if delta < -CURRENT_WINDOW {
        MatchStatus::Completed
    } else if delta <= CURRENT_WINDOW {
        MatchStatus::Current
    } else {
        MatchStatus::Upcoming
    }
}

/// Tournament round. Serialized as the TBA `comp_level` string.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum CompLevel {
    #[serde(rename = "qm")]
    Qualification,
    #[serde(rename = "sf")]
    Semifinal,
    #[serde(rename = "f")]
    Final,
}

impl CompLevel {
    /// Parse a TBA `comp_level`. TBA omits it on some historical events, and the
    /// retired sync defaulted a blank value to qualification -- keep that.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "qm" | "" => Some(Self::Qualification),
            "sf" => Some(Self::Semifinal),
            "f" => Some(Self::Final),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Qualification => "qm",
            Self::Semifinal => "sf",
            Self::Final => "f",
        }
    }

    /// Short human label, e.g. `Q42`, `SF3`, `F1`.
    ///
    /// A playoff match is numbered by its set: in the double-elimination
    /// bracket every `sf` match is its own set, and TBA's `match_number` is 1
    /// for all thirteen (`2026mabil_sf5m1`). Labelled by `match_number` they
    /// were all `SF1`. A second match in a set -- a replay, or a best-of-three
    /// from before 2023 -- reads `SF5-2`. The finals are one set of up to
    /// three, so those are numbered by match.
    pub fn label(self, set_number: i32, match_number: i32) -> String {
        match self {
            Self::Qualification => format!("Q{match_number}"),
            Self::Semifinal if match_number > 1 => format!("SF{set_number}-{match_number}"),
            Self::Semifinal => format!("SF{set_number}"),
            Self::Final => format!("F{match_number}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(hour: u32, minute: u32) -> DateTime<Utc> {
        use chrono::TimeZone;
        Utc.with_ymd_and_hms(2026, 3, 14, hour, minute, 0).unwrap()
    }

    #[test]
    fn unscheduled_when_no_start_time() {
        assert_eq!(classify(None, at(12, 0)), MatchStatus::Unscheduled);
    }

    #[test]
    fn current_at_exactly_the_scheduled_time() {
        assert_eq!(classify(Some(at(12, 0)), at(12, 0)), MatchStatus::Current);
    }

    #[test]
    fn window_boundaries_are_inclusive_on_both_sides() {
        // Exactly 15 minutes early and exactly 15 minutes late are both current.
        assert_eq!(classify(Some(at(12, 15)), at(12, 0)), MatchStatus::Current);
        assert_eq!(classify(Some(at(11, 45)), at(12, 0)), MatchStatus::Current);
    }

    #[test]
    fn one_minute_past_each_boundary_leaves_current() {
        assert_eq!(classify(Some(at(12, 16)), at(12, 0)), MatchStatus::Upcoming);
        assert_eq!(
            classify(Some(at(11, 44)), at(12, 0)),
            MatchStatus::Completed
        );
    }

    #[test]
    fn blank_comp_level_is_qualification() {
        // TBA omits comp_level on some events; the retired sync defaulted to qm.
        assert_eq!(CompLevel::parse(""), Some(CompLevel::Qualification));
        assert_eq!(CompLevel::parse("  "), Some(CompLevel::Qualification));
    }

    #[test]
    fn comp_level_parse_is_case_insensitive_and_rejects_junk() {
        assert_eq!(CompLevel::parse("QM"), Some(CompLevel::Qualification));
        assert_eq!(CompLevel::parse("SF"), Some(CompLevel::Semifinal));
        assert_eq!(CompLevel::parse("qf"), None);
    }

    #[test]
    fn labels_match_the_scouting_vocabulary() {
        assert_eq!(CompLevel::Qualification.label(1, 42), "Q42");
        assert_eq!(CompLevel::Semifinal.label(3, 1), "SF3");
        assert_eq!(CompLevel::Final.label(1, 1), "F1");
    }

    #[test]
    fn every_double_elimination_match_has_its_own_label() {
        // sf1m1 .. sf13m1: the set is the match.
        let labels: Vec<String> = (1..=13)
            .map(|set| CompLevel::Semifinal.label(set, 1))
            .collect();
        assert_eq!(labels[0], "SF1");
        assert_eq!(labels[12], "SF13");
        let mut unique = labels.clone();
        unique.dedup();
        assert_eq!(unique.len(), 13, "{labels:?}");
    }

    #[test]
    fn a_second_match_in_a_set_says_so_and_the_finals_count_by_match() {
        assert_eq!(CompLevel::Semifinal.label(5, 2), "SF5-2", "a replay");
        assert_eq!(
            CompLevel::Semifinal.label(1, 3),
            "SF1-3",
            "pre-2023 best of three"
        );
        assert_eq!(CompLevel::Final.label(1, 2), "F2");
        assert_eq!(CompLevel::Final.label(1, 3), "F3");
    }

    #[test]
    fn a_qualification_ignores_its_set() {
        // TBA sends set_number 1 for quals; anything else must not leak in.
        assert_eq!(CompLevel::Qualification.label(7, 12), "Q12");
    }

    #[test]
    fn comp_level_round_trips_through_its_string() {
        for level in [
            CompLevel::Qualification,
            CompLevel::Semifinal,
            CompLevel::Final,
        ] {
            assert_eq!(CompLevel::parse(level.as_str()), Some(level));
        }
    }
}
