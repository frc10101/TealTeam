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

/// The QR reader (zxing-wasm) inside the file, for scanning the lead's
/// receipt: a `file://` page cannot fetch `/static/vendor/zxing/`, and is no
/// secure context, so it has no `BarcodeDetector` either. `scan.js` takes it
/// from `window.ttZxing` when it is there. About a megabyte, most of it the
/// wasm as base64.
fn zxing_inline() -> String {
    use base64::Engine;
    let wasm = crate::assets::find("vendor/zxing/zxing_reader.wasm")
        .expect("vendor/zxing/zxing_reader.wasm is compiled in");
    let js = serde_json::to_string(&asset_text("vendor/zxing/zxing-reader.js"))
        .expect("a string serialises");
    script_safe(&format!(
        "window.ttZxing = {{ js: {js}, wasm: \"{}\" }};",
        base64::engine::general_purpose::STANDARD.encode(wasm.bytes)
    ))
}

fn refuse(status: StatusCode, why: &str) -> Response {
    (status, why.to_string()).into_response()
}

/// The file for `user` at `context`'s event, as an attachment.
/// How the browser reached the server, for the address the file posts to.
pub struct Reached<'a> {
    pub uri: &'a axum::http::Uri,
    pub headers: &'a HeaderMap,
    /// Over HTTPS (H1).
    pub tls: bool,
}

pub async fn download(
    state: &AppState,
    user: &User,
    context: &EventContext,
    reached: &Reached<'_>,
) -> Response {
    let headers = reached.headers;
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
        // Where the file posts: the Pi as this browser reached it, by the
        // same scheme, host and port.
        "pi": pi_origin(reached),
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
        scripts: std::iter::once(zxing_inline())
            .chain(SCRIPTS.iter().map(|path| script_safe(&asset_text(path))))
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

/// The address that reached the Pi from this tablet, so the file posts to
/// the same one. HTTP/2 (every HTTPS request from Chrome) names the host in
/// the request's authority and sends no `Host`, so that comes first; and
/// the scheme is the one the request came by, or a file downloaded over
/// HTTPS would post to `http://` on the HTTPS port.
fn pi_origin(reached: &Reached<'_>) -> String {
    let plain = |h: &str| {
        !h.is_empty()
            && h.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']'))
    };
    let host = reached
        .uri
        .authority()
        .map(|a| a.as_str())
        .filter(|h| plain(h))
        .or_else(|| {
            reached
                .headers
                .get(header::HOST)
                .and_then(|h| h.to_str().ok())
                .filter(|h| plain(h))
        })
        .unwrap_or("localhost");
    let scheme = if reached.tls { "https" } else { "http" };
    format!("{scheme}://{host}")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_file_posts_back_the_way_the_browser_came() {
        use axum::http::{HeaderValue, Uri};
        let origin = |uri: &str, host: Option<&str>, tls: bool| {
            let uri: Uri = uri.parse().unwrap();
            let mut headers = HeaderMap::new();
            if let Some(h) = host {
                headers.insert(header::HOST, HeaderValue::from_str(h).unwrap());
            }
            pi_origin(&Reached {
                uri: &uri,
                headers: &headers,
                tls,
            })
        };
        // HTTP/2 over HTTPS: the authority, and no Host.
        assert_eq!(
            origin("https://10.101.0.1:8443/submission/download", None, true),
            "https://10.101.0.1:8443"
        );
        // HTTP/1.1: the Host, and the scheme it came by.
        assert_eq!(
            origin("/submission/download", Some("tealteam.local"), false),
            "http://tealteam.local"
        );
        assert_eq!(
            origin("/submission/download", Some("10.101.0.1:8443"), true),
            "https://10.101.0.1:8443"
        );
        // Nothing to go on, or something odd: this machine.
        assert_eq!(
            origin("/submission/download", None, false),
            "http://localhost"
        );
        assert_eq!(
            origin("/submission/download", Some("evil\"x"), false),
            "http://localhost"
        );
    }
}
