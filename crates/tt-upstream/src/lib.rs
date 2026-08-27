//! FIRST and Blue Alliance HTTP clients (I1, I2), the uplink probe (I10), and
//! the sync that lands their data in a [`Repo`](tt_repo::Repo).
//!
//! Parsing lives in `tt_core::upstream`; this crate is transport and
//! orchestration only. That split exists so the deserializers can be tested
//! against recorded payloads without a network, and so they can later compile to
//! wasm32 for a client that has signal to fetch upstream itself (S4).

pub mod first;
pub mod probe;
pub mod sync;
pub mod tba;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use tt_core::connectivity::NetworkSnapshot;

/// Attempts per request, including the first.
///
/// Three. At an event the useful question is "is the uplink up", and the answer
/// arrives quickly either way; retrying longer just delays telling the operator
/// something is wrong.
pub const MAX_ATTEMPTS: u32 = 3;

/// How long any single upstream request may take.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

/// Delay before the next attempt. 250ms, 500ms, then 1s.
pub fn backoff(attempt: u32) -> Duration {
    match attempt {
        0 => Duration::from_millis(250),
        1 => Duration::from_millis(500),
        _ => Duration::from_secs(1),
    }
}

/// Whether a status code is worth trying again.
///
/// Rate limits and server faults only. A 401 or 404 will say the same thing
/// three times.
pub fn is_retryable(status: u16) -> bool {
    status == 429 || (500..=599).contains(&status)
}

#[derive(Debug, thiserror::Error)]
pub enum UpstreamError {
    /// No route to the host. Expected at an event; not an error worth alarming
    /// anyone about.
    #[error("no internet connection")]
    Offline,

    #[error("{api} {path} returned {status}: {body}")]
    Status {
        api: &'static str,
        path: String,
        status: u16,
        body: String,
    },

    #[error("{api} {path} returned a payload we could not read: {source}")]
    Payload {
        api: &'static str,
        path: String,
        source: serde_json::Error,
    },

    #[error("{api} {path} failed: {source}")]
    Transport {
        api: &'static str,
        path: String,
        source: reqwest::Error,
    },

    #[error("credentials for {0} are not configured")]
    NotConfigured(&'static str),
}

impl UpstreamError {
    /// Whether this is simply "we are at an event with no internet", which
    /// callers log quietly rather than surfacing as a failure.
    pub fn is_offline(&self) -> bool {
        matches!(self, UpstreamError::Offline)
    }
}

pub type Result<T> = std::result::Result<T, UpstreamError>;

/// Shared, thread-safe view of the uplink.
///
/// Every client updates it on success and failure, so the badge reflects real
/// traffic rather than a separate poller's opinion.
#[derive(Clone, Default)]
pub struct Uplink {
    snapshot: Arc<Mutex<NetworkSnapshot>>,
}

impl Uplink {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn snapshot(&self) -> NetworkSnapshot {
        // A poisoned lock here would mean a panic while holding it; the snapshot
        // is advisory, so recovering beats propagating.
        self.snapshot
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    fn update(&self, f: impl FnOnce(&mut NetworkSnapshot)) {
        if let Ok(mut guard) = self.snapshot.lock() {
            f(&mut guard);
        }
    }

    pub fn record_success(&self) {
        self.update(|s| s.record_api_success(Utc::now()));
    }

    pub fn record_error(&self, message: &str) {
        self.update(|s| s.record_api_error(Utc::now(), message));
    }

    pub fn record_probe(&self, reachable: bool, error: &str) {
        self.update(|s| s.record_probe(Utc::now(), reachable, error));
    }

    pub fn record_sync(&self) {
        self.update(|s| s.record_sync(Utc::now()));
    }
}

/// Truncate an error body before it reaches a log.
///
/// Upstream error pages can be entire HTML documents; a Pi's journal does not
/// need them.
pub(crate) fn truncate(body: &str) -> String {
    const LIMIT: usize = 512;
    if body.len() <= LIMIT {
        return body.to_string();
    }
    let cut = body
        .char_indices()
        .take_while(|(i, _)| *i <= LIMIT)
        .last()
        .map(|(i, _)| i)
        .unwrap_or(0);
    format!("{}…", &body[..cut])
}

#[cfg(test)]
mod tests {
    use super::*;
    use tt_core::connectivity::UplinkState;

    #[test]
    fn only_rate_limits_and_server_faults_are_retried() {
        assert!(is_retryable(429));
        assert!(is_retryable(500));
        assert!(is_retryable(503));
        // Retrying these just says the same thing three times.
        assert!(!is_retryable(400));
        assert!(!is_retryable(401));
        assert!(!is_retryable(404));
        assert!(!is_retryable(200));
    }

    #[test]
    fn backoff_grows_and_then_settles() {
        assert_eq!(backoff(0), Duration::from_millis(250));
        assert_eq!(backoff(1), Duration::from_millis(500));
        assert_eq!(backoff(2), Duration::from_secs(1));
        assert_eq!(backoff(99), Duration::from_secs(1));
    }

    #[test]
    fn long_error_bodies_are_truncated() {
        let long = "x".repeat(5000);
        let short = truncate(&long);
        assert!(short.len() < 600);
        assert!(short.ends_with('…'));
    }

    #[test]
    fn short_bodies_pass_through_unchanged() {
        assert_eq!(truncate("not found"), "not found");
    }

    #[test]
    fn truncation_does_not_split_a_multibyte_character() {
        let emoji = "🤖".repeat(1000);
        let cut = truncate(&emoji);
        // Would panic on a byte-slice boundary error if this were wrong.
        assert!(cut.chars().count() > 1);
    }

    #[test]
    fn an_uplink_starts_offline_and_follows_traffic() {
        let uplink = Uplink::new();
        let now = Utc::now();
        assert_eq!(uplink.snapshot().classify(now), UplinkState::Offline);

        uplink.record_success();
        assert_eq!(uplink.snapshot().classify(Utc::now()), UplinkState::Online);

        uplink.record_error("500 boom");
        assert_eq!(
            uplink.snapshot().classify(Utc::now()),
            UplinkState::ApiError
        );
    }

    #[test]
    fn an_uplink_is_shared_between_clones() {
        // Both API clients hold a clone; a success on either must be visible.
        let a = Uplink::new();
        let b = a.clone();
        b.record_success();
        assert!(a.snapshot().last_api_success.is_some());
    }

    #[test]
    fn offline_errors_are_distinguishable() {
        assert!(UpstreamError::Offline.is_offline());
        assert!(!UpstreamError::NotConfigured("TBA").is_offline());
    }
}
