//! The team profile (U11): one view model for everything known about a team at
//! an event -- what the upstream feeds say, what scouts recorded, and its
//! matches.
//!
//! The retired page assembled this in three places that disagreed. Here it is
//! one struct, and an absent value is an em dash, never a zero: "no OPR synced
//! yet" and "an OPR of 0.0" are different facts (REBUILD_SPEC.md 5.4).

use askama::Template;
use tt_core::profile::{SectionSummary, Summary};
use tt_core::records::{Event, MatchRecord, Team, TeamEventStats};

use crate::{Nav, RosterEntry, scout_href};

#[derive(Template)]
#[template(path = "pages/team.html")]
pub struct TeamPage {
    pub title: String,
    pub nav: Nav,
    /// What is in the team-number box: the team shown, or what was typed.
    pub query: String,
    /// Type-ahead for the box: the selected event's roster.
    pub roster: Vec<RosterEntry>,
    pub errors: Vec<String>,
    pub team: Option<TeamCard>,
    /// The team at the selected event. `None` when it is not at it.
    pub at_event: Option<TeamAtEvent>,
    /// Why there is no event panel, when there is a team.
    pub not_at_event: String,
    /// The team's other events here, each a link to its profile there.
    pub other_events: Vec<EventLink>,
}

#[derive(Debug, Clone)]
pub struct TeamCard {
    pub number: i32,
    pub name: String,
    /// `"Boston, MA · USA"`; empty when unknown.
    pub location: String,
    pub rookie_year: String,
    pub website: String,
}

impl TeamCard {
    pub fn new(team: &Team) -> Self {
        Self {
            number: team.number,
            name: team.name.clone(),
            location: team.location_line(),
            rookie_year: team.rookie_year.map(|y| y.to_string()).unwrap_or_default(),
            // Only links that go somewhere a browser should follow.
            website: team
                .website
                .clone()
                .filter(|w| w.starts_with("http://") || w.starts_with("https://"))
                .unwrap_or_default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct EventLink {
    pub label: String,
    pub href: String,
}

/// Where a team profile lives: `/teams?event=…&team=…`.
pub fn team_href(event_key: &str, team: i32) -> String {
    format!("/teams?event={event_key}&team={team}")
}

impl EventLink {
    pub fn new(event: &Event, team: i32) -> Self {
        Self {
            label: match event.date_range() {
                Some(dates) => format!("{} · {dates}", event.name),
                None => event.name.clone(),
            },
            href: team_href(&event.key, team),
        }
    }
}

/// Everything about the team at one event.
#[derive(Debug, Clone)]
pub struct TeamAtEvent {
    pub event_name: String,
    /// Empty when nothing has been synced for this team here.
    pub stats: Vec<StatLine>,
    /// `"12 minutes ago"`; empty when never synced.
    pub synced: String,
    /// Synced long enough ago not to trust as live.
    pub stale: bool,
    /// Approved observations the summaries are drawn from.
    pub observed: usize,
    /// When the newest of them was recorded: `"12 minutes ago"` (U14).
    pub latest_scouted: String,
    /// Observations still waiting for review, not counted.
    pub waiting: usize,
    pub sections: Vec<SummarySection>,
    /// The notes the viewer's team wrote on these observations, and only
    /// those (U13).
    pub notes: Vec<NoteLine>,
    /// Whose notes are shown: the viewer's team. `None` when the viewer has
    /// no team, and so reads none.
    pub notes_team: Option<i32>,
    /// The notes view narrowed to this team (U22).
    pub notes_href: String,
    /// The graph view showing this team (U21).
    pub graph_href: String,
    pub matches: Vec<TeamMatchLine>,
}

/// One note, from one approved observation.
#[derive(Debug, Clone)]
pub struct NoteLine {
    /// `"Q14 · Priya"`.
    pub heading: String,
    /// The field it was written in; empty when the form has only one.
    pub label: String,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct StatLine {
    pub label: &'static str,
    /// `"—"` when the feed has not said.
    pub value: String,
}

/// Format an upstream statistic, or an em dash when there is none.
fn stat<T>(value: Option<T>, show: impl Fn(T) -> String) -> String {
    value.map(show).unwrap_or_else(|| "—".into())
}

/// The synced statistics the feeds have published, in the order a strategist
/// reads them. A value not published yet is left out rather than shown as a
/// zero or a dash: early in an event most are absent, and a column of dashes
/// on a phone hides the few that matter. Empty when nothing was synced.
pub fn stat_lines(stats: Option<&TeamEventStats>) -> Vec<StatLine> {
    let Some(s) = stats.filter(|s| s.has_any_data() || s.wins.is_some()) else {
        return Vec::new();
    };
    let whole = |n: i32| n.to_string();
    let two = |x: f64| format!("{x:.2}");
    let one = |x: f64| format!("{x:.1}");
    let record =
        (s.wins.is_some() || s.losses.is_some() || s.ties.is_some()).then(|| s.record_line());
    [
        StatLine {
            label: "Rank",
            value: stat(s.rank, whole),
        },
        StatLine {
            label: "Record",
            value: stat(record, |r| r),
        },
        StatLine {
            label: "Matches played",
            value: stat(s.matches_played, whole),
        },
        StatLine {
            label: "Ranking score",
            value: stat(s.qual_average, two),
        },
        StatLine {
            label: "Average match points",
            value: stat(s.avg_match_points, one),
        },
        StatLine {
            label: "Disqualifications",
            value: stat(s.dq_count, whole),
        },
        StatLine {
            label: "OPR",
            value: stat(s.opr, two),
        },
        StatLine {
            label: "DPR",
            value: stat(s.dpr, two),
        },
        StatLine {
            label: "CCWM",
            value: stat(s.ccwm, two),
        },
        StatLine {
            label: "Auto OPR",
            value: stat(s.auto_opr, two),
        },
        StatLine {
            label: "Teleop OPR",
            value: stat(s.teleop_opr, two),
        },
        StatLine {
            label: "Endgame OPR",
            value: stat(s.endgame_opr, two),
        },
        StatLine {
            label: "Qualification points",
            value: stat(s.qual_points, whole),
        },
        StatLine {
            label: "Playoff points",
            value: stat(s.elim_points, whole),
        },
        StatLine {
            label: "Award points",
            value: stat(s.award_points, whole),
        },
        StatLine {
            label: "Alliance selection points",
            value: stat(s.alliance_points, whole),
        },
        StatLine {
            label: "District points",
            value: stat(s.total_points, whole),
        },
    ]
    .into_iter()
    .filter(|line| line.value != "—")
    .collect()
}

#[derive(Debug, Clone)]
pub struct SummarySection {
    pub label: String,
    pub fields: Vec<SummaryLine>,
}

#[derive(Debug, Clone)]
pub struct SummaryLine {
    pub label: String,
    /// `"Center 2 · Left 1"`, `"avg 6.0 · best 9"`, `"1 of 2"`, or `"—"`;
    /// with `"· 2 answered"` when fewer than all `observed` answered it (U14).
    pub text: String,
}

/// The U12 summaries, in words. `observed` is how many observations they
/// were drawn from: a field fewer of them answered says how many did, so an
/// average of two is never read as an average of ten (U14).
pub fn summary_sections(sections: &[SectionSummary], observed: usize) -> Vec<SummarySection> {
    sections
        .iter()
        .map(|section| SummarySection {
            label: section.label.clone(),
            fields: section
                .fields
                .iter()
                .map(|f| {
                    let answered = if f.answered < observed {
                        format!(" · {} answered", f.answered)
                    } else {
                        String::new()
                    };
                    SummaryLine {
                        label: f.label.clone(),
                        text: match &f.summary {
                            None => "—".into(),
                            Some(Summary::Choice(tally)) => {
                                let tally = tally
                                    .iter()
                                    .map(|(label, n)| format!("{label} {n}"))
                                    .collect::<Vec<_>>()
                                    .join(" · ");
                                format!("{tally}{answered}")
                            }
                            Some(Summary::Counter { average, best }) => {
                                format!("avg {average:.1} · best {best}{answered}")
                            }
                            // "1 of 2" already says how many answered.
                            Some(Summary::Toggle { yes }) => format!("{yes} of {}", f.answered),
                        },
                    }
                })
                .collect(),
        })
        .collect()
}

/// One of the team's matches at the event.
#[derive(Debug, Clone)]
pub struct TeamMatchLine {
    /// `"Q14"`.
    pub label: String,
    /// `"red"` or `"blue"`.
    pub alliance: &'static str,
    /// `"with 254, 1678"`.
    pub partners: String,
    /// `"Won 88–72"`, `"Lost 60–75"`, `"Tied 70–70"`; empty until played.
    pub result: String,
    /// Scout this team in this match.
    pub scout_href: String,
}

impl TeamMatchLine {
    pub fn new(record: &MatchRecord, team: i32) -> Option<Self> {
        let alliance = record.alliance_of(team)?;
        let partners = record
            .partners_of(team)
            .iter()
            .map(i32::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        let (ours, theirs) = match alliance {
            "red" => (record.red_score, record.blue_score),
            _ => (record.blue_score, record.red_score),
        };
        let result = match (record.played, ours, theirs) {
            (true, Some(a), Some(b)) => {
                let verdict = match a.cmp(&b) {
                    std::cmp::Ordering::Greater => "Won",
                    std::cmp::Ordering::Less => "Lost",
                    std::cmp::Ordering::Equal => "Tied",
                };
                format!("{verdict} {a}–{b}")
            }
            (true, _, _) => "Played".into(),
            _ => String::new(),
        };
        Some(Self {
            label: record.label(),
            alliance,
            partners: if partners.is_empty() {
                String::new()
            } else {
                format!("with {partners}")
            },
            result,
            scout_href: scout_href(&record.event_key, &record.key, Some(team)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tt_core::matches::CompLevel;
    use tt_core::profile::FieldSummary;

    #[test]
    fn absent_statistics_are_dashes_and_none_at_all_is_empty() {
        assert!(stat_lines(None).is_empty());
        let blank = TeamEventStats {
            team_number: 254,
            event_key: "2026mabil".into(),
            ..TeamEventStats::default()
        };
        assert!(
            stat_lines(Some(&blank)).is_empty(),
            "a row with nothing in it"
        );

        let some = TeamEventStats {
            rank: Some(3),
            opr: Some(41.256),
            wins: Some(7),
            losses: Some(2),
            ..blank
        };
        let lines = stat_lines(Some(&some));
        let value = |label: &str| {
            lines
                .iter()
                .find(|l| l.label == label)
                .map(|l| l.value.clone())
                .unwrap()
        };
        assert_eq!(value("Rank"), "3");
        assert_eq!(value("OPR"), "41.26");
        assert_eq!(value("Record"), "7W 2L");
        assert!(
            !lines.iter().any(|l| l.label == "DPR"),
            "not synced is left out, never a zero"
        );
    }

    #[test]
    fn summaries_read_as_words() {
        let sections = [SectionSummary {
            label: "Scoring".into(),
            fields: vec![
                FieldSummary {
                    label: "Start".into(),
                    answered: 3,
                    summary: Some(Summary::Choice(vec![
                        ("Center".into(), 2),
                        ("Left".into(), 1),
                    ])),
                },
                FieldSummary {
                    label: "Teleop".into(),
                    answered: 3,
                    summary: Some(Summary::Counter {
                        average: 6.0,
                        best: 9,
                    }),
                },
                FieldSummary {
                    label: "Broke".into(),
                    answered: 2,
                    summary: Some(Summary::Toggle { yes: 1 }),
                },
                FieldSummary {
                    label: "Auto".into(),
                    answered: 0,
                    summary: None,
                },
            ],
        }];
        let texts: Vec<String> = summary_sections(&sections, 3)[0]
            .fields
            .iter()
            .map(|f| f.text.clone())
            .collect();
        assert_eq!(
            texts,
            ["Center 2 · Left 1", "avg 6.0 · best 9", "1 of 2", "—"]
        );

        // Drawn from five: the fields three answered say so.
        let texts: Vec<String> = summary_sections(&sections, 5)[0]
            .fields
            .iter()
            .map(|f| f.text.clone())
            .collect();
        assert_eq!(
            texts,
            [
                "Center 2 · Left 1 · 3 answered",
                "avg 6.0 · best 9 · 3 answered",
                "1 of 2",
                "—"
            ]
        );
    }

    #[test]
    fn a_match_line_says_who_with_and_how_it_went() {
        let record = MatchRecord {
            key: "2026mabil_qm14".into(),
            event_key: "2026mabil".into(),
            comp_level: CompLevel::Qualification,
            set_number: 1,
            match_number: 14,
            red: [Some(10101), Some(254), Some(1)],
            blue: [Some(2), Some(3), Some(4)],
            red_score: Some(60),
            blue_score: Some(75),
            winner: Some("blue".into()),
            played: true,
            scheduled_at: None,
            actual_at: None,
        };
        let line = TeamMatchLine::new(&record, 254).unwrap();
        assert_eq!(
            (
                line.label.as_str(),
                line.alliance,
                line.partners.as_str(),
                line.result.as_str()
            ),
            ("Q14", "red", "with 10101, 1", "Lost 60–75")
        );
        assert_eq!(TeamMatchLine::new(&record, 3).unwrap().result, "Won 75–60");
        assert!(TeamMatchLine::new(&record, 9999).is_none());
    }

    #[test]
    fn only_a_web_address_becomes_a_link() {
        let team = |site: &str| Team {
            number: 254,
            name: "Cheesy Poofs".into(),
            nickname: None,
            school: None,
            city: Some("San Jose".into()),
            state: Some("CA".into()),
            country: Some("USA".into()),
            rookie_year: Some(1999),
            website: Some(site.into()),
        };
        assert_eq!(
            TeamCard::new(&team("https://team254.com")).website,
            "https://team254.com"
        );
        assert_eq!(TeamCard::new(&team("javascript:alert(1)")).website, "");
        let card = TeamCard::new(&team("https://team254.com"));
        assert_eq!(
            (card.location.as_str(), card.rookie_year.as_str()),
            ("San Jose, CA · USA", "1999")
        );
    }
}
