//! Askama templates and their view models.
//!
//! Askama compiles templates to Rust at build time, so a malformed template is a
//! build error rather than a runtime 500 -- worth a great deal on a team of
//! high-school students (REBUILD_SPEC.md 7).
//!
//! This crate has no server dependencies and is held to the wasm32 gate along
//! with `tt-core`, because that is what allows rendering to move into a service
//! worker later without rewriting the UI (ACTION_ITEMS.md C5/C6).
//!
//! View models live beside their templates: handlers assemble a struct, the
//! template consumes it, and no template ever reaches back into storage.

use askama::Template;
use tt_core::user::User;

/// A template failed to render.
///
/// Wraps the engine's error so callers never name it. `tt-web` depends on this
/// crate, not on Askama -- swapping engines is then a change here and nowhere
/// else.
#[derive(Debug, thiserror::Error)]
#[error("template render failed: {0}")]
pub struct RenderError(#[from] askama::Error);

/// Renders to a complete HTML string.
///
/// Blanket-implemented for every Askama template in this crate, so adding a page
/// costs one `#[derive(Template)]` and nothing else.
pub trait Page {
    fn render_html(&self) -> Result<String, RenderError>;
}

impl<T: Template> Page for T {
    fn render_html(&self) -> Result<String, RenderError> {
        Ok(self.render()?)
    }
}

/// Layout chrome: what the nav bar and footer need on every page.
///
/// Assembled once per request from the signed-in user. Role flags are
/// pre-resolved here so templates never contain access logic -- and note that
/// hiding a nav link is a courtesy, not a permission check. The typed guard on
/// the handler is the actual control.
#[derive(Debug, Clone, Default)]
pub struct Nav {
    pub signed_in: bool,
    pub name: String,
    pub can_lead: bool,
    pub can_coach: bool,
    pub can_admin: bool,
    /// False when the database is unreachable, so the footer can say so rather
    /// than letting a scout type into a form that will not save.
    pub storage_ready: bool,
}

impl Nav {
    pub fn anonymous(storage_ready: bool) -> Self {
        Self {
            storage_ready,
            ..Self::default()
        }
    }

    pub fn for_user(user: Option<&User>, storage_ready: bool) -> Self {
        match user {
            Some(u) => Self {
                signed_in: true,
                name: u.name.clone(),
                can_lead: u.roles.can_lead(),
                can_coach: u.roles.can_coach(),
                can_admin: u.roles.can_admin(),
                storage_ready,
            },
            None => Self::anonymous(storage_ready),
        }
    }
}

/// Status page shown when there is nothing else to say yet -- including when the
/// database is down and the app is deliberately serving degraded pages.
#[derive(Template)]
#[template(path = "health.html")]
pub struct HealthPage {
    pub storage_ready: bool,
    pub schema_version: Option<i64>,
}

#[derive(Template)]
#[template(path = "pages/home.html")]
pub struct HomePage {
    pub title: String,
    pub nav: Nav,
    pub team_display: String,
    pub season_name: String,
    pub season_year: i32,
}

#[derive(Template)]
#[template(path = "pages/sign_in.html")]
pub struct SignInPage {
    pub title: String,
    pub nav: Nav,
    /// Preserved across a failed attempt so the user does not retype it.
    pub email: String,
    pub error: String,
}

#[derive(Template)]
#[template(path = "pages/sign_up.html")]
pub struct SignUpPage {
    pub title: String,
    pub nav: Nav,
    pub name: String,
    pub email: String,
    pub team_number: String,
    pub error: String,
    /// The first account on a fresh database becomes an admin; say so, so it is
    /// a deliberate act rather than a surprise.
    pub first_account: bool,
}

/// A reachable, access-controlled page whose content is still to come.
///
/// Exists so the nav never links a 404, and so the guard on a privileged route
/// is settled before there is anything on it worth protecting.
#[derive(Template)]
#[template(path = "pages/placeholder.html")]
pub struct PlaceholderPage {
    pub title: String,
    pub nav: Nav,
    pub heading: String,
    pub summary: String,
    pub season_name: String,
}

#[derive(Template)]
#[template(path = "pages/account.html")]
pub struct AccountPage {
    pub title: String,
    pub nav: Nav,
    pub user_name: String,
    pub user_email: String,
    pub team_display: String,
    pub role_labels: Vec<&'static str>,
    pub error: String,
    pub success: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tt_core::user::Roles;

    fn user(name: &str, roles: Roles) -> User {
        User {
            id: 1,
            email: "scout@example.com".into(),
            name: name.into(),
            team_number: Some(10101),
            roles,
        }
    }

    fn nav(roles: Roles) -> Nav {
        Nav::for_user(Some(&user("Sam", roles)), true)
    }

    #[test]
    fn renders_ready_state_with_schema_version() {
        let html = HealthPage {
            storage_ready: true,
            schema_version: Some(3),
        }
        .render_html()
        .expect("render");
        assert!(html.contains("ready"));
        assert!(html.contains("v3"));
    }

    #[test]
    fn renders_degraded_state_when_storage_is_down() {
        // The degraded path is the one that matters: this page has to render at
        // all when the database is unreachable.
        let html = HealthPage {
            storage_ready: false,
            schema_version: None,
        }
        .render_html()
        .expect("render");
        assert!(html.contains("unavailable"));
        assert!(html.contains("not yet migrated"));
    }

    #[test]
    fn nav_hides_privileged_links_from_a_plain_scout() {
        let html = HomePage {
            title: "Home".into(),
            nav: nav(Roles::SCOUT),
            team_display: "10101".into(),
            season_name: "Rebuilt".into(),
            season_year: 2026,
        }
        .render_html()
        .expect("render");

        assert!(html.contains("/submission"));
        assert!(!html.contains("/lead-scout"));
        assert!(!html.contains("/drive-coach"));
    }

    #[test]
    fn nav_shows_lead_and_coach_links_to_an_admin() {
        let html = HomePage {
            title: "Home".into(),
            nav: nav(Roles {
                is_admin: true,
                ..Roles::SCOUT
            }),
            team_display: "10101".into(),
            season_name: "Rebuilt".into(),
            season_year: 2026,
        }
        .render_html()
        .expect("render");

        assert!(html.contains("/lead-scout"));
        assert!(html.contains("/drive-coach"));
    }

    #[test]
    fn anonymous_visitors_see_sign_in_not_scouting() {
        let html = HomePage {
            title: "Home".into(),
            nav: Nav::anonymous(true),
            team_display: String::new(),
            season_name: "Rebuilt".into(),
            season_year: 2026,
        }
        .render_html()
        .expect("render");

        assert!(html.contains("/sign-in"));
        assert!(html.contains("/sign-up"));
        assert!(!html.contains("/submission"));
    }

    #[test]
    fn a_degraded_footer_warns_that_nothing_is_being_saved() {
        let html = HomePage {
            title: "Home".into(),
            nav: Nav::anonymous(false),
            team_display: String::new(),
            season_name: "Rebuilt".into(),
            season_year: 2026,
        }
        .render_html()
        .expect("render");
        assert!(html.contains("Storage unavailable"));
    }

    #[test]
    fn user_supplied_values_are_html_escaped() {
        // Askama escapes by default; this test is here so that a future switch
        // to a raw filter cannot silently open an injection.
        let html = SignInPage {
            title: "Sign in".into(),
            nav: Nav::anonymous(true),
            email: "\"><script>alert(1)</script>".into(),
            error: "<img src=x onerror=alert(1)>".into(),
        }
        .render_html()
        .expect("render");

        assert!(!html.contains("<script>"));
        assert!(!html.contains("<img src=x"));
        // Askama emits numeric entities rather than named ones.
        assert!(html.contains("&#60;script&#62;"));
    }

    #[test]
    fn sign_in_preserves_the_email_across_a_failed_attempt() {
        let html = SignInPage {
            title: "Sign in".into(),
            nav: Nav::anonymous(true),
            email: "scout@example.com".into(),
            error: "Invalid email or password".into(),
        }
        .render_html()
        .expect("render");

        assert!(html.contains("scout@example.com"));
        assert!(html.contains("Invalid email or password"));
    }

    #[test]
    fn sign_up_announces_that_the_first_account_is_an_admin() {
        let html = SignUpPage {
            title: "Sign up".into(),
            nav: Nav::anonymous(true),
            name: String::new(),
            email: String::new(),
            team_number: String::new(),
            error: String::new(),
            first_account: true,
        }
        .render_html()
        .expect("render");
        assert!(html.contains("administrator"));
    }

    #[test]
    fn account_page_lists_every_role_badge() {
        let html = AccountPage {
            title: "Account".into(),
            nav: nav(Roles {
                is_admin: true,
                is_lead_scout: true,
                is_coach: true,
            }),
            user_name: "Sam".into(),
            user_email: "scout@example.com".into(),
            team_display: "10101".into(),
            role_labels: vec!["Admin", "Lead Scout", "Drive Coach"],
            error: String::new(),
            success: "Password changed".into(),
        }
        .render_html()
        .expect("render");

        assert!(html.contains("Admin"));
        assert!(html.contains("Lead Scout"));
        assert!(html.contains("Drive Coach"));
        assert!(html.contains("Password changed"));
    }
}
