use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

use super::fs_util::write_atomic;

/// Stores metadata about the application's state, such as the last update time and version.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metadata {
    /// The timestamp of the last update (Unix epoch in seconds).
    pub last_update: u64,

    /// The semantic version string of the last update.
    pub last_version: String,
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
    let display_path = path.display();
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Cannot read metadata file: {display_path}"))?;

    let metadata: Metadata = serde_json::from_str(&content)
        .with_context(|| format!("Invalid JSON in metadata file: {display_path}"))?;

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
    let json = serde_json::to_string_pretty(metadata)?;
    write_atomic(path, json.as_bytes())
}
