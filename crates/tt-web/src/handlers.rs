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

use axum::extract::{Form, State};
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::cookie::CookieJar;
use chrono::Utc;
use serde::Deserialize;
use tt_core::user::{self, Roles};
use tt_repo::{NewUser, Repo};
use tt_templates::{AccountPage, HomePage, Nav, Page, SignInPage, SignUpPage};

use crate::auth::{
    Auth, Coach, LeadScout, MaybeAuth, SESSION_COOKIE, clear_session_cookie, device_uuid,
    hash_password, new_session, session_cookie, verify_password,
};
use crate::startup::AppState;

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

async fn nav_for(state: &AppState, user: Option<&tt_core::user::User>) -> Nav {
    Nav::for_user(user, state.repo.health().await.is_ready())
}

fn team_display(team_number: Option<i32>) -> String {
    team_number.map(|n| n.to_string()).unwrap_or_default()
}

// ── Home ────────────────────────────────────────────────────────────────────

pub async fn home(State(state): State<AppState>, MaybeAuth(user): MaybeAuth) -> Response {
    html(HomePage {
        title: "Home".into(),
        nav: nav_for(&state, user.as_ref()).await,
        team_display: team_display(user.as_ref().and_then(|u| u.team_number)),
        season_name: state.season.name.clone(),
        season_year: state.season.season,
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

pub async fn account(State(state): State<AppState>, Auth(user): Auth) -> Response {
    let nav = nav_for(&state, Some(&user)).await;
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
    let Some(uuid) = device_uuid(&parts) else {
        return axum::Json(serde_json::json!({ "status": "no-device-id" })).into_response();
    };

    let team = user.and_then(|u| u.team_number);
    match state.repo.touch_device(&uuid, team, Utc::now()).await {
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

// ── Role-guarded pages ──────────────────────────────────────────────────────
//
// The nav links these for users who hold the role, so they must exist. Their
// content arrives in phase 2 (L1-L12, U18-U20); what matters now is that the
// guard is on the handler, so the access rule is settled before the page has
// anything worth protecting.

pub async fn lead_scout(State(state): State<AppState>, LeadScout(user): LeadScout) -> Response {
    placeholder(
        &state,
        &user,
        "Lead Scout",
        "Assignments, review queue, and rankings",
    )
    .await
}

pub async fn drive_coach(State(state): State<AppState>, Coach(user): Coach) -> Response {
    placeholder(
        &state,
        &user,
        "Drive Coach",
        "Match schedule and alliance partners",
    )
    .await
}

pub async fn submission(State(state): State<AppState>, Auth(user): Auth) -> Response {
    placeholder(&state, &user, "Scout", "The scouting form for this match").await
}

async fn placeholder(
    state: &AppState,
    user: &tt_core::user::User,
    title: &str,
    summary: &str,
) -> Response {
    html(tt_templates::PlaceholderPage {
        title: title.to_string(),
        nav: nav_for(state, Some(user)).await,
        heading: title.to_string(),
        summary: summary.to_string(),
        season_name: state.season.name.clone(),
    })
}
