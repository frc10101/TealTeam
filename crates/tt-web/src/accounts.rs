//! Account administration (G1): `/admin/users`.
//!
//! Before this, the first sign-up was the only admin and every later lead
//! scout or coach took hand-written SQL on the Pi. Every route here takes the
//! `Admin` extractor (`crate::auth`), so nobody else reaches them; the two
//! rules that matter -- the last admin stays one, and a reset password ends
//! that account's sessions and offline tokens -- are kept by
//! `tt_repo_sqlite::accounts`, not here.
//!
//! Each change redirects back to the page with what happened in the URL, so
//! a reload shows the list again rather than posting twice. The new password
//! is never in that URL.

use chrono::Utc;
use tracing::{info, warn};
use tt_core::connectivity::describe_age;
use tt_core::user::{self, Roles, User};
use tt_repo_sqlite::accounts::{Account, AccountChange};
use tt_templates::{AccountRow, AdminUsersPage, Nav};

use crate::auth::hash_password;
use crate::startup::AppState;

/// What the change that led here did.
///
/// Read from the URL, so it is only a message: a crafted link can make the
/// page say "Saved Kim" to the admin who crafted it, and change nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Done {
    Saved,
    Reset,
    LastAdmin,
    BadTeam,
    ShortPassword,
    Gone,
    Failed,
}

impl Done {
    const ALL: [(Self, &'static str); 7] = [
        (Self::Saved, "saved"),
        (Self::Reset, "reset"),
        (Self::LastAdmin, "last-admin"),
        (Self::BadTeam, "bad-team"),
        (Self::ShortPassword, "short-password"),
        (Self::Gone, "gone"),
        (Self::Failed, "failed"),
    ];

    fn code(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(done, _)| *done == self)
            .map(|(_, code)| *code)
            .unwrap_or("failed")
    }

    pub fn parse(code: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .find(|(_, c)| *c == code)
            .map(|(done, _)| *done)
    }

    fn is_error(self) -> bool {
        !matches!(self, Self::Saved | Self::Reset)
    }

    fn message(self, name: &str) -> String {
        match self {
            Self::Saved => format!("Saved {name}."),
            Self::Reset => format!(
                "{name}'s password is reset, and they are signed out everywhere, every \
                 tablet's offline sign-in included. Tell them the new one."
            ),
            Self::LastAdmin => format!(
                "{name} is the only admin, so stays one. Make someone else an admin first. \
                 Nothing was changed."
            ),
            Self::BadTeam => {
                "Team number must be a whole number, 1 or more, and not too large. Nothing was changed."
                    .into()
            }
            Self::ShortPassword => format!(
                "A password must be at least {} characters. {name}'s was not changed.",
                user::MIN_PASSWORD_LEN
            ),
            Self::Gone => "That account no longer exists.".into(),
            Self::Failed => "Could not save that. Try again.".into(),
        }
    }
}

/// Back to the list, saying what happened to account `id`, scrolled to it.
fn back(event_query: &str, done: Done, id: i64) -> String {
    let sep = if event_query.is_empty() { '?' } else { '&' };
    format!(
        "/admin/users{event_query}{sep}done={}&id={id}#account-{id}",
        done.code()
    )
}

fn row(account: &Account, you: &User, event_query: &str, now: chrono::DateTime<Utc>) -> AccountRow {
    AccountRow {
        id: account.id,
        name: account.name.clone(),
        email: account.email.clone(),
        team_number: account
            .team_number
            .map(|n| n.to_string())
            .unwrap_or_default(),
        role_labels: account.roles.labels(),
        is_admin: account.roles.is_admin,
        is_lead_scout: account.roles.is_lead_scout,
        is_coach: account.roles.is_coach,
        last_sign_in: account
            .last_login_at
            .map(|at| describe_age(now - at))
            .unwrap_or_else(|| "Never".into()),
        is_you: account.id == you.id,
        save_action: format!("/api/admin/users/{}{event_query}", account.id),
        reset_action: format!("/api/admin/users/{}/password{event_query}", account.id),
    }
}

/// `GET /admin/users`.
pub async fn page(
    state: &AppState,
    you: &User,
    nav: Nav,
    done: Option<(Done, i64)>,
) -> AdminUsersPage {
    let event_query = nav.event.query();
    let mut out = AdminUsersPage {
        title: "Accounts".into(),
        nav,
        unavailable: String::new(),
        notice: String::new(),
        notice_is_error: false,
        accounts: Vec::new(),
    };
    let accounts = match state.repo.accounts().await {
        Ok(accounts) => accounts,
        Err(e) => {
            warn!("listing accounts: {e}");
            out.unavailable = "Could not read the accounts. Reload to try again.".into();
            return out;
        }
    };
    if let Some((done, id)) = done {
        let name = accounts
            .iter()
            .find(|a| a.id == id)
            .map(|a| a.name.as_str())
            .unwrap_or("That account");
        out.notice = done.message(name);
        out.notice_is_error = done.is_error();
    }
    let now = Utc::now();
    out.accounts = accounts
        .iter()
        .map(|a| row(a, you, &event_query, now))
        .collect();
    out
}

fn ticked(pairs: &[(String, String)], name: &str) -> bool {
    pairs
        .iter()
        .any(|(key, value)| key == name && value == "on")
}

fn posted<'a>(pairs: &'a [(String, String)], name: &str) -> &'a str {
    pairs
        .iter()
        .rev()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
        .unwrap_or_default()
}

/// `POST /api/admin/users/{id}`: roles and team number, together.
pub async fn save(
    state: &AppState,
    admin: &User,
    event_query: &str,
    id: i64,
    pairs: &[(String, String)],
) -> String {
    let Ok(team_number) = user::validate_team_number(posted(pairs, "team_number")) else {
        return back(event_query, Done::BadTeam, id);
    };
    let roles = Roles {
        is_admin: ticked(pairs, "is_admin"),
        is_lead_scout: ticked(pairs, "is_lead_scout"),
        is_coach: ticked(pairs, "is_coach"),
    };
    let done = match state
        .repo
        .set_account(id, team_number, roles, Utc::now())
        .await
    {
        Ok(AccountChange::Saved) => {
            info!(user = %admin.email, account = id, ?roles, ?team_number, "account changed");
            Done::Saved
        }
        Ok(AccountChange::LastAdmin) => Done::LastAdmin,
        Ok(AccountChange::NoSuchAccount) => Done::Gone,
        Err(e) => {
            warn!("changing account {id}: {e}");
            Done::Failed
        }
    };
    back(event_query, done, id)
}

/// `POST /api/admin/users/{id}/password`: a forgotten password. The account
/// is signed out everywhere in the same step.
pub async fn reset_password(
    state: &AppState,
    admin: &User,
    event_query: &str,
    id: i64,
    pairs: &[(String, String)],
) -> String {
    let password = posted(pairs, "new_password");
    if user::validate_password(password).is_err() {
        return back(event_query, Done::ShortPassword, id);
    }
    let Ok(hash) = hash_password(password) else {
        tracing::error!("hashing a reset password failed");
        return back(event_query, Done::Failed, id);
    };
    let done = match state
        .repo
        .reset_password(id, &hash, admin.id, Utc::now())
        .await
    {
        Ok(true) => {
            info!(user = %admin.email, account = id, "password reset; signed out everywhere");
            Done::Reset
        }
        Ok(false) => Done::Gone,
        Err(e) => {
            warn!("resetting account {id}'s password: {e}");
            Done::Failed
        }
    };
    back(event_query, done, id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_notice_reads_back_as_itself() {
        for (done, code) in Done::ALL {
            assert_eq!(done.code(), code);
            assert_eq!(Done::parse(code), Some(done));
        }
        assert_eq!(Done::parse("nonsense"), None);
    }

    #[test]
    fn the_way_back_keeps_the_event_and_never_carries_a_password() {
        assert_eq!(
            back("", Done::Saved, 4),
            "/admin/users?done=saved&id=4#account-4"
        );
        assert_eq!(
            back("?event=2026miket", Done::Reset, 4),
            "/admin/users?event=2026miket&done=reset&id=4#account-4"
        );
    }
}
