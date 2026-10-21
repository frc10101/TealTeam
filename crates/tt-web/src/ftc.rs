//! The optional FTC Events pull, wired into the server (FTC_BETA_PLAN.md, F7).
//!
//! Off unless `FTC_API_USERNAME` and `FTC_API_KEY` are set. FIRST, TBA, the
//! tether watch (S6), and the tablet courier (S7) stay off as F4 left them;
//! this is its own switch. The one trigger is the lead scout's "Pull from FTC
//! Events" button on `/lead-scout/schedule` (`POST /api/ftc/pull`), which
//! shows what the pull did or why it could not. There is no background poll:
//! a schedule pasted by hand is never replaced unless someone asks.
//!
//! The same pull also answers the lead-scout page's "Sync now" on this build
//! (F10), and works out OPR, DPR, and CCWM from the played matches.
//!
//! What a pull stores is `tt_upstream::ftc::pull`; this adds the schedule
//! tidy the CSV import does (`schedule::tidy`), so a re-pull and a re-import
//! behave the same. The CSV paste stays, and stays the fallback.

use std::time::Duration;

use tracing::{info, warn};
use tt_upstream::UpstreamError;
use tt_upstream::ftc;

use crate::schedule::{self, plural};
use crate::startup::AppState;

/// What the button says with no key set. No request is made.
pub const NO_KEY: &str = "No FTC Events API key is set on the server, so nothing was \
    requested. Set FTC_API_USERNAME and FTC_API_KEY and restart, or paste the schedule below.";

/// Longest one pull may take.
pub const PULL_TIMEOUT: Duration = Duration::from_secs(90);

/// What a pull did, for the schedule page.
#[derive(Debug, Clone, Default)]
pub struct Outcome {
    /// The event pulled, when the pull got that far.
    pub event_key: Option<String>,
    pub report: Vec<String>,
    /// `"Q6 blue1: 19459"`.
    pub surrogates: Vec<String>,
    /// Why nothing was pulled.
    pub errors: Vec<String>,
}

/// Say an error the way a lead scout can act on it.
fn describe(error: &UpstreamError) -> String {
    match error {
        UpstreamError::Offline => "Offline: the Pi has no internet. It reaches FTC Events only \
            through the phone tether, plugged in by hand. Nothing changed; the CSV paste below \
            still works."
            .into(),
        UpstreamError::Status {
            status: status @ (401 | 403),
            ..
        } => format!(
            "FTC Events refused the key ({status}). Check FTC_API_USERNAME and FTC_API_KEY on \
             the server."
        ),
        UpstreamError::NotConfigured(_) => NO_KEY.into(),
        UpstreamError::Busy { retry_after, .. } => format!(
            "FTC Events is busy and asked us to wait{}. Nothing changed; pull again then.",
            retry_after
                .map(|s| format!(" {}", plural(s.div_ceil(60).max(1), "minute", "minutes")))
                .unwrap_or_default()
        ),
        other => format!("FTC Events did not answer usably: {other}"),
    }
}

/// Pull now, if a key is set and no other pull is running.
pub async fn pull(state: &AppState) -> Outcome {
    let upstream = &state.upstream;
    let Some(client) = &upstream.ftc else {
        return Outcome {
            errors: vec![NO_KEY.into()],
            ..Outcome::default()
        };
    };
    let Ok(_one_at_a_time) = upstream.ftc_pull.try_lock() else {
        return Outcome {
            errors: vec!["A pull is already running; try again when it finishes.".into()],
            ..Outcome::default()
        };
    };

    let pulled = match tokio::time::timeout(PULL_TIMEOUT, ftc::pull(&*state.repo, client)).await {
        Ok(Ok(pulled)) => pulled,
        Ok(Err(e)) => {
            warn!("FTC pull: {e}");
            return Outcome {
                errors: vec![describe(&e)],
                ..Outcome::default()
            };
        }
        Err(_) => {
            return Outcome {
                errors: vec![format!(
                    "Stopped after {PULL_TIMEOUT:?}; what it stored is kept. Try again."
                )],
                ..Outcome::default()
            };
        }
    };

    let mut report = Vec::new();
    if pulled.changed {
        upstream.uplink.record_sync();
        report.push(format!(
            "Pulled {}, {} ({} played), and {} for {} from FTC Events.",
            plural(pulled.teams as u64, "team", "teams"),
            plural(pulled.matches as u64, "match", "matches"),
            pulled.played,
            plural(pulled.rankings as u64, "ranking", "rankings"),
            if pulled.event_name.is_empty() {
                &pulled.event_key
            } else {
                &pulled.event_name
            },
        ));
    } else if !pulled.event_name.is_empty() {
        report.push(format!(
            "Nothing has changed at FTC Events since the last pull: {}, {}.",
            plural(pulled.teams as u64, "team", "teams"),
            plural(pulled.matches as u64, "match", "matches"),
        ));
    }
    if let Some((keys, roster)) = &pulled.tidy {
        match schedule::tidy(state, &pulled.event_key, keys, roster).await {
            Ok(lines) => report.extend(lines),
            Err(e) => {
                warn!("tidying {} after an FTC pull: {e}", pulled.event_key);
                report.push(
                    "The schedule could not be tidied: the server's storage did not \
                    answer. Pull again."
                        .into(),
                );
            }
        }
    }
    if pulled.score_sheets > 0 && pulled.changed {
        report.push(format!(
            "Kept {} in the log for after the event.",
            plural(pulled.score_sheets as u64, "score sheet", "score sheets")
        ));
    }
    match pulled.rated {
        Some(ftc::Rated { teams, played }) if teams > 0 => report.push(format!(
            "Worked out OPR, DPR, and CCWM for {} from {}.",
            plural(teams as u64, "team", "teams"),
            plural(played as u64, "played match", "played matches"),
        )),
        Some(ftc::Rated { played, .. }) if played > 0 => report.push(format!(
            "OPR, DPR, and CCWM stay empty until enough matches are played: {played} so far."
        )),
        _ => {}
    }
    report.extend(pulled.problems);
    info!(event = %pulled.event_key, "FTC pull: {}", report.join(" "));
    Outcome {
        event_key: (!pulled.event_name.is_empty()).then_some(pulled.event_key),
        report,
        surrogates: pulled.surrogates.iter().map(|s| s.describe()).collect(),
        errors: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_say_what_to_do() {
        assert!(describe(&UpstreamError::Offline).starts_with("Offline"));
        let refused = UpstreamError::Status {
            api: "ftc",
            path: "/2026/events".into(),
            status: 401,
            body: String::new(),
        };
        assert!(describe(&refused).contains("(401)"));
        let busy = UpstreamError::Busy {
            api: "ftc",
            path: "/2026/rankings/USALATQ".into(),
            status: 503,
            retry_after: Some(90),
        };
        assert!(
            describe(&busy).contains("wait 2 minutes"),
            "{}",
            describe(&busy)
        );
    }
}
