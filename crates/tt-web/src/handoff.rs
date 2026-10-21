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
//! The codes carry no token (T1). They name the scout's token's holder
//! (`tt_core::token::Holder`) and carry a MAC over the forms, made with the
//! device key only that tablet was given. The Pi derives the key again and
//! checks the MAC, so only that tablet could have made these codes, for
//! these forms. A photo of them is good for those forms alone, which record
//! once anyway, within `qr::HANDOFF_WINDOW` of being shown, and nothing
//! else: no sync takes a holder. The receipt comes back with a MAC by the
//! same key and the codes' nonce, so the tablet deletes only what the Pi
//! answered for those codes. Each scan that records is a `handoff_scans`
//! row naming the lead.

use chrono::{DateTime, Utc};
use tracing::{info, warn};
use tt_core::form::{RawAnswers, read_answers_as_given};
use tt_core::outbox::{MAX_PER_PUSH, Outcome, QueuedObservation, Receipt};
use tt_core::qr::{
    self, FormsFile, HandedForm, Handoff, HandoffBody, HandoffReceipts, Kind, ReceiptBody,
};
use tt_core::token::{self, Purpose};
use tt_core::user::User;
use tt_repo::Repo;
use tt_repo_sqlite::token_key::HandoffScan;
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
    let what = Carried {
        are: "those codes are",
        were: "those codes were",
        purpose: Purpose::Handoff,
        windowed: true,
    };
    take(state, lead, &handoff.body, &handoff.mac, what, query).await
}

/// Record what a scout's saved file carries (C15): the forms the
/// downloadable scouting page could not deliver, as `static/js/scout-file.js`
/// saves them. The same checks as codes, but by [`Purpose::File`], and with
/// no [`qr::HANDOFF_WINDOW`]: the file lasts as long as the token it names.
pub async fn receive_file(
    state: &AppState,
    lead: &User,
    text: &str,
    query: &str,
) -> Result<HandoffResult, Vec<String>> {
    let file: FormsFile = match serde_json::from_str::<FormsFile>(text) {
        Ok(file) if file.format == qr::FORMS_FILE && file.version == 1 => file,
        Ok(_) => {
            return Err(vec![
                "Nothing was recorded: that file is not a scout's saved forms.".into(),
            ]);
        }
        Err(e) => {
            return Err(vec![format!(
                "Nothing was recorded: that file is not a scout's saved forms ({e})."
            )]);
        }
    };
    let what = Carried {
        are: "that file is",
        were: "that file was",
        purpose: Purpose::File,
        windowed: false,
    };
    take(state, lead, &file.body, &file.mac, what, query).await
}

/// How the forms came: codes, or a file.
struct Carried {
    /// For the lead: `"those codes are"`, `"those codes were"`.
    are: &'static str,
    were: &'static str,
    purpose: Purpose,
    /// Codes must be fresh; a file need not be.
    windowed: bool,
}

/// Check a body and its MAC as `what` says, and record its forms.
async fn take(
    state: &AppState,
    lead: &User,
    signed_body: &str,
    mac: &str,
    what: Carried,
    query: &str,
) -> Result<HandoffResult, Vec<String>> {
    let fail = |why: String| Err(vec![why]);
    // Read before it is checked, for who to check it as; nothing in it is
    // believed until the MAC holds.
    let body: HandoffBody = match serde_json::from_str(signed_body) {
        Ok(body) => body,
        Err(e) => {
            return fail(format!(
                "Nothing was recorded: {} not a scout's forms ({e}).",
                what.are
            ));
        }
    };
    if body.forms.len() > MAX_PER_PUSH {
        return fail(format!(
            "Nothing was recorded: at most {MAX_PER_PUSH} forms at a time."
        ));
    }
    let signed_in_again = "The scout signs in on it again once it reaches the server.";
    let (user, key) = match crate::token::holder_key(state, &body.scout).await {
        Ok(found) => found,
        Err(e) => {
            return fail(format!(
                "Nothing was recorded: the tablet's sign-in could not be used ({e}). {signed_in_again}"
            ));
        }
    };
    if !token::mac_holds(&key, what.purpose, signed_body, mac) {
        info!(lead = %lead.email, scout = %user.email, "refused a handoff whose MAC does not hold");
        return fail(format!(
            "Nothing was recorded: {} not made by {}'s tablet, or changed since. {signed_in_again}",
            what.were, user.name
        ));
    }
    let device = state
        .repo
        .device_by_uuid(&body.scout.device)
        .await
        .unwrap_or_else(|e| {
            warn!("looking up a handoff's tablet: {e}");
            None
        });

    let now = Utc::now();
    // When the codes were made, on the Pi's clock as near as the tablet's
    // last measurement (S12) says.
    let offset = device.as_ref().and_then(|d| d.clock_offset_ms).unwrap_or(0);
    let made = DateTime::from_timestamp_millis(body.at.saturating_add(offset));
    if what.windowed && made.is_none_or(|made| (now - made).abs() > qr::HANDOFF_WINDOW) {
        return fail(format!(
            "Nothing was recorded: those codes are more than {} minutes old, or the tablet's clock is off. \
             Have the scout press Hide and show the codes again.",
            qr::HANDOFF_WINDOW.num_minutes()
        ));
    }

    let now = Utc::now();
    let mut rows = Vec::with_capacity(body.forms.len());
    let mut receipts = Vec::with_capacity(body.forms.len());
    for form in &body.forms {
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
        device = %body.scout.device,
        sent = rows.len(),
        recorded,
        by = if what.windowed { "QR" } else { "file" },
        "forms handed over"
    );
    let scan = HandoffScan {
        at: now,
        lead_id: lead.id,
        scout_id: user.id,
        device_uuid: &body.scout.device,
        nonce: &body.nonce,
        sent: rows.len(),
        recorded,
    };
    if let Err(e) = state.repo.record_handoff_scan(&scan).await {
        // The forms stand; the log line above still names the lead.
        warn!("recording a handoff scan: {e}");
    }
    // The forms stand either way; a second scan of fewer records none twice.
    let receipt = ReceiptBody {
        nonce: body.nonce.clone(),
        receipts,
    };
    let Some(receipt) = receipt_frames(&key, &receipt) else {
        return fail(format!(
            "The forms were recorded, but their receipt is too big to show as codes. \
             Have the scout send fewer at a time, and scan again for each receipt ({} sent).",
            rows.len()
        ));
    };
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

/// The receipt with its MAC by the scout's device key, as the tablet
/// checks it before deleting anything.
fn signed_receipt(key: &[u8; 32], receipt: &ReceiptBody) -> HandoffReceipts {
    let body = serde_json::to_string(receipt).expect("a receipt serializes");
    HandoffReceipts {
        mac: token::mac(key, Purpose::Receipt, &body),
        body,
    }
}

/// The receipt as codes, with its MAC by the scout's device key. A handoff
/// is at most [`MAX_PER_PUSH`] forms, so an honest receipt is a few frames
/// at most. But each one echoes the record id and, in a refusal's reason,
/// the match key as the tablet sent them, so codes made to fill
/// [`qr::MAX_FRAMES`] can come back as more than fit: `None` then, never a
/// panic.
fn receipt_frames(key: &[u8; 32], receipt: &ReceiptBody) -> Option<Vec<QrFrame>> {
    let json = serde_json::to_vec(&signed_receipt(key, receipt)).ok()?;
    let frames = qr::frames(Kind::Receipt, &json).ok()?;
    let count = frames.len();
    frames
        .iter()
        .enumerate()
        .map(|(i, text)| {
            let symbol = qr::symbol(text).ok()?;
            Some(QrFrame {
                size: symbol.size,
                path: symbol.path,
                label: format!("Part {} of {count}", i + 1),
            })
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
            red: [Some(10101), Some(254)],
            blue: [Some(2), Some(3)],
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
                "f.robot_status": "played",
                "f.launched": "9",
                "f.teleop_park": true,
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
        let issued: serde_json::Value = serde_json::from_str(&issued).unwrap();

        // Two forms saved with no server: one good, one of a robot that is
        // not in the match.
        let watched = Utc::now() - TimeDelta::minutes(20);
        let forms = vec![form(GOOD, "254", watched), form(WRONG, "9999", watched)];
        let (handoff, _) = signed(&issued, forms.clone(), Utc::now());
        let frames = qr::frames(Kind::Handoff, &serde_json::to_vec(&handoff).unwrap()).unwrap();
        // The codes carry no token, and nothing the sync would take.
        let shown =
            String::from_utf8(qr::assemble(frames.iter().map(String::as_str)).unwrap().1).unwrap();
        assert!(!shown.contains("v4.public."), "{shown}");
        assert!(!shown.contains(issued["key"].as_str().unwrap()), "{shown}");

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
        // A lead's scan is on the record (T1).
        let scans = state.repo.handoff_scans(10).await.unwrap();
        assert_eq!(scans.len(), 1);
        assert_eq!(scans[0].lead.as_deref(), Some("Sam"));
        assert_eq!(scans[0].scout.as_deref(), Some("Kim"));
        assert_eq!((scans[0].sent, scans[0].recorded), (2, 1));

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
        assert_eq!(saved.payload["launched"], Value::Count(9));
        assert_eq!(saved.payload["teleop_park"], Value::Flag(true));
        assert_eq!(saved.payload["auto_park"], Value::Flag(false));
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

        let sent: HandoffBody = serde_json::from_str(&handoff.body).unwrap();

        // Forms swapped under the tablet's MAC, or a MAC by another key:
        // nothing recorded.
        let mut swapped = sent.clone();
        swapped.forms[0].team = "1".into();
        let swapped = Handoff {
            body: serde_json::to_string(&swapped).unwrap(),
            mac: handoff.mac.clone(),
        };
        let forged = Handoff {
            mac: token::mac(&[3; 32], Purpose::Handoff, &handoff.body),
            ..handoff.clone()
        };
        for bad in [swapped, forged] {
            let codes = qr::frames(Kind::Handoff, &serde_json::to_vec(&bad).unwrap()).unwrap();
            let (_, page) = post_frames(&state, &sam, &codes).await;
            assert!(
                page.contains("were not made by Kim&#39;s tablet, or changed since"),
                "{page}"
            );
        }
        // Codes made an hour ago: too old.
        let (stale, _) = signed(&issued, forms.clone(), Utc::now() - TimeDelta::hours(1));
        let codes = qr::frames(Kind::Handoff, &serde_json::to_vec(&stale).unwrap()).unwrap();
        let (_, page) = post_frames(&state, &sam, &codes).await;
        assert!(page.contains("more than 20 minutes old"), "{page}");

        // Some parts missing, and nothing recorded.
        let more = (0..40)
            .map(|i| {
                form(
                    &format!("0191f7ac-1234-7000-8000-0000000001{i:02}"),
                    "254",
                    watched,
                )
            })
            .collect();
        let (more, _) = signed(&issued, more, Utc::now());
        let long = qr::frames(Kind::Handoff, &serde_json::to_vec(&more).unwrap()).unwrap();
        assert!(long.len() > 1);
        let (_, page) = post_frames(&state, &sam, &long[1..]).await;
        assert!(page.contains("Nothing was recorded: 1 of"), "{page}");
        assert!(page.contains("data-scanner"), "back to the scanner");
        assert_eq!(state.repo.handoff_scans(10).await.unwrap().len(), 2);

        // Kim's tablet is lost: a lead signs her out everywhere, and its
        // codes are refused from then on.
        let holder: HandoffBody = serde_json::from_str(&handoff.body).unwrap();
        let out = Request::post(format!(
            "/api/users/{}/sign-out?event=2026now",
            holder.scout.user_id
        ))
        .header(header::COOKIE, &kim)
        .body(Body::empty())
        .unwrap();
        assert_eq!(send(&state, out).await.0, 303, "a scout cannot");
        assert_eq!(
            state.repo.token_epoch(holder.scout.user_id).await.unwrap(),
            0
        );
        let out = Request::post(format!(
            "/api/users/{}/sign-out?event=2026now",
            holder.scout.user_id
        ))
        .header(header::COOKIE, &sam)
        .body(Body::empty())
        .unwrap();
        let (status, _) = send(&state, out).await;
        assert_eq!(status, 303);
        assert_eq!(
            state.repo.token_epoch(holder.scout.user_id).await.unwrap(),
            1
        );
        let (fresh, _) = signed(&issued, forms, Utc::now());
        let codes = qr::frames(Kind::Handoff, &serde_json::to_vec(&fresh).unwrap()).unwrap();
        let (_, page) = post_frames(&state, &sam, &codes).await;
        assert!(page.contains("signed out everywhere since"), "{page}");
        // And her session with it.
        let (status, _) = send(
            &state,
            Request::get("/account")
                .header(header::COOKIE, &kim)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, 303, "Kim signs in again");
    }

    /// A handoff of `forms` as Kim's tablet makes it at `at`, and her key.
    fn signed(
        issued: &serde_json::Value,
        forms: Vec<serde_json::Value>,
        at: DateTime<Utc>,
    ) -> (Handoff, [u8; 32]) {
        use base64::Engine;
        let token = issued["token"].as_str().unwrap();
        let key: [u8; 32] = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(issued["key"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap();
        let claims = token::read_unverified(token).unwrap();
        let body = serde_json::json!({
            "scout": token::Holder::of(&claims),
            "nonce": format!("nonce-{}", at.timestamp_millis()),
            "at": at.timestamp_millis(),
            "forms": forms,
        })
        .to_string();
        let mac = token::mac(&key, Purpose::Handoff, &body);
        (Handoff { body, mac }, key)
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
                qr::frames(Kind::Receipt, br#"{"body":"{}","mac":""}"#).unwrap(),
                "That is a receipt from the server",
            ),
            (
                qr::frames(Kind::Handoff, b"{}").unwrap(),
                "not a scout&#39;s forms",
            ),
            (
                // As made before T1: a token in the codes.
                qr::frames(Kind::Handoff, br#"{"token":"v4.public.nope","forms":[]}"#).unwrap(),
                "not a scout&#39;s forms",
            ),
            (
                qr::frames(
                    Kind::Handoff,
                    br#"{"body":"{\"scout\":{\"sub\":\"99\",\"dev\":\"x\",\"ep\":0,\"iat\":\"2099-01-01T00:00:00Z\"},\"nonce\":\"n\",\"at\":0,\"forms\":[]}","mac":"m"}"#,
                )
                .unwrap(),
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

    #[test]
    fn a_receipt_too_big_for_codes_is_none_not_a_panic() {
        // Incompressible text, as a handoff made to fill its codes echoes.
        let mut seed: u64 = 0x2545_f491_4f6c_dd1d;
        let mut noise = |n: usize| -> String {
            (0..n)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    char::from(b'!' + (seed % 90) as u8)
                })
                .collect()
        };
        let receipts = (0..MAX_PER_PUSH)
            .map(|_| Receipt {
                client_record_id: noise(800),
                outcome: Outcome::Refused { reason: noise(800) },
            })
            .collect();
        let key = [5; 32];
        let nonce = "n".to_string();
        assert!(
            receipt_frames(
                &key,
                &ReceiptBody {
                    nonce: nonce.clone(),
                    receipts
                }
            )
            .is_none()
        );

        let small = ReceiptBody {
            nonce,
            receipts: vec![Receipt {
                client_record_id: GOOD.into(),
                outcome: Outcome::Recorded,
            }],
        };
        assert_eq!(receipt_frames(&key, &small).map(|f| f.len()), Some(1));

        // The tablet's check: by its key, over the body as sent.
        let signed = signed_receipt(&key, &small);
        assert!(token::mac_holds(
            &key,
            Purpose::Receipt,
            &signed.body,
            &signed.mac
        ));
        assert!(!token::mac_holds(
            &[6; 32],
            Purpose::Receipt,
            &signed.body,
            &signed.mac
        ));
        assert!(!token::mac_holds(
            &key,
            Purpose::Handoff,
            &signed.body,
            &signed.mac
        ));
        let said: ReceiptBody = serde_json::from_str(&signed.body).unwrap();
        assert_eq!(said, small);
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
                "js/hmac.js",
                "js/handoff.js",
            ] {
                assert!(
                    page.contains(&format!(r#"src="/static/{script}""#)),
                    "{script}"
                );
            }
        }
    }

    /// Kim's tablet, known to the Pi, and the token and key it was given.
    async fn kims_tablet(state: &AppState) -> (String, serde_json::Value) {
        let kim = sign_up(state, "Kim").await;
        let kims_tablet = format!("{kim}; {KIMS_TABLET}");
        let heartbeat = Request::post("/api/device/heartbeat")
            .header(header::COOKIE, &kims_tablet)
            .body(Body::empty())
            .unwrap();
        assert_eq!(send(state, heartbeat).await.0, 200);
        let issue = Request::post("/api/auth/token")
            .header(header::COOKIE, &kims_tablet)
            .body(Body::empty())
            .unwrap();
        let (status, issued) = send(state, issue).await;
        assert_eq!(status, 200, "{issued}");
        (kims_tablet, serde_json::from_str(&issued).unwrap())
    }

    #[tokio::test]
    async fn a_scout_downloads_a_scouting_page_that_needs_no_server() {
        let state = state().await;
        let _sam = sign_up(&state, "Sam").await;
        let (kims_tablet, _) = kims_tablet(&state).await;

        let request = Request::get("/submission/download?event=2026now")
            .header(header::COOKIE, &kims_tablet)
            .header(header::HOST, "pi.lan:8080")
            .header(header::ACCEPT, "text/html")
            .body(Body::empty())
            .unwrap();
        let response = router(state.clone()).oneshot(request).await.unwrap();
        assert_eq!(response.status(), 200);
        let saved_as = response.headers()[header::CONTENT_DISPOSITION]
            .to_str()
            .unwrap()
            .to_string();
        assert_eq!(
            saved_as,
            r#"attachment; filename="tealteam-scouting-2026now-kim.html""#
        );
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let page = String::from_utf8(bytes.to_vec()).unwrap();

        // Everything is inside it: it loads nothing from anywhere.
        for outside in ["<script src", "<link", "@import", r#"href="/"#] {
            assert!(!page.contains(outside), "{outside}");
        }
        assert_eq!(
            page.matches("<script").count(),
            page.matches("</script>").count(),
            "no script ends early"
        );
        assert!(page.contains(".scout-form"), "the stylesheet");
        assert!(page.contains("window.ttToken = {"), "scout-file.js");
        assert!(page.contains("window.ttHmac"), "hmac.js");
        assert!(page.contains(r#"name="f.launched""#), "the form");
        assert!(page.contains("data-handoff"), "the QR handoff");
        // Plain http, as at an event: nothing that needs a secure context.
        assert!(!page.contains("serviceWorker.register"));
        assert!(!page.contains("navigator.storage.getDirectory"));

        let data = page
            .split(r#"id="tt-file-data">"#)
            .nth(1)
            .and_then(|rest| rest.split("</script>").next())
            .unwrap();
        let data: serde_json::Value = serde_json::from_str(data).unwrap();
        assert_eq!(data["pi"], "http://pi.lan:8080");
        assert_eq!(data["scout"], "Kim");
        assert_eq!(data["event"], "2026now");
        assert_eq!(data["matches"][0]["key"], "2026now_qm2");
        assert_eq!(data["matches"][0]["red"][1], 254);
        let credential = format!(
            "Bearer {}~{}",
            data["token"].as_str().unwrap(),
            data["key"].as_str().unwrap()
        );

        // Opened from a file its origin is null, and it sends no cookie:
        // the browser asks first, then posts with the credential.
        let ask = Request::options("/api/sync/push")
            .header(header::ORIGIN, "null")
            .header("access-control-request-method", "POST")
            .header(
                "access-control-request-headers",
                "authorization, content-type",
            )
            .header("access-control-request-private-network", "true")
            .body(Body::empty())
            .unwrap();
        let answer = router(state.clone()).oneshot(ask).await.unwrap();
        assert_eq!(answer.status(), 204);
        let allowed = |name: &str| answer.headers()[name].to_str().unwrap().to_string();
        assert_eq!(allowed("access-control-allow-origin"), "*");
        assert!(allowed("access-control-allow-headers").contains("authorization"));
        assert_eq!(allowed("access-control-allow-private-network"), "true");
        assert!(
            !answer
                .headers()
                .contains_key("access-control-allow-credentials")
        );

        let entry = |id: &str, team: i32| {
            serde_json::json!({
                "client_record_id": id,
                "match_key": "2026now_qm2",
                "team_number": team,
                "payload": {"robot_status": "played", "launched": 9},
                "schema_version": state.season.version,
                "observed_at": Utc::now() - TimeDelta::minutes(30),
            })
        };
        let push = serde_json::json!({ "version": 2, "observations": [entry(GOOD, 254), entry(WRONG, 9999)] });
        let request = Request::post("/api/sync/push")
            .header(header::ORIGIN, "null")
            .header(header::AUTHORIZATION, &credential)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(push.to_string()))
            .unwrap();
        let response = router(state.clone()).oneshot(request).await.unwrap();
        assert_eq!(response.status(), 200, "{push}");
        assert_eq!(response.headers()["access-control-allow-origin"], "*");
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let reply: tt_core::outbox::PushReply = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(reply.receipts[0].outcome, Outcome::Recorded);
        assert!(matches!(reply.receipts[1].outcome, Outcome::Refused { .. }));
        let pending = state.repo.pending_observations("2026now").await.unwrap();
        assert_eq!(pending[0].scouter_name.as_deref(), Some("Kim"));
        let open = state.repo.open_refusals().await.unwrap();
        assert!(
            open[0].device_name.is_some(),
            "from the tablet the token names"
        );

        // A refusal of the whole push is readable from the file too.
        let request = Request::post("/api/sync/push")
            .header(header::ORIGIN, "null")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(push.to_string()))
            .unwrap();
        let response = router(state.clone()).oneshot(request).await.unwrap();
        assert_eq!(response.status(), 401);
        assert_eq!(response.headers()["access-control-allow-origin"], "*");

        // The scouting page offers it; with no device id there is no file.
        let (_, scout) = send(
            &state,
            Request::get("/submission?event=2026now")
                .header(header::COOKIE, &kims_tablet)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert!(
            scout.contains(r#"href="/submission/download?event=2026now""#),
            "{scout}"
        );
        let kim = kims_tablet.split("; ").next().unwrap();
        let request = Request::get("/submission/download?event=2026now")
            .header(header::COOKIE, kim)
            .body(Body::empty())
            .unwrap();
        assert_eq!(send(&state, request).await.0, 400);
    }

    async fn post_file(state: &AppState, cookie: &str, file: &str) -> (u16, String) {
        let body: String = form_urlencoded::Serializer::new(String::new())
            .append_pair("file", file)
            .finish();
        let request = Request::post("/api/handoff?event=2026now")
            .header(header::COOKIE, cookie)
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .unwrap();
        send(state, request).await
    }

    #[tokio::test]
    async fn a_lead_uploads_the_forms_a_scout_saved_as_a_file() {
        let state = state().await;
        let sam = sign_up(&state, "Sam").await;
        let (_, issued) = kims_tablet(&state).await;
        // Saved hours ago, far outside the codes' window: a file need not
        // be fresh.
        let then = Utc::now() - TimeDelta::hours(3);
        let (codes, key) = signed(&issued, vec![form(GOOD, "254", then)], then);
        let file = |mac: &str| {
            serde_json::json!({
                "format": "tealteam-forms",
                "version": 1,
                "body": codes.body,
                "mac": mac,
            })
            .to_string()
        };
        let file_mac = token::mac(&key, Purpose::File, &codes.body);

        // A MAC made for codes does not pass as a file's, nor a file's as
        // codes': the window is the codes' only.
        let (status, page) = post_file(&state, &sam, &file(&codes.mac)).await;
        assert_eq!(status, 200);
        assert!(
            page.contains("that file was not made by Kim&#39;s tablet"),
            "{page}"
        );
        let as_codes = Handoff {
            body: codes.body.clone(),
            mac: file_mac.clone(),
        };
        let frames = qr::frames(Kind::Handoff, &serde_json::to_vec(&as_codes).unwrap()).unwrap();
        let (_, page) = post_frames(&state, &sam, &frames).await;
        assert!(
            page.contains("those codes were not made by Kim&#39;s tablet"),
            "{page}"
        );
        let (_, page) = post_file(&state, &sam, "{}").await;
        assert!(
            page.contains("that file is not a scout&#39;s saved forms"),
            "{page}"
        );
        assert!(
            state
                .repo
                .pending_observations("2026now")
                .await
                .unwrap()
                .is_empty()
        );

        let (status, page) = post_file(&state, &sam, &file(&file_mac)).await;
        assert_eq!(status, 200);
        assert!(
            page.contains("Recorded 1 of 1 form from Kim&#39;s tablet"),
            "{page}"
        );
        assert!(
            page.contains("Show this to Kim"),
            "with the receipt to scan"
        );
        let pending = state.repo.pending_observations("2026now").await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].scouter_name.as_deref(), Some("Kim"));
        // Uploaded twice, recorded once.
        let (_, page) = post_file(&state, &sam, &file(&file_mac)).await;
        assert!(page.contains("Recorded 1 of 1 form"), "{page}");
        assert_eq!(
            state
                .repo
                .pending_observations("2026now")
                .await
                .unwrap()
                .len(),
            1
        );

        // A scout cannot upload one.
        let kim = sign_up(&state, "Lee").await;
        assert_eq!(post_file(&state, &kim, &file(&file_mac)).await.0, 303);
    }
}
