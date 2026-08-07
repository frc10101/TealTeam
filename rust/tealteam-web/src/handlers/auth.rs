// Sign-in/sign-up pages, login/signup/logout endpoints, account page and
// password change (port of Controllers/AuthController.cs).

use askama::Template;
use axum::extract::State;
use axum::response::{Html, IntoResponse};
use axum_extra::extract::cookie::CookieJar;
use chrono::Utc;
use tracing::{error, info, warn};

use super::*;
use crate::models::User;
use crate::session;
use crate::state::SharedState;

#[derive(Template)]
#[template(path = "pages/sign_in.html")]
struct SignInTemplate {
    title: String,
    nav: Nav,
}

#[derive(Template)]
#[template(path = "pages/sign_up.html")]
struct SignUpTemplate {
    title: String,
    nav: Nav,
}

#[derive(Template)]
#[template(path = "pages/account.html")]
struct AccountTemplate {
    title: String,
    nav: Nav,
    user_name: String,
    user_email: String,
    team_display: String,
    role_badges: Vec<String>,
}

pub async fn sign_in_page(State(state): State<SharedState>, jar: CookieJar) -> HandlerResult {
    if current_user(&state.pool, &jar).await.is_some() {
        return Ok(redirect("/"));
    }
    Ok(render(&SignInTemplate {
        title: "Sign In".to_string(),
        nav: Nav::default(),
    }))
}

pub async fn sign_up_page(State(state): State<SharedState>, jar: CookieJar) -> HandlerResult {
    if current_user(&state.pool, &jar).await.is_some() {
        return Ok(redirect("/"));
    }
    Ok(render(&SignUpTemplate {
        title: "Sign Up".to_string(),
        nav: Nav::default(),
    }))
}

pub async fn login(
    State(state): State<SharedState>,
    jar: CookieJar,
    body: String,
) -> HandlerResult {
    let form = form_map(&body);
    let email = form_str(&form, "email").trim().to_string();
    let password = form_str(&form, "password").to_string();

    if email.is_empty() || password.is_empty() {
        return Ok(auth_error("Email and password are required"));
    }

    let user: Option<User> = match sqlx::query_as("SELECT * FROM users WHERE email = $1")
        .bind(&email)
        .fetch_optional(&state.pool)
        .await
    {
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
    if !session::check_password_hash(&password, &user.password_hash) {
        return Ok(auth_error("Invalid email or password"));
    }

    let session_id = match session::create_session(&state.pool, user.id).await {
        Ok(id) => id,
        Err(e) => {
            error!("failed to create session: {e}");
            return Ok(auth_error("Failed to create session"));
        }
    };

    if let Err(e) = sqlx::query("UPDATE users SET last_login = $1 WHERE id = $2")
        .bind(Utc::now())
        .bind(user.id)
        .execute(&state.pool)
        .await
    {
        warn!("failed to update last login for user {}: {e}", user.id);
    }

    if let Some(team_number) = user.team_number.filter(|n| *n > 0) {
        let pool = state.pool.clone();
        tokio::spawn(async move {
            let sync = crate::first_sync::sync_team_for_user(&pool, team_number);
            match tokio::time::timeout(std::time::Duration::from_secs(60), sync).await {
                Ok(Ok(_)) => {}
                Ok(Err(e)) => error!("failed to sync team data on login (team {team_number}): {e}"),
                Err(_) => error!("team sync timed out on login (team {team_number})"),
            }
        });
    }

    let jar = jar.add(session::session_cookie(session_id));
    Ok((jar, up_navigate("/")).into_response())
}

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

    let existing: Result<Option<i32>, _> =
        sqlx::query_scalar("SELECT id FROM users WHERE email = $1 LIMIT 1")
            .bind(&email)
            .fetch_optional(&state.pool)
            .await;
    match existing {
        Ok(Some(_)) => {
            return Ok(auth_error("An account with this email already exists"));
        }
        Ok(None) => {}
        Err(e) => {
            error!("database error checking existing user: {e}");
            return Ok(auth_error("An error occurred. Please try again."));
        }
    }

    let password_hash = session::hash_password(&password)?;

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

    let user_id: i32 = match sqlx::query_scalar(
        "INSERT INTO users (name, email, password_hash, team_number, role, is_lead_scout, is_coach)
         VALUES ($1, $2, $3, $4, 'user', $5, $6)
         RETURNING id",
    )
    .bind(&name)
    .bind(&email)
    .bind(&password_hash)
    .bind(parsed_team_number)
    .bind(lead_scout)
    .bind(coach)
    .fetch_one(&state.pool)
    .await
    {
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
            let sync = crate::first_sync::sync_team_for_user(&pool, team_number);
            match tokio::time::timeout(std::time::Duration::from_secs(60), sync).await {
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

pub async fn account_page(State(state): State<SharedState>, jar: CookieJar) -> HandlerResult {
    let Some(user) = current_user(&state.pool, &jar).await else {
        return Ok(redirect("/sign-in"));
    };

    let mut role_badges = Vec::new();
    if user.is_admin {
        role_badges.push("Admin".to_string());
    }
    if user.is_lead_scout {
        role_badges.push("Lead Scout".to_string());
    }
    if user.is_coach {
        role_badges.push("Drive Coach".to_string());
    }
    if role_badges.is_empty() {
        role_badges.push("Scout".to_string());
    }

    Ok(render(&AccountTemplate {
        title: "Account Settings".to_string(),
        nav: Nav::from_user(Some(&user)),
        user_name: user.name.clone(),
        user_email: user.email.clone(),
        team_display: user
            .team_number
            .map(|n| n.to_string())
            .unwrap_or_else(|| "No team".to_string()),
        role_badges,
    }))
}

pub async fn change_password(
    State(state): State<SharedState>,
    jar: CookieJar,
    body: String,
) -> HandlerResult {
    let Some(user) = current_user(&state.pool, &jar).await else {
        return Ok(password_change_response(false, "Please log in to change your password"));
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
    if !session::check_password_hash(&current_password, &user.password_hash) {
        return Ok(password_change_response(false, "Current password is incorrect"));
    }

    let new_hash = session::hash_password(&new_password)?;
    if let Err(e) = sqlx::query("UPDATE users SET password_hash = $1 WHERE id = $2")
        .bind(&new_hash)
        .bind(user.id)
        .execute(&state.pool)
        .await
    {
        error!("failed to update password for user {}: {e}", user.id);
        return Ok(password_change_response(
            false,
            "Failed to update password. Please try again.",
        ));
    }

    info!("password changed for user {} ({})", user.id, user.email);
    Ok(password_change_response(true, "Password changed successfully!"))
}

fn password_change_response(success: bool, message: &str) -> axum::response::Response {
    let encoded = html_escape::encode_text(message);
    // Wrapped so the response root matches up-target="#password-response". On
    // success, up-on-inserted resets the form (Unpoly does not run <script> in
    // swapped fragments).
    let html = if success {
        format!(
            r##"<div id="password-response" up-on-inserted="document.getElementById('change-password-form').reset()">
    <div class="bg-green-900/20 border border-green-500 text-green-300 px-4 py-3 rounded" role="alert">
        <div class="flex items-center gap-2">
            <svg class="w-5 h-5 flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12l2 2 4-4m6 2a9 9 0 11-18 0 9 9 0 0118 0z"></path>
            </svg>
            <span class="block sm:inline font-medium">{encoded}</span>
        </div>
    </div>
</div>"##
        )
    } else {
        format!(
            r##"<div id="password-response">
    <div class="bg-red-900/20 border border-red-500 text-red-300 px-4 py-3 rounded" role="alert">
        <div class="flex items-center gap-2">
            <svg class="w-5 h-5 flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"></path>
            </svg>
            <span class="block sm:inline font-medium">{encoded}</span>
        </div>
    </div>
</div>"##
        )
    };
    Html(html).into_response()
}
