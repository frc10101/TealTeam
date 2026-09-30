//! Where the database actually lives (P3).
//!
//! An SD card wears out under a database's small, constant writes, and the
//! time it fails is mid-event: the Pi's database belongs on an NVMe or USB SSD
//! (REBUILD_SPEC.md 10, docs/PI_STORAGE.md). The server cannot move it there,
//! but it can say where it is, so a database left on the SD card is a warning
//! at startup and not a surprise on Saturday afternoon.
//!
//! Linux only, and best effort: the block device is looked up through
//! `/sys/dev/block`, and anything that cannot be resolved (a container's
//! overlay, tmpfs, another OS) is reported as unknown, never as an error.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use sqlx::sqlite::SqliteConnectOptions;

/// What a database file sits on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Medium {
    /// `sqlite::memory:`: nothing on disk at all.
    Memory,
    /// An `mmcblk` device: the Pi's SD card, or eMMC, which wears the same way.
    SdCard { device: String },
    /// Any other block device: `nvme0n1p1`, `sda1`, ...
    Disk { device: String },
    /// No block device could be found for it.
    Unknown,
}

impl Medium {
    /// From a block device's kernel name.
    pub fn of_device(device: &str) -> Medium {
        let device = device.to_owned();
        if device.starts_with("mmcblk") {
            Medium::SdCard { device }
        } else {
            Medium::Disk { device }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    /// Absolute, even when the URL was relative. `None` in memory.
    pub path: Option<PathBuf>,
    pub medium: Medium,
}

impl Location {
    /// For the startup log.
    pub fn describe(&self) -> String {
        let Some(path) = &self.path else {
            return "in memory: nothing is kept after the server stops".into();
        };
        let on = match &self.medium {
            Medium::SdCard { device } => format!("the SD card ({device})"),
            Medium::Disk { device } => device.clone(),
            Medium::Unknown | Medium::Memory => "an unknown device".into(),
        };
        format!("{} on {on}", path.display())
    }

    pub fn on_sd_card(&self) -> bool {
        matches!(self.medium, Medium::SdCard { .. })
    }
}

/// Where `url` puts the database. `None` when the URL does not parse, which
/// `SqliteRepo::connect` reports in its own words.
pub fn locate(url: &str) -> Option<Location> {
    let options = SqliteConnectOptions::from_str(url).ok()?;
    let file = options.get_filename();
    if file.as_os_str().is_empty() || file == Path::new(":memory:") || url.contains(":memory:") {
        return Some(Location {
            path: None,
            medium: Medium::Memory,
        });
    }
    let path = if file.is_absolute() {
        file.to_path_buf()
    } else {
        // Without the "./", so the log shows a path someone can paste.
        let relative: PathBuf = file
            .components()
            .filter(|c| !matches!(c, std::path::Component::CurDir))
            .collect();
        std::env::current_dir().ok()?.join(relative)
    };
    Some(on_disk(path))
}

/// Where an absolute path is stored: a database file, or a backup folder.
pub fn on_disk(path: PathBuf) -> Location {
    // The file, or its directory, may not exist yet: the device is the one
    // under the nearest part of the path that does.
    let medium = path
        .ancestors()
        .find(|p| p.exists())
        .and_then(block_device)
        .map(|device| Medium::of_device(&device))
        .unwrap_or(Medium::Unknown);
    Location {
        path: Some(path),
        medium,
    }
}

/// Split a Linux `st_dev` into major and minor numbers, as glibc's
/// `gnu_dev_major` / `gnu_dev_minor` do.
pub fn major_minor(dev: u64) -> (u64, u64) {
    let major = ((dev >> 8) & 0xfff) | ((dev >> 32) & 0xffff_f000);
    let minor = (dev & 0xff) | ((dev >> 12) & 0xffff_ff00);
    (major, minor)
}

/// The kernel name of the block device `path` is stored on: `mmcblk0p2`.
///
/// By device number through `/sys/dev/block` first. Some filesystems (btrfs,
/// for one) report a device number with no block device behind it, so failing
/// that, by the mount the path is under, from `/proc/self/mountinfo`.
#[cfg(target_os = "linux")]
fn block_device(path: &Path) -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    let (major, minor) = major_minor(std::fs::metadata(path).ok()?.dev());
    if let Ok(target) = std::fs::read_link(format!("/sys/dev/block/{major}:{minor}"))
        && let Some(name) = target.file_name().and_then(|n| n.to_str())
    {
        return Some(name.to_owned());
    }
    let source = mount_source(
        &std::fs::read_to_string("/proc/self/mountinfo").ok()?,
        &std::fs::canonicalize(path).ok()?,
    )?;
    // /dev/mapper/root is a link to /dev/dm-0; the kernel name is the target's.
    let device = std::fs::canonicalize(&source).unwrap_or(source.into());
    Some(device.file_name()?.to_str()?.to_owned())
}

#[cfg(not(target_os = "linux"))]
fn block_device(_path: &Path) -> Option<String> {
    None
}

/// The `/dev/...` source of the deepest mount containing `path`, from the text
/// of `/proc/self/mountinfo`.
pub fn mount_source(mountinfo: &str, path: &Path) -> Option<String> {
    mountinfo
        .lines()
        .filter_map(|line| {
            let (mount, rest) = line.split_once(" - ")?;
            // Field 5 is the mount point, with spaces escaped as \040.
            let point = mount.split(' ').nth(4)?.replace("\\040", " ");
            let source = rest.split(' ').nth(1)?;
            Some((PathBuf::from(point), source))
        })
        .filter(|(point, _)| path.starts_with(point))
        .max_by_key(|(point, _)| point.components().count())
        .map(|(_, source)| source)
        .filter(|source| source.starts_with("/dev/"))
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mmcblk_is_the_sd_card_and_the_rest_are_disks() {
        assert!(matches!(
            Medium::of_device("mmcblk0p2"),
            Medium::SdCard { .. }
        ));
        assert_eq!(
            Medium::of_device("nvme0n1p1"),
            Medium::Disk {
                device: "nvme0n1p1".into()
            }
        );
        assert!(matches!(Medium::of_device("sda1"), Medium::Disk { .. }));
    }

    #[test]
    fn device_numbers_split_as_the_kernel_packs_them() {
        // glibc's makedev, the other direction.
        fn makedev(major: u64, minor: u64) -> u64 {
            (minor & 0xff)
                | ((major & 0xfff) << 8)
                | ((minor & !0xff) << 12)
                | ((major & !0xfff) << 32)
        }
        // mmcblk0p2 is 179:2 on a Pi; nvme0n1p1 is 259:1.
        assert_eq!(major_minor(makedev(179, 2)), (179, 2));
        assert_eq!(major_minor(makedev(259, 1)), (259, 1));
        // Large numbers spill into the high bits.
        assert_eq!(major_minor(makedev(8, 0x12345)), (8, 0x12345));
        assert_eq!(major_minor(makedev(0x1234, 7)), (0x1234, 7));
    }

    #[test]
    fn the_deepest_mount_wins() {
        let mountinfo = "\
22 1 179:2 / / rw,noatime - ext4 /dev/mmcblk0p2 rw
23 22 0:5 / /dev rw - devtmpfs udev rw
31 22 259:1 / /srv/tealteam rw,noatime - ext4 /dev/nvme0n1p1 rw
32 22 0:33 / /mnt/my\\040disk rw - btrfs /dev/sda1 rw
33 22 0:34 / /run rw - tmpfs tmpfs rw";
        let on = |p: &str| mount_source(mountinfo, Path::new(p));
        assert_eq!(
            on("/srv/tealteam/data/tealteam.db").as_deref(),
            Some("/dev/nvme0n1p1")
        );
        assert_eq!(
            on("/home/pi/tealteam.db").as_deref(),
            Some("/dev/mmcblk0p2")
        );
        assert_eq!(on("/mnt/my disk/t.db").as_deref(), Some("/dev/sda1"));
        // Not a partition: nothing to name.
        assert_eq!(on("/run/t.db"), None);
        // A sibling that only shares a prefix is not under the mount.
        assert_eq!(on("/srv/tealteam2/t.db").as_deref(), Some("/dev/mmcblk0p2"));
    }

    #[test]
    fn memory_is_nowhere() {
        let at = locate("sqlite::memory:").unwrap();
        assert_eq!(at.medium, Medium::Memory);
        assert!(at.path.is_none());
        assert!(!at.on_sd_card());
    }

    #[test]
    fn a_relative_path_is_reported_absolute_even_before_it_exists() {
        let at = locate("sqlite://./not-yet/tealteam.db").unwrap();
        let path = at.path.clone().unwrap();
        assert!(path.is_absolute(), "{path:?}");
        assert!(path.ends_with("not-yet/tealteam.db"));
        assert!(!path.display().to_string().contains("/./"), "{path:?}");
        assert!(at.describe().starts_with(&path.display().to_string()));
    }

    #[test]
    fn describe_names_the_sd_card() {
        let at = Location {
            path: Some("/home/pi/tealteam.db".into()),
            medium: Medium::of_device("mmcblk0p2"),
        };
        assert!(at.on_sd_card());
        assert_eq!(
            at.describe(),
            "/home/pi/tealteam.db on the SD card (mmcblk0p2)"
        );
    }
}
