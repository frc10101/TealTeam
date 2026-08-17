//! Internet and API health tracking, for offline-aware behaviour at events.
//!
//! A port of `internal/frc/connectivity.go` and `Services/Connectivity.cs`.
//!
//! Competition venues routinely have no usable uplink, and the difference
//! between "no internet" and "the API rejected us" is what the UI needs to
//! tell a scout. This module keeps one process-wide snapshot of both, updated
//! by every outbound call, and exposes it to the status badge
//! ([`crate::views::network`]).
//!
//! Reachability is a TCP connect to [`PROBE_HOST`] rather than an HTTP request
//! — it is fast enough to run before each call, and the result is cached for
//! [`CACHE_TTL`] so a burst of calls probes once. A successful API call counts
//! as proof of reachability on its own.
//!
//! LAN and loopback base URLs skip the check entirely
//! ([`should_skip_connectivity_check`]), so a local mock or an on-site mirror
//! works with no internet at all.

use std::sync::Mutex;
use std::time::Duration;

use chrono::{DateTime, Utc};
use once_cell::sync::Lazy;
use serde::Serialize;

/// Reachability probe target: a well-known anycast resolver.
const PROBE_HOST: &str = "1.1.1.1";
const PROBE_PORT: u16 = 443;
const PROBE_TIMEOUT: Duration = Duration::from_millis(1500);
/// How long a probe result is reused before probing again.
const CACHE_TTL: Duration = Duration::from_secs(3);
/// Attempts per API call, including the first.
pub const API_RETRY_MAX_ATTEMPTS: u32 = 3;

/// The uplink itself is down, as opposed to the API refusing a request.
#[derive(Debug)]
pub struct InternetUnavailable(pub String);

impl std::fmt::Display for InternetUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for InternetUnavailable {}

/// True if an error chain bottoms out in [`InternetUnavailable`], which is
/// how controllers choose between "you are offline" and "the API failed".
pub fn is_internet_unavailable(err: &anyhow::Error) -> bool {
    err.chain().any(|e| e.is::<InternetUnavailable>())
}

/// Current connectivity state. Serialized as camelCase for the JSON status
/// endpoint.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkStatusSnapshot {
    pub checked_at: Option<DateTime<Utc>>,
    pub internet_reachable: bool,
    pub internet_error: String,
    pub last_api_success_at: Option<DateTime<Utc>>,
    pub last_api_error_at: Option<DateTime<Utc>>,
    pub last_api_error: String,
    pub last_successful_sync: Option<DateTime<Utc>>,
}

impl NetworkStatusSnapshot {
    /// Overall state shown in the UI: "internet-ok", "api-error" or "offline".
    /// A recent API success outranks everything — if calls are landing, the
    /// uplink works whatever the last reachability probe said.
    pub fn classify(&self) -> String {
        let recent_window = chrono::Duration::minutes(10);
        if let Some(success) = self.last_api_success_at {
            if Utc::now().signed_duration_since(success) <= recent_window {
                let newer_than_error = self
                    .last_api_error_at
                    .map(|err| success > err)
                    .unwrap_or(true);
                if newer_than_error {
                    return "internet-ok".to_string();
                }
            }
        }

        if !self.internet_reachable {
            return "offline".to_string();
        }
        if let Some(err) = self.last_api_error_at {
            let newer_than_success = self.last_api_success_at.map(|ok| err > ok).unwrap_or(true);
            if newer_than_success {
                return "api-error".to_string();
            }
        }
        "internet-ok".to_string()
    }
}

static STATE: Lazy<Mutex<NetworkStatusSnapshot>> =
    Lazy::new(|| Mutex::new(NetworkStatusSnapshot::default()));

/// A copy of the current state.
pub fn snapshot() -> NetworkStatusSnapshot {
    STATE.lock().unwrap().clone()
}

/// Re-probes reachability, ignoring the result — the snapshot is the point.
pub async fn refresh() {
    let _ = ensure_internet().await;
}

/// Confirms the uplink is usable, using the cached result when it is fresh.
pub async fn ensure_internet() -> Result<(), InternetUnavailable> {
    let cached = snapshot();
    if let Some(checked_at) = cached.checked_at {
        let age = Utc::now().signed_duration_since(checked_at);
        if age.to_std().map(|a| a <= CACHE_TTL).unwrap_or(false) {
            if cached.internet_reachable {
                return Ok(());
            }
            return Err(InternetUnavailable(cached.internet_error));
        }
    }

    probe(PROBE_HOST, PROBE_PORT).await
}

/// Probes the API host itself rather than the generic target, so a reachable
/// internet but unreachable API is caught before the request.
pub async fn ensure_internet_for_base_url(base_url: &str) -> Result<(), InternetUnavailable> {
    match reqwest::Url::parse(base_url) {
        Ok(url) if url.host_str().is_some() => {
            let host = url.host_str().unwrap().to_string();
            let port = url
                .port()
                .unwrap_or(if url.scheme() == "http" { 80 } else { 443 });
            probe(&host, port).await
        }
        _ => ensure_internet().await,
    }
}

/// True for loopback, RFC1918 and link-local hosts: a LAN target needs no
/// internet, so probing would only produce false failures.
pub fn should_skip_connectivity_check(base_url: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(base_url) else {
        return false;
    };
    let Some(host) = url.host_str() else {
        return false;
    };

    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }

    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        if ip.is_loopback() {
            return true;
        }
        if let std::net::IpAddr::V4(v4) = ip {
            let o = v4.octets();
            // RFC1918 private ranges + link-local
            if o[0] == 10 {
                return true;
            }
            if o[0] == 172 && (16..=31).contains(&o[1]) {
                return true;
            }
            if o[0] == 192 && o[1] == 168 {
                return true;
            }
            if o[0] == 169 && o[1] == 254 {
                return true;
            }
        }
    }

    false
}

/// TCP connect with a short timeout, recording the outcome.
async fn probe(host: &str, port: u16) -> Result<(), InternetUnavailable> {
    let addr = format!("{host}:{port}");
    let result = tokio::time::timeout(PROBE_TIMEOUT, tokio::net::TcpStream::connect(&addr)).await;

    match result {
        Ok(Ok(_)) => {
            record_connectivity_result(true, "");
            Ok(())
        }
        Ok(Err(e)) => {
            let reason = format!("failed to reach {addr} ({e})");
            record_connectivity_result(false, &reason);
            Err(InternetUnavailable(reason))
        }
        Err(_) => {
            let reason = format!("failed to reach {addr} (timeout)");
            record_connectivity_result(false, &reason);
            Err(InternetUnavailable(reason))
        }
    }
}

fn record_connectivity_result(ok: bool, err_text: &str) {
    let mut s = STATE.lock().unwrap();
    s.checked_at = Some(Utc::now());
    s.internet_reachable = ok;
    s.internet_error = err_text.to_string();
}

/// Records a successful API call, which also proves reachability.
pub fn record_api_success() {
    let mut s = STATE.lock().unwrap();
    let now = Utc::now();
    s.last_api_success_at = Some(now);
    // A successful API call proves the internet/API path is reachable.
    s.checked_at = Some(now);
    s.internet_reachable = true;
    s.internet_error.clear();
}

/// Records a failed API call and its message, shown in the badge tooltip.
pub fn record_api_error(message: &str) {
    let mut s = STATE.lock().unwrap();
    s.last_api_error_at = Some(Utc::now());
    s.last_api_error = message.to_string();
}

/// Records a completed sync, shown as "last sync" in the badge.
pub fn record_successful_sync() {
    STATE.lock().unwrap().last_successful_sync = Some(Utc::now());
}

/// Retry throttling and server errors; take a client error at its word.
pub fn should_retry_status_code(status: u16) -> bool {
    status == 429 || (500..=599).contains(&status)
}

/// Backoff between attempts: 250ms, 500ms, then 1s.
pub fn backoff_delay(attempt: u32) -> Duration {
    match attempt {
        0 => Duration::from_millis(250),
        1 => Duration::from_millis(500),
        _ => Duration::from_secs(1),
    }
}
