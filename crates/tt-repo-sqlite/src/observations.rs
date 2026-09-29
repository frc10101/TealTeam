//! Scouting observations against SQLite (U4).
//!
//! Only the write path and what the scouting form needs to read. The review
//! queue (L8-L10) reads this table too, and adds its own queries here.

use chrono::{DateTime, Utc};
use sqlx::Row;
use tt_core::assignments::Sighting;
use tt_core::review::{Decision, ReviewState};
use tt_core::season::{parse_payload, payload_to_json};
use tt_repo::{NewObservation, Recorded, RepoError, Result, StoredObservation};

use crate::SqliteRepo;
use crate::users::{from_sql, is_unique_violation, query_err, to_sql};

fn stored_from_row(row: &sqlx::sqlite::SqliteRow) -> StoredObservation {
    let ts = |column: &str| {
        row.get::<Option<String>, _>(column)
            .as_deref()
            .and_then(from_sql)
    };
    let id: i64 = row.get("id");
    StoredObservation {
        id,
        match_key: row.get("match_key"),
        event_key: row.get("event_key"),
        team_number: row.get("team_number"),
        alliance: row.get("alliance"),
        payload: parse_payload(&row.get::<String, _>("payload")).unwrap_or_else(|e| {
            tracing::warn!("observation {id} has an unreadable payload: {e}");
            Default::default()
        }),
        schema_version: row.get("schema_version"),
        scouter_id: row.get("scouter_id"),
        scouter_name: row.get("scouter_name"),
        submitting_team: row.get("submitting_team"),
        // The CHECK constraint allows nothing else; pending is the safe reading.
        review_state: ReviewState::parse(&row.get::<String, _>("review_state"))
            .unwrap_or(ReviewState::Pending),
        review_note: row.get("review_note"),
        reviewer_name: row.get("reviewer_name"),
        reviewed_at: ts("reviewed_at"),
        observed_at: ts("observed_at"),
        created_at: ts("created_at"),
    }
}

impl SqliteRepo {
    pub(crate) async fn record_observation_impl(
        &self,
        observation: &NewObservation,
        now: DateTime<Utc>,
    ) -> Result<Recorded> {
        let ts = to_sql(now);
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| query_err("starting observation write", e))?;

        // Match slots are plain team numbers, but observations reference
        // `teams`, and the TBA schedule routinely lists robots before the FIRST
        // roster sync has created them. A placeholder row satisfies the key; the
        // next roster sync overwrites its name, because upsert_team replaces it.
        sqlx::query(
            "INSERT INTO teams (team_number, name, created_at, updated_at) VALUES (?, ?, ?, ?) \
             ON CONFLICT (team_number) DO NOTHING",
        )
        .bind(observation.team_number)
        .bind(format!("Team {}", observation.team_number))
        .bind(&ts)
        .bind(&ts)
        .execute(&mut *tx)
        .await
        .map_err(|e| query_err("ensuring observed team exists", e))?;

        // DO NOTHING covers only client_record_id. Any other unique violation
        // -- the per-scout coverage index -- still fails, and is reported as a
        // conflict below.
        let inserted = sqlx::query(
            "INSERT INTO observations (client_record_id, match_key, team_number, event_key, \
                 alliance, payload, schema_version, scouter_id, device_id, submitting_team, \
                 observed_at, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT (client_record_id) DO NOTHING \
             RETURNING id",
        )
        .bind(&observation.client_record_id)
        .bind(&observation.match_key)
        .bind(observation.team_number)
        .bind(&observation.event_key)
        .bind(observation.alliance)
        .bind(payload_to_json(&observation.payload))
        .bind(observation.schema_version)
        .bind(observation.scouter_id)
        .bind(observation.device_id)
        .bind(observation.submitting_team)
        .bind(to_sql(observation.observed_at))
        .bind(&ts)
        .bind(&ts)
        .fetch_optional(&mut *tx)
        .await;

        let recorded = match inserted {
            Ok(Some(row)) => Recorded::Created(row.get("id")),
            Ok(None) => {
                let id =
                    sqlx::query_scalar("SELECT id FROM observations WHERE client_record_id = ?")
                        .bind(&observation.client_record_id)
                        .fetch_one(&mut *tx)
                        .await
                        .map_err(|e| query_err("loading duplicate observation", e))?;
                Recorded::Duplicate(id)
            }
            Err(e) if is_unique_violation(&e) => {
                return Err(RepoError::Conflict {
                    what: "An observation of this robot in this match by this scout",
                });
            }
            Err(e) => return Err(query_err("recording observation", e)),
        };

        tx.commit()
            .await
            .map_err(|e| query_err("committing observation", e))?;
        Ok(recorded)
    }

    pub(crate) async fn observed_teams_impl(
        &self,
        match_key: &str,
        scouter_id: i64,
    ) -> Result<Vec<i32>> {
        sqlx::query_scalar(
            "SELECT team_number FROM observations \
             WHERE match_key = ? AND scouter_id = ? AND review_state <> 'declined' \
             ORDER BY team_number",
        )
        .bind(match_key)
        .bind(scouter_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| query_err("listing observed teams", e))
    }

    pub(crate) async fn recorded_by_impl(
        &self,
        event_key: &str,
        scouter_id: i64,
    ) -> Result<Vec<(String, i32)>> {
        sqlx::query_as(
            "SELECT match_key, team_number FROM observations \
             WHERE event_key = ? AND scouter_id = ? AND review_state <> 'declined' \
             ORDER BY match_key, team_number",
        )
        .bind(event_key)
        .bind(scouter_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| query_err("listing recorded robots", e))
    }

    pub(crate) async fn pending_observations_impl(
        &self,
        event_key: &str,
    ) -> Result<Vec<StoredObservation>> {
        let rows = sqlx::query(
            "SELECT o.*, s.name AS scouter_name, r.name AS reviewer_name \
             FROM observations o \
             LEFT JOIN users s ON s.id = o.scouter_id \
             LEFT JOIN users r ON r.id = o.reviewed_by \
             WHERE o.event_key = ? AND o.review_state = 'pending' \
             ORDER BY o.created_at, o.id",
        )
        .bind(event_key)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| query_err("listing pending observations", e))?;
        Ok(rows.iter().map(stored_from_row).collect())
    }

    pub(crate) async fn observation_impl(&self, id: i64) -> Result<Option<StoredObservation>> {
        let row = sqlx::query(
            "SELECT o.*, s.name AS scouter_name, r.name AS reviewer_name \
             FROM observations o \
             LEFT JOIN users s ON s.id = o.scouter_id \
             LEFT JOIN users r ON r.id = o.reviewed_by \
             WHERE o.id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| query_err("loading observation", e))?;
        Ok(row.as_ref().map(stored_from_row))
    }

    pub(crate) async fn review_observation_impl(
        &self,
        id: i64,
        decision: &Decision,
        reviewer_id: i64,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        let note = match decision {
            Decision::Approve => None,
            Decision::Decline(reason) => Some(reason.as_str()),
        };
        let ts = to_sql(now);
        // One statement, guarded on the state: two leads pressing at once
        // cannot both review it, and the second learns so from the count.
        let done = sqlx::query(
            "UPDATE observations SET \
                review_state = ?, review_note = ?, reviewed_by = ?, reviewed_at = ?, \
                updated_at = ?, \
                submitting_team = COALESCE(submitting_team, \
                    (SELECT team_number FROM users WHERE id = observations.scouter_id)) \
             WHERE id = ? AND review_state = 'pending'",
        )
        .bind(decision.state().as_str())
        .bind(note)
        .bind(reviewer_id)
        .bind(&ts)
        .bind(&ts)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| query_err("reviewing observation", e))?;
        Ok(done.rows_affected() == 1)
    }

    pub(crate) async fn declined_for_impl(
        &self,
        event_key: &str,
        scouter_id: i64,
    ) -> Result<Vec<StoredObservation>> {
        let rows = sqlx::query(
            "SELECT o.*, s.name AS scouter_name, r.name AS reviewer_name \
             FROM observations o \
             LEFT JOIN users s ON s.id = o.scouter_id \
             LEFT JOIN users r ON r.id = o.reviewed_by \
             WHERE o.event_key = ? AND o.scouter_id = ? AND o.review_state = 'declined' \
               AND NOT EXISTS (SELECT 1 FROM observations again \
                   WHERE again.match_key = o.match_key AND again.team_number = o.team_number \
                     AND again.scouter_id = o.scouter_id AND again.review_state <> 'declined') \
             ORDER BY o.reviewed_at DESC, o.id DESC",
        )
        .bind(event_key)
        .bind(scouter_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| query_err("listing declined observations", e))?;
        Ok(rows.iter().map(stored_from_row).collect())
    }

    pub(crate) async fn event_sightings_impl(&self, event_key: &str) -> Result<Vec<Sighting>> {
        let rows = sqlx::query(
            "SELECT match_key, team_number, scouter_id, device_id FROM observations \
             WHERE event_key = ? AND review_state <> 'declined' \
             ORDER BY match_key, team_number, id",
        )
        .bind(event_key)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| query_err("listing sightings", e))?;
        Ok(rows
            .iter()
            .map(|row| Sighting {
                match_key: row.get("match_key"),
                team_number: row.get("team_number"),
                scouter_id: row.get("scouter_id"),
                device_id: row.get("device_id"),
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tt_core::matches::CompLevel;
    use tt_core::records::{Event, MatchRecord, Team};
    use tt_core::season::Value;
    use tt_core::user::Roles;
    use tt_repo::{NewUser, Repo};

    const MATCH: &str = "2026mabil_qm14";

    /// An event with one match: 10101, 254, 1 on red; 2, 3, 4 on blue. Only
    /// 10101 and 254 are on a synced roster.
    async fn repo() -> SqliteRepo {
        let repo = SqliteRepo::connect("sqlite::memory:").expect("connect");
        crate::migrate::apply(repo.pool()).await.expect("migrate");
        let now = Utc::now();

        repo.upsert_event(
            &Event {
                key: "2026mabil".into(),
                name: "Boston".into(),
                location: None,
                timezone: None,
                start_date: None,
                end_date: None,
                event_code: None,
                event_type: None,
                district_key: None,
                week: None,
            },
            now,
        )
        .await
        .expect("event");
        for number in [10101, 254] {
            repo.upsert_team(&team(number, &format!("Synced {number}")), now)
                .await
                .expect("team");
        }
        repo.upsert_match(
            &MatchRecord {
                key: MATCH.into(),
                event_key: "2026mabil".into(),
                comp_level: CompLevel::Qualification,
                set_number: 1,
                match_number: 14,
                red: [Some(10101), Some(254), Some(1)],
                blue: [Some(2), Some(3), Some(4)],
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
        for email in ["sam@example.com", "kim@example.com"] {
            repo.create_user(
                NewUser {
                    email: email.into(),
                    name: email.into(),
                    password_hash: "x".into(),
                    team_number: Some(10101),
                    roles: Roles::SCOUT,
                },
                now,
            )
            .await
            .expect("user");
        }
        repo
    }

    fn team(number: i32, name: &str) -> Team {
        Team {
            number,
            name: name.into(),
            nickname: None,
            school: None,
            city: None,
            state: None,
            country: None,
            rookie_year: None,
            website: None,
        }
    }

    /// Scout 1 observing `team_number`, under record id `n`.
    fn observation(n: u8, team_number: i32) -> NewObservation {
        NewObservation {
            client_record_id: format!("0191f7ac-1234-7000-8000-0000000000{n:02x}"),
            match_key: MATCH.into(),
            event_key: "2026mabil".into(),
            team_number,
            alliance: "red",
            payload: [("teleop_scored".to_string(), Value::Count(9))].into(),
            schema_version: 1,
            scouter_id: Some(1),
            device_id: None,
            submitting_team: Some(10101),
            observed_at: Utc::now(),
        }
    }

    async fn count(repo: &SqliteRepo) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM observations")
            .fetch_one(repo.pool())
            .await
            .expect("count")
    }

    #[tokio::test]
    async fn an_observation_is_stored_pending_review_with_its_payload() {
        let repo = repo().await;
        let recorded = repo
            .record_observation(&observation(1, 254), Utc::now())
            .await
            .expect("record");
        let Recorded::Created(id) = recorded else {
            panic!("expected a new row, got {recorded:?}");
        };

        let row = sqlx::query(
            "SELECT review_state, payload, schema_version, alliance, submitting_team \
             FROM observations WHERE id = ?",
        )
        .bind(id)
        .fetch_one(repo.pool())
        .await
        .expect("row");
        assert_eq!(row.get::<String, _>("review_state"), "pending");
        assert_eq!(row.get::<String, _>("payload"), r#"{"teleop_scored":9}"#);
        assert_eq!(row.get::<i64, _>("schema_version"), 1);
        assert_eq!(row.get::<String, _>("alliance"), "red");
        assert_eq!(row.get::<Option<i32>, _>("submitting_team"), Some(10101));
    }

    #[tokio::test]
    async fn the_same_record_id_twice_is_one_observation() {
        // A double-tapped Save, or a post retried after the network dropped.
        let repo = repo().await;
        let first = repo
            .record_observation(&observation(1, 254), Utc::now())
            .await
            .expect("first");
        let again = repo
            .record_observation(&observation(1, 254), Utc::now())
            .await
            .expect("replay");

        let (Recorded::Created(id), Recorded::Duplicate(same)) = (first, again) else {
            panic!("expected created then duplicate, got {first:?} then {again:?}");
        };
        assert_eq!(id, same);
        assert_eq!(count(&repo).await, 1);
    }

    #[tokio::test]
    async fn a_second_observation_of_one_robot_by_one_scout_conflicts() {
        let repo = repo().await;
        repo.record_observation(&observation(1, 254), Utc::now())
            .await
            .expect("first");

        let second = repo
            .record_observation(&observation(2, 254), Utc::now())
            .await;
        assert!(
            matches!(second, Err(RepoError::Conflict { .. })),
            "{second:?}"
        );
        assert_eq!(count(&repo).await, 1);
    }

    #[tokio::test]
    async fn two_scouts_may_both_observe_one_robot() {
        let repo = repo().await;
        repo.record_observation(&observation(1, 254), Utc::now())
            .await
            .expect("first scout");
        let other_scout = NewObservation {
            scouter_id: Some(2),
            ..observation(2, 254)
        };
        repo.record_observation(&other_scout, Utc::now())
            .await
            .expect("second scout");
        assert_eq!(count(&repo).await, 2);
    }

    #[tokio::test]
    async fn a_declined_observation_can_be_replaced() {
        let repo = repo().await;
        repo.record_observation(&observation(1, 254), Utc::now())
            .await
            .expect("first");
        sqlx::query("UPDATE observations SET review_state = 'declined'")
            .execute(repo.pool())
            .await
            .expect("decline");

        assert!(repo.observed_teams(MATCH, 1).await.unwrap().is_empty());
        repo.record_observation(&observation(2, 254), Utc::now())
            .await
            .expect("a correction after a decline");
        assert_eq!(repo.observed_teams(MATCH, 1).await.unwrap(), [254]);
    }

    #[tokio::test]
    async fn a_robot_on_no_synced_roster_can_still_be_observed() {
        let repo = repo().await;
        repo.record_observation(&observation(1, 3), Utc::now())
            .await
            .expect("team 3 has no teams row yet");
        assert_eq!(repo.team(3).await.unwrap().unwrap().name, "Team 3");

        // The roster sync, arriving later, names it properly.
        repo.upsert_team(&team(3, "Synced 3"), Utc::now())
            .await
            .expect("sync");
        assert_eq!(repo.team(3).await.unwrap().unwrap().name, "Synced 3");
    }

    #[tokio::test]
    async fn recording_leaves_a_synced_team_alone() {
        let repo = repo().await;
        repo.record_observation(&observation(1, 254), Utc::now())
            .await
            .expect("record");
        assert_eq!(repo.team(254).await.unwrap().unwrap().name, "Synced 254");
    }

    #[tokio::test]
    async fn observed_teams_are_one_scouts_robots_in_one_match() {
        let repo = repo().await;
        for (n, team) in [(1, 254), (2, 10101)] {
            repo.record_observation(&observation(n, team), Utc::now())
                .await
                .expect("record");
        }
        let other_scout = NewObservation {
            scouter_id: Some(2),
            ..observation(3, 1)
        };
        repo.record_observation(&other_scout, Utc::now())
            .await
            .expect("record");

        assert_eq!(repo.observed_teams(MATCH, 1).await.unwrap(), [254, 10101]);
        assert_eq!(repo.observed_teams(MATCH, 2).await.unwrap(), [1]);
        assert!(
            repo.observed_teams("2026mabil_qm15", 1)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn recorded_by_lists_one_scouts_robots_across_the_event() {
        let repo = repo().await;
        for (n, team) in [(1, 254), (2, 10101)] {
            repo.record_observation(&observation(n, team), Utc::now())
                .await
                .expect("record");
        }
        let other_scout = NewObservation {
            scouter_id: Some(2),
            ..observation(3, 1)
        };
        repo.record_observation(&other_scout, Utc::now())
            .await
            .expect("record");

        assert_eq!(
            repo.recorded_by("2026mabil", 1).await.unwrap(),
            [(MATCH.to_string(), 254), (MATCH.to_string(), 10101)]
        );
        assert!(repo.recorded_by("2026nope", 1).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn sightings_are_every_undeclined_observation_at_the_event() {
        let repo = repo().await;
        repo.record_observation(&observation(1, 254), Utc::now())
            .await
            .expect("record");
        let declined = NewObservation {
            scouter_id: Some(2),
            ..observation(2, 1)
        };
        repo.record_observation(&declined, Utc::now())
            .await
            .expect("record");
        sqlx::query("UPDATE observations SET review_state = 'declined' WHERE team_number = 1")
            .execute(repo.pool())
            .await
            .expect("decline");

        let sightings = repo.event_sightings("2026mabil").await.unwrap();
        assert_eq!(sightings.len(), 1);
        assert_eq!(
            (sightings[0].team_number, sightings[0].scouter_id),
            (254, Some(1))
        );
        assert!(repo.event_sightings("2026nope").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn the_queue_is_the_events_pending_observations_oldest_first() {
        let repo = repo().await;
        repo.record_observation(&observation(1, 254), Utc::now())
            .await
            .unwrap();
        let later = NewObservation {
            scouter_id: Some(2),
            ..observation(2, 1)
        };
        repo.record_observation(&later, Utc::now() + chrono::TimeDelta::seconds(5))
            .await
            .unwrap();

        let queue = repo.pending_observations("2026mabil").await.unwrap();
        let teams: Vec<i32> = queue.iter().map(|o| o.team_number).collect();
        assert_eq!(teams, [254, 1]);
        assert_eq!(queue[0].scouter_name.as_deref(), Some("sam@example.com"));
        assert_eq!(queue[0].payload.len(), 1, "the payload comes back parsed");
        assert_eq!(queue[0].review_state, ReviewState::Pending);
    }

    #[tokio::test]
    async fn approving_is_an_update_that_fills_in_the_scouts_team() {
        let repo = repo().await;
        let no_team = NewObservation {
            submitting_team: None,
            ..observation(1, 254)
        };
        let Recorded::Created(id) = repo.record_observation(&no_team, Utc::now()).await.unwrap()
        else {
            panic!("created")
        };

        assert!(
            repo.review_observation(id, &Decision::Approve, 2, Utc::now())
                .await
                .unwrap()
        );
        let stored = repo.observation(id).await.unwrap().unwrap();
        assert_eq!(stored.review_state, ReviewState::Approved);
        assert_eq!(stored.submitting_team, Some(10101), "the scout's team");
        assert_eq!(stored.reviewer_name.as_deref(), Some("kim@example.com"));
        assert!(
            repo.pending_observations("2026mabil")
                .await
                .unwrap()
                .is_empty()
        );

        // Reviewed once is reviewed: a second press changes nothing.
        let decline = Decision::decline("late").unwrap();
        assert!(
            !repo
                .review_observation(id, &decline, 2, Utc::now())
                .await
                .unwrap()
        );
        assert_eq!(
            repo.observation(id).await.unwrap().unwrap().review_state,
            ReviewState::Approved
        );
    }

    #[tokio::test]
    async fn a_decline_keeps_the_row_and_the_reason_until_it_is_recorded_again() {
        let repo = repo().await;
        let Recorded::Created(id) = repo
            .record_observation(&observation(1, 254), Utc::now())
            .await
            .unwrap()
        else {
            panic!("created")
        };
        let decline = Decision::decline("that was 1678").unwrap();
        assert!(
            repo.review_observation(id, &decline, 2, Utc::now())
                .await
                .unwrap()
        );

        assert_eq!(count(&repo).await, 1, "nothing is destroyed");
        let declined = repo.declined_for("2026mabil", 1).await.unwrap();
        assert_eq!(declined.len(), 1);
        assert_eq!(declined[0].review_note.as_deref(), Some("that was 1678"));
        assert!(repo.declined_for("2026mabil", 2).await.unwrap().is_empty());

        // Recording the robot again is allowed, and answers the decline.
        repo.record_observation(&observation(2, 254), Utc::now())
            .await
            .expect("the coverage index ignores declined rows");
        assert!(repo.declined_for("2026mabil", 1).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_match_is_found_by_its_key() {
        let repo = repo().await;
        let found = repo.match_by_key(MATCH).await.unwrap().expect("found");
        assert_eq!(found.red, [Some(10101), Some(254), Some(1)]);
        assert!(repo.match_by_key("2026mabil_qm99").await.unwrap().is_none());
    }
}
