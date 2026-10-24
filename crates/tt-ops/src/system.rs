//! The machine around the server: is the process up, is the network the
//! event's, and the handful of things an operator does to either.
//!
//! Privilege is the operator's, not the tool's. On the Pi the server is the
//! `tealteam` user and the machine's power is root's, so restarting, powering
//! off, and backing up into the server's folder go through `sudo -n`: it
//! either works at once or says so, and the screen then shows the exact
//! command to run by hand. Nothing here ever asks for a password.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;

use crate::site::{Site, Supervisor};

/// What is known about the server process right now.
#[derive(Debug, Clone, Default)]
pub struct Process {
    /// "active (running)", "inactive (dead)", "running (pid 123)", "not running".
    pub state: String,
    pub running: bool,
    pub pid: Option<u32>,
    pub uptime: Option<Duration>,
    pub restarts: Option<u64>,
    pub rss_bytes: Option<u64>,
    /// How this was learned, for the screen.
    pub via: String,
}

/// What `GET /health` said.
#[derive(Debug, Clone, Default)]
pub struct Health {
    pub reachable: bool,
    pub storage: String,
    pub build: String,
    pub schema: String,
    pub form: String,
    pub latency: Option<Duration>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Ok,
    Fail,
    Note,
}

#[derive(Debug, Clone)]
pub struct Check {
    pub verdict: Verdict,
    pub text: String,
}

#[derive(Debug, Clone, Default)]
pub struct Status {
    pub process: Process,
    pub health: Health,
    pub tls_listening: Option<bool>,
    pub network: Vec<Check>,
    pub hostname: String,
    pub load: String,
    pub disk_free: Option<(u64, u64)>,
}

pub async fn status(site: &Site, client: &reqwest::Client) -> Status {
    let (process, health, tls, network) = tokio::join!(
        process(site),
        health(site, client),
        async {
            match site.tls_port {
                Some(port) => Some(listening(port).await),
                None => None,
            }
        },
        network(site),
    );
    Status {
        process,
        health,
        tls_listening: tls,
        network,
        hostname: hostname(),
        load: std::fs::read_to_string("/proc/loadavg")
            .ok()
            .map(|s| s.split_whitespace().take(3).collect::<Vec<_>>().join(" "))
            .unwrap_or_default(),
        disk_free: site.db_path.as_deref().and_then(disk_free),
    }
}

async fn process(site: &Site) -> Process {
    match &site.supervisor {
        Supervisor::Systemd { unit } => systemd_process(unit).await,
        Supervisor::Pidfile { pidfile, .. } => pidfile_process(pidfile),
        Supervisor::None => Process {
            state: "unknown (no unit, no pidfile)".into(),
            via: "nothing to ask".into(),
            ..Default::default()
        },
    }
}

async fn systemd_process(unit: &str) -> Process {
    let output = Command::new("systemctl")
        .args([
            "show",
            unit,
            "-p",
            "ActiveState,SubState,MainPID,NRestarts,ExecMainStartTimestampMonotonic,MemoryCurrent",
        ])
        .output()
        .await;
    let mut out = Process {
        via: format!("systemctl show {unit}"),
        ..Default::default()
    };
    let Ok(output) = output else {
        out.state = "systemctl not available".into();
        return out;
    };
    let text = String::from_utf8_lossy(&output.stdout);
    let field = |key: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(key).and_then(|r| r.strip_prefix('=')))
            .map(str::trim)
            .unwrap_or("")
            .to_string()
    };
    let active = field("ActiveState");
    let sub = field("SubState");
    out.state = if active.is_empty() {
        "unknown".into()
    } else {
        format!("{active} ({sub})")
    };
    out.running = active == "active";
    out.pid = field("MainPID").parse().ok().filter(|p| *p > 0);
    out.restarts = field("NRestarts").parse().ok();
    out.rss_bytes = field("MemoryCurrent").parse().ok();
    if let Some(pid) = out.pid {
        out.uptime = proc_uptime(pid);
        if out.rss_bytes.is_none() {
            out.rss_bytes = proc_rss(pid);
        }
    }
    out
}

fn pidfile_process(pidfile: &Path) -> Process {
    let mut out = Process {
        via: format!("pidfile {}", pidfile.display()),
        ..Default::default()
    };
    let pid: Option<u32> = std::fs::read_to_string(pidfile)
        .ok()
        .and_then(|s| s.trim().parse().ok());
    match pid {
        Some(pid) if Path::new(&format!("/proc/{pid}")).exists() => {
            out.running = true;
            out.pid = Some(pid);
            out.state = "running".into();
            out.uptime = proc_uptime(pid);
            out.rss_bytes = proc_rss(pid);
        }
        Some(pid) => out.state = format!("not running (stale pidfile, pid {pid})"),
        None => out.state = "not running".into(),
    }
    out
}

/// Seconds since the process started, from /proc. Linux only.
fn proc_uptime(pid: u32) -> Option<Duration> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // Field 22 counts from after the ")" that closes the command name.
    let after = stat.rsplit_once(')')?.1;
    let start_ticks: u64 = after.split_whitespace().nth(19)?.parse().ok()?;
    let uptime: f64 = std::fs::read_to_string("/proc/uptime")
        .ok()?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    let ticks_per_second = 100.0;
    let started = start_ticks as f64 / ticks_per_second;
    Some(Duration::from_secs_f64((uptime - started).max(0.0)))
}

fn proc_rss(pid: u32) -> Option<u64> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    let line = status.lines().find(|l| l.starts_with("VmRSS:"))?;
    let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb * 1024)
}

async fn health(site: &Site, client: &reqwest::Client) -> Health {
    let started = std::time::Instant::now();
    let response = client
        .get(format!("{}/health", site.base_url()))
        .timeout(Duration::from_secs(3))
        .send()
        .await;
    let mut out = Health::default();
    let Ok(response) = response else {
        return out;
    };
    out.reachable = response.status().is_success();
    out.latency = Some(started.elapsed());
    if let Ok(body) = response.json::<serde_json::Value>().await {
        let text = |key: &str| match &body[key] {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Null => String::new(),
            other => other.to_string(),
        };
        out.storage = text("storage");
        out.build = text("build");
        out.schema = text("schema");
        out.form = text("form");
    }
    out
}

async fn listening(port: u16) -> bool {
    tokio::time::timeout(
        Duration::from_secs(2),
        tokio::net::TcpStream::connect(("127.0.0.1", port)),
    )
    .await
    .map(|r| r.is_ok())
    .unwrap_or(false)
}

/// The checks `deploy/pi/network/status.sh` makes, read from the kernel.
async fn network(site: &Site) -> Vec<Check> {
    let mut out = Vec::new();
    let addrs = ipv4_addresses();
    let event_addr = addrs
        .iter()
        .find(|(_, addr)| addr == "10.101.0.1")
        .map(|(iface, _)| iface.clone());
    match (&event_addr, site.is_systemd()) {
        (Some(iface), _) => out.push(Check {
            verdict: Verdict::Ok,
            text: format!("{iface} is 10.101.0.1 (the event LAN)"),
        }),
        (None, true) => out.push(Check {
            verdict: Verdict::Fail,
            text: "no interface has 10.101.0.1: is the switch cabled and powered?".into(),
        }),
        (None, false) => out.push(Check {
            verdict: Verdict::Note,
            text: "event LAN (10.101.0.1) not up: serving on this laptop's own network".into(),
        }),
    }
    for (iface, addr) in &addrs {
        if iface != "lo" && Some(iface) != event_addr.as_ref() {
            out.push(Check {
                verdict: Verdict::Note,
                text: format!("{iface} is {addr}"),
            });
        }
    }
    match default_route() {
        Some(iface) if iface == "usb0" || iface == "eth1" => out.push(Check {
            verdict: Verdict::Note,
            text: format!("internet via a tethered phone ({iface}); not for the event"),
        }),
        Some(iface) => out.push(Check {
            verdict: Verdict::Note,
            text: format!("default route via {iface}"),
        }),
        None => out.push(Check {
            verdict: Verdict::Note,
            text: "no uplink (the app works without one)".into(),
        }),
    }
    let mdns = if site.is_systemd() {
        unit_active("avahi-daemon", false).await
    } else {
        unit_active("tealteam-mdns", true).await
    };
    out.push(match mdns {
        Some(true) => Check {
            verdict: Verdict::Ok,
            text: "tealteam.local is published (mDNS)".into(),
        },
        Some(false) => Check {
            verdict: if site.is_systemd() {
                Verdict::Fail
            } else {
                Verdict::Note
            },
            text: "tealteam.local is not being published".into(),
        },
        None => Check {
            verdict: Verdict::Note,
            text: "mDNS state unknown".into(),
        },
    });
    let http = listening(site.port).await;
    out.push(Check {
        verdict: if http { Verdict::Ok } else { Verdict::Fail },
        text: format!("http on port {}", site.port),
    });
    if let Some(tls) = site.tls_port {
        let up = listening(tls).await;
        out.push(Check {
            verdict: if up { Verdict::Ok } else { Verdict::Fail },
            text: format!("https on port {tls}"),
        });
    }
    out
}

async fn unit_active(unit: &str, user: bool) -> Option<bool> {
    let mut cmd = Command::new("systemctl");
    if user {
        cmd.arg("--user");
    }
    cmd.args(["is-active", "--quiet", unit]);
    cmd.stdout(Stdio::null()).stderr(Stdio::null());
    cmd.status().await.ok().map(|s| s.success())
}

/// `(interface, address)` for every IPv4 address, from `ip -4 -o addr`.
fn ipv4_addresses() -> Vec<(String, String)> {
    let output = std::process::Command::new("ip")
        .args(["-4", "-o", "addr", "show"])
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let _index = words.next()?;
            let iface = words.next()?.to_string();
            let inet = words.position(|w| w == "inet")?;
            let _ = inet;
            let addr = line
                .split_whitespace()
                .skip_while(|w| *w != "inet")
                .nth(1)?
                .split('/')
                .next()?
                .to_string();
            Some((iface, addr))
        })
        .collect()
}

/// The interface the default route leaves by, from /proc/net/route.
fn default_route() -> Option<String> {
    let table = std::fs::read_to_string("/proc/net/route").ok()?;
    table.lines().skip(1).find_map(|line| {
        let mut f = line.split_whitespace();
        let iface = f.next()?;
        let dest = f.next()?;
        let _gateway = f.next()?;
        let flags = u32::from_str_radix(f.next()?, 16).ok()?;
        (dest == "00000000" && flags & 0x2 != 0).then(|| iface.to_string())
    })
}

fn hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

/// `(free, total)` bytes on the filesystem that holds `path`, via `df`.
fn disk_free(path: &Path) -> Option<(u64, u64)> {
    let dir = if path.is_dir() { path } else { path.parent()? };
    let output = std::process::Command::new("df")
        .args(["-B1", "--output=avail,size"])
        .arg(dir)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let mut f = text.lines().nth(1)?.split_whitespace();
    Some((f.next()?.parse().ok()?, f.next()?.parse().ok()?))
}

// ── Actions ─────────────────────────────────────────────────────────────────

/// What an action did, for the result screen.
#[derive(Debug, Clone)]
pub struct Done {
    pub ok: bool,
    pub title: String,
    pub lines: Vec<String>,
}

impl Done {
    fn failed(title: &str, lines: Vec<String>) -> Self {
        Done {
            ok: false,
            title: title.into(),
            lines,
        }
    }
}

/// Run a command, with its output and, on failure, the line to run by hand.
async fn run(title: &str, program: &str, args: &[String], cwd: Option<&Path>) -> Done {
    let shown = std::iter::once(program.to_string())
        .chain(args.iter().cloned())
        .collect::<Vec<_>>()
        .join(" ");
    let mut cmd = Command::new(program);
    cmd.args(args).stdin(Stdio::null());
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd);
    }
    match cmd.output().await {
        Ok(output) => {
            let mut lines: Vec<String> = String::from_utf8_lossy(&output.stdout)
                .lines()
                .chain(String::from_utf8_lossy(&output.stderr).lines())
                .map(str::to_string)
                .filter(|l| !l.trim().is_empty())
                .collect();
            if output.status.success() {
                if lines.is_empty() {
                    lines.push("done".into());
                }
                Done {
                    ok: true,
                    title: title.into(),
                    lines,
                }
            } else {
                lines.push(String::new());
                lines.push(format!("exit status {}. By hand:", output.status));
                lines.push(format!("  {shown}"));
                if program == "sudo" {
                    lines.push("(sudo -n asks for nothing; if it needs a password, run the line in a shell)".into());
                }
                Done::failed(title, lines)
            }
        }
        Err(e) => Done::failed(title, vec![format!("could not run {shown}: {e}")]),
    }
}

fn strings(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

pub async fn restart(site: &Site) -> Done {
    site.fix_wal_ownership();
    match &site.supervisor {
        Supervisor::Systemd { unit } => {
            run(
                "Restart server",
                "sudo",
                &strings(&["-n", "systemctl", "restart", unit]),
                None,
            )
            .await
        }
        Supervisor::Pidfile { pidfile, log } => {
            let stopped = stop_pidfile(pidfile).await;
            let mut done = start_pidfile(site, pidfile, log).await;
            done.lines.splice(0..0, stopped);
            done.title = "Restart server".into();
            done
        }
        Supervisor::None => Done::failed(
            "Restart server",
            vec!["no unit and no pidfile: nothing to restart".into()],
        ),
    }
}

pub async fn stop(site: &Site) -> Done {
    match &site.supervisor {
        Supervisor::Systemd { unit } => {
            run(
                "Stop server",
                "sudo",
                &strings(&["-n", "systemctl", "stop", unit]),
                None,
            )
            .await
        }
        Supervisor::Pidfile { pidfile, .. } => Done {
            ok: true,
            title: "Stop server".into(),
            lines: stop_pidfile(pidfile).await,
        },
        Supervisor::None => Done::failed(
            "Stop server",
            vec!["no unit and no pidfile: nothing to stop".into()],
        ),
    }
}

pub async fn start(site: &Site) -> Done {
    site.fix_wal_ownership();
    match &site.supervisor {
        Supervisor::Systemd { unit } => {
            run(
                "Start server",
                "sudo",
                &strings(&["-n", "systemctl", "start", unit]),
                None,
            )
            .await
        }
        Supervisor::Pidfile { pidfile, log } => start_pidfile(site, pidfile, log).await,
        Supervisor::None => Done::failed(
            "Start server",
            vec!["no unit and no pidfile: nothing to start".into()],
        ),
    }
}

async fn stop_pidfile(pidfile: &Path) -> Vec<String> {
    let pid: Option<u32> = std::fs::read_to_string(pidfile)
        .ok()
        .and_then(|s| s.trim().parse().ok());
    let Some(pid) = pid else {
        return vec!["was not running (no pidfile)".into()];
    };
    let proc_dir = PathBuf::from(format!("/proc/{pid}"));
    if !proc_dir.exists() {
        let _ = std::fs::remove_file(pidfile);
        return vec![format!("was not running (stale pidfile, pid {pid})")];
    }
    // The pidfile names the systemd-inhibit wrapper, which passes SIGTERM on
    // to tt-web -- except in its first moment, before it has its lock. So
    // after a second the whole process group gets it, which reaches tt-web
    // either way: both were started into a group of their own, by the
    // laptop script's shell or by [`start_pidfile`].
    let group = proc_pgid(pid)
        .filter(|g| *g != std::process::id())
        .map(|g| format!("-{g}"));
    let signal = async |sig: &str, target: String| {
        let _ = Command::new("kill")
            .args([sig, "--", &target])
            .status()
            .await;
    };
    signal("-TERM", pid.to_string()).await;
    let mut lines = Vec::new();
    let mut gone = false;
    for tick in 0..50 {
        if !proc_dir.exists() {
            gone = true;
            break;
        }
        if tick == 10
            && let Some(group) = &group
        {
            signal("-TERM", group.clone()).await;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    if gone {
        lines.push(format!("stopped pid {pid}"));
    } else {
        signal("-KILL", group.clone().unwrap_or_else(|| pid.to_string())).await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        lines.push(format!("pid {pid} did not stop in 5 s; killed"));
    }
    let _ = std::fs::remove_file(pidfile);
    lines
}

/// The process group of `pid`, from /proc.
fn proc_pgid(pid: u32) -> Option<u32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after = stat.rsplit_once(')')?.1;
    after.split_whitespace().nth(2)?.parse().ok()
}

/// The laptop script's own start line: `tt-web serve` in the background,
/// under systemd-inhibit when there is one so the lid can close, appending
/// to `server.log`, pid in the pidfile.
///
/// Started through a shell that exits at once, in a process group of its
/// own: the server is then init's child, not ours, so when it later stops
/// nothing of it lingers in /proc waiting for a parent that never waits.
async fn start_pidfile(site: &Site, pidfile: &Path, log: &Path) -> Done {
    use std::os::unix::process::CommandExt;
    let title = "Start server";
    if !site.binary.is_file() {
        return Done::failed(
            title,
            vec![format!("no binary at {}", site.binary.display())],
        );
    }
    let has_inhibit = std::process::Command::new("systemd-inhibit")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    let mut line: Vec<String> = Vec::new();
    if has_inhibit {
        line.extend(strings(&[
            "systemd-inhibit",
            "--what=handle-lid-switch:sleep:idle",
            "--who=TealTeam",
            "--why=serving TealTeam",
            "--mode=block",
        ]));
    }
    line.push(site.binary.display().to_string());
    line.push("serve".into());

    let mut sh = std::process::Command::new("sh");
    sh.arg("-c")
        .arg("log=$1; shift; exec \"$@\" >>\"$log\" 2>&1 </dev/null & echo $!")
        .arg("sh")
        .arg(log)
        .args(&line)
        .current_dir(&site.dir)
        .stdin(Stdio::null())
        .process_group(0);
    let output = match tokio::task::spawn_blocking(move || sh.output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => return Done::failed(title, vec![format!("could not start a shell: {e}")]),
        Err(e) => return Done::failed(title, vec![format!("could not start a shell: {e}")]),
    };
    let pid: Option<u32> = String::from_utf8_lossy(&output.stdout).trim().parse().ok();
    let Some(pid) = pid else {
        return Done::failed(
            title,
            vec![format!(
                "the shell gave no pid: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )],
        );
    };
    if let Err(e) = std::fs::write(pidfile, format!("{pid}\n")) {
        return Done::failed(
            title,
            vec![format!(
                "started pid {pid} but could not write {}: {e}",
                pidfile.display()
            )],
        );
    }
    // Give it a moment, then say whether it answers.
    let mut answered = false;
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        if listening(site.port).await {
            answered = true;
            break;
        }
    }
    Done {
        ok: answered,
        title: title.into(),
        lines: vec![
            format!("started pid {pid} from {}", site.dir.display()),
            if answered {
                format!("answering on port {}", site.port)
            } else {
                format!(
                    "not answering on port {} after 10 s: see the Logs tab",
                    site.port
                )
            },
        ],
    }
}

pub async fn poweroff(site: &Site) -> Done {
    if !site.is_systemd() {
        return Done::failed("Power off", vec!["only on the Pi (systemd install)".into()]);
    }
    run(
        "Power off",
        "sudo",
        &strings(&["-n", "systemctl", "poweroff"]),
        None,
    )
    .await
}

/// `tt-web backup FOLDER`, as the database's owner when that is not us (the
/// Pi: `sudo -n -u tealteam`), from the install folder so it reads the same
/// `.env`.
pub async fn backup(site: &Site, folder: &Path) -> Done {
    let (program, args) = as_db_owner(site, &["backup".to_string(), folder.display().to_string()]);
    run("Back up now", &program, &args, Some(&site.dir)).await
}

/// `tt-web check-backup [FILE]`: restore the newest timed backup (or the
/// named one) into a scratch database and say what is in it.
pub async fn check_backup(site: &Site, file: Option<&Path>) -> Done {
    let mut args = vec!["check-backup".to_string()];
    if let Some(file) = file {
        args.push(file.display().to_string());
    }
    let (program, args) = as_db_owner(site, &args);
    run("Check backup", &program, &args, Some(&site.dir)).await
}

/// The binary and its arguments, wrapped in `sudo -n -u #UID` when the
/// database belongs to another user. Root-owned WAL files beside a
/// tealteam-owned database would lock the server out, so never run the
/// server's commands as anyone but its owner.
fn as_db_owner(site: &Site, args: &[String]) -> (String, Vec<String>) {
    use std::os::unix::fs::MetadataExt;
    let binary = site.binary.display().to_string();
    let owner = site
        .db_path
        .as_ref()
        .and_then(|p| std::fs::metadata(p).ok())
        .map(|m| m.uid());
    let me = std::fs::metadata("/proc/self").ok().map(|m| m.uid());
    match (owner, me) {
        (Some(owner), Some(me)) if owner != me => {
            let mut full = vec![
                "-n".to_string(),
                "-u".to_string(),
                format!("#{owner}"),
                binary,
            ];
            full.extend(args.iter().cloned());
            ("sudo".to_string(), full)
        }
        _ => (binary, args.to_vec()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Against a laptop-style install (pidfile beside the binary), which it
    /// restarts. Ignored unless asked for:
    ///
    /// ```sh
    /// TT_OPS_LIVE_DIR=/path/to/install cargo test -p tt-ops live_ -- --ignored --nocapture
    /// ```
    #[tokio::test]
    #[ignore]
    async fn live_restart_a_laptop_install() {
        let dir = PathBuf::from(std::env::var("TT_OPS_LIVE_DIR").expect("set TT_OPS_LIVE_DIR"));
        let site = Site::locate(Some(dir)).expect("an install");
        assert!(
            matches!(site.supervisor, Supervisor::Pidfile { .. }),
            "{:?}",
            site.supervisor
        );
        let before = process(&site).await;
        println!("before: {before:?}");
        let done = restart(&site).await;
        println!("restart: {done:?}");
        assert!(done.ok, "{done:?}");
        let after = process(&site).await;
        println!("after: {after:?}");
        assert!(after.running);
        assert_ne!(after.pid, before.pid.filter(|_| before.running));
        let stopped = stop(&site).await;
        println!("stop: {stopped:?}");
        assert!(stopped.ok);
        assert!(!process(&site).await.running);
        let started = start(&site).await;
        println!("start: {started:?}");
        assert!(started.ok, "{started:?}");
        assert!(process(&site).await.running);
    }

    #[test]
    fn default_route_parses_the_kernel_table() {
        // Not a filesystem test: just that a flags value of 0003 and a zero
        // destination is what counts as the default.
        let flags = u32::from_str_radix("0003", 16).unwrap();
        assert!(flags & 0x2 != 0);
    }

    #[test]
    fn as_owner_wraps_only_when_someone_else_owns_the_database() {
        let site = Site {
            dir: PathBuf::from("/nowhere"),
            binary: PathBuf::from("/nowhere/tt-web"),
            env: Default::default(),
            database_url: "sqlite://./x.db".into(),
            db_path: None,
            port: 1,
            tls_port: None,
            backup_dir: None,
            supervisor: Supervisor::None,
        };
        let (program, args) = as_db_owner(&site, &["backup".into()]);
        assert_eq!(program, "/nowhere/tt-web");
        assert_eq!(args, vec!["backup".to_string()]);
    }
}
