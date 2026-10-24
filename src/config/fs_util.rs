use crate::Result;
use anyhow::Context;
use std::io::Write;
use std::path::Path;

/// Writes data to a file atomically by first writing to a temporary file with
/// an unpredictable name and then renaming it to the target path.
///
/// The temporary file is created in the same directory as the target, with a
/// random name, so a predictable path cannot be pre-created (symlink attack)
/// in a world-writable directory.
///
/// # Errors
///
/// Returns an error if:
/// - The temporary file cannot be created
/// - Writing to the file fails
/// - Syncing to disk fails
/// - The atomic rename fails
pub fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));

    // NamedTempFile creates the file with O_CREAT | O_EXCL and a random name,
    // and removes it on drop if the rename never happens.
    let mut temp = tempfile::NamedTempFile::new_in(dir)
        .with_context(|| format!("Failed to create temp file in {}", dir.display()))?;

    temp.write_all(data)
        .with_context(|| format!("Failed to write to temp file: {}", temp.path().display()))?;

    temp.as_file()
        .sync_all()
        .context("Failed to sync temp file to disk")?;

    // Atomic rename. On POSIX this is guaranteed to be atomic.
    temp.persist(path)
        .map_err(|e| anyhow::anyhow!("Failed to rename temp file to {}: {}", path.display(), e))?;

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

        // NamedTempFile uses a random name; after a successful persist the
        // temp file is gone, so the directory holds only the target.
        let entries: Vec<_> = std::fs::read_dir(dir.path())?
            .filter_map(std::result::Result::ok)
            .collect();
        assert_eq!(entries.len(), 1, "unexpected entries: {entries:?}");
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
