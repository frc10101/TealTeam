//! Rankings typed in by hand (I14), written over the synced ones.
//!
//! A typed ranking is a snapshot of the audience display, so it replaces the
//! event's ranking outright rather than merging into it:
//!
//! - Each listed team gets its rank, and the ranking score and record if they
//!   were typed. Its other ranking columns go to NULL, because `synced_at`
//!   moves to now and would otherwise vouch for numbers from an older sync.
//! - The OPR columns are left alone. They are not on the display, and the
//!   freshness they carry is the same trade TBA's partial syncs already make.
//! - Every team at the event that was not listed loses its rank, so a typed
//!   top twenty can never leave a stale "5th" beside a new one.
//!
//! The next TBA sync overwrites all of it, which is right: TBA is the source
//! whenever it can be reached.

use chrono::{DateTime, Utc};
use tt_core::standings::Standing;
use tt_repo::Result;

use crate::SqliteRepo;
use crate::users::{query_err, to_sql};

impl SqliteRepo {
    pub(crate) async fn record_standings_impl(
        &self,
        event_key: &str,
        standings: &[Standing],
        now: DateTime<Utc>,
    ) -> Result<()> {
        let ts = to_sql(now);
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| query_err("starting standings write", e))?;

        sqlx::query("UPDATE team_event_stats SET rank = NULL WHERE event_key = ?")
            .bind(event_key)
            .execute(&mut *tx)
            .await
            .map_err(|e| query_err("clearing ranks", e))?;

        for s in standings {
            // With no roster synced, typed teams may not exist yet. The next
            // roster sync replaces the placeholder name.
            sqlx::query(
                "INSERT INTO teams (team_number, name, created_at, updated_at) \
                 VALUES (?, ?, ?, ?) ON CONFLICT (team_number) DO NOTHING",
            )
            .bind(s.team_number)
            .bind(format!("Team {}", s.team_number))
            .bind(&ts)
            .bind(&ts)
            .execute(&mut *tx)
            .await
            .map_err(|e| query_err("ensuring ranked team exists", e))?;

            let record = s.record;
            sqlx::query(
                "INSERT INTO team_event_stats (team_number, event_key, rank, qual_average, \
                     wins, losses, ties, matches_played, synced_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) \
                 ON CONFLICT (team_number, event_key) DO UPDATE SET \
                    rank = excluded.rank, qual_average = excluded.qual_average, \
                    wins = excluded.wins, losses = excluded.losses, ties = excluded.ties, \
                    matches_played = excluded.matches_played, \
                    avg_match_points = NULL, dq_count = NULL, qual_points = NULL, \
                    total_points = NULL, synced_at = excluded.synced_at",
            )
            .bind(s.team_number)
            .bind(event_key)
            .bind(s.rank)
            .bind(s.ranking_score)
            .bind(record.map(|r| r.wins))
            .bind(record.map(|r| r.losses))
            .bind(record.map(|r| r.ties))
            .bind(record.map(|r| r.played()))
            .bind(&ts)
            .execute(&mut *tx)
            .await
            .map_err(|e| query_err("storing a typed rank", e))?;
        }

        tx.commit()
            .await
            .map_err(|e| query_err("committing standings", e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tt_core::records::{Event, Team, TeamEventStats};
    use tt_core::standings::Record;
    use tt_repo::Repo;

    fn event(key: &str) -> Event {
        Event {
            key: key.into(),
            name: key.into(),
            location: None,
            timezone: None,
            start_date: None,
            end_date: None,
            event_code: None,
            event_type: None,
            district_key: None,
            week: None,
        }
    }

    /// Two events, and teams 254 and 118.
    async fn repo() -> SqliteRepo {
        let repo = SqliteRepo::connect("sqlite::memory:").expect("connect");
        crate::migrate::apply(repo.pool()).await.expect("migrate");
        let now = Utc::now();
        for key in ["2026mabil", "2026nhgrs"] {
            repo.upsert_event(&event(key), now).await.expect("event");
        }
        for number in [254, 118] {
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
            repo.upsert_team(&team, now).await.expect("team");
        }
        repo
    }

    fn standing(rank: i32, team_number: i32) -> Standing {
        Standing {
            rank,
            team_number,
            ranking_score: None,
            record: None,
        }
    }

    #[tokio::test]
    async fn typed_ranks_replace_the_ranking_and_keep_the_oprs() {
        let repo = repo().await;
        let synced = chrono::DateTime::parse_from_rfc3339("2026-03-14T15:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        // What TBA said an hour ago: 254 first, 118 fifth.
        for (team, rank) in [(254, 1), (118, 5)] {
            let stats = TeamEventStats {
                team_number: team,
                event_key: "2026mabil".into(),
                opr: Some(50.0),
                rank: Some(rank),
                dq_count: Some(0),
                total_points: Some(20),
                synced_at: Some(synced),
                ..TeamEventStats::default()
            };
            repo.upsert_team_stats(&stats, synced).await.expect("stats");
        }

        let now = Utc::now();
        // 1678 is on no roster yet, which is the no-uplink case.
        let typed = [
            Standing {
                ranking_score: Some(3.42),
                record: Some(Record {
                    wins: 11,
                    losses: 1,
                    ties: 0,
                }),
                ..standing(1, 1678)
            },
            standing(2, 254),
        ];
        repo.record_standings_impl("2026mabil", &typed, now)
            .await
            .expect("record");

        let stats = repo.event_stats("2026mabil").await.expect("stats");
        let of = |team: i32| stats.iter().find(|s| s.team_number == team).unwrap();

        let first = of(1678);
        assert_eq!(first.rank, Some(1));
        assert_eq!(first.qual_average, Some(3.42));
        assert_eq!(
            (first.wins, first.losses, first.ties, first.matches_played),
            (Some(11), Some(1), Some(0), Some(12))
        );

        let second = of(254);
        assert_eq!(second.rank, Some(2));
        assert_eq!(second.opr, Some(50.0), "OPRs are not on the display");
        assert_eq!(
            (second.dq_count, second.total_points),
            (None, None),
            "not vouched for by the new timestamp"
        );
        assert!(second.synced_at.unwrap() > synced);

        let dropped = of(118);
        assert_eq!(dropped.rank, None, "no stale fifth beside a new ranking");
        assert_eq!(dropped.total_points, Some(20));
        assert_eq!(dropped.synced_at, Some(synced));
    }

    #[tokio::test]
    async fn another_events_ranks_are_untouched() {
        let repo = repo().await;
        repo.record_standings_impl("2026nhgrs", &[standing(1, 254)], Utc::now())
            .await
            .expect("other");
        repo.record_standings_impl("2026mabil", &[standing(1, 118)], Utc::now())
            .await
            .expect("this");

        let stats = repo.event_stats("2026nhgrs").await.expect("stats");
        assert_eq!(stats[0].rank, Some(1));
    }
}
