use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Duration;

use super::fs_util::write_atomic;

/// Stores metadata about the application's state, such as the last update time and version.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Metadata {
    /// The timestamp of the last update (Unix epoch in seconds).
    pub last_update: u64,

    /// The semantic version string of the last update.
    pub last_version: String,
}

impl Metadata {
    /// Checks if the metadata is older than the specified duration relative to `current_time`.
    #[inline]
    pub const fn is_stale(&self, current_time: u64, max_age: Duration) -> bool {
        current_time.saturating_sub(self.last_update) >= max_age.as_secs()
    }
}

/// Loads metadata from the specified file path.
///
/// # Errors
///
/// This function will return an error if:
/// - The metadata file cannot be read from the specified path.
/// - The file contents are not valid JSON or cannot be deserialized into `Metadata`.
pub fn load_metadata<P: AsRef<Path>>(path_arg: P) -> Result<Metadata> {
    let path = path_arg.as_ref();
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Cannot read metadata file: {}", path.display()))?;

    let metadata: Metadata = serde_json::from_str(&content)
        .with_context(|| format!("Invalid JSON in metadata file: {}", path.display()))?;

    Ok(metadata)
}

/// Saves the given metadata to the specified file path.
///
/// # Errors
///
/// This function will return an error if:
/// - The metadata struct fails to serialize into JSON.
/// - The file cannot be written to the specified path.
pub fn save_metadata<P: AsRef<Path>>(path_arg: P, metadata: &Metadata) -> Result<()> {
    let path = path_arg.as_ref();
    let json = serde_json::to_string(metadata)?;
    write_atomic(path, json.as_bytes())
}
