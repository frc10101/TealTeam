//! The dev checks (F13): every rule about who may see or do what, asserted
//! against the seeded event through the real router.
//!
//! Each check drives `startup::router` (the same app `serve` runs) with
//! `tower::ServiceExt::oneshot`, as one seeded user, and prints PASS or FAIL
//! with the user and the route. The rules come from the code and from
//! `docs/plan/REBUILD_SPEC.md` and `docs/plan/ACTION_ITEMS.md`; where a check cites a
//! rule, the comment says where it is stated. A FAIL is either a real hole or
//! a wrong expectation: find out which.
//!
//! The checks write (a push, a verdict, a sign-out), so they run on a copy of
//! the dev database made with `VACUUM INTO`, and the copy is deleted after.
//! The data being served is never touched by a check.
//!
//! To add a check: a line in [`ROUTES`] if it is a new route (a test fails
//! until every route in the router is listed), or a `check` call in one of
//! the sections of [`run_all`].

use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{HeaderMap, Request, header};
use chrono::Utc;
use serde_json::Value as Json;
use tower::ServiceExt;
use tt_core::review::ReviewState;
use tt_core::token::CREDENTIAL_SEPARATOR;
use tt_core::user::User;
use tt_repo::Repo;
use tt_repo_sqlite::SqliteRepo;

use super::seed::{self, NOTE_NONE, NOTE_OTHER, NOTE_OURS, OTHER, OURS};
use crate::startup::{AppState, router};
use crate::upstream::Upstream;

// ── The report ──────────────────────────────────────────────────────────────

pub struct Outcome {
    pub ok: bool,
    pub user: String,
    pub route: String,
    pub what: String,
    pub detail: String,
}

#[derive(Default)]
pub struct Report {
    pub outcomes: Vec<Outcome>,
}

impl Report {
    pub fn total(&self) -> usize {
        self.outcomes.len()
    }

    pub fn failed(&self) -> usize {
        self.outcomes.iter().filter(|o| !o.ok).count()
    }

    /// `full`: every line. Otherwise only the failures and the summary.
    pub fn render(&self, full: bool) -> String {
        let mut out = String::new();
        for o in &self.outcomes {
            if full || !o.ok {
                out.push_str(&format!(
                    "{}  {:<20} {:<44} {}{}\n",
                    if o.ok { "PASS" } else { "FAIL" },
                    o.user,
                    o.route,
                    o.what,
                    if o.ok || o.detail.is_empty() {
                        String::new()
                    } else {
                        format!("  [{}]", o.detail)
                    }
                ));
            }
        }
        out.push_str(&format!(
            "dev checks: {} passed, {} failed, {} total\n",
            self.total() - self.failed(),
            self.failed(),
            self.total()
        ));
        out
    }
}

// ── The harness ─────────────────────────────────────────────────────────────

struct Resp {
    status: u16,
    location: Option<String>,
    headers: HeaderMap,
    body: String,
    bytes: Vec<u8>,
}

impl Resp {
    fn json(&self) -> Json {
        serde_json::from_str(&self.body).unwrap_or(Json::Null)
    }
}

struct Harness {
    state: AppState,
    event: String,
    /// email -> `tt_session=..; tt_device=..`
    cookies: HashMap<&'static str, String>,
    ids: HashMap<&'static str, i64>,
    report: Report,
    scratch: std::path::PathBuf,
}

/// Which way a body goes.
enum Send<'a> {
    Nothing,
    Form(&'a str),
    Bytes(&'a [u8]),
}

fn device_of(index: usize) -> String {
    if index == 3 {
        return seed::TABLET.into();
    }
    format!("dev00000-0000-4000-8000-{:012}", 100 + index)
}

impl Harness {
    async fn send(&self, method: &str, uri: &str, who: Option<&str>, body: Send<'_>) -> Resp {
        self.send_with(method, uri, who, &[], body).await
    }

    async fn send_with(
        &self,
        method: &str,
        uri: &str,
        who: Option<&str>,
        extra: &[(&str, String)],
        body: Send<'_>,
    ) -> Resp {
        let mut req = Request::builder().method(method).uri(uri);
        if let Some(who) = who {
            req = req.header(header::COOKIE, &self.cookies[who]);
        }
        for (k, v) in extra {
            req = req.header(*k, v);
        }
        let body = match body {
            Send::Nothing => Body::empty(),
            Send::Form(f) => {
                req = req.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
                Body::from(f.to_string())
            }
            Send::Bytes(b) => {
                req = req.header(header::CONTENT_TYPE, "application/json");
                Body::from(b.to_vec())
            }
        };
        let response = router(self.state.clone())
            .oneshot(req.body(body).expect("a request"))
            .await
            .expect("the router answers");
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let location = headers
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("a body")
            .to_vec();
        Resp {
            status,
            location,
            headers,
            body: String::from_utf8_lossy(&bytes).into_owned(),
            bytes,
        }
    }

    /// As someone holding only this `Authorization: Bearer` credential, on
    /// the device `device` (a cookie), with no session.
    async fn bearer(
        &self,
        method: &str,
        uri: &str,
        credential: &str,
        device: Option<&str>,
        body: Send<'_>,
    ) -> Resp {
        let mut extra = vec![("authorization", format!("Bearer {credential}"))];
        if let Some(device) = device {
            extra.push(("cookie", format!("tt_device={device}")));
        }
        self.send_with(method, uri, None, &extra, body).await
    }

    fn check(&mut self, user: &str, route: &str, what: &str, ok: bool, detail: impl Into<String>) {
        self.report.outcomes.push(Outcome {
            ok,
            user: user.into(),
            route: route.into(),
            what: what.into(),
            detail: if ok { String::new() } else { detail.into() },
        });
    }

    fn id(&self, email: &str) -> i64 {
        self.ids[email]
    }

    async fn user(&self, email: &str) -> User {
        self.state
            .repo
            .user_by_id(self.id(email))
            .await
            .expect("storage")
            .expect("a dev user")
    }
}

fn label(who: Option<&str>) -> String {
    who.map_or("(signed out)".into(), |w| {
        w.trim_end_matches("@dev.test").to_string()
    })
}

// ── The route matrix ────────────────────────────────────────────────────────

/// Who may use a route, as its extractor says (`auth.rs`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Access {
    /// Anyone, signed in or not.
    Open,
    /// Anyone signed in; the signed-out go to sign-in.
    Auth,
    /// Lead scout or admin; other signed-in users are sent home.
    Lead,
    /// Drive coach or admin.
    Coach,
    /// Lead scout, coach, or admin: the pick list and `/ask`.
    Pick,
    Admin,
    /// A JSON sync route: the signed-out get 401, anyone signed in is served.
    Sync,
    /// Lead-only JSON: 401 signed out, 403 for anyone signed in who is not
    /// a lead.
    LeadJson,
    /// Not on the FTC build: 404 for everyone (`upstream::frc_only`).
    NotHere,
    /// Checked on its own, below.
    Own(&'static str),
}

use Access::*;

/// Every route in the router. `{id}` is filled in with a real one.
const ROUTES: &[(&str, &str, Access)] = &[
    ("GET", "/", Open),
    ("GET", "/sign-in", Own("the sign-in page")),
    ("GET", "/sign-up", Own("the sign-up page")),
    ("GET", "/account", Auth),
    ("GET", "/help", Open),
    (
        "GET",
        "/ca.crt",
        Own("the CA certificate: tls.rs; dev has no CA, so 404"),
    ),
    ("GET", "/submission", Auth),
    ("GET", "/submission/download", Auth),
    ("GET", "/teams", Auth),
    ("GET", "/notes", Auth),
    ("GET", "/graph", Auth),
    ("GET", "/compare", Auth),
    ("GET", "/lead-scout", Lead),
    ("GET", "/lead-scout/assignments", Lead),
    ("GET", "/lead-scout/submissions/{obs}", Lead),
    ("GET", "/lead-scout/refused/{refused}", Lead),
    ("GET", "/lead-scout/rankings", Lead),
    ("GET", "/lead-scout/rankings/enter", Lead),
    ("GET", "/drive-coach", Coach),
    ("GET", "/lead-scout/weights", Lead),
    ("GET", "/lead-scout/schedule", Lead),
    ("GET", "/lead-scout/scan", Lead),
    ("GET", "/pick-list", Pick),
    ("GET", "/ask", Pick),
    ("POST", "/ask", Pick),
    ("GET", "/admin/db", Admin),
    ("GET", "/admin/users", Admin),
    ("POST", "/api/auth/login", Own("sign-in")),
    ("POST", "/api/auth/signup", Own("sign-up")),
    ("POST", "/api/auth/logout", Own("sign-out")),
    ("POST", "/api/auth/token", Own("tokens")),
    ("POST", "/api/account/change-password", Auth),
    ("POST", "/api/submission", Auth),
    ("POST", "/api/device/heartbeat", Open),
    ("POST", "/api/assignments/match", Lead),
    ("POST", "/api/assignments/clear-match", Lead),
    ("POST", "/api/assignments/clear", Lead),
    ("POST", "/api/assignments/auto", Lead),
    ("POST", "/api/devices/{device}/rename", Lead),
    ("POST", "/api/users/{scout2}/sign-out", Lead),
    ("POST", "/api/admin/users/{scout2}", Admin),
    ("POST", "/api/admin/users/{scout2}/password", Admin),
    ("POST", "/api/weights", Lead),
    ("POST", "/api/rankings/manual", Lead),
    ("POST", "/api/schedule/import", Lead),
    ("POST", "/api/ftc/pull", Lead),
    ("POST", "/api/pick-list", Pick),
    ("GET", "/api/pick-list/doc", Pick),
    ("POST", "/api/pick-list/doc", Pick),
    ("GET", "/api/sync/pull", Sync),
    ("GET", "/api/sync/stream", Auth),
    ("GET", "/api/sync/snapshot", Auth),
    ("POST", "/api/sync/push", Own("pushes")),
    ("POST", "/api/sync/bundle", LeadJson),
    ("POST", "/api/weights/reset", Lead),
    ("POST", "/api/refused/{refused}/record", Lead),
    ("POST", "/api/refused/{refused}/dismiss", Lead),
    ("POST", "/api/handoff", Lead),
    ("POST", "/api/observations/{obs}/approve", Lead),
    ("POST", "/api/observations/{obs}/decline", Lead),
    ("POST", "/api/observations/{obs}/reopen", Lead),
    ("GET", "/sw.js", Open),
    ("GET", "/offline", Open),
    ("GET", "/health", Open),
    ("GET", "/status", Open),
    ("GET", "/static/css/site.css", Open),
    ("GET", "/api/upstream/key", NotHere),
    ("POST", "/api/frc/sync", NotHere),
];

/// A route in `ROUTES` form, as the router source writes it: `{id}` and
/// `{*path}` stay, the static glob becomes the one file listed.
fn router_paths() -> BTreeSet<String> {
    let source = include_str!("../startup.rs");
    let source = &source[..source.find("#[cfg(test)]").unwrap_or(source.len())];
    let mut paths = BTreeSet::new();
    for piece in source.split(".route(").skip(1) {
        let Some(open) = piece.find('"') else {
            continue;
        };
        let rest = &piece[open + 1..];
        if let Some(close) = rest.find('"') {
            paths.insert(rest[..close].to_string());
        }
    }
    paths
}

/// The path a `ROUTES` line stands for in the router's own spelling.
fn pattern(path: &str) -> String {
    let mut out = String::new();
    let mut rest = path;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let close = rest[open..].find('}').map_or(rest.len(), |c| open + c);
        out.push_str("{id}");
        rest = &rest[(close + 1).min(rest.len())..];
    }
    out.push_str(rest);
    out
}

/// Routes the router lists that `ROUTES` covers by another spelling.
fn normalise_router(path: &str) -> String {
    if path == "/static/{*path}" {
        return "/static/css/site.css".into();
    }
    pattern(path)
}

const MATRIX_USERS: &[Option<&str>] = &[
    None,
    Some("scout1@dev.test"),
    Some("coach@dev.test"),
    Some("lead@dev.test"),
    Some("admin@dev.test"),
    Some("noteam@dev.test"),
    Some("lead2@dev.test"),
    Some("noteamlead@dev.test"),
    Some("scoutb1@dev.test"),
];

/// What `access` makes of `who`: `200`, a redirect, or a status.
#[derive(Debug, PartialEq, Eq, Clone)]
enum Expect {
    Ok,
    To(&'static str),
    Status(u16),
}

fn expect(access: Access, who: Option<&str>) -> Expect {
    let roles = who.map(|w| seed::user(w).roles);
    let to_home = Expect::To("/");
    let to_sign_in = Expect::To("/sign-in");
    match (access, roles) {
        (Open, _) => Expect::Ok,
        (NotHere, _) => Expect::Status(404),
        (Auth, None) => to_sign_in,
        (Auth, Some(_)) => Expect::Ok,
        (Sync, None) => Expect::Status(401),
        (Sync, Some(_)) => Expect::Ok,
        (LeadJson, None) => Expect::Status(401),
        (LeadJson, Some(r)) => {
            if r.can_lead() {
                Expect::Ok
            } else {
                Expect::Status(403)
            }
        }
        (Lead, None) | (Coach, None) | (Pick, None) | (Admin, None) => to_sign_in,
        (Lead, Some(r)) => {
            if r.can_lead() {
                Expect::Ok
            } else {
                to_home
            }
        }
        (Coach, Some(r)) => {
            if r.can_coach() {
                Expect::Ok
            } else {
                to_home
            }
        }
        (Pick, Some(r)) => {
            if r.can_pick() {
                Expect::Ok
            } else {
                to_home
            }
        }
        (Admin, Some(r)) => {
            if r.can_admin() {
                Expect::Ok
            } else {
                to_home
            }
        }
        (Own(_), _) => Expect::Ok,
    }
}

// ── Fixtures the sections share ─────────────────────────────────────────────

struct Facts {
    /// A waiting observation by one of our scouts / the other team's.
    ours_waiting: i64,
    other_waiting: i64,
    /// Every observation id.
    observations: Vec<i64>,
    refused: Vec<i64>,
    device: i64,
    /// A robot both teams' scouts watched.
    shared_robot: i32,
}

async fn facts(h: &Harness) -> anyhow::Result<Facts> {
    let repo = &*h.state.repo;
    let mut observations = Vec::new();
    let mut id = 1;
    while let Some(o) = repo.observation(id).await? {
        observations.push(o.id);
        id += 1;
    }
    let pending = repo.pending_observations(&h.event).await?;
    let pick = |team: i32, state: ReviewState| -> anyhow::Result<i64> {
        let _ = state;
        pending
            .iter()
            .find(|o| o.submitting_team == Some(team))
            .map(|o| o.id)
            .ok_or_else(|| anyhow::anyhow!("no waiting observation by team {team}"))
    };
    let device = repo
        .device_by_uuid(seed::TABLET)
        .await?
        .map(|d| d.id)
        .ok_or_else(|| anyhow::anyhow!("no tablet"))?;
    let q1 = repo
        .match_by_key(&seed::qual_key(h.state.season.season, 1))
        .await?
        .ok_or_else(|| anyhow::anyhow!("no Q1"))?;
    Ok(Facts {
        ours_waiting: pick(OURS, ReviewState::Pending)?,
        other_waiting: pick(OTHER, ReviewState::Pending)?,
        observations,
        refused: repo.open_refusals().await?.iter().map(|r| r.id).collect(),
        device,
        shared_robot: q1.teams().next().expect("Q1 has robots"),
    })
}

fn fill(path: &str, h: &Harness, f: &Facts) -> String {
    path.replace("{obs}", &f.ours_waiting.to_string())
        .replace("{refused}", &f.refused[0].to_string())
        .replace("{device}", &f.device.to_string())
        .replace("{scout2}", &h.id("scout2@dev.test").to_string())
}

// ── Running them ────────────────────────────────────────────────────────────

/// Seed nothing; check `state`'s database. The checks work on a copy made in
/// `dir`, and nothing is left behind.
pub async fn run(state: &AppState, dir: &Path) -> Report {
    match run_all(state, dir).await {
        Ok(report) => report,
        Err(e) => {
            let mut report = Report::default();
            report.outcomes.push(Outcome {
                ok: false,
                user: "(setup)".into(),
                route: "-".into(),
                what: "the checks could not run".into(),
                detail: format!("{e:#}"),
            });
            report
        }
    }
}

async fn run_all(state: &AppState, dir: &Path) -> anyhow::Result<Report> {
    // A copy to write to.
    let scratch = dir.join(format!("check-{}.db", std::process::id()));
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", scratch.display()));
    }
    sqlx::query("VACUUM INTO ?")
        .bind(
            scratch
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("a utf-8 path"))?,
        )
        .execute(state.repo.pool())
        .await?;
    let repo = SqliteRepo::connect(&format!("sqlite://{}?mode=rwc", scratch.display()))?;
    let copy = AppState {
        repo: Arc::new(repo),
        season: state.season.clone(),
        upstream: Arc::new(Upstream::off()),
        snapshots: Default::default(),
        tokens: Default::default(),
    };

    let result = sections(copy, dir).await;
    copy_cleanup(&scratch);
    result
}

fn copy_cleanup(scratch: &Path) {
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", scratch.display()));
    }
}

async fn sections(state: AppState, dir: &Path) -> anyhow::Result<Report> {
    let now = Utc::now();
    let event = seed::event_key(state.season.season);

    // A session per account, and a device cookie.
    let mut cookies = HashMap::new();
    let mut ids = HashMap::new();
    for (index, u) in seed::USERS.iter().enumerate() {
        let credentials = state
            .repo
            .credentials_by_email(u.email)
            .await?
            .ok_or_else(|| anyhow::anyhow!("{} was not seeded", u.email))?;
        let session = crate::auth::new_session(credentials.user.id);
        state.repo.create_session(&session, now).await?;
        cookies.insert(
            u.email,
            format!("tt_session={}; tt_device={}", session.id, device_of(index)),
        );
        ids.insert(u.email, credentials.user.id);
    }
    let mut h = Harness {
        state,
        event,
        cookies,
        ids,
        report: Report::default(),
        scratch: dir.to_path_buf(),
    };
    let f = facts(&h).await?;

    routes_exist(&mut h);
    matrix(&mut h, &f).await;
    sign_in(&mut h).await;
    data_sharing(&mut h, &f).await;
    ask(&mut h).await;
    admin_viewer(&mut h).await;
    data_present(&mut h).await;
    sync_and_snapshot(&mut h).await?;
    stream(&mut h).await;
    tokens(&mut h).await?;
    writes(&mut h, &f).await?;
    upstream_off(&mut h).await;
    Ok(h.report)
}

// ── Section: every route is in the matrix ───────────────────────────────────

fn routes_exist(h: &mut Harness) {
    let listed: BTreeSet<(String, String)> = ROUTES
        .iter()
        .map(|(_, path, _)| ("".to_string(), pattern(path)))
        .collect();
    let listed: BTreeSet<String> = listed.into_iter().map(|(_, p)| p).collect();
    let in_router: BTreeSet<String> = router_paths().iter().map(|p| normalise_router(p)).collect();
    let missing: Vec<_> = in_router.difference(&listed).cloned().collect();
    let extra: Vec<_> = listed.difference(&in_router).cloned().collect();
    h.check(
        "(router)",
        "all routes",
        "every route in startup::router is in the matrix, and nothing else is",
        missing.is_empty() && extra.is_empty(),
        format!("not in the matrix: {missing:?}; not in the router: {extra:?}"),
    );
}

// ── Section: route x role ───────────────────────────────────────────────────

async fn matrix(h: &mut Harness, f: &Facts) {
    for (method, path, access) in ROUTES {
        if matches!(access, Own(_)) {
            continue;
        }
        let mut uri = fill(path, h, f);
        // The pick list's document needs its event named.
        if *path == "/api/pick-list/doc" {
            uri = format!("{uri}?event={}", h.event);
        }
        for who in MATRIX_USERS {
            let mut want = expect(*access, *who);
            // A pick list belongs to a team: with none there is no list.
            if *path == "/api/pick-list/doc"
                && want == Expect::Ok
                && who.is_some_and(|w| seed::user(w).team.is_none())
            {
                want = Expect::Status(403);
            }
            // A route the user may use is read, not run, when it writes.
            if want == Expect::Ok && *method == "POST" {
                continue;
            }
            // A stream stays open: the signed-in are checked in `sync`.
            if want == Expect::Ok && *path == "/api/sync/stream" {
                continue;
            }
            let r = match *method {
                "POST" => h.send("POST", &uri, *who, Send::Form("")).await,
                _ => h.send("GET", &uri, *who, Send::Nothing).await,
            };
            let ok = match &want {
                Expect::Ok => r.status == 200,
                Expect::To(to) => r.status == 303 && r.location.as_deref() == Some(*to),
                Expect::Status(s) => r.status == *s,
            };
            let said = match &want {
                Expect::Ok => "served (200)".to_string(),
                Expect::To(to) => format!("sent to {to} (303)"),
                Expect::Status(s) => format!("refused ({s})"),
            };
            h.check(
                &label(*who),
                &format!("{method} {path}"),
                &said,
                ok,
                format!("got {} {:?}", r.status, r.location),
            );
        }
    }
}

// ── Section: sign-in, sign-up, sign-out ─────────────────────────────────────

async fn sign_in(h: &mut Harness) {
    // A signed-in person is sent home from the sign-in and sign-up pages.
    for page in ["/sign-in", "/sign-up"] {
        let out = h.send("GET", page, None, Send::Nothing).await;
        h.check(
            "(signed out)",
            &format!("GET {page}"),
            "served (200)",
            out.status == 200,
            format!("{}", out.status),
        );
        let signed_in = h
            .send("GET", page, Some("scout1@dev.test"), Send::Nothing)
            .await;
        h.check(
            "scout1",
            &format!("GET {page}"),
            "sent home (303 to /)",
            signed_in.status == 303 && signed_in.location.as_deref() == Some("/"),
            format!("{} {:?}", signed_in.status, signed_in.location),
        );
    }

    // A real sign-in: right password sets a cookie that works over plain
    // http (no Secure); a wrong one does not.
    let good = h
        .send(
            "POST",
            "/api/auth/login",
            None,
            Send::Form("email=scout2%40dev.test&password=dev"),
        )
        .await;
    let set = good
        .headers
        .get(header::SET_COOKIE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    h.check(
        "scout2",
        "POST /api/auth/login",
        "the password dev signs in with a cookie that works over plain http",
        good.status == 303
            && set.starts_with("tt_session=")
            && !set.to_lowercase().contains("secure"),
        format!("{} {set}", good.status),
    );
    let cookie = set.split(';').next().unwrap_or_default().to_string();
    let account = h
        .send_with(
            "GET",
            "/account",
            None,
            &[("cookie", cookie)],
            Send::Nothing,
        )
        .await;
    h.check(
        "scout2",
        "GET /account",
        "the new session works",
        account.status == 200,
        format!("{}", account.status),
    );

    let bad = h
        .send(
            "POST",
            "/api/auth/login",
            None,
            Send::Form("email=scout2%40dev.test&password=wrong"),
        )
        .await;
    h.check(
        "scout2",
        "POST /api/auth/login",
        "a wrong password gets no cookie",
        bad.headers.get(header::SET_COOKIE).is_none() && bad.status == 200,
        format!("{}", bad.status),
    );

    // Signing out ends the session.
    let out = h
        .send(
            "POST",
            "/api/auth/logout",
            Some("noteam@dev.test"),
            Send::Form(""),
        )
        .await;
    let after = h
        .send("GET", "/account", Some("noteam@dev.test"), Send::Nothing)
        .await;
    h.check(
        "noteam",
        "POST /api/auth/logout",
        "signing out ends the session",
        out.status == 303 && after.status == 303 && after.location.as_deref() == Some("/sign-in"),
        format!("{} then {} {:?}", out.status, after.status, after.location),
    );
    // Put the session back for the sections after this one.
    if let Ok(Some(c)) = h.state.repo.credentials_by_email("noteam@dev.test").await {
        let session = crate::auth::new_session(c.user.id);
        let _ = h.state.repo.create_session(&session, Utc::now()).await;
        h.cookies.insert(
            "noteam@dev.test",
            format!("tt_session={}; tt_device={}", session.id, device_of(11)),
        );
    }
}

// ── Section: notes and who sees what ────────────────────────────────────────

/// The note markers a user may not see: every team's but their own, and the
/// teamless notes, which nobody reads (`Notes::for_viewer`, REBUILD_SPEC.md
/// 5.4: "a viewer with no team sees no notes at all").
fn forbidden(team: Option<i32>) -> Vec<&'static str> {
    let mut out = vec![NOTE_NONE];
    if team != Some(OURS) {
        out.push(NOTE_OURS);
    }
    if team != Some(OTHER) {
        out.push(NOTE_OTHER);
    }
    out
}

fn own_note(team: Option<i32>) -> Option<&'static str> {
    match team {
        Some(OURS) => Some(NOTE_OURS),
        Some(OTHER) => Some(NOTE_OTHER),
        _ => None,
    }
}

async fn data_sharing(h: &mut Harness, f: &Facts) {
    let robot = f.shared_robot;
    let readers = [
        "scout1@dev.test",
        "coach@dev.test",
        "admin@dev.test",
        "lead@dev.test",
        "scoutb1@dev.test",
        "lead2@dev.test",
        "noteam@dev.test",
        "noteamlead@dev.test",
    ];

    // U13: notes are read by the team that wrote them. /notes and the team
    // profile, for each reader.
    for who in readers {
        let team = seed::user(who).team;
        for page in [
            "/notes".to_string(),
            format!("/teams?team={robot}"),
            "/".to_string(),
            "/graph".to_string(),
            "/compare".to_string(),
            "/submission/download".to_string(),
        ] {
            let r = h.send("GET", &page, Some(who), Send::Nothing).await;
            let leaked: Vec<_> = forbidden(team)
                .into_iter()
                .filter(|m| r.body.contains(m))
                .collect();
            h.check(
                &label(Some(who)),
                &format!("GET {page}"),
                "shows no other team's notes (U13)",
                r.status == 200 && leaked.is_empty(),
                format!("{} leaked {leaked:?}", r.status),
            );
        }
        // They do read their own.
        if let Some(own) = own_note(team) {
            let r = h.send("GET", "/notes", Some(who), Send::Nothing).await;
            h.check(
                &label(Some(who)),
                "GET /notes",
                "lists their own team's notes",
                r.body.contains(own),
                "their own note is missing",
            );
        }
    }

    // The review page: a lead of another team, or of none, sees the counts and
    // choices of an observation but not its notes (ACTION_ITEMS.md, U13).
    for who in [
        "lead@dev.test",
        "lead2@dev.test",
        "noteamlead@dev.test",
        "admin@dev.test",
    ] {
        let team = seed::user(who).team;
        for id in &f.observations {
            let r = h
                .send(
                    "GET",
                    &format!("/lead-scout/submissions/{id}"),
                    Some(who),
                    Send::Nothing,
                )
                .await;
            let leaked: Vec<_> = forbidden(team)
                .into_iter()
                .filter(|m| r.body.contains(m))
                .collect();
            if !leaked.is_empty() || r.status != 200 {
                h.check(
                    &label(Some(who)),
                    &format!("GET /lead-scout/submissions/{id}"),
                    "shows no other team's notes",
                    false,
                    format!("{} leaked {leaked:?}", r.status),
                );
            }
        }
        h.check(
            &label(Some(who)),
            "GET /lead-scout/submissions/{id}",
            &format!(
                "none of {} observations shows another team's notes",
                f.observations.len()
            ),
            true,
            "",
        );
    }
    let hidden = h
        .send(
            "GET",
            &format!("/lead-scout/submissions/{}", f.ours_waiting),
            Some("lead2@dev.test"),
            Send::Nothing,
        )
        .await;
    h.check(
        "lead2",
        "GET /lead-scout/submissions/{id}",
        "says whose notes are held back",
        hidden
            .body
            .contains("Only scouts on team 10101 can read these notes."),
        "the held-back line is missing",
    );
    let theirs = h
        .send(
            "GET",
            &format!("/lead-scout/submissions/{}", f.other_waiting),
            Some("lead@dev.test"),
            Send::Nothing,
        )
        .await;
    h.check(
        "lead",
        "GET /lead-scout/submissions/{id}",
        "holds back another team's notes and says whose",
        theirs
            .body
            .contains("Only scouts on team 10202 can read these notes.")
            && !theirs.body.contains(NOTE_OTHER),
        "the other team's notes are shown, or the line is missing",
    );
    let own = h
        .send(
            "GET",
            &format!("/lead-scout/submissions/{}", f.ours_waiting),
            Some("lead@dev.test"),
            Send::Nothing,
        )
        .await;
    h.check(
        "lead",
        "GET /lead-scout/submissions/{id}",
        "shows their own team's notes",
        own.body.contains(NOTE_OURS),
        "own notes missing",
    );
    // Same for a refused entry (C10).
    for who in ["lead@dev.test", "lead2@dev.test", "noteamlead@dev.test"] {
        let team = seed::user(who).team;
        for id in &f.refused {
            let r = h
                .send(
                    "GET",
                    &format!("/lead-scout/refused/{id}"),
                    Some(who),
                    Send::Nothing,
                )
                .await;
            let leaked: Vec<_> = forbidden(team)
                .into_iter()
                .filter(|m| r.body.contains(m))
                .collect();
            h.check(
                &label(Some(who)),
                &format!("GET /lead-scout/refused/{id}"),
                "shows no other team's notes",
                r.status == 200 && leaked.is_empty(),
                format!("{} leaked {leaked:?}", r.status),
            );
        }
    }

    // A sweep: every page a lead may open, for every lead, never carries
    // another team's notes.
    for who in [
        "lead@dev.test",
        "lead2@dev.test",
        "noteamlead@dev.test",
        "admin@dev.test",
        "coach@dev.test",
    ] {
        let team = seed::user(who).team;
        for page in [
            "/lead-scout",
            "/lead-scout/assignments",
            "/lead-scout/rankings",
            "/lead-scout/schedule",
            "/drive-coach",
            "/pick-list",
            "/submission",
        ] {
            let r = h.send("GET", page, Some(who), Send::Nothing).await;
            if r.status != 200 {
                continue;
            }
            let leaked: Vec<_> = forbidden(team)
                .into_iter()
                .filter(|m| r.body.contains(m))
                .collect();
            h.check(
                &label(Some(who)),
                &format!("GET {page}"),
                "shows no other team's notes",
                leaked.is_empty(),
                format!("leaked {leaked:?}"),
            );
        }
    }

    // A scout is told of their own declines, and no one else's.
    let reasons = [
        ("scout2@dev.test", Some(seed::DECLINE_OURS)),
        ("scout1@dev.test", None),
        ("scout3@dev.test", None),
        ("scoutb2@dev.test", Some(seed::DECLINE_OTHER)),
        ("scoutb1@dev.test", None),
    ];
    for (who, sees) in reasons {
        let r = h.send("GET", "/submission", Some(who), Send::Nothing).await;
        let others: Vec<_> = [seed::DECLINE_OURS, seed::DECLINE_OTHER]
            .into_iter()
            .filter(|m| Some(*m) != sees && r.body.contains(m))
            .collect();
        let own_ok = sees.is_none_or(|m| r.body.contains(m));
        h.check(
            &label(Some(who)),
            "GET /submission",
            "a scout reads only their own declines",
            r.status == 200 && others.is_empty() && own_ok,
            format!("leaked {others:?}; own shown: {own_ok}"),
        );
    }
    // Decline reasons are written for the scout: not on any other scout's
    // pages or the shared ones.
    for who in [
        "scout1@dev.test",
        "scoutb1@dev.test",
        "coach@dev.test",
        "noteam@dev.test",
    ] {
        for page in [
            "/notes",
            "/",
            "/graph",
            "/compare",
            &format!("/teams?team={robot}"),
        ] {
            let r = h.send("GET", page, Some(who), Send::Nothing).await;
            h.check(
                &label(Some(who)),
                &format!("GET {page}"),
                "carries no decline reason",
                !r.body.contains("DEVDECLINE"),
                "a decline reason is on a shared page",
            );
        }
    }
}

// ── Section: the pages have data in them ───────────────────────────────────

async fn data_present(h: &mut Harness) {
    let e = h.event.clone();
    let pages: [(&str, String, &str, &[&str]); 7] = [
        (
            "lead@dev.test",
            format!("/lead-scout/rankings?event={e}"),
            "the rankings page lists teams with FTC ranks (F7)",
            &["Velocity Voles", "Rank comes from FTC Events"],
        ),
        (
            "coach@dev.test",
            format!("/drive-coach?event={e}"),
            "the drive coach page has our matches with OPR and DPR (F10)",
            &["OPR", "DPR", "Played, latest first"],
        ),
        (
            "coach@dev.test",
            format!("/teams?team=10003&event={e}"),
            "a team page has rank, record and OPR, DPR, CCWM",
            &["Record", "CCWM", "OPR"],
        ),
        (
            "coach@dev.test",
            format!("/graph?team=10003&metric=opr&event={e}"),
            "the graph offers OPR from the FTC results",
            &["OPR (FTC results"],
        ),
        (
            "lead@dev.test",
            format!("/lead-scout/assignments?event={e}"),
            "the assignment page has the coverage table (F9)",
            &["Covered"],
        ),
        (
            "lead@dev.test",
            format!("/lead-scout/schedule?event={e}"),
            "the schedule page holds the 21 imported matches",
            &["21,"],
        ),
        (
            "lead@dev.test",
            format!("/lead-scout?event={e}"),
            "the lead's page shows the refused entries waiting (C10)",
            &["Refused when sent"],
        ),
    ];
    for (who, uri, what, needles) in pages {
        let r = h.send("GET", &uri, Some(who), Send::Nothing).await;
        let missing: Vec<_> = needles.iter().filter(|n| !r.body.contains(**n)).collect();
        h.check(
            &label(Some(who)),
            &format!("GET {}", uri.split('?').next().unwrap_or(&uri)),
            what,
            r.status == 200 && missing.is_empty(),
            format!("{} missing {missing:?}", r.status),
        );
    }
}

// ── Section: /ask ───────────────────────────────────────────────────────────

async fn ask(h: &mut Harness) {
    // With the assistant off (ASK_BACKEND is never read in dev), the page
    // says so, and nothing else changes.
    for who in ["lead@dev.test", "coach@dev.test", "admin@dev.test"] {
        let r = h.send("GET", "/ask", Some(who), Send::Nothing).await;
        h.check(
            &label(Some(who)),
            "GET /ask",
            "says the assistant is not set up",
            r.status == 200 && r.body.contains("not set up"),
            format!("{}", r.status),
        );
        let q = h
            .send(
                "POST",
                "/ask",
                Some(who),
                Send::Form("question=Who+is+best%3F"),
            )
            .await;
        h.check(
            &label(Some(who)),
            "POST /ask",
            "a question gets no answer and no model call with the backend off",
            q.status == 200 && q.body.contains("not set up") && !q.body.contains("DEVNOTE"),
            format!("{}", q.status),
        );
    }

    // The sheet a model would be given, per user (the sheet builder): our
    // approved observations and notes for our team, never another team's,
    // never a waiting or declined one's.
    for who in [
        "lead@dev.test",
        "coach@dev.test",
        "admin@dev.test",
        "lead2@dev.test",
        "noteamlead@dev.test",
    ] {
        let user = h.user(who).await;
        let event = h
            .state
            .repo
            .event(&h.event)
            .await
            .ok()
            .flatten()
            .expect("the dev event");
        let sheet = crate::ask::build_sheet(&h.state, &user, &event, "").await;
        let Ok(sheet) = sheet else {
            h.check(
                &label(Some(who)),
                "ask sheet",
                "the sheet builds",
                false,
                "storage error",
            );
            continue;
        };
        let leaked: Vec<_> = forbidden(user.team_number)
            .into_iter()
            .filter(|m| sheet.text.contains(m))
            .collect();
        h.check(
            &label(Some(who)),
            "ask sheet",
            "has no other team's notes",
            leaked.is_empty(),
            format!("leaked {leaked:?}"),
        );
        // Waiting, declined and undone observations of the viewer's own team
        // are not in it either: only approved ones (SCOUTING_ASSISTANT.md).
        let own = own_note(user.team_number);
        if let Some(own) = own {
            let counted = sheet.text.contains(own);
            // Which of the seeded notes are not approved: ours, Q7-Q8 waiting,
            // Q3 slot 2 declined, Q5 slot 3 undone; theirs, Q5-Q6 waiting,
            // Q2 slot 2 declined.
            let unapproved: &[&str] = if own == NOTE_OURS {
                &["Q7 robot", "Q8 robot", "Q3 robot 2", "Q5 robot 3"]
            } else {
                &["Q5 robot", "Q6 robot", "Q2 robot 2"]
            };
            let not_approved = unapproved
                .iter()
                .any(|n| sheet.text.contains(&format!("{own}: {n}")));
            h.check(
                &label(Some(who)),
                "ask sheet",
                "holds the team's approved notes and not its waiting or declined ones",
                counted && !not_approved,
                format!("has own approved: {counted}; has unapproved: {not_approved}"),
            );
        } else {
            h.check(
                &label(Some(who)),
                "ask sheet",
                "a user with no team gets no notes at all",
                !sheet.text.contains("DEVNOTE"),
                "notes in a teamless sheet",
            );
        }
    }
    // And the one rule beside them: a scout cannot reach it (the matrix has
    // GET and POST /ask for every user; here, that nothing is rendered).
    let r = h
        .send("GET", "/ask", Some("scout1@dev.test"), Send::Nothing)
        .await;
    h.check(
        "scout1",
        "GET /ask",
        "a scout is sent home, not shown the page",
        r.status == 303 && !r.body.contains("assistant"),
        format!("{}", r.status),
    );
}

// ── Section: the admin database viewer ──────────────────────────────────────

async fn admin_viewer(h: &mut Harness) {
    let listing = h
        .send("GET", "/admin/db", Some("admin@dev.test"), Send::Nothing)
        .await;
    for hidden in ["sessions", "token_key"] {
        h.check(
            "admin",
            "GET /admin/db",
            &format!("the table list leaves out {hidden} (U17)"),
            !listing.body.contains(&format!("table={hidden}\"")),
            "listed",
        );
        let r = h
            .send(
                "GET",
                &format!("/admin/db?table={hidden}"),
                Some("admin@dev.test"),
                Send::Nothing,
            )
            .await;
        let session = h.cookies["admin@dev.test"]
            .split(';')
            .next()
            .and_then(|c| c.strip_prefix("tt_session="))
            .unwrap_or("-")
            .to_string();
        h.check(
            "admin",
            &format!("GET /admin/db?table={hidden}"),
            "asking for it by name shows nothing",
            !r.body.contains(&session) && !r.body.contains("<td>"),
            "rows shown",
        );
    }
    let users = h
        .send(
            "GET",
            "/admin/db?table=users",
            Some("admin@dev.test"),
            Send::Nothing,
        )
        .await;
    h.check(
        "admin",
        "GET /admin/db?table=users",
        "email and password hash are masked",
        users.status == 200
            && !users.body.contains("@dev.test")
            && !users.body.contains("$argon2")
            && users.body.contains("••••••"),
        "an email or hash is shown",
    );
}

// ── Section: sync pull, stream and snapshot ─────────────────────────────────

/// Changes are served only once they are two seconds old (`sync::LAG`).
async fn wait_for_changes(h: &Harness) {
    for _ in 0..40 {
        let (heads, _) = h.state.repo.log_heads().await.unwrap_or((0, 0));
        let visible = h
            .state
            .repo
            .changes_since(0, 100_000, Utc::now() - crate::sync::LAG)
            .await
            .map(|c| c.last().map_or(0, |c| c.seq))
            .unwrap_or(0);
        if visible >= heads {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
}

async fn sync_and_snapshot(h: &mut Harness) -> anyhow::Result<()> {
    wait_for_changes(h).await;

    for who in [
        "scout1@dev.test",
        "scoutb1@dev.test",
        "noteam@dev.test",
        "coach@dev.test",
    ] {
        let team = seed::user(who).team;
        let r = h
            .send("GET", "/api/sync/pull", Some(who), Send::Nothing)
            .await;
        let body = r.json();
        let changes = body["changes"].as_array().cloned().unwrap_or_default();
        h.check(
            &label(Some(who)),
            "GET /api/sync/pull",
            "carries the event's changes",
            r.status == 200 && !changes.is_empty(),
            format!("{} with {} changes", r.status, changes.len()),
        );
        // Only the replicated tables' entities (replication.rs): never a
        // session, a token key, a user, a device or a refused entry.
        let entities: BTreeSet<String> = changes
            .iter()
            .filter_map(|c| c["entity"].as_str().map(str::to_owned))
            .collect();
        let allowed: BTreeSet<&str> = ["observation", "assignment", "pick_list_entry"].into();
        let stray: Vec<_> = entities
            .iter()
            .filter(|e| !allowed.contains(e.as_str()))
            .collect();
        h.check(
            &label(Some(who)),
            "GET /api/sync/pull",
            "carries only observations, assignments and pick list entries",
            stray.is_empty(),
            format!("also {stray:?}"),
        );
        let leaked: Vec<_> = forbidden(team)
            .into_iter()
            .filter(|m| r.body.contains(m))
            .collect();
        h.check(
            &label(Some(who)),
            "GET /api/sync/pull",
            "carries no other team's notes (S3, U13)",
            leaked.is_empty(),
            format!("leaked {leaked:?}"),
        );
        h.check(
            &label(Some(who)),
            "GET /api/sync/pull",
            "carries no password hash, email or session",
            !r.body.contains("argon2")
                && !r.body.contains("@dev.test")
                && !r.body.contains("tt_session"),
            "a secret is in the pull",
        );
        // A pick list goes to its own team only.
        let foreign_picks: Vec<_> = changes
            .iter()
            .filter(|c| c["entity"] == "pick_list_entry")
            .filter(|c| {
                let owner = c["row"]["owning_team"].as_i64();
                owner.is_none() || owner.map(|o| Some(o as i32) != team).unwrap_or(true)
            })
            .collect();
        h.check(
            &label(Some(who)),
            "GET /api/sync/pull",
            "carries only its own team's pick list (S3)",
            foreign_picks.is_empty(),
            format!("{} foreign or unreadable pick rows", foreign_picks.len()),
        );
        if let Some(own) = own_note(team) {
            h.check(
                &label(Some(who)),
                "GET /api/sync/pull",
                "carries its own team's notes",
                r.body.contains(own),
                "own notes missing",
            );
        }
    }

    // The snapshot: the same cuts, as a SQLite file.
    for who in ["scout1@dev.test", "scoutb1@dev.test", "noteam@dev.test"] {
        let team = seed::user(who).team;
        let r = h
            .send("GET", "/api/sync/snapshot", Some(who), Send::Nothing)
            .await;
        h.check(
            &label(Some(who)),
            "GET /api/sync/snapshot",
            "is a SQLite file",
            r.status == 200 && r.bytes.starts_with(b"SQLite format 3"),
            format!("{}", r.status),
        );
        let path = h.scratch.join(format!(
            "tt-dev-snapshot-{}-{}.db",
            std::process::id(),
            label(Some(who))
        ));
        std::fs::write(&path, &r.bytes)?;
        let counts = snapshot_counts(&path, team).await;
        let _ = std::fs::remove_file(&path);
        match counts {
            Ok(rows) => {
                for (what, n, want_zero) in rows {
                    h.check(
                        &label(Some(who)),
                        "GET /api/sync/snapshot",
                        &what,
                        (n == 0) == want_zero,
                        format!("{n} rows"),
                    );
                }
            }
            Err(e) => h.check(
                &label(Some(who)),
                "GET /api/sync/snapshot",
                "can be read",
                false,
                format!("{e}"),
            ),
        }
    }
    Ok(())
}

/// Questions put to a snapshot, as `(what it says, rows, true if it wants 0)`.
async fn snapshot_counts(
    path: &Path,
    team: Option<i32>,
) -> anyhow::Result<Vec<(String, i64, bool)>> {
    use sqlx::Connection;
    let mut conn =
        sqlx::SqliteConnection::connect(&format!("sqlite://{}?mode=ro", path.display())).await?;
    let mut out = Vec::new();
    for table in [
        "sessions",
        "token_key",
        "token_epochs",
        "devices",
        "refused_entries",
        "pick_list_docs",
        "handoff_scans",
    ] {
        let n: i64 =
            sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT COUNT(*) FROM {table}")))
                .fetch_one(&mut conn)
                .await?;
        out.push((
            format!("has an empty {table} table (never replicated)"),
            n,
            true,
        ));
    }
    let n: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM users WHERE email LIKE '%@%' OR password_hash LIKE '$argon2%'",
    )
    .fetch_one(&mut conn)
    .await?;
    out.push(("has no email or password hash in users".into(), n, true));
    for marker in forbidden(team) {
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM observations WHERE payload LIKE ?")
            .bind(format!("%{marker}%"))
            .fetch_one(&mut conn)
            .await?;
        out.push((format!("has no {marker} note"), n, true));
    }
    if let Some(own) = own_note(team) {
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM observations WHERE payload LIKE ?")
            .bind(format!("%{own}%"))
            .fetch_one(&mut conn)
            .await?;
        out.push((format!("keeps its own {own} notes"), n, false));
    }
    let foreign: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM pick_list_entries WHERE owning_team IS NOT ?")
            .bind(team)
            .fetch_one(&mut conn)
            .await?;
    out.push(("has only its own team's pick list".into(), foreign, true));
    Ok(out)
}

/// A few seconds of the live stream (S8), as `(status, text)`.
async fn read_stream(state: &AppState, event: &str, cookie: &str) -> (u16, String) {
    use futures_util::StreamExt;
    let request = Request::builder()
        .uri(format!("/api/sync/stream?event={event}"))
        .header(header::COOKIE, cookie)
        .body(Body::empty())
        .expect("a request");
    let response = router(state.clone())
        .oneshot(request)
        .await
        .expect("the router answers");
    let status = response.status().as_u16();
    let mut frames = response.into_body().into_data_stream();
    let mut seen = Vec::new();
    let _ = tokio::time::timeout(std::time::Duration::from_millis(2500), async {
        while let Some(Ok(chunk)) = frames.next().await {
            seen.extend_from_slice(&chunk);
        }
    })
    .await;
    (status, String::from_utf8_lossy(&seen).into_owned())
}

/// The live stream carries what the pull does.
async fn stream(h: &mut Harness) {
    let (a, b, c) = (
        h.cookies["scout1@dev.test"].clone(),
        h.cookies["scoutb1@dev.test"].clone(),
        h.cookies["noteam@dev.test"].clone(),
    );
    let (ra, rb, rc) = tokio::join!(
        read_stream(&h.state, &h.event, &a),
        read_stream(&h.state, &h.event, &b),
        read_stream(&h.state, &h.event, &c)
    );
    for (who, (status, text)) in [
        ("scout1@dev.test", ra),
        ("scoutb1@dev.test", rb),
        ("noteam@dev.test", rc),
    ] {
        let team = seed::user(who).team;
        let leaked: Vec<_> = forbidden(team)
            .into_iter()
            .filter(|m| text.contains(m))
            .collect();
        h.check(
            &label(Some(who)),
            "GET /api/sync/stream",
            "streams the event's changes with no other team's notes (S8)",
            status == 200 && text.contains("observation") && leaked.is_empty(),
            format!("{status}, {} bytes, leaked {leaked:?}", text.len()),
        );
    }
}

// ── Section: offline tokens (T1) ────────────────────────────────────────────

fn credential(issued: &Json) -> String {
    format!(
        "{}{CREDENTIAL_SEPARATOR}{}",
        issued["token"].as_str().unwrap_or_default(),
        issued["key"].as_str().unwrap_or_default()
    )
}

fn push_body(entries: &[(&str, &str, i32)], season_version: i64) -> Vec<u8> {
    let observations: Vec<Json> = entries
        .iter()
        .map(|(id, match_key, team)| {
            serde_json::json!({
                "client_record_id": id,
                "match_key": match_key,
                "team_number": team,
                "payload": { "robot_status": "played", "auto_leave": true },
                "schema_version": season_version,
                "observed_at": Utc::now(),
            })
        })
        .collect();
    serde_json::to_vec(&serde_json::json!({
        "observations": observations,
        "version": tt_core::outbox::OUTBOX_VERSION,
    }))
    .expect("a push")
}

async fn tokens(h: &mut Harness) -> anyhow::Result<()> {
    let device = seed::TABLET;
    let issued = h
        .send(
            "POST",
            "/api/auth/token",
            Some("scout1@dev.test"),
            Send::Nothing,
        )
        .await;
    h.check(
        "scout1",
        "POST /api/auth/token",
        "a signed-in scout is issued a token and its device key",
        issued.status == 200 && issued.json()["key"].is_string(),
        format!("{}", issued.status),
    );
    let signed_out = h.send("POST", "/api/auth/token", None, Send::Nothing).await;
    h.check(
        "(signed out)",
        "POST /api/auth/token",
        "no token without a session",
        signed_out.status == 303 && signed_out.location.as_deref() == Some("/sign-in"),
        format!("{}", signed_out.status),
    );
    let good = credential(&issued.json());
    let token_only = good
        .split(CREDENTIAL_SEPARATOR)
        .next()
        .unwrap_or_default()
        .to_string();

    let pull = h
        .bearer("GET", "/api/sync/pull", &good, Some(device), Send::Nothing)
        .await;
    h.check(
        "scout1 (token)",
        "GET /api/sync/pull",
        "token and key on its own tablet are taken",
        pull.status == 200,
        format!("{}", pull.status),
    );
    let no_cookie = h
        .bearer("GET", "/api/sync/pull", &good, None, Send::Nothing)
        .await;
    h.check(
        "scout1 (token)",
        "GET /api/sync/pull",
        "with no device cookie (the downloaded page, C15) it is taken as the tablet it names",
        no_cookie.status == 200,
        format!("{}", no_cookie.status),
    );
    let alone = h
        .bearer(
            "GET",
            "/api/sync/pull",
            &token_only,
            Some(device),
            Send::Nothing,
        )
        .await;
    h.check(
        "scout1 (token)",
        "GET /api/sync/pull",
        "a token without its device key is refused (T1)",
        alone.status == 401,
        format!("{}", alone.status),
    );
    let other_device = h
        .bearer(
            "GET",
            "/api/sync/pull",
            &good,
            Some("dev00000-0000-4000-8000-0000000000ff"),
            Send::Nothing,
        )
        .await;
    h.check(
        "scout1 (token)",
        "GET /api/sync/pull",
        "from another tablet it is refused",
        other_device.status == 401,
        format!("{}", other_device.status),
    );

    // Another scout's key does not open this token.
    let lead_issued = h
        .send(
            "POST",
            "/api/auth/token",
            Some("lead@dev.test"),
            Send::Nothing,
        )
        .await;
    let lead_key = lead_issued.json()["key"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let swapped = format!("{token_only}{CREDENTIAL_SEPARATOR}{lead_key}");
    let wrong_key = h
        .bearer(
            "GET",
            "/api/sync/pull",
            &swapped,
            Some(device),
            Send::Nothing,
        )
        .await;
    h.check(
        "scout1 (token)",
        "GET /api/sync/pull",
        "with another holder's key it is refused",
        wrong_key.status == 401,
        format!("{}", wrong_key.status),
    );

    // A token signs no one in to a page.
    let page = h
        .bearer("GET", "/account", &good, Some(device), Send::Nothing)
        .await;
    h.check(
        "scout1 (token)",
        "GET /account",
        "a token is no sign-in for pages",
        page.status == 303,
        format!("{}", page.status),
    );
    let lead_page = h
        .bearer("GET", "/lead-scout", &good, Some(device), Send::Nothing)
        .await;
    h.check(
        "scout1 (token)",
        "GET /lead-scout",
        "a scout's token opens no lead page",
        lead_page.status == 303,
        format!("{}", lead_page.status),
    );

    // A scout's token cannot be used for the lead's bundle push or key.
    let bundle = h
        .bearer(
            "POST",
            "/api/sync/bundle",
            &good,
            Some(device),
            Send::Nothing,
        )
        .await;
    h.check(
        "scout1 (token)",
        "POST /api/sync/bundle",
        "a scout's token cannot push a bundle",
        bundle.status == 403,
        format!("{}", bundle.status),
    );

    // A push by token records as that scout.
    let q13 = seed::qual_key(h.state.season.season, 13);
    let record = h.state.repo.match_by_key(&q13).await?.expect("Q13");
    let robot = record.teams().next().expect("a robot");
    let body = push_body(
        &[("0191f7ac-1234-7000-8000-00000000ff01", &q13, robot)],
        h.state.season.version,
    );
    let pushed = h
        .bearer(
            "POST",
            "/api/sync/push",
            &good,
            Some(device),
            Send::Bytes(&body),
        )
        .await;
    let outcome = pushed.json()["receipts"][0]["outcome"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    h.check(
        "scout1 (token)",
        "POST /api/sync/push",
        "a push with a good token is recorded",
        pushed.status == 200 && outcome == "recorded",
        format!("{} {}", pushed.status, pushed.body),
    );
    let stored = h
        .state
        .repo
        .recorded_by(&h.event, h.id("scout1@dev.test"))
        .await?;
    h.check(
        "scout1 (token)",
        "POST /api/sync/push",
        "and it is recorded as that scout",
        stored.contains(&(q13.clone(), robot)),
        "not recorded under scout1",
    );
    let refused_push = h
        .bearer(
            "POST",
            "/api/sync/push",
            &token_only,
            Some(device),
            Send::Bytes(&body),
        )
        .await;
    h.check(
        "scout1 (token)",
        "POST /api/sync/push",
        "a push without the key is refused",
        refused_push.status == 401,
        format!("{}", refused_push.status),
    );

    // Sid was signed out everywhere: a token from before is refused, and a
    // fresh one issued after signing back in works.
    let sid = h.user("signedout@dev.test").await;
    let seed_bytes = h.state.repo.token_seed([0u8; 32], Utc::now()).await?;
    let old_claims = tt_core::token::Claims::new(&sid, device, 0, Utc::now());
    let old_token = tt_core::token::sign(&seed_bytes, &old_claims);
    let old_key = tt_core::token::device_key(&seed_bytes, &tt_core::token::Holder::of(&old_claims));
    let old = tt_core::token::credential(&old_token, &old_key);
    for (route, method) in [("/api/sync/pull", "GET"), ("/api/sync/push", "POST")] {
        let r = h
            .bearer(
                method,
                route,
                &old,
                Some(device),
                if method == "POST" {
                    Send::Bytes(&body)
                } else {
                    Send::Nothing
                },
            )
            .await;
        h.check(
            "signedout (token)",
            &format!("{method} {route}"),
            "a token from before they were signed out everywhere is refused",
            r.status == 401,
            format!("{}", r.status),
        );
    }
    let epoch = h.state.repo.token_epoch(sid.id).await?;
    h.check(
        "signedout",
        "token epoch",
        "was bumped when they were signed out",
        epoch >= 1,
        format!("epoch {epoch}"),
    );
    let fresh = h
        .send(
            "POST",
            "/api/auth/token",
            Some("signedout@dev.test"),
            Send::Nothing,
        )
        .await;
    let fresh = credential(&fresh.json());
    let r = h
        .bearer(
            "GET",
            "/api/sync/pull",
            &fresh,
            Some(&device_of(7)),
            Send::Nothing,
        )
        .await;
    h.check(
        "signedout (token)",
        "GET /api/sync/pull",
        "a token issued after is accepted",
        r.status == 200,
        format!("{}", r.status),
    );
    Ok(())
}

// ── Section: writes ─────────────────────────────────────────────────────────

async fn writes(h: &mut Harness, f: &Facts) -> anyhow::Result<()> {
    let season = h.state.season.season;
    let version = h.state.season.version;
    let q14 = seed::qual_key(season, 14);
    let robot = h
        .state
        .repo
        .match_by_key(&q14)
        .await?
        .expect("Q14")
        .teams()
        .next()
        .expect("a robot");

    // A push answers each entry by the form's rules (push::record_as).
    let body = push_body(
        &[
            ("0191f7ac-1234-7000-8000-00000000ee01", &q14, 99_998),
            (
                "0191f7ac-1234-7000-8000-00000000ee02",
                &seed::qual_key(season, 98),
                robot,
            ),
            ("0191f7ac-1234-7000-8000-00000000ee03", &q14, robot),
            ("0191f7ac-1234-7000-8000-00000000ee04", &q14, robot),
        ],
        version,
    );
    let r = h
        .send(
            "POST",
            "/api/sync/push",
            Some("scout3@dev.test"),
            Send::Bytes(&body),
        )
        .await;
    let receipts = r.json()["receipts"].as_array().cloned().unwrap_or_default();
    let outcomes: Vec<String> = receipts
        .iter()
        .map(|x| x["outcome"].as_str().unwrap_or("?").to_string())
        .collect();
    let reasons: Vec<String> = receipts
        .iter()
        .map(|x| x["reason"].as_str().unwrap_or("").to_string())
        .collect();
    h.check(
        "scout3",
        "POST /api/sync/push",
        "a robot that is not in the match, and a match that is not scheduled, are refused",
        outcomes.first().map(String::as_str) == Some("refused")
            && outcomes.get(1).map(String::as_str) == Some("refused")
            && reasons[0].contains("is not in")
            && reasons[1].contains("not on the schedule"),
        format!("{outcomes:?} {reasons:?}"),
    );
    h.check(
        "scout3",
        "POST /api/sync/push",
        "a valid entry is recorded, and the same robot again by the same scout is refused",
        outcomes.get(2).map(String::as_str) == Some("recorded")
            && (outcomes.get(3).map(String::as_str) == Some("recorded")
                || reasons[3].contains("already")),
        format!("{outcomes:?} {reasons:?}"),
    );
    let mine = h
        .state
        .repo
        .recorded_by(&h.event, h.id("scout3@dev.test"))
        .await?;
    h.check(
        "scout3",
        "POST /api/sync/push",
        "what was recorded is under their own account and team",
        mine.contains(&(q14.clone(), robot)),
        "not stored under scout3",
    );
    let bad_answers = serde_json::to_vec(&serde_json::json!({
        "observations": [{
            "client_record_id": "0191f7ac-1234-7000-8000-00000000ee05",
            "match_key": q14, "team_number": robot,
            "payload": { "not_a_field": 1 }, "schema_version": version,
            "observed_at": Utc::now(),
        }],
        "version": tt_core::outbox::OUTBOX_VERSION,
    }))?;
    let r = h
        .send(
            "POST",
            "/api/sync/push",
            Some("scout4@dev.test"),
            Send::Bytes(&bad_answers),
        )
        .await;
    h.check(
        "scout4",
        "POST /api/sync/push",
        "answers that do not fit the form are refused",
        r.json()["receipts"][0]["outcome"] == "refused",
        r.body.clone(),
    );
    let refused_before = h.state.repo.open_refusals().await?.len();
    h.check(
        "scout4",
        "POST /api/sync/push",
        "and kept for the lead scout (C10)",
        refused_before > f.refused.len(),
        format!("{refused_before} kept"),
    );
    let sync_unsigned = h
        .send("POST", "/api/sync/push", None, Send::Bytes(&body))
        .await;
    h.check(
        "(signed out)",
        "POST /api/sync/push",
        "nothing is recorded without a sign-in",
        sync_unsigned.status == 401,
        format!("{}", sync_unsigned.status),
    );

    // Verdicts are the lead's. A scout, a coach or a teamless scout who posts
    // one is sent home, and the observation is as it was.
    let id = f.ours_waiting;
    for who in [
        "scout1@dev.test",
        "coach@dev.test",
        "scoutb1@dev.test",
        "noteam@dev.test",
        "",
    ] {
        let who = (!who.is_empty()).then_some(who);
        for verb in ["approve", "decline", "reopen"] {
            let r = h
                .send(
                    "POST",
                    &format!("/api/observations/{id}/{verb}"),
                    who,
                    Send::Form("reason=nope"),
                )
                .await;
            let state = h.state.repo.observation(id).await?.map(|o| o.review_state);
            h.check(
                &label(who),
                &format!("POST /api/observations/{{id}}/{verb}"),
                "is refused and the observation stays waiting",
                r.status == 303 && state == Some(ReviewState::Pending),
                format!("{} {:?}", r.status, state),
            );
        }
    }
    // The lead's own verdict works, and an undo puts it back (G3).
    let r = h
        .send(
            "POST",
            &format!("/api/observations/{id}/approve"),
            Some("lead@dev.test"),
            Send::Form(""),
        )
        .await;
    let state = h.state.repo.observation(id).await?.map(|o| o.review_state);
    h.check(
        "lead",
        "POST /api/observations/{id}/approve",
        "approves",
        r.status == 303 && state == Some(ReviewState::Approved),
        format!("{} {state:?}", r.status),
    );
    let r = h
        .send(
            "POST",
            &format!("/api/observations/{id}/reopen"),
            Some("scout1@dev.test"),
            Send::Form(""),
        )
        .await;
    let state = h.state.repo.observation(id).await?.map(|o| o.review_state);
    h.check(
        "scout1",
        "POST /api/observations/{id}/reopen",
        "a scout cannot undo a verdict",
        r.status == 303 && state == Some(ReviewState::Approved),
        format!("{} {state:?}", r.status),
    );
    let r = h
        .send(
            "POST",
            &format!("/api/observations/{id}/reopen"),
            Some("lead@dev.test"),
            Send::Form(""),
        )
        .await;
    let state = h.state.repo.observation(id).await?.map(|o| o.review_state);
    h.check(
        "lead",
        "POST /api/observations/{id}/reopen",
        "undoes it (G3)",
        r.status == 303 && state == Some(ReviewState::Pending),
        format!("{} {state:?}", r.status),
    );

    // Assignments are the lead's.
    let before = h.state.repo.event_assignments(&h.event).await?.len();
    for who in ["scout1@dev.test", "coach@dev.test"] {
        let r = h
            .send(
                "POST",
                &format!("/api/assignments/clear?event={}", h.event),
                Some(who),
                Send::Form(""),
            )
            .await;
        let after = h.state.repo.event_assignments(&h.event).await?.len();
        h.check(
            &label(Some(who)),
            "POST /api/assignments/clear",
            "does not clear assignments",
            r.status == 303 && after == before,
            format!("{} {before} -> {after}", r.status),
        );
    }

    // Signing a scout out everywhere is the lead's, and works: the session
    // goes and the old token is refused.
    let scout2 = h
        .send(
            "POST",
            "/api/auth/token",
            Some("scout2@dev.test"),
            Send::Nothing,
        )
        .await;
    let scout2_credential = credential(&scout2.json());
    let target = h.id("scout2@dev.test");
    let denied = h
        .send(
            "POST",
            &format!("/api/users/{target}/sign-out?event={}", h.event),
            Some("scout1@dev.test"),
            Send::Form(""),
        )
        .await;
    let still = h
        .send("GET", "/account", Some("scout2@dev.test"), Send::Nothing)
        .await;
    h.check(
        "scout1",
        "POST /api/users/{id}/sign-out",
        "a scout cannot sign another out",
        denied.status == 303 && still.status == 200,
        format!("{} {}", denied.status, still.status),
    );
    let done = h
        .send(
            "POST",
            &format!("/api/users/{target}/sign-out?event={}", h.event),
            Some("lead@dev.test"),
            Send::Form(""),
        )
        .await;
    let gone = h
        .send("GET", "/account", Some("scout2@dev.test"), Send::Nothing)
        .await;
    let token = h
        .bearer(
            "GET",
            "/api/sync/pull",
            &scout2_credential,
            Some(&device_of(4)),
            Send::Nothing,
        )
        .await;
    h.check(
        "lead",
        "POST /api/users/{id}/sign-out",
        "ends their sessions and their offline token (T1)",
        done.status == 303 && gone.status == 303 && token.status == 401,
        format!("{} then {} and {}", done.status, gone.status, token.status),
    );

    // Pick lists belong to a team: the doc endpoint hands each lead their own
    // team's, a teamless user none, and a merge lands only in the caller's.
    let picks = |bytes: &[u8]| -> Vec<i32> {
        tt_core::picklist::PickDoc::load(bytes)
            .map(|d| d.entries().iter().map(|e| e.team_number).collect())
            .unwrap_or_default()
    };
    let ours = h
        .send(
            "GET",
            &format!("/api/pick-list/doc?event={}", h.event),
            Some("lead@dev.test"),
            Send::Nothing,
        )
        .await;
    let theirs = h
        .send(
            "GET",
            &format!("/api/pick-list/doc?event={}", h.event),
            Some("lead2@dev.test"),
            Send::Nothing,
        )
        .await;
    let (ours_list, theirs_list) = (picks(&ours.bytes), picks(&theirs.bytes));
    h.check(
        "lead",
        "GET /api/pick-list/doc",
        "is our team's list",
        ours_list == vec![10003, 10007, 10011, 10002],
        format!("{ours_list:?}"),
    );
    h.check(
        "lead2",
        "GET /api/pick-list/doc",
        "is their own team's list, not ours",
        theirs_list == vec![10005, 10009, 10001, 10012],
        format!("{theirs_list:?}"),
    );
    let none = h
        .send(
            "GET",
            &format!("/api/pick-list/doc?event={}", h.event),
            Some("noteamlead@dev.test"),
            Send::Nothing,
        )
        .await;
    h.check(
        "noteamlead",
        "GET /api/pick-list/doc",
        "a user with no team has no list (403)",
        none.status == 403,
        format!("{}", none.status),
    );
    let before_other = h.state.repo.pick_list(OTHER, &h.event).await?.len();
    let mut doc = tt_core::picklist::PickDoc::load(&ours.bytes).map_err(anyhow::Error::msg)?;
    let roster: Vec<i32> = h
        .state
        .repo
        .event_teams(&h.event)
        .await?
        .iter()
        .map(|t| t.number)
        .collect();
    let update = doc
        .apply(
            &tt_core::picklist::Edit::Add {
                team: 10004,
                record_id: "0191f7ac-1234-7000-8000-00000000dd01".into(),
            },
            &roster,
        )
        .map_err(anyhow::Error::msg)?
        .unwrap_or_default();
    let merged = h
        .send(
            "POST",
            &format!("/api/pick-list/doc?event={}", h.event),
            Some("coach@dev.test"),
            Send::Bytes(&update),
        )
        .await;
    h.check(
        "coach",
        "POST /api/pick-list/doc",
        "a coach's edit lands in our team's list",
        picks(&merged.bytes).contains(&10004),
        format!("{}", merged.status),
    );
    let after_other = h.state.repo.pick_list(OTHER, &h.event).await?.len();
    h.check(
        "coach",
        "POST /api/pick-list/doc",
        "and never in another team's",
        before_other == after_other,
        format!("{before_other} -> {after_other}"),
    );
    let scout_edit = h
        .send(
            "POST",
            &format!("/api/pick-list/doc?event={}", h.event),
            Some("scout1@dev.test"),
            Send::Bytes(&update),
        )
        .await;
    h.check(
        "scout1",
        "POST /api/pick-list/doc",
        "a scout cannot edit the pick list",
        scout_edit.status == 303,
        format!("{}", scout_edit.status),
    );

    // A malformed schedule is refused row by row, and changes nothing.
    let matches_before = h.state.repo.event_matches(&h.event).await?.len();
    let bad = "code=DEVEVT&name=Dev&timezone=America%2FChicago&date=&schedule=match%2Cred1%2Cred2%2Cblue1%2Cblue2%0A1%2C1%2C2%2C3&roster=";
    let r = h
        .send(
            "POST",
            &format!("/api/schedule/import?event={}", h.event),
            Some("lead@dev.test"),
            Send::Form(bad),
        )
        .await;
    let matches_after = h.state.repo.event_matches(&h.event).await?.len();
    h.check(
        "lead",
        "POST /api/schedule/import",
        "a bad row is named and nothing is imported (F3)",
        matches_before == matches_after && r.body.contains("line"),
        format!("{matches_before} -> {matches_after}"),
    );
    Ok(())
}

// ── Section: upstream is off ────────────────────────────────────────────────

async fn upstream_off(h: &mut Harness) {
    let page = h
        .send(
            "GET",
            &format!("/lead-scout/schedule?event={}", h.event),
            Some("lead@dev.test"),
            Send::Nothing,
        )
        .await;
    h.check(
        "lead",
        "GET /lead-scout/schedule",
        "has no Pull from FTC Events button: no key is read in dev (F7)",
        page.status == 200 && !page.body.contains("Pull from FTC"),
        "the button is offered",
    );
    let pull = h
        .send(
            "POST",
            &format!("/api/ftc/pull?event={}", h.event),
            Some("lead@dev.test"),
            Send::Form(""),
        )
        .await;
    h.check(
        "lead",
        "POST /api/ftc/pull",
        "says no key is set and makes no request",
        pull.body.contains("No FTC Events API key"),
        format!("{}", pull.status),
    );
    h.check(
        "(state)",
        "Upstream",
        "FIRST/TBA are off on this build and there is no FTC client",
        !h.state.upstream.enabled && h.state.upstream.ftc.is_none() && !h.state.upstream.loops(),
        "an upstream client is configured",
    );
    h.check(
        "(state)",
        "ASK_BACKEND",
        "the assistant has no backend in dev",
        crate::ask::configured().is_none(),
        "a backend is set",
    );
}
