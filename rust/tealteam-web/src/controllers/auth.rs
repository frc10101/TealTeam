//! Sign-in and sign-up pages, the login/signup/logout endpoints, the account
//! page and password change.
//!
//! Form posts here answer with an inline banner fragment on failure and an
//! Unpoly navigation on success, so the browser lands on a fresh page with the
//! new session cookie already set.
//!
//! Sign-in and sign-up kick off a background FIRST sync for the user's team
//! ([`crate::services::first_sync::sync_team_for_user`]) so a new account has
//! its events and roster populated by the time it reaches the home page. It is
//! spawned rather than awaited — a slow or unreachable FIRST API must not
//! delay signing in — and bounded by [`TEAM_SYNC_TIMEOUT`].
//!
//! Failure messages deliberately do not distinguish an unknown email from a
//! wrong password, to avoid confirming which addresses have accounts.

use axum::extract::State;
use axum::response::IntoResponse;
use axum_extra::extract::cookie::CookieJar;
use tracing::{error, info, warn};

use crate::models::session;
use crate::models::user::{self, NewUser};
use crate::services::first_sync;
use crate::state::SharedState;
use crate::views::auth::{
    auth_error, password_change_response, AccountTemplate, SignInTemplate, SignUpTemplate,
};
use crate::views::{render, Nav};
use crate::web::*;

/// FIRST sync on login/signup runs in the background with this ceiling.
const TEAM_SYNC_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// `GET /sign-in` — sign-in form, or home if already signed in.
pub async fn sign_in_page(State(state): State<SharedState>, jar: CookieJar) -> HandlerResult {
    if current_user(&state.pool, &jar).await.is_some() {
        return Ok(redirect("/"));
    }
    Ok(render(&SignInTemplate {
        title: "Sign In".to_string(),
        nav: Nav::default(),
    }))
}

/// `GET /sign-up` — sign-up form, or home if already signed in.
pub async fn sign_up_page(State(state): State<SharedState>, jar: CookieJar) -> HandlerResult {
    if current_user(&state.pool, &jar).await.is_some() {
        return Ok(redirect("/"));
    }
    Ok(render(&SignUpTemplate {
        title: "Sign Up".to_string(),
        nav: Nav::default(),
    }))
}

/// `POST /api/auth/login` — verifies credentials, opens a session, sets the
/// cookie and navigates home.
pub async fn login(State(state): State<SharedState>, jar: CookieJar, body: String) -> HandlerResult {
    let form = form_map(&body);
    let email = form_str(&form, "email").trim().to_string();
    let password = form_str(&form, "password").to_string();

    if email.is_empty() || password.is_empty() {
        return Ok(auth_error("Email and password are required"));
    }

    let user = match user::find_by_email(&state.pool, &email).await {
        Ok(u) => u,
        Err(e) => {
            error!("database error during login: {e}");
            return Ok(auth_error("An error occurred. Please try again."));
        }
    };

    // Generic error message to prevent user enumeration.
    let Some(user) = user else {
        return Ok(auth_error("Invalid email or password"));
    };
    if !user::check_password_hash(&password, &user.password_hash) {
        return Ok(auth_error("Invalid email or password"));
    }

    let session_id = match session::create_session(&state.pool, user.id).await {
        Ok(id) => id,
        Err(e) => {
            error!("failed to create session: {e}");
            return Ok(auth_error("Failed to create session"));
        }
    };

    if let Err(e) = user::touch_last_login(&state.pool, user.id).await {
        warn!("failed to update last login for user {}: {e}", user.id);
    }

    if let Some(team_number) = user.active_team_number() {
        let pool = state.pool.clone();
        tokio::spawn(async move {
            let sync = first_sync::sync_team_for_user(&pool, team_number);
            match tokio::time::timeout(TEAM_SYNC_TIMEOUT, sync).await {
                Ok(Ok(_)) => {}
                Ok(Err(e)) => error!("failed to sync team data on login (team {team_number}): {e}"),
                Err(_) => error!("team sync timed out on login (team {team_number})"),
            }
        });
    }

    let jar = jar.add(session::session_cookie(session_id));
    Ok((jar, up_navigate("/")).into_response())
}

/// `POST /api/auth/signup` — validates the form, creates the account and
/// signs the new user straight in.
///
/// If the session cannot be created the account still exists, so the user is
/// sent to the sign-in page rather than shown a failure.
pub async fn signup(
    State(state): State<SharedState>,
    jar: CookieJar,
    body: String,
) -> HandlerResult {
    let form = form_map(&body);
    let name = form_str(&form, "name").trim().to_string();
    let email = form_str(&form, "email").trim().to_string();
    let password = form_str(&form, "password").to_string();
    let confirm_password = form_str(&form, "confirm-password").to_string();
    let team_number_raw = form_str(&form, "team-number").trim().to_string();
    let lead_scout = !form_str(&form, "lead-scout").trim().is_empty();
    let coach = !form_str(&form, "coach").trim().is_empty();

    if name.is_empty() || email.is_empty() || password.is_empty() || confirm_password.is_empty() {
        return Ok(auth_error("All fields are required"));
    }
    if password != confirm_password {
        return Ok(auth_error("Passwords do not match"));
    }
    if password.len() < 8 {
        return Ok(auth_error("Password must be at least 8 characters long"));
    }
    if !email.contains('@') || !email.contains('.') {
        return Ok(auth_error("Invalid email format"));
    }

    match user::email_exists(&state.pool, &email).await {
        Ok(true) => return Ok(auth_error("An account with this email already exists")),
        Ok(false) => {}
        Err(e) => {
            error!("database error checking existing user: {e}");
            return Ok(auth_error("An error occurred. Please try again."));
        }
    }

    let password_hash = user::hash_password(&password)?;

    let parsed_team_number: Option<i32> = if team_number_raw.is_empty() {
        None
    } else {
        match team_number_raw.parse() {
            Ok(v) => Some(v),
            Err(_) => {
                warn!("invalid team number on signup: {email} {team_number_raw}");
                None
            }
        }
    };

    let new_user = NewUser {
        name: &name,
        email: &email,
        password_hash: &password_hash,
        team_number: parsed_team_number,
        lead_scout,
        coach,
    };
    let user_id = match user::create(&state.pool, &new_user).await {
        Ok(id) => id,
        Err(e) => {
            error!("failed to create user {email}: {e}");
            let msg = if e.to_string().contains("duplicate") || e.to_string().contains("unique") {
                "An account with this email already exists"
            } else {
                "Failed to create account. Please try again."
            };
            return Ok(auth_error(msg));
        }
    };

    if let Some(team_number) = parsed_team_number.filter(|n| *n > 0) {
        info!("user signed up: {user_id} {email} team {team_number}");
        let pool = state.pool.clone();
        tokio::spawn(async move {
            let sync = first_sync::sync_team_for_user(&pool, team_number);
            match tokio::time::timeout(TEAM_SYNC_TIMEOUT, sync).await {
                Ok(Ok(result)) => info!(
                    "synced team on signup: team {team_number} events={} teams={} event_teams={}",
                    result.events, result.teams, result.event_teams
                ),
                Ok(Err(e)) => error!("failed to sync team on signup (team {team_number}): {e}"),
                Err(_) => error!("team sync timed out on signup (team {team_number})"),
            }
        });
    }

    let session_id = match session::create_session(&state.pool, user_id).await {
        Ok(id) => id,
        Err(e) => {
            error!("failed to create session on signup: {e}");
            return Ok(up_navigate("/sign-in"));
        }
    };

    let jar = jar.add(session::session_cookie(session_id));
    Ok((jar, up_navigate("/")).into_response())
}

/// `POST /api/auth/logout` — deletes the session row and clears the cookie.
pub async fn logout(State(state): State<SharedState>, jar: CookieJar) -> HandlerResult {
    if let Some(cookie) = jar.get(session::COOKIE_NAME) {
        let session_id = cookie.value().to_string();
        if !session_id.is_empty() {
            if let Err(e) = session::delete_session(&state.pool, &session_id).await {
                warn!("failed to delete session on logout: {e}");
            }
        }
    }

    let jar = jar.add(session::clear_session_cookie());
    Ok((jar, up_navigate("/sign-in")).into_response())
}

/// `GET /account` — profile, roles and the change-password form.
pub async fn account_page(State(state): State<SharedState>, jar: CookieJar) -> HandlerResult {
    let Some(user) = current_user(&state.pool, &jar).await else {
        return Ok(redirect("/sign-in"));
    };
    Ok(render(&AccountTemplate::for_user(&user)))
}

/// `POST /api/account/change-password` — checks the current password and the
/// new one's rules, then stores it. Answers with a banner either way.
pub async fn change_password(
    State(state): State<SharedState>,
    jar: CookieJar,
    body: String,
) -> HandlerResult {
    let Some(user) = current_user(&state.pool, &jar).await else {
        return Ok(password_change_response(
            false,
            "Please log in to change your password",
        ));
    };

    let form = form_map(&body);
    let current_password = form_str(&form, "current-password").to_string();
    let new_password = form_str(&form, "new-password").to_string();
    let confirm_password = form_str(&form, "confirm-password").to_string();

    if current_password.is_empty() || new_password.is_empty() || confirm_password.is_empty() {
        return Ok(password_change_response(false, "All fields are required"));
    }
    if new_password.len() < 8 {
        return Ok(password_change_response(
            false,
            "New password must be at least 8 characters long",
        ));
    }
    if new_password != confirm_password {
        return Ok(password_change_response(false, "New passwords do not match"));
    }
    if current_password == new_password {
        return Ok(password_change_response(
            false,
            "New password must be different from your current password",
        ));
    }
    if !user::check_password_hash(&current_password, &user.password_hash) {
        return Ok(password_change_response(
            false,
            "Current password is incorrect",
        ));
    }

    let new_hash = user::hash_password(&new_password)?;
    if let Err(e) = user::update_password(&state.pool, user.id, &new_hash).await {
        error!("failed to update password for user {}: {e}", user.id);
        return Ok(password_change_response(
            false,
            "Failed to update password. Please try again.",
        ));
    }

    info!("password changed for user {} ({})", user.id, user.email);
    Ok(password_change_response(
        true,
        "Password changed successfully!",
    ))
}
