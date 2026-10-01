//! `POST /api/sync/push` (C7): a device's outbox, delivered.
//!
//! The body is a [`Push`] as JSON: observations a scout recorded on the
//! device, likely with no signal. Each one is checked by the same rules as
//! the scouting form's post, and recorded as the signed-in scout, from the
//! tablet whose cookie came with it. The answer is a [`Receipt`] per entry:
//! recorded (now, or already), or refused with a reason a person can read.
//! The device clears an entry only once it comes back through the change
//! log (`tt_core::outbox` says why).
//!
//! Refusals of the whole push are status codes with a JSON reason, never the
//! guards' redirect, so the device knows to keep its queue:
//!   - 401 not signed in,
//!   - 409 another schema (S11, `?schema=`, as for the pull),
//!   - 413 more than [`MAX_PER_PUSH`] entries,
//!   - 422 not a push, with why,
//!   - 503 storage down; nothing after the first failure was tried.

use std::collections::HashMap;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, TimeDelta, Utc};
use serde_json::json;
use tracing::{info, warn};
use tt_core::outbox::{MAX_PER_PUSH, Outcome, Push, PushReply, QueuedObservation, Receipt};
use tt_core::record_id;
use tt_core::user::User;
use tt_repo::{Device, NewObservation, Repo, RepoError};

use crate::auth::{MaybeAuth, device_uuid};
use crate::startup::AppState;

fn refuse(status: StatusCode, error: impl Into<String>) -> Response {
    (status, Json(json!({ "error": error.into() }))).into_response()
}

pub async fn push(
    State(state): State<AppState>,
    MaybeAuth(user): MaybeAuth,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    body: Bytes,
) -> Response {
    let Some(user) = user else {
        return refuse(
            StatusCode::UNAUTHORIZED,
            "sign in to send what this device saved",
        );
    };
    if let Some(refused) = crate::sync::schema_mismatch(&state, &query) {
        return refused;
    }
    let push: Push = match serde_json::from_slice(&body) {
        Ok(push) => push,
        Err(e) => {
            return refuse(
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("not an outbox: {e}"),
            );
        }
    };
    if push.observations.len() > MAX_PER_PUSH {
        return refuse(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!("at most {MAX_PER_PUSH} observations at a time"),
        );
    }

    let device = match device_uuid(&headers) {
        Some(uuid) => state.repo.device_by_uuid(&uuid).await.unwrap_or_else(|e| {
            warn!("looking up device: {e}");
            None
        }),
        None => None,
    };
    let now = Utc::now();
    let mut receipts = Vec::with_capacity(push.observations.len());
    for queued in push.observations {
        let client_record_id = queued.client_record_id.clone();
        match record(&state, &user, device.as_ref(), queued, now).await {
            Ok(outcome) => receipts.push(Receipt {
                client_record_id,
                outcome,
            }),
            Err(e) => {
                warn!("recording a pushed observation: {e}");
                return refuse(StatusCode::SERVICE_UNAVAILABLE, "storage unavailable");
            }
        }
    }

    let refused = receipts
        .iter()
        .filter(|r| matches!(r.outcome, Outcome::Refused { .. }))
        .count();
    info!(
        user = %user.email,
        device = device.as_ref().map_or("-", |d| d.device_uuid.as_str()),
        sent = receipts.len(),
        refused,
        "outbox pushed"
    );
    Json(PushReply {
        schema: crate::shell::page_version(&state).schema,
        receipts,
    })
    .into_response()
}

/// One entry, as `scouting::submit` would take it from the form. `Err` only
/// for storage; everything about the entry itself is an [`Outcome`].
async fn record(
    state: &AppState,
    user: &User,
    device: Option<&Device>,
    queued: QueuedObservation,
    now: DateTime<Utc>,
) -> Result<Outcome, RepoError> {
    let refused = |reason: String| Ok(Outcome::Refused { reason });
    let Some(client_record_id) = record_id::normalize(&queued.client_record_id) else {
        return refused("Not saved: it has no record id.".into());
    };
    // The schedule decides the event and alliance, never the device.
    let Some(record) = state.repo.match_by_key(&queued.match_key).await? else {
        return refused(format!(
            "Not saved: {} is not on the schedule.",
            queued.match_key
        ));
    };
    let Some(alliance) = record.alliance_of(queued.team_number) else {
        return refused(format!(
            "Not saved: team {} is not in {}.",
            queued.team_number, record.key
        ));
    };
    // Answers on another form are kept as they are, and the review page
    // flags them (S11); on this form they must fit it.
    if queued.schema_version == state.season.version
        && let Err(e) = state.season.validate_payload(&queued.payload)
    {
        return refused(format!("Not saved: the answers do not fit the form ({e})."));
    }

    let observation = NewObservation {
        client_record_id,
        match_key: record.key.clone(),
        event_key: record.event_key.clone(),
        team_number: queued.team_number,
        alliance,
        payload: queued.payload,
        schema_version: queued.schema_version,
        scouter_id: Some(user.id),
        device_id: device.map(|d| d.id),
        submitting_team: user.team_number,
        observed_at: observed_at(queued.observed_at, device, now),
    };
    match state.repo.record_observation(&observation, now).await {
        Ok(_) => Ok(Outcome::Recorded),
        Err(RepoError::Conflict { .. }) => refused(format!(
            "Not saved: you already have an observation of team {} in {}.",
            observation.team_number, record.key
        )),
        Err(e) => Err(e),
    }
}

/// The device's time on the server's clock: corrected by the tablet's
/// measured offset (S12), and never later than now.
fn observed_at(
    device_time: DateTime<Utc>,
    device: Option<&Device>,
    now: DateTime<Utc>,
) -> DateTime<Utc> {
    let offset = device
        .and_then(|d| d.clock_offset_ms)
        .map_or(TimeDelta::zero(), TimeDelta::milliseconds);
    (device_time + offset).min(now)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use axum::body::Body;
    use axum::http::{Request, header};
    use chrono::TimeZone;
    use tower::ServiceExt;
    use tt_client::ClientRepo;
    use tt_client::sync::{Reply, SyncClient, Transport};
    use tt_core::matches::CompLevel;
    use tt_core::records::{Event, MatchRecord, Team};
    use tt_core::season::{Payload, Value};
    use tt_repo_sqlite::SqliteRepo;

    use crate::startup::router;

    const MINE: &str = "0191f7ac-1234-7000-8000-000000000001";
    const WRONG: &str = "0191f7ac-1234-7000-8000-000000000002";
    const DEVICE: &str = "tt_device=0191f7ac-1234-7000-8000-abcdefabcdef";

    /// The device's requests, straight into the router, with its cookies.
    struct Wire {
        state: AppState,
        cookies: Option<String>,
    }

    impl Wire {
        async fn send(&self, request: axum::http::request::Builder, body: Body) -> Reply {
            let request = match &self.cookies {
                Some(c) => request.header(header::COOKIE, c),
                None => request,
            };
            let response = router(self.state.clone())
                .oneshot(request.body(body).unwrap())
                .await
                .unwrap();
            let status = response.status().as_u16();
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            Reply {
                status,
                body: String::from_utf8_lossy(&bytes).into_owned(),
            }
        }
    }

    impl Transport for &Wire {
        async fn get(&self, path: &str) -> Result<Reply, String> {
            Ok(self.send(Request::get(path), Body::empty()).await)
        }
        async fn post_json(&self, path: &str, body: String) -> Result<Reply, String> {
            let request = Request::post(path).header(header::CONTENT_TYPE, "application/json");
            Ok(self.send(request, Body::from(body)).await)
        }
    }

    fn observation(id: &str, team: i32) -> NewObservation {
        let payload: Payload = [
            ("starting_position", Value::Text("center".into())),
            ("teleop_scored", Value::Count(9)),
            ("broke_down", Value::Flag(true)),
            ("notes", Value::Text("tippy on the ramp".into())),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
        NewObservation {
            client_record_id: id.into(),
            match_key: "2026now_qm2".into(),
            event_key: "2026now".into(),
            team_number: team,
            alliance: "red",
            payload,
            schema_version: 1,
            scouter_id: Some(1),
            device_id: None,
            submitting_team: Some(10101),
            observed_at: Utc::now() - TimeDelta::minutes(20),
        }
    }

    async fn seed(state: &AppState) {
        let now = Utc::now();
        let event = Event {
            key: "2026now".into(),
            name: "This Weekend".into(),
            location: None,
            timezone: None,
            start_date: Some(now.date_naive()),
            end_date: Some(now.date_naive()),
            event_code: None,
            event_type: None,
            district_key: None,
            week: None,
        };
        state.repo.upsert_event(&event, now).await.unwrap();
        for number in [10101, 254] {
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
            state.repo.upsert_team(&team, now).await.unwrap();
            state
                .repo
                .link_event_team("2026now", number, now)
                .await
                .unwrap();
        }
        let game = MatchRecord {
            key: "2026now_qm2".into(),
            event_key: "2026now".into(),
            comp_level: CompLevel::Qualification,
            set_number: 1,
            match_number: 2,
            red: [Some(10101), Some(254), Some(1)],
            blue: [Some(2), Some(3), Some(4)],
            red_score: None,
            blue_score: None,
            winner: None,
            played: false,
            scheduled_at: None,
            actual_at: None,
        };
        state.repo.upsert_match(&game, now).await.unwrap();
    }

    #[tokio::test]
    async fn a_scout_who_saved_offline_is_recorded_when_the_tablet_reconnects() {
        // A file: the snapshot is VACUUM INTO, which an in-memory Pi cannot.
        let dir = std::env::temp_dir().join(format!("tt-web-push-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let repo =
            SqliteRepo::connect(&format!("sqlite://{}", dir.join("pi.db").display())).unwrap();
        tt_repo_sqlite::migrate::apply(repo.pool()).await.unwrap();
        let state = AppState {
            repo: Arc::new(repo),
            season: Arc::new(tt_core::season::current_season().unwrap()),
            upstream: Arc::new(crate::upstream::Upstream::disabled()),
            snapshots: Default::default(),
        };
        seed(&state).await;
        let anonymous = Wire {
            state: state.clone(),
            cookies: None,
        };
        let signup = "name=Sam&email=sam%40example.com&team_number=10101\
                      &password=longenough1&confirm_password=longenough1";
        let response = router(state.clone())
            .oneshot(
                Request::post("/api/auth/signup")
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from(signup))
                    .unwrap(),
            )
            .await
            .unwrap();
        let session = response.headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string();
        let sam = Wire {
            state: state.clone(),
            cookies: Some(format!("{session}; {DEVICE}")),
        };
        (&sam)
            .post_json("/api/device/heartbeat", String::new())
            .await
            .unwrap();

        // The tablet's copy, and two observations made with no signal: one
        // good, one of a robot that is not in the match.
        let snapshot = router(state.clone())
            .oneshot(
                Request::get("/api/sync/snapshot?event=2026now")
                    .header(header::COOKIE, sam.cookies.as_deref().unwrap())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(snapshot.status(), 200);
        let bytes = axum::body::to_bytes(snapshot.into_body(), usize::MAX)
            .await
            .unwrap();
        let device = ClientRepo::from_bytes(&bytes).unwrap();
        device
            .queue_observation(&observation(MINE, 254), Utc::now())
            .await
            .unwrap();
        device
            .queue_observation(&observation(WRONG, 9999), Utc::now())
            .await
            .unwrap();

        let client = SyncClient::new(&sam, vec!["2026now".into()]);
        let report = client.sync(&device, Utc::now()).await.unwrap();
        assert_eq!((report.sent, report.refused, report.stopped), (2, 1, None));
        let on_the_pi = state.repo.pending_observations("2026now").await.unwrap();
        assert_eq!(on_the_pi.len(), 1);
        let saved = &on_the_pi[0];
        assert_eq!(saved.team_number, 254);
        assert_eq!(saved.alliance, "red");
        assert_eq!(saved.scouter_name.as_deref(), Some("Sam"));
        assert_eq!(saved.submitting_team, Some(10101));
        assert_eq!(
            saved.payload.get("notes"),
            Some(&Value::Text("tippy on the ramp".into()))
        );
        let outbox = device.outbox().unwrap();
        assert_eq!(
            outbox[1].refused.as_deref(),
            Some("Not saved: team 9999 is not in 2026now_qm2.")
        );
        assert!(device.is_queued(MINE).unwrap(), "not until it comes back");

        // Once the change has settled into the log, it comes back.
        tokio::time::sleep(
            (crate::sync::LAG + TimeDelta::milliseconds(300))
                .to_std()
                .unwrap(),
        )
        .await;
        let report = client.sync(&device, Utc::now()).await.unwrap();
        assert_eq!((report.sent, report.echoed, report.stopped), (1, 1, None));
        assert!(!device.is_queued(MINE).unwrap());
        assert_eq!(device.outbox().unwrap().len(), 1, "the refused one stays");
        assert_eq!(
            state
                .repo
                .pending_observations("2026now")
                .await
                .unwrap()
                .len(),
            1,
            "sent twice, stored once"
        );
        // The device's repo is the browser's: its own trait, the same methods.
        let mine = tt_repo::LocalRepo::pending_observations(&device, "2026now")
            .await
            .unwrap();
        assert_eq!(mine.len(), 1);
        // Sam's row as the Pi has it. Not Sam's name: the snapshot was
        // taken before any row named Sam, and keeps only those (S10b).
        assert_eq!(mine[0].scouter_id, saved.scouter_id);

        // Refusals of the whole push say why, and never redirect.
        let push = |n: usize| {
            serde_json::to_string(&Push {
                observations: vec![
                    QueuedObservation {
                        client_record_id: MINE.into(),
                        match_key: "2026now_qm2".into(),
                        team_number: 254,
                        payload: Payload::new(),
                        schema_version: 1,
                        observed_at: Utc::now(),
                    };
                    n
                ],
            })
            .unwrap()
        };
        let status = |reply: Reply| reply.status;
        assert_eq!(
            status(
                (&anonymous)
                    .post_json("/api/sync/push", push(1))
                    .await
                    .unwrap()
            ),
            401
        );
        let ahead = format!(
            "/api/sync/push?schema={}",
            tt_repo_sqlite::migrate::latest() + 1
        );
        let refused = (&sam).post_json(&ahead, push(1)).await.unwrap();
        assert_eq!(refused.status, 409);
        assert!(refused.body.contains("server-behind"), "{}", refused.body);
        assert_eq!(
            status(
                (&sam)
                    .post_json("/api/sync/push", "[1, 2]".into())
                    .await
                    .unwrap()
            ),
            422
        );
        assert_eq!(
            status(
                (&sam)
                    .post_json("/api/sync/push", push(MAX_PER_PUSH + 1))
                    .await
                    .unwrap()
            ),
            413
        );
        // On this form, answers that do not fit it are refused.
        let reply = (&sam).post_json("/api/sync/push", push(1)).await.unwrap();
        let reply: PushReply = serde_json::from_str(&reply.body).unwrap();
        assert!(
            matches!(&reply.receipts[0].outcome, Outcome::Refused { reason } if reason.contains("do not fit")),
            "{reply:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_tablet_clock_is_corrected_and_the_future_is_now() {
        let now = Utc.with_ymd_and_hms(2026, 3, 14, 12, 0, 0).unwrap();
        let tablet = |offset_ms| Device {
            id: 1,
            device_uuid: "d".into(),
            name: None,
            team_number: None,
            last_seen_at: None,
            last_user_id: None,
            clock_offset_ms: offset_ms,
        };
        let watched = now - TimeDelta::minutes(30);
        // Four minutes slow: the server's clock was four minutes later.
        let slow = tablet(Some(240_000));
        assert_eq!(
            observed_at(watched, Some(&slow), now),
            watched + TimeDelta::minutes(4)
        );
        assert_eq!(observed_at(watched, Some(&tablet(None)), now), watched);
        assert_eq!(observed_at(watched, None, now), watched);
        let fast = now + TimeDelta::hours(2);
        assert_eq!(observed_at(fast, None, now), now);
    }
}
