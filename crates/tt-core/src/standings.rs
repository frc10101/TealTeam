//! Rankings typed in by hand (I14): the true last resort.
//!
//! When no uplink works -- no tether, no scout with signal, no QR -- a lead
//! scout can still read the standings off the audience display. The format is
//! built for typing forty rows in five minutes on whatever is to hand:
//!
//! ```text
//! 254 3.42 11-1-0
//! 1678 3.17 10-2
//! 10101
//! ```
//!
//! One team per line, **in rank order**: the rank is the line, so it is never
//! typed and can never disagree with the order. After the team number, a
//! ranking score and a `W-L-T` record are optional and may come in either
//! order. Spaces, tabs, and commas all separate, so a pasted spreadsheet column
//! works too.
//!
//! Every problem is reported at once, by line, and nothing is saved until
//! there are none -- a half-applied ranking is worse than a stale one.

use crate::records::TeamEventStats;

/// One row of typed standings.
#[derive(Debug, Clone, PartialEq)]
pub struct Standing {
    /// 1 for the first line, and so on. Blank lines do not count.
    pub rank: i32,
    pub team_number: i32,
    /// The ranking score, as the audience display shows it.
    pub ranking_score: Option<f64>,
    pub record: Option<Record>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Record {
    pub wins: i32,
    pub losses: i32,
    pub ties: i32,
}

impl Record {
    pub fn played(&self) -> i32 {
        self.wins + self.losses + self.ties
    }
}

/// Read typed standings.
///
/// `roster` is the event's teams. A number not on it is almost always a typo,
/// or a rank typed where the team belongs, so it is refused -- unless the
/// roster is empty, when there is nothing to check against and every number is
/// taken at its word.
pub fn parse(text: &str, roster: &[i32]) -> Result<Vec<Standing>, Vec<String>> {
    let mut standings: Vec<Standing> = Vec::new();
    let mut errors = Vec::new();

    for (index, line) in text.lines().enumerate() {
        let tokens: Vec<&str> = line
            .split(|c: char| c.is_whitespace() || c == ',')
            .filter(|t| !t.is_empty())
            .collect();
        let Some((team, rest)) = tokens.split_first() else {
            continue;
        };
        let line_no = index + 1;
        let rank = standings.len() as i32 + 1;

        let team_number = match parse_team(team) {
            Some(n) => n,
            None => {
                errors.push(format!("Line {line_no}: “{team}” is not a team number."));
                // Keep counting ranks, so the next line's rank is still right.
                standings.push(Standing {
                    rank,
                    team_number: 0,
                    ranking_score: None,
                    record: None,
                });
                continue;
            }
        };

        if !roster.is_empty() && !roster.contains(&team_number) {
            errors.push(format!(
                "Line {line_no}: team {team_number} is not at this event. \
                 Each line starts with the team number; the rank is the line."
            ));
        } else if let Some(earlier) = standings.iter().find(|s| s.team_number == team_number) {
            errors.push(format!(
                "Line {line_no}: team {team_number} is already ranked {}.",
                earlier.rank
            ));
        }

        let mut standing = Standing {
            rank,
            team_number,
            ranking_score: None,
            record: None,
        };
        for token in rest {
            if token.contains('-') {
                match (parse_record(token), standing.record) {
                    (Some(record), None) => standing.record = Some(record),
                    (Some(_), Some(_)) => errors.push(format!(
                        "Line {line_no}: two records for team {team_number}."
                    )),
                    (None, _) => errors.push(format!(
                        "Line {line_no}: “{token}” is not a record. Write wins-losses-ties, like 9-2-1."
                    )),
                }
            } else {
                match (parse_score(token), standing.ranking_score) {
                    (Some(score), None) => standing.ranking_score = Some(score),
                    (Some(_), Some(_)) => errors.push(format!(
                        "Line {line_no}: two ranking scores for team {team_number}. \
                         After the team, only a ranking score and a record."
                    )),
                    (None, _) => {
                        errors.push(format!("Line {line_no}: “{token}” is not a ranking score."))
                    }
                }
            }
        }
        standings.push(standing);
    }

    if standings.is_empty() && errors.is_empty() {
        errors.push("Type at least one team.".into());
    }
    if errors.is_empty() {
        Ok(standings)
    } else {
        Err(errors)
    }
}

/// `254` or `frc254`, as TBA writes it.
fn parse_team(token: &str) -> Option<i32> {
    let digits = token
        .strip_prefix("frc")
        .or_else(|| token.strip_prefix("FRC"))
        .unwrap_or(token);
    digits.parse().ok().filter(|n| (1..=99_999).contains(n))
}

/// `9-2-1`, or `9-2` with no ties.
fn parse_record(token: &str) -> Option<Record> {
    let parts: Vec<i32> = token
        .split('-')
        .map(|p| p.parse().ok().filter(|n: &i32| (0..=200).contains(n)))
        .collect::<Option<_>>()?;
    match parts[..] {
        [wins, losses] => Some(Record {
            wins,
            losses,
            ties: 0,
        }),
        [wins, losses, ties] => Some(Record { wins, losses, ties }),
        _ => None,
    }
}

fn parse_score(token: &str) -> Option<f64> {
    token
        .parse::<f64>()
        .ok()
        .filter(|s| s.is_finite() && *s >= 0.0 && *s < 1000.0)
}

/// Stored standings written back out in the format [`parse`] reads, ranked
/// teams only, best first -- so the form opens on what is there to correct.
pub fn format(stats: &[TeamEventStats]) -> String {
    let mut ranked: Vec<&TeamEventStats> = stats.iter().filter(|s| s.rank.is_some()).collect();
    ranked.sort_by_key(|s| (s.rank, s.team_number));
    ranked
        .iter()
        .map(|s| {
            let mut line = s.team_number.to_string();
            if let Some(score) = s.qual_average {
                line.push_str(&format!(" {}", trim(score)));
            }
            if let (Some(w), Some(l)) = (s.wins, s.losses) {
                line.push_str(&format!(" {w}-{l}-{}", s.ties.unwrap_or(0)));
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `3.42`, `3`, never `3.4200000000000004`.
fn trim(score: f64) -> String {
    let s = format!("{score:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROSTER: &[i32] = &[254, 1678, 10101, 118];

    #[test]
    fn the_rank_is_the_line_and_extras_are_optional_in_either_order() {
        let text = "254 3.42 11-1-0\n\n  1678\t10-2 3.17 \nfrc10101,2.5\n";
        let standings = parse(text, ROSTER).expect("parses");
        assert_eq!(
            standings,
            [
                Standing {
                    rank: 1,
                    team_number: 254,
                    ranking_score: Some(3.42),
                    record: Some(Record {
                        wins: 11,
                        losses: 1,
                        ties: 0
                    }),
                },
                Standing {
                    rank: 2,
                    team_number: 1678,
                    ranking_score: Some(3.17),
                    record: Some(Record {
                        wins: 10,
                        losses: 2,
                        ties: 0
                    }),
                },
                Standing {
                    rank: 3,
                    team_number: 10101,
                    ranking_score: Some(2.5),
                    record: None,
                },
            ]
        );
        assert_eq!(standings[0].record.unwrap().played(), 12);
    }

    #[test]
    fn every_bad_line_is_reported_at_once() {
        let text = "254\n25x\n254\n1 118\n1678 3.1 2.9\n118 9-x";
        let errors = parse(text, ROSTER).expect_err("refused");
        assert_eq!(
            errors,
            [
                "Line 2: “25x” is not a team number.",
                "Line 3: team 254 is already ranked 1.",
                "Line 4: team 1 is not at this event. \
                 Each line starts with the team number; the rank is the line.",
                "Line 5: two ranking scores for team 1678. \
                 After the team, only a ranking score and a record.",
                "Line 6: “9-x” is not a record. Write wins-losses-ties, like 9-2-1.",
            ]
        );
    }

    #[test]
    fn with_no_roster_any_team_number_is_taken() {
        let standings = parse("9999\n254", &[]).expect("parses");
        assert_eq!(standings[0].team_number, 9999);
    }

    #[test]
    fn nothing_typed_is_an_error_not_an_empty_ranking() {
        assert_eq!(
            parse(" \n\n", ROSTER).expect_err("empty"),
            ["Type at least one team."]
        );
    }

    #[test]
    fn stored_standings_come_back_as_text_that_parses_to_the_same() {
        let stat = |team: i32, rank: Option<i32>| TeamEventStats {
            team_number: team,
            rank,
            ..TeamEventStats::default()
        };
        let stats = [
            TeamEventStats {
                qual_average: Some(3.4200000000000004),
                wins: Some(11),
                losses: Some(1),
                ties: Some(0),
                ..stat(254, Some(1))
            },
            stat(118, None),
            TeamEventStats {
                qual_average: Some(3.0),
                ..stat(10101, Some(2))
            },
        ];
        let text = format(&stats);
        assert_eq!(text, "254 3.42 11-1-0\n10101 3");

        let again = parse(&text, ROSTER).expect("parses");
        assert_eq!(
            again.iter().map(|s| s.team_number).collect::<Vec<_>>(),
            [254, 10101]
        );
    }
}
