//! `tt-load`: the load test to run before the season (Q3, REBUILD_SPEC.md 11).
//!
//! Thirty scouts' phones, each saving an observation, pulling, and holding a
//! live stream, against a real tt-web for two hours, while the cable is
//! pulled and the power killed on a schedule. It reports p95 latency per
//! request, every stream drop and why, and checks the two things that matter
//! more than speed: every save a scout was told was saved is on the server
//! once, and every one reached every other scout's stream.
//!
//! Filling a phone's storage is a browser's fault, not a server's: that is
//! `crates/tt-web/tests/browser/storage-full.mjs`.
//!
//! How to run it, on this machine or against the Pi: docs/LOAD_TEST.md.

mod faults;
mod report;
mod scout;
mod seed;
mod sse;

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use tokio::sync::{Notify, watch};

use crate::faults::{Cable, Server};
use crate::report::{Bars, Fault, Shared, Stored};
use crate::scout::Plan;

const USAGE: &str = "\
usage: tt-load <command>

commands:
  run --server <path to tt-web> [options]
              start tt-web on a fresh database, seed an event, run the scouts
              against it, and pull the cable and the power on a schedule.
              The whole test on one machine.
  run --url <http://server:port> [options]
              run the scouts against a server already running, seeded with
              `tt-load seed`. Pull the cable and the power by hand; the
              report shows when requests failed.
  seed <database url> [matches]
              store the load-test event (2026load) with 600 matches, or as
              many as asked. Safe to run again, and with the server running.
  help        show this message

options:
  --clients N        scouts (30)
  --duration D       how long scouts keep saving: 90s, 20m, 2h (2h)
  --cycle D          one scout's time between saves (15s)
  --p95 D            the bar for steady p95 latency (500ms)
with --server:
  --faults-every D   a fault this often, the cable and the power in turn
                     (15m; 0 for none)
  --cable D          how long the cable stays out (45s)
  --power D          how long the power stays off (20s)
  --dir PATH         where the database, server.log, and report go
                     (a new folder under the system's temp folder)";

#[derive(Debug, PartialEq)]
enum Target {
    Url(String),
    Server(PathBuf),
}

#[derive(Debug, PartialEq)]
struct Options {
    target: Target,
    clients: usize,
    duration: Duration,
    cycle: Duration,
    p95: Duration,
    faults_every: Duration,
    cable: Duration,
    power: Duration,
    dir: Option<PathBuf>,
}

#[derive(Debug, PartialEq)]
enum Command {
    Run(Box<Options>),
    Seed(String, u32),
    Help,
}

/// `90s`, `20m`, `2h`, `500ms`, or bare seconds.
fn duration(raw: &str) -> Result<Duration, String> {
    let split = raw.find(|c: char| !c.is_ascii_digit()).unwrap_or(raw.len());
    let (number, unit) = raw.split_at(split);
    let n: u64 = number
        .parse()
        .map_err(|_| format!("{raw:?} is not a duration"))?;
    Ok(match unit {
        "ms" => Duration::from_millis(n),
        "" | "s" => Duration::from_secs(n),
        "m" => Duration::from_secs(n * 60),
        "h" => Duration::from_secs(n * 3600),
        _ => return Err(format!("{raw:?}: the unit is ms, s, m, or h")),
    })
}

impl Command {
    /// Hand-parsed, like tt-web's: a few flags do not justify a dependency.
    fn parse(mut args: impl Iterator<Item = String>) -> Result<Self, String> {
        match args.next().as_deref() {
            None | Some("help" | "-h" | "--help") => Ok(Self::Help),
            Some("seed") => {
                let url = args.next().ok_or("seed needs a database url")?;
                let matches = match args.next() {
                    Some(n) => n.parse().map_err(|_| format!("{n:?} is not a number"))?,
                    None => 600,
                };
                Ok(Self::Seed(url, matches))
            }
            Some("run") => {
                let mut target = None;
                let mut o = Options {
                    target: Target::Url(String::new()),
                    clients: 30,
                    duration: Duration::from_secs(2 * 3600),
                    cycle: Duration::from_secs(15),
                    p95: Duration::from_millis(500),
                    faults_every: Duration::from_secs(15 * 60),
                    cable: Duration::from_secs(45),
                    power: Duration::from_secs(20),
                    dir: None,
                };
                while let Some(flag) = args.next() {
                    let value = args.next().ok_or(format!("{flag} needs a value"))?;
                    match flag.as_str() {
                        "--url" => target = Some(Target::Url(value.trim_end_matches('/').into())),
                        "--server" => target = Some(Target::Server(value.into())),
                        "--clients" => {
                            o.clients = value
                                .parse()
                                .ok()
                                .filter(|n| *n > 0)
                                .ok_or(format!("{value:?} is not a number of scouts"))?
                        }
                        "--duration" => o.duration = duration(&value)?,
                        "--cycle" => o.cycle = duration(&value)?,
                        "--p95" => o.p95 = duration(&value)?,
                        "--faults-every" => o.faults_every = duration(&value)?,
                        "--cable" => o.cable = duration(&value)?,
                        "--power" => o.power = duration(&value)?,
                        "--dir" => o.dir = Some(value.into()),
                        _ => return Err(format!("unknown option {flag:?}")),
                    }
                }
                o.target = target.ok_or("run needs --server or --url")?;
                if o.cycle.is_zero() {
                    return Err("--cycle must be more than 0".into());
                }
                Ok(Self::Run(Box::new(o)))
            }
            Some(other) => Err(format!("unknown command {other:?}")),
        }
    }
}

/// When each fault starts, since the run began: every `every`, while the
/// fault and its settling end before the scouts stop.
fn schedule(o: &Options) -> Vec<(Duration, &'static str)> {
    if o.faults_every.is_zero() {
        return Vec::new();
    }
    (1u32..)
        .map(|k| {
            let what = if k % 2 == 1 { "cable" } else { "power" };
            (o.faults_every * k, what)
        })
        .take_while(|(at, what)| {
            let lasts = if *what == "cable" { o.cable } else { o.power };
            *at + lasts + report::SETTLE < o.duration
        })
        .collect()
}

/// Enough matches that no scout runs out, at the quickest pace jitter allows.
fn matches_for(o: &Options) -> u32 {
    (o.duration.as_secs_f64() / (o.cycle.as_secs_f64() * 0.8)).ceil() as u32 + 10
}

async fn run(o: Options) -> anyhow::Result<bool> {
    // ── The server, and the cable to it ──
    let mut server = None;
    let mut cable = None;
    let mut dir = None;
    let base = match &o.target {
        Target::Url(url) => url.clone(),
        Target::Server(binary) => {
            let folder = o.dir.clone().unwrap_or_else(|| {
                std::env::temp_dir().join(format!(
                    "tt-load-{}",
                    chrono::Utc::now().format("%Y%m%d-%H%M%S")
                ))
            });
            std::fs::create_dir_all(&folder)?;
            let binary = std::fs::canonicalize(binary)
                .with_context(|| format!("no tt-web at {}", binary.display()))?;
            seed::seed(&Server::database_url(&folder), matches_for(&o)).await?;
            let mut s = Server::new(&binary, &folder)?;
            s.start().await?;
            let c = Cable::start(s.addr()).await?;
            let base = format!("http://{}", c.addr);
            println!("server: {} on {}", binary.display(), folder.display());
            server = Some(s);
            cable = Some(Arc::new(c));
            dir = Some(folder);
            base
        }
    };
    println!(
        "{} scouts, a save each every {:?}, for {:?}, against {base}",
        o.clients, o.cycle, o.duration
    );

    // ── Everyone signed in, and where the streams start ──
    let mut cookies = Vec::new();
    for i in 0..o.clients {
        cookies.push(scout::sign_in(&base, i).await?);
    }
    let (cursors, _) = scout::pull(&Shared::new(), &scout::client(), &base, &cookies[0], (0, 0))
        .await
        .context("the first pull failed")?;

    let shared = Arc::new(Shared::new());
    let plan = Arc::new(Plan {
        base: base.clone(),
        clients: o.clients,
        cycle: o.cycle,
        until: o.duration,
        matches: matches_for(&o),
        cursors,
    });
    let (stop, stopped) = watch::channel(false);

    // ── The faults ──
    let faults = match (server, cable) {
        (Some(mut server), Some(cable)) => {
            let shared = shared.clone();
            let plan = schedule(&o);
            let (cable_for, power_for) = (o.cable, o.power);
            Some(tokio::spawn(async move {
                for (at, what) in plan {
                    tokio::time::sleep(at.saturating_sub(shared.elapsed())).await;
                    let from = shared.elapsed();
                    println!("{}  {what} out", clock(from));
                    cable.set_plugged(false);
                    if what == "cable" {
                        tokio::time::sleep(cable_for).await;
                    } else {
                        server.kill().await;
                        tokio::time::sleep(power_for).await;
                        if let Err(e) = server.start().await {
                            eprintln!("the server did not come back: {e:#}");
                        }
                    }
                    cable.set_plugged(true);
                    let to = shared.elapsed();
                    println!("{}  {what} back", clock(to));
                    let what = if what == "cable" {
                        "cable pulled"
                    } else {
                        "power killed"
                    };
                    shared.faults.lock().unwrap().push(Fault { what, from, to });
                }
                server
            }))
        }
        _ => None,
    };

    // ── Progress, every five minutes ──
    let progress = {
        let shared = shared.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(300)).await;
                let samples = shared.samples.lock().unwrap();
                println!(
                    "{}  {} saves confirmed, {} requests, {} failed",
                    clock(shared.elapsed()),
                    shared.acked.lock().unwrap().len(),
                    samples.len(),
                    samples.iter().filter(|s| !s.ok).count(),
                );
            }
        })
    };

    // ── The scouts ──
    let mut scouts = Vec::new();
    let mut streams = Vec::new();
    for (index, cookie) in cookies.into_iter().enumerate() {
        let navigated = Arc::new(Notify::new());
        let (shared_s, base_s, cookie_s, navigated_s, stopped) = (
            shared.clone(),
            base.clone(),
            cookie.clone(),
            navigated.clone(),
            stopped.clone(),
        );
        let from = format!("{}-{}", cursors.0, cursors.1);
        streams.push(tokio::spawn(async move {
            sse::follow(&shared_s, &base_s, &cookie_s, from, &navigated_s, stopped).await
        }));
        let (shared, plan) = (shared.clone(), plan.clone());
        scouts.push(tokio::spawn(async move {
            scout::work(&shared, &plan, index, &cookie, &navigated).await
        }));
    }
    for scout in scouts {
        scout.await?;
    }
    let server = match faults {
        Some(f) => Some(f.await?),
        None => None,
    };

    // Long enough for the last save to clear the stream's two-second lag.
    tokio::time::sleep(Duration::from_secs(15)).await;
    stop.send_replace(true);
    progress.abort();
    let mut stats = Vec::new();
    for stream in streams {
        stats.push(stream.await?);
    }

    // ── What the server holds ──
    let verifier = scout::sign_in(&base, 0).await?;
    let (_, changes) = scout::pull(&Shared::new(), &scout::client(), &base, &verifier, cursors)
        .await
        .context("the closing pull failed")?;
    let mut stored = Stored::new();
    for change in &changes {
        if change["entity"] == "observation"
            && change["op"] == "upsert"
            && let Some(id) = change["entity_pk"].as_str()
        {
            *stored.entry(id.to_string()).or_default() += 1;
        }
    }
    drop(server);

    let bars = Bars {
        p95: o.p95,
        injected: dir.is_some(),
    };
    let (text, passed) = report::summarise(&shared, &stats, &stored, &bars);
    println!("\n{text}");
    if let Some(dir) = dir {
        std::fs::write(dir.join("report.txt"), &text)?;
        println!("report and server.log in {}", dir.display());
    }
    Ok(passed)
}

fn clock(d: Duration) -> String {
    let s = d.as_secs();
    format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

#[tokio::main]
async fn main() -> ExitCode {
    let result = match Command::parse(std::env::args().skip(1)) {
        Ok(Command::Help) => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Ok(Command::Seed(url, matches)) => seed::seed(&url, matches).await.map(|()| {
            println!("seeded {} with {matches} matches", seed::EVENT);
            true
        }),
        Ok(Command::Run(options)) => run(*options).await,
        Err(e) => {
            eprintln!("{e}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("fatal: {e:#}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Command, String> {
        Command::parse(args.iter().map(|a| a.to_string()))
    }

    fn options(args: &[&str]) -> Options {
        match parse(args).unwrap() {
            Command::Run(o) => *o,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn durations_take_a_unit() {
        assert_eq!(duration("90s"), Ok(Duration::from_secs(90)));
        assert_eq!(duration("20m"), Ok(Duration::from_secs(1200)));
        assert_eq!(duration("2h"), Ok(Duration::from_secs(7200)));
        assert_eq!(duration("500ms"), Ok(Duration::from_millis(500)));
        assert_eq!(duration("45"), Ok(Duration::from_secs(45)));
        assert!(duration("2d").is_err());
        assert!(duration("m").is_err());
    }

    #[test]
    fn the_defaults_are_the_spec() {
        let o = options(&["run", "--server", "target/release/tt-web"]);
        assert_eq!(o.clients, 30);
        assert_eq!(o.duration, Duration::from_secs(7200));
        assert_eq!(o.target, Target::Server("target/release/tt-web".into()));
    }

    #[test]
    fn a_run_needs_somewhere_to_run() {
        assert!(parse(&["run"]).unwrap_err().contains("--server or --url"));
        assert!(
            parse(&["run", "--url"])
                .unwrap_err()
                .contains("needs a value")
        );
        assert!(parse(&["run", "--url", "x", "--bogus", "1"]).is_err());
        assert_eq!(
            options(&["run", "--url", "http://pi:8080/"]).target,
            Target::Url("http://pi:8080".into())
        );
        assert_eq!(
            parse(&["seed", "sqlite://x"]),
            Ok(Command::Seed("sqlite://x".into(), 600))
        );
    }

    #[test]
    fn faults_alternate_and_all_end_before_the_scouts_stop() {
        let o = options(&["run", "--server", "x"]);
        let plan = schedule(&o);
        let whats: Vec<&str> = plan.iter().map(|(_, w)| *w).collect();
        assert_eq!(
            whats,
            [
                "cable", "power", "cable", "power", "cable", "power", "cable"
            ]
        );
        assert_eq!(plan[0].0, Duration::from_secs(900));

        let short = options(&[
            "run",
            "--server",
            "x",
            "--duration",
            "5m",
            "--faults-every",
            "2m",
        ]);
        assert_eq!(schedule(&short).len(), 2);
        let none = options(&["run", "--server", "x", "--faults-every", "0"]);
        assert!(schedule(&none).is_empty());
    }

    #[test]
    fn there_are_matches_enough_for_the_quickest_scout() {
        let o = options(&["run", "--server", "x"]);
        // 7200s at 12s at the quickest is 600 saves.
        assert_eq!(matches_for(&o), 610);
    }
}
