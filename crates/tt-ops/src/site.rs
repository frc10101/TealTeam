//! Where the server lives on this machine, and how it is kept running.
//!
//! Two installs exist. The Pi runs `/opt/tealteam/tt-web` as the `tealteam`
//! system user under systemd (deploy/pi/service). The Omen laptop runs
//! `~/tealteam-demo/tt-web` as the person at the keyboard, in the background
//! with a pidfile and a `server.log` (deploy/laptop/omenMaxwell-setup.sh).
//! Both keep a `.env` beside the binary, so that folder is all this needs to
//! find: the rest is read from the file, the way tt-web reads it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, bail};

/// How the server process is started and stopped here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Supervisor {
    /// A systemd unit, the Pi's `tealteam.service`.
    Systemd { unit: String },
    /// A background process the laptop script started, with its pidfile and
    /// log beside the binary.
    Pidfile { pidfile: PathBuf, log: PathBuf },
    /// Nothing recognised: status and data still work, process control is off.
    None,
}

#[derive(Debug, Clone)]
pub struct Site {
    /// The folder with the binary and `.env`; the process's working directory.
    pub dir: PathBuf,
    pub binary: PathBuf,
    pub env: BTreeMap<String, String>,
    pub database_url: String,
    /// The database file, absolute. `None` when the URL names memory.
    pub db_path: Option<PathBuf>,
    pub port: u16,
    pub tls_port: Option<u16>,
    pub backup_dir: Option<PathBuf>,
    pub supervisor: Supervisor,
}

impl Site {
    /// `dir` from the command line, else the first of `/opt/tealteam`,
    /// `~/tealteam-demo`, and the current folder that holds a `.env`.
    pub fn locate(dir: Option<PathBuf>) -> anyhow::Result<Site> {
        let dir = match dir {
            Some(dir) => dir,
            None => {
                let mut candidates = vec![PathBuf::from("/opt/tealteam")];
                if let Ok(home) = std::env::var("HOME") {
                    candidates.push(Path::new(&home).join("tealteam-demo"));
                }
                if let Ok(cwd) = std::env::current_dir() {
                    candidates.push(cwd);
                }
                match candidates.iter().find(|d| d.join(".env").is_file()) {
                    Some(found) => found.clone(),
                    None => bail!(
                        "no server install found: none of {} has a .env. Name the folder with --dir",
                        candidates
                            .iter()
                            .map(|d| d.display().to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                }
            }
        };
        let dir = dir
            .canonicalize()
            .with_context(|| format!("{} does not exist", dir.display()))?;

        let mut env = BTreeMap::new();
        let dotenv = dir.join(".env");
        if let Ok(content) = std::fs::read_to_string(&dotenv) {
            for (key, value) in parse_dotenv(&content) {
                env.insert(key, value);
            }
        }
        // The process environment wins, as it does for tt-web.
        for key in ["DATABASE_URL", "PORT", "TLS_PORT", "BACKUP_DIR"] {
            if let Ok(value) = std::env::var(key) {
                env.insert(key.to_string(), value);
            }
        }

        let database_url = env
            .get("DATABASE_URL")
            .cloned()
            .unwrap_or_else(|| "sqlite://./tealteam.db".to_string());
        let db_path = database_path(&database_url, &dir);
        let port = env.get("PORT").and_then(|p| p.parse().ok()).unwrap_or(8080);
        let tls_port = match env.get("TLS_PORT").map(String::as_str) {
            None | Some("") | Some("off") | Some("0") => None,
            Some(p) => p.parse().ok(),
        };
        let backup_dir = env
            .get("BACKUP_DIR")
            .filter(|d| !d.is_empty())
            .map(|d| absolute(Path::new(d), &dir))
            .or_else(|| {
                db_path
                    .as_ref()
                    .and_then(|p| p.parent())
                    .map(|p| p.join("backups"))
            });

        let binary = dir.join("tt-web");
        let supervisor = detect_supervisor(&dir);

        Ok(Site {
            dir,
            binary,
            env,
            database_url,
            db_path,
            port,
            tls_port,
            backup_dir,
            supervisor,
        })
    }

    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// A word for the install, for the title bar.
    pub fn kind(&self) -> &'static str {
        match self.supervisor {
            Supervisor::Systemd { .. } => "systemd",
            Supervisor::Pidfile { .. } => "laptop",
            Supervisor::None => "unsupervised",
        }
    }

    /// Whether this is the Pi-style install, where the operator is not the
    /// server's user and the machine can be powered off from here.
    pub fn is_systemd(&self) -> bool {
        matches!(self.supervisor, Supervisor::Systemd { .. })
    }

    /// Root beside a database another user owns leaves root-owned `-wal` and
    /// `-shm` files when it was the only connection, and the server then
    /// cannot open its own database. Give them back to the database's owner.
    /// Called after each read and before the server is started; a no-op when
    /// the files are already theirs, or we are not root.
    pub fn fix_wal_ownership(&self) {
        use std::os::unix::fs::MetadataExt;
        let Some(db) = &self.db_path else { return };
        let Ok(meta) = std::fs::metadata(db) else {
            return;
        };
        let me = std::fs::metadata("/proc/self")
            .map(|m| m.uid())
            .unwrap_or(u32::MAX);
        if meta.uid() == me {
            return;
        }
        for suffix in ["-wal", "-shm", "-journal"] {
            let mut side = db.as_os_str().to_owned();
            side.push(suffix);
            let side = PathBuf::from(side);
            if let Ok(side_meta) = std::fs::metadata(&side)
                && side_meta.uid() != meta.uid()
            {
                let _ = std::os::unix::fs::chown(&side, Some(meta.uid()), Some(meta.gid()));
            }
        }
    }
}

/// The unit when systemd knows one called `tealteam`; else the laptop's
/// pidfile when the folder has one, or has ever had a log.
fn detect_supervisor(dir: &Path) -> Supervisor {
    let unit = "tealteam".to_string();
    let known = Command::new("systemctl")
        .args(["cat", "--no-pager", &unit])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if known {
        return Supervisor::Systemd { unit };
    }
    let pidfile = dir.join("server.pid");
    let log = dir.join("server.log");
    if pidfile.exists() || log.exists() {
        return Supervisor::Pidfile { pidfile, log };
    }
    Supervisor::None
}

/// The file a SQLite URL names, absolute against `dir` (the server's working
/// directory). `None` for `:memory:`.
pub fn database_path(url: &str, dir: &Path) -> Option<PathBuf> {
    if url.contains(":memory:") {
        return None;
    }
    let rest = url
        .strip_prefix("sqlite://")
        .or_else(|| url.strip_prefix("sqlite:"))
        .unwrap_or(url);
    let rest = rest.split('?').next().unwrap_or(rest);
    if rest.is_empty() {
        return None;
    }
    Some(absolute(Path::new(rest), dir))
}

fn absolute(path: &Path, dir: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        let relative: PathBuf = path
            .components()
            .filter(|c| !matches!(c, std::path::Component::CurDir))
            .collect();
        dir.join(relative)
    }
}

/// The same rules as tt-web's config: `KEY=VALUE`, an optional `export `,
/// matching quotes stripped, `#` lines skipped.
pub fn parse_dotenv(content: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        let value = value.trim();
        let value = value
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
            .unwrap_or(value);
        out.push((key.to_string(), value.to_string()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn database_path_resolves_relative_urls_against_the_install() {
        let dir = Path::new("/opt/tealteam");
        assert_eq!(
            database_path("sqlite://./tealteam.db", dir),
            Some(PathBuf::from("/opt/tealteam/tealteam.db"))
        );
        assert_eq!(
            database_path("sqlite:///srv/tealteam/data/tealteam.db?mode=rwc", dir),
            Some(PathBuf::from("/srv/tealteam/data/tealteam.db"))
        );
        assert_eq!(database_path("sqlite::memory:", dir), None);
    }

    #[test]
    fn dotenv_strips_quotes_and_export() {
        let parsed = parse_dotenv("# c\nexport A=\"x y\"\nB='z'\nC=plain\n\nbad line\n");
        assert_eq!(
            parsed,
            vec![
                ("A".into(), "x y".into()),
                ("B".into(), "z".into()),
                ("C".into(), "plain".into()),
            ]
        );
    }
}
