//! The single-instance lock: one host, so one broker, per data directory.

use std::fmt;
use std::fs::{File, OpenOptions, TryLockError};
use std::path::Path;

/// The name of the lock file in the directory given to [`InstanceLock::acquire`].
const LOCK_FILE: &str = "instance.lock";

/// An exclusive lock on `instance.lock`. It is released when the value is dropped or the
/// process ends, however it ends.
#[derive(Debug)]
pub struct InstanceLock {
    // Held only for its lock; closing the file releases it.
    _file: File,
}

/// Why the instance lock was not taken.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstanceError {
    /// Another holder, in this process or another, has the lock.
    Held,
    /// The directory or the lock file could not be made or opened, or locking failed.
    Io(String),
}

impl fmt::Display for InstanceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InstanceError::Held => write!(f, "another instance holds the instance lock"),
            InstanceError::Io(detail) => write!(f, "the instance lock failed: {detail}"),
        }
    }
}

impl std::error::Error for InstanceError {}

impl InstanceLock {
    /// Creates `directory` if needed (mode 0700 on Unix), opens `instance.lock` in it and takes
    /// an exclusive lock without waiting.
    pub fn acquire(directory: &Path) -> Result<InstanceLock, InstanceError> {
        create_directory(directory)
            .map_err(|e| InstanceError::Io(format!("creating {}: {e}", directory.display())))?;
        let path = directory.join(LOCK_FILE);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|e| InstanceError::Io(format!("opening {}: {e}", path.display())))?;
        match file.try_lock() {
            Ok(()) => Ok(InstanceLock { _file: file }),
            Err(TryLockError::WouldBlock) => Err(InstanceError::Held),
            Err(TryLockError::Error(e)) => Err(InstanceError::Io(format!(
                "locking {}: {e}",
                path.display()
            ))),
        }
    }
}

#[cfg(unix)]
fn create_directory(directory: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    // Only directories made here get mode 0700; an existing directory is left as it is.
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(directory)
}

#[cfg(not(unix))]
fn create_directory(directory: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(directory)
}
