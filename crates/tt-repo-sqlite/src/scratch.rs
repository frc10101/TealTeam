//! A scratch file in the temp dir that no one else can have made or read
//! first: a snapshot's uncut copy (`VACUUM INTO` writes into an empty file
//! already there), or a pushed bundle before it is checked.
//!
//! The file sits alone in a directory made here, exclusively and readable by
//! this user only, under a name nobody can guess. A directory already there
//! is never used. Dropping it removes the lot, journal, `-wal`, and `-shm`
//! included.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub struct Scratch {
    dir: PathBuf,
    file: PathBuf,
}

impl Scratch {
    /// A new private directory holding `<prefix>.sqlite`, not yet written.
    pub fn new(prefix: &str) -> io::Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let mut last = None;
        for _ in 0..8 {
            // Seeded from the OS's randomness, new for each `RandomState`.
            let mut hasher = RandomState::new().build_hasher();
            hasher.write_u64(NEXT.fetch_add(1, Ordering::Relaxed));
            let dir = std::env::temp_dir().join(format!(
                "tt-{prefix}-{}-{:016x}",
                std::process::id(),
                hasher.finish()
            ));
            let mut builder = std::fs::DirBuilder::new();
            #[cfg(unix)]
            std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
            match builder.create(&dir) {
                Ok(()) => {
                    let file = dir.join(format!("{prefix}.sqlite"));
                    return Ok(Self { dir, file });
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => last = Some(e),
                Err(e) => return Err(e),
            }
        }
        Err(last.unwrap_or_else(|| io::Error::other("no scratch directory")))
    }

    pub fn path(&self) -> &Path {
        &self.file
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scratch_file_is_private_unguessable_and_cleaned_up_whole() {
        let one = Scratch::new("test").unwrap();
        let two = Scratch::new("test").unwrap();
        assert_ne!(one.dir, two.dir);
        let dir = one.path().parent().unwrap().to_path_buf();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700);
        }
        std::fs::write(one.path(), b"x").unwrap();
        std::fs::write(dir.join("test.sqlite-wal"), b"x").unwrap();
        drop(one);
        assert!(!dir.exists(), "the journal goes with it");
    }
}
