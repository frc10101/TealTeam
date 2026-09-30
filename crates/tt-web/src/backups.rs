//! Backups (Q4): the ten-minute snapshots while serving, and the two commands
//! a person runs -- `tt-web backup` between match blocks and `tt-web
//! check-backup` to prove one restores. The snapshotting itself is
//! `tt_repo_sqlite::backup`.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use chrono::Utc;
use tokio::task::JoinHandle;
use tracing::{info, warn};
use tt_repo_sqlite::backup::{self, Restored};
use tt_repo_sqlite::storage;

use crate::config::Config;

/// Where the timed snapshots go: `BACKUP_DIR`, or a `backups` folder beside
/// the database. `None` for an in-memory database, which has nothing to keep.
/// The flag says whether the folder was named, and so must already exist.
pub fn timed_dir(config: &Config) -> Option<(PathBuf, bool)> {
    if let Some(dir) = &config.backup_dir {
        return Some((dir.clone(), true));
    }
    let db = storage::locate(&config.database_url)?.path?;
    Some((db.parent()?.join("backups"), false))
}

/// Snapshot every ten minutes, and prune to a day's worth.
///
/// A named folder is never created: on the Pi it lives on the SSD, and if the
/// SSD is missing, creating it would put the backups on the SD card. It is
/// checked again at each snapshot, so plugging the SSD back in is enough.
pub fn spawn(config: &Config) -> Option<JoinHandle<()>> {
    let Some((dir, named)) = timed_dir(config) else {
        info!("database is in memory; no backups");
        return None;
    };
    if !named && let Err(e) = std::fs::create_dir_all(&dir) {
        warn!("backups are off: cannot create {}: {e}", dir.display());
        return None;
    }
    describe(&dir);

    let url = config.database_url.clone();
    Some(tokio::spawn(async move {
        let every = backup::EVERY.to_std().expect("positive");
        let mut ticks = tokio::time::interval_at(tokio::time::Instant::now() + every, every);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticks.tick().await;
            let now = Utc::now();
            match backup::snapshot(&url, &dir, now).await {
                Ok(file) => info!("backed up to {}", file.display()),
                // Keep trying: a missing SSD may be plugged back in.
                Err(e) => {
                    warn!("backup failed: {e}");
                    continue;
                }
            }
            match backup::prune(&dir, now) {
                Ok(0) => {}
                Ok(n) => info!("removed {n} backup(s) older than a day"),
                Err(e) => warn!("pruning backups in {}: {e}", dir.display()),
            }
        }
    }))
}

fn describe(dir: &Path) {
    let dir = std::path::absolute(dir).unwrap_or(dir.to_path_buf());
    let at = storage::on_disk(dir);
    if at.on_sd_card() {
        warn!(
            "backing up every 10 minutes to {} -- the SD card, which is what the \
             backups are meant to survive. Set BACKUP_DIR to a folder on the SSD: \
             docs/PI_STORAGE.md",
            at.describe()
        );
    } else {
        info!(
            "backing up every 10 minutes to {}, keeping a day",
            at.describe()
        );
    }
}

/// `tt-web backup [folder]`: a fresh snapshot into `folder` (or
/// `BACKUP_COPY_TO`), restored straight away to check it.
pub async fn copy(
    config: &Config,
    to: Option<PathBuf>,
    out: &mut impl Write,
) -> anyhow::Result<()> {
    let Some(dir) = to.or_else(|| config.backup_copy_to.clone()) else {
        bail!(
            "say where to copy it: tt-web backup /media/usb, or set BACKUP_COPY_TO. \
             Where the off-site copy goes is open decision 6 in docs/ACTION_ITEMS.md"
        );
    };
    if storage::locate(&config.database_url).is_none_or(|at| at.path.is_none()) {
        bail!("the database is in memory; there is nothing on disk to back up");
    }
    let file = backup::snapshot(&config.database_url, &dir, Utc::now())
        .await
        .with_context(|| format!("backing up to {}", dir.display()))?;
    let size = std::fs::metadata(&file).map(|m| m.len()).unwrap_or(0);
    writeln!(out, "Backed up to {} ({})", file.display(), megabytes(size))?;
    let restored = backup::check(&file)
        .await
        .context("the copy was written but did not restore")?;
    report(&restored, out)?;
    writeln!(
        out,
        "It is on the disk. Eject the stick before pulling it out."
    )?;
    Ok(())
}

/// `tt-web check-backup [file]`: restore `file`, or the newest timed
/// snapshot, into a fresh database and say what came back.
pub async fn check(
    config: &Config,
    file: Option<PathBuf>,
    out: &mut impl Write,
) -> anyhow::Result<()> {
    let file = match file {
        Some(file) => file,
        None => {
            let Some((dir, _)) = timed_dir(config) else {
                bail!("the database is in memory, so there are no backups; name a file");
            };
            newest(&dir).with_context(|| format!("no backups in {}", dir.display()))?
        }
    };
    writeln!(out, "Restoring {} into a fresh database...", file.display())?;
    let restored = backup::check(&file).await.context("it did not restore")?;
    report(&restored, out)?;
    Ok(())
}

fn newest(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| {
            let name = e.ok()?.file_name().into_string().ok()?;
            Some((backup::taken_at(&name)?, name))
        })
        .max()
        .map(|(_, name)| dir.join(name))
}

fn report(restored: &Restored, out: &mut impl Write) -> std::io::Result<()> {
    writeln!(out, "Restored and checked: the database is whole. It holds")?;
    for (table, n) in &restored.counts {
        writeln!(out, "  {n:>7}  {}", table.replace('_', " "))?;
    }
    if restored.migrated > 0 {
        writeln!(
            out,
            "It was taken by an older build: {} migration(s) brought it up to date.",
            restored.migrated
        )?;
    }
    Ok(())
}

fn megabytes(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_000_000.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tt_repo::Repo;
    use tt_repo_sqlite::SqliteRepo;

    fn config(url: &str) -> Config {
        Config::from_lookup(|key| (key == "DATABASE_URL").then(|| url.to_string())).unwrap()
    }

    struct Scratch(PathBuf);
    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("tt-web-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn timed_backups_sit_beside_the_database_unless_named() {
        let at = config("sqlite:///srv/tealteam/data/tealteam.db");
        assert_eq!(
            timed_dir(&at),
            Some((PathBuf::from("/srv/tealteam/data/backups"), false))
        );
        let mut named = at.clone();
        named.backup_dir = Some("/srv/tealteam/backups".into());
        assert_eq!(
            timed_dir(&named),
            Some((PathBuf::from("/srv/tealteam/backups"), true))
        );
        assert_eq!(timed_dir(&config("sqlite::memory:")), None);
    }

    #[tokio::test]
    async fn the_copy_command_writes_restores_and_reports() {
        let scratch = Scratch::new("copy");
        let url = format!("sqlite://{}", scratch.0.join("tealteam.db").display());
        let repo = SqliteRepo::connect(&url).unwrap();
        tt_repo_sqlite::migrate::apply(repo.pool()).await.unwrap();
        let usb = scratch.0.join("usb");
        std::fs::create_dir(&usb).unwrap();

        let mut out = Vec::new();
        copy(&config(&url), Some(usb.clone()), &mut out)
            .await
            .unwrap();
        let out = String::from_utf8(out).unwrap();
        assert!(out.contains("Restored and checked"), "{out}");
        assert!(out.contains("observations"), "{out}");

        // And check-backup finds it by name.
        let file = newest(&usb).expect("one backup");
        let mut out = Vec::new();
        check(&config(&url), Some(file), &mut out).await.unwrap();
        assert!(
            String::from_utf8(out)
                .unwrap()
                .contains("the database is whole")
        );
    }

    #[tokio::test]
    async fn with_nowhere_to_copy_to_it_says_so() {
        let err = copy(&config("sqlite:///x/tealteam.db"), None, &mut Vec::new())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("open decision 6"), "{err}");
    }

    #[tokio::test]
    async fn a_missing_folder_is_an_error_not_a_new_folder() {
        let scratch = Scratch::new("missing");
        let url = format!("sqlite://{}", scratch.0.join("tealteam.db").display());
        // health() opens it, which creates the file.
        let _ = SqliteRepo::connect(&url).unwrap().health().await;
        let gone = scratch.0.join("not-plugged-in");
        let err = copy(&config(&url), Some(gone.clone()), &mut Vec::new())
            .await
            .unwrap_err();
        assert!(format!("{err:#}").contains("does not exist"), "{err:#}");
        assert!(!gone.exists());
    }
}
