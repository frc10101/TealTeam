//! Scout assignments against SQLite (L1).
//!
//! Reads only, for the lead scout's grid. Setting, distributing, and clearing
//! (L2) add their writes here.

use sqlx::Row;
use tt_core::assignments::{Assignee, Assignment};
use tt_repo::{Device, Result};

use crate::SqliteRepo;
use crate::users::{from_sql, query_err};

fn assignment_from_row(row: &sqlx::sqlite::SqliteRow) -> Option<Assignment> {
    let scout = row
        .get::<Option<i64>, _>("scouter_id")
        .map(|id| Assignee::Scout {
            id,
            name: row.get("scouter_name"),
        });
    // The display name falls back to a UUID prefix exactly as it does in the
    // device list, so a tablet reads the same on the grid as everywhere else.
    let device = || {
        let id = row.get::<Option<i64>, _>("device_id")?;
        let device = Device {
            id,
            device_uuid: row.get("device_uuid"),
            name: row.get("device_name"),
            team_number: None,
            last_seen_at: row
                .get::<Option<String>, _>("last_seen_at")
                .as_deref()
                .and_then(from_sql),
        };
        Some(Assignee::Device {
            id,
            name: device.display_name(),
        })
    };

    Some(Assignment {
        match_key: row.get("match_key"),
        team_number: row.get("team_number"),
        // The schema's CHECK guarantees one of the two; a row with neither is
        // skipped rather than invented an assignee.
        assignee: scout.or_else(device)?,
    })
}

impl SqliteRepo {
    pub(crate) async fn event_assignments_impl(&self, event_key: &str) -> Result<Vec<Assignment>> {
        let rows = sqlx::query(
            "SELECT a.match_key, a.team_number, \
                    a.scouter_id, u.name AS scouter_name, \
                    a.device_id, d.device_uuid, d.name AS device_name, d.last_seen_at \
             FROM scout_assignments a \
             LEFT JOIN users u ON u.id = a.scouter_id \
             LEFT JOIN devices d ON d.id = a.device_id \
             WHERE a.event_key = ? \
             ORDER BY a.match_key, a.team_number",
        )
        .bind(event_key)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| query_err("listing assignments", e))?;
        Ok(rows.iter().filter_map(assignment_from_row).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use tt_core::matches::CompLevel;
    use tt_core::records::{Event, MatchRecord, Team};
    use tt_core::user::Roles;
    use tt_repo::{NewUser, Repo};

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

    /// Q1 at two events, with 254 on red at both; one scout and two tablets,
    /// one of them named.
    async fn repo() -> SqliteRepo {
        let repo = SqliteRepo::connect("sqlite::memory:").expect("connect");
        crate::migrate::apply(repo.pool()).await.expect("migrate");
        let now = Utc::now();

        for key in ["2026mabil", "2026nhgrs"] {
            repo.upsert_event(&event(key), now).await.expect("event");
        }
        for number in [254, 1678] {
            repo.upsert_team(
                &Team {
                    number,
                    name: format!("Team {number}"),
                    nickname: None,
                    school: None,
                    city: None,
                    state: None,
                    country: None,
                    rookie_year: None,
                    website: None,
                },
                now,
            )
            .await
            .expect("team");
        }
        for event_key in ["2026mabil", "2026nhgrs"] {
            repo.upsert_match(
                &MatchRecord {
                    key: format!("{event_key}_qm1"),
                    event_key: event_key.into(),
                    comp_level: CompLevel::Qualification,
                    set_number: 1,
                    match_number: 1,
                    red: [Some(254), Some(1678), None],
                    blue: [None, None, None],
                    red_score: None,
                    blue_score: None,
                    winner: None,
                    played: false,
                    scheduled_at: None,
                    actual_at: None,
                },
                now,
            )
            .await
            .expect("match");
        }
        repo.create_user(
            NewUser {
                email: "sam@example.com".into(),
                name: "Sam".into(),
                password_hash: "x".into(),
                team_number: Some(10101),
                roles: Roles::SCOUT,
            },
            now,
        )
        .await
        .expect("user");
        let named = repo
            .touch_device("0191f7ac-aaaa-7000-8000-000000000001", None, now)
            .await
            .expect("device");
        repo.rename_device(named.id, "Stands Left", now)
            .await
            .expect("rename");
        repo.touch_device("0191f7ad-bbbb-7000-8000-000000000002", None, now)
            .await
            .expect("device");
        repo
    }

    async fn assign(
        repo: &SqliteRepo,
        event_key: &str,
        team: i32,
        scouter: Option<i64>,
        device: Option<i64>,
    ) {
        let now = crate::users::to_sql(Utc::now());
        sqlx::query(
            "INSERT INTO scout_assignments \
                 (match_key, team_number, event_key, scouter_id, device_id, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(format!("{event_key}_qm1"))
        .bind(team)
        .bind(event_key)
        .bind(scouter)
        .bind(device)
        .bind(&now)
        .bind(&now)
        .execute(repo.pool())
        .await
        .expect("assign");
    }

    #[tokio::test]
    async fn assignees_come_back_named_whether_scouts_or_tablets() {
        let repo = repo().await;
        assign(&repo, "2026mabil", 254, Some(1), None).await;
        assign(&repo, "2026mabil", 1678, None, Some(1)).await;

        let found = repo.event_assignments("2026mabil").await.expect("list");
        assert_eq!(
            found,
            [
                Assignment {
                    match_key: "2026mabil_qm1".into(),
                    team_number: 254,
                    assignee: Assignee::Scout {
                        id: 1,
                        name: "Sam".into()
                    },
                },
                Assignment {
                    match_key: "2026mabil_qm1".into(),
                    team_number: 1678,
                    assignee: Assignee::Device {
                        id: 1,
                        name: "Stands Left".into()
                    },
                },
            ]
        );
    }

    #[tokio::test]
    async fn an_unnamed_tablet_reads_as_it_does_in_the_device_list() {
        let repo = repo().await;
        assign(&repo, "2026mabil", 254, None, Some(2)).await;

        let found = repo.event_assignments("2026mabil").await.expect("list");
        assert_eq!(found[0].assignee.name(), "Device 0191f7ad");
    }

    #[tokio::test]
    async fn a_row_naming_both_reports_the_scout() {
        let repo = repo().await;
        assign(&repo, "2026mabil", 254, Some(1), Some(1)).await;

        let found = repo.event_assignments("2026mabil").await.expect("list");
        assert_eq!(found[0].assignee.name(), "Sam");
    }

    #[tokio::test]
    async fn only_the_asked_for_events_assignments_are_listed() {
        let repo = repo().await;
        assign(&repo, "2026mabil", 254, Some(1), None).await;
        assign(&repo, "2026nhgrs", 1678, Some(1), None).await;

        let found = repo.event_assignments("2026nhgrs").await.expect("list");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].match_key, "2026nhgrs_qm1");
        assert!(
            repo.event_assignments("2026none")
                .await
                .expect("list")
                .is_empty()
        );
    }
}
