//! OPR, DPR, and CCWM by least squares (FTC_BETA_PLAN.md, F10).
//!
//! The Blue Alliance works these out for FRC. FTC Events publishes none, so
//! the FTC build works them out here from the qualification results, the way
//! TBA does: each alliance's score in a match is one equation, the sum of its
//! robots' contributions,
//!
//! ```text
//!   x[red1] + x[red2] = red score
//! ```
//!
//! and the contributions are the least-squares fit over every such equation,
//! from the normal equations `AᵀA x = Aᵀb`. OPR fits each alliance's own
//! score, DPR the score its opponents made against it, and CCWM their
//! difference (by linearity, the fit of the winning margin). Any number of
//! robots an alliance works, so FTC's two (`ALLIANCE_SIZE`) need nothing
//! special.
//!
//! Too few matches leave the system underdetermined: early in an event, or a
//! team that has not played. Then there is no single answer, and [`solve`]
//! gives none rather than one of infinitely many.

use std::collections::BTreeMap;

/// One alliance in one played match: its robots and the points to fit.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub teams: Vec<i32>,
    pub points: f64,
}

/// A pivot this much smaller than the largest diagonal entry is taken as
/// zero: the matrix is singular, and there is no unique fit.
const SINGULAR: f64 = 1e-9;

/// Each team's least-squares contribution over `rows`, or `None` when the
/// rows do not determine one: no rows, fewer equations than teams, or teams
/// that only ever played together.
pub fn solve(rows: &[Row]) -> Option<BTreeMap<i32, f64>> {
    let teams: Vec<i32> = rows
        .iter()
        .flat_map(|r| r.teams.iter().copied())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let n = teams.len();
    if n == 0 || rows.len() < n {
        return None;
    }
    let index = |team: i32| teams.binary_search(&team).ok();

    // AᵀA and Aᵀb, built row by row; A is 0/1, so each pair in a row adds 1.
    let mut m = vec![vec![0.0; n + 1]; n];
    for row in rows {
        let at: Vec<usize> = row.teams.iter().filter_map(|t| index(*t)).collect();
        for &i in &at {
            for &j in &at {
                m[i][j] += 1.0;
            }
            m[i][n] += row.points;
        }
    }
    let scale = (0..n).map(|i| m[i][i]).fold(0.0, f64::max);

    // Gaussian elimination with partial pivoting on [AᵀA | Aᵀb].
    for col in 0..n {
        let pivot = (col..n).max_by(|a, b| m[*a][col].abs().total_cmp(&m[*b][col].abs()))?;
        if m[pivot][col].abs() <= SINGULAR * scale {
            return None;
        }
        m.swap(col, pivot);
        for r in col + 1..n {
            let factor = m[r][col] / m[col][col];
            if factor != 0.0 {
                let pivot_row = m[col].clone();
                for (cell, base) in m[r][col..=n].iter_mut().zip(&pivot_row[col..=n]) {
                    *cell -= factor * base;
                }
            }
        }
    }
    let mut x = vec![0.0; n];
    for r in (0..n).rev() {
        let known: f64 = (r + 1..n).map(|c| m[r][c] * x[c]).sum();
        x[r] = (m[r][n] - known) / m[r][r];
    }
    if x.iter().any(|v| !v.is_finite()) {
        return None;
    }
    Some(teams.into_iter().zip(x).collect())
}

/// One played match, for [`event`]: both alliances' robots and the points
/// each alliance scored, as the fit should count them (fouls left out).
#[derive(Debug, Clone, PartialEq)]
pub struct Played {
    pub red: Vec<i32>,
    pub blue: Vec<i32>,
    pub red_points: f64,
    pub blue_points: f64,
}

/// OPR, DPR, and CCWM for one team.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ratings {
    pub opr: f64,
    pub dpr: f64,
    pub ccwm: f64,
}

/// OPR, DPR, and CCWM for every team in `played`, or `None` when the
/// matches do not determine them.
pub fn event(played: &[Played]) -> Option<BTreeMap<i32, Ratings>> {
    let rows = |own: bool| -> Vec<Row> {
        played
            .iter()
            .flat_map(|m| {
                let (red, blue) = if own {
                    (m.red_points, m.blue_points)
                } else {
                    (m.blue_points, m.red_points)
                };
                [
                    Row {
                        teams: m.red.clone(),
                        points: red,
                    },
                    Row {
                        teams: m.blue.clone(),
                        points: blue,
                    },
                ]
            })
            .collect()
    };
    let oprs = solve(&rows(true))?;
    let dprs = solve(&rows(false))?;
    Some(
        oprs.into_iter()
            .filter_map(|(team, opr)| {
                let dpr = *dprs.get(&team)?;
                Some((
                    team,
                    Ratings {
                        opr,
                        dpr,
                        ccwm: opr - dpr,
                    },
                ))
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: i32 = 1;
    const B: i32 = 2;
    const C: i32 = 3;
    const D: i32 = 4;

    fn played(red: [i32; 2], blue: [i32; 2], scores: (f64, f64)) -> Played {
        Played {
            red: red.to_vec(),
            blue: blue.to_vec(),
            red_points: scores.0,
            blue_points: scores.1,
        }
    }

    fn close(got: f64, want: f64) {
        assert!((got - want).abs() < 1e-9, "{got} is not {want}");
    }

    /// Four teams, each paired once with every other: 6 equations, 4
    /// unknowns. Every team is in 3 rows and with each other team once, so
    /// `AᵀA = 2I + J` and `x = (Aᵀb − S) / 2`, with `S` a sixth of `ΣAᵀb`.
    fn round_robin(q3_red: f64) -> Vec<Played> {
        vec![
            played([A, B], [C, D], (30.0, 70.0)),
            played([A, C], [B, D], (40.0, 60.0)),
            played([A, D], [B, C], (q3_red, 50.0)),
        ]
    }

    #[test]
    fn scores_that_fit_exactly_give_each_robots_share() {
        let r = event(&round_robin(50.0)).expect("determined");
        // A 10, B 20, C 30, D 40 make every score.
        for (team, opr) in [(A, 10.0), (B, 20.0), (C, 30.0), (D, 40.0)] {
            close(r[&team].opr, opr);
        }
        // Against: Aᵀb = 180, 160, 140, 120, so S = 100.
        for (team, dpr) in [(A, 40.0), (B, 30.0), (C, 20.0), (D, 10.0)] {
            close(r[&team].dpr, dpr);
            close(r[&team].ccwm, r[&team].opr - dpr);
        }
    }

    #[test]
    fn scores_that_do_not_fit_give_the_least_squares_answer() {
        // Q3's red made 56, not 50: Aᵀb = 126, 140, 160, 186, S = 102.
        let r = event(&round_robin(56.0)).expect("determined");
        for (team, opr) in [(A, 12.0), (B, 19.0), (C, 29.0), (D, 42.0)] {
            close(r[&team].opr, opr);
        }
        // The residuals sum to zero over each team's rows.
        close(r[&A].opr + r[&B].opr - 30.0, 1.0);
        close(r[&A].opr + r[&D].opr - 56.0, -2.0);
    }

    #[test]
    fn too_few_matches_give_nothing_rather_than_a_guess() {
        assert!(event(&[]).is_none());
        // One match: 2 equations, 4 teams.
        assert!(event(&round_robin(50.0)[..1]).is_none());
        // Enough equations, but A and B only ever play together: their sum
        // is known, not their shares.
        let together = vec![
            played([A, B], [C, D], (30.0, 70.0)),
            played([A, B], [C, D], (32.0, 68.0)),
            played([A, B], [C, D], (28.0, 72.0)),
            played([A, B], [C, D], (30.0, 70.0)),
        ];
        assert!(event(&together).is_none());
    }

    #[test]
    fn a_three_robot_alliance_solves_the_same_way() {
        let rows = [
            Row {
                teams: vec![A, B, C],
                points: 60.0,
            },
            Row {
                teams: vec![A, B, D],
                points: 70.0,
            },
            Row {
                teams: vec![A, C, D],
                points: 80.0,
            },
            Row {
                teams: vec![B, C, D],
                points: 90.0,
            },
        ];
        let x = solve(&rows).expect("square and full rank");
        // A 10, B 20, C 30, D 40.
        for (team, want) in [(A, 10.0), (B, 20.0), (C, 30.0), (D, 40.0)] {
            close(x[&team], want);
        }
    }
}
