//! Page and form handlers.
//!
//! Two conventions, both carried forward deliberately:
//!
//! * **Access control is in the signature.** A handler that takes [`LeadScout`]
//!   cannot be reached without the role, because the extractor refuses to build.
//!   No handler re-checks role flags in its body (REBUILD_SPEC.md 12.4).
//!
//! * **Failed forms re-render with the input preserved.** Redirecting to an
//!   empty form loses what someone typed, which on a phone in a gymnasium is the
//!   difference between fixing a typo and giving up.

use std::collections::HashMap;

use axum::extract::{Form, Path, Query, State};
use axum::http::header::ACCEPT;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::cookie::CookieJar;
use chrono::Utc;
use serde::Deserialize;
use tt_core::user::{self, Roles};
use tt_repo::{NewUser, Repo};
use tt_templates::{AccountPage, HomePage, LeadScoutPage, Nav, Page, SignInPage, SignUpPage};

use crate::assignments::{self, GridParams};
use crate::auth::{
    Auth, Coach, LeadScout, MaybeAuth, SESSION_COOKIE, Strategist, clear_session_cookie,
    device_uuid, hash_password, new_session, session_cookie, verify_password,
};
use crate::coach;
use crate::events::{self, EventContext, EventParam};
use crate::picklist;
use crate::ranking::{self, RankingParams};
use crate::review::{self, ReviewedParam};
use crate::scouting::{self, ScoutParams};
use crate::standings;
use crate::startup::AppState;
use crate::teams::{self, TeamParam};
use crate::upstream::{self, ManualSync};

/// Shown instead of a specific reason when a login fails.
///
/// Identical for "no such account" and "wrong password", so the form cannot be
/// used to discover which email addresses are registered.
const LOGIN_FAILED: &str = "Invalid email or password";

/// Render a template, or return a 500 that says so.
fn html(page: impl Page) -> Response {
    match page.render_html() {
        Ok(body) => axum::response::Html(body).into_response(),
        Err(e) => {
            tracing::error!("render failed: {e}");
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "Something went wrong rendering this page.",
            )
                .into_response()
        }
    }
}

pub(crate) async fn nav_for(state: &AppState, user: Option<&tt_core::user::User>) -> Nav {
    let mut nav = Nav::for_user(user, state.repo.health().await.is_ready());
    nav.version = crate::shell::page_version(state);
    nav
}

/// Nav with the event switcher, plus the event the page is about (U2).
async fn event_page(
    state: &AppState,
    user: Option<&tt_core::user::User>,
    requested: Option<&str>,
) -> (Nav, EventContext) {
    // Each event judged by its own calendar (Q5).
    let context = events::resolve(&*state.repo, user, requested, Utc::now()).await;
    let mut nav = nav_for(state, user).await;
    nav.event = context.switcher();
    (nav, context)
}

fn team_display(team_number: Option<i32>) -> String {
    team_number.map(|n| n.to_string()).unwrap_or_default()
}

// ── Home ────────────────────────────────────────────────────────────────────

pub async fn home(
    State(state): State<AppState>,
    MaybeAuth(user): MaybeAuth,
    EventParam(requested): EventParam,
) -> Response {
    let (nav, context) = event_page(&state, user.as_ref(), requested.as_deref()).await;
    let team = user.as_ref().and_then(|u| u.team_number);
    html(HomePage {
        title: "Home".into(),
        nav,
        team_display: team_display(team),
        season_name: state.season.name.clone(),
        season_year: state.season.season,
        event: events::panel(&*state.repo, &context, team).await,
    })
}

// ── Sign in ─────────────────────────────────────────────────────────────────

pub async fn sign_in_page(State(state): State<AppState>, MaybeAuth(user): MaybeAuth) -> Response {
    if user.is_some() {
        return Redirect::to("/").into_response();
    }
    html(SignInPage {
        title: "Sign in".into(),
        nav: nav_for(&state, None).await,
        email: String::new(),
        error: String::new(),
    })
}

#[derive(Deserialize)]
pub struct LoginForm {
    email: String,
    password: String,
}

pub async fn login(
    State(state): State<AppState>,
    jar: CookieJar,
    Form(form): Form<LoginForm>,
) -> Response {
    let email = user::normalize_email(&form.email);

    let failed = |state: &AppState, nav: Nav| {
        let _ = state;
        html(SignInPage {
            title: "Sign in".into(),
            nav,
            email: email.clone(),
            error: LOGIN_FAILED.into(),
        })
    };

    let nav = nav_for(&state, None).await;

    let credentials = match state.repo.credentials_by_email(&email).await {
        Ok(Some(c)) => c,
        Ok(None) => {
            // Hash anyway. Returning early here would make a missing account
            // measurably faster to reject than a wrong password, which leaks
            // exactly what the generic message is meant to hide.
            let _ = verify_password(&form.password, DUMMY_HASH);
            return failed(&state, nav);
        }
        Err(e) => {
            tracing::error!("login lookup failed: {e}");
            return failed(&state, nav);
        }
    };

    if !verify_password(&form.password, &credentials.password_hash) {
        return failed(&state, nav);
    }

    let session = new_session(credentials.user.id);
    let now = Utc::now();
    if let Err(e) = state.repo.create_session(&session, now).await {
        tracing::error!("creating session: {e}");
        return failed(&state, nav);
    }
    if let Err(e) = state.repo.record_login(credentials.user.id, now).await {
        // Not worth failing a login over.
        tracing::warn!("recording login time: {e}");
    }

    (jar.add(session_cookie(session.id)), Redirect::to("/")).into_response()
}

/// A real Argon2id hash of a value nobody will guess, used to burn the same CPU
/// time on a missing account as on a wrong password.
const DUMMY_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$c29tZXNhbHRzb21lc2E$\
                          A7oMTMYWIWKrCJcHrCFVJTnE+kVSJq4nAgLNqJNBLpQ";

// ── Sign up ─────────────────────────────────────────────────────────────────

pub async fn sign_up_page(State(state): State<AppState>, MaybeAuth(user): MaybeAuth) -> Response {
    if user.is_some() {
        return Redirect::to("/").into_response();
    }
    let first_account = !state.repo.has_any_user().await.unwrap_or(true);
    html(SignUpPage {
        title: "Create an account".into(),
        nav: nav_for(&state, None).await,
        name: String::new(),
        email: String::new(),
        team_number: String::new(),
        error: String::new(),
        first_account,
    })
}

#[derive(Deserialize)]
pub struct SignUpForm {
    name: String,
    email: String,
    #[serde(default)]
    team_number: String,
    password: String,
    confirm_password: String,
}

pub async fn signup(
    State(state): State<AppState>,
    jar: CookieJar,
    Form(form): Form<SignUpForm>,
) -> Response {
    let nav = nav_for(&state, None).await;
    let has_users = state.repo.has_any_user().await.unwrap_or(true);

    let reject = |message: String| {
        html(SignUpPage {
            title: "Create an account".into(),
            nav: nav.clone(),
            name: form.name.trim().to_string(),
            email: form.email.trim().to_string(),
            team_number: form.team_number.trim().to_string(),
            error: message,
            first_account: !has_users,
        })
    };

    let name = form.name.trim().to_string();
    if name.is_empty() {
        return reject("Your name is required.".into());
    }

    let email = match user::validate_email(&form.email) {
        Ok(e) => e,
        Err(_) => return reject("That email address does not look right.".into()),
    };

    let team_number = match user::validate_team_number(&form.team_number) {
        Ok(t) => t,
        Err(_) => return reject("Team number must be a positive whole number.".into()),
    };

    if form.password != form.confirm_password {
        return reject("The two passwords do not match.".into());
    }
    if let Err(e) = user::validate_password(&form.password) {
        return reject(e.to_string());
    }

    let Ok(password_hash) = hash_password(&form.password) else {
        tracing::error!("hashing password failed");
        return reject("Could not create the account. Try again.".into());
    };

    // The first account on a fresh database is an administrator -- otherwise a
    // new deployment has nobody who can grant anybody anything.
    let roles = Roles {
        is_admin: !has_users,
        is_lead_scout: !has_users,
        is_coach: false,
    };

    let now = Utc::now();
    let created = state
        .repo
        .create_user(
            NewUser {
                email,
                name,
                password_hash,
                team_number,
                roles,
            },
            now,
        )
        .await;

    let created = match created {
        Ok(u) => u,
        Err(tt_repo::RepoError::Conflict { what }) => {
            return reject(format!("{what} already exists."));
        }
        Err(e) => {
            tracing::error!("creating user: {e}");
            return reject("Could not create the account. Try again.".into());
        }
    };

    let session = new_session(created.id);
    if let Err(e) = state.repo.create_session(&session, now).await {
        tracing::error!("creating session after signup: {e}");
        // The account exists; send them to sign in rather than pretending.
        return Redirect::to("/sign-in").into_response();
    }

    (jar.add(session_cookie(session.id)), Redirect::to("/")).into_response()
}

// ── Sign out ────────────────────────────────────────────────────────────────

pub async fn logout(State(state): State<AppState>, jar: CookieJar) -> Response {
    if let Some(cookie) = jar.get(SESSION_COOKIE)
        && let Err(e) = state.repo.delete_session(cookie.value()).await
    {
        // The cookie is cleared regardless: a scout signing out on a tablet with
        // no database must still stop being signed in on it.
        tracing::warn!("deleting session: {e}");
    }
    (jar.add(clear_session_cookie()), Redirect::to("/sign-in")).into_response()
}

// ── Account ─────────────────────────────────────────────────────────────────

fn account_page(nav: Nav, user: &tt_core::user::User, error: String, success: String) -> Response {
    html(AccountPage {
        title: "Account".into(),
        nav,
        user_name: user.name.clone(),
        user_email: user.email.clone(),
        team_display: match user.team_number {
            Some(n) => n.to_string(),
            None => "No team".into(),
        },
        role_labels: user.roles.labels(),
        error,
        success,
    })
}

pub async fn account(
    State(state): State<AppState>,
    Auth(user): Auth,
    EventParam(requested): EventParam,
) -> Response {
    let (nav, _) = event_page(&state, Some(&user), requested.as_deref()).await;
    account_page(nav, &user, String::new(), String::new())
}

#[derive(Deserialize)]
pub struct ChangePasswordForm {
    current_password: String,
    new_password: String,
    confirm_password: String,
}

pub async fn change_password(
    State(state): State<AppState>,
    Auth(user): Auth,
    Form(form): Form<ChangePasswordForm>,
) -> Response {
    let nav = nav_for(&state, Some(&user)).await;
    let fail = |message: &str| account_page(nav.clone(), &user, message.into(), String::new());

    if form.new_password != form.confirm_password {
        return fail("The two new passwords do not match.");
    }
    if user::validate_password(&form.new_password).is_err() {
        return fail("New password must be at least 8 characters.");
    }
    if form.current_password == form.new_password {
        return fail("The new password must differ from the current one.");
    }

    let stored = match state.repo.password_hash(user.id).await {
        Ok(Some(h)) => h,
        _ => return fail("Could not change the password. Try again."),
    };
    if !verify_password(&form.current_password, &stored) {
        return fail("Current password is incorrect.");
    }

    let Ok(new_hash) = hash_password(&form.new_password) else {
        return fail("Could not change the password. Try again.");
    };
    if let Err(e) = state
        .repo
        .set_password_hash(user.id, &new_hash, Utc::now())
        .await
    {
        tracing::error!("updating password: {e}");
        return fail("Could not change the password. Try again.");
    }

    account_page(nav, &user, String::new(), "Password changed.".into())
}

// ── Device heartbeat (A5) ───────────────────────────────────────────────────

/// Record that a tablet is present.
///
/// Called by `static/js/device.js` on load and every 60 seconds. The device id
/// arrives in a cookie rather than a body because the server has to be able to
/// read it on ordinary page requests too -- `localStorage` is not visible
/// server-side.
///
/// Always answers 200, even when it could not record anything: a scout's tablet
/// going offline is normal, and a red error in their console helps nobody.
pub async fn device_heartbeat(
    State(state): State<AppState>,
    MaybeAuth(user): MaybeAuth,
    parts: axum::http::request::Parts,
) -> Response {
    let Some(uuid) = device_uuid(&parts.headers) else {
        return axum::Json(serde_json::json!({ "status": "no-device-id" })).into_response();
    };

    match state
        .repo
        .touch_device(&uuid, user.as_ref(), Utc::now())
        .await
    {
        Ok(device) => axum::Json(serde_json::json!({
            "status": "ok",
            "device": device.display_name(),
        }))
        .into_response(),
        Err(e) => {
            tracing::warn!("device heartbeat failed: {e}");
            axum::Json(serde_json::json!({ "status": "not-recorded" })).into_response()
        }
    }
}

// ── Lead scout ──────────────────────────────────────────────────────────────

pub async fn lead_scout(
    State(state): State<AppState>,
    LeadScout(user): LeadScout,
    EventParam(requested): EventParam,
    reviewed: ReviewedParam,
) -> Response {
    lead_scout_page(
        &state,
        &user,
        requested.as_deref(),
        None,
        reviewed.message(),
    )
    .await
}

// ── Assignments (L1, L2) ────────────────────────────────────────────────────

/// `GET /lead-scout/assignments`: who scouts which robot in each match.
pub async fn assignments(
    State(state): State<AppState>,
    LeadScout(user): LeadScout,
    EventParam(requested): EventParam,
    params: GridParams,
) -> Response {
    let (nav, context) = event_page(&state, Some(&user), requested.as_deref()).await;
    html(assignments::page(&state, nav, &context, &params, None, Vec::new()).await)
}

/// Answer an assignment change: a 303 back to the grid, or the grid again
/// with why it did not happen.
async fn assignment_outcome(
    state: &AppState,
    user: &tt_core::user::User,
    requested: Option<&str>,
    outcome: Result<String, Box<assignments::Refused>>,
) -> Response {
    match outcome {
        Ok(next) => Redirect::to(&next).into_response(),
        Err(refused) => {
            let refused = *refused;
            let event = refused.event_key.as_deref().or(requested);
            let (nav, context) = event_page(state, Some(user), event).await;
            html(
                assignments::page(
                    state,
                    nav,
                    &context,
                    &GridParams::default(),
                    refused.draft,
                    refused.errors,
                )
                .await,
            )
        }
    }
}

/// `POST /api/assignments/match`: set one match's six robots.
pub async fn save_match_assignments(
    State(state): State<AppState>,
    LeadScout(user): LeadScout,
    EventParam(requested): EventParam,
    Form(pairs): Form<Vec<(String, String)>>,
) -> Response {
    let outcome = assignments::save_match(&state, &user, &pairs).await;
    assignment_outcome(&state, &user, requested.as_deref(), outcome).await
}

/// `POST /api/assignments/clear-match`.
pub async fn clear_match_assignments(
    State(state): State<AppState>,
    LeadScout(user): LeadScout,
    EventParam(requested): EventParam,
    Form(pairs): Form<Vec<(String, String)>>,
) -> Response {
    let outcome = assignments::clear_match(&state, &user, &pairs).await;
    assignment_outcome(&state, &user, requested.as_deref(), outcome).await
}

/// `POST /api/assignments/clear?event=`.
pub async fn clear_all_assignments(
    State(state): State<AppState>,
    LeadScout(user): LeadScout,
    EventParam(requested): EventParam,
    Form(pairs): Form<Vec<(String, String)>>,
) -> Response {
    let outcome = assignments::clear_all(&state, &user, requested.as_deref(), &pairs).await;
    assignment_outcome(&state, &user, requested.as_deref(), outcome).await
}

/// `POST /api/assignments/auto?event=`.
pub async fn distribute_assignments(
    State(state): State<AppState>,
    LeadScout(user): LeadScout,
    EventParam(requested): EventParam,
    Form(pairs): Form<Vec<(String, String)>>,
) -> Response {
    let outcome = assignments::distribute(&state, &user, requested.as_deref(), &pairs).await;
    assignment_outcome(&state, &user, requested.as_deref(), outcome).await
}

/// `POST /api/devices/{id}/rename?event=`.
pub async fn rename_device(
    State(state): State<AppState>,
    LeadScout(user): LeadScout,
    EventParam(requested): EventParam,
    Path(id): Path<i64>,
    Form(pairs): Form<Vec<(String, String)>>,
) -> Response {
    let outcome = assignments::rename_device(&state, &user, requested.as_deref(), id, &pairs).await;
    assignment_outcome(&state, &user, requested.as_deref(), outcome).await
}

/// `POST /api/frc/sync` (I13): refresh upstream data now.
///
/// One route, two callers. A script gets JSON counts. A browser posting the
/// lead-scout page's form gets that page back with the outcome on it: every
/// browser request is a plain navigation (U8), so a JSON body would be the
/// whole screen.
pub async fn manual_sync(
    State(state): State<AppState>,
    LeadScout(user): LeadScout,
    EventParam(requested): EventParam,
    headers: HeaderMap,
) -> Response {
    tracing::info!(user = %user.email, "manual sync requested");
    let outcome = upstream::sync_now(&state.repo, &state.upstream).await;

    if wants_html(&headers) {
        lead_scout_page(
            &state,
            &user,
            requested.as_deref(),
            Some(&outcome),
            String::new(),
        )
        .await
    } else {
        axum::Json(outcome).into_response()
    }
}

async fn lead_scout_page(
    state: &AppState,
    user: &tt_core::user::User,
    requested: Option<&str>,
    outcome: Option<&ManualSync>,
    reviewed: String,
) -> Response {
    let (nav, context) = event_page(state, Some(user), requested).await;
    html(LeadScoutPage {
        title: "Lead Scout".into(),
        nav,
        season_name: state.season.name.clone(),
        upstream: upstream::panel(&state.upstream, outcome, Utc::now()),
        stored: events::stored(&*state.repo, &context).await,
        queue: review::queue(state, &context).await,
        reviewed,
    })
}

// ── Team profile (U11) ──────────────────────────────────────────────────────

/// `GET /teams?team=`: everything known about a team at the selected event.
pub async fn team(
    State(state): State<AppState>,
    Auth(user): Auth,
    EventParam(requested): EventParam,
    team: TeamParam,
) -> Response {
    let (nav, context) = event_page(&state, Some(&user), requested.as_deref()).await;
    html(teams::page(&state, nav, &user, &context, &team).await)
}

// ── Pick list (U20) ─────────────────────────────────────────────────────────

/// `GET /pick-list`: the viewer's team's list for the selected event.
pub async fn pick_list(
    State(state): State<AppState>,
    Strategist(user): Strategist,
    EventParam(requested): EventParam,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let (nav, context) = event_page(&state, Some(&user), requested.as_deref()).await;
    let removed = query.get("removed").and_then(|n| n.parse().ok());
    html(
        picklist::page(
            &state,
            nav,
            &user,
            &context,
            Vec::new(),
            String::new(),
            removed,
        )
        .await,
    )
}

/// `POST /api/pick-list?event=`: one change.
pub async fn change_pick_list(
    State(state): State<AppState>,
    Strategist(user): Strategist,
    EventParam(requested): EventParam,
    Form(form): Form<Vec<(String, String)>>,
) -> Response {
    let (nav, context) = event_page(&state, Some(&user), requested.as_deref()).await;
    match picklist::change(&state, &user, requested.as_deref(), &context, &form).await {
        Ok(next) => Redirect::to(&next).into_response(),
        Err(error) => {
            // Only an add has typing worth keeping.
            let typed = form
                .iter()
                .any(|(n, v)| n == "op" && v == "add")
                .then(|| form.iter().find(|(n, _)| n == "team"))
                .flatten()
                .map(|(_, v)| v.clone())
                .unwrap_or_default();
            html(picklist::page(&state, nav, &user, &context, vec![error], typed, None).await)
        }
    }
}

// ── Rankings (L11) and point values (L12) ───────────────────────────────────

/// `GET /lead-scout/rankings?sort=`.
pub async fn rankings(
    State(state): State<AppState>,
    LeadScout(user): LeadScout,
    EventParam(requested): EventParam,
    params: RankingParams,
) -> Response {
    let (nav, context) = event_page(&state, Some(&user), requested.as_deref()).await;
    html(ranking::page(&state, nav, &context, params.sort).await)
}

/// `GET /lead-scout/rankings/enter`: type the rankings in (I14).
pub async fn rankings_entry(
    State(state): State<AppState>,
    LeadScout(user): LeadScout,
    EventParam(requested): EventParam,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let (nav, context) = event_page(&state, Some(&user), requested.as_deref()).await;
    let saved = query.get("saved").and_then(|n| n.parse().ok());
    html(standings::page(&state, nav, &context, None, Vec::new(), saved).await)
}

/// `POST /api/rankings/manual?event=`: all of it, or nothing.
pub async fn save_rankings(
    State(state): State<AppState>,
    LeadScout(user): LeadScout,
    EventParam(requested): EventParam,
    Form(pairs): Form<Vec<(String, String)>>,
) -> Response {
    let text = pairs
        .iter()
        .find(|(name, _)| name == "standings")
        .map(|(_, value)| value.as_str())
        .unwrap_or_default();
    let (nav, context) = event_page(&state, Some(&user), requested.as_deref()).await;
    match standings::save(&state, &user, requested.as_deref(), &context, text).await {
        Ok(next) => Redirect::to(&next).into_response(),
        Err(errors) => {
            html(standings::page(&state, nav, &context, Some(text.to_string()), errors, None).await)
        }
    }
}

/// `GET /lead-scout/weights`.
pub async fn weights(
    State(state): State<AppState>,
    LeadScout(user): LeadScout,
    EventParam(requested): EventParam,
    params: RankingParams,
) -> Response {
    let (nav, _) = event_page(&state, Some(&user), requested.as_deref()).await;
    let notice = match params.saved.as_deref() {
        Some("reset") => "Every point value is back to the season's default.",
        Some(_) => "Saved. Rankings use the new values now.",
        None => "",
    };
    html(ranking::weights_page(&state, nav, notice.into(), None).await)
}

fn weights_saved(requested: Option<&str>, what: &str) -> Response {
    let event = requested
        .map(|key| format!("event={key}&"))
        .unwrap_or_default();
    Redirect::to(&format!("/lead-scout/weights?{event}saved={what}")).into_response()
}

/// `POST /api/weights`: the whole form, or nothing.
pub async fn save_weights(
    State(state): State<AppState>,
    LeadScout(user): LeadScout,
    EventParam(requested): EventParam,
    Form(pairs): Form<Vec<(String, String)>>,
) -> Response {
    match ranking::save_weights(&state, &user, &pairs).await {
        Ok(()) => weights_saved(requested.as_deref(), "1"),
        Err(refused) => {
            let (nav, _) = event_page(&state, Some(&user), requested.as_deref()).await;
            let (errors, storage) = match refused {
                ranking::WeightsRefused::Invalid(errors) => (errors, false),
                ranking::WeightsRefused::Storage => (Default::default(), true),
            };
            let mut page =
                ranking::weights_page(&state, nav, String::new(), Some((&pairs, errors))).await;
            if storage {
                page.errors
                    .push("Not saved: the server's storage did not answer. Try again.".into());
            }
            html(page)
        }
    }
}

/// `POST /api/weights/reset`.
pub async fn reset_weights(
    State(state): State<AppState>,
    LeadScout(user): LeadScout,
    EventParam(requested): EventParam,
) -> Response {
    if ranking::reset_weights(&state, &user).await {
        weights_saved(requested.as_deref(), "reset")
    } else {
        let (nav, _) = event_page(&state, Some(&user), requested.as_deref()).await;
        let mut page = ranking::weights_page(&state, nav, String::new(), None).await;
        page.errors
            .push("Not reset: the server's storage did not answer. Try again.".into());
        html(page)
    }
}

// ── Review (L8-L10) ─────────────────────────────────────────────────────────

/// `GET /lead-scout/submissions/{id}`: one observation in full.
pub async fn review_page(
    State(state): State<AppState>,
    LeadScout(user): LeadScout,
    EventParam(requested): EventParam,
    reviewed: ReviewedParam,
    Path(id): Path<i64>,
) -> Response {
    let (nav, _) = event_page(&state, Some(&user), requested.as_deref()).await;
    match review::page(
        &state,
        nav,
        &user,
        id,
        reviewed.message(),
        Vec::new(),
        String::new(),
    )
    .await
    {
        Some(page) => html(page),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// `POST /api/observations/{id}/approve` and `.../decline`.
async fn verdict(
    state: AppState,
    user: tt_core::user::User,
    requested: Option<String>,
    id: i64,
    pairs: Vec<(String, String)>,
    decline: bool,
) -> Response {
    match review::decide(&state, &user, id, &pairs, decline).await {
        Ok(next) => Redirect::to(&next).into_response(),
        Err(review::Refused::Missing) => StatusCode::NOT_FOUND.into_response(),
        Err(review::Refused::Again { errors, reason }) => {
            let (nav, _) = event_page(&state, Some(&user), requested.as_deref()).await;
            match review::page(&state, nav, &user, id, String::new(), errors, reason).await {
                Some(page) => html(page),
                None => StatusCode::NOT_FOUND.into_response(),
            }
        }
    }
}

pub async fn approve_observation(
    State(state): State<AppState>,
    LeadScout(user): LeadScout,
    EventParam(requested): EventParam,
    Path(id): Path<i64>,
    Form(pairs): Form<Vec<(String, String)>>,
) -> Response {
    verdict(state, user, requested, id, pairs, false).await
}

pub async fn decline_observation(
    State(state): State<AppState>,
    LeadScout(user): LeadScout,
    EventParam(requested): EventParam,
    Path(id): Path<i64>,
    Form(pairs): Form<Vec<(String, String)>>,
) -> Response {
    verdict(state, user, requested, id, pairs, true).await
}

/// Whether the caller is a browser expecting a page, rather than a script
/// expecting data. Browsers put `text/html` in `Accept` on every navigation and
/// form post; `fetch` and `curl` send `*/*`.
pub(crate) fn wants_html(headers: &HeaderMap) -> bool {
    headers
        .get(ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("text/html"))
}

// ── Scouting (U4) ───────────────────────────────────────────────────────────

pub async fn submission(
    State(state): State<AppState>,
    Auth(user): Auth,
    EventParam(requested): EventParam,
    params: ScoutParams,
    headers: HeaderMap,
) -> Response {
    let (nav, context) = event_page(&state, Some(&user), requested.as_deref()).await;
    let device = scouting::device_id(&state, device_uuid(&headers).as_deref()).await;
    html(
        scouting::page(
            &state,
            &user,
            device,
            nav,
            &context,
            &params,
            None,
            Vec::new(),
        )
        .await,
    )
}

/// `POST /api/submission`: save an observation, then show the next step.
///
/// Post/redirect/get on success, so a reload of the confirmation cannot post
/// again. A failed save re-renders the page with the answers kept.
pub async fn submit_observation(
    State(state): State<AppState>,
    Auth(user): Auth,
    EventParam(requested): EventParam,
    headers: HeaderMap,
    Form(pairs): Form<Vec<(String, String)>>,
) -> Response {
    let device = device_uuid(&headers);
    match scouting::submit(&state, &user, device.as_deref(), &pairs).await {
        Ok(next) => Redirect::to(&next).into_response(),
        Err(rejected) => {
            let rejected = *rejected;
            let event = rejected.event_key.as_deref().or(requested.as_deref());
            let (nav, context) = event_page(&state, Some(&user), event).await;
            let device = scouting::device_id(&state, device.as_deref()).await;
            html(
                scouting::page(
                    &state,
                    &user,
                    device,
                    nav,
                    &context,
                    &rejected.params,
                    rejected.draft,
                    rejected.errors,
                )
                .await,
            )
        }
    }
}

// ── Drive coach (U18) ───────────────────────────────────────────────────────

/// `GET /drive-coach`: the coach's team's matches, from the local schedule.
pub async fn drive_coach(
    State(state): State<AppState>,
    Coach(user): Coach,
    EventParam(requested): EventParam,
) -> Response {
    let (nav, context) = event_page(&state, Some(&user), requested.as_deref()).await;
    html(coach::page(&state, &user, nav, &context).await)
}
