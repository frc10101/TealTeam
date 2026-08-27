//! Whether the server can currently reach the internet (I10/I11).
//!
//! The classification is here, in the pure crate, because it is a state machine
//! over timestamps and has nothing to do with sockets. `tt-upstream` does the
//! probing and feeds this.
//!
//! # What this describes, and what it does not
//!
//! This is the **server's** view of its own uplink. It answers "can the Pi reach
//! TBA right now", which is what decides whether a sync will work and how stale
//! the rankings on screen might be.
//!
//! It is *not* the client's connection to the server. The retired app conflated
//! the two and showed scouts a badge about the Pi's internet while their own
//! tablet was the thing that had dropped off the LAN — which is where the
//! "what does offline mode even do" confusion came from (REBUILD_SPEC.md 6.4).
//! The client-side chip is a separate thing (I11) and belongs in the browser.

use chrono::{DateTime, TimeDelta, Utc};

/// How long an API success keeps counting as proof of connectivity.
///
/// Ten minutes: long enough to survive the gap between syncs during quals, short
/// enough that a badge does not claim "online" through a whole match break.
pub const SUCCESS_WINDOW: TimeDelta = TimeDelta::minutes(10);

/// What the server currently believes about its uplink.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NetworkSnapshot {
    pub checked_at: Option<DateTime<Utc>>,
    pub reachable: bool,
    pub probe_error: String,

    pub last_api_success: Option<DateTime<Utc>>,
    pub last_api_error: Option<DateTime<Utc>>,
    pub api_error_message: String,

    /// Last time a full sync completed, as opposed to a single call succeeding.
    pub last_sync: Option<DateTime<Utc>>,
}

/// The three states the server's uplink can be in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UplinkState {
    /// Upstream is answering.
    Online,
    /// Reachable, but the API is refusing or erroring — a bad key, or TBA down.
    /// Distinct from `Offline` because the fix is completely different.
    ApiError,
    /// No route to the internet at all. The normal state at an event.
    Offline,
}

impl UplinkState {
    pub fn label(self) -> &'static str {
        match self {
            UplinkState::Online => "Upstream online",
            UplinkState::ApiError => "Upstream error",
            UplinkState::Offline => "No internet",
        }
    }

    /// CSS modifier, so templates carry no state logic.
    pub fn css_class(self) -> &'static str {
        match self {
            UplinkState::Online => "badge-teal",
            UplinkState::ApiError => "badge-amber",
            UplinkState::Offline => "badge-red",
        }
    }
}

impl NetworkSnapshot {
    /// Classify the current state.
    ///
    /// Order matters:
    ///
    ///   1. A recent API success outranks everything. It is direct evidence, and
    ///      it beats a stale probe.
    ///   2. Otherwise an unreachable probe means offline.
    ///   3. Otherwise an API error newer than the last success means the network
    ///      is fine and the API is not.
    pub fn classify(&self, now: DateTime<Utc>) -> UplinkState {
        if let Some(success) = self.last_api_success
            && now.signed_duration_since(success) <= SUCCESS_WINDOW
            && self.last_api_error.is_none_or(|err| success > err)
        {
            return UplinkState::Online;
        }

        if !self.reachable {
            return UplinkState::Offline;
        }

        match (self.last_api_error, self.last_api_success) {
            (Some(err), Some(ok)) if err > ok => UplinkState::ApiError,
            (Some(_), None) => UplinkState::ApiError,
            _ => UplinkState::Online,
        }
    }

    /// How stale the last successful sync is.
    ///
    /// Feeds the freshness badges (I12): rankings that look live but are forty
    /// minutes old cause bad alliance picks.
    pub fn sync_age(&self, now: DateTime<Utc>) -> Option<TimeDelta> {
        self.last_sync.map(|t| now.signed_duration_since(t))
    }

    pub fn record_probe(&mut self, now: DateTime<Utc>, reachable: bool, error: &str) {
        self.checked_at = Some(now);
        self.reachable = reachable;
        self.probe_error = error.to_string();
    }

    /// A successful call proves the uplink works, so it back-fills the probe.
    pub fn record_api_success(&mut self, now: DateTime<Utc>) {
        self.last_api_success = Some(now);
        self.checked_at = Some(now);
        self.reachable = true;
        self.probe_error.clear();
    }

    pub fn record_api_error(&mut self, now: DateTime<Utc>, message: &str) {
        self.last_api_error = Some(now);
        self.api_error_message = message.to_string();
    }

    pub fn record_sync(&mut self, now: DateTime<Utc>) {
        self.last_sync = Some(now);
    }
}

/// How stale upstream data may be before it is shown as suspect.
///
/// Twenty minutes is roughly two quals cycles.
pub const STALE_AFTER: TimeDelta = TimeDelta::minutes(20);

/// Whether a timestamp is old enough that a viewer should be warned.
pub fn is_stale(synced_at: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
    match synced_at {
        Some(t) => now.signed_duration_since(t) > STALE_AFTER,
        // Never synced is the most stale thing there is.
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 3, 14, 12, minute, 0).unwrap()
    }

    #[test]
    fn a_fresh_snapshot_with_no_history_reads_offline() {
        assert_eq!(
            NetworkSnapshot::default().classify(at(0)),
            UplinkState::Offline
        );
    }

    #[test]
    fn a_recent_api_success_means_online() {
        let mut s = NetworkSnapshot::default();
        s.record_api_success(at(0));
        assert_eq!(s.classify(at(5)), UplinkState::Online);
    }

    #[test]
    fn a_success_older_than_the_window_stops_counting() {
        let mut s = NetworkSnapshot::default();
        s.record_api_success(at(0));
        s.reachable = false; // probe has since failed
        assert_eq!(s.classify(at(11)), UplinkState::Offline);
    }

    #[test]
    fn an_api_error_after_a_success_is_an_api_error_not_offline() {
        // The distinction matters: offline means plug in a cable, api error
        // means check the key or wait for TBA.
        let mut s = NetworkSnapshot::default();
        s.record_api_success(at(0));
        s.record_api_error(at(1), "401 unauthorized");
        assert_eq!(s.classify(at(2)), UplinkState::ApiError);
    }

    #[test]
    fn a_success_after_an_error_clears_back_to_online() {
        let mut s = NetworkSnapshot::default();
        s.record_api_error(at(0), "500");
        s.record_api_success(at(1));
        assert_eq!(s.classify(at(2)), UplinkState::Online);
    }

    #[test]
    fn an_unreachable_probe_beats_a_stale_success() {
        let mut s = NetworkSnapshot::default();
        s.record_api_success(at(0));
        s.record_probe(at(20), false, "no route to host");
        assert_eq!(s.classify(at(21)), UplinkState::Offline);
    }

    #[test]
    fn errors_with_no_success_at_all_are_api_errors_when_reachable() {
        let mut s = NetworkSnapshot::default();
        s.record_probe(at(0), true, "");
        s.record_api_error(at(0), "403 forbidden");
        assert_eq!(s.classify(at(1)), UplinkState::ApiError);
    }

    #[test]
    fn a_successful_call_backfills_the_probe_state() {
        let mut s = NetworkSnapshot::default();
        s.record_probe(at(0), false, "timeout");
        s.record_api_success(at(1));
        assert!(s.reachable);
        assert!(s.probe_error.is_empty());
    }

    #[test]
    fn sync_age_is_none_until_a_sync_happens() {
        let mut s = NetworkSnapshot::default();
        assert_eq!(s.sync_age(at(5)), None);
        s.record_sync(at(0));
        assert_eq!(s.sync_age(at(5)), Some(TimeDelta::minutes(5)));
    }

    #[test]
    fn never_synced_counts_as_stale() {
        assert!(is_stale(None, at(0)));
    }

    #[test]
    fn staleness_turns_over_at_twenty_minutes() {
        assert!(!is_stale(Some(at(0)), at(20)));
        assert!(is_stale(Some(at(0)), at(21)));
    }

    #[test]
    fn every_state_has_a_label_and_a_class() {
        for state in [
            UplinkState::Online,
            UplinkState::ApiError,
            UplinkState::Offline,
        ] {
            assert!(!state.label().is_empty());
            assert!(state.css_class().starts_with("badge-"));
        }
    }
}
