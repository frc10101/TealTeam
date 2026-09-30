//! Which clock an event runs on (Q5).
//!
//! Everything is stored in UTC. An event's own zone decides two things: what
//! "today" is there, and how a match time reads on its schedule. The server's
//! zone never enters into it (docs/TIMEZONE_HANDLING.md).
//!
//! FIRST does not send IANA names. It sends Windows ones -- `"Central Standard
//! Time"` -- and not always the right one: in 2026 it gave Perth, Sanya, and
//! Trabzon `"Eastern Standard Time"`, and Arizona, which keeps no daylight
//! time, plain `"Mountain Standard Time"`. So [`event_zone`] reads the country
//! and state first wherever those settle the question, and the Windows name
//! only where they do not.

use chrono::{DateTime, NaiveDate, Timelike, Utc};
use chrono_tz::{OffsetName, Tz};

/// The IANA zone for an event, from FIRST's `timezone`, `country`, and
/// `stateprov`. An IANA name is taken as it is. `None` when nothing fits.
pub fn event_zone(timezone: &str, country: &str, stateprov: &str) -> Option<Tz> {
    let timezone = timezone.trim();
    if let Ok(tz) = timezone.parse::<Tz>() {
        return Some(tz);
    }
    let state = stateprov.trim().to_ascii_uppercase();
    by_place(country.trim(), &state)
        .or_else(|| by_windows_name(timezone, country.trim()))
        .and_then(|name| name.parse().ok())
}

/// Places whose zone the country, or the country and state, decide.
fn by_place(country: &str, state: &str) -> Option<&'static str> {
    Some(match (country, state) {
        ("China", _) => "Asia/Shanghai",
        ("Türkiye" | "Turkey", _) => "Europe/Istanbul",
        ("Israel", _) => "Asia/Jerusalem",
        ("Japan", _) => "Asia/Tokyo",
        ("Chinese Taipei" | "Taiwan", _) => "Asia/Taipei",
        ("USA", "AZ") => "America/Phoenix",
        ("USA", "HI") => "Pacific/Honolulu",
        ("Canada", "SK") => "America/Regina",
        ("Australia", "NSW" | "ACT") => "Australia/Sydney",
        ("Australia", "VIC") => "Australia/Melbourne",
        ("Australia", "QLD") => "Australia/Brisbane",
        ("Australia", "SA") => "Australia/Adelaide",
        ("Australia", "WA") => "Australia/Perth",
        ("Australia", "TAS") => "Australia/Hobart",
        ("Australia", "NT") => "Australia/Darwin",
        // Most of Mexico has kept central time all year since 2022.
        ("Mexico", "BCN") => "America/Tijuana",
        ("Mexico", "SON") => "America/Hermosillo",
        ("Mexico", "CHH") => "America/Chihuahua",
        ("Mexico", "BCS" | "SIN" | "NAY") => "America/Mazatlan",
        ("Mexico", "ROO") => "America/Cancun",
        ("Mexico", _) => "America/Mexico_City",
        _ => return None,
    })
}

/// Windows zone names, as CLDR maps them, for the countries FIRST uses them in.
fn by_windows_name(name: &str, country: &str) -> Option<&'static str> {
    let canada = country == "Canada";
    Some(match name {
        "Eastern Standard Time" if canada => "America/Toronto",
        "Eastern Standard Time" => "America/New_York",
        "Central Standard Time" if canada => "America/Winnipeg",
        "Central Standard Time" => "America/Chicago",
        "Mountain Standard Time" if canada => "America/Edmonton",
        "Mountain Standard Time" => "America/Denver",
        "US Mountain Standard Time" => "America/Phoenix",
        "Pacific Standard Time" if canada => "America/Vancouver",
        "Pacific Standard Time" => "America/Los_Angeles",
        "Alaskan Standard Time" => "America/Anchorage",
        "Hawaiian Standard Time" => "Pacific/Honolulu",
        "Atlantic Standard Time" => "America/Halifax",
        "Newfoundland Standard Time" => "America/St_Johns",
        "Canada Central Standard Time" => "America/Regina",
        "Central Standard Time (Mexico)" => "America/Mexico_City",
        "E. South America Standard Time" => "America/Sao_Paulo",
        "SA Pacific Standard Time" => "America/Bogota",
        "AUS Eastern Standard Time" => "Australia/Sydney",
        "China Standard Time" => "Asia/Shanghai",
        "Taipei Standard Time" => "Asia/Taipei",
        "Tokyo Standard Time" => "Asia/Tokyo",
        "Israel Standard Time" => "Asia/Jerusalem",
        "Turkey Standard Time" => "Europe/Istanbul",
        "GMT Standard Time" => "Europe/London",
        "W. Europe Standard Time" => "Europe/Berlin",
        "UTC" => "UTC",
        _ => return None,
    })
}

/// The calendar date at `now` in `zone`, or UTC's without one.
pub fn local_date(zone: Option<Tz>, now: DateTime<Utc>) -> NaiveDate {
    match zone {
        Some(tz) => now.with_timezone(&tz).date_naive(),
        None => now.date_naive(),
    }
}

/// `"1:30 PM CDT"`: `at` on `zone`'s clock, or `"18:30 UTC"` without one.
/// Assembled by hand: chrono's formatter needs its `alloc` feature.
pub fn clock_time(zone: Option<Tz>, at: DateTime<Utc>) -> String {
    let Some(tz) = zone else {
        return format!("{:02}:{:02} UTC", at.hour(), at.minute());
    };
    let local = at.with_timezone(&tz);
    let (pm, hour) = local.hour12();
    let abbreviation = local.offset().abbreviation();
    format!(
        "{hour}:{:02} {} {abbreviation}",
        local.minute(),
        if pm { "PM" } else { "AM" }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn zone(timezone: &str, country: &str, state: &str) -> Option<&'static str> {
        event_zone(timezone, country, state).map(|tz| tz.name())
    }

    #[test]
    fn an_iana_name_is_kept() {
        assert_eq!(zone("America/New_York", "", ""), Some("America/New_York"));
        assert_eq!(
            zone(" Europe/Istanbul ", "USA", "MA"),
            Some("Europe/Istanbul")
        );
    }

    #[test]
    fn firsts_windows_names_map_by_country() {
        // Every pairing FIRST sent for 2026, bar the wrong ones below.
        assert_eq!(
            zone("Eastern Standard Time", "USA", "MA"),
            Some("America/New_York")
        );
        assert_eq!(
            zone("Eastern Standard Time", "Canada", "ON"),
            Some("America/Toronto")
        );
        assert_eq!(
            zone("Central Standard Time", "USA", "MS"),
            Some("America/Chicago")
        );
        assert_eq!(
            zone("Mountain Standard Time", "USA", "CO"),
            Some("America/Denver")
        );
        assert_eq!(
            zone("Mountain Standard Time", "Canada", "AB"),
            Some("America/Edmonton")
        );
        assert_eq!(
            zone("Pacific Standard Time", "USA", "CA"),
            Some("America/Los_Angeles")
        );
        assert_eq!(
            zone("Pacific Standard Time", "Canada", "BC"),
            Some("America/Vancouver")
        );
        assert_eq!(
            zone("Hawaiian Standard Time", "USA", "HI"),
            Some("Pacific/Honolulu")
        );
        assert_eq!(
            zone("E. South America Standard Time", "Brazil", "SP"),
            Some("America/Sao_Paulo")
        );
        assert_eq!(
            zone("AUS Eastern Standard Time", "Australia", "VIC"),
            Some("Australia/Melbourne")
        );
        assert_eq!(
            zone("Israel Standard Time", "Israel", "TA"),
            Some("Asia/Jerusalem")
        );
        assert_eq!(
            zone("Turkey Standard Time", "Türkiye", "IST"),
            Some("Europe/Istanbul")
        );
        assert_eq!(
            zone("China Standard Time", "China", "SH"),
            Some("Asia/Shanghai")
        );
        assert_eq!(
            zone("Central Standard Time (Mexico)", "Mexico", "MEX"),
            Some("America/Mexico_City")
        );
    }

    #[test]
    fn the_place_wins_over_a_wrong_windows_name() {
        assert_eq!(
            zone("Eastern Standard Time", "Australia", "WA"),
            Some("Australia/Perth")
        );
        assert_eq!(
            zone("Eastern Standard Time", "China", "OTH"),
            Some("Asia/Shanghai")
        );
        assert_eq!(
            zone("Eastern Standard Time", "Türkiye", "TRB"),
            Some("Europe/Istanbul")
        );
        assert_eq!(
            zone("Mountain Standard Time", "USA", "AZ"),
            Some("America/Phoenix"),
            "Arizona keeps no daylight time"
        );
        assert_eq!(
            zone("Mountain Standard Time", "Mexico", "COA"),
            Some("America/Mexico_City")
        );
    }

    #[test]
    fn nothing_recognisable_is_none() {
        assert_eq!(zone("", "", ""), None);
        assert_eq!(zone("Martian Standard Time", "USA", "MA"), None);
    }

    #[test]
    fn the_last_evening_of_a_us_event_is_still_its_last_day() {
        // 8 PM in Chicago on Saturday is 1 AM Sunday in UTC.
        let now = Utc.with_ymd_and_hms(2026, 3, 22, 1, 0, 0).unwrap();
        let chicago = event_zone("America/Chicago", "", "");
        assert_eq!(
            local_date(chicago, now),
            NaiveDate::from_ymd_opt(2026, 3, 21).unwrap()
        );
        assert_eq!(
            local_date(None, now),
            NaiveDate::from_ymd_opt(2026, 3, 22).unwrap()
        );
    }

    #[test]
    fn a_match_time_reads_on_the_events_clock_across_daylight_time() {
        let chicago = event_zone("Central Standard Time", "USA", "MS");
        let winter = Utc.with_ymd_and_hms(2026, 3, 7, 19, 30, 0).unwrap();
        let summer = Utc.with_ymd_and_hms(2026, 3, 21, 18, 30, 0).unwrap();
        assert_eq!(clock_time(chicago, winter), "1:30 PM CST");
        assert_eq!(clock_time(chicago, summer), "1:30 PM CDT");
        let phoenix = event_zone("Mountain Standard Time", "USA", "AZ");
        assert_eq!(clock_time(phoenix, summer), "11:30 AM MST");
        assert_eq!(clock_time(None, summer), "18:30 UTC");
    }
}
