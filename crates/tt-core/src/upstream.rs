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

use serde::{Deserialize, Deserializer};
use std::collections::HashMap;

use crate::matches::CompLevel;

/// Upstream sends `null` where a field has nothing to say -- TBA does for a
/// match's `actual_time` until it is played -- and serde's `default` covers
/// only a field left out. Read `null` as left out, or one null fails a whole
/// event's sync.
fn or_default<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::deserialize(d)?.unwrap_or_default())
}

// ── The Blue Alliance ───────────────────────────────────────────────────────

/// `/event/{key}/oprs`
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Oprs {
    #[serde(default, deserialize_with = "or_default")]
    pub oprs: HashMap<String, f64>,
    #[serde(default, deserialize_with = "or_default")]
    pub dprs: HashMap<String, f64>,
    #[serde(default, deserialize_with = "or_default")]
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
    /// Of the components whose name contains the phase word, prefers one that
    /// mentions points, then one that says total, then the first by name.
    /// Never guesses across phases: a missing endgame component yields `None`,
    /// not a teleop number.
    ///
    /// The total matters. 2026 has both `autoTowerPoints` and `totalAutoPoints`,
    /// and taking the first points component found in a `HashMap` gave one or
    /// the other from run to run.
    pub fn phase_opr(&self, team_key: &str, phase: Phase) -> Option<f64> {
        let needle = phase.needle();
        self.components
            .iter()
            .filter_map(|(name, values)| {
                let lower = name.to_ascii_lowercase();
                let value = values.get(team_key).copied()?;
                lower.contains(needle).then(|| {
                    let rank = (lower.contains("point"), lower.contains("total"));
                    (rank, std::cmp::Reverse(name), value)
                })
            })
            .max_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)))
            .map(|(_, _, value)| value)
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
    #[serde(default, deserialize_with = "or_default")]
    pub wins: i32,
    #[serde(default, deserialize_with = "or_default")]
    pub losses: i32,
    #[serde(default, deserialize_with = "or_default")]
    pub ties: i32,
}

/// One row of `/event/{key}/rankings`.
///
/// The nullable primitives are the legacy schema; `sort_orders` and
/// `extra_stats` are where modern seasons put the same numbers.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Ranking {
    pub team_key: String,
    #[serde(default, deserialize_with = "or_default")]
    pub rank: i32,
    #[serde(default, deserialize_with = "or_default")]
    pub matches_played: i32,
    #[serde(default, deserialize_with = "or_default")]
    pub dq: i32,
    #[serde(default, deserialize_with = "or_default")]
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
    #[serde(default, deserialize_with = "or_default")]
    pub sort_orders: Vec<f64>,
    /// `[0]` an alternative total ranking points.
    #[serde(default, deserialize_with = "or_default")]
    pub extra_stats: Vec<f64>,
}

impl Ranking {
    /// Ranking score. Legacy primitive, else `sort_orders[0]`.
    pub fn effective_qual_average(&self) -> Option<f64> {
        self.qual_average
            .or_else(|| self.sort_orders.first().copied())
    }

    /// Average match points by position alone: `sort_orders[1]`. There is no
    /// legacy primitive for it, which is why the retired app displayed
    /// nothing. Position is right for 2026 and wrong for 2019, where `[1]` is
    /// cargo; [`Rankings::avg_match_points`] reads the column's name and only
    /// falls back to this when TBA sent none.
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

/// `/event/{key}/rankings`: the rows, and what each `sort_orders` column is.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Rankings {
    #[serde(default, deserialize_with = "or_default")]
    pub rankings: Vec<Ranking>,
    /// One per `sort_orders` column, in order. The columns are the season's
    /// tiebreakers, so they differ every year.
    #[serde(default, deserialize_with = "or_default")]
    pub sort_order_info: Vec<SortOrderInfo>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SortOrderInfo {
    #[serde(default, deserialize_with = "or_default")]
    pub name: String,
}

impl Rankings {
    /// The row for `team_key`, if it is ranked.
    pub fn for_team(&self, team_key: &str) -> Option<&Ranking> {
        self.rankings.iter().find(|r| r.team_key == team_key)
    }

    /// Average match points: the column TBA names "Avg Match". A season
    /// without one has none -- 2019 ranked on cargo and hatch panels -- rather
    /// than whatever sits in its place. With no names at all, position.
    pub fn avg_match_points(&self, ranking: &Ranking) -> Option<f64> {
        if self.sort_order_info.is_empty() {
            return ranking.effective_avg_match_points();
        }
        let column = self.sort_order_info.iter().position(|info| {
            let name = info.name.to_ascii_lowercase();
            name.contains("avg") && name.contains("match")
        })?;
        ranking.sort_orders.get(column).copied()
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct MatchAlliance {
    /// TBA reports `-1` for an unplayed match.
    #[serde(default = "minus_one", deserialize_with = "or_minus_one")]
    pub score: i64,
    #[serde(default, deserialize_with = "or_default")]
    pub team_keys: Vec<String>,
}

fn minus_one() -> i64 {
    -1
}

fn or_minus_one<'de, D: Deserializer<'de>>(d: D) -> Result<i64, D::Error> {
    Ok(Option::deserialize(d)?.unwrap_or_else(minus_one))
}

/// One entry of `/event/{key}/matches`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Match {
    pub key: String,
    #[serde(default, deserialize_with = "or_default")]
    pub comp_level: String,
    #[serde(default = "one", deserialize_with = "or_one")]
    pub set_number: i32,
    #[serde(default, deserialize_with = "or_default")]
    pub match_number: i32,
    #[serde(default, deserialize_with = "or_default")]
    pub alliances: MatchAlliances,
    /// Unix seconds. `0` or absent means unknown.
    #[serde(default, deserialize_with = "or_default")]
    pub time: i64,
    #[serde(default, deserialize_with = "or_default")]
    pub actual_time: i64,
    #[serde(default, deserialize_with = "or_default")]
    pub predicted_time: i64,
    #[serde(default)]
    pub score_breakdown: Option<serde_json::Value>,
}

fn one() -> i32 {
    1
}

fn or_one<'de, D: Deserializer<'de>>(d: D) -> Result<i32, D::Error> {
    Ok(Option::deserialize(d)?.unwrap_or_else(one))
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct MatchAlliances {
    #[serde(default, deserialize_with = "or_default")]
    pub red: MatchAlliance,
    #[serde(default, deserialize_with = "or_default")]
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
    #[serde(default, deserialize_with = "or_default")]
    pub code: String,
    #[serde(default, deserialize_with = "or_default")]
    pub name: String,
    #[serde(default, deserialize_with = "or_default")]
    pub venue: String,
    #[serde(default, deserialize_with = "or_default")]
    pub city: String,
    #[serde(default, deserialize_with = "or_default")]
    pub stateprov: String,
    #[serde(default, deserialize_with = "or_default")]
    pub country: String,
    #[serde(default)]
    pub timezone: Option<String>,
    #[serde(default, deserialize_with = "or_default")]
    pub date_start: String,
    #[serde(default, deserialize_with = "or_default")]
    pub date_end: String,
    /// `"Regional"`, `"ChampionshipDivision"`. FIRST calls it `type`, which
    /// `rename_all` would have looked for as `eventType`.
    #[serde(default, rename = "type")]
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
    /// Right for most events, wrong for about sixty a season: see
    /// [`tba_key_in`](Self::tba_key_in).
    pub fn tba_key(&self) -> Option<String> {
        let code = self.code.trim().to_ascii_lowercase();
        if code.is_empty() {
            return None;
        }
        let year = parse_date(&self.date_start)?.0;
        Some(format!("{year}{code}"))
    }

    /// This event's key at TBA: the one TBA lists under this FIRST code, else
    /// [`tba_key`](Self::tba_key). `known` is from [`tba_keys`].
    pub fn tba_key_in(&self, known: &HashMap<String, String>) -> Option<String> {
        let code = self.code.trim().to_ascii_lowercase();
        known.get(&code).cloned().or_else(|| self.tba_key())
    }
}

/// One entry of TBA's `/events/{year}`, as much as matching it to FIRST needs.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct TbaEvent {
    pub key: String,
    /// The event's code at FIRST, in whatever case TBA stored it.
    #[serde(default)]
    pub first_event_code: Option<String>,
}

/// FIRST code, lowercased, to TBA key, for every TBA event that names one.
///
/// The two agree on most events but not all (I15). In 2026 FIRST called the
/// Championship divisions `MILSTEIN` and `ARCHIMEDES` where TBA has `2026mil`
/// and `2026arc`, and some fifty offseason events differ outright: the
/// Arizona League's qualifiers are `AZGLE`-`AZGLE3` at FIRST and `2026azrl1`
/// to `2026azrl4` at TBA. A key built from FIRST's code 404s at TBA for all of
/// them.
pub fn tba_keys(events: &[TbaEvent]) -> HashMap<String, String> {
    events
        .iter()
        .filter_map(|e| {
            let code = e.first_event_code.as_deref()?.trim().to_ascii_lowercase();
            (!code.is_empty()).then(|| (code, e.key.clone()))
        })
        .collect()
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
    fn average_match_points_come_from_the_column_named_for_them() {
        let body = |names: &str| {
            format!(
                r#"{{"rankings": [{{"team_key": "frc1", "sort_orders": [2.5, 219.0, 81.3]}}],
                    "sort_order_info": {names}}}"#
            )
        };
        let moved: Rankings = serde_json::from_str(&body(
            r#"[{"name": "Ranking Score"}, {"name": "Cargo"}, {"name": "Avg Match"}]"#,
        ))
        .expect("parse");
        assert_eq!(moved.avg_match_points(&moved.rankings[0]), Some(81.3));

        let none: Rankings =
            serde_json::from_str(&body(r#"[{"name": "Ranking Score"}, {"name": "Cargo"}]"#))
                .expect("parse");
        assert_eq!(none.avg_match_points(&none.rankings[0]), None, "not cargo");

        let unnamed: Rankings = serde_json::from_str(&body("null")).expect("parse");
        assert_eq!(
            unnamed.avg_match_points(&unnamed.rankings[0]),
            Some(219.0),
            "no names: position, as before"
        );
        assert!(unnamed.for_team("frc1").is_some() && unnamed.for_team("frc2").is_none());
    }

    #[test]
    fn a_total_points_component_wins_and_the_choice_never_varies() {
        // 2026's auto components. Each new `HashMap` iterates in its own
        // random order, so building it many times tries many orders.
        for _ in 0..32 {
            let c = ComponentOprs {
                components: [
                    ("Hub Auto Fuel Count", 9.0),
                    ("autoTowerPoints", 0.0),
                    ("totalAutoPoints", 2.5),
                ]
                .into_iter()
                .map(|(name, v)| (name.to_string(), HashMap::from([("frc1".to_string(), v)])))
                .collect(),
            };
            assert_eq!(c.phase_opr("frc1", Phase::Auto), Some(2.5));
        }
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
    fn an_unplayed_match_with_null_times_parses_and_has_no_time() {
        // TBA's shape for a match not yet played. One of these failed the
        // whole event's match sync.
        let m: Match = serde_json::from_str(
            r#"{"key": "2026mabil_qm40", "comp_level": "qm", "set_number": null,
                "match_number": 40, "time": null, "actual_time": null,
                "predicted_time": null, "score_breakdown": null,
                "alliances": {"red": {"score": null, "team_keys": ["frc254"]},
                              "blue": {"score": -1, "team_keys": null}}}"#,
        )
        .expect("parse");
        assert_eq!(m.set_number, 1);
        assert_eq!((m.scheduled_unix(), m.actual_unix()), (None, None));
        assert!(!m.played());
        assert_eq!(
            (m.red_score(), m.blue_score(), m.winner()),
            (None, None, None)
        );
        assert_eq!(m.red_teams(), [Some(254), None, None]);
        assert_eq!(m.blue_teams(), [None, None, None]);
    }

    #[test]
    fn a_ranking_of_nulls_parses_and_every_fallback_is_none() {
        let r: Ranking = serde_json::from_str(
            r#"{"team_key": "frc254", "rank": null, "matches_played": null,
                "dq": null, "record": null, "qual_average": null,
                "sort_orders": null, "extra_stats": null}"#,
        )
        .expect("parse");
        assert_eq!((r.rank, r.matches_played, r.record.wins), (0, 0, 0));
        assert_eq!(r.effective_qual_average(), None);
        assert_eq!(r.effective_avg_match_points(), None);
        assert_eq!(r.effective_total_points(), None);
        assert_eq!(r.effective_qual_points(), None);
    }

    #[test]
    fn a_fallback_rounds_half_away_from_zero_rather_than_truncating() {
        let r: Ranking = serde_json::from_str(
            r#"{"team_key": "frc1", "sort_orders": [2.5, 31.6], "extra_stats": [17.5]}"#,
        )
        .expect("parse");
        assert_eq!(r.effective_qual_points(), Some(3));
        assert_eq!(r.effective_total_points(), Some(18));
        assert_eq!(r.effective_avg_match_points(), Some(31.6));
    }

    #[test]
    fn null_component_oprs_and_event_fields_read_as_empty() {
        let o: Oprs =
            serde_json::from_str(r#"{"oprs": null, "dprs": {}, "ccwms": null}"#).expect("parse");
        assert!(o.oprs.is_empty() && o.ccwms.is_empty());
        let e: FirstEvent = serde_json::from_str(
            r#"{"code": "MABIL", "venue": null, "city": "Boston", "stateprov": null,
                "country": "USA", "dateStart": "2026-03-12T00:00:00", "dateEnd": null}"#,
        )
        .expect("parse");
        assert_eq!(e.location(), "Boston, USA");
        assert_eq!(e.tba_key().as_deref(), Some("2026mabil"));
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
    fn a_first_code_resolves_to_the_key_tba_lists_it_under() {
        let tba: Vec<TbaEvent> = serde_json::from_str(
            r#"[{"key": "2026mil", "first_event_code": "milstein"},
                {"key": "2026nyro2", "first_event_code": "NYROC"},
                {"key": "2026mslr", "first_event_code": "mslr"},
                {"key": "2026cabl", "first_event_code": null},
                {"key": "2026x", "first_event_code": "  "}]"#,
        )
        .expect("parse");
        let known = tba_keys(&tba);
        assert_eq!(known.len(), 3, "an event without a FIRST code maps nothing");
        let first = |code: &str| FirstEvent {
            code: code.into(),
            date_start: "2026-04-29T00:00:00".into(),
            ..FirstEvent::default()
        };
        assert_eq!(
            first("MILSTEIN").tba_key_in(&known).as_deref(),
            Some("2026mil")
        );
        assert_eq!(
            first("NYROC").tba_key_in(&known).as_deref(),
            Some("2026nyro2"),
            "any case"
        );
        assert_eq!(
            first("MSLR").tba_key_in(&known).as_deref(),
            Some("2026mslr")
        );
        assert_eq!(
            first("WAPER").tba_key_in(&known).as_deref(),
            Some("2026waper"),
            "unknown to TBA: built from the code, as before"
        );
        assert_eq!(first("").tba_key_in(&known), None);
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
