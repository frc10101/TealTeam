//! The dev environment's checks (DEV1): the real router, driven as every
//! seeded user, held to the rules the code and the docs state.
//!
//! Every check is a [`Check`] with PASS or FAIL, who it ran as, and what it
//! asked. A FAIL is a finding: either the app is wrong, and gets fixed with a
//! regression test where the rule lives, or the expectation is, and gets
//! fixed here with a note of where the rule is stated.
//!
//! The route matrix is [`routes`]: one row per route in `startup::router`,
//! with who may use it. A test reads the router's source and fails when a
//! route has no row, so a new route cannot go unchecked.
//!
//! **To add a check:** a route goes in [`routes`]; a rule about what a page or
//! the sync carries goes in one of the `async fn`s `Harness::run` calls
//! (`sharing`, `sync_scope`, `tokens`, `writes`, `admin_viewer`, `logins`),
//! using `Harness::expect` and the markers from `seed`. Checks must leave the
//! data as they found it: a request that would change something is sent only
//! as a user who is refused, or with an id that is not there, and `matrix`
//! compares the tables before and after to prove it.

use std::collections::BTreeMap;

use axum::body::Body;
use axum::http::{Request, header};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::Utc;
use tower::ServiceExt;
use tt_core::picklist::PickDoc;
use tt_core::review::ReviewState;
use tt_core::token::{self, Claims, Holder};
use tt_core::user::{Roles, User};
use tt_repo::Repo;

use super::seed::{self, EVENT, TABLET, robots};
use super::{OTHER_TEAM, OUR_TEAM, PASSWORD, USERS};
use crate::auth::{SESSION_COOKIE, new_session};
use crate::startup::{AppState, router};

// ── The report ──────────────────────────────────────────────────────────────

pub struct Check {
    pub pass: bool,
    pub who: String,
    pub what: String,
    pub detail: String,
}

#[derive(Default)]
pub struct Report {
    pub checks: Vec<Check>,
}

impl Report {
    pub fn failed(&self) -> usize {
        self.checks.iter().filter(|c| !c.pass).count()
    }

    pub fn summary(&self) -> String {
        let failed = self.failed();
        format!(
            "dev checks: {} passed, {failed} failed, {} in all",
            self.checks.len() - failed,
            self.checks.len()
        )
    }

    fn line(c: &Check) -> String {
        let verdict = if c.pass { "PASS" } else { "FAIL" };
        let detail = if c.detail.is_empty() {
            String::new()
        } else {
            format!("  ({})", c.detail)
        };
        format!("{verdict}  [{}] {}{detail}", c.who, c.what)
    }

    pub fn print_all(&self) {
        for c in &self.checks {
            println!("{}", Self::line(c));
        }
    }

    pub fn print_failures(&self) {
        for c in self.checks.iter().filter(|c| !c.pass) {
            println!("{}", Self::line(c));
        }
    }
}

// ── Requests ────────────────────────────────────────────────────────────────

struct Resp {
    status: u16,
    location: Option<String>,
    headers: axum::http::HeaderMap,
    bytes: Vec<u8>,
}

impl Resp {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.bytes).into_owned()
    }

    fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.bytes).unwrap_or(serde_json::Value::Null)
    }

    /// Whether this is the app turning someone away: 401, 403, or a redirect
    /// to sign in or home.
    fn is_refusal(&self) -> bool {
        matches!(self.status, 401 | 403)
            || (self.status == 303 && matches!(self.location.as_deref(), Some("/" | "/sign-in")))
    }

    fn describe(&self) -> String {
        match &self.location {
            Some(to) => format!("{} -> {to}", self.status),
            None => self.status.to_string(),
        }
    }
}

/// A request to send.
struct Call {
    method: &'static str,
    uri: String,
    body: Vec<u8>,
    content_type: Option<&'static str>,
    cookie: Vec<String>,
    bearer: Option<String>,
    /// Do not read the body: it never ends (a live stream).
    stream: bool,
}

impl Call {
    fn new(method: &'static str, uri: impl Into<String>) -> Self {
        Self {
            method,
            uri: uri.into(),
            body: Vec::new(),
            content_type: None,
            cookie: Vec::new(),
            bearer: None,
            stream: false,
        }
    }

    fn get(uri: impl Into<String>) -> Self {
        Self::new("GET", uri)
    }

    fn form(mut self, body: &str) -> Self {
        self.content_type = Some("application/x-www-form-urlencoded");
        self.body = body.as_bytes().to_vec();
        self
    }

    fn json(mut self, body: serde_json::Value) -> Self {
        self.content_type = Some("application/json");
        self.body = body.to_string().into_bytes();
        self
    }

    fn cookie(mut self, pair: impl Into<String>) -> Self {
        self.cookie.push(pair.into());
        self
    }

    fn bearer(mut self, credential: impl Into<String>) -> Self {
        self.bearer = Some(credential.into());
        self
    }
}

/// One seeded user, signed in.
#[derive(Clone)]
struct Actor {
    key: &'static str,
    /// `"scout1 (scout, team 10101)"`.
    label: String,
    user: User,
    session: String,
}

impl Actor {
    fn roles(&self) -> Roles {
        self.user.roles
    }
}

/// Who a request is from: nobody, or an actor.
#[derive(Clone, Copy)]
struct Who<'a>(Option<&'a Actor>);

impl Who<'_> {
    fn label(&self) -> String {
        self.0
            .as_ref()
            .map_or("signed out".into(), |a| a.label.clone())
    }
}

// ── The route matrix ────────────────────────────────────────────────────────

/// Who may use a route, as its extractor and the docs say.
#[derive(Clone, Copy, Debug)]
enum Access {
    /// Anyone, signed in or not.
    Public,
    /// Signed out only: a signed-in visitor is sent home.
    AnonOnly,
    /// Anyone signed in (`Auth`).
    Member,
    /// A lead scout or admin (`LeadScout`).
    Lead,
    /// A coach or admin (`Coach`).
    Coach,
    /// A lead scout, coach or admin (`Strategist`): the pick list.
    Strategist,
    /// An admin (`Admin`).
    Admin,
    /// A session or an offline credential; 401 without (`sync_user`).
    Sync,
    /// As `Sync`, and a lead scout or admin; 403 for anyone else.
    SyncLead,
}

/// What a request should get.
#[derive(Debug, PartialEq, Eq)]
enum Want {
    /// The route did its job: not turned away, and 200 for a page.
    Through,
    Redirect(&'static str),
    Status(u16),
}

impl Access {
    fn want(self, roles: Option<Roles>) -> Want {
        let guarded = |allowed: fn(Roles) -> bool| match roles {
            None => Want::Redirect("/sign-in"),
            Some(r) if allowed(r) => Want::Through,
            Some(_) => Want::Redirect("/"),
        };
        match self {
            Access::Public => Want::Through,
            Access::AnonOnly => match roles {
                None => Want::Through,
                Some(_) => Want::Redirect("/"),
            },
            Access::Member => guarded(|_| true),
            Access::Lead => guarded(Roles::can_lead),
            Access::Coach => guarded(Roles::can_coach),
            Access::Strategist => guarded(Roles::can_pick),
            Access::Admin => guarded(Roles::can_admin),
            Access::Sync => match roles {
                None => Want::Status(401),
                Some(_) => Want::Through,
            },
            Access::SyncLead => match roles {
                None => Want::Status(401),
                Some(r) if r.can_lead() => Want::Through,
                Some(_) => Want::Status(403),
            },
        }
    }
}

struct Route {
    method: &'static str,
    /// As written in `startup::router`.
    pattern: &'static str,
    /// With ids filled in.
    uri: String,
    form: &'static str,
    access: Access,
    /// Whether a user who is let in is sent it too. Not for a request that
    /// would change data even when it is only empty, like clearing every
    /// assignment: for those only the users who are refused are tried.
    probe_allowed: bool,
    stream: bool,
}

fn r(
    method: &'static str,
    pattern: &'static str,
    access: Access,
    probe_allowed: bool,
    form: &'static str,
) -> Route {
    Route {
        method,
        pattern,
        uri: pattern.to_string(),
        form,
        access,
        probe_allowed,
        stream: false,
    }
}

/// Every route in `startup::router`, but the three that sign people in and
/// out and are checked on their own (`logins`).
fn routes(waiting: i64, refused: i64) -> Vec<Route> {
    use Access::*;
    let mut table = vec![
        r("GET", "/", Public, true, ""),
        r("GET", "/sign-in", AnonOnly, true, ""),
        r("GET", "/sign-up", AnonOnly, true, ""),
        r("GET", "/account", Member, true, ""),
        r("GET", "/help", Public, true, ""),
        r("GET", "/submission", Member, true, ""),
        r("GET", "/submission/download", Member, true, ""),
        r("GET", "/teams", Member, true, ""),
        r("GET", "/notes", Member, true, ""),
        r("GET", "/graph", Member, true, ""),
        r("GET", "/lead-scout", Lead, true, ""),
        r("GET", "/lead-scout/assignments", Lead, true, ""),
        r("GET", "/lead-scout/submissions/{id}", Lead, true, ""),
        r("GET", "/lead-scout/refused/{id}", Lead, true, ""),
        r("GET", "/lead-scout/rankings", Lead, true, ""),
        r("GET", "/lead-scout/rankings/enter", Lead, true, ""),
        r("GET", "/lead-scout/weights", Lead, true, ""),
        r("GET", "/lead-scout/scan", Lead, true, ""),
        r("GET", "/drive-coach", Coach, true, ""),
        r("GET", "/pick-list", Strategist, true, ""),
        r("GET", "/admin/db", Admin, true, ""),
        r("GET", "/admin/users", Admin, true, ""),
        // Forms. An empty body or an id that is not there changes nothing.
        r("POST", "/api/auth/token", Member, true, ""),
        r("POST", "/api/account/change-password", Member, true, ""),
        r("POST", "/api/submission", Member, true, ""),
        r("POST", "/api/device/heartbeat", Public, true, ""),
        r("POST", "/api/frc/sync", Lead, false, ""),
        r("POST", "/api/assignments/match", Lead, false, ""),
        r("POST", "/api/assignments/clear-match", Lead, false, ""),
        r("POST", "/api/assignments/clear", Lead, false, ""),
        r("POST", "/api/assignments/auto", Lead, false, ""),
        r("POST", "/api/devices/{id}/rename", Lead, true, ""),
        r("POST", "/api/users/{id}/sign-out", Lead, true, ""),
        r("POST", "/api/admin/users/{id}", Admin, true, ""),
        r("POST", "/api/admin/users/{id}/password", Admin, true, ""),
        r("POST", "/api/weights", Lead, false, ""),
        r("POST", "/api/weights/reset", Lead, false, ""),
        r("POST", "/api/rankings/manual", Lead, false, ""),
        r("POST", "/api/pick-list", Strategist, false, ""),
        r("GET", "/api/pick-list/doc", Strategist, true, ""),
        r("POST", "/api/pick-list/doc", Strategist, true, ""),
        r("GET", "/api/sync/pull", Sync, true, ""),
        // A live stream is a page's: it takes a session, not an offline token.
        r("GET", "/api/sync/stream", Member, true, ""),
        r("GET", "/api/sync/snapshot", Member, true, ""),
        r("POST", "/api/sync/push", Sync, true, ""),
        r("POST", "/api/sync/bundle", SyncLead, true, ""),
        r("GET", "/api/upstream/key", SyncLead, true, ""),
        r("POST", "/api/refused/{id}/record", Lead, true, ""),
        r("POST", "/api/refused/{id}/dismiss", Lead, true, ""),
        r("POST", "/api/handoff", Lead, true, ""),
        r("POST", "/api/observations/{id}/approve", Lead, true, ""),
        r("POST", "/api/observations/{id}/decline", Lead, true, ""),
        r("POST", "/api/observations/{id}/reopen", Lead, true, ""),
        r("GET", "/sw.js", Public, true, ""),
        r("GET", "/offline", Public, true, ""),
        r("GET", "/health", Public, true, ""),
        r("GET", "/status", Public, true, ""),
        r("GET", "/static/{*path}", Public, true, ""),
    ];
    for route in &mut table {
        let id = if route.method == "GET" {
            if route.pattern.contains("refused") {
                refused
            } else {
                waiting
            }
        } else {
            999_999
        };
        route.uri = route.pattern.replace("{id}", &id.to_string());
        if route.pattern.starts_with("/api/pick-list/doc") {
            route.uri = format!("/api/pick-list/doc?event={EVENT}");
        }
        if route.pattern == "/static/{*path}" {
            route.uri = "/static/js/link.js".into();
        }
        route.stream = route.pattern == "/api/sync/stream";
    }
    table
}

/// Routes `startup::router` registers that [`routes`] does not list, and are
/// not the sign-in ones. Empty is the point.
#[cfg(test)]
fn unlisted_routes() -> Vec<String> {
    let source = include_str!("../startup.rs");
    let router = source
        .split("pub fn router")
        .nth(1)
        .and_then(|s| s.split("async fn health_page").next())
        .unwrap_or_default();
    let listed: Vec<&str> = routes(0, 0).iter().map(|r| r.pattern).collect::<Vec<_>>();
    let listed: Vec<String> = listed.iter().map(|s| s.to_string()).collect();
    let mut missing = Vec::new();
    for chunk in router.split(".route(").skip(1) {
        let path: String = chunk
            .trim_start()
            .trim_start_matches('"')
            .chars()
            .take_while(|c| *c != '"')
            .collect();
        // `/ca.crt` 404s in dev, which has no CA (H1); tls.rs checks it serves.
        let handled_elsewhere = [
            "/api/auth/login",
            "/api/auth/signup",
            "/api/auth/logout",
            "/ca.crt",
        ];
        if !listed.contains(&path) && !handled_elsewhere.contains(&path.as_str()) {
            missing.push(path);
        }
    }
    missing
}

// ── The harness ─────────────────────────────────────────────────────────────

struct Harness<'a> {
    state: &'a AppState,
    actors: Vec<Actor>,
    report: std::cell::RefCell<Report>,
    /// A waiting observation of ours, and one of the other team's.
    ours_waiting: i64,
    theirs_waiting: i64,
    /// A refused entry of ours, and one of the other team's.
    ours_refused: i64,
    theirs_refused: i64,
}

/// Run every check against the data in `state`. Leaves the data as it was.
pub async fn run(state: &AppState) -> Report {
    let harness = match Harness::new(state).await {
        Ok(h) => h,
        Err(e) => {
            let mut report = Report::default();
            report.checks.push(Check {
                pass: false,
                who: "setup".into(),
                what: "find the seeded users and rows".into(),
                detail: e.to_string(),
            });
            return report;
        }
    };
    harness.matrix().await;
    harness.sharing().await;
    harness.sync_scope().await;
    harness.admin_viewer().await;
    harness.tokens().await;
    harness.writes().await;
    harness.logins().await;
    harness.accounts().await;
    harness.finish().await
}

impl<'a> Harness<'a> {
    async fn new(state: &'a AppState) -> anyhow::Result<Self> {
        let mut actors = Vec::new();
        for u in USERS {
            let found = state
                .repo
                .credentials_by_email(u.email)
                .await?
                .ok_or_else(|| anyhow::anyhow!("{} is not in the database", u.email))?;
            let session = new_session(found.user.id);
            state.repo.create_session(&session, Utc::now()).await?;
            let team = u
                .team
                .map_or("no team".to_string(), |t| format!("team {t}"));
            actors.push(Actor {
                key: u.key,
                label: format!(
                    "{} ({}, {team})",
                    u.key,
                    u.label.split(" (").next().unwrap()
                ),
                user: found.user,
                session: session.id,
            });
        }
        let pending = state.repo.pending_observations(EVENT).await?;
        let waiting_of = |team| {
            pending
                .iter()
                .find(|o| o.submitting_team == Some(team))
                .map(|o| o.id)
                .ok_or_else(|| anyhow::anyhow!("no waiting observation of team {team}"))
        };
        let refusals = state.repo.open_refusals().await?;
        let refused_of = |team| {
            refusals
                .iter()
                .find(|r| r.submitting_team == Some(team))
                .map(|r| r.id)
                .ok_or_else(|| anyhow::anyhow!("no refused entry of team {team}"))
        };
        Ok(Self {
            state,
            actors,
            report: Default::default(),
            ours_waiting: waiting_of(OUR_TEAM)?,
            theirs_waiting: waiting_of(OTHER_TEAM)?,
            ours_refused: refused_of(OUR_TEAM)?,
            theirs_refused: refused_of(OTHER_TEAM)?,
        })
    }

    async fn finish(self) -> Report {
        for actor in &self.actors {
            let _ = self.state.repo.delete_session(&actor.session).await;
        }
        self.report.take()
    }

    fn who(&self, key: &str) -> Who<'_> {
        Who(Some(self.actor(key)))
    }

    fn actor(&self, key: &str) -> &Actor {
        self.actors
            .iter()
            .find(|a| a.key == key)
            .unwrap_or_else(|| panic!("no dev user {key}"))
    }

    fn expect(&self, who: &str, what: impl Into<String>, pass: bool, detail: impl Into<String>) {
        self.report.borrow_mut().checks.push(Check {
            pass,
            who: who.to_string(),
            what: what.into(),
            detail: if pass { String::new() } else { detail.into() },
        });
    }

    async fn send(&self, who: Who<'_>, call: Call) -> Resp {
        let mut cookies = call.cookie.clone();
        if let Some(actor) = who.0 {
            cookies.insert(0, format!("{SESSION_COOKIE}={}", actor.session));
        }
        let mut request = Request::builder().method(call.method).uri(&call.uri);
        if !cookies.is_empty() {
            request = request.header(header::COOKIE, cookies.join("; "));
        }
        if let Some(kind) = call.content_type {
            request = request.header(header::CONTENT_TYPE, kind);
        }
        if let Some(credential) = &call.bearer {
            request = request.header(header::AUTHORIZATION, format!("Bearer {credential}"));
        }
        let response = router(self.state.clone())
            .oneshot(request.body(Body::from(call.body)).expect("a request"))
            .await
            .expect("the router answers");
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let location = headers
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .map(String::from);
        let bytes = if call.stream {
            Vec::new()
        } else {
            axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .map(|b| b.to_vec())
                .unwrap_or_default()
        };
        Resp {
            status,
            location,
            headers,
            bytes,
        }
    }

    /// The users the matrix is run as, signed out first.
    fn matrix_users(&self) -> Vec<Option<&Actor>> {
        let mut users = vec![None];
        users.extend(
            ["scout1", "lead", "coach", "admin", "noteam", "lead2"]
                .iter()
                .map(|k| Some(self.actor(k))),
        );
        users
    }

    // ── Route x role ────────────────────────────────────────────────────

    async fn matrix(&self) {
        let before = fingerprint(self.state).await;
        let table = routes(self.ours_waiting, self.ours_refused);
        for route in &table {
            for actor in self.matrix_users() {
                let roles = actor.map(Actor::roles);
                let want = route.access.want(roles);
                if want == Want::Through && !route.probe_allowed {
                    continue;
                }
                // A browser always has a device id (static/js/device.js).
                let mut call = Call::new(route.method, route.uri.clone())
                    .cookie("tt_device=dev-matrix-browser");
                if route.method == "POST" {
                    call = call.form(route.form);
                }
                call.stream = route.stream;
                let who = Who(actor);
                let got = self.send(who, call).await;
                let ok = match &want {
                    Want::Through if route.method == "GET" => got.status == 200,
                    Want::Through => !got.is_refusal() && got.status < 500,
                    Want::Redirect(to) => got.status == 303 && got.location.as_deref() == Some(to),
                    Want::Status(n) => got.status == *n,
                };
                self.expect(
                    &who.label(),
                    format!("{} {} -> {:?}", route.method, route.uri, want),
                    ok,
                    format!("got {}", got.describe()),
                );
            }
        }
        let after = fingerprint(self.state).await;
        let changed: Vec<&String> = before
            .iter()
            .filter(|(table, rows)| after.get(*table) != Some(rows))
            .map(|(table, _)| table)
            .collect();
        self.expect(
            "matrix",
            "no request in the matrix changed any data",
            changed.is_empty(),
            format!("changed: {changed:?}"),
        );
    }

    // ── What each team may read ─────────────────────────────────────────

    async fn page(&self, who: &str, uri: &str) -> String {
        self.send(self.who(who), Call::get(uri)).await.text()
    }

    /// `who`'s page at `uri` shows `shown` and not `hidden`.
    async fn shows(&self, who: &str, uri: &str, shown: &[&str], hidden: &[&str]) {
        let body = self.page(who, uri).await;
        let label = self.actor(who).label.clone();
        for s in shown {
            self.expect(
                &label,
                format!("GET {uri} shows {s}"),
                body.contains(s),
                "missing",
            );
        }
        for h in hidden {
            self.expect(
                &label,
                format!("GET {uri} does not show {h}"),
                !body.contains(h),
                "leaked",
            );
        }
    }

    /// U13 and the rest of what the docs say a viewer may read.
    async fn sharing(&self) {
        let ours = format!("N{OUR_TEAM}-");
        let theirs = format!("N{OTHER_TEAM}-");
        // Notes: the viewer's own team's only (U13).
        for who in ["scout1", "lead", "coach", "admin"] {
            self.shows(who, "/notes", &[&ours], &[&theirs]).await;
        }
        for who in ["scout5", "lead2"] {
            self.shows(who, "/notes", &[&theirs], &[&ours]).await;
        }
        self.shows("noteam", "/notes", &[], &[&ours, &theirs]).await;

        // A robot both teams wrote notes on, approved by both leads.
        let shared = robots(1)[0];
        let profile = format!("/teams?team={shared}");
        self.shows("scout1", &profile, &[&ours], &[&theirs]).await;
        self.shows("scout5", &profile, &[&theirs], &[&ours]).await;
        self.shows("noteam", &profile, &[], &[&ours, &theirs]).await;

        // A lead reviews any team's scouts without reading their notes.
        let (o, t) = (self.ours_waiting, self.theirs_waiting);
        self.shows(
            "lead",
            &format!("/lead-scout/submissions/{o}"),
            &[&ours],
            &[&theirs],
        )
        .await;
        self.shows(
            "lead",
            &format!("/lead-scout/submissions/{t}"),
            &[],
            &[&theirs],
        )
        .await;
        self.shows(
            "lead2",
            &format!("/lead-scout/submissions/{o}"),
            &[],
            &[&ours],
        )
        .await;
        self.shows(
            "lead2",
            &format!("/lead-scout/submissions/{t}"),
            &[&theirs],
            &[&ours],
        )
        .await;
        self.shows("lead", "/lead-scout", &[], &[&theirs]).await;
        self.shows("lead2", "/lead-scout", &[], &[&ours]).await;

        // The same for entries the Pi refused.
        let (o, t) = (self.ours_refused, self.theirs_refused);
        self.shows(
            "lead",
            &format!("/lead-scout/refused/{o}"),
            &[&ours],
            &[&theirs],
        )
        .await;
        self.shows("lead", &format!("/lead-scout/refused/{t}"), &[], &[&theirs])
            .await;
        self.shows(
            "lead2",
            &format!("/lead-scout/refused/{t}"),
            &[&theirs],
            &[&ours],
        )
        .await;

        // A scout is told of their own declines, and only those.
        let s3 = seed::decline_reason("s3", 8);
        let s6 = seed::decline_reason("s6", 7);
        let s3 = s3.split(':').next().unwrap_or_default().to_string();
        let s6 = s6.split(':').next().unwrap_or_default().to_string();
        let scouting = format!("/submission?event={EVENT}");
        self.shows("scout3", &scouting, &[&s3], &[&s6]).await;
        self.shows("scout6", &scouting, &[&s6], &[&s3]).await;
        for who in ["scout1", "scout2", "scout4", "scout5", "noteam"] {
            self.shows(who, &scouting, &[], &[&s3, &s6]).await;
        }

        // A pick list is its team's: the doc each lead is handed is theirs.
        for (who, team, picks) in [
            ("lead", OUR_TEAM, vec![3103, 3107, 3105, 3110]),
            ("lead2", OTHER_TEAM, vec![3102, 3104, 3109]),
        ] {
            let resp = self
                .send(
                    self.who(who),
                    Call::get(format!("/api/pick-list/doc?event={EVENT}")),
                )
                .await;
            let mut teams: Vec<i32> = PickDoc::load(&resp.bytes)
                .map(|d| d.entries().iter().map(|e| e.team_number).collect())
                .unwrap_or_default();
            teams.sort_unstable();
            let mut want = picks;
            want.sort_unstable();
            let label = self.actor(who).label.clone();
            self.expect(
                &label,
                format!("GET /api/pick-list/doc is team {team}'s list only"),
                resp.status == 200 && teams == want,
                format!("{} {teams:?}", resp.status),
            );
        }
    }

    // ── What the sync carries ───────────────────────────────────────────

    async fn sync_scope(&self) {
        let sessions: Vec<String> = self.actors.iter().map(|a| a.session.clone()).collect();
        let emails: Vec<&str> = USERS.iter().map(|u| u.email).collect();
        for key in [
            "scout1", "lead", "coach", "admin", "scout5", "lead2", "noteam",
        ] {
            let actor = self.actor(key);
            let (label, team) = (actor.label.clone(), actor.user.team_number);
            let resp = self
                .send(
                    Who(Some(actor)),
                    Call::get(format!("/api/sync/pull?event={EVENT}")),
                )
                .await;
            let body = resp.text();
            let json = resp.json();
            let changes = json["changes"].as_array().cloned().unwrap_or_default();
            self.expect(
                &label,
                "GET /api/sync/pull carries changes at all",
                resp.status == 200 && !changes.is_empty(),
                format!("{} with {} changes", resp.status, changes.len()),
            );
            let entities_ok = changes.iter().all(|c| {
                matches!(
                    c["entity"].as_str(),
                    Some("observation" | "assignment" | "pick_list_entry")
                )
            });
            self.expect(
                &label,
                "pull carries only replicated tables",
                entities_ok,
                "",
            );
            let secrets = sessions
                .iter()
                .map(String::as_str)
                .chain(emails.iter().copied());
            let leaked: Vec<&str> = secrets
                .chain(["$argon2id$", "token_key", "password_hash"])
                .filter(|s| body.contains(s))
                .collect();
            self.expect(
                &label,
                "pull holds no session, email or hash",
                leaked.is_empty(),
                format!("{leaked:?}"),
            );

            // Notes: only where the writing team is the viewer's.
            let mut foreign_notes = 0;
            let mut own_notes = 0;
            for c in changes.iter().filter(|c| c["entity"] == "observation") {
                let row = &c["row"];
                let writer = row["submitting_team"].as_i64().map(|t| t as i32);
                let has_notes = row["payload"]["notes"].is_string();
                match (has_notes, writer.is_some() && writer == team) {
                    (true, true) => own_notes += 1,
                    (true, false) => foreign_notes += 1,
                    _ => {}
                }
            }
            self.expect(
                &label,
                "pull shows no other team's notes",
                foreign_notes == 0,
                format!("{foreign_notes} observations with notes from another team"),
            );
            self.expect(
                &label,
                "pull shows the viewer's own team's notes",
                (team.is_none()) || own_notes > 0,
                "none",
            );
            // Pick lists: the viewer's team's.
            let others_picks = changes
                .iter()
                .filter(|c| c["entity"] == "pick_list_entry")
                .filter(|c| c["row"]["owning_team"].as_i64().map(|t| t as i32) != team)
                .count();
            self.expect(
                &label,
                "pull shows only the viewer's team's pick list",
                others_picks == 0,
                format!("{others_picks} entries of another team's"),
            );

            // S3: an event not asked for is not sent.
            let none = self
                .send(
                    self.who(key),
                    Call::get("/api/sync/pull?event=2026elsewhere"),
                )
                .await
                .json();
            let sent = none["changes"].as_array().map_or(usize::MAX, Vec::len);
            self.expect(
                &label,
                "pull scoped to another event carries no changes",
                sent == 0,
                format!("{sent}"),
            );
        }

        // Snapshots are cut the same way.
        for key in ["scout1", "scout5", "noteam", "lead"] {
            self.snapshot(key).await;
        }
    }

    async fn snapshot(&self, key: &str) {
        let actor = self.actor(key);
        let (label, team, session) = (
            actor.label.clone(),
            actor.user.team_number,
            actor.session.clone(),
        );
        let resp = self
            .send(
                Who(Some(actor)),
                Call::get(format!("/api/sync/snapshot?event={EVENT}")),
            )
            .await;
        let is_sqlite = resp.bytes.starts_with(b"SQLite format 3");
        self.expect(
            &label,
            "GET /api/sync/snapshot is a database",
            resp.status == 200 && is_sqlite,
            resp.describe(),
        );
        let raw = String::from_utf8_lossy(&resp.bytes).into_owned();
        let other = match team {
            Some(OUR_TEAM) => Some(OTHER_TEAM),
            Some(_) => Some(OUR_TEAM),
            None => None,
        };
        let mut hidden = vec!["$argon2id$".to_string(), session, "@dev.test".to_string()];
        match other {
            Some(t) => hidden.push(format!("N{t}-")),
            None => {
                hidden.push(format!("N{OUR_TEAM}-"));
                hidden.push(format!("N{OTHER_TEAM}-"));
            }
        }
        for h in &hidden {
            let shown = if h.len() > 12 {
                "a session id"
            } else {
                h.as_str()
            };
            self.expect(
                &label,
                format!("snapshot bytes do not contain {shown}"),
                !raw.contains(h.as_str()),
                "found",
            );
        }
        // And by its tables, opened as the device would.
        let path =
            std::env::temp_dir().join(format!("tt-dev-snapshot-{}-{key}.db", std::process::id()));
        let opened = async {
            std::fs::write(&path, &resp.bytes)?;
            let repo = tt_repo_sqlite::SqliteRepo::connect(&format!("sqlite://{}?mode=rwc", path.display()))?;
            let count = |sql: &'static str| {
                let pool = repo.pool().clone();
                async move { sqlx::query_scalar::<_, i64>(sql).fetch_one(&pool).await }
            };
            anyhow::Ok((
                count("SELECT COUNT(*) FROM sessions").await?,
                count("SELECT COUNT(*) FROM token_key").await?,
                count("SELECT COUNT(*) FROM devices").await?,
                count("SELECT COUNT(*) FROM refused_entries").await?,
                count("SELECT COUNT(*) FROM users WHERE password_hash IS NOT NULL AND password_hash <> ''").await.unwrap_or(0),
                sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM pick_list_entries WHERE owning_team IS NOT ?")
                    .bind(team)
                    .fetch_one(repo.pool())
                    .await?,
            ))
        }
        .await;
        let _ = std::fs::remove_file(&path);
        match opened {
            Ok((sessions, keys, devices, refused, hashes, others_picks)) => {
                self.expect(
                    &label,
                    "snapshot has no sessions, keys, devices or refused entries",
                    sessions + keys + devices + refused == 0,
                    format!("{sessions} {keys} {devices} {refused}"),
                );
                self.expect(
                    &label,
                    "snapshot has no password hashes",
                    hashes == 0,
                    format!("{hashes}"),
                );
                self.expect(
                    &label,
                    "snapshot has only the viewer's team's pick list",
                    others_picks == 0,
                    format!("{others_picks} rows"),
                );
            }
            Err(e) => self.expect(&label, "snapshot opens as a database", false, e.to_string()),
        }
    }

    // ── The database viewer ─────────────────────────────────────────────

    async fn admin_viewer(&self) {
        let sessions: Vec<String> = self.actors.iter().map(|a| a.session.clone()).collect();
        let label = self.actor("admin").label.clone();
        for uri in [
            "/admin/db",
            "/admin/db?table=users",
            "/admin/db?table=sessions",
            "/admin/db?table=token_key",
            "/admin/db?table=devices",
            "/admin/users",
        ] {
            let body = self.page("admin", uri).await;
            let leaked: Vec<&str> = ["$argon2id$", "password_hash\u{0}"]
                .into_iter()
                .filter(|s| body.contains(s))
                .chain(
                    sessions
                        .iter()
                        .map(String::as_str)
                        .filter(|s| body.contains(s)),
                )
                .collect();
            self.expect(
                &label,
                format!("GET {uri} shows no hash or session id"),
                leaked.is_empty(),
                "leaked",
            );
            if uri.starts_with("/admin/db") {
                self.expect(
                    &label,
                    format!("GET {uri} shows no email"),
                    !body.contains("@dev.test"),
                    "leaked",
                );
            }
        }
        let listing = self.page("admin", "/admin/db").await;
        self.expect(
            &label,
            "the viewer does not list sessions or token_key",
            !listing.contains("table=sessions") && !listing.contains("table=token_key"),
            "listed",
        );
        let users = self.page("admin", "/admin/db?table=users").await;
        self.expect(
            &label,
            "the viewer shows the users table, masked",
            users.contains("Ada Admin") && users.contains('\u{2022}'),
            "no mask",
        );
        let hidden = self.page("admin", "/admin/db?table=sessions").await;
        self.expect(
            &label,
            "asking the viewer for sessions by name shows no rows",
            !hidden.contains("expires_at"),
            "rows shown",
        );
        // The account page is the one place emails show, and never a hash.
        let accounts = self.page("admin", "/admin/users").await;
        self.expect(
            &label,
            "GET /admin/users lists the accounts",
            accounts.contains("lead@dev.test"),
            "missing",
        );
    }

    // ── Offline tokens ──────────────────────────────────────────────────

    async fn credential(&self, user: &User, device: &str, epoch: Option<i64>) -> String {
        match epoch {
            None => {
                let issued = crate::token::issue_to(self.state, user, device)
                    .await
                    .expect("a token");
                format!(
                    "{}{}{}",
                    issued.token,
                    token::CREDENTIAL_SEPARATOR,
                    issued.key
                )
            }
            Some(epoch) => {
                let seed = crate::token::seed(self.state).await.expect("the seed");
                let claims = Claims::new(user, device, epoch, Utc::now());
                let key = URL_SAFE_NO_PAD.encode(token::device_key(&seed, &Holder::of(&claims)));
                format!(
                    "{}{}{}",
                    token::sign(&seed, &claims),
                    token::CREDENTIAL_SEPARATOR,
                    key
                )
            }
        }
    }

    async fn tokens(&self) {
        let scout = self.actor("scout1").user.clone();
        let lead = self.actor("lead").user.clone();
        let signedout = self.actor("signedout").user.clone();
        let nobody = Who(None);
        let device = format!("tt_device={TABLET}");
        let pull = "/api/sync/pull";
        let good = self.credential(&scout, TABLET, None).await;
        let epoch = self.state.repo.token_epoch(signedout.id).await.unwrap_or(0);

        let cases: Vec<(String, Call, u16)> = vec![
            ("a token with its key, from its device".into(), Call::get(pull).bearer(good.clone()).cookie(device.clone()), 200),
            ("a token with its key, from the downloaded page (no device cookie)".into(), Call::get(pull).bearer(good.clone()), 200),
            ("a token without its key (T1)".into(), Call::get(pull).bearer(good.split(token::CREDENTIAL_SEPARATOR).next().unwrap_or_default()), 401),
            ("a token with another device's key (T1)".into(), Call::get(pull).bearer(format!("{}{}{}", good.split(token::CREDENTIAL_SEPARATOR).next().unwrap_or_default(), token::CREDENTIAL_SEPARATOR, self.credential(&lead, TABLET, None).await.split(token::CREDENTIAL_SEPARATOR).nth(1).unwrap_or_default())), 401),
            ("a token shown with another device's cookie".into(), Call::get(pull).bearer(good.clone()).cookie("tt_device=some-other-tablet"), 401),
            ("a token from before the user was signed out everywhere".into(), Call::get(pull).bearer(self.credential(&signedout, TABLET, Some(epoch - 1)).await), 401),
            ("a token made after, at the new epoch".into(), Call::get(pull).bearer(self.credential(&signedout, TABLET, Some(epoch)).await), 200),
            ("a token signed by some other key".into(), Call::get(pull).bearer("v4.public.AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA~AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"), 401),
            ("a scout's token on a lead-only route".into(), Call::get("/api/upstream/key").bearer(good.clone()), 403),
            ("a lead's token on a lead-only route".into(), Call::get("/api/upstream/key").bearer(self.credential(&lead, TABLET, None).await), 200),
            ("a scout's token on a bundle push".into(), Call::new("POST", "/api/sync/bundle").bearer(good.clone()), 403),
        ];
        for (what, call, want) in cases {
            let uri = call.uri.clone();
            let got = self.send(nobody, call).await;
            self.expect(
                "offline token",
                format!("{what}: GET {uri} -> {want}"),
                got.status == want,
                format!("got {}", got.describe()),
            );
        }
        // The signed-out-everywhere user's old token is refused on a push as well.
        let old = self.credential(&signedout, TABLET, Some(epoch - 1)).await;
        let got = self
            .send(
                nobody,
                Call::new("POST", "/api/sync/push")
                    .bearer(old)
                    .json(serde_json::json!({"observations": []})),
            )
            .await;
        self.expect(
            "offline token",
            "an old-epoch token on POST /api/sync/push -> 401",
            got.status == 401,
            got.describe(),
        );
        let got = self
            .send(
                nobody,
                Call::new("POST", "/api/sync/push")
                    .bearer(good.clone())
                    .json(serde_json::json!({"observations": []})),
            )
            .await;
        self.expect(
            "offline token",
            "a good token on POST /api/sync/push -> 200",
            got.status == 200,
            got.describe(),
        );
        // A token signs no one in: pages need a session.
        let got = self
            .send(nobody, Call::get("/lead-scout").bearer(good.clone()))
            .await;
        self.expect(
            "offline token",
            "a token is not a session: GET /lead-scout -> 303 /sign-in",
            got.status == 303 && got.location.as_deref() == Some("/sign-in"),
            got.describe(),
        );
        let got = self.send(nobody, Call::get("/").bearer(good)).await;
        self.expect(
            "offline token",
            "a token is not a session: GET / shows no account",
            !got.text().contains("Sam Scout"),
            "signed in",
        );
    }

    // ── Writes ──────────────────────────────────────────────────────────

    async fn writes(&self) {
        let scout = self.who("scout1");
        let label = self.actor("scout1").label.clone();
        let m1 = seed::match_key(1);
        let first = robots(1)[0];

        // A robot not in the match is refused with a reason, not stored.
        let body = |m: &str, team: i32, id: &str| {
            format!("match={m}&team={team}&record_id={id}&f.starting_position=center")
        };
        let got = self
            .send(
                scout,
                Call::new("POST", "/api/submission").form(&body(
                    &seed::match_key(11),
                    9999,
                    "0191f7ac-0000-7000-8000-00000000d001",
                )),
            )
            .await;
        self.expect(
            &label,
            "a scout cannot save a robot that is not in the match",
            got.status == 200 && got.text().contains("pick the robot you watched"),
            got.describe(),
        );
        let got = self
            .send(
                scout,
                Call::new("POST", "/api/submission").form(&body(
                    "2026devevt_qm99",
                    first,
                    "0191f7ac-0000-7000-8000-00000000d002",
                )),
            )
            .await;
        self.expect(
            &label,
            "a scout cannot save a match that is not on the schedule",
            got.status == 200 && got.text().contains("not on the schedule"),
            got.describe(),
        );
        // One observation per robot per match.
        let got = self
            .send(
                scout,
                Call::new("POST", "/api/submission").form(&body(
                    &m1,
                    first,
                    "0191f7ac-0000-7000-8000-00000000d003",
                )),
            )
            .await;
        self.expect(
            &label,
            "a scout cannot record the same robot in the same match twice",
            got.status == 200 && got.text().contains("one observation per robot per match"),
            got.describe(),
        );

        // A push refuses what the schedule does not hold, and says why.
        let entry = |m: &str, team: i32| {
            serde_json::json!({"observations": [{
                "client_record_id": "0191f7ac-0000-7000-8000-00000000d004",
                "match_key": m, "team_number": team, "payload": {"starting_position": "center"},
                "schema_version": self.state.season.version, "observed_at": Utc::now(),
            }]})
        };
        let got = self
            .send(
                scout,
                Call::new("POST", "/api/sync/push").json(entry("2026devevt_qm99", first)),
            )
            .await;
        let refused = got.json()["receipts"][0]["outcome"] == "refused";
        self.expect(
            &label,
            "a push for a match not on the schedule is refused",
            got.status == 200 && refused,
            got.text(),
        );
        let got = self
            .send(
                scout,
                Call::new("POST", "/api/sync/push").json(entry(&m1, 9999)),
            )
            .await;
        let refused = got.json()["receipts"][0]["outcome"] == "refused";
        self.expect(
            &label,
            "a push for a robot not in the match is refused",
            got.status == 200 && refused,
            got.text(),
        );

        // Only a lead reviews: nobody else can approve, decline or undo.
        let id = self.ours_waiting;
        for who in ["scout1", "coach", "noteam", "scout5"] {
            for verb in ["approve", "decline", "reopen"] {
                let actor = self.who(who);
                let form = if verb == "decline" { "reason=nope" } else { "" };
                let got = self
                    .send(
                        actor,
                        Call::new("POST", format!("/api/observations/{id}/{verb}")).form(form),
                    )
                    .await;
                self.expect(
                    &actor.label(),
                    format!("POST /api/observations/{id}/{verb} -> 303 /"),
                    got.status == 303 && got.location.as_deref() == Some("/"),
                    got.describe(),
                );
            }
        }
        let got = self
            .send(
                Who(None),
                Call::new("POST", format!("/api/observations/{id}/approve")),
            )
            .await;
        self.expect(
            "signed out",
            "approving -> 303 /sign-in",
            got.status == 303 && got.location.as_deref() == Some("/sign-in"),
            got.describe(),
        );
        let state = self
            .state
            .repo
            .observation(id)
            .await
            .ok()
            .flatten()
            .map(|o| o.review_state);
        self.expect(
            "review",
            "after all that the observation is still waiting",
            state == Some(ReviewState::Pending),
            format!("{state:?}"),
        );

        // And a lead can, and can take it back (G3): a canary that ends as it began.
        let lead = self.who("lead");
        let lead_label = lead.label();
        let canary = self.canary().await;
        let go = |verb: &str, form: &str| {
            Call::new("POST", format!("/api/observations/{canary}/{verb}")).form(form)
        };
        let got = self.send(lead, go("decline", "reason=")).await;
        let state = self
            .state
            .repo
            .observation(canary)
            .await
            .ok()
            .flatten()
            .map(|o| o.review_state);
        self.expect(
            &lead_label,
            "a decline needs a reason",
            got.status == 200 && state == Some(ReviewState::Pending),
            format!("{} {state:?}", got.describe()),
        );
        let got = self.send(lead, go("approve", "")).await;
        let state = self
            .state
            .repo
            .observation(canary)
            .await
            .ok()
            .flatten()
            .map(|o| o.review_state);
        self.expect(
            &lead_label,
            "a lead approves a waiting observation",
            got.status == 303 && state == Some(ReviewState::Approved),
            format!("{} {state:?}", got.describe()),
        );
        let got = self.send(self.who("scout1"), go("reopen", "")).await;
        let state = self
            .state
            .repo
            .observation(canary)
            .await
            .ok()
            .flatten()
            .map(|o| o.review_state);
        self.expect(
            &label,
            "a scout cannot undo a review",
            got.status == 303 && state == Some(ReviewState::Approved),
            format!("{} {state:?}", got.describe()),
        );
        let got = self.send(lead, go("reopen", "")).await;
        let state = self
            .state
            .repo
            .observation(canary)
            .await
            .ok()
            .flatten()
            .map(|o| o.review_state);
        self.expect(
            &lead_label,
            "a lead undoes a review (G3)",
            got.status == 303 && state == Some(ReviewState::Pending),
            format!("{} {state:?}", got.describe()),
        );
    }

    /// A waiting observation of ours that is not the one other checks read.
    async fn canary(&self) -> i64 {
        self.state
            .repo
            .pending_observations(EVENT)
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|o| o.submitting_team == Some(OUR_TEAM) && o.id != self.ours_waiting)
            .map(|o| o.id)
            .next_back()
            .unwrap_or(self.ours_waiting)
    }

    // ── Signing in and out ──────────────────────────────────────────────

    async fn logins(&self) {
        for u in USERS {
            let form = |password: &str| {
                format!("email={}&password={password}", u.email.replace('@', "%40"))
            };
            let got = self
                .send(
                    Who(None),
                    Call::new("POST", "/api/auth/login").form(&form(PASSWORD)),
                )
                .await;
            let cookie = got
                .headers
                .get_all(header::SET_COOKIE)
                .iter()
                .filter_map(|v| v.to_str().ok())
                .find(|v| v.starts_with(SESSION_COOKIE))
                .and_then(|v| v.split(';').next())
                .map(String::from);
            self.expect(
                u.key,
                "signing in with the password dev works",
                got.status == 303 && got.location.as_deref() == Some("/") && cookie.is_some(),
                got.describe(),
            );
            let bad = self
                .send(
                    Who(None),
                    Call::new("POST", "/api/auth/login").form(&form("not-it")),
                )
                .await;
            let set = bad.headers.get_all(header::SET_COOKIE).iter().any(|v| {
                v.to_str()
                    .is_ok_and(|v| v.starts_with(SESSION_COOKIE) && !v.contains("Max-Age=0"))
            });
            self.expect(
                u.key,
                "signing in with a wrong password does not",
                bad.status == 303 && bad.location.as_deref() == Some("/sign-in?failed=1") && !set,
                bad.describe(),
            );
            if let Some(cookie) = cookie {
                let home = self
                    .send(Who(None), Call::get("/account").cookie(cookie.clone()))
                    .await;
                self.expect(
                    u.key,
                    "the new session opens /account",
                    home.status == 200,
                    home.describe(),
                );
                let out = self
                    .send(
                        Who(None),
                        Call::new("POST", "/api/auth/logout").cookie(cookie.clone()),
                    )
                    .await;
                let after = self
                    .send(Who(None), Call::get("/account").cookie(cookie))
                    .await;
                self.expect(
                    u.key,
                    "signing out ends the session",
                    out.status == 303
                        && after.status == 303
                        && after.location.as_deref() == Some("/sign-in"),
                    format!("{} then {}", out.describe(), after.describe()),
                );
            }
        }
    }

    // ── Account administration (G1) ─────────────────────────────────────

    async fn accounts(&self) {
        let admin = self.who("admin");
        let (admin_label, admin_id) = (admin.label(), self.actor("admin").user.id);
        let scout = self.actor("scout6").user.clone();
        let save = |id: i64| format!("/api/admin/users/{id}");

        // The only admin stays one.
        let got = self
            .send(
                admin,
                Call::new("POST", save(admin_id)).form("team_number=10101"),
            )
            .await;
        let still = self
            .state
            .repo
            .user_by_id(admin_id)
            .await
            .ok()
            .flatten()
            .is_some_and(|u| u.roles.is_admin);
        self.expect(
            &admin_label,
            "the last admin cannot be demoted",
            got.location
                .as_deref()
                .is_some_and(|to| to.contains("done=last-admin"))
                && still,
            got.describe(),
        );

        // A lead cannot change anyone's roles, and an admin can, and undo it.
        let lead = self.who("lead");
        let got = self
            .send(
                lead,
                Call::new("POST", save(scout.id)).form("team_number=4455&is_admin=on"),
            )
            .await;
        let roles = self
            .state
            .repo
            .user_by_id(scout.id)
            .await
            .ok()
            .flatten()
            .map(|u| u.roles);
        self.expect(
            &lead.label(),
            "a lead cannot make anyone an admin",
            got.status == 303 && got.location.as_deref() == Some("/") && roles == Some(scout.roles),
            format!("{} {roles:?}", got.describe()),
        );
        let coaching =
            |on: bool| format!("team_number=4455{}", if on { "&is_coach=on" } else { "" });
        let got = self
            .send(
                admin,
                Call::new("POST", save(scout.id)).form(&coaching(true)),
            )
            .await;
        let page = self
            .send(self.who("scout6"), Call::get("/drive-coach"))
            .await;
        self.expect(
            &admin_label,
            "an admin makes a scout a drive coach, who then sees the coach page",
            got.status == 303 && page.status == 200,
            format!("{} then {}", got.describe(), page.describe()),
        );
        let got = self
            .send(
                admin,
                Call::new("POST", save(scout.id)).form(&coaching(false)),
            )
            .await;
        let page = self
            .send(self.who("scout6"), Call::get("/drive-coach"))
            .await;
        self.expect(
            &admin_label,
            "and takes it away again",
            got.status == 303 && page.status == 303,
            format!("{} then {}", got.describe(), page.describe()),
        );

        // A password too short to keep is refused and changes nothing.
        let before = self.state.repo.password_hash(scout.id).await.ok().flatten();
        let got = self
            .send(
                admin,
                Call::new("POST", format!("/api/admin/users/{}/password", scout.id))
                    .form("new_password=dev"),
            )
            .await;
        let after = self.state.repo.password_hash(scout.id).await.ok().flatten();
        self.expect(
            &admin_label,
            "a reset to a short password is refused",
            got.location
                .as_deref()
                .is_some_and(|to| to.contains("done=short-password"))
                && before == after,
            got.describe(),
        );
        let got = self
            .send(
                lead,
                Call::new("POST", format!("/api/admin/users/{}/password", scout.id))
                    .form("new_password=a-long-enough-one"),
            )
            .await;
        let after = self.state.repo.password_hash(scout.id).await.ok().flatten();
        self.expect(
            &lead.label(),
            "a lead cannot reset a password",
            got.location.as_deref() == Some("/") && before == after,
            got.describe(),
        );

        // A session that has run out is no session.
        let old = tt_core::user::Session {
            id: crate::auth::generate_session_id(),
            user_id: scout.id,
            expires_at: Utc::now() - chrono::TimeDelta::minutes(1),
        };
        let _ = self
            .state
            .repo
            .create_session(&old, Utc::now() - chrono::TimeDelta::hours(2))
            .await;
        let got = self
            .send(
                Who(None),
                Call::get("/account").cookie(format!("{SESSION_COOKIE}={}", old.id)),
            )
            .await;
        self.expect(
            "expired session",
            "an expired session is sent to sign in",
            got.status == 303 && got.location.as_deref() == Some("/sign-in"),
            got.describe(),
        );
    }
}

/// The tables the checks must not change, read as the admin viewer reads them.
async fn fingerprint(state: &AppState) -> BTreeMap<String, String> {
    let mut seen = BTreeMap::new();
    // Who is who, and what opens their account. Not the raw table, whose
    // "last seen" a heartbeat updates by design (A6).
    let mut accounts = String::new();
    for account in state.repo.accounts().await.unwrap_or_default() {
        let hash = state.repo.password_hash(account.id).await.ok().flatten();
        let epoch = state.repo.token_epoch(account.id).await.ok();
        accounts.push_str(&format!("{account:?}{hash:?}{epoch:?}"));
    }
    seen.insert("accounts".to_string(), accounts);
    for table in [
        "observations",
        "scout_assignments",
        "pick_list_entries",
        "pick_list_docs",
        "scouting_point_weights",
        "refused_entries",
        "token_epochs",
        "matches",
        "event_teams",
    ] {
        let mut text = String::new();
        for page in 0.. {
            match tt_repo_sqlite::admin::rows(state.repo.pool(), table, page).await {
                Ok(Some(rows)) => {
                    text.push_str(&format!("{:?}", rows.rows));
                    if (page + 1) * rows.page_size >= rows.total {
                        break;
                    }
                }
                other => {
                    text.push_str(&format!("{other:?}"));
                    break;
                }
            }
        }
        seen.insert(table.to_string(), text);
    }
    seen
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_route_has_a_row_in_the_matrix() {
        assert_eq!(unlisted_routes(), Vec::<String>::new());
    }

    #[test]
    fn what_each_kind_of_route_expects_of_each_kind_of_user() {
        let scout = Some(Roles::SCOUT);
        let lead = Some(Roles {
            is_lead_scout: true,
            ..Roles::SCOUT
        });
        let coach = Some(Roles {
            is_coach: true,
            ..Roles::SCOUT
        });
        let admin = Some(Roles {
            is_admin: true,
            ..Roles::SCOUT
        });
        assert_eq!(Access::Lead.want(None), Want::Redirect("/sign-in"));
        assert_eq!(Access::Lead.want(scout), Want::Redirect("/"));
        assert_eq!(Access::Lead.want(coach), Want::Redirect("/"));
        assert_eq!(Access::Lead.want(lead), Want::Through);
        assert_eq!(Access::Lead.want(admin), Want::Through);
        // A lead is not a coach, and a coach is not a lead.
        assert_eq!(Access::Coach.want(lead), Want::Redirect("/"));
        assert_eq!(Access::Strategist.want(coach), Want::Through);
        assert_eq!(Access::Admin.want(lead), Want::Redirect("/"));
        assert_eq!(Access::SyncLead.want(scout), Want::Status(403));
        assert_eq!(Access::Sync.want(None), Want::Status(401));
    }

    /// The whole matrix on a fresh dev database. Every check is a test.
    #[tokio::test]
    async fn the_dev_environment_passes_its_own_checks() {
        let dir = std::env::temp_dir().join(format!("tt-dev-checks-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let state = super::super::open(&dir).await.expect("seeded");
        let report = run(&state).await;
        let failures: Vec<String> = report
            .checks
            .iter()
            .filter(|c| !c.pass)
            .map(Report::line)
            .collect();
        std::fs::remove_dir_all(&dir).ok();
        assert!(report.checks.len() > 300, "{}", report.summary());
        assert!(
            failures.is_empty(),
            "{}\n{}",
            report.summary(),
            failures.join("\n")
        );
    }
}
