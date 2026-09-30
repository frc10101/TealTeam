//! Scout assignments against SQLite (L1, L2).
//!
//! The grid's read, and the lead scout's writes: set, distribute, clear.

use chrono::{DateTime, Utc};
use sqlx::Row;
use tt_core::assignments::{Assignee, AssigneeKey, Assignment};
use tt_repo::{Device, NewAssignment, Result};

use crate::SqliteRepo;
use crate::users::{from_sql, query_err, to_sql};

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
            last_user_id: None,
            clock_offset_ms: None,
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

    pub(crate) async fn set_assignments_impl(
        &self,
        assignments: &[NewAssignment],
        assigned_by: i64,
        now: DateTime<Utc>,
    ) -> Result<()> {
        let ts = to_sql(now);
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| query_err("starting assignment write", e))?;

        for a in assignments {
            // As for observations: the schedule names robots before the roster
            // sync creates them, and the row references `teams`. The next
            // roster sync replaces the placeholder name.
            sqlx::query(
                "INSERT INTO teams (team_number, name, created_at, updated_at) \
                 VALUES (?, ?, ?, ?) ON CONFLICT (team_number) DO NOTHING",
            )
            .bind(a.team_number)
            .bind(format!("Team {}", a.team_number))
            .bind(&ts)
            .bind(&ts)
            .execute(&mut *tx)
            .await
            .map_err(|e| query_err("ensuring assigned team exists", e))?;

            // Exactly one of the two, always: replacing a scout with a tablet
            // must clear the scout, or the row would name both.
            let (scouter, device) = match a.assignee {
                AssigneeKey::Scout(id) => (Some(id), None),
                AssigneeKey::Device(id) => (None, Some(id)),
            };
            sqlx::query(
                "INSERT INTO scout_assignments (match_key, team_number, event_key, scouter_id, \
                     device_id, assigned_by, created_at, updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?) \
                 ON CONFLICT (match_key, team_number) DO UPDATE SET \
                    scouter_id  = excluded.scouter_id, \
                    device_id   = excluded.device_id, \
                    assigned_by = excluded.assigned_by, \
                    updated_at  = excluded.updated_at",
            )
            .bind(&a.match_key)
            .bind(a.team_number)
            .bind(&a.event_key)
            .bind(scouter)
            .bind(device)
            .bind(assigned_by)
            .bind(&ts)
            .bind(&ts)
            .execute(&mut *tx)
            .await
            .map_err(|e| query_err("assigning a robot", e))?;
        }

        tx.commit()
            .await
            .map_err(|e| query_err("committing assignments", e))
    }

    pub(crate) async fn unassign_impl(&self, match_key: &str, team_number: i32) -> Result<()> {
        sqlx::query("DELETE FROM scout_assignments WHERE match_key = ? AND team_number = ?")
            .bind(match_key)
            .bind(team_number)
            .execute(&self.pool)
            .await
            .map_err(|e| query_err("removing an assignment", e))?;
        Ok(())
    }

    pub(crate) async fn clear_assignments_impl(
        &self,
        event_key: &str,
        match_key: Option<&str>,
    ) -> Result<u64> {
        let done = sqlx::query(
            "DELETE FROM scout_assignments WHERE event_key = ? AND (?2 IS NULL OR match_key = ?2)",
        )
        .bind(event_key)
        .bind(match_key)
        .execute(&self.pool)
        .await
        .map_err(|e| query_err("clearing assignments", e))?;
        Ok(done.rows_affected())
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

    fn new_assignment(event_key: &str, team_number: i32, assignee: AssigneeKey) -> NewAssignment {
        NewAssignment {
            match_key: format!("{event_key}_qm1"),
            event_key: event_key.into(),
            team_number,
            assignee,
        }
    }

    #[tokio::test]
    async fn setting_replaces_whoever_had_the_robot() {
        let repo = repo().await;
        let now = Utc::now();
        repo.set_assignments(
            &[new_assignment("2026mabil", 254, AssigneeKey::Scout(1))],
            1,
            now,
        )
        .await
        .expect("set");
        repo.set_assignments(
            &[new_assignment("2026mabil", 254, AssigneeKey::Device(1))],
            1,
            now,
        )
        .await
        .expect("replace");

        let found = repo.event_assignments("2026mabil").await.expect("list");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].assignee.key(), AssigneeKey::Device(1));
        let scouter: Option<i64> = sqlx::query_scalar("SELECT scouter_id FROM scout_assignments")
            .fetch_one(repo.pool())
            .await
            .expect("row");
        assert_eq!(
            scouter, None,
            "the scout is cleared, not left beside the tablet"
        );
    }

    #[tokio::test]
    async fn a_robot_no_roster_has_synced_can_be_assigned() {
        let repo = repo().await;
        repo.set_assignments(
            &[new_assignment("2026mabil", 9999, AssigneeKey::Scout(1))],
            1,
            Utc::now(),
        )
        .await
        .expect("a placeholder team satisfies the key");
        assert_eq!(
            repo.team(9999).await.expect("team").unwrap().name,
            "Team 9999"
        );
    }

    #[tokio::test]
    async fn a_batch_is_all_or_nothing() {
        let repo = repo().await;
        let result = repo
            .set_assignments(
                &[
                    new_assignment("2026mabil", 254, AssigneeKey::Scout(1)),
                    // No user 42: the foreign key fails, and 254 must not stick.
                    new_assignment("2026mabil", 1678, AssigneeKey::Scout(42)),
                ],
                1,
                Utc::now(),
            )
            .await;
        assert!(result.is_err());
        assert!(
            repo.event_assignments("2026mabil")
                .await
                .expect("list")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn clearing_takes_a_robot_a_match_or_an_event() {
        let repo = repo().await;
        let now = Utc::now();
        for event_key in ["2026mabil", "2026nhgrs"] {
            repo.set_assignments(
                &[
                    new_assignment(event_key, 254, AssigneeKey::Scout(1)),
                    new_assignment(event_key, 1678, AssigneeKey::Device(2)),
                ],
                1,
                now,
            )
            .await
            .expect("set");
        }

        repo.unassign("2026mabil_qm1", 254).await.expect("unassign");
        repo.unassign("2026mabil_qm1", 254)
            .await
            .expect("twice is fine");
        assert_eq!(repo.event_assignments("2026mabil").await.unwrap().len(), 1);

        let gone = repo
            .clear_assignments("2026mabil", Some("2026mabil_qm1"))
            .await
            .expect("clear match");
        assert_eq!(gone, 1);
        assert_eq!(
            repo.clear_assignments("2026nhgrs", None)
                .await
                .expect("clear event"),
            2
        );
        assert_eq!(repo.clear_assignments("2026nhgrs", None).await.unwrap(), 0);
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
