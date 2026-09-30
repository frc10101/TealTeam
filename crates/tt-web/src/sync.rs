//! `GET /api/sync/pull` (S2): what changed since a client last asked.
//!
//! Two streams, two cursors, one request. `changes` is the venue stream: what
//! scouts and leads wrote, written by triggers so deletions are in it too.
//! `upstream` is S1's log of FIRST and TBA responses. Each comes back with the
//! cursor to send next time; `more` says to ask again straight away.
//!
//! Changes are served only once they are [`LAG`] old. A change still being
//! committed has its `seq` already, and a client that read past it would never
//! see it; waiting two seconds costs nothing a person would notice.
//!
//! What a viewer may see is decided per change in [`visible`], the one place
//! S3's subscription scope will go.
//!
//! `GET /api/sync/stream` (S8) is the same two streams pushed as server-sent
//! events, for as long as the connection lasts: see [`stream`].
//!
//! **Schema handshake (S11).** A client says which schema it was built for
//! with `?schema=<n>`, the newest migration it knows. The change log's rows
//! are shaped by the schema, so a client on another one is refused with 409
//! and told which side is behind, rather than fed rows it will misread: an
//! older client must reload into the new version, and a newer one has met a
//! Pi running an old build, which the lead scout needs to hear about. Every
//! answer also says the server's schema, build, and form version, so a
//! client that did not send one is still told.

use std::collections::{HashMap, VecDeque};
use std::convert::Infallible;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::Json;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use chrono::{TimeDelta, Utc};
use futures_util::stream;
use serde_json::{Value as JsonValue, json};
use tracing::warn;
use tt_core::notes::{self, Notes};
use tt_core::season::{Payload, SeasonSchema};
use tt_core::user::User;
use tt_repo::{Change, Repo};

use crate::auth::Auth;
use crate::startup::AppState;

/// How old a change must be before it is served.
pub const LAG: TimeDelta = TimeDelta::seconds(2);
/// Most changes per pull. Small rows; a whole event is a few thousand.
pub const CHANGES_PER_PULL: i64 = 500;
/// Most upstream responses per pull. These are whole API bodies.
pub const UPSTREAM_PER_PULL: i64 = 20;

/// The 409 for a client that said it was built for another schema (S11), or
/// nothing when it matches or did not say. The same for `pull` and `stream`.
fn schema_mismatch(state: &AppState, query: &HashMap<String, String>) -> Option<Response> {
    let client = query.get("schema")?.parse::<i64>().ok()?;
    let server = crate::shell::page_version(state);
    (client != server.schema).then(|| {
        (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "schema mismatch",
                "client_schema": client,
                "server_schema": server.schema,
                "server_build": server.build,
                // What the client should do about it.
                "action": if client < server.schema { "reload" } else { "server-behind" },
            })),
        )
            .into_response()
    })
}

/// `GET /api/sync/pull?changes=<cursor>&upstream=<cursor>`. Cursors default to
/// 0, which is everything.
pub async fn pull(
    State(state): State<AppState>,
    Auth(viewer): Auth,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let cursor = |name: &str| {
        query
            .get(name)
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(0)
            .max(0)
    };
    let (after_change, after_upstream) = (cursor("changes"), cursor("upstream"));

    if let Some(refused) = schema_mismatch(&state, &query) {
        return refused;
    }
    let server = crate::shell::page_version(&state);

    let changes = state
        .repo
        .changes_since(after_change, CHANGES_PER_PULL, Utc::now() - LAG)
        .await;
    let upstream = state
        .repo
        .upstream_since(after_upstream, UPSTREAM_PER_PULL)
        .await;
    let (changes, upstream) = match (changes, upstream) {
        (Ok(c), Ok(u)) => (c, u),
        (Err(e), _) | (_, Err(e)) => {
            warn!("sync pull: {e}");
            return (
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "error": "storage unavailable" })),
            )
                .into_response();
        }
    };

    // The cursor moves past every change read, shown or not, so a client is
    // never sent back for what it may not see.
    let changes_cursor = changes.last().map_or(after_change, |c| c.seq);
    let changes_more = changes.len() as i64 == CHANGES_PER_PULL;
    let upstream_cursor = upstream.last().map_or(after_upstream, |u| u.seq);
    let upstream_more = upstream.len() as i64 == UPSTREAM_PER_PULL;

    let shown: Vec<JsonValue> = changes
        .into_iter()
        .filter_map(|c| visible(&state.season, &viewer, c))
        .collect();
    let upstream: Vec<JsonValue> = upstream
        .into_iter()
        .map(|u| {
            json!({
                "seq": u.seq,
                "api": u.entry.api,
                "path": u.entry.path,
                "etag": u.entry.etag,
                "body": u.entry.body,
                "fetched_at": u.entry.fetched_at,
            })
        })
        .collect();

    Json(json!({
        "schema": server.schema,
        "build": server.build,
        "form": server.form,
        "changes": shown,
        "changes_cursor": changes_cursor,
        "changes_more": changes_more,
        "upstream": upstream,
        "upstream_cursor": upstream_cursor,
        "upstream_more": upstream_more,
    }))
    .into_response()
}

/// What `viewer` may see of `change`, or nothing.
///
/// Two rules hold now and always: a team-scoped change (a pick list) goes to
/// that team only, and an observation's notes go only to the team that wrote
/// them (U13). Everything else is public.
///
/// **S3 hook:** a client's subscription scope (its event, what it asked for)
/// filters here, before these two rules, never instead of them.
pub fn visible(schema: &SeasonSchema, viewer: &User, change: Change) -> Option<JsonValue> {
    if change
        .team_scope
        .is_some_and(|team| viewer.team_number != Some(team))
    {
        return None;
    }

    let mut row: Option<JsonValue> = change
        .payload
        .as_deref()
        .map(|raw| serde_json::from_str(raw).unwrap_or_else(|_| json!({})));
    if change.entity == "observation"
        && let Some(JsonValue::Object(fields)) = row.as_mut()
    {
        let writer = fields
            .get("submitting_team")
            .and_then(JsonValue::as_i64)
            .and_then(|t| i32::try_from(t).ok());
        if !Notes::for_viewer(viewer.team_number, writer).shown() {
            // Unreadable answers are sent as none: never fail open.
            let mut answers: Payload = fields
                .get("payload")
                .cloned()
                .and_then(|p| serde_json::from_value(p).ok())
                .unwrap_or_default();
            notes::redact(schema, &mut answers);
            fields.insert(
                "payload".into(),
                serde_json::to_value(answers).unwrap_or_else(|_| json!({})),
            );
        }
    }

    Some(json!({
        "seq": change.seq,
        "entity": change.entity,
        "entity_pk": change.entity_pk,
        "op": change.op,
        "row": row,
        "event_key": change.event_key,
        "at": change.created_at,
    }))
}

// ── Fan-out (S8) ────────────────────────────────────────────────────────────

/// How often an open stream looks for new rows. Changes wait [`LAG`] anyway,
/// so a second more is not noticed, and a query a second per tablet is
/// nothing to SQLite.
pub const STREAM_POLL: Duration = Duration::from_secs(1);
/// A comment line this often, so phones and proxies keep an idle stream open.
pub const HEARTBEAT: Duration = Duration::from_secs(15);
/// Most streams open at once. A team's tablets and laptops are a few dozen;
/// past this a client polls [`pull`] instead.
pub const MAX_STREAMS: usize = 64;

static OPEN_STREAMS: AtomicUsize = AtomicUsize::new(0);

/// One of the [`MAX_STREAMS`], held for as long as its stream lives.
pub struct Slot(&'static AtomicUsize);

impl Slot {
    pub fn acquire(open: &'static AtomicUsize, max: usize) -> Option<Self> {
        open.fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
            (n < max).then_some(n + 1)
        })
        .ok()
        .map(|_| Self(open))
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Both cursors, as one event id: `"<changes>-<upstream>"`. What a browser
/// sends back as `Last-Event-ID` when it reconnects.
pub fn event_id(changes: i64, upstream: i64) -> String {
    format!("{changes}-{upstream}")
}

/// The cursors from a `Last-Event-ID`; `None` for anything else.
pub fn parse_event_id(raw: &str) -> Option<(i64, i64)> {
    let (changes, upstream) = raw.trim().split_once('-')?;
    Some((
        changes.parse::<i64>().ok()?.max(0),
        upstream.parse::<i64>().ok()?.max(0),
    ))
}

/// `GET /api/sync/stream`: the two streams of [`pull`], pushed.
///
/// Resumes from `Last-Event-ID` (a browser's reconnect), else from
/// `?changes=&upstream=` (a client's saved cursors), else from the start.
/// Event types, each with the cursors as its id:
///
/// - `change`: one row of the venue stream, as [`visible`] shows it.
/// - `upstream`: one FIRST or TBA response.
/// - `cursor`: only an id. Rows went by that this viewer may not see; saves
///   a reconnect reading them again.
///
/// Later types join these on the same channel: assignment pushes (S9) and
/// chat (X2). A client listens for the types it knows, and `EventSource`
/// ignores the rest. Over [`MAX_STREAMS`] the answer is a 503 naming the
/// polling fallback. With `?schema=` not the server's, a 409 as for [`pull`]
/// (S11).
pub async fn stream(
    State(state): State<AppState>,
    Auth(viewer): Auth,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    // Before a slot is taken. A 409 also stops an EventSource for good,
    // where a stream that ended would have it reconnect over and over; the
    // client learns why from a pull with the same ?schema=.
    if let Some(refused) = schema_mismatch(&state, &query) {
        return refused;
    }
    let Some(slot) = Slot::acquire(&OPEN_STREAMS, MAX_STREAMS) else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            [(header::RETRY_AFTER, "30")],
            Json(json!({
                "error": "too many live connections; poll instead",
                "poll": "/api/sync/pull",
            })),
        )
            .into_response();
    };

    let from_query = |name: &str| {
        query
            .get(name)
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(0)
            .max(0)
    };
    let (changes, upstream) = headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(parse_event_id)
        .unwrap_or_else(|| (from_query("changes"), from_query("upstream")));

    let tail = Tail {
        state,
        viewer,
        changes,
        upstream,
        pending: VecDeque::new(),
        first: true,
        _slot: slot,
    };
    let events = stream::unfold(tail, |mut tail| async move {
        loop {
            if let Some(event) = tail.pending.pop_front() {
                return Some((Ok::<_, Infallible>(event), tail));
            }
            if !tail.first {
                tokio::time::sleep(STREAM_POLL).await;
            }
            tail.first = false;
            tail.fill().await;
        }
    });
    Sse::new(events)
        .keep_alive(KeepAlive::new().interval(HEARTBEAT))
        .into_response()
}

/// One open stream: whose it is, how far it has got, and what is ready to send.
struct Tail {
    state: AppState,
    viewer: User,
    changes: i64,
    upstream: i64,
    pending: VecDeque<Event>,
    first: bool,
    _slot: Slot,
}

impl Tail {
    /// Queue whatever is new since the cursors. A storage error sends nothing
    /// and is tried again next time round.
    async fn fill(&mut self) {
        match self
            .state
            .repo
            .changes_since(self.changes, CHANGES_PER_PULL, Utc::now() - LAG)
            .await
        {
            Ok(rows) => {
                let mut unsent = false;
                for change in rows {
                    self.changes = change.seq;
                    match visible(&self.state.season, &self.viewer, change) {
                        Some(row) => {
                            unsent = false;
                            self.push("change", &row);
                        }
                        None => unsent = true,
                    }
                }
                if unsent {
                    self.push("cursor", &json!({}));
                }
            }
            Err(e) => warn!("sync stream: {e}"),
        }
        match self
            .state
            .repo
            .upstream_since(self.upstream, UPSTREAM_PER_PULL)
            .await
        {
            Ok(rows) => {
                for u in rows {
                    self.upstream = u.seq;
                    self.push(
                        "upstream",
                        &json!({
                            "seq": u.seq,
                            "api": u.entry.api,
                            "path": u.entry.path,
                            "etag": u.entry.etag,
                            "body": u.entry.body,
                            "fetched_at": u.entry.fetched_at,
                        }),
                    );
                }
            }
            Err(e) => warn!("sync stream: {e}"),
        }
    }

    fn push(&mut self, kind: &str, data: &JsonValue) {
        let event = Event::default()
            .event(kind)
            .id(event_id(self.changes, self.upstream))
            .data(data.to_string());
        self.pending.push_back(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_event_id_carries_both_cursors_and_junk_is_ignored() {
        assert_eq!(event_id(42, 7), "42-7");
        assert_eq!(parse_event_id(" 42-7 "), Some((42, 7)));
        assert_eq!(parse_event_id("-5-3"), None);
        assert_eq!(parse_event_id("42"), None);
        assert_eq!(parse_event_id("a-b"), None);
    }

    #[test]
    fn streams_past_the_cap_are_refused_and_a_closed_one_frees_its_slot() {
        static OPEN: AtomicUsize = AtomicUsize::new(0);
        let first = Slot::acquire(&OPEN, 2).expect("one");
        let _second = Slot::acquire(&OPEN, 2).expect("two");
        assert!(Slot::acquire(&OPEN, 2).is_none(), "full");
        drop(first);
        assert!(Slot::acquire(&OPEN, 2).is_some(), "freed on close");
    }
}
