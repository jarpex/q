//! Helpers shared by the unit tests.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// A lock that keeps the tests that change the environment apart.
///
/// The environment is shared by every thread of the process, so the tests that
/// set a variable must not run at the same time as the tests that read it.
pub(crate) struct EnvGuard {
    _lock: MutexGuard<'static, ()>,
}

impl EnvGuard {
    /// Takes the lock. It waits until the other tests are done.
    pub(crate) fn new() -> Self {
        let lock = env_lock().lock();
        Self {
            _lock: lock.unwrap_or_else(std::sync::PoisonError::into_inner),
        }
    }
}

/// A directory under the system temporary directory that is deleted on drop.
pub(crate) struct TempDir {
    path: PathBuf,
}

impl TempDir {
    /// Creates a directory that no other test uses.
    pub(crate) fn new(tag: &str) -> Result<Self, std::io::Error> {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("q-test-{tag}-{}-{id}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path)?;
        Ok(Self { path })
    }

    /// Returns the path of the directory.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.path());
    }
}
