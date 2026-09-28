//! Error pages (U10).
//!
//! Handlers answer their own mistakes: a form that cannot be saved comes back
//! with the message beside the field, from that page's template. This is for
//! what goes wrong around a handler instead -- a mistyped address, a form's
//! address reopened from history, a form body the extractor rejects, a page
//! that fails to render. Left alone, axum answers those with an empty body or a
//! line of plain text: on a phone, a blank screen with no nav and no way back.
//!
//! One layer covers every route, including ones not written yet. An error
//! status going to a browser, with a body that is not already HTML, gets the
//! error page instead. The status and headers stay (a 405 keeps `Allow`), and
//! the text it replaces is logged, since that is the part worth reading.
//! Scripts, and a page's own `<script>` and `<link>` loads, do not ask for
//! `text/html`, so they get the response untouched.

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::header::{CONTENT_LENGTH, CONTENT_TYPE, COOKIE};
use axum::http::{HeaderMap, HeaderValue};
use axum::middleware::Next;
use axum::response::Response;
use tt_templates::{ErrorPage, Page};

use crate::auth;
use crate::handlers::{nav_for, wants_html};
use crate::startup::AppState;

/// The most of a replaced body worth keeping for the log.
const LOGGED_BODY_LIMIT: usize = 4096;

pub async fn html_errors(State(state): State<AppState>, request: Request, next: Next) -> Response {
    if !wants_html(request.headers()) {
        return next.run(request).await;
    }
    // The request is gone once it is handed on; keep what the page needs.
    let cookies: HeaderMap = request
        .headers()
        .get_all(COOKIE)
        .iter()
        .map(|value| (COOKIE, value.clone()))
        .collect();
    let path = request.uri().path().to_owned();

    let response = next.run(request).await;
    let status = response.status();
    if !(status.is_client_error() || status.is_server_error()) || is_html(&response) {
        return response;
    }

    let (mut parts, body) = response.into_parts();
    let replaced = axum::body::to_bytes(body, LOGGED_BODY_LIMIT)
        .await
        .unwrap_or_default();
    let reason = String::from_utf8_lossy(&replaced);
    if status.is_server_error() {
        tracing::warn!(%status, path = %path, %reason, "error page");
    } else {
        tracing::info!(%status, path = %path, %reason, "error page");
    }

    let user = auth::current_user(&state, &cookies).await;
    let nav = nav_for(&state, user.as_ref()).await;
    // Whichever body goes out, it is not the one this length was for.
    parts.headers.remove(CONTENT_LENGTH);
    match ErrorPage::new(status.as_u16(), path, nav).render_html() {
        Ok(html) => {
            parts.headers.insert(
                CONTENT_TYPE,
                HeaderValue::from_static("text/html; charset=utf-8"),
            );
            Response::from_parts(parts, Body::from(html))
        }
        Err(e) => {
            // The layout itself will not render, so the error page cannot
            // either; the original text is still better than nothing.
            tracing::error!("rendering the error page: {e}");
            Response::from_parts(parts, Body::from(replaced))
        }
    }
}

fn is_html(response: &Response) -> bool {
    response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("text/html"))
}
