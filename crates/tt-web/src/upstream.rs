//! Upstream sync, wired into the server (I4, I7, I8).
//!
//! `tt-upstream` has the clients and the sync; this decides when they run:
//!
//!   - **At boot**, the FIRST event sync, bounded at 60 seconds. In the
//!     background: the server never waits on the internet to start serving.
//!   - **Then indefinitely**, the TBA loop (I7): every two minutes while an
//!     event is live, every three hours otherwise.
//!   - **On demand**, `tt-web bulk-load` (I8): the full snapshot, run at the
//!     shop before anyone leaves.
//!
//! Missing credentials switch the matching piece off. That is a supported
//! configuration, not an error -- scouting works with no upstream at all.

use std::io::Write;
use std::sync::Arc;

use anyhow::{Context, bail};
use tokio::task::JoinHandle;
use tracing::{info, warn};
use tt_repo::Repo;
use tt_repo_sqlite::SqliteRepo;
use tt_upstream::Uplink;
use tt_upstream::first::{EventFilters, FirstClient};
use tt_upstream::sync;
use tt_upstream::tba::TbaClient;

/// The configured upstream clients, sharing one uplink tracker.
pub struct Upstream {
    pub first: Option<FirstClient>,
    pub tba: Option<TbaClient>,
    pub filters: EventFilters,
    pub uplink: Uplink,
}

impl Upstream {
    /// Read `FIRST_*` and `TBA_AUTH_KEY`. Call after the `.env` files load.
    pub fn from_env() -> Self {
        let uplink = Uplink::new();
        Self {
            first: FirstClient::from_env(uplink.clone()),
            tba: TbaClient::from_env(uplink.clone()),
            filters: EventFilters::from_env(),
            uplink,
        }
    }
}

/// Start the boot sync and the background loop, and return at once.
///
/// `None` when there is nothing to run: no credentials, or only FIRST
/// credentials with the boot sync switched off.
pub fn spawn(
    repo: Arc<SqliteRepo>,
    upstream: Upstream,
    sync_on_boot: bool,
) -> Option<JoinHandle<()>> {
    let Upstream {
        first,
        tba,
        filters,
        uplink,
    } = upstream;

    let first = match first {
        None => {
            info!("FIRST_API_USERNAME / FIRST_API_KEY not set; no event sync at boot");
            None
        }
        Some(_) if !sync_on_boot => {
            info!("FIRST_SYNC_ON_BOOT is off; no event sync at boot");
            None
        }
        configured => configured,
    };
    if tba.is_none() {
        info!("TBA_AUTH_KEY not set; background match and stats sync is off");
    }
    if first.is_none() && tba.is_none() {
        return None;
    }

    Some(tokio::spawn(async move {
        // The event list first, so the loop's first pass sees today's events.
        if let Some(first) = first {
            boot_sync(&repo, &first, &filters, &uplink).await;
        }
        if let Some(tba) = tba {
            sync::run_loop(repo, tba, uplink).await;
        }
    }))
}

async fn boot_sync(
    repo: &SqliteRepo,
    first: &FirstClient,
    filters: &EventFilters,
    uplink: &Uplink,
) {
    let run = sync::sync_events(repo, first, filters);
    match tokio::time::timeout(sync::BOOT_SYNC_TIMEOUT, run).await {
        Ok(Ok(report)) => {
            if !report.is_empty() {
                uplink.record_sync();
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::routing::get;

    const EVENTS: &str = r#"{"Events":[{
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
    async fn stub(events: &'static str) -> String {
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

    async fn repo() -> Arc<SqliteRepo> {
        let repo = SqliteRepo::connect("sqlite::memory:").expect("connect");
        tt_repo_sqlite::migrate::apply(repo.pool())
            .await
            .expect("migrate");
        Arc::new(repo)
    }

    fn upstream(first_base: Option<&str>) -> Upstream {
        let uplink = Uplink::new();
        Upstream {
            first: first_base.map(|base| {
                FirstClient::new("user", "token", 2026, uplink.clone())
                    .expect("client")
                    .with_base_url(base)
            }),
            tba: None,
            filters: EventFilters::all(),
            uplink,
        }
    }

    #[tokio::test]
    async fn nothing_starts_without_credentials() {
        assert!(spawn(repo().await, upstream(None), true).is_none());
    }

    #[tokio::test]
    async fn the_boot_sync_can_be_switched_off() {
        let base = stub(EVENTS).await;
        assert!(spawn(repo().await, upstream(Some(&base)), false).is_none());
    }

    #[tokio::test]
    async fn the_boot_sync_lands_events_without_holding_up_startup() {
        let base = stub(EVENTS).await;
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

    #[tokio::test]
    async fn a_bulk_load_prints_what_the_database_holds() {
        let base = stub(EVENTS).await;
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
        let base = stub(r#"{"Events":[]}"#).await;
        let repo = repo().await;
        let error = bulk_load(&repo, &upstream(Some(&base)), &mut Vec::new())
            .await
            .expect_err("an empty load is not a success");
        assert!(error.to_string().contains("nothing was loaded"), "{error}");
    }
}
