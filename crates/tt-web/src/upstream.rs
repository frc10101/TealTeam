//! Upstream sync, wired into the server (I4, I7, I8, I13).
//!
//! `tt-upstream` has the clients and the sync; this decides when they run:
//!
//!   - **At boot**, the FIRST event sync, bounded at 60 seconds. In the
//!     background: the server never waits on the internet to start serving.
//!   - **Then indefinitely**, the TBA loop (I7): every two minutes while an
//!     event is live, every three hours otherwise.
//!   - **When a lead scout asks**, `POST /api/frc/sync` (I13): the FIRST sync
//!     again, then a nudge that wakes the loop.
//!   - **On demand**, `tt-web bulk-load` (I8): the full snapshot, run at the
//!     shop before anyone leaves.
//!
//! Missing credentials switch the matching piece off. That is a supported
//! configuration, not an error -- scouting works with no upstream at all.
//!
//! **No page render calls upstream** (U15). Pages read storage and say how old
//! it is; the only request that waits on the network is the sync button, which
//! is asked to. The retired server fetched FIRST mid-render for a team with no
//! local events and for an empty roster (REBUILD_SPEC.md 12.7).

use std::io::Write;
use std::sync::Arc;

use anyhow::{Context, bail};
use chrono::{DateTime, Utc};
use serde::Serialize;
use tokio::sync::{Mutex, Notify};
use tokio::task::JoinHandle;
use tracing::{info, warn};
use tt_core::connectivity;
use tt_repo::Repo;
use tt_repo_sqlite::SqliteRepo;
use tt_templates::UpstreamPanel;
use tt_upstream::Uplink;
use tt_upstream::first::{EventFilters, FirstClient};
use tt_upstream::sync::{self, SyncReport};
use tt_upstream::tba::TbaClient;

/// The configured upstream clients and the state they share. One per process,
/// held in `AppState` so the manual sync and the background loop are talking
/// about the same uplink.
pub struct Upstream {
    pub first: Option<FirstClient>,
    pub tba: Option<TbaClient>,
    pub filters: EventFilters,
    pub uplink: Uplink,
    /// Cuts the background loop's pause short.
    wake: Arc<Notify>,
    /// Held for the length of any FIRST event sync, at boot or on request. A
    /// full sync is a hundred-odd requests; two lead scouts pressing the button
    /// at once should not make it two hundred.
    first_sync: Mutex<()>,
}

impl Upstream {
    pub fn new(
        first: Option<FirstClient>,
        tba: Option<TbaClient>,
        filters: EventFilters,
        uplink: Uplink,
    ) -> Self {
        Self {
            first,
            tba,
            filters,
            uplink,
            wake: Arc::new(Notify::new()),
            first_sync: Mutex::new(()),
        }
    }

    /// Read `FIRST_*` and `TBA_AUTH_KEY`. Call after the `.env` files load.
    pub fn from_env() -> Self {
        let uplink = Uplink::new();
        Self::new(
            FirstClient::from_env(uplink.clone()),
            TbaClient::from_env(uplink.clone()),
            EventFilters::from_env(),
            uplink,
        )
    }

    /// No upstream at all.
    #[cfg(test)]
    pub fn disabled() -> Self {
        Self::new(None, None, EventFilters::all(), Uplink::new())
    }
}

/// Start the boot sync and the background loop, and return at once.
///
/// `None` when there is nothing to run: no credentials, or only FIRST
/// credentials with the boot sync switched off.
pub fn spawn(
    repo: Arc<SqliteRepo>,
    upstream: Arc<Upstream>,
    sync_on_boot: bool,
) -> Option<JoinHandle<()>> {
    let boot = match &upstream.first {
        None => {
            info!("FIRST_API_USERNAME / FIRST_API_KEY not set; no event sync at boot");
            false
        }
        Some(_) if !sync_on_boot => {
            info!("FIRST_SYNC_ON_BOOT is off; no event sync at boot");
            false
        }
        Some(_) => true,
    };
    if upstream.tba.is_none() {
        info!("TBA_AUTH_KEY not set; background match and stats sync is off");
    }
    if !boot && upstream.tba.is_none() {
        return None;
    }

    Some(tokio::spawn(async move {
        // The event list first, so the loop's first pass sees today's events.
        if boot {
            boot_sync(&repo, &upstream).await;
        }
        if let Some(tba) = upstream.tba.clone() {
            let (uplink, wake) = (upstream.uplink.clone(), upstream.wake.clone());
            sync::run_loop(repo, tba, uplink, wake).await;
        }
    }))
}

async fn boot_sync(repo: &SqliteRepo, upstream: &Upstream) {
    let Some(first) = &upstream.first else {
        return;
    };
    let _one_at_a_time = upstream.first_sync.lock().await;

    let run = sync::sync_events(repo, first, upstream.tba.as_ref(), &upstream.filters);
    match tokio::time::timeout(sync::BOOT_SYNC_TIMEOUT, run).await {
        Ok(Ok(report)) => {
            if !report.is_empty() {
                upstream.uplink.record_sync();
            }
            info!("event sync at boot: {}", report.summary());
        }
        Ok(Err(e)) if e.is_offline() => {
            info!("no internet at boot; serving the events already stored");
        }
        Ok(Err(e)) => warn!("event sync at boot failed: {e}"),
        Err(_) => warn!(
            "event sync at boot stopped after {:?}; keeping what it stored",
            sync::BOOT_SYNC_TIMEOUT
        ),
    }
}

// ── Manual sync (I13) ───────────────────────────────────────────────────────

/// What a manual sync did. Serialised as the JSON answer to
/// `POST /api/frc/sync`, and rendered on the lead-scout page.
#[derive(Debug, Serialize)]
pub struct ManualSync {
    /// Nothing went wrong.
    pub ok: bool,
    pub events: usize,
    pub teams: usize,
    pub event_teams: usize,
    pub problems: Vec<String>,
    /// The background loop was woken, so matches and statistics follow.
    pub tba_pass_started: bool,
}

impl ManualSync {
    /// One sentence for a person, above the list of problems.
    pub fn headline(&self) -> String {
        let landed = self.events > 0 || self.teams > 0;
        let mut text = match (landed, self.ok) {
            (true, true) => format!(
                "Synced {} event(s) and {} team(s).",
                self.events, self.teams
            ),
            (true, false) => format!(
                "Synced {} event(s) and {} team(s), but not everything worked:",
                self.events, self.teams
            ),
            (false, true) => "FIRST returned no events for the configured filters.".into(),
            (false, false) => "The event sync did not complete:".into(),
        };
        if self.tba_pass_started {
            text.push_str(" Matches and statistics are updating in the background.");
        }
        text
    }
}

/// `POST /api/frc/sync` (I13): pull the FIRST event list now, then wake the
/// background loop so matches and statistics follow at once.
///
/// This is the in-app remedy for a Pi that booted without internet. The boot
/// sync does not retry, and the loop may be hours into a pause it chose when
/// the event list was empty.
pub async fn sync_now(repo: &SqliteRepo, upstream: &Upstream) -> ManualSync {
    let mut report = SyncReport::default();

    match (&upstream.first, upstream.first_sync.try_lock()) {
        (None, _) => report
            .problems
            .push("FIRST_API_USERNAME and FIRST_API_KEY are not set on the server".into()),
        (Some(_), Err(_)) => report
            .problems
            .push("A sync is already running; try again when it finishes".into()),
        (Some(first), Ok(_one_at_a_time)) => {
            let run = sync::sync_events(repo, first, upstream.tba.as_ref(), &upstream.filters);
            match tokio::time::timeout(sync::MANUAL_SYNC_TIMEOUT, run).await {
                Ok(Ok(r)) => report.merge(r),
                Ok(Err(e)) => report.problems.push(e.to_string()),
                Err(_) => report.problems.push(format!(
                    "Stopped after {:?}; what it stored is kept",
                    sync::MANUAL_SYNC_TIMEOUT
                )),
            }
        }
    }

    if !report.is_empty() {
        upstream.uplink.record_sync();
    }
    // Wake the loop even when the event sync failed: it syncs the events
    // already stored, which is still the freshest data available.
    let tba_pass_started = upstream.tba.is_some();
    if tba_pass_started {
        upstream.wake.notify_one();
    }

    info!("manual sync: {}", report.summary());
    ManualSync {
        ok: report.problems.is_empty(),
        events: report.events,
        teams: report.teams,
        event_teams: report.event_teams,
        problems: report.problems,
        tba_pass_started,
    }
}

/// The lead-scout page's upstream card, with the outcome of a sync just
/// requested from it, if there was one.
pub fn panel(
    upstream: &Upstream,
    outcome: Option<&ManualSync>,
    now: DateTime<Utc>,
) -> UpstreamPanel {
    let snapshot = upstream.uplink.snapshot();

    // An uplink nobody has tested yet is unknown, not offline. Without this a
    // server with no credentials would claim to have no internet.
    let (uplink_label, uplink_class) = match snapshot.checked_at {
        None => ("Not checked yet", "badge-gray"),
        Some(_) => {
            let state = snapshot.classify(now);
            (state.label(), state.css_class())
        }
    };

    UpstreamPanel {
        uplink_label,
        uplink_class,
        last_sync: snapshot
            .sync_age(now)
            .map(connectivity::describe_age)
            .unwrap_or_else(|| "never".into()),
        // "never" says it already; a stale badge beside it adds nothing.
        stale: snapshot.last_sync.is_some() && connectivity::is_stale(snapshot.last_sync, now),
        first_configured: upstream.first.is_some(),
        tba_configured: upstream.tba.is_some(),
        result_headline: outcome.map(ManualSync::headline).unwrap_or_default(),
        result_ok: outcome.is_some_and(|o| o.ok),
        result_problems: outcome.map(|o| o.problems.clone()).unwrap_or_default(),
    }
}

/// `tt-web bulk-load` (I8): pull the full snapshot and print what the database
/// now holds.
///
/// The per-event counts are read back from storage rather than taken from the
/// sync's own tally, so what gets checked at the shop is what the Pi will serve.
pub async fn bulk_load(
    repo: &SqliteRepo,
    upstream: &Upstream,
    out: &mut impl Write,
) -> anyhow::Result<()> {
    let Some(first) = &upstream.first else {
        bail!("FIRST_API_USERNAME and FIRST_API_KEY must be set to bulk-load");
    };

    let report = sync::bulk_load(
        repo,
        first,
        upstream.tba.as_ref(),
        &upstream.filters,
        &upstream.uplink,
    )
    .await
    .context("bulk load failed")?;

    writeln!(out, "Loaded {}", report.summary())?;
    for problem in &report.problems {
        writeln!(out, "  ! {problem}")?;
    }

    writeln!(out)?;
    writeln!(
        out,
        "{:<16} {:>6} {:>8} {:>6}  NAME",
        "EVENT", "TEAMS", "MATCHES", "STATS"
    )?;
    for event in repo.list_events().await? {
        let teams = repo.event_teams(&event.key).await?.len();
        let matches = repo.event_matches(&event.key).await?.len();
        let stats = repo.event_stats(&event.key).await?.len();
        writeln!(
            out,
            "{:<16} {teams:>6} {matches:>8} {stats:>6}  {}",
            event.key, event.name
        )?;
    }

    if report.is_empty() {
        bail!("nothing was loaded; see the problems above");
    }
    Ok(())
}

/// Stubs shared by this module's tests and the router tests in `startup`.
#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use axum::Router;
    use axum::routing::get;
    use std::sync::atomic::{AtomicUsize, Ordering};

    pub const EVENTS: &str = r#"{"Events":[{
      "code":"MABIL","name":"Greater Boston Regional","city":"Boston",
      "stateprov":"MA","country":"USA",
      "dateStart":"2026-03-12T00:00:00","dateEnd":"2026-03-15T00:00:00"
    }]}"#;

    const TEAMS: &str = r#"{"teams":[
      {"teamNumber":10101,"nameShort":"Teal Team","country":"USA"},
      {"teamNumber":254,"nameShort":"The Cheesy Poofs","country":"USA"}
    ]}"#;

    /// A FIRST stub on loopback, which the client never probes the internet
    /// for. Returns its base URL.
    pub async fn first_stub(events: &'static str) -> String {
        let app = Router::new()
            .route("/2026/events", get(move || async move { events }))
            .route("/2026/teams", get(|| async { TEAMS }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        format!("http://{addr}")
    }

    /// A stub that counts every request and never answers one: an upstream
    /// that is configured, reachable, and hopeless. Returns its base URL and
    /// the count.
    pub async fn hanging_stub() -> (String, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let app = Router::new().fallback(move || {
            counted.fetch_add(1, Ordering::SeqCst);
            std::future::pending::<&'static str>()
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        (format!("http://{addr}"), calls)
    }

    /// FIRST and TBA both pointed at `base`.
    pub fn upstream_everywhere(base: &str) -> Upstream {
        let uplink = Uplink::new();
        let first = FirstClient::new("user", "token", 2026, uplink.clone())
            .expect("client")
            .with_base_url(base);
        let tba = TbaClient::new("key", uplink.clone())
            .expect("client")
            .with_base_url(base);
        Upstream::new(Some(first), Some(tba), EventFilters::all(), uplink)
    }

    /// FIRST pointed at `first_base` when given; TBA configured when asked,
    /// pointed somewhere that is never actually called.
    pub fn upstream_at(first_base: Option<&str>, with_tba: bool) -> Upstream {
        let uplink = Uplink::new();
        let first = first_base.map(|base| {
            FirstClient::new("user", "token", 2026, uplink.clone())
                .expect("client")
                .with_base_url(base)
        });
        let tba = with_tba.then(|| {
            TbaClient::new("key", uplink.clone())
                .expect("client")
                .with_base_url("http://127.0.0.1:9")
        });
        Upstream::new(first, tba, EventFilters::all(), uplink)
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{EVENTS, first_stub, upstream_at};
    use super::*;
    use std::time::Duration;

    async fn repo() -> Arc<SqliteRepo> {
        let repo = SqliteRepo::connect("sqlite::memory:").expect("connect");
        tt_repo_sqlite::migrate::apply(repo.pool())
            .await
            .expect("migrate");
        Arc::new(repo)
    }

    fn upstream(first_base: Option<&str>) -> Arc<Upstream> {
        Arc::new(upstream_at(first_base, false))
    }

    // ── Boot ────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn nothing_starts_without_credentials() {
        assert!(spawn(repo().await, upstream(None), true).is_none());
    }

    #[tokio::test]
    async fn the_boot_sync_can_be_switched_off() {
        let base = first_stub(EVENTS).await;
        assert!(spawn(repo().await, upstream(Some(&base)), false).is_none());
    }

    #[tokio::test]
    async fn the_boot_sync_lands_events_without_holding_up_startup() {
        let base = first_stub(EVENTS).await;
        let repo = repo().await;

        let task = spawn(repo.clone(), upstream(Some(&base)), true).expect("a task");
        // With no TBA key the task ends after the boot sync, so it can be
        // awaited here. With one, it would carry on into the loop.
        task.await.expect("boot sync task");

        let events = repo.list_events().await.expect("list");
        assert_eq!(events.len(), 1);
        assert_eq!(
            repo.event_teams("2026mabil").await.expect("roster").len(),
            2
        );
    }

    // ── Manual sync (I13) ───────────────────────────────────────────────────

    #[tokio::test]
    async fn a_manual_sync_lands_events_and_says_so() {
        let base = first_stub(EVENTS).await;
        let repo = repo().await;
        let upstream = upstream(Some(&base));

        let outcome = sync_now(&repo, &upstream).await;

        assert!(outcome.ok, "{:?}", outcome.problems);
        assert_eq!((outcome.events, outcome.teams), (1, 2));
        assert!(!outcome.tba_pass_started, "no TBA key, so no pass follows");
        assert_eq!(outcome.headline(), "Synced 1 event(s) and 2 team(s).");
        assert!(upstream.uplink.snapshot().last_sync.is_some());
    }

    #[tokio::test]
    async fn a_manual_sync_without_first_credentials_says_which_are_missing() {
        let outcome = sync_now(&*repo().await, &upstream(None)).await;
        assert!(!outcome.ok);
        assert!(outcome.problems[0].contains("FIRST_API_USERNAME"));
        assert_eq!(outcome.headline(), "The event sync did not complete:");
    }

    #[tokio::test]
    async fn a_second_sync_while_one_runs_is_turned_away() {
        let base = first_stub(EVENTS).await;
        let repo = repo().await;
        let upstream = upstream(Some(&base));

        let _running = upstream.first_sync.lock().await;
        let outcome = sync_now(&repo, &upstream).await;

        assert!(outcome.problems[0].contains("already running"));
        assert!(
            repo.list_events().await.unwrap().is_empty(),
            "no second sync"
        );
    }

    #[tokio::test]
    async fn a_manual_sync_wakes_the_background_loop() {
        let base = first_stub(EVENTS).await;
        let upstream = upstream_at(Some(&base), true);

        let outcome = sync_now(&*repo().await, &upstream).await;

        assert!(outcome.tba_pass_started);
        assert!(outcome.headline().ends_with("updating in the background."));
        // notify_one stores a permit when nobody is waiting, so the loop picks
        // the nudge up even if it was mid-pass when the request landed.
        tokio::time::timeout(Duration::from_millis(100), upstream.wake.notified())
            .await
            .expect("the loop was not woken");
    }

    #[test]
    fn headlines_distinguish_partial_success_from_nothing_at_all() {
        let outcome = |events, ok| ManualSync {
            ok,
            events,
            teams: events * 30,
            event_teams: 0,
            problems: Vec::new(),
            tba_pass_started: false,
        };
        assert_eq!(
            outcome(2, false).headline(),
            "Synced 2 event(s) and 60 team(s), but not everything worked:"
        );
        assert_eq!(
            outcome(0, true).headline(),
            "FIRST returned no events for the configured filters."
        );
    }

    // ── The lead-scout card ─────────────────────────────────────────────────

    #[test]
    fn an_untested_uplink_is_unknown_not_offline() {
        let panel = panel(&Upstream::disabled(), None, Utc::now());
        assert_eq!(panel.uplink_label, "Not checked yet");
        assert_eq!(panel.last_sync, "never");
        assert!(!panel.stale, "never synced already says it");
        assert!(!panel.first_configured && !panel.tba_configured);
        assert!(panel.result_headline.is_empty());
    }

    #[test]
    fn a_sync_ages_from_fresh_to_stale() {
        let upstream = Upstream::disabled();
        upstream.uplink.record_success();
        upstream.uplink.record_sync();

        let soon = panel(&upstream, None, Utc::now() + chrono::TimeDelta::minutes(5));
        assert_eq!(soon.last_sync, "5 minutes ago");
        assert!(!soon.stale);
        assert_eq!(soon.uplink_label, "Upstream online");

        let later = panel(&upstream, None, Utc::now() + chrono::TimeDelta::minutes(30));
        assert!(later.stale, "past twenty minutes, rankings are suspect");
    }

    // ── Bulk load (I8) ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn a_bulk_load_prints_what_the_database_holds() {
        let base = first_stub(EVENTS).await;
        let repo = repo().await;
        let mut out = Vec::new();

        bulk_load(&repo, &upstream(Some(&base)), &mut out)
            .await
            .expect("bulk load");

        let text = String::from_utf8(out).expect("utf8");
        assert!(text.contains("1 event(s), 2 team(s)"), "{text}");
        let row = text
            .lines()
            .find(|l| l.starts_with("2026mabil"))
            .unwrap_or_else(|| panic!("no row for the event in:\n{text}"));
        assert!(row.contains("Greater Boston Regional"), "{row}");
        assert_eq!(
            row.split_whitespace().take(4).collect::<Vec<_>>(),
            ["2026mabil", "2", "0", "0"],
            "teams, matches, stats"
        );
        // No TBA key: the operator is told why there are no matches.
        assert!(text.contains("! TBA key not configured"), "{text}");
    }

    #[tokio::test]
    async fn a_bulk_load_without_first_credentials_says_which_are_missing() {
        let repo = repo().await;
        let error = bulk_load(&repo, &upstream(None), &mut Vec::new())
            .await
            .expect_err("no credentials");
        assert!(error.to_string().contains("FIRST_API_USERNAME"), "{error}");
    }

    #[tokio::test]
    async fn a_bulk_load_that_lands_nothing_fails() {
        let base = first_stub(r#"{"Events":[]}"#).await;
        let repo = repo().await;
        let error = bulk_load(&repo, &upstream(Some(&base)), &mut Vec::new())
            .await
            .expect_err("an empty load is not a success");
        assert!(error.to_string().contains("nothing was loaded"), "{error}");
    }
}
