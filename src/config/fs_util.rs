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

    // The guard removes the temp file if any step fails. Without it, a leftover
    // file from a crashed process makes `create_new(true)` fail.
    let guard = CleanupGuard(&temp_path);

    let mut options = OpenOptions::new();
    options.write(true).create_new(true);

    // Set permissions atomically on Unix to prevent a TOCTOU window. The file is
    // created with 0600, so it is never briefly 0644.
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

#[cfg(test)]
#[allow(clippy::result_large_err)]
mod tests {
    use super::write_atomic;
    use crate::test_support::TempDir;

    #[test]
    fn writes_a_file() -> anyhow::Result<()> {
        let dir = TempDir::new("atomic")?;
        let path = dir.path().join("data.json");

        write_atomic(&path, b"content")?;

        assert_eq!(std::fs::read_to_string(&path)?, "content");
        Ok(())
    }

    #[test]
    fn replaces_an_existing_file() -> anyhow::Result<()> {
        let dir = TempDir::new("atomic")?;
        let path = dir.path().join("data.json");
        write_atomic(&path, b"old")?;

        write_atomic(&path, b"new")?;

        assert_eq!(std::fs::read_to_string(&path)?, "new");
        Ok(())
    }

    #[test]
    fn leaves_no_temporary_file_behind() -> anyhow::Result<()> {
        let dir = TempDir::new("atomic")?;
        let path = dir.path().join("data.json");

        write_atomic(&path, b"content")?;

        let leftovers: Vec<_> = std::fs::read_dir(dir.path())?
            .filter_map(std::result::Result::ok)
            .filter(|entry| entry.path().extension().is_some_and(|e| e == "tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temporary files remain");
        Ok(())
    }

    #[test]
    fn fails_when_the_directory_is_missing() -> anyhow::Result<()> {
        let dir = TempDir::new("atomic")?;
        let path = dir.path().join("missing").join("data.json");

        assert!(write_atomic(&path, b"content").is_err());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn keeps_the_file_private() -> anyhow::Result<()> {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new("atomic")?;
        let path = dir.path().join("data.json");

        write_atomic(&path, b"content")?;

        let mode = std::fs::metadata(&path)?.permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "mode: {mode:o}");
        Ok(())
    }
}
