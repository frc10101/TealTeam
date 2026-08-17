//! Sign-in, sign-up and account pages, plus the inline alert fragments the
//! auth forms swap in via Unpoly.
//!
//! The two alert helpers below are built with `format!` rather than templates
//! because they are the response body for a failed form post: the browser
//! swaps them into a known element id and nothing else on the page changes.
//! Both escape their message — the text can contain user input.

use askama::Template;
use axum::response::{Html, IntoResponse, Response};

use super::Nav;
use crate::models::User;

/// Sign-in form.
#[derive(Template)]
#[template(path = "pages/sign_in.html")]
pub struct SignInTemplate {
    pub title: String,
    pub nav: Nav,
}

/// Sign-up form, including team number and the lead scout / coach checkboxes.
#[derive(Template)]
#[template(path = "pages/sign_up.html")]
pub struct SignUpTemplate {
    pub title: String,
    pub nav: Nav,
}

/// Account settings: identity, roles, and the change-password form.
#[derive(Template)]
#[template(path = "pages/account.html")]
pub struct AccountTemplate {
    pub title: String,
    pub nav: Nav,
    pub user_name: String,
    pub user_email: String,
    pub team_display: String,
    pub role_badges: Vec<String>,
}

impl AccountTemplate {
    /// Presents a user's profile, labelling their roles. Everyone gets at
    /// least the "Scout" badge, since every account can scout.
    pub fn for_user(user: &User) -> Self {
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

        Self {
            title: "Account Settings".to_string(),
            nav: Nav::from_user(Some(user)),
            user_name: user.name.clone(),
            user_email: user.email.clone(),
            team_display: user
                .team_number
                .map(|n| n.to_string())
                .unwrap_or_else(|| "No team".to_string()),
            role_badges,
        }
    }
}

/// Failure fragment for the auth forms, wrapped so its root matches the
/// up-target (`#form-response`). On success the handlers navigate instead.
pub fn auth_error(message: &str) -> Response {
    let encoded = html_escape::encode_text(message);
    let html = format!(
        r##"<div id="form-response" class="fade-swap">
    <div class="bg-red-900/20 border border-red-500 text-red-300 px-4 py-3 rounded mb-4" role="alert">
        <div class="flex items-center gap-2">
            <svg class="w-5 h-5 flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"></path>
            </svg>
            <span class="block sm:inline font-medium">{encoded}</span>
        </div>
    </div>
</div>"##
    );
    Html(html).into_response()
}

/// Result banner for the change-password form: green on success (and resets
/// the form), red otherwise.
pub fn password_change_response(success: bool, message: &str) -> Response {
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
