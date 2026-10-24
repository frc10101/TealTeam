//! `tt-ops --once`: the overview as plain text, for a script or a shell
//! with no terminal. The same reads as the screen, printed once.

use std::process::ExitCode;

use chrono::Utc;
use tt_repo::DEVICE_ONLINE_WINDOW;

use crate::data::{self, ago, human_bytes};
use crate::server::Server;
use crate::site::Site;
use crate::system::{self, Verdict};

pub async fn once(site: Site) -> anyhow::Result<ExitCode> {
    let repo = tt_repo_sqlite::SqliteRepo::connect(&site.database_url)?;
    let server = Server::new(site.base_url());
    let (snapshot, status) = tokio::join!(
        data::load(&repo, &site),
        system::status(&site, server.client())
    );
    drop(repo);
    site.fix_wal_ownership();
    let now = Utc::now();

    println!(
        "TealTeam server on {} ({} install, {})",
        status.hostname,
        site.kind(),
        site.dir.display()
    );
    let p = &status.process;
    let h = &status.health;
    println!(
        "  {:<5} {}{}",
        if p.running && h.reachable {
            "ok"
        } else if p.running {
            "WAIT"
        } else {
            "FAIL"
        },
        p.state,
        match (p.pid, p.uptime) {
            (Some(pid), Some(up)) => format!(" (pid {pid}, up {} s)", up.as_secs()),
            (Some(pid), None) => format!(" (pid {pid})"),
            _ => String::new(),
        }
    );
    if h.reachable {
        println!(
            "  {:<5} /health: storage {}, build {}, schema {}, form {}",
            if h.storage == "ready" { "ok" } else { "FAIL" },
            h.storage,
            h.build,
            h.schema,
            h.form
        );
    } else {
        println!("  FAIL  /health on port {} is not answering", site.port);
    }
    if let Some(tls) = status.tls_listening {
        println!(
            "  {:<5} https on port {}",
            if tls { "ok" } else { "FAIL" },
            site.tls_port.unwrap_or(0)
        );
    }
    for check in &status.network {
        let mark = match check.verdict {
            Verdict::Ok => "ok",
            Verdict::Fail => "FAIL",
            Verdict::Note => "--",
        };
        println!("  {mark:<5} {}", check.text);
    }

    println!();
    println!(
        "  {:<5} database {}{}",
        if snapshot.storage_ready { "ok" } else { "FAIL" },
        snapshot.db_location,
        snapshot
            .db_bytes
            .map(|b| format!(" ({})", human_bytes(b)))
            .unwrap_or_default()
    );
    if let Some((free, total)) = status.disk_free {
        println!(
            "  --    disk: {} free of {}",
            human_bytes(free),
            human_bytes(total)
        );
    }
    let newest = snapshot.backups.first().and_then(|b| b.taken_at);
    let backups_ok = newest.is_some_and(|at| now - at < chrono::TimeDelta::minutes(15));
    println!(
        "  {:<5} backups: {} kept, newest {}",
        if backups_ok { "ok" } else { "WARN" },
        snapshot.backups.len(),
        ago(newest, now)
    );
    println!(
        "  --    online: {} scouts, {} devices in the last {} min; {} accounts",
        snapshot.online_scouts(now),
        snapshot.online_devices(now),
        DEVICE_ONLINE_WINDOW.num_minutes(),
        snapshot.accounts.len()
    );
    for event in &snapshot.events {
        println!(
            "  --    {}{}: {} matches, {} teams, {} forms ({} pending), {} assignments",
            event.key,
            if snapshot.default_event.as_deref() == Some(event.key.as_str()) {
                " (default)"
            } else {
                ""
            },
            event.matches,
            event.teams,
            event.observations,
            event.pending,
            event.assignments
        );
    }
    for problem in &snapshot.problems {
        println!("  FAIL  {problem}");
    }
    let ok = p.running && h.reachable && snapshot.storage_ready;
    Ok(if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
