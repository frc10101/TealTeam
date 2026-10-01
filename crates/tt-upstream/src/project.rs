//! The tables, derived from the `upstream` log (S5).
//!
//! A pushed bundle's responses are appended to the log first, then applied
//! from it, with the same parsers and the same storing code as the Pi's own
//! sync ([`sync::store_matches`], [`sync::store_stats`]). What is applied is
//! always the log's newest response for each path, not the bundle's, so a
//! bundle that brought only rankings is combined with the OPRs already
//! there.
//!
//! TBA's per-event matches, rankings, and OPRs are applied: the live data a
//! phone with signal is for. Anything else a bundle carries, such as season
//! lists and FIRST's rosters, is kept in the log, where clients get it, but
//! not applied here; the bulk load (I8) brings those before the event.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use tt_core::upstream::{ComponentOprs, Match, Oprs, Rankings};
use tt_repo::{Repo, UpstreamEntry};

use crate::sync::{self, SyncReport};

/// What applying some responses did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Projected {
    pub report: SyncReport,
    /// Responses kept in the log but not applied to any table here.
    pub logged_only: usize,
}

/// What an appended response feeds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Feed {
    Matches,
    Stats,
}

/// The event and table a TBA path feeds, if it is one applied here.
fn feed<'a>(api: &str, path: &'a str) -> Option<(&'a str, Feed)> {
    if api != "tba" {
        return None;
    }
    let (event, what) = path.strip_prefix("/event/")?.split_once('/')?;
    let feed = match what {
        "matches" => Feed::Matches,
        "oprs" | "rankings" | "coprs" => Feed::Stats,
        _ => return None,
    };
    Some((event, feed))
}

/// Apply the log's newest response for each `(api, path)` given, which are
/// the ones a bundle just appended.
pub async fn project<R: Repo + Sync>(repo: &R, appended: &[(String, String)]) -> Projected {
    let mut projected = Projected::default();
    // Per event: whether its matches, its stats, or both changed.
    let mut events: BTreeMap<&str, (bool, bool)> = BTreeMap::new();
    for (api, path) in appended {
        match feed(api, path) {
            Some((event, Feed::Matches)) => events.entry(event).or_default().0 = true,
            Some((event, Feed::Stats)) => events.entry(event).or_default().1 = true,
            None => projected.logged_only += 1,
        }
    }

    let report = &mut projected.report;
    for (event, (matches, stats)) in events {
        match repo.event(event).await {
            Ok(Some(_)) => {}
            Ok(None) => {
                report.problems.push(format!(
                    "{event} is not an event on this server; its responses are kept in the \
                     log but not applied"
                ));
                continue;
            }
            Err(e) => {
                report.problems.push(format!("reading event {event}: {e}"));
                continue;
            }
        }
        if matches {
            report.merge(project_matches(repo, event).await);
        }
        if stats {
            report.merge(project_stats(repo, event).await);
        }
    }
    projected
}

async fn latest<R: Repo + Sync>(
    repo: &R,
    event: &str,
    what: &str,
) -> std::result::Result<Option<UpstreamEntry>, String> {
    repo.latest_upstream("tba", &format!("/event/{event}/{what}"))
        .await
        .map_err(|e| format!("reading {event}'s {what} from the log: {e}"))
}

/// A body as the client would have read it: TBA's `null` is empty.
fn parse<T: DeserializeOwned + Default>(entry: &UpstreamEntry) -> std::result::Result<T, String> {
    serde_json::from_str::<Option<T>>(&entry.entry.body)
        .map(Option::unwrap_or_default)
        .map_err(|e| format!("{} could not be read: {e}", entry.entry.path))
}

async fn project_matches<R: Repo + Sync>(repo: &R, event: &str) -> SyncReport {
    let mut report = SyncReport::default();
    let parsed = latest(repo, event, "matches").await.and_then(|entry| {
        let entry = entry.ok_or_else(|| format!("{event}'s matches are not in the log"))?;
        Ok((parse::<Vec<Match>>(&entry)?, entry.entry.fetched_at))
    });
    match parsed {
        Ok((matches, fetched_at)) => {
            report.merge(sync::store_matches(repo, event, &matches, fetched_at).await)
        }
        Err(problem) => report.problems.push(problem),
    }
    report
}

async fn project_stats<R: Repo + Sync>(repo: &R, event: &str) -> SyncReport {
    let mut report = SyncReport::default();
    let read = async {
        let (Some(oprs), Some(rankings)) = (
            latest(repo, event, "oprs").await?,
            latest(repo, event, "rankings").await?,
        ) else {
            return Err(format!(
                "{event}'s stats need both its rankings and its OPRs, and the log has only one"
            ));
        };
        // Optional, as in the Pi's own sync.
        let components = match latest(repo, event, "coprs").await? {
            Some(entry) => Some(parse::<ComponentOprs>(&entry)?),
            None => None,
        };
        // As old as the older of the two it is made from.
        let fetched_at: DateTime<Utc> = oprs.entry.fetched_at.min(rankings.entry.fetched_at);
        Ok((
            parse::<Oprs>(&oprs)?,
            parse::<Rankings>(&rankings)?,
            components,
            fetched_at,
        ))
    };
    match read.await {
        Ok((oprs, rankings, components, fetched_at)) => {
            match sync::store_stats(
                repo,
                event,
                &oprs,
                &rankings,
                components.as_ref(),
                fetched_at,
            )
            .await
            {
                Ok(stored) => report.merge(stored),
                Err(e) => report
                    .problems
                    .push(format!("storing stats for {event}: {e}")),
            }
        }
        Err(problem) => report.problems.push(problem),
    }
    report
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use tt_core::records::{Event, Team};
    use tt_repo::NewUpstream;
    use tt_repo_sqlite::SqliteRepo;

    use super::*;

    fn at(hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 3, 14, hour, 0, 0).unwrap()
    }

    async fn repo() -> SqliteRepo {
        let repo = SqliteRepo::connect("sqlite::memory:").unwrap();
        tt_repo_sqlite::migrate::apply(repo.pool()).await.unwrap();
        let event = Event {
            key: "2026mslr".into(),
            name: "Magnolia".into(),
            location: None,
            timezone: None,
            start_date: None,
            end_date: None,
            event_code: None,
            event_type: None,
            district_key: None,
            week: None,
        };
        repo.upsert_event(&event, at(0)).await.unwrap();
        for number in [254, 1678] {
            let team = Team {
                number,
                name: format!("Team {number}"),
                nickname: None,
                school: None,
                city: None,
                state: None,
                country: None,
                rookie_year: None,
                website: None,
            };
            repo.upsert_team(&team, at(0)).await.unwrap();
            repo.link_event_team("2026mslr", number, at(0))
                .await
                .unwrap();
        }
        repo
    }

    async fn log(repo: &SqliteRepo, path: &str, body: &str, hour: u32) -> (String, String) {
        repo.append_upstream(&NewUpstream {
            api: "tba".into(),
            path: path.into(),
            etag: None,
            body: body.into(),
            fetched_at: at(hour),
            via: "bundle:1".into(),
        })
        .await
        .unwrap();
        ("tba".into(), path.into())
    }

    const MATCHES: &str = r#"[{"key":"2026mslr_qm1","comp_level":"qm","set_number":1,
        "match_number":1,"alliances":{"red":{"score":40,"team_keys":["frc254"]},
        "blue":{"score":30,"team_keys":["frc1678"]}},"winning_alliance":"red"}]"#;

    #[tokio::test]
    async fn a_bundles_matches_land_as_of_when_the_phone_fetched_them() {
        let repo = repo().await;
        let appended = vec![log(&repo, "/event/2026mslr/matches", MATCHES, 9).await];
        let projected = project(&repo, &appended).await;
        assert_eq!(
            projected.report.matches, 1,
            "{:?}",
            projected.report.problems
        );
        let stored = repo.match_by_key("2026mslr_qm1").await.unwrap().unwrap();
        assert_eq!(stored.red_score, Some(40));
    }

    #[tokio::test]
    async fn rankings_alone_combine_with_the_oprs_already_logged() {
        let repo = repo().await;
        log(
            &repo,
            "/event/2026mslr/oprs",
            r#"{"oprs":{"frc254":50.5},"dprs":{},"ccwms":{}}"#,
            8,
        )
        .await;
        let rankings = r#"{"rankings":[{"team_key":"frc254","rank":1,"matches_played":3,
            "dq":0,"record":{"wins":3,"losses":0,"ties":0}}],"sort_order_info":[]}"#;
        let appended = vec![log(&repo, "/event/2026mslr/rankings", rankings, 9).await];

        let projected = project(&repo, &appended).await;
        assert!(
            projected.report.problems.is_empty(),
            "{:?}",
            projected.report.problems
        );
        let stats = repo.team_stats("2026mslr", 254).await.unwrap().unwrap();
        assert_eq!(stats.rank, Some(1));
        assert_eq!(stats.opr, Some(50.5));
        assert_eq!(stats.synced_at, Some(at(8)), "as old as the older half");
    }

    #[tokio::test]
    async fn what_cannot_be_applied_is_said_and_the_rest_still_lands() {
        let repo = repo().await;
        let appended = vec![
            log(&repo, "/events/2026", "[]", 9).await,
            log(&repo, "/event/2026other/matches", "[]", 9).await,
            log(&repo, "/event/2026mslr/rankings", "{}", 9).await,
            log(&repo, "/event/2026mslr/matches", MATCHES, 9).await,
        ];
        let projected = project(&repo, &appended).await;
        assert_eq!(projected.logged_only, 1);
        assert_eq!(projected.report.matches, 1);
        let problems = projected.report.problems.join("\n");
        assert!(problems.contains("2026other is not an event"), "{problems}");
        assert!(
            problems.contains("both its rankings and its OPRs"),
            "{problems}"
        );
    }

    #[test]
    fn only_tbas_per_event_live_data_is_applied() {
        assert_eq!(
            feed("tba", "/event/2026mslr/coprs"),
            Some(("2026mslr", Feed::Stats))
        );
        assert_eq!(feed("tba", "/event/2026mslr/teams"), None);
        assert_eq!(feed("first", "/event/2026mslr/matches"), None);
        assert_eq!(feed("tba", "/events/2026"), None);
    }
}
