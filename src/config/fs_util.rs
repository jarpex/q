use anyhow::{Context, Result};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

/// RAII guard to ensure temporary files are cleaned up if an operation fails.
/// If the operation succeeds, `std::mem::forget` must be called on this guard
/// to prevent it from deleting the successfully renamed file.
struct CleanupGuard<'a>(&'a Path);

impl Drop for CleanupGuard<'_> {
    fn drop(&mut self) {
        // Best-effort cleanup. Ignore errors (e.g., if rename succeeded and file is gone).
        let _ = fs::remove_file(self.0);
    }
}

/// Writes data to a file atomically by first writing to a temporary file
/// and then renaming it to the target path.
///
/// # Errors
///
/// Returns an error if:
/// - The temporary file cannot be created (e.g., due to a leftover temp file or symlink attack)
/// - Writing to the file fails
/// - Syncing to disk fails
/// - The atomic rename fails
pub fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    // Use generic `.tmp` instead of `.json.tmp` to avoid hardcoding extensions.
    let temp_path = path.with_extension("tmp");

    // SECURITY & RELIABILITY: RAII Guard to clean up the temporary file if any step fails.
    // This prevents `create_new(true)` from failing on subsequent runs due to a leftover
    // temp file from a previously crashed process.
    let guard = CleanupGuard(&temp_path);

    let mut options = OpenOptions::new();
    options.write(true).create_new(true);

    // SECURITY: Set permissions atomically on Unix to prevent TOCTOU window.
    // The file is created with 0600 immediately, avoiding the brief 0644 exposure.
    #[cfg(unix)]
    options.mode(0o600);

    let mut file = options.open(&temp_path).with_context(|| {
        format!(
            "Failed to create temp file (symlink attack or leftover file prevented): {}",
            temp_path.display()
        )
    })?;

    file.write_all(data)
        .with_context(|| format!("Failed to write to temp file: {}", temp_path.display()))?;

    file.sync_all()
        .context("Failed to sync temp file to disk")?;

    drop(file);

    // Atomic rename. On POSIX, this is guaranteed to be atomic and will overwrite
    // the target file if it exists, without breaking hardlinks to the target.
    fs::rename(&temp_path, path)
        .with_context(|| format!("Failed to rename temp file to: {}", path.display()))?;

    // Rename succeeded, prevent the guard from deleting the final file.
    std::mem::forget(guard);

    Ok(())
}
