//! Teams side by side (DR1, `docs/design/DATA_REVIEW.md`): the numbers a lead reads
//! before alliance selection, one column per team, one row per thing scouts
//! record.
//!
//! Modelled on AdvantageScope's statistics view: a number is never shown
//! alone, but with how many matches it rests on and how spread out they were
//! (median, range, quartiles, standard deviation), because a 6-point average
//! from 6, 6, 6 is a different robot from one made of 0, 6 and 12.
//!
//! # What a row is
//!
//! The form, section by section, in form order, as on the team page:
//!
//! - a **counter**, or a choice that carries points, is a spread of its
//!   per-match values;
//! - a **yes/no** is a rate, "4 of 6";
//! - a **choice without points** (robot status, driver skill) is a tally,
//!   most common first, since it has no "higher";
//! - free text is not compared; notes are read on the team page.
//!
//! The scout estimate leads, and the event's official numbers (rank, OPR,
//! DPR, CCWM) close the table.
//!
//! # What a value is
//!
//! A team's matches, in schedule order. Two scouts on one robot in one match
//! make one value, their average, as on the graph. [`Window::Last`] keeps a
//! team's last few matches, to see how it plays now rather than on Friday.

use crate::graph;
use crate::records::TeamEventStats;
use crate::season::{FieldKind, Payload, SeasonSchema, Value, WeightOverrides};

/// Teams compared at once: columns that still fit a laptop screen.
pub const MAX_TEAMS: usize = 6;
/// Teams shown before the viewer chooses: the best by scout estimate.
pub const DEFAULT_TEAMS: usize = 4;
/// Fewer matches than this is too few to trust, as on the rankings page.
pub const THIN: usize = 3;

/// How many of a team's matches count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Window {
    All,
    /// The team's last `n` scouted matches.
    Last(usize),
}

impl Window {
    /// The choices offered, in the URL's words.
    pub const OFFERED: [Window; 3] = [Window::All, Window::Last(5), Window::Last(3)];

    /// `"all"`, `"5"` or `"3"`; anything else is every match.
    pub fn parse(raw: &str) -> Self {
        Self::OFFERED
            .into_iter()
            .find(|w| w.as_str() == raw.trim())
            .unwrap_or(Window::All)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Window::All => "all",
            Window::Last(5) => "5",
            Window::Last(3) => "3",
            Window::Last(_) => "all",
        }
    }

    pub fn label(self) -> String {
        match self {
            Window::All => "Every match".into(),
            Window::Last(n) => format!("Last {n} matches"),
        }
    }

    fn keep<T>(self, matches: &[T]) -> &[T] {
        match self {
            Window::All => matches,
            Window::Last(n) => &matches[matches.len().saturating_sub(n)..],
        }
    }
}

/// Put `add` among `teams`, which already hold at most `max`. When they are
/// full, the last one not in `spare` (our team, the reference) makes room.
/// Returns the team that made room, if one had to.
pub fn make_room(teams: &mut Vec<i32>, add: i32, spare: &[i32], max: usize) -> Option<i32> {
    if teams.contains(&add) || max == 0 {
        return None;
    }
    let mut dropped = None;
    if teams.len() >= max
        && let Some(at) = teams.iter().rposition(|t| !spare.contains(t))
    {
        dropped = Some(teams.remove(at));
    }
    if teams.len() < max {
        teams.push(add);
    }
    dropped
}

/// How spread out a set of values is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spread {
    pub n: usize,
    pub mean: f64,
    pub median: f64,
    pub min: f64,
    pub max: f64,
    /// First and third quartiles: the middle half of the values lies between.
    pub q1: f64,
    pub q3: f64,
    /// Sample standard deviation; `None` from a single value.
    pub sd: Option<f64>,
}

/// The spread of `values`, or `None` when there are none.
pub fn spread(values: &[f64]) -> Option<Spread> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let n = sorted.len();
    let mean = sorted.iter().sum::<f64>() / n as f64;
    let sd = (n > 1)
        .then(|| (sorted.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1) as f64).sqrt());
    Some(Spread {
        n,
        mean,
        median: quantile(&sorted, 0.5),
        min: sorted[0],
        max: sorted[n - 1],
        q1: quantile(&sorted, 0.25),
        q3: quantile(&sorted, 0.75),
        sd,
    })
}

/// The `q` quantile of sorted values, interpolating between neighbours, as
/// spreadsheets do.
fn quantile(sorted: &[f64], q: f64) -> f64 {
    let at = q * (sorted.len() - 1) as f64;
    let (lo, hi) = (at.floor() as usize, at.ceil() as usize);
    sorted[lo] + (sorted[hi] - sorted[lo]) * (at - lo as f64)
}

/// Which way is good, so the best team in a row can be marked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Better {
    Higher,
    Lower,
    /// Neither: a yes/no worth no points, a tally.
    Neither,
}

/// One team's value in one row.
#[derive(Debug, Clone, PartialEq)]
pub enum Cell {
    /// Per-match values; `None` when no counted match answered it.
    Spread(Option<Spread>),
    /// A yes/no: yes in `yes` of `of` matches.
    Rate { yes: usize, of: usize },
    /// A choice: each option chosen, with how often, most common first.
    Tally(Vec<(String, usize)>),
    /// One number for the event, from its results.
    Official(Option<f64>),
}

impl Cell {
    /// Nothing recorded.
    pub fn is_empty(&self) -> bool {
        match self {
            Cell::Tally(tally) => tally.is_empty(),
            _ => self.value().is_none(),
        }
    }

    /// The number a row is ranked by, when it has one.
    pub fn value(&self) -> Option<f64> {
        match self {
            Cell::Spread(s) => s.map(|s| s.mean),
            Cell::Rate { yes, of } => (*of > 0).then(|| *yes as f64 / *of as f64),
            Cell::Tally(_) => None,
            Cell::Official(v) => *v,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub key: String,
    pub label: String,
    pub better: Better,
    /// One per team, in the table's order.
    pub cells: Vec<Cell>,
}

impl Row {
    /// Which cells hold the row's best value: none when fewer than two teams
    /// have one or all are equal, every one tied for best otherwise.
    pub fn best(&self) -> Vec<bool> {
        let values: Vec<Option<f64>> = self.cells.iter().map(Cell::value).collect();
        let present: Vec<f64> = values.iter().flatten().copied().collect();
        let pick = match self.better {
            Better::Higher => present.iter().copied().reduce(f64::max),
            Better::Lower => present.iter().copied().reduce(f64::min),
            Better::Neither => None,
        };
        let spread = present.iter().copied().reduce(f64::max).unwrap_or(0.0)
            - present.iter().copied().reduce(f64::min).unwrap_or(0.0);
        match pick {
            Some(best) if present.len() >= 2 && spread > 1e-9 => values
                .iter()
                .map(|v| v.is_some_and(|v| (v - best).abs() < 1e-9))
                .collect(),
            _ => vec![false; self.cells.len()],
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub label: String,
    pub rows: Vec<Row>,
}

/// One team's matches, in schedule order, each with every approved
/// observation of it on the current form.
#[derive(Debug, Clone)]
pub struct TeamMatches<'a> {
    pub team_number: i32,
    pub matches: Vec<Vec<&'a Payload>>,
    pub stats: Option<&'a TeamEventStats>,
}

/// The whole table.
#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    /// Matches each team's column counts, after the window.
    pub matches: Vec<usize>,
    pub groups: Vec<Group>,
}

/// Compare `teams`, column by column in the order given.
pub fn table(
    schema: &SeasonSchema,
    overrides: &WeightOverrides,
    teams: &[TeamMatches],
    window: Window,
) -> Table {
    let kept: Vec<&[Vec<&Payload>]> = teams.iter().map(|t| window.keep(&t.matches)).collect();
    let metrics = graph::metrics(schema);
    let metric = |key: &str| metrics.iter().find(|m| m.key == key);

    // Per match, the scouts' average, as on the graph.
    let spread_of = |key: &str| -> Vec<Cell> {
        let Some(metric) = metric(key) else {
            return vec![Cell::Spread(None); teams.len()];
        };
        kept.iter()
            .map(|matches| {
                let values: Vec<f64> = matches
                    .iter()
                    .filter_map(|observed| {
                        let found: Vec<f64> = observed
                            .iter()
                            .filter_map(|p| graph::value(schema, overrides, metric, p))
                            .collect();
                        (!found.is_empty()).then(|| found.iter().sum::<f64>() / found.len() as f64)
                    })
                    .collect();
                Cell::Spread(spread(&values))
            })
            .collect()
    };

    let mut groups = vec![Group {
        label: "Overall".into(),
        rows: vec![Row {
            key: graph::POINTS.into(),
            label: "Scout estimate (points)".into(),
            better: Better::Higher,
            cells: spread_of(graph::POINTS),
        }],
    }];

    for section in &schema.sections {
        let mut rows = Vec::new();
        for field in &section.fields {
            let (better, cells) = match &field.kind {
                FieldKind::Counter { .. } => {
                    // Fouls are the one count where fewer is better.
                    let better = if field.key.contains("foul") {
                        Better::Lower
                    } else {
                        Better::Higher
                    };
                    (better, spread_of(&format!("f.{}", field.key)))
                }
                FieldKind::Select { options } if options.iter().any(|o| o.points != 0) => {
                    (Better::Higher, spread_of(&format!("f.{}", field.key)))
                }
                FieldKind::Toggle { points } => {
                    // A match counts once, as yes when any scout said yes.
                    let cells = kept
                        .iter()
                        .map(|matches| {
                            let answered: Vec<bool> = matches
                                .iter()
                                .filter_map(|observed| {
                                    let flags: Vec<bool> = observed
                                        .iter()
                                        .filter_map(|p| match p.get(&field.key) {
                                            Some(Value::Flag(on)) => Some(*on),
                                            _ => None,
                                        })
                                        .collect();
                                    (!flags.is_empty()).then(|| flags.iter().any(|on| *on))
                                })
                                .collect();
                            Cell::Rate {
                                yes: answered.iter().filter(|on| **on).count(),
                                of: answered.len(),
                            }
                        })
                        .collect();
                    let better = if *points != 0 {
                        Better::Higher
                    } else {
                        Better::Neither
                    };
                    (better, cells)
                }
                FieldKind::Select { options } => {
                    // Every scout's answer counts in a tally, as on the team page.
                    let cells = kept
                        .iter()
                        .map(|matches| {
                            let mut tally: Vec<(String, usize)> = options
                                .iter()
                                .map(|o| {
                                    let n = matches
                                        .iter()
                                        .flatten()
                                        .filter(|p| {
                                            matches!(p.get(&field.key), Some(Value::Text(t)) if *t == o.key)
                                        })
                                        .count();
                                    (o.label.clone(), n)
                                })
                                .filter(|(_, n)| *n > 0)
                                .collect();
                            tally.sort_by_key(|a| std::cmp::Reverse(a.1));
                            Cell::Tally(tally)
                        })
                        .collect();
                    (Better::Neither, cells)
                }
                FieldKind::Text { .. } => continue,
            };
            // A row nobody recorded anything in is noise when reading fast.
            if cells.iter().all(Cell::is_empty) {
                continue;
            }
            rows.push(Row {
                key: field.key.clone(),
                label: field.label.clone(),
                better,
                cells,
            });
        }
        if !rows.is_empty() {
            groups.push(Group {
                label: section.label.clone(),
                rows,
            });
        }
    }

    let official = |read: fn(&TeamEventStats) -> Option<f64>| -> Vec<Cell> {
        teams
            .iter()
            .map(|t| Cell::Official(t.stats.and_then(read)))
            .collect()
    };
    let rows: Vec<Row> = [
        (
            "rank",
            "Rank",
            Better::Lower,
            official(|s| s.rank.map(f64::from)),
        ),
        ("opr", "OPR", Better::Higher, official(|s| s.opr)),
        ("dpr", "DPR", Better::Lower, official(|s| s.dpr)),
        ("ccwm", "CCWM", Better::Higher, official(|s| s.ccwm)),
    ]
    .into_iter()
    .filter(|(_, _, _, cells)| cells.iter().any(|c| c.value().is_some()))
    .map(|(key, label, better, cells)| Row {
        key: key.into(),
        label: label.into(),
        better,
        cells,
    })
    .collect();
    if !rows.is_empty() {
        groups.push(Group {
            label: "Official, from the event".into(),
            rows,
        });
    }

    Table {
        matches: kept.iter().map(|m| m.len()).collect(),
        groups,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::season::current_season;

    fn payload(pairs: &[(&str, Value)]) -> Payload {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    fn row<'t>(table: &'t Table, key: &str) -> &'t Row {
        table
            .groups
            .iter()
            .flat_map(|g| &g.rows)
            .find(|r| r.key == key)
            .unwrap_or_else(|| panic!("{key}"))
    }

    #[test]
    fn a_spread_has_its_middle_and_its_edges() {
        let s = spread(&[12.0, 0.0, 6.0, 6.0]).unwrap();
        assert_eq!(
            (s.n, s.mean, s.median, s.min, s.max),
            (4, 6.0, 6.0, 0.0, 12.0)
        );
        assert_eq!((s.q1, s.q3), (4.5, 7.5));
        assert!((s.sd.unwrap() - 24f64.sqrt()).abs() < 1e-9);
        let one = spread(&[3.0]).unwrap();
        assert_eq!((one.median, one.q1, one.q3, one.sd), (3.0, 3.0, 3.0, None));
        assert_eq!(spread(&[]), None);
    }

    #[test]
    fn a_team_added_to_a_full_comparison_takes_the_place_of_the_last_one_not_spared() {
        let mut teams = vec![1, 2, 3];
        assert_eq!(make_room(&mut teams, 4, &[], 4), None);
        assert_eq!(teams, [1, 2, 3, 4]);
        assert_eq!(make_room(&mut teams, 5, &[4], 4), Some(3), "4 is ours");
        assert_eq!(teams, [1, 2, 4, 5]);
        assert_eq!(make_room(&mut teams, 2, &[], 4), None, "already there");
        assert_eq!(teams, [1, 2, 4, 5]);
        let mut all_spared = vec![1, 2];
        assert_eq!(make_room(&mut all_spared, 3, &[1, 2], 2), None);
        assert_eq!(all_spared, [1, 2], "nothing may go, so nothing comes in");
    }

    #[test]
    fn a_window_keeps_the_last_matches() {
        assert_eq!(Window::parse("3"), Window::Last(3));
        assert_eq!(Window::parse("junk"), Window::All);
        assert_eq!(Window::Last(3).keep(&[1, 2, 3, 4, 5]), &[3, 4, 5]);
        assert_eq!(Window::Last(5).keep(&[1, 2]), &[1, 2]);
    }

    #[test]
    fn the_table_follows_the_form_with_spreads_rates_and_tallies() {
        let schema = current_season().unwrap();
        let none = WeightOverrides::new();
        let a1 = payload(&[
            ("garden_delivered", Value::Count(4)),
            ("teleop_park", Value::Flag(true)),
            ("robot_status", Value::Text("played".into())),
        ]);
        let a2 = payload(&[
            ("garden_delivered", Value::Count(6)),
            ("teleop_park", Value::Flag(false)),
            ("robot_status", Value::Text("played".into())),
        ]);
        let a3 = payload(&[
            ("garden_delivered", Value::Count(11)),
            ("robot_status", Value::Text("disabled".into())),
        ]);
        let b1 = payload(&[("garden_delivered", Value::Count(2))]);
        let stats = TeamEventStats {
            team_number: 7,
            event_key: "e".into(),
            rank: Some(2),
            opr: Some(30.0),
            ..Default::default()
        };
        let teams = [
            TeamMatches {
                team_number: 7,
                // Two scouts on match one, averaged to 5.
                matches: vec![vec![&a1, &a2], vec![&a3]],
                stats: Some(&stats),
            },
            TeamMatches {
                team_number: 8,
                matches: vec![vec![&b1]],
                stats: None,
            },
        ];
        let table = table(&schema, &none, &teams, Window::All);
        assert_eq!(table.matches, [2, 1]);
        let labels: Vec<&str> = table.groups.iter().map(|g| g.label.as_str()).collect();
        assert_eq!(labels.first(), Some(&"Overall"));
        assert_eq!(labels.last(), Some(&"Official, from the event"));
        assert!(!labels.contains(&"Notes"), "free text is not compared");
        assert!(
            !table
                .groups
                .iter()
                .flat_map(|g| &g.rows)
                .any(|r| r.key == "driver_skill"),
            "a field nobody answered is left out"
        );

        let garden = row(&table, "garden_delivered");
        let Cell::Spread(Some(s)) = garden.cells[0] else {
            panic!()
        };
        assert_eq!(
            (s.n, s.mean, s.max),
            (2, 8.0, 11.0),
            "two scouts make one match"
        );
        assert_eq!(garden.best(), [true, false]);

        assert_eq!(
            row(&table, "teleop_park").cells[0],
            Cell::Rate { yes: 1, of: 1 },
            "any scout's yes makes the match a yes; a match nobody answered is not counted"
        );
        assert_eq!(
            row(&table, "robot_status").cells[0],
            Cell::Tally(vec![("Played".into(), 2), ("Disabled".into(), 1)])
        );
        assert_eq!(row(&table, "rank").cells[0], Cell::Official(Some(2.0)));
        assert!(
            !table
                .groups
                .iter()
                .flat_map(|g| &g.rows)
                .any(|r| r.key == "ccwm"),
            "an official number nobody has is left out"
        );
    }

    #[test]
    fn the_window_counts_only_the_last_matches() {
        let schema = current_season().unwrap();
        let p = |n| payload(&[("garden_delivered", Value::Count(n))]);
        let (m1, m2, m3) = (p(0), p(9), p(9));
        let teams = [TeamMatches {
            team_number: 7,
            matches: vec![vec![&m1], vec![&m2], vec![&m3]],
            stats: None,
        }];
        let all = table(&schema, &WeightOverrides::new(), &teams, Window::All);
        let last = table(&schema, &WeightOverrides::new(), &teams, Window::Last(2));
        let mean = |t: &Table| row(t, "garden_delivered").cells[0].value().unwrap();
        assert_eq!((mean(&all), mean(&last)), (6.0, 9.0));
        assert_eq!(last.matches, [2]);
    }

    #[test]
    fn the_best_is_marked_only_when_teams_differ_and_fewer_fouls_is_better() {
        let r = |better, values: &[Option<f64>]| Row {
            key: "k".into(),
            label: "k".into(),
            better,
            cells: values.iter().map(|v| Cell::Official(*v)).collect(),
        };
        assert_eq!(
            r(Better::Higher, &[Some(1.0), Some(3.0), Some(3.0)]).best(),
            [false, true, true]
        );
        assert_eq!(
            r(Better::Lower, &[Some(1.0), Some(3.0), None]).best(),
            [true, false, false]
        );
        assert_eq!(
            r(Better::Higher, &[Some(2.0), Some(2.0)]).best(),
            [false, false]
        );
        assert_eq!(r(Better::Higher, &[Some(2.0), None]).best(), [false, false]);
        assert_eq!(
            r(Better::Neither, &[Some(1.0), Some(3.0)]).best(),
            [false, false]
        );

        let schema = current_season().unwrap();
        let fouls = payload(&[("fouls_observed", Value::Count(1))]);
        let teams = [TeamMatches {
            team_number: 7,
            matches: vec![vec![&fouls]],
            stats: None,
        }];
        let table = table(&schema, &WeightOverrides::new(), &teams, Window::All);
        assert_eq!(row(&table, "fouls_observed").better, Better::Lower);
    }
}
