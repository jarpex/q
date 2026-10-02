use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

/// Writes data to a file atomically by first writing to a temporary file
/// and then renaming it to the target path.
///
/// # Errors
///
/// Returns an error if:
/// - The temporary file cannot be created
/// - Writing to the file fails
/// - Syncing to disk fails
/// - Setting permissions fails (on Unix)
/// - The atomic rename fails
pub fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    let temp_path = path.with_extension("json.tmp");
    let display_temp = temp_path.display();
    let mut file = fs::File::create(&temp_path)
        .with_context(|| format!("Failed to create temp file: {display_temp}"))?;

    std::io::Write::write_all(&mut file, data)
        .with_context(|| format!("Failed to write to temp file: {display_temp}"))?;

    file.sync_all()
        .with_context(|| "Failed to sync temp file to disk")?;

    drop(file);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temp_path, fs::Permissions::from_mode(0o600))
            .with_context(|| format!("Failed to set permissions on temp file: {display_temp}"))?;
    }

    let display_path = path.display();
    fs::rename(&temp_path, path)
        .with_context(|| format!("Failed to rename temp file to: {display_path}"))?;

    Ok(())
}
