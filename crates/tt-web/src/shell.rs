//! The offline shell (C1): `/sw.js`, and the `/offline` page it falls back to.
//!
//! The worker's source is `src/sw.js`. What it precaches, and the build it
//! belongs to, are filled in here from the embedded asset table, so the list
//! cannot fall out of step with what is actually served. `BUILD_VERSION`
//! (from `build.rs`) changes whenever a static file, a template, or the
//! worker itself does, and a changed `/sw.js` is what makes a browser install
//! the new worker and drop the old cache.
//!
//! S11's version handshake builds on the same number: `/health` reports it,
//! so a page (or later the wasm client) can tell a new binary is running.

use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::response::{Html, IntoResponse, Response};
use tt_templates::{OfflinePage, Page, PageVersion};

use crate::assets::{ASSETS, BUILD_VERSION};

const SOURCE: &str = include_str!("sw.js");

/// What a page is stamped with, and `/health` reports (S11).
pub fn page_version(state: &crate::startup::AppState) -> PageVersion {
    PageVersion {
        build: BUILD_VERSION.to_string(),
        schema: tt_repo_sqlite::migrate::latest(),
        form: state.season.version,
    }
}

/// Everything the worker caches at install: the shell page and every static
/// file. Never a page with anyone's data in it.
pub fn precache() -> Vec<String> {
    std::iter::once("/offline".to_string())
        .chain(ASSETS.iter().map(|a| format!("/static/{}", a.path)))
        .collect()
}

pub fn script() -> String {
    SOURCE.replace("__BUILD__", BUILD_VERSION).replace(
        "__PRECACHE__",
        &serde_json::to_string(&precache()).expect("strings serialise"),
    )
}

/// `GET /sw.js`. From the root, so the worker's scope is the whole site.
/// `no-cache`, so the browser's update check always sees the current build.
pub async fn service_worker() -> Response {
    (
        [
            (CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (CACHE_CONTROL, "no-cache"),
        ],
        script(),
    )
        .into_response()
}

/// `GET /offline`: the same bytes for everyone.
pub async fn offline_page() -> Response {
    match OfflinePage::default().render_html() {
        Ok(html) => Html(html).into_response(),
        Err(e) => {
            tracing::error!("rendering the offline page: {e}");
            axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_worker_carries_its_build_and_the_whole_shell() {
        let js = script();
        assert!(js.contains(&format!("const BUILD = \"{BUILD_VERSION}\";")));
        assert!(!js.contains("__PRECACHE__") && !js.contains("__BUILD__"));
        let list = precache();
        assert_eq!(list[0], "/offline");
        for asset in ASSETS {
            assert!(
                list.contains(&format!("/static/{}", asset.path)),
                "{}",
                asset.path
            );
        }
        assert!(!list.iter().any(|u| u == "/sw.js"), "never itself");
        assert_eq!(BUILD_VERSION.len(), 16);
    }

    #[test]
    fn the_shell_holds_nothing_about_anyone() {
        let html = OfflinePage::default().render_html().unwrap();
        assert!(html.contains(r#"id="offline-shell""#));
        assert!(!html.contains("Sign out") && !html.contains("Sign in"));
        assert!(!html.contains("nav-links"), "no one's sections");
        assert!(
            !html.contains("Storage unavailable"),
            "not a snapshot of the server's state"
        );
    }
}
