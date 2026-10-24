//! tt-ops: the operator's terminal for the server beside it.
//!
//! Run it on the machine that serves -- the Pi over SSH, or the laptop --
//! and it shows the process, the network, the people and the data, follows
//! the log, and does the few things an event asks of its operator: restart,
//! back up, sync, import a schedule, fix an account, power off.
//!
//! It is deliberately not a page the server serves. Nothing new listens on
//! the LAN, the server binary carries no terminal code, and the tool works
//! while the server is down, which is when it is needed most. Status comes
//! from systemd (or the laptop's pidfile), the kernel, and the database
//! file itself; changes to the data go through the running server's own
//! forms as a signed-in admin, never around them.

mod app;
mod data;
mod logs;
mod report;
mod server;
mod site;
mod system;
mod ui;

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use ratatui::crossterm::event::{self, Event, KeyEventKind};
use tokio::sync::mpsc;

use crate::app::{App, Msg, REFRESH};
use crate::site::Site;

const USAGE: &str = "\
usage: tt-ops [--dir FOLDER] [--port N] [--once]

The operator's terminal for the TealTeam server on this machine.

  --dir FOLDER   the install: the folder with tt-web and its .env
                 (default: /opt/tealteam, then ~/tealteam-demo, then here)
  --port N       the server's HTTP port, if .env does not say
  --once         print the overview as plain text and exit (for a script,
                 or a shell with no terminal); exit 1 if the server is down
  --help         this message

Keys: 1-5 pick a tab, j/k move, Enter acts, q quits. Each tab's keys are
on the bottom line. docs/event/OPS_TERMINAL.md has the rest.";

fn main() -> ExitCode {
    let mut dir: Option<PathBuf> = None;
    let mut port: Option<u16> = None;
    let mut once = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--dir" => match args.next() {
                Some(d) => dir = Some(PathBuf::from(d)),
                None => return usage("--dir needs a folder"),
            },
            "--port" => match args.next().and_then(|p| p.parse().ok()) {
                Some(p) => port = Some(p),
                None => return usage("--port needs a number"),
            },
            "--once" => once = true,
            "-h" | "--help" | "help" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            other => return usage(&format!("unknown argument {other}")),
        }
    }

    let mut site = match Site::locate(dir) {
        Ok(site) => site,
        Err(e) => {
            eprintln!("tt-ops: {e:#}");
            return ExitCode::from(2);
        }
    };
    if let Some(port) = port {
        site.port = port;
    }
    // The server resolves a relative DATABASE_URL against its own folder.
    if std::env::set_current_dir(&site.dir).is_err() {
        eprintln!("tt-ops: cannot enter {}", site.dir.display());
        return ExitCode::from(2);
    }

    let runtime = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("tt-ops: {e}");
            return ExitCode::FAILURE;
        }
    };
    let outcome = if once {
        runtime.block_on(report::once(site))
    } else {
        runtime.block_on(run(site))
    };
    match outcome {
        Ok(code) => code,
        Err(e) => {
            eprintln!("tt-ops: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn usage(problem: &str) -> ExitCode {
    eprintln!("tt-ops: {problem}\n\n{USAGE}");
    ExitCode::from(2)
}

async fn run(site: Site) -> anyhow::Result<ExitCode> {
    // Lazy, like the server's: a missing file is a red card, not a crash.
    let repo = tt_repo_sqlite::SqliteRepo::connect(&site.database_url)?;
    let (tx, mut rx) = mpsc::unbounded_channel::<Msg>();

    // The keyboard, on its own thread: crossterm's read blocks.
    let keys = tx.clone();
    std::thread::spawn(move || {
        loop {
            match event::poll(Duration::from_millis(250)) {
                Ok(true) => match event::read() {
                    Ok(Event::Key(key)) if key.kind != KeyEventKind::Release => {
                        if keys.send(Msg::Key(key)).is_err() {
                            return;
                        }
                    }
                    Ok(Event::Resize(_, _)) => {
                        if keys.send(Msg::Resize).is_err() {
                            return;
                        }
                    }
                    Ok(_) => {}
                    Err(_) => {
                        let _ = keys.send(Msg::Quit);
                        return;
                    }
                },
                Ok(false) => {
                    if keys.is_closed() {
                        return;
                    }
                }
                Err(_) => {
                    let _ = keys.send(Msg::Quit);
                    return;
                }
            }
        }
    });

    let mut app = App::new(site.clone(), repo, tx.clone());
    tokio::spawn(logs::follow(site, tx));
    app.refresh();

    let mut terminal = ratatui::init();
    let mut tick = tokio::time::interval(REFRESH);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let outcome = loop {
        if let Err(e) = terminal.draw(|frame| ui::draw(frame, &app)) {
            break Err(anyhow::Error::from(e).context("drawing"));
        }
        tokio::select! {
            msg = rx.recv() => match msg {
                Some(msg) => app.handle(msg),
                None => break Ok(ExitCode::SUCCESS),
            },
            _ = tick.tick() => app.on_tick(),
        }
        // Drain what else arrived, so a burst of log lines is one redraw.
        while let Ok(msg) = rx.try_recv() {
            app.handle(msg);
        }
        if app.should_quit {
            break Ok(ExitCode::SUCCESS);
        }
    };
    ratatui::restore();
    app.site.fix_wal_ownership();
    outcome
}
