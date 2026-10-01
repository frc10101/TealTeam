//! The event the simulated scouts work at.
//!
//! Written straight into the database, as a sync would leave it: an event
//! running today, a roster, and a qualification schedule long enough that no
//! scout runs out of matches. Each scout keeps one observation per robot per
//! match, so a two-hour run at one save every 15 seconds needs 480 matches.

use anyhow::Context;
use chrono::{TimeDelta, Utc};
use tt_core::matches::CompLevel;
use tt_core::records::{Event, MatchRecord, Team};
use tt_repo::Repo;
use tt_repo_sqlite::SqliteRepo;

pub const EVENT: &str = "2026load";
/// The roster: 36 teams, numbered clear of anything real.
pub const FIRST_TEAM: i32 = 9001;
pub const TEAMS: i32 = 36;

/// Match `n`'s key.
pub fn match_key(n: u32) -> String {
    format!("{EVENT}_qm{n}")
}

/// The six robots in match `n`, red then blue: the roster in turn, so every
/// team plays and no team plays itself.
pub fn robots(n: u32) -> [i32; 6] {
    std::array::from_fn(|slot| FIRST_TEAM + ((n as i32 * 6 + slot as i32) % TEAMS))
}

/// Migrate `database_url` and store the event with `matches` matches. Safe to
/// run again: everything is an upsert.
pub async fn seed(database_url: &str, matches: u32) -> anyhow::Result<()> {
    let repo = SqliteRepo::connect(database_url)?;
    tt_repo_sqlite::migrate::apply(repo.pool())
        .await
        .context("applying migrations")?;

    let now = Utc::now();
    let today = now.date_naive();
    let event = Event {
        key: EVENT.into(),
        name: "Load Test".into(),
        location: None,
        timezone: None,
        start_date: Some(today - TimeDelta::days(1)),
        end_date: Some(today + TimeDelta::days(1)),
        event_code: None,
        event_type: None,
        district_key: None,
        week: None,
    };
    repo.upsert_event(&event, now).await?;

    for number in FIRST_TEAM..FIRST_TEAM + TEAMS {
        let team = Team {
            number,
            name: format!("Load {number}"),
            nickname: None,
            school: None,
            city: None,
            state: None,
            country: None,
            rookie_year: None,
            website: None,
        };
        repo.upsert_team(&team, now).await?;
        repo.link_event_team(EVENT, number, now).await?;
    }

    for n in 1..=matches {
        let [r1, r2, r3, b1, b2, b3] = robots(n);
        let record = MatchRecord {
            key: match_key(n),
            event_key: EVENT.into(),
            comp_level: CompLevel::Qualification,
            set_number: 1,
            match_number: n as i32,
            red: [Some(r1), Some(r2), Some(r3)],
            blue: [Some(b1), Some(b2), Some(b3)],
            red_score: None,
            blue_score: None,
            winner: None,
            played: false,
            scheduled_at: None,
            actual_at: None,
        };
        repo.upsert_match(&record, now).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_match_has_six_different_teams_from_the_roster() {
        for n in 1..=500 {
            let teams = robots(n);
            let mut sorted = teams;
            sorted.sort_unstable();
            sorted
                .windows(2)
                .for_each(|w| assert_ne!(w[0], w[1], "Q{n}"));
            assert!(
                teams
                    .iter()
                    .all(|t| (FIRST_TEAM..FIRST_TEAM + TEAMS).contains(t))
            );
        }
    }

    #[tokio::test]
    async fn seeding_twice_leaves_one_schedule() {
        let dir = std::env::temp_dir().join(format!("tt-load-seed-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let url = format!("sqlite://{}/t.db?mode=rwc", dir.display());
        seed(&url, 10).await.unwrap();
        seed(&url, 10).await.unwrap();

        let repo = SqliteRepo::connect(&url).unwrap();
        let found = repo.match_by_key(&match_key(10)).await.unwrap().unwrap();
        assert_eq!(found.red[0], Some(robots(10)[0]));
        assert_eq!(found.alliance_of(robots(10)[5]), Some("blue"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
