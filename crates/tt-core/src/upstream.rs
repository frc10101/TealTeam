//! Parsing and interpretation of FIRST and Blue Alliance payloads (I3).
//!
//! Pure: types, deserialization, and the derivations that turn an upstream
//! response into something the schema can hold. No HTTP — transport lives in
//! `tt-upstream`, so these can be exercised against recorded payloads and, later,
//! compiled to wasm32 so a client with signal can fetch upstream itself (S4).
//!
//! # Why the fallbacks exist
//!
//! TBA's response shape drifts between seasons, and the retired implementation
//! lost real data to it (see `docs/TBA_SCHEMA_FIX_SUMMARY.md`):
//!
//!   * `/coprs` returns **dynamically named** components — `totalAutoPoints`,
//!     `totalTeleopPoints` and friends — not fixed `auto_oprs` fields. Code that
//!     read fixed names got nulls for every component OPR.
//!   * Modern rankings put the numbers in `sort_orders` and `extra_stats` arrays
//!     and leave `qual_points` / `total_points` **null**. Code that read the
//!     primitives got zeros.
//!
//! Every extractor below therefore tries the legacy primitive first, then the
//! documented array position, and only then gives up. Do not "simplify" them to
//! direct field access; that is the bug.

use serde::Deserialize;
use std::collections::HashMap;

use crate::matches::CompLevel;

// ── The Blue Alliance ───────────────────────────────────────────────────────

/// `/event/{key}/oprs`
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Oprs {
    #[serde(default)]
    pub oprs: HashMap<String, f64>,
    #[serde(default)]
    pub dprs: HashMap<String, f64>,
    #[serde(default)]
    pub ccwms: HashMap<String, f64>,
}

/// `/event/{key}/coprs`
///
/// Component names are season-specific and unknown ahead of time, so this is a
/// map of maps: component name -> team key -> value.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(transparent)]
pub struct ComponentOprs {
    pub components: HashMap<String, HashMap<String, f64>>,
}

/// Which phase of a match a component OPR describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Auto,
    Teleop,
    Endgame,
}

impl Phase {
    /// Substring matched case-insensitively against the component name.
    fn needle(self) -> &'static str {
        match self {
            Phase::Auto => "auto",
            Phase::Teleop => "teleop",
            Phase::Endgame => "endgame",
        }
    }
}

impl ComponentOprs {
    /// Best-effort component OPR for one team in one phase.
    ///
    /// Prefers a component whose name contains the phase word and also mentions
    /// points (`totalAutoPoints`), then any component mentioning the phase, then
    /// nothing. Never guesses across phases: a missing endgame component yields
    /// `None`, not a teleop number.
    pub fn phase_opr(&self, team_key: &str, phase: Phase) -> Option<f64> {
        let needle = phase.needle();

        let mut fallback = None;
        for (name, values) in &self.components {
            let lower = name.to_ascii_lowercase();
            if !lower.contains(needle) {
                continue;
            }
            let Some(value) = values.get(team_key).copied() else {
                continue;
            };
            if lower.contains("point") {
                // Strongest signal: a points component for this phase.
                return Some(value);
            }
            fallback.get_or_insert(value);
        }
        fallback
    }

    /// All three phases at once.
    pub fn phases(&self, team_key: &str) -> (Option<f64>, Option<f64>, Option<f64>) {
        (
            self.phase_opr(team_key, Phase::Auto),
            self.phase_opr(team_key, Phase::Teleop),
            self.phase_opr(team_key, Phase::Endgame),
        )
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct WinLossRecord {
    #[serde(default)]
    pub wins: i32,
    #[serde(default)]
    pub losses: i32,
    #[serde(default)]
    pub ties: i32,
}

/// One row of `/event/{key}/rankings`.
///
/// The nullable primitives are the legacy schema; `sort_orders` and
/// `extra_stats` are where modern seasons put the same numbers.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Ranking {
    pub team_key: String,
    #[serde(default)]
    pub rank: i32,
    #[serde(default)]
    pub matches_played: i32,
    #[serde(default)]
    pub dq: i32,
    #[serde(default)]
    pub record: WinLossRecord,

    #[serde(default)]
    pub qual_average: Option<f64>,
    #[serde(default)]
    pub qual_points: Option<i64>,
    #[serde(default)]
    pub total_points: Option<i64>,
    #[serde(default)]
    pub elim_points: Option<i64>,
    #[serde(default)]
    pub award_points: Option<i64>,
    #[serde(default)]
    pub alliance_points: Option<i64>,

    /// `[0]` ranking score / qual average, `[1]` average match points.
    #[serde(default)]
    pub sort_orders: Vec<f64>,
    /// `[0]` an alternative total ranking points.
    #[serde(default)]
    pub extra_stats: Vec<f64>,
}

impl Ranking {
    /// Ranking score. Legacy primitive, else `sort_orders[0]`.
    pub fn effective_qual_average(&self) -> Option<f64> {
        self.qual_average
            .or_else(|| self.sort_orders.first().copied())
    }

    /// Average match points. Only ever `sort_orders[1]`; there is no legacy
    /// primitive for it, which is why the retired app displayed nothing.
    pub fn effective_avg_match_points(&self) -> Option<f64> {
        self.sort_orders.get(1).copied()
    }

    /// Total ranking points. Legacy primitive, else `extra_stats[0]`.
    pub fn effective_total_points(&self) -> Option<i64> {
        self.total_points
            .or_else(|| self.extra_stats.first().map(|v| v.round() as i64))
    }

    /// Qualification points. Legacy primitive, else `sort_orders[0]` rounded --
    /// in seasons that dropped the primitive, the ranking score is the closest
    /// equivalent.
    pub fn effective_qual_points(&self) -> Option<i64> {
        self.qual_points
            .or_else(|| self.sort_orders.first().map(|v| v.round() as i64))
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct MatchAlliance {
    /// TBA reports `-1` for an unplayed match.
    #[serde(default = "minus_one")]
    pub score: i64,
    #[serde(default)]
    pub team_keys: Vec<String>,
}

fn minus_one() -> i64 {
    -1
}

/// One entry of `/event/{key}/matches`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Match {
    pub key: String,
    #[serde(default)]
    pub comp_level: String,
    #[serde(default = "one")]
    pub set_number: i32,
    #[serde(default)]
    pub match_number: i32,
    #[serde(default)]
    pub alliances: MatchAlliances,
    /// Unix seconds. `0` or absent means unknown.
    #[serde(default)]
    pub time: i64,
    #[serde(default)]
    pub actual_time: i64,
    #[serde(default)]
    pub predicted_time: i64,
    #[serde(default)]
    pub score_breakdown: Option<serde_json::Value>,
}

fn one() -> i32 {
    1
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct MatchAlliances {
    #[serde(default)]
    pub red: MatchAlliance,
    #[serde(default)]
    pub blue: MatchAlliance,
}

/// Which alliance won, or `None` for a tie or an unplayed match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Winner {
    Red,
    Blue,
}

impl Winner {
    pub fn as_str(self) -> &'static str {
        match self {
            Winner::Red => "red",
            Winner::Blue => "blue",
        }
    }
}

impl Match {
    pub fn comp_level(&self) -> Option<CompLevel> {
        CompLevel::parse(&self.comp_level)
    }

    /// Whether the match has actually been played.
    ///
    /// True if it has an actual start time, **or** it has a score breakdown and
    /// both scores are non-negative. The second clause matters because TBA
    /// sometimes has results before it has timing.
    pub fn played(&self) -> bool {
        if self.actual_time > 0 {
            return true;
        }
        let has_breakdown = self
            .score_breakdown
            .as_ref()
            .is_some_and(|value| !value.is_null());
        has_breakdown && self.alliances.red.score >= 0 && self.alliances.blue.score >= 0
    }

    pub fn winner(&self) -> Option<Winner> {
        let (red, blue) = (self.alliances.red.score, self.alliances.blue.score);
        if red < 0 || blue < 0 {
            return None;
        }
        match red.cmp(&blue) {
            std::cmp::Ordering::Greater => Some(Winner::Red),
            std::cmp::Ordering::Less => Some(Winner::Blue),
            std::cmp::Ordering::Equal => None,
        }
    }

    /// Score, or `None` when unplayed. Keeps TBA's `-1` sentinel out of storage.
    pub fn red_score(&self) -> Option<i64> {
        (self.alliances.red.score >= 0).then_some(self.alliances.red.score)
    }

    pub fn blue_score(&self) -> Option<i64> {
        (self.alliances.blue.score >= 0).then_some(self.alliances.blue.score)
    }

    /// The three red robots, by team number, padded with `None`.
    pub fn red_teams(&self) -> [Option<i32>; 3] {
        slots(&self.alliances.red.team_keys)
    }

    pub fn blue_teams(&self) -> [Option<i32>; 3] {
        slots(&self.alliances.blue.team_keys)
    }

    /// Best available scheduled time, in Unix seconds: the real start if known,
    /// then TBA's prediction, then the published schedule.
    pub fn scheduled_unix(&self) -> Option<i64> {
        [self.actual_time, self.predicted_time, self.time]
            .into_iter()
            .find(|t| *t > 0)
    }

    pub fn actual_unix(&self) -> Option<i64> {
        (self.actual_time > 0).then_some(self.actual_time)
    }
}

fn slots(keys: &[String]) -> [Option<i32>; 3] {
    let mut out = [None; 3];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = keys.get(i).and_then(|k| team_number(k));
    }
    out
}

/// `"frc1234"` -> `1234`. Case-insensitive; anything else is `None`.
pub fn team_number(team_key: &str) -> Option<i32> {
    let trimmed = team_key.trim();
    let digits = trimmed
        .strip_prefix("frc")
        .or_else(|| trimmed.strip_prefix("FRC"))?;
    digits.parse().ok()
}

/// `1234` -> `"frc1234"`.
pub fn team_key(team_number: i32) -> String {
    format!("frc{team_number}")
}

/// Ensure an event key carries its season prefix: `"mabil"` -> `"2026mabil"`.
///
/// Idempotent, so a key that already has one is returned unchanged.
pub fn normalize_event_key(raw: &str, season: i32) -> Option<String> {
    let key = raw.trim().to_ascii_lowercase();
    if key.is_empty() {
        return None;
    }
    let prefix = season.to_string();
    Some(if key.starts_with(&prefix) {
        key
    } else {
        format!("{prefix}{key}")
    })
}

/// Split `"2026mabil"` into its season and event code.
pub fn split_event_key(key: &str) -> Option<(i32, String)> {
    let key = key.trim().to_ascii_lowercase();
    if key.len() < 5 {
        return None;
    }
    let (year, code) = key.split_at(4);
    let year: i32 = year.parse().ok()?;
    (!code.is_empty()).then_some((year, code.to_string()))
}

// ── FIRST Events API ────────────────────────────────────────────────────────

/// One entry of `/{season}/events`.
///
/// FIRST uses PascalCase; `rename_all` maps it rather than annotating each field.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FirstEvent {
    #[serde(default)]
    pub code: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub venue: String,
    #[serde(default)]
    pub city: String,
    #[serde(default)]
    pub stateprov: String,
    #[serde(default)]
    pub country: String,
    #[serde(default)]
    pub timezone: Option<String>,
    #[serde(default)]
    pub date_start: String,
    #[serde(default)]
    pub date_end: String,
    #[serde(default)]
    pub event_type: Option<String>,
    #[serde(default)]
    pub district_code: Option<String>,
    #[serde(default)]
    pub week_number: Option<i32>,
}

impl FirstEvent {
    /// `"venue, city, state, country"`, skipping blanks.
    pub fn location(&self) -> String {
        [&self.venue, &self.city, &self.stateprov, &self.country]
            .iter()
            .map(|part| part.trim())
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// The TBA-style key for this event, derived from its start year and code.
    pub fn tba_key(&self) -> Option<String> {
        let code = self.code.trim().to_ascii_lowercase();
        if code.is_empty() {
            return None;
        }
        let year = parse_date(&self.date_start)?.0;
        Some(format!("{year}{code}"))
    }
}

/// One entry of `/{season}/teams`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FirstTeam {
    pub team_number: i32,
    #[serde(default)]
    pub name_full: Option<String>,
    #[serde(default)]
    pub name_short: Option<String>,
    #[serde(default)]
    pub school_name: Option<String>,
    #[serde(default)]
    pub city: Option<String>,
    #[serde(default)]
    pub state_prov: Option<String>,
    #[serde(default)]
    pub country: Option<String>,
    #[serde(default)]
    pub rookie_year: Option<i32>,
    #[serde(default)]
    pub website: Option<String>,
}

impl FirstTeam {
    /// Display name: the short name if there is one, else the full name, else
    /// the team number. Never empty -- the schema requires a name.
    pub fn display_name(&self) -> String {
        let candidates = [self.name_short.as_deref(), self.name_full.as_deref()];
        candidates
            .into_iter()
            .flatten()
            .map(str::trim)
            .find(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("Team {}", self.team_number))
    }
}

/// Parse a FIRST date, which appears in three shapes across endpoints.
///
/// Returns `(year, month, day)`. Deliberately not a chrono type: the caller
/// stores dates as ISO strings, and this keeps the parsing itself trivial to
/// test.
pub fn parse_date(raw: &str) -> Option<(i32, u32, u32)> {
    let trimmed = raw.trim().trim_matches('"');
    if trimmed.len() < 10 {
        return None;
    }
    // All three shapes -- "YYYY-MM-DD", "YYYY-MM-DDTHH:MM:SS", and RFC3339 --
    // agree on the first ten characters.
    let date = &trimmed[..10];
    let mut parts = date.split('-');
    let year: i32 = parts.next()?.parse().ok()?;
    let month: u32 = parts.next()?.parse().ok()?;
    let day: u32 = parts.next()?.parse().ok()?;
    (1..=12).contains(&month).then_some(())?;
    (1..=31).contains(&day).then_some(())?;
    Some((year, month, day))
}

/// Format a parsed date back to ISO `YYYY-MM-DD` for storage.
pub fn iso_date(parts: (i32, u32, u32)) -> String {
    format!("{:04}-{:02}-{:02}", parts.0, parts.1, parts.2)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Team keys ───────────────────────────────────────────────────────────

    #[test]
    fn team_keys_parse_both_cases_and_reject_junk() {
        assert_eq!(team_number("frc10101"), Some(10101));
        assert_eq!(team_number("FRC254"), Some(254));
        assert_eq!(team_number("  frc1  "), Some(1));
        assert_eq!(team_number("10101"), None);
        assert_eq!(team_number("frcABC"), None);
        assert_eq!(team_number(""), None);
    }

    #[test]
    fn team_keys_round_trip() {
        assert_eq!(team_number(&team_key(10101)), Some(10101));
    }

    // ── Event keys ──────────────────────────────────────────────────────────

    #[test]
    fn event_keys_gain_a_season_prefix_only_once() {
        assert_eq!(normalize_event_key("mabil", 2026).unwrap(), "2026mabil");
        assert_eq!(normalize_event_key("2026mabil", 2026).unwrap(), "2026mabil");
        assert_eq!(normalize_event_key("MABIL", 2026).unwrap(), "2026mabil");
        assert_eq!(normalize_event_key("   ", 2026), None);
    }

    #[test]
    fn event_keys_split_into_season_and_code() {
        assert_eq!(split_event_key("2026mabil"), Some((2026, "mabil".into())));
        assert_eq!(split_event_key("2026"), None);
        assert_eq!(split_event_key("abc"), None);
    }

    // ── Component OPRs (the dynamic-name problem) ───────────────────────────

    /// The 2026 shape: components are named per season, not fixed.
    fn coprs_2026() -> ComponentOprs {
        serde_json::from_str(
            r#"{
              "totalAutoPoints":    {"frc6328": 20.1572, "frc254": 15.0},
              "totalTeleopPoints":  {"frc6328": 84.3,    "frc254": 70.5},
              "totalEndgamePoints": {"frc6328": 12.9,    "frc254": 9.25}
            }"#,
        )
        .expect("parse")
    }

    #[test]
    fn component_oprs_are_found_by_dynamic_name() {
        // Reading fixed field names is what produced nulls for every component
        // OPR in the retired implementation.
        let c = coprs_2026();
        assert_eq!(c.phase_opr("frc6328", Phase::Auto), Some(20.1572));
        assert_eq!(c.phase_opr("frc6328", Phase::Teleop), Some(84.3));
        assert_eq!(c.phase_opr("frc6328", Phase::Endgame), Some(12.9));
    }

    #[test]
    fn component_oprs_return_all_three_phases_at_once() {
        assert_eq!(
            coprs_2026().phases("frc254"),
            (Some(15.0), Some(70.5), Some(9.25))
        );
    }

    #[test]
    fn a_team_absent_from_the_component_data_yields_none() {
        assert_eq!(coprs_2026().phase_opr("frc9999", Phase::Auto), None);
    }

    #[test]
    fn a_points_component_is_preferred_over_a_bare_phase_match() {
        let c: ComponentOprs = serde_json::from_str(
            r#"{"autoMobility": {"frc1": 1.0}, "totalAutoPoints": {"frc1": 9.0}}"#,
        )
        .expect("parse");
        assert_eq!(c.phase_opr("frc1", Phase::Auto), Some(9.0));
    }

    #[test]
    fn a_missing_phase_never_borrows_another_phases_number() {
        let c: ComponentOprs =
            serde_json::from_str(r#"{"totalTeleopPoints": {"frc1": 50.0}}"#).expect("parse");
        assert_eq!(c.phase_opr("frc1", Phase::Teleop), Some(50.0));
        assert_eq!(c.phase_opr("frc1", Phase::Endgame), None);
        assert_eq!(c.phase_opr("frc1", Phase::Auto), None);
    }

    #[test]
    fn empty_component_data_is_not_an_error() {
        let c: ComponentOprs = serde_json::from_str("{}").expect("parse");
        assert_eq!(c.phases("frc1"), (None, None, None));
    }

    // ── Rankings (the nullable-primitive problem) ───────────────────────────

    /// A 2026-shaped ranking: primitives null, numbers in the arrays.
    fn ranking_2026() -> Ranking {
        serde_json::from_str(
            r#"{
              "team_key": "frc6328",
              "rank": 3,
              "matches_played": 12,
              "dq": 0,
              "record": {"wins": 9, "losses": 3, "ties": 0},
              "qual_average": null,
              "qual_points": null,
              "total_points": null,
              "sort_orders": [12.0, 171.0, 55.5],
              "extra_stats": [12.0]
            }"#,
        )
        .expect("parse")
    }

    #[test]
    fn modern_rankings_read_their_numbers_out_of_the_arrays() {
        // Direct primitive access here returns null/zero, which is what made the
        // retired app show qual_points=0 for a team ranked 3rd.
        let r = ranking_2026();
        assert_eq!(r.effective_qual_average(), Some(12.0));
        assert_eq!(r.effective_avg_match_points(), Some(171.0));
        assert_eq!(r.effective_total_points(), Some(12));
        assert_eq!(r.effective_qual_points(), Some(12));
    }

    #[test]
    fn legacy_rankings_still_prefer_their_primitives() {
        let r: Ranking = serde_json::from_str(
            r#"{
              "team_key": "frc254",
              "qual_average": 42.5,
              "qual_points": 30,
              "total_points": 88,
              "sort_orders": [1.0, 2.0],
              "extra_stats": [3.0]
            }"#,
        )
        .expect("parse");

        assert_eq!(r.effective_qual_average(), Some(42.5));
        assert_eq!(r.effective_qual_points(), Some(30));
        assert_eq!(r.effective_total_points(), Some(88));
        // No primitive exists for this one, so it comes from the array either way.
        assert_eq!(r.effective_avg_match_points(), Some(2.0));
    }

    #[test]
    fn a_ranking_with_neither_primitives_nor_arrays_yields_none() {
        let r: Ranking = serde_json::from_str(r#"{"team_key": "frc1"}"#).expect("parse");
        assert_eq!(r.effective_qual_average(), None);
        assert_eq!(r.effective_avg_match_points(), None);
        assert_eq!(r.effective_total_points(), None);
        assert_eq!(r.effective_qual_points(), None);
    }

    #[test]
    fn a_single_element_sort_orders_has_no_average_match_points() {
        let r: Ranking =
            serde_json::from_str(r#"{"team_key": "frc1", "sort_orders": [5.0]}"#).expect("parse");
        assert_eq!(r.effective_qual_average(), Some(5.0));
        assert_eq!(r.effective_avg_match_points(), None);
    }

    #[test]
    fn ranking_records_survive_a_missing_record_object() {
        let r: Ranking = serde_json::from_str(r#"{"team_key": "frc1"}"#).expect("parse");
        assert_eq!((r.record.wins, r.record.losses, r.record.ties), (0, 0, 0));
    }

    // ── Matches ─────────────────────────────────────────────────────────────

    fn played_match() -> Match {
        serde_json::from_str(
            r#"{
              "key": "2026mabil_qm14",
              "comp_level": "qm",
              "set_number": 1,
              "match_number": 14,
              "time": 1773500000,
              "actual_time": 1773500400,
              "score_breakdown": {"red": {}, "blue": {}},
              "alliances": {
                "red":  {"score": 88, "team_keys": ["frc10101", "frc254", "frc1"]},
                "blue": {"score": 74, "team_keys": ["frc2", "frc3", "frc4"]}
              }
            }"#,
        )
        .expect("parse")
    }

    fn unplayed_match() -> Match {
        serde_json::from_str(
            r#"{
              "key": "2026mabil_qm40",
              "comp_level": "qm",
              "match_number": 40,
              "time": 1773600000,
              "alliances": {
                "red":  {"score": -1, "team_keys": ["frc10101", "frc5", "frc6"]},
                "blue": {"score": -1, "team_keys": ["frc7", "frc8", "frc9"]}
              }
            }"#,
        )
        .expect("parse")
    }

    #[test]
    fn a_played_match_reports_its_scores_and_winner() {
        let m = played_match();
        assert!(m.played());
        assert_eq!(m.winner(), Some(Winner::Red));
        assert_eq!(m.red_score(), Some(88));
        assert_eq!(m.blue_score(), Some(74));
    }

    #[test]
    fn an_unplayed_match_has_no_scores_rather_than_minus_one() {
        // TBA's -1 sentinel must not reach the database as a score.
        let m = unplayed_match();
        assert!(!m.played());
        assert_eq!(m.winner(), None);
        assert_eq!(m.red_score(), None);
        assert_eq!(m.blue_score(), None);
    }

    #[test]
    fn a_tie_has_no_winner() {
        let mut m = played_match();
        m.alliances.blue.score = 88;
        assert_eq!(m.winner(), None);
    }

    #[test]
    fn results_without_timing_still_count_as_played() {
        // TBA sometimes publishes a breakdown before it publishes actual_time.
        let mut m = played_match();
        m.actual_time = 0;
        assert!(m.played(), "a scored breakdown means the match happened");
    }

    #[test]
    fn a_null_score_breakdown_does_not_count_as_played() {
        let m: Match = serde_json::from_str(
            r#"{"key": "k", "score_breakdown": null,
                "alliances": {"red": {"score": 5}, "blue": {"score": 3}}}"#,
        )
        .expect("parse");
        assert!(!m.played());
    }

    #[test]
    fn alliance_slots_are_team_numbers_padded_to_three() {
        let m = played_match();
        assert_eq!(m.red_teams(), [Some(10101), Some(254), Some(1)]);
        assert_eq!(m.blue_teams(), [Some(2), Some(3), Some(4)]);
    }

    #[test]
    fn a_short_alliance_pads_with_none_rather_than_panicking() {
        // Surrogate and no-show situations produce two-robot alliances.
        let m: Match = serde_json::from_str(
            r#"{"key": "k", "alliances": {"red": {"score": -1, "team_keys": ["frc1"]},
                                          "blue": {"score": -1, "team_keys": []}}}"#,
        )
        .expect("parse");
        assert_eq!(m.red_teams(), [Some(1), None, None]);
        assert_eq!(m.blue_teams(), [None, None, None]);
    }

    #[test]
    fn scheduled_time_prefers_actual_then_predicted_then_published() {
        let mut m = played_match();
        assert_eq!(m.scheduled_unix(), Some(1773500400)); // actual

        m.actual_time = 0;
        m.predicted_time = 1773500100;
        assert_eq!(m.scheduled_unix(), Some(1773500100)); // predicted

        m.predicted_time = 0;
        assert_eq!(m.scheduled_unix(), Some(1773500000)); // published

        m.time = 0;
        assert_eq!(m.scheduled_unix(), None);
    }

    #[test]
    fn actual_time_is_only_reported_when_real() {
        assert_eq!(played_match().actual_unix(), Some(1773500400));
        assert_eq!(unplayed_match().actual_unix(), None);
    }

    #[test]
    fn comp_level_comes_through_the_shared_parser() {
        assert_eq!(played_match().comp_level(), Some(CompLevel::Qualification));
    }

    #[test]
    fn a_playoff_match_keeps_its_set_number() {
        let m: Match = serde_json::from_str(
            r#"{"key": "2026mabil_sf2m1", "comp_level": "sf",
                "set_number": 2, "match_number": 1, "alliances": {}}"#,
        )
        .expect("parse");
        // The retired schema collapsed these into set*100 + number to force a
        // unique integer; here all three stay separate and honest.
        assert_eq!(m.comp_level(), Some(CompLevel::Semifinal));
        assert_eq!(m.set_number, 2);
        assert_eq!(m.match_number, 1);
    }

    // ── FIRST ───────────────────────────────────────────────────────────────

    fn first_event() -> FirstEvent {
        serde_json::from_str(
            r#"{
              "code": "MABIL",
              "name": "Greater Boston Regional",
              "venue": "Reggie Lewis Center",
              "city": "Boston",
              "stateprov": "MA",
              "country": "USA",
              "dateStart": "2026-03-12T00:00:00",
              "dateEnd": "2026-03-15T00:00:00",
              "weekNumber": 3
            }"#,
        )
        .expect("parse")
    }

    #[test]
    fn first_events_build_a_location_and_a_tba_key() {
        let e = first_event();
        assert_eq!(e.location(), "Reggie Lewis Center, Boston, MA, USA");
        assert_eq!(e.tba_key().as_deref(), Some("2026mabil"));
    }

    #[test]
    fn location_skips_blank_parts_without_leaving_stray_commas() {
        let e = FirstEvent {
            city: "Boston".into(),
            country: "USA".into(),
            ..Default::default()
        };
        assert_eq!(e.location(), "Boston, USA");
    }

    #[test]
    fn an_event_without_a_code_has_no_key() {
        let e = FirstEvent {
            date_start: "2026-03-12".into(),
            ..Default::default()
        };
        assert_eq!(e.tba_key(), None);
    }

    #[test]
    fn first_dates_parse_in_all_three_shapes() {
        assert_eq!(parse_date("2026-03-12"), Some((2026, 3, 12)));
        assert_eq!(parse_date("2026-03-12T09:30:00"), Some((2026, 3, 12)));
        assert_eq!(parse_date("2026-03-12T09:30:00-05:00"), Some((2026, 3, 12)));
        assert_eq!(parse_date("\"2026-03-12\""), Some((2026, 3, 12)));
    }

    #[test]
    fn nonsense_dates_are_rejected() {
        for bad in ["", "not a date", "2026-13-01", "2026-03-99", "2026-3-1"] {
            assert_eq!(parse_date(bad), None, "{bad:?} should not parse");
        }
    }

    #[test]
    fn dates_format_back_to_iso_with_padding() {
        assert_eq!(iso_date((2026, 3, 1)), "2026-03-01");
    }

    #[test]
    fn team_names_fall_back_through_short_full_then_number() {
        let full = FirstTeam {
            team_number: 10101,
            name_short: Some("Teal Team".into()),
            name_full: Some("Some Very Long Sponsor List".into()),
            ..Default::default()
        };
        assert_eq!(full.display_name(), "Teal Team");

        let only_full = FirstTeam {
            team_number: 10101,
            name_short: Some("   ".into()),
            name_full: Some("Sponsors & School".into()),
            ..Default::default()
        };
        assert_eq!(only_full.display_name(), "Sponsors & School");

        let neither = FirstTeam {
            team_number: 10101,
            ..Default::default()
        };
        assert_eq!(neither.display_name(), "Team 10101");
    }
}
