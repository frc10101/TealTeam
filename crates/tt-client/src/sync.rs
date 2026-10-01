//! The sync client (C7): push the outbox, pull what changed, keep the
//! device's copy of the Pi's tables current.
//!
//! One [`SyncClient::sync`] is a push and then pulls until caught up:
//!
//! - **Push.** The outbox goes to `POST /api/sync/push` in turns of
//!   [`MAX_PER_PUSH`]. A refusal is kept with its reason; a "recorded" waits.
//! - **Pull.** `GET /api/sync/pull` from the cursors the snapshot (S10) left
//!   in `sync_state`. Each change is applied to the table it names, keyed as
//!   the log keys it, and an observation coming back clears its outbox
//!   entry: that echo is the only "saved" the device believes. Upstream
//!   responses are appended to the device's log as the Pi appends them.
//!
//! Every request carries `?schema=`, the newest migration in this file
//! (S11). The Pi answers another schema with 409, and the client stops:
//! [`Stop::Reload`] when this device is behind, and [`Stop::ServerBehind`]
//! when the Pi is, which is for the lead scout and never a reload. Neither
//! drops the outbox; [`ClientRepo::export_outbox`] is the way out of it.
//!
//! The network is a [`Transport`]: the browser's `fetch` ([`Fetch`]) on
//! wasm, and anything at all in a test. Nothing here runs on a timer or
//! listens to `/api/sync/stream`; whatever owns the database (C5's service
//! worker) calls `sync` when the page asks, when the network returns, and on
//! each event from [`SyncClient::stream_url`].

use chrono::{DateTime, Utc};
use rusqlite::{Transaction, params};
use serde::Deserialize;
use serde_json::{Map, Value as JsonValue};
use tt_core::outbox::{MAX_PER_PUSH, Outcome, Push, PushReply};
use tt_repo::{NewUpstream, RepoError, Result};

use crate::ClientRepo;
use crate::sql::{Context, to_sql};

/// The cursors a snapshot leaves (`tt_repo_sqlite::snapshot`).
pub const CHANGES_SOURCE: &str = "server:changes";
pub const UPSTREAM_SOURCE: &str = "server:upstream";

/// Most pulls in one sync. A pull is 500 changes, so this is a whole event
/// several times over; past it, the next sync carries on.
const MAX_PULLS: usize = 40;

/// What came back over the network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    /// 0 for a redirect the browser would not follow (`redirect: "manual"`).
    pub status: u16,
    pub body: String,
}

/// How requests reach the Pi. `Err` is no answer at all: offline.
// Not `Send`: a browser's futures never are, and the device has one thread.
#[allow(async_fn_in_trait)]
pub trait Transport {
    async fn get(&self, path: &str) -> std::result::Result<Reply, String>;
    async fn post_json(&self, path: &str, body: String) -> std::result::Result<Reply, String>;
    /// A SQLite file, as `application/vnd.sqlite3`: an upstream bundle (S7).
    async fn post_file(&self, path: &str, body: Vec<u8>) -> std::result::Result<Reply, String>;
}

/// Why a sync stopped before it was done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stop {
    /// No answer, or none that could be used yet. Try again later.
    Offline(String),
    /// This device's build is older than the Pi's. Reload into the new one,
    /// once the outbox is exported if it cannot be pushed.
    Reload,
    /// The Pi runs an older build than this device. Tell the lead scout;
    /// reloading would not help.
    ServerBehind,
    /// The session is gone. Sign in again; the outbox waits.
    SignedOut,
    /// The Pi refused the request as a whole, and saying it again will not
    /// change that.
    Refused(String),
}

/// What one sync did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    /// Outbox entries sent, counting a resend.
    pub sent: usize,
    /// Entries the Pi refused this time.
    pub refused: usize,
    /// Entries whose observation came back, and so are done.
    pub echoed: usize,
    /// Changes applied to this device's tables.
    pub applied: usize,
    /// Upstream responses appended to this device's log.
    pub upstream: usize,
    pub stopped: Option<Stop>,
}

pub struct SyncClient<T> {
    transport: T,
    /// The events subscribed to (S3); none is every event.
    events: Vec<String>,
}

impl<T: Transport> SyncClient<T> {
    pub fn new(transport: T, events: Vec<String>) -> Self {
        let events = events
            .into_iter()
            .map(|e| e.trim().to_ascii_lowercase())
            .filter(|e| !e.is_empty() && e.bytes().all(|b| b.is_ascii_alphanumeric()))
            .collect();
        Self { transport, events }
    }

    /// Push the outbox, then pull until caught up, saving as it goes.
    pub async fn sync(&self, repo: &ClientRepo, now: DateTime<Utc>) -> Result<Report> {
        let mut report = Report::default();
        report.stopped = self.push(repo, now, &mut report).await?;
        if report.stopped.is_none() {
            report.stopped = self.pull(repo, now, &mut report).await?;
        }
        Ok(report)
    }

    /// The outbox, in turns, oldest id first.
    async fn push(
        &self,
        repo: &ClientRepo,
        now: DateTime<Utc>,
        report: &mut Report,
    ) -> Result<Option<Stop>> {
        let path = format!("/api/sync/push?{}", self.schema_query(repo)?);
        let mut after: Option<String> = None;
        loop {
            let observations = repo.waiting(after.as_deref(), MAX_PER_PUSH)?;
            let Some(last) = observations.last() else {
                return Ok(None);
            };
            after = Some(last.client_record_id.clone());
            let full = observations.len() == MAX_PER_PUSH;
            let ids: Vec<String> = observations
                .iter()
                .map(|o| o.client_record_id.clone())
                .collect();
            let body = serde_json::to_string(&Push { observations })
                .map_err(|e| RepoError::Query(format!("writing the outbox out: {e}")))?;

            let reply = self.transport.post_json(&path, body).await;
            repo.mark_tried(&ids.iter().map(String::as_str).collect::<Vec<_>>(), now)?;
            repo.flush().await?;
            let reply = match reply {
                Ok(reply) if reply.status == 200 => reply,
                Ok(reply) => return Ok(Some(stopped(&reply))),
                Err(why) => return Ok(Some(Stop::Offline(why))),
            };
            let Ok(answer) = serde_json::from_str::<PushReply>(&reply.body) else {
                return Ok(Some(Stop::Offline("the Pi's answer was unreadable".into())));
            };
            report.sent += ids.len();
            for receipt in answer.receipts {
                if let Outcome::Refused { reason } = receipt.outcome {
                    repo.mark_refused(&receipt.client_record_id, &reason)?;
                    report.refused += 1;
                }
            }
            repo.flush().await?;
            if !full {
                return Ok(None);
            }
        }
    }

    /// Pull and apply until the Pi has nothing more, or [`MAX_PULLS`].
    async fn pull(
        &self,
        repo: &ClientRepo,
        now: DateTime<Utc>,
        report: &mut Report,
    ) -> Result<Option<Stop>> {
        let schema = repo.schema()?;
        for _ in 0..MAX_PULLS {
            let path = format!(
                "/api/sync/pull?{}&changes={}&upstream={}{}",
                self.schema_query(repo)?,
                repo.cursor(CHANGES_SOURCE)?,
                repo.cursor(UPSTREAM_SOURCE)?,
                self.event_query(),
            );
            let reply = match self.transport.get(&path).await {
                Ok(reply) if reply.status == 200 => reply,
                Ok(reply) => return Ok(Some(stopped(&reply))),
                Err(why) => return Ok(Some(Stop::Offline(why))),
            };
            let Ok(pulled) = serde_json::from_str::<Pulled>(&reply.body) else {
                // A sign-in page where JSON should be is a lost session.
                return Ok(Some(if reply.body.trim_start().starts_with('<') {
                    Stop::SignedOut
                } else {
                    Stop::Offline("the Pi's answer was unreadable".into())
                }));
            };
            // Sent `?schema=`, so a 409 would have come instead; this is for
            // a file that somehow has none.
            if let Some(ours) = schema
                && pulled.schema != ours
            {
                return Ok(Some(behind(ours, pulled.schema)));
            }
            let more = pulled.changes_more || pulled.upstream_more;
            repo.apply(pulled, now, report)?;
            repo.flush().await?;
            if !more {
                return Ok(None);
            }
        }
        Ok(None)
    }

    /// Where to open an `EventSource` for this device (S8): its cursors, its
    /// schema, and its events. An event on it means "sync now".
    pub fn stream_url(&self, repo: &ClientRepo) -> Result<String> {
        Ok(format!(
            "/api/sync/stream?{}&changes={}&upstream={}{}",
            self.schema_query(repo)?,
            repo.cursor(CHANGES_SOURCE)?,
            repo.cursor(UPSTREAM_SOURCE)?,
            self.event_query(),
        ))
    }

    fn schema_query(&self, repo: &ClientRepo) -> Result<String> {
        Ok(match repo.schema()? {
            Some(n) => format!("schema={n}"),
            None => "schema=".into(),
        })
    }

    fn event_query(&self) -> String {
        if self.events.is_empty() {
            String::new()
        } else {
            format!("&event={}", self.events.join(","))
        }
    }
}

/// Why a reply other than 200 stops the sync.
pub(crate) fn stopped(reply: &Reply) -> Stop {
    let error = || {
        serde_json::from_str::<JsonValue>(&reply.body)
            .ok()
            .and_then(|v| v.get("error").and_then(JsonValue::as_str).map(String::from))
    };
    match reply.status {
        // The guards redirect to sign-in; `fetch` with `redirect: "manual"`
        // shows that as 0.
        0 | 300..=399 | 401 | 403 => Stop::SignedOut,
        409 => {
            let body: JsonValue = serde_json::from_str(&reply.body).unwrap_or_default();
            match body.get("action").and_then(JsonValue::as_str) {
                Some("server-behind") => Stop::ServerBehind,
                Some(_) => Stop::Reload,
                None => Stop::Refused(error().unwrap_or_else(|| "conflict".into())),
            }
        }
        413 | 422 => Stop::Refused(error().unwrap_or_else(|| format!("HTTP {}", reply.status))),
        status => Stop::Offline(error().unwrap_or_else(|| format!("HTTP {status}"))),
    }
}

fn behind(ours: i64, theirs: i64) -> Stop {
    if ours < theirs {
        Stop::Reload
    } else {
        Stop::ServerBehind
    }
}

// ── Applying a pull ─────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct Pulled {
    schema: i64,
    changes: Vec<PulledChange>,
    changes_cursor: i64,
    changes_more: bool,
    upstream: Vec<PulledUpstream>,
    upstream_cursor: i64,
    upstream_more: bool,
}

#[derive(Deserialize)]
struct PulledChange {
    seq: i64,
    entity: String,
    entity_pk: String,
    op: String,
    row: Option<JsonValue>,
}

#[derive(Deserialize)]
struct PulledUpstream {
    api: String,
    path: String,
    etag: Option<String>,
    body: String,
    fetched_at: DateTime<Utc>,
}

/// A change's row, read field by field. A field the log always writes but
/// this one lacks makes the change unusable, and it is skipped.
struct Fields<'a>(&'a Map<String, JsonValue>);

impl Fields<'_> {
    fn text(&self, key: &str) -> Option<String> {
        self.0
            .get(key)
            .and_then(JsonValue::as_str)
            .map(String::from)
    }
    fn int(&self, key: &str) -> Option<i64> {
        self.0.get(key).and_then(JsonValue::as_i64)
    }
    fn json(&self, key: &str) -> Option<String> {
        self.0.get(key).map(JsonValue::to_string)
    }
}

impl ClientRepo {
    /// How far this device has read one of the Pi's logs.
    pub fn cursor(&self, source: &str) -> Result<i64> {
        self.conn
            .query_row(
                "SELECT COALESCE(MAX(cursor), 0) FROM sync_state WHERE source = ?",
                [source],
                |row| row.get(0),
            )
            .ctx("reading a sync cursor")
    }

    fn set_cursor(tx: &Transaction, source: &str, cursor: i64, now: DateTime<Utc>) -> Result<()> {
        tx.execute(
            "INSERT INTO sync_state (source, cursor, applied_at) VALUES (?1, ?2, ?3) \
             ON CONFLICT (source) DO UPDATE SET cursor = ?2, applied_at = ?3",
            params![source, cursor, to_sql(now)],
        )
        .map(|_| ())
        .ctx("moving a sync cursor")
    }

    /// One pull's changes in one transaction with their cursor, then its
    /// upstream responses, through the same append as the device's own.
    fn apply(&self, pulled: Pulled, now: DateTime<Utc>, report: &mut Report) -> Result<()> {
        self.ensure_outbox()?;
        let tx = self.conn.unchecked_transaction().ctx("applying a pull")?;
        for change in &pulled.changes {
            match apply_change(&tx, change) {
                Ok(true) => report.applied += 1,
                Ok(false) => tracing::warn!(
                    "skipped change {} ({} {}): not a row this device can store",
                    change.seq,
                    change.entity,
                    change.entity_pk
                ),
                Err(e) => return Err(crate::sql::query_err("applying a change", e)),
            }
            if change.entity == "observation" && change.op == "upsert" {
                report.echoed += tx
                    .execute(
                        "DELETE FROM outbox WHERE record_id = ?",
                        [&change.entity_pk],
                    )
                    .ctx("clearing an outbox entry")?;
            }
        }
        Self::set_cursor(&tx, CHANGES_SOURCE, pulled.changes_cursor, now)?;
        tx.commit().ctx("applying a pull")?;

        for entry in pulled.upstream {
            let appended = self.append_upstream_impl(&NewUpstream {
                api: entry.api,
                path: entry.path,
                etag: entry.etag,
                body: entry.body,
                fetched_at: entry.fetched_at,
                via: "pi".into(),
            })?;
            report.upstream += usize::from(appended.is_some());
        }
        let tx = self.conn.unchecked_transaction().ctx("applying a pull")?;
        Self::set_cursor(&tx, UPSTREAM_SOURCE, pulled.upstream_cursor, now)?;
        tx.commit().ctx("applying a pull")
    }
}

/// Apply one change. `Ok(false)` when it is not one this device can store.
///
/// The Pi is the source of truth, so its row replaces whatever this device
/// holds under the same key, and also any row that would break a unique
/// rule with it: a pick list naming the same team twice, or a second live
/// observation of a robot in a match by the same scout. Such a local row was
/// never on the Pi, and its outbox entry (if any) is refused when pushed.
fn apply_change(tx: &Transaction, change: &PulledChange) -> rusqlite::Result<bool> {
    let pk = change.entity_pk.as_str();
    if change.op == "delete" {
        match change.entity.as_str() {
            "observation" => {
                tx.execute("DELETE FROM observations WHERE client_record_id = ?", [pk])?;
            }
            "assignment" => {
                let Some((match_key, team)) = pk.rsplit_once(':') else {
                    return Ok(false);
                };
                tx.execute(
                    "DELETE FROM scout_assignments WHERE match_key = ? AND team_number = ?",
                    params![match_key, team.parse::<i64>().unwrap_or(-1)],
                )?;
            }
            "pick_list_entry" => {
                tx.execute(
                    "DELETE FROM pick_list_entries WHERE client_record_id = ?",
                    [pk],
                )?;
            }
            _ => return Ok(false),
        }
        return Ok(true);
    }
    let Some(JsonValue::Object(row)) = &change.row else {
        return Ok(false);
    };
    let f = Fields(row);
    match change.entity.as_str() {
        "observation" => upsert_observation(tx, &f),
        "assignment" => upsert_assignment(tx, &f),
        "pick_list_entry" => upsert_pick(tx, &f),
        _ => Ok(false),
    }
}

fn upsert_observation(tx: &Transaction, f: &Fields) -> rusqlite::Result<bool> {
    let (
        Some(id),
        Some(match_key),
        Some(team),
        Some(event_key),
        Some(alliance),
        Some(payload),
        Some(state),
        Some(observed_at),
        Some(updated_at),
    ) = (
        f.text("client_record_id"),
        f.text("match_key"),
        f.int("team_number"),
        f.text("event_key"),
        f.text("alliance"),
        f.json("payload"),
        f.text("review_state"),
        f.text("observed_at"),
        f.text("updated_at"),
    )
    else {
        return Ok(false);
    };
    let scouter = f.int("scouter_id");
    if state != "declined"
        && let Some(scouter) = scouter
    {
        tx.execute(
            "DELETE FROM observations WHERE match_key = ? AND team_number = ? \
             AND scouter_id = ? AND review_state <> 'declined' AND client_record_id <> ?",
            params![match_key, team, scouter, id],
        )?;
    }
    tx.execute(
        "INSERT INTO observations (client_record_id, match_key, team_number, event_key, \
             alliance, payload, schema_version, scouter_id, device_id, submitting_team, \
             review_state, review_note, reviewed_by, reviewed_at, observed_at, created_at, \
             updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?16) \
         ON CONFLICT (client_record_id) DO UPDATE SET match_key = ?2, team_number = ?3, \
             event_key = ?4, alliance = ?5, payload = ?6, schema_version = ?7, \
             scouter_id = ?8, device_id = ?9, submitting_team = ?10, review_state = ?11, \
             review_note = ?12, reviewed_by = ?13, reviewed_at = ?14, observed_at = ?15, \
             updated_at = ?16",
        params![
            id,
            match_key,
            team,
            event_key,
            alliance,
            payload,
            f.int("schema_version").unwrap_or(0),
            scouter,
            f.int("device_id"),
            f.int("submitting_team"),
            state,
            f.text("review_note"),
            f.int("reviewed_by"),
            f.text("reviewed_at"),
            observed_at,
            updated_at,
        ],
    )?;
    Ok(true)
}

fn upsert_assignment(tx: &Transaction, f: &Fields) -> rusqlite::Result<bool> {
    let (Some(match_key), Some(team), Some(event_key), Some(updated_at)) = (
        f.text("match_key"),
        f.int("team_number"),
        f.text("event_key"),
        f.text("updated_at"),
    ) else {
        return Ok(false);
    };
    let (scouter, device) = (f.int("scouter_id"), f.int("device_id"));
    if scouter.is_none() && device.is_none() {
        return Ok(false);
    }
    tx.execute(
        "INSERT INTO scout_assignments (match_key, team_number, event_key, scouter_id, \
             device_id, assigned_by, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7) \
         ON CONFLICT (match_key, team_number) DO UPDATE SET event_key = ?3, \
             scouter_id = ?4, device_id = ?5, assigned_by = ?6, updated_at = ?7",
        params![
            match_key,
            team,
            event_key,
            scouter,
            device,
            f.int("assigned_by"),
            updated_at
        ],
    )?;
    Ok(true)
}

fn upsert_pick(tx: &Transaction, f: &Fields) -> rusqlite::Result<bool> {
    let (Some(id), Some(owner), Some(event_key), Some(picked), Some(updated_at)) = (
        f.text("client_record_id"),
        f.int("owning_team"),
        f.text("event_key"),
        f.int("picked_team"),
        f.text("updated_at"),
    ) else {
        return Ok(false);
    };
    tx.execute(
        "DELETE FROM pick_list_entries WHERE owning_team = ? AND event_key = ? \
         AND picked_team = ? AND client_record_id <> ?",
        params![owner, event_key, picked, id],
    )?;
    tx.execute(
        "INSERT INTO pick_list_entries (client_record_id, owning_team, event_key, picked_team, \
             color, crossed, position, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8) \
         ON CONFLICT (client_record_id) DO UPDATE SET owning_team = ?2, event_key = ?3, \
             picked_team = ?4, color = ?5, crossed = ?6, position = ?7, updated_at = ?8",
        params![
            id,
            owner,
            event_key,
            picked,
            f.text("color"),
            f.int("crossed").unwrap_or(0),
            f.int("position").unwrap_or(0),
            updated_at
        ],
    )?;
    Ok(true)
}

// ── The browser's transport ─────────────────────────────────────────────────

/// `fetch`, on whatever global this runs in: a page, a worker, or the
/// service worker. Same-origin, so the session and device cookies go with
/// it, and redirects are not followed, so a lost session reads as one.
///
/// `bearer` is the device's offline token (C9, `static/js/token.js`), sent
/// as `Authorization: Bearer`, so a sync still goes as the scout once the
/// session has run out. A worker cannot read `localStorage`; whoever owns
/// the database is handed it by the page.
///
/// `timeout_ms` gives up on a request that long unanswered, as no answer: a
/// tablet off the Pi's network can otherwise wait on its address for a
/// minute or more.
#[cfg(target_arch = "wasm32")]
#[derive(Debug, Clone, Default)]
pub struct Fetch {
    pub bearer: Option<String>,
    pub timeout_ms: Option<u32>,
}

#[cfg(target_arch = "wasm32")]
impl Transport for Fetch {
    async fn get(&self, path: &str) -> std::result::Result<Reply, String> {
        fetch(self, path, "GET", None).await
    }

    async fn post_json(&self, path: &str, body: String) -> std::result::Result<Reply, String> {
        let body = (wasm_bindgen::JsValue::from(body), "application/json");
        fetch(self, path, "POST", Some(body)).await
    }

    async fn post_file(&self, path: &str, body: Vec<u8>) -> std::result::Result<Reply, String> {
        let body = js_sys::Uint8Array::from(body.as_slice()).into();
        fetch(self, path, "POST", Some((body, "application/vnd.sqlite3"))).await
    }
}

#[cfg(target_arch = "wasm32")]
async fn fetch(
    options: &Fetch,
    path: &str,
    method: &str,
    body: Option<(wasm_bindgen::JsValue, &str)>,
) -> std::result::Result<Reply, String> {
    use js_sys::{Object, Reflect};
    use wasm_bindgen::JsValue;

    use crate::opfs::{call, get};

    let set = |target: &JsValue, key: &str, value: JsValue| {
        Reflect::set(target, &key.into(), &value).map(|_| ())
    };
    let init: JsValue = Object::new().into();
    let built = (|| {
        set(&init, "method", method.into())?;
        set(&init, "credentials", "same-origin".into())?;
        set(&init, "redirect", "manual".into())?;
        set(&init, "cache", "no-store".into())?;
        let headers: JsValue = Object::new().into();
        if let Some(token) = &options.bearer {
            set(&headers, "authorization", format!("Bearer {token}").into())?;
        }
        if let Some((body, kind)) = body {
            set(&headers, "content-type", kind.into())?;
            set(&init, "body", body)?;
        }
        set(&init, "headers", headers)?;
        Ok::<(), JsValue>(())
    })();
    built.map_err(|e| format!("{e:?}"))?;

    let global = js_sys::global();
    // `AbortSignal.timeout`, where there is one; without it, the browser's
    // own patience.
    if let Some(ms) = options.timeout_ms
        && let Ok(signal) = get(&global, "AbortSignal")
        && let Ok(signal) = call(&signal, "timeout", &[ms.into()]).await
    {
        let _ = set(&init, "signal", signal);
    }
    // A rejected fetch is the network, not the server: offline.
    let response = call(&global, "fetch", &[path.into(), init])
        .await
        .map_err(|_| "no connection to the Pi".to_string())?;
    let status = get(&response, "status")
        .ok()
        .and_then(|s| s.as_f64())
        .unwrap_or(0.0) as u16;
    let body = call(&response, "text", &[])
        .await
        .ok()
        .and_then(|t| t.as_string())
        .unwrap_or_default();
    Ok(Reply { status, body })
}
