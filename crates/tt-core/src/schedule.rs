//! A qualification schedule seeded by hand (FTC_BETA_PLAN.md, F3).
//!
//! With upstream switched off there is no sync to bring an event's teams and
//! matches, so the lead scout pastes the posted schedule in:
//!
//! ```text
//! match,red1,red2,blue1,blue2
//! 1,10001,10006,10002,10008
//! 4,10007,10008,10002*,10005
//! ```
//!
//! The header has one column per slot ([`slot_names`]), so it follows
//! [`ALLIANCE_SIZE`](crate::ALLIANCE_SIZE) rather than being written out here.
//! A `*` after a team number marks a surrogate: the robot plays, but the
//! match does not count for its team. The marker is stripped and the slot
//! listed in the report, since a surrogate match is still worth scouting.
//!
//! A roster is optional: `team,name` lines, the header optional. Without one
//! the roster is every team the schedule names. With one, a scheduled team
//! missing from it is refused, because that is a typo on one side or the
//! other.
//!
//! Every bad row is named, by line, with why, and nothing is imported until
//! there are none: half a schedule hands out the wrong robots.

use chrono::{DateTime, NaiveDate, Utc};

use crate::matches::{ALLIANCE_SIZE, Alliance, slot_names};
use crate::records::Event;

/// A schedule ready to store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schedule {
    /// In the order given.
    pub matches: Vec<ScheduledMatch>,
    /// By team number.
    pub roster: Vec<RosterTeam>,
    /// Every slot marked `*`, in schedule order.
    pub surrogates: Vec<Surrogate>,
    /// No roster was given, so it is the schedule's teams.
    pub roster_from_schedule: bool,
}

/// One qualification match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduledMatch {
    pub number: i32,
    pub red: Alliance,
    pub blue: Alliance,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RosterTeam {
    pub number: i32,
    /// `None` when the roster gave none, or there was no roster.
    pub name: Option<String>,
}

/// A slot marked `*`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Surrogate {
    pub match_number: i32,
    /// `"blue1"`.
    pub slot: String,
    pub team: i32,
}

impl Surrogate {
    /// `"Q4 blue1: 10002"`.
    pub fn describe(&self) -> String {
        format!("Q{} {}: {}", self.match_number, self.slot, self.team)
    }
}

/// `match,red1,…,blue<N>`: the header a schedule must start with.
pub fn header() -> String {
    std::iter::once("match".to_string())
        .chain(slot_names())
        .collect::<Vec<_>>()
        .join(",")
}

/// Cells of one CSV line, trimmed. No quoting: nothing in a schedule needs it.
fn cells(line: &str) -> Vec<&str> {
    line.split(',').map(str::trim).collect()
}

fn whole_number(raw: &str) -> Option<i32> {
    if raw.is_empty() || !raw.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    raw.parse().ok().filter(|n| *n > 0)
}

/// Read a schedule and, if not blank, a roster. Every problem at once, or
/// what to store.
pub fn parse(schedule: &str, roster: &str) -> Result<Schedule, Vec<String>> {
    let mut errors = Vec::new();
    let given_roster = parse_roster(roster, &mut errors);

    let slots = slot_names();
    let expected = header();
    let columns = slots.len() + 1;

    let mut lines = schedule
        .trim_start_matches('\u{feff}')
        .lines()
        .enumerate()
        .map(|(i, line)| (i + 1, line.trim()))
        .filter(|(_, line)| !line.is_empty());

    match lines.next() {
        None => {
            errors.push(format!(
                "The schedule is empty. Paste it with its header, {expected}."
            ));
            return Err(errors);
        }
        Some((n, line)) => {
            let found: Vec<String> = cells(line).iter().map(|c| c.to_lowercase()).collect();
            if found.join(",") != expected {
                errors.push(format!(
                    "Line {n}: the first line must be the header {expected}, not “{line}”."
                ));
                return Err(errors);
            }
        }
    }

    let mut matches: Vec<ScheduledMatch> = Vec::new();
    let mut seen: Vec<(i32, usize)> = Vec::new();
    let mut surrogates = Vec::new();

    for (n, line) in lines {
        let row = cells(line);
        if row.len() != columns {
            errors.push(format!(
                "Line {n}: expected {columns} columns ({expected}), found {}.",
                row.len()
            ));
            continue;
        }
        let Some(number) = whole_number(row[0]) else {
            errors.push(format!(
                "Line {n}: the match number “{}” is not a whole number.",
                row[0]
            ));
            continue;
        };
        let at = format!("Line {n} (Q{number})");
        let before = errors.len();

        if let Some((_, first)) = seen.iter().find(|(m, _)| *m == number) {
            errors.push(format!("{at}: match {number} is already on line {first}."));
        } else {
            seen.push((number, n));
        }

        let mut teams: Vec<Option<i32>> = Vec::with_capacity(slots.len());
        let mut marked = Vec::new();
        for (slot, raw) in slots.iter().zip(&row[1..]) {
            let (bare, surrogate) = match raw.strip_suffix('*') {
                Some(bare) => (bare.trim_end(), true),
                None => (*raw, false),
            };
            match whole_number(bare) {
                Some(team) => {
                    teams.push(Some(team));
                    if surrogate {
                        marked.push(Surrogate {
                            match_number: number,
                            slot: slot.clone(),
                            team,
                        });
                    }
                }
                None => {
                    errors.push(format!("{at}: {slot} “{raw}” is not a team number."));
                    teams.push(None);
                }
            }
        }

        for (i, team) in teams.iter().enumerate() {
            let Some(team) = team else { continue };
            if let Some(j) = teams[..i].iter().position(|t| t == &Some(*team)) {
                errors.push(format!(
                    "{at}: team {team} is in the match twice ({} and {}).",
                    slots[j], slots[i]
                ));
            }
            if let Some(roster) = &given_roster
                && !roster.iter().any(|t| t.number == *team)
            {
                errors.push(format!(
                    "{at}: team {team} ({}) is not on the roster.",
                    slots[i]
                ));
            }
        }

        if errors.len() == before {
            let alliance = |from: usize| -> Alliance { std::array::from_fn(|i| teams[from + i]) };
            matches.push(ScheduledMatch {
                number,
                red: alliance(0),
                blue: alliance(ALLIANCE_SIZE),
            });
            surrogates.extend(marked);
        }
    }

    if matches.is_empty() && errors.is_empty() {
        errors.push("The schedule has a header but no matches.".into());
    }
    if !errors.is_empty() {
        return Err(errors);
    }

    let roster_from_schedule = given_roster.is_none();
    let mut roster = given_roster.unwrap_or_else(|| {
        let mut numbers: Vec<i32> = matches
            .iter()
            .flat_map(|m| m.red.iter().chain(&m.blue).flatten().copied())
            .collect();
        numbers.sort_unstable();
        numbers.dedup();
        numbers
            .into_iter()
            .map(|number| RosterTeam { number, name: None })
            .collect()
    });
    roster.sort_by_key(|t| t.number);

    Ok(Schedule {
        matches,
        roster,
        surrogates,
        roster_from_schedule,
    })
}

/// `None` for a blank roster. Problems go to `errors`; the teams that were
/// readable are still returned, so the schedule is checked against them.
fn parse_roster(text: &str, errors: &mut Vec<String>) -> Option<Vec<RosterTeam>> {
    let mut lines = text
        .trim_start_matches('\u{feff}')
        .lines()
        .enumerate()
        .map(|(i, line)| (i + 1, line.trim()))
        .filter(|(_, line)| !line.is_empty())
        .peekable();
    let (_, first) = lines.peek()?;
    if cells(first)[0].eq_ignore_ascii_case("team") {
        lines.next();
    }

    let mut roster: Vec<(RosterTeam, usize)> = Vec::new();
    for (n, line) in lines {
        let (raw, name) = match line.split_once(',') {
            Some((raw, name)) => (raw.trim(), Some(name.trim()).filter(|s| !s.is_empty())),
            None => (line, None),
        };
        let Some(number) = whole_number(raw) else {
            errors.push(format!("Roster line {n}: “{raw}” is not a team number."));
            continue;
        };
        if let Some((_, first)) = roster.iter().find(|(t, _)| t.number == number) {
            errors.push(format!(
                "Roster line {n}: team {number} is already on line {first}."
            ));
            continue;
        }
        roster.push((
            RosterTeam {
                number,
                name: name.map(str::to_string),
            },
            n,
        ));
    }
    Some(roster.into_iter().map(|(t, _)| t).collect())
}

/// Longest event code and name kept.
pub const CODE_MAX: usize = 30;
pub const NAME_MAX: usize = 100;

/// The event a schedule is for, from the import form: a code, a name, an
/// IANA zone, and a date (blank is today, at the event).
///
/// The key is the code with the season in front, as an upstream event's is
/// (`USMIBIO` in 2026 is `2026usmibio`). `existing` is the event already
/// under that key, whose other details are kept.
pub fn event_from_form(
    code: &str,
    name: &str,
    timezone: &str,
    date: &str,
    season: i32,
    existing: Option<&Event>,
    now: DateTime<Utc>,
) -> Result<Event, Vec<String>> {
    let mut errors = Vec::new();
    let code = code.trim();
    let name = name.trim();
    let timezone = timezone.trim();

    if code.is_empty() {
        errors.push("Give the event a code, such as USMIBIO.".into());
    } else if code.len() > CODE_MAX || !code.bytes().all(|b| b.is_ascii_alphanumeric()) {
        errors.push(format!(
            "The event code “{code}” must be letters and digits only, at most {CODE_MAX}."
        ));
    }
    if name.is_empty() {
        errors.push("Give the event a name.".into());
    } else if name.chars().count() > NAME_MAX {
        errors.push(format!(
            "The event name is longer than {NAME_MAX} characters."
        ));
    }
    let zone: Option<chrono_tz::Tz> = timezone.parse().ok();
    if zone.is_none() {
        errors.push(format!(
            "“{timezone}” is not a time zone. Use its IANA name, such as America/Detroit."
        ));
    }
    let date = match date.trim() {
        "" => Some(crate::timezone::local_date(zone, now)),
        raw => {
            let parsed = parse_date(raw);
            if parsed.is_none() {
                errors.push(format!("“{raw}” is not a date. Use YYYY-MM-DD."));
            }
            parsed
        }
    };
    if !errors.is_empty() {
        return Err(errors);
    }

    let key = crate::upstream::normalize_event_key(code, season).unwrap_or_default();
    let mut event = existing.cloned().unwrap_or(Event {
        key: key.clone(),
        name: String::new(),
        location: None,
        timezone: None,
        start_date: None,
        end_date: None,
        event_code: None,
        event_type: None,
        district_key: None,
        week: None,
    });
    event.key = key;
    event.name = name.to_string();
    event.timezone = zone.map(|z| z.name().to_string());
    event.start_date = date;
    event.end_date = date;
    event.event_code = Some(code.to_ascii_lowercase());
    Ok(event)
}

/// `YYYY-MM-DD`, by hand: chrono's parser needs its `alloc` feature.
fn parse_date(raw: &str) -> Option<NaiveDate> {
    let mut parts = raw.split('-');
    let year = parts.next()?.parse().ok()?;
    let month = parts.next()?.parse().ok()?;
    let day = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    NaiveDate::from_ymd_opt(year, month, day)
}

/// A stored schedule as the import takes it, for correcting and re-running.
pub fn format(matches: &[(i32, Alliance, Alliance)]) -> String {
    let mut out = header();
    for (number, red, blue) in matches {
        out.push('\n');
        out.push_str(&number.to_string());
        for slot in red.iter().chain(blue) {
            out.push(',');
            if let Some(team) = slot {
                out.push_str(&team.to_string());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const REHEARSAL: &str = include_str!("../../../docs/ftc/FTC_BETA_REHEARSAL_SCHEDULE.csv");

    #[test]
    fn the_header_follows_the_alliance_size() {
        assert_eq!(header(), "match,red1,red2,blue1,blue2");
        assert_eq!(header().split(',').count(), 1 + 2 * ALLIANCE_SIZE);
    }

    #[test]
    fn the_rehearsal_schedule_reads_with_its_three_surrogates() {
        let schedule = parse(REHEARSAL, "").expect("clean");
        assert_eq!(schedule.matches.len(), 16);
        assert_eq!(schedule.roster.len(), 12);
        assert!(schedule.roster_from_schedule);
        let surrogates: Vec<String> = schedule
            .surrogates
            .iter()
            .map(Surrogate::describe)
            .collect();
        assert_eq!(
            surrogates,
            ["Q4 blue1: 10002", "Q7 blue2: 10009", "Q8 blue2: 10012"]
        );
        let q4 = &schedule.matches[3];
        assert_eq!(q4.number, 4);
        assert_eq!(
            q4.blue,
            [Some(10002), Some(10005)],
            "the marker is stripped"
        );
    }

    #[test]
    fn every_bad_row_is_named_with_why_and_nothing_is_kept() {
        let text = "match,red1,red2,blue1,blue2\n\
                    1,10001,10002,10003,10004\n\
                    2,10001,10002,10003\n\
                    3,10001,abc,10003,10004\n\
                    4,10001,10002,10001,10004\n\
                    1,10005,10006,10007,10008\n\
                    x,10001,10002,10003,10004\n";
        let errors = parse(text, "").unwrap_err();
        assert_eq!(
            errors,
            [
                "Line 3: expected 5 columns (match,red1,red2,blue1,blue2), found 4.",
                "Line 4 (Q3): red2 “abc” is not a team number.",
                "Line 5 (Q4): team 10001 is in the match twice (red1 and blue1).",
                "Line 6 (Q1): match 1 is already on line 2.",
                "Line 7: the match number “x” is not a whole number.",
            ]
        );
    }

    #[test]
    fn a_roster_names_teams_and_refuses_a_scheduled_team_it_lacks() {
        let roster = "team,name\n10001,Alpha\n10002\n10003,Gamma, Inc\n";
        let text = "match,red1,red2,blue1,blue2\n1,10001,10002,10003,10004\n";
        assert_eq!(
            parse(text, roster).unwrap_err(),
            ["Line 2 (Q1): team 10004 (blue2) is not on the roster."]
        );

        let roster = format!("{roster}10004,Delta\n10099\n");
        let schedule = parse(text, &roster).expect("clean");
        assert!(!schedule.roster_from_schedule);
        assert_eq!(schedule.roster.len(), 5, "a roster team need not play");
        assert_eq!(schedule.roster[2].name.as_deref(), Some("Gamma, Inc"));
        assert_eq!(schedule.roster[1].name, None);
    }

    #[test]
    fn a_bad_roster_line_is_named_too() {
        let text = "match,red1,red2,blue1,blue2\n1,10001,10002,10003,10004\n";
        let errors = parse(text, "10001\nten\n10001,Again\n").unwrap_err();
        assert!(errors.contains(&"Roster line 2: “ten” is not a team number.".to_string()));
        assert!(errors.contains(&"Roster line 3: team 10001 is already on line 1.".to_string()));
    }

    #[test]
    fn a_missing_header_or_an_empty_schedule_is_said() {
        let errors = parse("1,10001,10002,10003,10004", "").unwrap_err();
        assert!(errors[0].starts_with("Line 1: the first line must be the header"));
        assert!(parse("  \n", "").unwrap_err()[0].starts_with("The schedule is empty"));
        assert_eq!(
            parse("MATCH, Red1,red2,blue1,blue2\n", "").unwrap_err(),
            ["The schedule has a header but no matches."]
        );
    }

    #[test]
    fn an_event_is_its_code_under_the_season_in_its_own_zone() {
        let now = DateTime::parse_from_rfc3339("2026-10-24T03:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let event = event_from_form(
            "USMIBIO",
            "BIOBUZZ Qualifier",
            "America/Detroit",
            "",
            2026,
            None,
            now,
        )
        .unwrap();
        assert_eq!(event.key, "2026usmibio");
        assert_eq!(event.timezone.as_deref(), Some("America/Detroit"));
        assert_eq!(
            event.start_date,
            NaiveDate::from_ymd_opt(2026, 10, 23),
            "still Friday evening in Detroit"
        );

        let errors = event_from_form("us mi", "", "Eastern", "10/24", 2026, None, now).unwrap_err();
        assert_eq!(errors.len(), 4, "{errors:?}");
    }

    #[test]
    fn a_stored_schedule_formats_back_to_what_imports() {
        let text = format(&[(1, [Some(1), Some(2)], [Some(3), Some(4)])]);
        assert_eq!(text, "match,red1,red2,blue1,blue2\n1,1,2,3,4");
        assert_eq!(parse(&text, "").unwrap().matches.len(), 1);
    }
}
