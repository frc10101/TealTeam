//! The service worker's way in (C5).
//!
//! `deploy/build-client.sh` runs wasm-bindgen over this crate, and `sw.js`
//! imports what it makes. When a page cannot be fetched, the worker calls
//! `wasm_bindgen.render(path, query)`: the page's HTML, made from the
//! device's copy, or `null` for the offline shell.
//!
//! The copy is read from OPFS on every call rather than kept. Pages are only
//! made while the server is gone, and `snapshot.js` may have replaced the file
//! from a page since the last one.
//!
//! `wasm_bindgen.courier(bearer)` is a lead scout's fetch with the tablet's
//! own signal (S7, [`crate::courier`]), asked for by `static/js/courier.js`
//! through the worker. It is the one call here that writes, so the worker
//! runs one at a time: one context must own the file (see the crate's docs).

use std::sync::OnceLock;

use chrono::DateTime;
use tt_core::season::{SeasonSchema, current_season};
use tt_repo::RepoError;
use wasm_bindgen::prelude::*;

use crate::opfs;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console)]
    fn warn(message: &str);
}

/// The season this build was made with, as the server's binary embeds it.
fn season() -> Option<&'static SeasonSchema> {
    static SEASON: OnceLock<Option<SeasonSchema>> = OnceLock::new();
    SEASON
        .get_or_init(|| {
            current_season()
                .inspect_err(|e| warn(&format!("TealTeam: the season does not parse: {e}")))
                .ok()
        })
        .as_ref()
}

/// The page at `path?query`, made from this device's copy, or `null`.
///
/// `null` too when there is no copy to make it from: over plain http, or
/// before the first snapshot (S10). Those are expected and say nothing.
#[wasm_bindgen]
pub async fn render(path: String, query: String) -> JsValue {
    let Some(season) = season() else {
        return JsValue::NULL;
    };
    let repo = match opfs::open().await {
        Ok(repo) => repo,
        Err(RepoError::Unavailable(_)) => return JsValue::NULL,
        Err(e) => {
            warn(&format!("TealTeam: opening this device's copy: {e}"));
            return JsValue::NULL;
        }
    };
    let now = DateTime::from_timestamp_millis(js_sys::Date::now() as i64).unwrap_or_default();
    match crate::pages::render(&repo, season, &path, &query, now).await {
        Some(html) => html.into(),
        None => JsValue::NULL,
    }
}

/// One courier tick (S7): ask the Pi for the key, fetch from TBA with this
/// device's signal, and push what the Pi has not had. The tick's report as
/// JSON, or `null` when there is no copy to keep it in.
///
/// `bearer` is the page's offline token (C9), since a worker cannot read
/// `localStorage`. The events are those the device's copy was cut for, or,
/// when it named none, those on today.
#[wasm_bindgen]
pub async fn courier(bearer: Option<String>) -> JsValue {
    let repo = match opfs::open().await {
        Ok(repo) => repo,
        Err(RepoError::Unavailable(_)) => return JsValue::NULL,
        Err(e) => {
            warn(&format!("TealTeam: opening this device's copy: {e}"));
            return JsValue::NULL;
        }
    };
    let now = DateTime::from_timestamp_millis(js_sys::Date::now() as i64).unwrap_or_default();
    let transport = crate::sync::Fetch {
        bearer: bearer.filter(|t| !t.is_empty()),
        timeout_ms: Some(15_000),
    };
    let courier = crate::courier::Courier::new(transport, opfs::snapshot_events().await);
    let fresh_log = || {
        let random: [u8; 10] = std::array::from_fn(|_| (js_sys::Math::random() * 256.0) as u8);
        tt_core::record_id::uuid_v7(now.timestamp_millis() as u64, random)
    };
    match courier.tick(&repo, now, fresh_log).await {
        Ok(tick) => serde_json::to_string(&tick)
            .map(JsValue::from)
            .unwrap_or(JsValue::NULL),
        Err(e) => {
            warn(&format!("TealTeam: fetching upstream on this device: {e}"));
            JsValue::NULL
        }
    }
}
