//! `POST /api/sync/push` (C7): a device's outbox, delivered.
//!
//! The body is a [`Push`] as JSON: observations a scout recorded on the
//! device, likely with no signal. Each one is checked by the same rules as
//! the scouting form's post, and recorded as the signed-in scout, from the
//! tablet whose cookie came with it. A scout whose session has lapsed is
//! still known by the device's offline token (C9, `crate::token`). The answer is a [`Receipt`] per entry:
//! recorded (now, or already), or refused with a reason a person can read.
//! The Pi keeps each refusal for the lead scout (C10, `crate::refused`).
//! The device clears an entry only once it comes back through the change
//! log (`tt_core::outbox` says why).
//!
//! Refusals of the whole push are status codes with a JSON reason, never the
//! guards' redirect, so the device knows to keep its queue:
//!   - 401 not signed in, and no token from this device,
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
use tt_repo::{Device, NewObservation, Recorded, Repo, RepoError};
use tt_repo_sqlite::refused::NewRefusal;

use crate::auth::device_uuid;
use crate::startup::AppState;

fn refuse(status: StatusCode, error: impl Into<String>) -> Response {
    (status, Json(json!({ "error": error.into() }))).into_response()
}

pub async fn push(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    body: Bytes,
) -> Response {
    let Some(user) = crate::token::sync_user(&state, &headers).await else {
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
/// for storage; everything about the entry itself is an [`Outcome`]. A
/// refusal is kept for the lead scout (C10) before it is answered, so one
/// the Pi could not keep is a 503 and the device sends it again. A handoff
/// by QR (S13) records each form through here too.
pub(crate) async fn record(
    state: &AppState,
    user: &User,
    device: Option<&Device>,
    mut queued: QueuedObservation,
    now: DateTime<Utc>,
) -> Result<Outcome, RepoError> {
    queued.observed_at = observed_at(queued.observed_at, device, now);
    let author = Author {
        scouter_id: Some(user.id),
        device_id: device.map(|d| d.id),
        submitting_team: user.team_number,
    };
    match record_as(state, &queued, author, now).await? {
        Ok(_) => Ok(Outcome::Recorded),
        Err(reason) => {
            let refusal = NewRefusal {
                entry: queued,
                scouter_id: author.scouter_id,
                device_id: author.device_id,
                submitting_team: author.submitting_team,
                reason: reason.clone(),
            };
            state.repo.keep_refusal(&refusal, now).await?;
            Ok(Outcome::Refused { reason })
        }
    }
}

/// Who an entry is recorded as. For a push, the signed-in scout, the
/// tablet, and the scout's team; for a lead recording a refusal (C10), the
/// same three as the push kept them.
#[derive(Debug, Clone, Copy)]
pub struct Author {
    pub scouter_id: Option<i64>,
    pub device_id: Option<i64>,
    pub submitting_team: Option<i32>,
}

/// Check an entry by the form's rules and record it. `observed_at` is
/// already on the Pi's clock. The inner `Err` is why it was not recorded,
/// for a person; the outer one is storage.
pub async fn record_as(
    state: &AppState,
    queued: &QueuedObservation,
    author: Author,
    now: DateTime<Utc>,
) -> Result<Result<Recorded, String>, RepoError> {
    let Some(client_record_id) = record_id::normalize(&queued.client_record_id) else {
        return Ok(Err("Not saved: it has no record id.".into()));
    };
    // The schedule decides the event and alliance, never the device.
    let Some(record) = state.repo.match_by_key(&queued.match_key).await? else {
        return Ok(Err(format!(
            "Not saved: {} is not on the schedule.",
            queued.match_key
        )));
    };
    let Some(alliance) = record.alliance_of(queued.team_number) else {
        return Ok(Err(format!(
            "Not saved: team {} is not in {}.",
            queued.team_number, record.key
        )));
    };
    // Answers on another form are kept as they are, and the review page
    // flags them (S11); on this form they must fit it.
    if queued.schema_version == state.season.version
        && let Err(e) = state.season.validate_payload(&queued.payload)
    {
        return Ok(Err(format!(
            "Not saved: the answers do not fit the form ({e})."
        )));
    }

    let observation = NewObservation {
        client_record_id,
        match_key: record.key.clone(),
        event_key: record.event_key.clone(),
        team_number: queued.team_number,
        alliance,
        payload: queued.payload.clone(),
        schema_version: queued.schema_version,
        scouter_id: author.scouter_id,
        device_id: author.device_id,
        submitting_team: author.submitting_team,
        observed_at: queued.observed_at,
    };
    match state.repo.record_observation(&observation, now).await {
        Ok(recorded) => Ok(Ok(recorded)),
        Err(RepoError::Conflict { .. }) => Ok(Err(format!(
            "Not saved: the scout already has an observation of team {} in {}.",
            observation.team_number, record.key
        ))),
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
        /// An offline token (C9), sent as `Authorization: Bearer`.
        bearer: Option<String>,
    }

    impl Wire {
        async fn send(&self, request: axum::http::request::Builder, body: Body) -> Reply {
            let request = match &self.cookies {
                Some(c) => request.header(header::COOKIE, c),
                None => request,
            };
            let request = match &self.bearer {
                Some(t) => request.header(header::AUTHORIZATION, format!("Bearer {t}")),
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
        async fn post_file(&self, path: &str, body: Vec<u8>) -> Result<Reply, String> {
            let request =
                Request::post(path).header(header::CONTENT_TYPE, "application/vnd.sqlite3");
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
            tokens: Default::default(),
        };
        seed(&state).await;
        let anonymous = Wire {
            state: state.clone(),
            cookies: None,
            bearer: None,
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
            bearer: None,
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

        // The Pi kept the refusal for the lead scout (C10). Sam made the
        // first account, so Sam is an admin and may act as lead.
        let open = state.repo.open_refusals().await.unwrap();
        assert_eq!(open.len(), 1);
        let kept = &open[0];
        assert_eq!(kept.entry.client_record_id, WRONG);
        assert_eq!(kept.reason, outbox[1].refused.clone().unwrap());
        assert_eq!(kept.scouter_name.as_deref(), Some("Sam"));
        assert_eq!(kept.submitting_team, Some(10101));
        assert!(kept.device_name.is_some(), "the tablet it came from");
        let lead_page = sam
            .send(Request::get("/lead-scout?event=2026now"), Body::empty())
            .await;
        assert!(
            lead_page.body.contains("Refused when sent"),
            "{}",
            lead_page.body
        );
        assert!(
            lead_page.body.contains("Q2 · Team 9999"),
            "{}",
            lead_page.body
        );
        let href = format!("/lead-scout/refused/{}", kept.id);
        assert!(lead_page.body.contains(&href));
        let detail = sam
            .send(Request::get(format!("{href}?event=2026now")), Body::empty())
            .await;
        assert_eq!(detail.status, 200);
        assert!(
            detail.body.contains("tippy on the ramp"),
            "Sam's own team's notes"
        );
        assert!(detail.body.contains("is not in 2026now_qm2"));

        let form = |body: &'static str| {
            sam.send(
                Request::post(format!("/api/refused/{}/record?event=2026now", kept.id))
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded"),
                Body::from(body),
            )
        };
        // The same checks as the push: Sam already has 254 in this match.
        let again = form("match=2026now_qm2&team=254").await;
        assert_eq!(again.status, 200);
        assert!(
            again
                .body
                .contains("already has an observation of team 254"),
            "{}",
            again.body
        );
        let again = form("match=2026now_qm2&team=many").await;
        assert!(again.body.contains("must be a number"), "{}", again.body);
        assert_eq!(state.repo.open_refusals().await.unwrap().len(), 1);
        // Sam meant the robot beside it.
        let recorded = form("match=2026now_qm2&team=1").await;
        assert_eq!(recorded.status, 303, "{}", recorded.body);
        assert!(state.repo.open_refusals().await.unwrap().is_empty());
        let pending = state.repo.pending_observations("2026now").await.unwrap();
        let fixed = pending.iter().find(|o| o.team_number == 1).unwrap();
        assert_eq!(fixed.scouter_name.as_deref(), Some("Sam"));
        assert_eq!(fixed.submitting_team, Some(10101));
        assert_eq!(fixed.observed_at, Some(kept.entry.observed_at));
        let done = sam
            .send(Request::get(format!("{href}?event=2026now")), Body::empty())
            .await;
        assert!(done.body.contains("Recorded by Sam"), "{}", done.body);
        assert!(!done.body.contains("Record it"));
        let lead_page = sam
            .send(Request::get("/lead-scout?event=2026now"), Body::empty())
            .await;
        assert!(!lead_page.body.contains("Refused when sent"));
        let twice = form("match=2026now_qm2&team=1").await;
        assert!(twice.body.contains("already dealt with"), "{}", twice.body);

        // It kept its record id, so the tablet's refused entry clears when
        // it comes back, like any other.
        tokio::time::sleep(
            (crate::sync::LAG + TimeDelta::milliseconds(300))
                .to_std()
                .unwrap(),
        )
        .await;
        let report = client.sync(&device, Utc::now()).await.unwrap();
        assert_eq!((report.sent, report.echoed), (0, 1));
        assert!(device.outbox().unwrap().is_empty());

        // One nobody can place is dismissed, and stays dismissed.
        let mut lost = kept.clone();
        lost.entry.client_record_id = "0191f7ac-1234-7000-8000-000000000003".into();
        lost.entry.match_key = "2026now_sf9m1".into();
        let refusal = tt_repo_sqlite::refused::NewRefusal {
            entry: lost.entry,
            scouter_id: lost.scouter_id,
            device_id: lost.device_id,
            submitting_team: lost.submitting_team,
            reason: "Not saved: 2026now_sf9m1 is not on the schedule.".into(),
        };
        state.repo.keep_refusal(&refusal, Utc::now()).await.unwrap();
        let id = state.repo.open_refusals().await.unwrap()[0].id;
        let dismissed = sam
            .send(
                Request::post(format!("/api/refused/{id}/dismiss?event=2026now")),
                Body::empty(),
            )
            .await;
        assert_eq!(dismissed.status, 303);
        assert!(state.repo.open_refusals().await.unwrap().is_empty());
        let done = sam
            .send(
                Request::get(format!("/lead-scout/refused/{id}")),
                Body::empty(),
            )
            .await;
        assert!(done.body.contains("Dismissed by Sam"), "{}", done.body);
        assert_eq!(
            sam.send(Request::get("/lead-scout/refused/999"), Body::empty())
                .await
                .status,
            404
        );

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

    #[tokio::test]
    async fn a_lead_corrects_a_refused_entrys_answers_and_the_chip_counts_what_waits() {
        let repo = SqliteRepo::connect("sqlite::memory:").unwrap();
        tt_repo_sqlite::migrate::apply(repo.pool()).await.unwrap();
        let state = AppState {
            repo: Arc::new(repo),
            season: Arc::new(tt_core::season::current_season().unwrap()),
            upstream: Arc::new(crate::upstream::Upstream::disabled()),
            snapshots: Default::default(),
            tokens: Default::default(),
        };
        seed(&state).await;
        let sign_up = async |name: &str, team: i32| {
            let form = format!(
                "name={name}&email={name}%40example.com&team_number={team}\
                 &password=longenough1&confirm_password=longenough1"
            );
            let response = router(state.clone())
                .oneshot(
                    Request::post("/api/auth/signup")
                        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                        .body(Body::from(form))
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
            Wire {
                state: state.clone(),
                cookies: Some(session),
                bearer: None,
            }
        };
        // Sam made the first account, so may act as lead; Kim scouts for 254.
        let sam = sign_up("Sam", 10101).await;
        let kim = sign_up("Kim", 254).await;
        let chip = |page: &Reply| {
            let at = page.body.find(r#"data-link="review""#)?;
            let tag = page.body[..at].rfind('<').unwrap();
            let end = at + page.body[at..].find('>').unwrap();
            Some(page.body[tag..=end].to_string())
        };
        let home = async |wire: &Wire| {
            wire.send(Request::get("/?event=2026now"), Body::empty())
                .await
        };
        assert_eq!(chip(&home(&kim).await), None, "nothing waits");

        // Kim's tablet sent 99 scored in teleop, and the form stops at 60.
        let payload: Payload = [
            ("starting_position", Value::Text("center".into())),
            ("teleop_scored", Value::Count(99)),
            ("notes", Value::Text("Kim's own notes".into())),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
        let entry = QueuedObservation {
            client_record_id: MINE.into(),
            match_key: "2026now_qm2".into(),
            team_number: 1,
            payload,
            schema_version: state.season.version,
            observed_at: Utc::now() - TimeDelta::minutes(5),
        };
        let refusal = NewRefusal {
            entry: entry.clone(),
            scouter_id: Some(2),
            device_id: None,
            submitting_team: Some(254),
            reason: "Not saved: the answers do not fit the form (teleop_scored).".into(),
        };
        state.repo.keep_refusal(&refusal, Utc::now()).await.unwrap();
        let id = state.repo.open_refusals().await.unwrap()[0].id;

        // Both chips say so; only the lead's goes somewhere.
        let kims = chip(&home(&kim).await).expect("Kim's chip");
        assert!(
            kims.starts_with("<span") && !kims.contains(" hidden"),
            "{kims}"
        );
        let sams = chip(&home(&sam).await).expect("Sam's chip");
        assert!(
            sams.contains(r#"href="/lead-scout?event=2026now#refused""#),
            "{sams}"
        );
        assert!(home(&sam).await.body.contains("1 needs review"));

        // The detail page is the form, saying what does not fit, and Kim's
        // notes are not Sam's to read.
        let href = format!("/lead-scout/refused/{id}?event=2026now");
        let detail = sam.send(Request::get(&href), Body::empty()).await;
        assert!(
            detail.body.contains("Must be between 0 and 60."),
            "{}",
            detail.body
        );
        assert!(detail.body.contains(r#"name="f.teleop_scored" value="99""#));
        assert!(
            detail
                .body
                .contains("Only scouts on team 254 can read these notes.")
        );
        assert!(!detail.body.contains("Kim&#39;s own notes") && !detail.body.contains("Kim's own"));

        let record = |body: &'static str| {
            sam.send(
                Request::post(format!("/api/refused/{id}/record?event=2026now"))
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded"),
                Body::from(body),
            )
        };
        // Untouched, it is refused for the same reason as the push.
        let again = record(
            "match=2026now_qm2&team=1&answers=1&f.starting_position=center\
             &f.teleop_scored=99&f.auto_scored=&f.penalties=",
        )
        .await;
        assert_eq!(again.status, 200);
        assert!(
            again.body.contains("do not fit the form (teleop_scored"),
            "{}",
            again.body
        );
        // Wrong in the form: said by the field, with what Sam typed kept.
        let again = record(
            "match=2026now_qm2&team=1&answers=1&f.starting_position=center\
             &f.teleop_scored=9x&f.auto_scored=&f.penalties=",
        )
        .await;
        assert_eq!(again.status, 200);
        assert!(
            again.body.contains("Enter a whole number."),
            "{}",
            again.body
        );
        assert!(again.body.contains(r#"value="9x""#));
        assert_eq!(state.repo.open_refusal_count(None).await.unwrap(), 1);

        // Corrected. Notes Sam was sent in their place are not kept.
        let recorded = record(
            "match=2026now_qm2&team=1&answers=1&f.starting_position=center\
             &f.teleop_scored=12&f.auto_scored=&f.penalties=&f.notes=gone",
        )
        .await;
        assert_eq!(recorded.status, 303, "{}", recorded.body);
        let pending = state.repo.pending_observations("2026now").await.unwrap();
        let fixed = pending.iter().find(|o| o.team_number == 1).unwrap();
        assert_eq!(fixed.payload["teleop_scored"], Value::Count(12));
        assert_eq!(
            fixed.payload["notes"],
            Value::Text("Kim's own notes".into())
        );
        assert_eq!(fixed.payload["broke_down"], Value::Flag(false));
        assert_eq!(fixed.scouter_name.as_deref(), Some("Kim"));
        assert_eq!(chip(&home(&kim).await), None);
        assert_eq!(chip(&home(&sam).await), None);

        // From an older form and posted back untouched, the answers stay
        // exactly as sent, including what this form no longer asks.
        let mut old = entry;
        old.client_record_id = WRONG.into();
        old.team_number = 254;
        old.schema_version = state.season.version - 1;
        old.payload = [
            ("teleop_scored".to_string(), Value::Count(5)),
            ("hang_level".to_string(), Value::Text("l3".into())),
        ]
        .into();
        let refusal = NewRefusal {
            entry: old,
            reason: "Not saved: 2026now_qm2 is not on the schedule.".into(),
            ..refusal
        };
        state.repo.keep_refusal(&refusal, Utc::now()).await.unwrap();
        let id = state.repo.open_refusals().await.unwrap()[0].id;
        let detail = sam
            .send(
                Request::get(format!("/lead-scout/refused/{id}?event=2026now")),
                Body::empty(),
            )
            .await;
        assert!(
            detail.body.contains("Not on the current form"),
            "{}",
            detail.body
        );
        assert!(
            !detail.body.contains("form-error"),
            "nothing flagged on another form"
        );
        let recorded = sam
            .send(
                Request::post(format!("/api/refused/{id}/record?event=2026now"))
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded"),
                Body::from(
                    "match=2026now_qm2&team=254&answers=1&f.teleop_scored=5\
                     &f.auto_scored=&f.penalties=&f.notes=",
                ),
            )
            .await;
        assert_eq!(recorded.status, 303, "{}", recorded.body);
        let pending = state.repo.pending_observations("2026now").await.unwrap();
        let kept = pending.iter().find(|o| o.team_number == 254).unwrap();
        assert_eq!(kept.schema_version, state.season.version - 1);
        assert_eq!(kept.payload, refusal.entry.payload);
    }

    #[tokio::test]
    async fn a_scout_whose_session_ran_out_still_syncs_with_the_tablets_token() {
        let repo = SqliteRepo::connect("sqlite::memory:").unwrap();
        tt_repo_sqlite::migrate::apply(repo.pool()).await.unwrap();
        let state = AppState {
            repo: Arc::new(repo),
            season: Arc::new(tt_core::season::current_season().unwrap()),
            upstream: Arc::new(crate::upstream::Upstream::disabled()),
            snapshots: Default::default(),
            tokens: Default::default(),
        };
        seed(&state).await;
        let wire = |cookies: Option<String>, bearer: Option<String>| Wire {
            state: state.clone(),
            cookies,
            bearer,
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

        // Friday, signed in: the tablet takes a token.
        let sam = wire(Some(format!("{session}; {DEVICE}")), None);
        let issued = (&sam)
            .post_json("/api/auth/token", String::new())
            .await
            .unwrap();
        assert_eq!(issued.status, 200, "{}", issued.body);
        let issued: serde_json::Value = serde_json::from_str(&issued.body).unwrap();
        let token = issued["token"].as_str().unwrap().to_string();
        let claims = tt_core::token::read_unverified(&token).unwrap();
        assert_eq!(claims.name, "Sam");
        assert_eq!(claims.team, Some(10101));
        assert_eq!(format!("tt_device={}", claims.device), DEVICE);
        // The device can check it with the key that came with it.
        use base64::Engine;
        let key: [u8; 32] = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(issued["public_key"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap();
        assert_eq!(tt_core::token::verify(&key, &token, Utc::now()), Ok(claims));
        // Only for someone signed in, on a browser with a device id.
        let anonymous = wire(Some(DEVICE.into()), None);
        let refused = (&anonymous)
            .post_json("/api/auth/token", String::new())
            .await
            .unwrap();
        assert_eq!(refused.status, 303);
        let deviceless = wire(Some(session.clone()), None);
        let refused = (&deviceless)
            .post_json("/api/auth/token", String::new())
            .await
            .unwrap();
        assert_eq!(refused.status, 400);

        // Saturday: the session is gone, and the tablet has a saved match.
        (&sam)
            .post_json("/api/auth/logout", String::new())
            .await
            .unwrap();
        let saved = observation(MINE, 254);
        let push = serde_json::to_string(&Push {
            observations: vec![QueuedObservation {
                client_record_id: MINE.into(),
                match_key: saved.match_key,
                team_number: saved.team_number,
                payload: saved.payload,
                schema_version: saved.schema_version,
                observed_at: saved.observed_at,
            }],
        })
        .unwrap();
        for nobody in [&sam, &anonymous] {
            assert_eq!(
                nobody
                    .post_json("/api/sync/push", push.clone())
                    .await
                    .unwrap()
                    .status,
                401
            );
            assert_eq!(nobody.get("/api/sync/pull").await.unwrap().status, 401);
        }

        // With the token, from the tablet it was issued to: recorded as Sam.
        let tablet = wire(Some(DEVICE.into()), Some(token.clone()));
        let reply = (&tablet)
            .post_json("/api/sync/push", push.clone())
            .await
            .unwrap();
        assert_eq!(reply.status, 200, "{}", reply.body);
        let reply: PushReply = serde_json::from_str(&reply.body).unwrap();
        assert_eq!(reply.receipts[0].outcome, Outcome::Recorded);
        let on_the_pi = state.repo.pending_observations("2026now").await.unwrap();
        assert_eq!(on_the_pi[0].scouter_name.as_deref(), Some("Sam"));
        assert_eq!(on_the_pi[0].submitting_team, Some(10101));
        assert_eq!((&tablet).get("/api/sync/pull").await.unwrap().status, 200);
        // A token is not a session: pages still want Sam to sign in.
        assert_eq!((&tablet).get("/account").await.unwrap().status, 303);

        // Copied to another browser, altered, or nonsense: refused.
        let elsewhere = wire(
            Some("tt_device=0191f7ac-1234-7000-8000-000000000999".into()),
            Some(token.clone()),
        );
        let mut altered = token.clone();
        altered.pop();
        altered.push(if token.ends_with('A') { 'B' } else { 'A' });
        for refused in [
            elsewhere,
            wire(Some(DEVICE.into()), Some(altered)),
            wire(Some(DEVICE.into()), Some("v4.public.nonsense".into())),
            wire(None, Some(token)),
        ] {
            let refused = &refused;
            assert_eq!(
                refused
                    .post_json("/api/sync/push", push.clone())
                    .await
                    .unwrap()
                    .status,
                401
            );
            assert_eq!(refused.get("/api/sync/pull").await.unwrap().status, 401);
        }
    }

    /// TBA for 2026now, on localhost: Q2 played, and an OPR for 254.
    async fn tba_stub() -> String {
        use axum::extract::Path;
        use axum::routing::get;

        async fn resource(Path((event, what)): Path<(String, String)>) -> Response {
            let body = match (event.as_str(), what.as_str()) {
                ("2026now", "matches") => {
                    r#"[{"key":"2026now_qm2","comp_level":"qm","set_number":1,"match_number":2,
                    "alliances":{"red":{"score":88,"team_keys":["frc10101","frc254","frc1"]},
                    "blue":{"score":74,"team_keys":["frc2","frc3","frc4"]}},
                    "winning_alliance":"red"}]"#
                }
                ("2026now", "oprs") => r#"{"oprs":{"frc254":51.5},"dprs":{},"ccwms":{}}"#,
                ("2026now", "rankings") => r#"{"rankings":[],"sort_order_info":[]}"#,
                ("2026now", "coprs") => "{}",
                _ => return StatusCode::NOT_FOUND.into_response(),
            };
            ([(header::ETAG, format!("W/\"{what}\""))], body).into_response()
        }
        let app = axum::Router::new().route("/event/{event}/{what}", get(resource));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn a_lead_scouts_tablet_hands_the_pi_what_it_fetched_with_its_own_signal() {
        // S7. A file: the snapshot is VACUUM INTO, and a bundle is ATTACHed.
        let dir = std::env::temp_dir().join(format!("tt-web-courier-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let repo =
            SqliteRepo::connect(&format!("sqlite://{}", dir.join("pi.db").display())).unwrap();
        tt_repo_sqlite::migrate::apply(repo.pool()).await.unwrap();
        let tba_base = tba_stub().await;
        // The Pi's own key, and no internet of its own today.
        let uplink = tt_upstream::Uplink::new();
        let tba = tt_upstream::tba::TbaClient::new("the-key", uplink.clone())
            .unwrap()
            .with_base_url(&tba_base);
        let state = AppState {
            repo: Arc::new(repo),
            season: Arc::new(tt_core::season::current_season().unwrap()),
            upstream: Arc::new(crate::upstream::Upstream::new(
                None,
                Some(tba),
                tt_upstream::first::EventFilters::all(),
                uplink,
            )),
            snapshots: Default::default(),
            tokens: Default::default(),
        };
        seed(&state).await;
        let wire = |cookies: Option<String>, bearer: Option<String>| Wire {
            state: state.clone(),
            cookies,
            bearer,
        };
        let sign_up = |name: &'static str| {
            let form = format!(
                "name={name}&email={name}%40example.com&team_number=10101\
                 &password=longenough1&confirm_password=longenough1"
            );
            let app = router(state.clone());
            async move {
                let response = app
                    .oneshot(
                        Request::post("/api/auth/signup")
                            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                            .body(Body::from(form))
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                response.headers()[header::SET_COOKIE]
                    .to_str()
                    .unwrap()
                    .split(';')
                    .next()
                    .unwrap()
                    .to_string()
            }
        };
        // Sam made the first account, so may act as lead; Kim scouts.
        let sam = wire(Some(format!("{}; {DEVICE}", sign_up("Sam").await)), None);
        let kim = wire(Some(sign_up("Kim").await), None);
        (&sam)
            .post_json("/api/device/heartbeat", String::new())
            .await
            .unwrap();

        // The key goes to a lead scout, and to no one else.
        let anonymous = wire(Some(DEVICE.into()), None);
        assert_eq!(
            (&anonymous).get("/api/upstream/key").await.unwrap().status,
            401
        );
        let refused = (&kim).get("/api/upstream/key").await.unwrap();
        assert_eq!(refused.status, 403, "{}", refused.body);
        let given = (&sam).get("/api/upstream/key").await.unwrap();
        assert_eq!(given.status, 200, "{}", given.body);
        let given: serde_json::Value = serde_json::from_str(&given.body).unwrap();
        assert_eq!(given["tba"], "the-key");
        assert_eq!(given["base"], tba_base.as_str());
        assert_eq!(given["uplink_online"], false);

        // Friday: the tablet's copy and its token. Saturday the session is
        // gone, and the tablet found signal in the lobby.
        let snapshot = router(state.clone())
            .oneshot(
                Request::get("/api/sync/snapshot?event=2026now")
                    .header(header::COOKIE, sam.cookies.as_deref().unwrap())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = axum::body::to_bytes(snapshot.into_body(), usize::MAX)
            .await
            .unwrap();
        let device = ClientRepo::from_bytes(&bytes).unwrap();
        let issued = (&sam)
            .post_json("/api/auth/token", String::new())
            .await
            .unwrap();
        let issued: serde_json::Value = serde_json::from_str(&issued.body).unwrap();
        let token = issued["token"].as_str().unwrap().to_string();
        (&sam)
            .post_json("/api/auth/logout", String::new())
            .await
            .unwrap();
        let tablet = wire(Some(DEVICE.into()), Some(token));

        let courier = tt_client::courier::Courier::new(&tablet, vec!["2026now".into()]);
        let tick = courier
            .tick(&device, Utc::now(), || "tablet-log".into())
            .await
            .unwrap();
        assert!(tick.pi && tick.key, "{tick:?}");
        assert_eq!(tick.stopped, None, "{tick:?}");
        assert_eq!(
            (tick.fetched, tick.pushed, tick.waiting),
            (4, 4, 0),
            "{tick:?}"
        );

        // On the Pi, in its tables, as if it had fetched them itself.
        let played = state
            .repo
            .match_by_key("2026now_qm2")
            .await
            .unwrap()
            .unwrap();
        assert_eq!((played.red_score, played.blue_score), (Some(88), Some(74)));
        let last = state.repo.bundle_imports(1).await.unwrap();
        assert_eq!(last[0].user.as_deref(), Some("Sam"));
        assert_eq!(last[0].appended, 4);
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
