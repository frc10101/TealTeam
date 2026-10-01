//! A tablet's forms, carried by QR (S13): `GET /lead-scout/scan` and what
//! the Pi makes of the codes it reads, `POST /api/handoff`.
//!
//! A scout whose tablet cannot reach the Pi shows their unsent forms as a
//! [`Handoff`] in QR codes (`static/js/handoff.js`). A lead scout at the Pi
//! scans them here. The page posts the frames' texts; this module puts them
//! together (`tt_core::qr`), checks the scout's offline token (C9), and
//! records each form through the push's own path (`crate::push::record`):
//! the form's rules, as the scout who saved it, from their tablet, with its
//! clock corrected (S12), and a refusal kept for a lead (C10). The answer is
//! a page saying what became of each, with a receipt drawn as a code for the
//! tablet to scan back, so it lets go of what the Pi now has.
//!
//! The token is what makes this safe to accept: it says who saved the forms,
//! and only the Pi could have signed it. It is not tied to the device that
//! posts, as a push's is, since the codes come off another screen. So a copy
//! of a scout's token could be shown as them by anyone, but only a lead
//! scout can scan it in, and only within the token's 72 hours.

use chrono::{DateTime, Utc};
use tracing::{info, warn};
use tt_core::form::{RawAnswers, read_answers_as_given};
use tt_core::outbox::{MAX_PER_PUSH, Outcome, QueuedObservation, Receipt};
use tt_core::qr::{self, HandedForm, Handoff, HandoffReceipts, Kind};
use tt_core::user::User;
use tt_repo::Repo;
use tt_templates::{HandoffResult, HandoffRow, Nav, QrFrame, ScanPage};

use crate::startup::AppState;

/// The scanner, or what came of the last scan.
pub fn page(nav: Nav, result: Option<HandoffResult>, errors: Vec<String>) -> ScanPage {
    let query = nav.event.query();
    ScanPage {
        title: "Scan a tablet's forms".into(),
        post_href: format!("/api/handoff{query}"),
        back_href: format!("/lead-scout{query}"),
        nav,
        errors,
        result,
    }
}

/// Record what the frames carry. `Err` is why nothing was recorded, for the
/// lead; the page shows the scanner again.
pub async fn receive(
    state: &AppState,
    lead: &User,
    frames: &str,
    query: &str,
) -> Result<HandoffResult, Vec<String>> {
    let fail = |why: String| Err(vec![why]);
    let (kind, bytes) = match qr::assemble(frames.lines().filter(|l| !l.trim().is_empty())) {
        Ok(read) => read,
        Err(e) => return fail(format!("Nothing was recorded: {e}. Scan the code again.")),
    };
    if kind != Kind::Handoff {
        return fail(
            "That is a receipt from the server, for a tablet to scan. Scan the code on the scout's tablet."
                .into(),
        );
    }
    let handoff: Handoff = match serde_json::from_slice(&bytes) {
        Ok(handoff) => handoff,
        Err(e) => {
            return fail(format!(
                "Nothing was recorded: those codes are not a scout's forms ({e})."
            ));
        }
    };
    if handoff.forms.len() > MAX_PER_PUSH {
        return fail(format!(
            "Nothing was recorded: at most {MAX_PER_PUSH} forms at a time."
        ));
    }
    let claims = match crate::token::verify(state, &handoff.token).await {
        Ok(claims) => claims,
        Err(e) => {
            return fail(format!(
                "Nothing was recorded: the tablet's sign-in could not be used ({e}). The scout signs in on it again once it reaches the server."
            ));
        }
    };
    // Roles and team as they are now, as for a push with a token.
    let user = match state.repo.user_by_id(claims.user_id).await {
        Ok(Some(user)) => user,
        Ok(None) => {
            return fail("Nothing was recorded: the account that saved them is gone.".into());
        }
        Err(e) => {
            warn!("loading a handoff's scout: {e}");
            return fail("Nothing was recorded: storage is unavailable. Try again.".into());
        }
    };
    let device = state
        .repo
        .device_by_uuid(&claims.device)
        .await
        .unwrap_or_else(|e| {
            warn!("looking up a handoff's tablet: {e}");
            None
        });

    let now = Utc::now();
    let mut rows = Vec::with_capacity(handoff.forms.len());
    let mut receipts = Vec::with_capacity(handoff.forms.len());
    for form in &handoff.forms {
        let queued = queued(state, form, now);
        let heading = heading(state, &queued).await;
        let client_record_id = queued.client_record_id.clone();
        let outcome = match crate::push::record(state, &user, device.as_ref(), queued, now).await {
            Ok(outcome) => outcome,
            Err(e) => {
                // Those before it stand; the tablet still has them all, and
                // a second scan records each once.
                warn!("recording a handed-over form: {e}");
                return fail(
                    "Storage is unavailable, so not every form was recorded. Scan the code again."
                        .into(),
                );
            }
        };
        rows.push(HandoffRow {
            heading,
            recorded: outcome == Outcome::Recorded,
            reason: match &outcome {
                Outcome::Recorded => String::new(),
                Outcome::Refused { reason } => reason.clone(),
            },
        });
        receipts.push(Receipt {
            client_record_id,
            outcome,
        });
    }

    let recorded = rows.iter().filter(|r| r.recorded).count();
    info!(
        lead = %lead.email,
        scout = %user.email,
        device = %claims.device,
        sent = rows.len(),
        recorded,
        "forms handed over by QR"
    );
    let receipt = receipt_frames(&HandoffReceipts { receipts });
    let forms = |n: usize| if n == 1 { "form" } else { "forms" };
    let heading = if rows.is_empty() {
        format!("{}'s tablet sent no forms", user.name)
    } else {
        format!(
            "Recorded {recorded} of {} {} from {}'s tablet",
            rows.len(),
            forms(rows.len()),
            user.name
        )
    };
    Ok(HandoffResult {
        heading,
        all_recorded: recorded == rows.len(),
        scout: user.name.clone(),
        rows,
        refused_href: format!("/lead-scout{query}#refused"),
        receipt,
        again_href: format!("/lead-scout/scan{query}"),
    })
}

/// A form as the push takes it. Its answers are read by the form it was
/// typed into when that is this one, else guessed (`read_answers_as_given`).
fn queued(state: &AppState, form: &HandedForm, now: DateTime<Utc>) -> QueuedObservation {
    let raw = RawAnswers::from_pairs(&form.pairs());
    let schema = (form.form_version == state.season.version).then_some(&*state.season);
    QueuedObservation {
        client_record_id: form.record_id.clone(),
        match_key: form.match_key.clone(),
        // Not a number: no team is 0, so the push refuses it, and a lead
        // can put it right.
        team_number: form.team.trim().parse().unwrap_or(0),
        payload: read_answers_as_given(schema, &raw),
        schema_version: form.form_version,
        observed_at: DateTime::from_timestamp_millis(form.saved_at).unwrap_or(now),
    }
}

/// `"Q14 · Team 254"`, or the match key as sent.
async fn heading(state: &AppState, queued: &QueuedObservation) -> String {
    let label = match state.repo.match_by_key(&queued.match_key).await {
        Ok(Some(record)) => record.label(),
        _ => queued.match_key.clone(),
    };
    format!("{label} · Team {}", queued.team_number)
}

/// The receipt as codes. A handoff is at most [`MAX_PER_PUSH`] forms, so
/// its receipt is a few frames at most and always fits.
fn receipt_frames(receipts: &HandoffReceipts) -> Vec<QrFrame> {
    let json = serde_json::to_vec(receipts).expect("receipts serialize");
    let frames = qr::frames(Kind::Receipt, &json).expect("a receipt fits");
    let count = frames.len();
    frames
        .iter()
        .enumerate()
        .map(|(i, text)| {
            let symbol = qr::symbol(text).expect("a frame fits one code");
            QrFrame {
                size: symbol.size,
                path: symbol.path,
                label: format!("Part {} of {count}", i + 1),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::{Request, header};
    use chrono::TimeDelta;
    use tower::ServiceExt;
    use tt_core::matches::CompLevel;
    use tt_core::records::{Event, MatchRecord};
    use tt_core::season::Value;
    use tt_repo_sqlite::SqliteRepo;

    use super::*;
    use crate::startup::router;

    const KIMS_TABLET: &str = "tt_device=0191f7ac-1234-7000-8000-00000000c1a0";
    const GOOD: &str = "0191f7ac-1234-7000-8000-0000000000a1";
    const WRONG: &str = "0191f7ac-1234-7000-8000-0000000000a2";

    async fn state() -> AppState {
        let repo = SqliteRepo::connect("sqlite::memory:").unwrap();
        tt_repo_sqlite::migrate::apply(repo.pool()).await.unwrap();
        let state = AppState {
            repo: Arc::new(repo),
            season: Arc::new(tt_core::season::current_season().unwrap()),
            upstream: Arc::new(crate::upstream::Upstream::disabled()),
            snapshots: Default::default(),
            tokens: Default::default(),
        };
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
        state
    }

    async fn send(state: &AppState, request: Request<Body>) -> (u16, String) {
        let response = router(state.clone()).oneshot(request).await.unwrap();
        let status = response.status().as_u16();
        let set_cookie = response
            .headers()
            .get(header::SET_COOKIE)
            .map(|v| v.to_str().unwrap().to_string());
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body = String::from_utf8_lossy(&bytes).into_owned();
        (status, set_cookie.unwrap_or(body))
    }

    /// A session cookie for a new account; the first is an admin.
    async fn sign_up(state: &AppState, name: &str) -> String {
        let form = format!(
            "name={name}&email={name}%40example.com&team_number=10101\
             &password=longenough1&confirm_password=longenough1"
        );
        let request = Request::post("/api/auth/signup")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(form))
            .unwrap();
        let (_, cookie) = send(state, request).await;
        cookie.split(';').next().unwrap().to_string()
    }

    async fn post_frames(state: &AppState, cookie: &str, frames: &[String]) -> (u16, String) {
        let body: String = form_urlencoded::Serializer::new(String::new())
            .append_pair("frames", &frames.join("\n"))
            .finish();
        let request = Request::post("/api/handoff?event=2026now")
            .header(header::COOKIE, cookie)
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .unwrap();
        send(state, request).await
    }

    fn form(record_id: &str, team: &str, saved_at: DateTime<Utc>) -> serde_json::Value {
        serde_json::json!({
            "record_id": record_id,
            "match": "2026now_qm2",
            "team": team,
            "form_version": 1,
            "saved_at": saved_at.timestamp_millis(),
            "answers": {
                "f.starting_position": "center",
                "f.teleop_scored": "9",
                "f.broke_down": true,
                "f.notes": "tippy on the ramp",
            },
        })
    }

    #[tokio::test]
    async fn a_lead_scans_a_tablets_forms_in_as_the_scout_who_saved_them() {
        let state = state().await;
        let sam = sign_up(&state, "Sam").await;
        let kim = sign_up(&state, "Kim").await;
        let kims_tablet = format!("{kim}; {KIMS_TABLET}");
        // Kim's tablet is known to the Pi, and was given a token while it
        // could reach it.
        let heartbeat = Request::post("/api/device/heartbeat")
            .header(header::COOKIE, &kims_tablet)
            .body(Body::empty())
            .unwrap();
        assert_eq!(send(&state, heartbeat).await.0, 200);
        let issue = Request::post("/api/auth/token")
            .header(header::COOKIE, &kims_tablet)
            .body(Body::empty())
            .unwrap();
        let (status, issued) = send(&state, issue).await;
        assert_eq!(status, 200, "{issued}");
        let token = serde_json::from_str::<serde_json::Value>(&issued).unwrap()["token"]
            .as_str()
            .unwrap()
            .to_string();

        // Two forms saved with no server: one good, one of a robot that is
        // not in the match.
        let watched = Utc::now() - TimeDelta::minutes(20);
        let handoff = serde_json::json!({
            "token": token,
            "forms": [form(GOOD, "254", watched), form(WRONG, "9999", watched)],
        });
        let frames = qr::frames(Kind::Handoff, &serde_json::to_vec(&handoff).unwrap()).unwrap();

        // The scanner is a lead's.
        let (status, page) = send(
            &state,
            Request::get("/lead-scout/scan?event=2026now")
                .header(header::COOKIE, &sam)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, 200);
        assert!(page.contains("data-scanner"), "{page}");
        assert!(
            page.contains(r#"action="/api/handoff?event=2026now""#),
            "{page}"
        );
        let (status, _) = post_frames(&state, &kim, &frames).await;
        assert_eq!(status, 303, "a scout cannot scan forms in");

        let (status, page) = post_frames(&state, &sam, &frames).await;
        assert_eq!(status, 200);
        assert!(
            page.contains("Recorded 1 of 2 forms from Kim&#39;s tablet"),
            "{page}"
        );
        assert!(page.contains("Q2 · Team 254"), "{page}");
        assert!(
            page.contains("Not saved: team 9999 is not in 2026now_qm2."),
            "{page}"
        );
        assert!(page.contains("Show this to Kim"), "{page}");
        assert!(page.contains(r#"<svg class="qr""#), "the receipt is a code");

        // Recorded as Kim, from Kim's tablet, when Kim watched.
        let pending = state.repo.pending_observations("2026now").await.unwrap();
        assert_eq!(pending.len(), 1);
        let saved = &pending[0];
        assert_eq!(saved.team_number, 254);
        assert_eq!(saved.scouter_name.as_deref(), Some("Kim"));
        assert_eq!(saved.submitting_team, Some(10101));
        assert_eq!(
            saved.observed_at.map(|t| t.timestamp_millis()),
            Some(watched.timestamp_millis())
        );
        assert_eq!(saved.payload["teleop_scored"], Value::Count(9));
        assert_eq!(saved.payload["broke_down"], Value::Flag(true));
        assert_eq!(saved.payload["no_show"], Value::Flag(false));
        // The refusal waits for a lead, as a push's does (C10).
        let open = state.repo.open_refusals().await.unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].entry.client_record_id, WRONG);
        assert_eq!(open[0].scouter_name.as_deref(), Some("Kim"));
        assert!(open[0].device_name.is_some(), "from Kim's tablet");

        // Scanned twice: nothing twice.
        let (_, page) = post_frames(&state, &sam, &frames).await;
        assert!(page.contains("Recorded 1 of 2 forms"), "{page}");
        assert_eq!(
            state
                .repo
                .pending_observations("2026now")
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(state.repo.open_refusals().await.unwrap().len(), 1);

        // Some parts missing, and nothing recorded.
        let mut more = handoff.clone();
        more["forms"] = serde_json::Value::Array(
            (0..12)
                .map(|i| {
                    form(
                        &format!("0191f7ac-1234-7000-8000-0000000001{i:02}"),
                        "254",
                        watched,
                    )
                })
                .collect(),
        );
        let long = qr::frames(Kind::Handoff, &serde_json::to_vec(&more).unwrap()).unwrap();
        assert!(long.len() > 1);
        let (_, page) = post_frames(&state, &sam, &long[1..]).await;
        assert!(page.contains("Nothing was recorded: 1 of"), "{page}");
        assert!(page.contains("data-scanner"), "back to the scanner");
    }

    #[tokio::test]
    async fn codes_that_are_not_a_scouts_forms_record_nothing() {
        let state = state().await;
        let sam = sign_up(&state, "Sam").await;
        let cases = [
            (
                vec!["https://example.com".to_string()],
                "not a TealTeam code",
            ),
            (
                qr::frames(Kind::Receipt, br#"{"receipts":[]}"#).unwrap(),
                "That is a receipt from the server",
            ),
            (
                qr::frames(Kind::Handoff, b"{}").unwrap(),
                "not a scout&#39;s forms",
            ),
            (
                qr::frames(Kind::Handoff, br#"{"token":"v4.public.nope","forms":[]}"#).unwrap(),
                "the tablet&#39;s sign-in could not be used",
            ),
        ];
        for (frames, says) in cases {
            let (status, page) = post_frames(&state, &sam, &frames).await;
            assert_eq!(status, 200);
            assert!(page.contains(says), "{says}: {page}");
            assert!(page.contains("data-scanner"));
        }
        assert!(state.repo.open_refusals().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn the_scouting_page_and_the_offline_shell_carry_the_handoff_panel() {
        let state = state().await;
        let sam = sign_up(&state, "Sam").await;
        let (_, page) = send(
            &state,
            Request::get("/submission?event=2026now&match=2026now_qm2&team=254")
                .header(header::COOKIE, &sam)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        let (_, shell) = send(
            &state,
            Request::get("/offline").body(Body::empty()).unwrap(),
        )
        .await;
        for page in [page, shell] {
            assert!(
                page.contains(r#"<details class="card handoff" id="handoff" data-handoff hidden>"#),
                "{page}"
            );
            for script in [
                "vendor/qrcode-generator/qrcode.js",
                "js/qr.js",
                "js/scan.js",
                "js/handoff.js",
            ] {
                assert!(
                    page.contains(&format!(r#"src="/static/{script}""#)),
                    "{script}"
                );
            }
        }
    }
}
