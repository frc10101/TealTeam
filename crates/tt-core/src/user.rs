//! People, their capabilities, and their sessions.
//!
//! Pure types and rules only. Password hashing lives in the server crate --
//! Argon2 is expensive by design and has no business in a wasm bundle.

use chrono::{DateTime, TimeDelta, Utc};
use serde::{Deserialize, Serialize};

use crate::error::{DomainError, Result};

/// How long a session lasts. Matches the retired implementation: an event day is
/// long, and being logged out mid-match is worse than the marginal risk on a
/// LAN nobody outside the venue can reach.
pub const SESSION_DURATION: TimeDelta = TimeDelta::hours(24);

/// Shortest password we accept.
pub const MIN_PASSWORD_LEN: usize = 8;

/// A person.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    pub id: i64,
    pub email: String,
    pub name: String,
    pub team_number: Option<i32>,
    pub roles: Roles,
}

/// Independent capabilities.
///
/// Not a hierarchy: `is_admin` grants lead-scout and coach access by OR at the
/// call site, never by inheritance in the data. Keeping them independent is what
/// lets a mentor be a coach without also being able to approve submissions.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Roles {
    pub is_admin: bool,
    pub is_lead_scout: bool,
    pub is_coach: bool,
}

impl Roles {
    pub const SCOUT: Self = Self {
        is_admin: false,
        is_lead_scout: false,
        is_coach: false,
    };

    /// May approve submissions, assign robots, and tune weights.
    pub fn can_lead(self) -> bool {
        self.is_admin || self.is_lead_scout
    }

    /// May see the coach panel.
    pub fn can_coach(self) -> bool {
        self.is_admin || self.is_coach
    }

    /// May see the database viewer and anything else unrestricted.
    ///
    /// The retired implementation left its database viewer with **no check at
    /// all**, exposing every user's email and all session rows to anyone who
    /// knew the URL (REBUILD_SPEC.md 12.4).
    pub fn can_admin(self) -> bool {
        self.is_admin
    }

    /// Badges for the account page, most privileged first. Never empty.
    pub fn labels(self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.is_admin {
            out.push("Admin");
        }
        if self.is_lead_scout {
            out.push("Lead Scout");
        }
        if self.is_coach {
            out.push("Drive Coach");
        }
        if out.is_empty() {
            out.push("Scout");
        }
        out
    }
}

/// A signed-in browser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub id: String,
    pub user_id: i64,
    pub expires_at: DateTime<Utc>,
}

impl Session {
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        now >= self.expires_at
    }
}

// ── Validation ──────────────────────────────────────────────────────────────

/// Normalise an email for storage and comparison.
///
/// Lowercased and trimmed; the schema's unique index is on `lower(email)` to
/// match, so `Alice@` and `alice@` cannot both register.
pub fn normalize_email(raw: &str) -> String {
    raw.trim().to_lowercase()
}

/// Check an email is plausible.
///
/// Deliberately loose. Strict RFC 5322 validation rejects addresses that work
/// and accepts ones that do not; the only real test is sending mail, which this
/// app never does. One `@`, something either side, a dot in the domain.
pub fn validate_email(raw: &str) -> Result<String> {
    let email = normalize_email(raw);
    if email.is_empty() {
        return Err(DomainError::Missing { field: "email" });
    }

    let mut parts = email.split('@');
    let (local, domain) = match (parts.next(), parts.next(), parts.next()) {
        (Some(l), Some(d), None) if !l.is_empty() && !d.is_empty() => (l, d),
        _ => {
            return Err(DomainError::Invalid {
                field: "email",
                value: email,
            });
        }
    };

    if local.is_empty() || !domain.contains('.') || domain.starts_with('.') || domain.ends_with('.')
    {
        return Err(DomainError::Invalid {
            field: "email",
            value: email,
        });
    }
    Ok(email)
}

/// Check a proposed password.
///
/// Length only. Composition rules ("one symbol, one digit") measurably push
/// people toward `Password1!` and, on a team of students sharing tablets, toward
/// writing it on the tablet.
pub fn validate_password(password: &str) -> Result<()> {
    if password.is_empty() {
        return Err(DomainError::Missing { field: "password" });
    }
    if password.chars().count() < MIN_PASSWORD_LEN {
        return Err(DomainError::Invalid {
            field: "password",
            value: format!("must be at least {MIN_PASSWORD_LEN} characters"),
        });
    }
    Ok(())
}

/// Check an FRC team number.
pub fn validate_team_number(raw: &str) -> Result<Option<i32>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    match trimmed.parse::<i32>() {
        Ok(n) if n > 0 => Ok(Some(n)),
        _ => Err(DomainError::Invalid {
            field: "team number",
            value: trimmed.to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 3, 14, hour, 0, 0).unwrap()
    }

    // ── Roles ───────────────────────────────────────────────────────────────

    #[test]
    fn admin_can_do_everything_without_the_other_flags_being_set() {
        let admin = Roles {
            is_admin: true,
            ..Roles::SCOUT
        };
        assert!(admin.can_lead());
        assert!(admin.can_coach());
        assert!(admin.can_admin());
    }

    #[test]
    fn a_coach_cannot_approve_submissions() {
        let coach = Roles {
            is_coach: true,
            ..Roles::SCOUT
        };
        assert!(coach.can_coach());
        assert!(!coach.can_lead());
        assert!(!coach.can_admin());
    }

    #[test]
    fn a_lead_scout_is_not_an_admin() {
        // The database viewer is admin-only; a lead scout must not reach it.
        let lead = Roles {
            is_lead_scout: true,
            ..Roles::SCOUT
        };
        assert!(lead.can_lead());
        assert!(!lead.can_admin());
    }

    #[test]
    fn a_plain_scout_can_do_none_of_it() {
        assert!(!Roles::SCOUT.can_lead());
        assert!(!Roles::SCOUT.can_coach());
        assert!(!Roles::SCOUT.can_admin());
    }

    #[test]
    fn role_labels_are_never_empty() {
        assert_eq!(Roles::SCOUT.labels(), vec!["Scout"]);
        assert_eq!(
            Roles {
                is_admin: true,
                is_lead_scout: true,
                is_coach: true
            }
            .labels(),
            vec!["Admin", "Lead Scout", "Drive Coach"]
        );
    }

    // ── Sessions ────────────────────────────────────────────────────────────

    #[test]
    fn a_session_expires_exactly_at_its_deadline() {
        let s = Session {
            id: "x".into(),
            user_id: 1,
            expires_at: at(12),
        };
        assert!(!s.is_expired(at(11)));
        assert!(s.is_expired(at(12)), "the deadline itself is expired");
        assert!(s.is_expired(at(13)));
    }

    // ── Email ───────────────────────────────────────────────────────────────

    #[test]
    fn emails_are_normalised_for_storage() {
        assert_eq!(
            validate_email("  Scout@Example.COM "),
            Ok("scout@example.com".to_string())
        );
    }

    #[test]
    fn implausible_emails_are_rejected() {
        for bad in [
            "",
            "  ",
            "no-at-sign",
            "@nolocal.com",
            "nodomain@",
            "two@@ats.com",
            "no@dot",
            "trailing@dot.",
            "@",
            "a@.com",
        ] {
            assert!(validate_email(bad).is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn ordinary_addresses_are_accepted() {
        for good in [
            "a@b.co",
            "first.last@school.k12.ma.us",
            "scout+2026@example.org",
        ] {
            assert!(validate_email(good).is_ok(), "{good:?} should be accepted");
        }
    }

    // ── Password ────────────────────────────────────────────────────────────

    #[test]
    fn passwords_have_a_length_floor_and_no_composition_rules() {
        assert!(validate_password("").is_err());
        assert!(validate_password("short").is_err());
        assert!(validate_password("12345678").is_ok());
        assert!(validate_password("all lowercase words no symbols").is_ok());
    }

    #[test]
    fn password_length_counts_characters_not_bytes() {
        // Eight emoji is eight characters, and a perfectly good passphrase.
        assert!(validate_password(&"🤖".repeat(8)).is_ok());
        assert!(validate_password(&"🤖".repeat(7)).is_err());
    }

    // ── Team number ─────────────────────────────────────────────────────────

    #[test]
    fn team_number_is_optional_but_must_be_sane_when_given() {
        assert_eq!(validate_team_number(""), Ok(None));
        assert_eq!(validate_team_number("   "), Ok(None));
        assert_eq!(validate_team_number("10101"), Ok(Some(10101)));
        assert!(validate_team_number("0").is_err());
        assert!(validate_team_number("-5").is_err());
        assert!(validate_team_number("ten").is_err());
    }
}
