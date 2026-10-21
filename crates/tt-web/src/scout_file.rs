//! The downloadable scouting page (C15): `GET /submission/download`.
//!
//! One HTML file a scout saves on their tablet while the Pi is in reach,
//! and opens later with no server at all. It is cut for them: the season's
//! form, the event's schedule and roster, their assignments (L4, by account
//! or by this tablet), and a fresh offline token and device key for this
//! tablet (C9, T1). The stylesheet and the scripts are inside it, so it
//! loads nothing.
//!
//! What it saves goes into the tablet's outbox in C13's format and reaches
//! the Pi by one of three roads: a POST to `/api/sync/push` with the
//! credential when the Pi answers (`crate::push`, which answers a page
//! opened from a file), the QR handoff (S13), or a file a lead uploads on
//! the scan page (`crate::handoff::receive_file`). The Pi checks each entry
//! as it checks any push. The file is good for as long as its token: 72
//! hours, or until a lead signs its scout out everywhere.

use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use serde_json::json;
use tracing::{info, warn};
use tt_core::assignments::{self, AssigneeKey};
use tt_core::form::{FormErrors, RawAnswers};
use tt_core::records::MatchRecord;
use tt_core::timezone::day_and_time;
use tt_core::user::User;
use tt_repo::Repo;
use tt_templates::{Page, ScoutFilePage, form_sections, json_in_script, script_safe};

use crate::auth::device_uuid;
use crate::events::EventContext;
use crate::startup::AppState;

/// What the file runs, in order. `scout-file.js` first: it draws the form
/// and stands in for `token.js` before the others look for either.
const SCRIPTS: [&str; 8] = [
    "js/scout-file.js",
    "js/counter.js",
    "js/draft.js",
    "vendor/qrcode-generator/qrcode.js",
    "js/qr.js",
    "js/scan.js",
    "js/hmac.js",
    "js/handoff.js",
];

fn asset_text(path: &str) -> String {
    let asset = crate::assets::find(path).unwrap_or_else(|| panic!("{path} is compiled in"));
    String::from_utf8_lossy(asset.bytes).into_owned()
}

fn refuse(status: StatusCode, why: &str) -> Response {
    (status, why.to_string()).into_response()
}

/// The file for `user` at `context`'s event, as an attachment.
pub async fn download(
    state: &AppState,
    user: &User,
    context: &EventContext,
    headers: &HeaderMap,
) -> Response {
    let Some(device) = device_uuid(headers) else {
        return refuse(
            StatusCode::BAD_REQUEST,
            "This browser has no device id yet. Open the scouting page, then download again.",
        );
    };
    let Some(event) = &context.selected else {
        return refuse(
            StatusCode::NOT_FOUND,
            "No events have been loaded yet, so there is nothing to scout.",
        );
    };
    let matches = match state.repo.event_matches(&event.key).await {
        Ok(matches) => matches,
        Err(e) => {
            warn!("matches for {}: {e}", event.key);
            return refuse(StatusCode::SERVICE_UNAVAILABLE, "storage unavailable");
        }
    };
    if matches.is_empty() {
        return refuse(
            StatusCode::NOT_FOUND,
            "This event has no match schedule yet, so there is nothing to scout.",
        );
    }
    let issued = match crate::token::issue_to(state, user, &device).await {
        Ok(issued) => issued,
        Err(e) => {
            warn!("issuing a token for a scouting file: {e}");
            return refuse(StatusCode::SERVICE_UNAVAILABLE, "storage unavailable");
        }
    };

    // Each costs the file a shortcut, never the file.
    let recorded = state
        .repo
        .recorded_by(&event.key, user.id)
        .await
        .unwrap_or_else(|e| {
            warn!("robots recorded by {} at {}: {e}", user.id, event.key);
            Vec::new()
        });
    let event_assignments = state
        .repo
        .event_assignments(&event.key)
        .await
        .unwrap_or_else(|e| {
            warn!("assignments for {}: {e}", event.key);
            Vec::new()
        });
    let roster = state
        .repo
        .event_teams(&event.key)
        .await
        .unwrap_or_else(|e| {
            warn!("roster for {}: {e}", event.key);
            Vec::new()
        });
    let mut me = vec![AssigneeKey::Scout(user.id)];
    me.extend(
        crate::scouting::device_id(state, Some(&device))
            .await
            .map(AssigneeKey::Device),
    );
    let agenda = assignments::agenda(&matches, &event_assignments, &me, &recorded);
    let order = |key: &str| matches.iter().position(|m| m.key == key);
    let mut duties: Vec<&(String, i32)> = agenda.missed.iter().chain(&agenda.upcoming).collect();
    duties.sort_by_key(|(key, _)| order(key));

    let zone = event.zone();
    let now = Utc::now();
    let slots = |teams: &[Option<i32>]| teams.to_vec();
    let data = json!({
        // Where the file posts. The Pi as this browser reached it: over the
        // event LAN that is plain http.
        "pi": pi_origin(headers),
        "user": user.id,
        "scout": user.name,
        "event": event.key,
        "event_name": event.name,
        "form": state.season.version,
        "made_at": now,
        "token": issued.token,
        "key": issued.key,
        "expires_at": issued.expires_at,
        "matches": matches.iter().map(|m: &MatchRecord| json!({
            "key": m.key,
            "label": m.label(),
            "time": m.scheduled_at.map(|at| day_and_time(zone, at)).unwrap_or_default(),
            "red": slots(&m.red),
            "blue": slots(&m.blue),
        })).collect::<Vec<_>>(),
        "teams": roster
            .iter()
            .map(|t| (t.number.to_string(), json!(t.name)))
            .collect::<serde_json::Map<_, _>>(),
        "duties": duties.iter().map(|(key, team)| json!([key, team])).collect::<Vec<_>>(),
        "recorded": recorded.iter().map(|(key, team)| format!("{key}:{team}")).collect::<Vec<_>>(),
    });

    let page = ScoutFilePage {
        scout: user.name.clone(),
        event_name: event.name.clone(),
        made: day_and_time(zone, now),
        expires: day_and_time(zone, issued.expires_at),
        form_version: state.season.version,
        sections: form_sections(
            &state.season,
            &RawAnswers::default(),
            &FormErrors::default(),
        ),
        notes_unshared: user.team_number.is_none(),
        css: asset_text("css/site.css"),
        data: json_in_script(&data.to_string()),
        scripts: SCRIPTS
            .iter()
            .map(|path| script_safe(&asset_text(path)))
            .collect(),
    };
    let html = match page.render_html() {
        Ok(html) => html,
        Err(e) => {
            warn!("rendering a scouting file: {e}");
            return refuse(StatusCode::INTERNAL_SERVER_ERROR, "the page did not render");
        }
    };
    info!(user = %user.email, %device, event = %event.key, duties = duties.len(), "scouting file downloaded");
    let name = format!(
        "attachment; filename=\"tealteam-scouting-{}-{}.html\"",
        event.key,
        file_word(&user.name)
    );
    (
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/html; charset=utf-8"),
            ),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
            (
                header::CONTENT_DISPOSITION,
                HeaderValue::from_str(&name).unwrap_or(HeaderValue::from_static("attachment")),
            ),
        ],
        html,
    )
        .into_response()
}

/// `http://` and the `Host` the browser asked for: the address that
/// reached the Pi from this tablet, so the file posts to the same one.
fn pi_origin(headers: &HeaderMap) -> String {
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .filter(|h| {
            !h.is_empty()
                && h.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']'))
        })
        .unwrap_or("localhost");
    format!("http://{host}")
}

/// A name as part of a file name: letters and digits, lower case.
fn file_word(name: &str) -> String {
    let word: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .take(24)
        .collect();
    if word.is_empty() {
        "scout".into()
    } else {
        word
    }
}
