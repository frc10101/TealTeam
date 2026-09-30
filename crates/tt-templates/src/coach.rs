//! The drive coach panel (U18): the coach's team's matches, from the local
//! schedule, with who is on each side and how strong they look.

use askama::Template;
use chrono::{DateTime, Utc};
use tt_core::coach::{self, Standing};
use tt_core::records::{MatchRecord, TeamEventStats};

use crate::{Nav, TeamMatchLine, team_href};

#[derive(Template)]
#[template(path = "pages/drive_coach.html")]
pub struct DriveCoachPage {
    pub title: String,
    pub nav: Nav,
    pub event_name: String,
    /// The team whose schedule this is; empty when the coach has none.
    pub team: String,
    /// Why there is no schedule, when there is not.
    pub unavailable: String,
    pub errors: Vec<String>,
    pub next: Option<CoachCard>,
    pub later: Vec<CoachCard>,
    pub played: Vec<CoachCard>,
    /// This page, which the schedule refreshes itself from.
    pub live_href: String,
}

/// One match, from the coach's side.
#[derive(Debug, Clone)]
pub struct CoachCard {
    /// The match key, as the element id.
    pub id: String,
    /// `"Q14"`.
    pub label: String,
    /// `"in 12 min"`, `"due now"`, `"running 22 min late"`; empty once played.
    pub timing: String,
    /// `"Won 88–72"`; empty until played.
    pub result: String,
    pub ours: AllianceView,
    pub theirs: AllianceView,
}

#[derive(Debug, Clone)]
pub struct AllianceView {
    /// `"red"` or `"blue"`.
    pub color: &'static str,
    pub teams: Vec<CoachTeam>,
    /// Summed OPR of the teams that have one, `"118.4"`; `"—"` when none do.
    pub opr_total: String,
    /// Some teams have no OPR yet, so the total undercounts.
    pub opr_partial: bool,
}

#[derive(Debug, Clone)]
pub struct CoachTeam {
    pub number: i32,
    pub href: String,
    /// `"41.3"`, or `"—"` when not synced.
    pub opr: String,
    pub dpr: String,
    /// The coach's own team.
    pub us: bool,
}

impl DriveCoachPage {
    /// Split `matches` (the event's, in playing order) into next, later, and
    /// played, from `team`'s side.
    pub fn schedule(
        &mut self,
        event_key: &str,
        matches: &[MatchRecord],
        team: i32,
        stats: &[TeamEventStats],
        now: DateTime<Utc>,
    ) {
        for (record, standing) in coach::schedule(matches, team) {
            let card = CoachCard::new(event_key, record, team, stats, standing, now);
            match standing {
                Standing::Next => self.next = Some(card),
                Standing::Later => self.later.push(card),
                // Most recent first: the last match is the one being talked about.
                Standing::Played => self.played.insert(0, card),
            }
        }
    }
}

impl CoachCard {
    fn new(
        event_key: &str,
        record: &MatchRecord,
        team: i32,
        stats: &[TeamEventStats],
        standing: Standing,
        now: DateTime<Utc>,
    ) -> Self {
        let line = TeamMatchLine::new(record, team);
        let (ours, theirs) = match record.alliance_of(team) {
            Some("blue") => (("blue", &record.blue), ("red", &record.red)),
            _ => (("red", &record.red), ("blue", &record.blue)),
        };
        let side = |(color, slots): (&'static str, &[Option<i32>; 3])| {
            let teams: Vec<CoachTeam> = slots
                .iter()
                .flatten()
                .map(|&number| {
                    let s = stats.iter().find(|s| s.team_number == number);
                    let one =
                        |v: Option<f64>| v.map(|v| format!("{v:.1}")).unwrap_or_else(|| "—".into());
                    CoachTeam {
                        number,
                        href: team_href(event_key, number),
                        opr: one(s.and_then(|s| s.opr)),
                        dpr: one(s.and_then(|s| s.dpr)),
                        us: number == team,
                    }
                })
                .collect();
            let oprs: Vec<f64> = slots
                .iter()
                .flatten()
                .filter_map(|n| stats.iter().find(|s| s.team_number == *n)?.opr)
                .collect();
            AllianceView {
                color,
                opr_total: if oprs.is_empty() {
                    "—".into()
                } else {
                    format!("{:.1}", oprs.iter().sum::<f64>())
                },
                opr_partial: !oprs.is_empty() && oprs.len() < teams.len(),
                teams,
            }
        };
        Self {
            id: record.key.clone(),
            label: record.label(),
            timing: if standing == Standing::Played {
                String::new()
            } else {
                coach::timing(record.scheduled_at, now)
            },
            result: line.map(|l| l.result).unwrap_or_default(),
            ours: side(ours),
            theirs: side(theirs),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use tt_core::matches::CompLevel;

    fn at(hour: u32, minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 3, 14, hour, minute, 0).unwrap()
    }

    fn scheduled(number: i32, played: bool) -> MatchRecord {
        MatchRecord {
            key: format!("2026mabil_qm{number}"),
            event_key: "2026mabil".into(),
            comp_level: CompLevel::Qualification,
            set_number: 1,
            match_number: number,
            red: [Some(2), Some(3), Some(4)],
            blue: [Some(10101), Some(254), None],
            red_score: played.then_some(60),
            blue_score: played.then_some(75),
            winner: None,
            played,
            scheduled_at: Some(at(12, 10 * number as u32)),
            actual_at: None,
        }
    }

    fn stats(team: i32, opr: f64) -> TeamEventStats {
        TeamEventStats {
            team_number: team,
            event_key: "2026mabil".into(),
            opr: Some(opr),
            dpr: Some(10.0),
            ..TeamEventStats::default()
        }
    }

    fn page() -> DriveCoachPage {
        DriveCoachPage {
            title: "Drive Coach".into(),
            nav: Nav::default(),
            event_name: "Boston".into(),
            team: "10101".into(),
            unavailable: String::new(),
            errors: Vec::new(),
            next: None,
            later: Vec::new(),
            played: Vec::new(),
            live_href: "/drive-coach?event=2026mabil".into(),
        }
    }

    #[test]
    fn the_schedule_splits_from_our_side_with_strength_on_each() {
        let matches = [scheduled(1, true), scheduled(2, false), scheduled(3, false)];
        let all = [stats(10101, 30.0), stats(254, 45.5), stats(2, 20.0)];
        let mut p = page();
        p.schedule("2026mabil", &matches, 10101, &all, at(12, 0));

        let next = p.next.as_ref().expect("Q2 is next");
        assert_eq!(
            (next.label.as_str(), next.timing.as_str()),
            ("Q2", "in 20 min")
        );
        assert_eq!(next.ours.color, "blue", "our side first, whatever colour");
        assert_eq!(next.ours.opr_total, "75.5");
        assert!(
            !next.ours.opr_partial,
            "the gap in blue 3 is no team, not a missing OPR"
        );
        assert!(next.ours.teams[0].us && !next.ours.teams[1].us);
        assert_eq!(next.theirs.opr_total, "20.0");
        assert!(next.theirs.opr_partial, "3 and 4 have no OPR yet");
        assert_eq!(next.theirs.teams[1].opr, "—");

        assert_eq!(p.later.len(), 1);
        assert_eq!(p.played[0].result, "Won 75–60");
        assert!(p.played[0].timing.is_empty());

        let html = p.render_html().expect("render");
        assert!(html.contains(r#"id="coach-schedule""#));
        assert!(html.contains(r#"href="/teams?event=2026mabil&#38;team=254""#));
    }

    use crate::Page;
}
