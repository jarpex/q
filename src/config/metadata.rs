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
/// Returns an error if:
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
/// Returns an error if:
/// - The metadata struct fails to serialize into JSON.
/// - The file cannot be written to the specified path.
pub fn save_metadata<P: AsRef<Path>>(path_arg: P, metadata: &Metadata) -> Result<()> {
    let path = path_arg.as_ref();
    let json = serde_json::to_string(metadata)?;
    write_atomic(path, json.as_bytes())
}

#[cfg(test)]
#[allow(clippy::result_large_err)]
mod tests {
    use super::{load_metadata, save_metadata, Metadata};
    use crate::test_support::TempDir;
    use std::time::Duration;

    #[test]
    fn saves_and_loads_the_same_metadata() -> anyhow::Result<()> {
        let dir = TempDir::new("metadata")?;
        let path = dir.path().join("metadata.json");
        let metadata = Metadata {
            last_update: 1234,
            last_version: "0.1.0".to_owned(),
        };

        save_metadata(&path, &metadata)?;
        let loaded = load_metadata(&path)?;

        assert_eq!(loaded.last_update, 1234);
        assert_eq!(loaded.last_version, "0.1.0");
        Ok(())
    }

    #[test]
    fn fails_on_a_missing_file() {
        assert!(load_metadata("/nonexistent/q/metadata.json").is_err());
    }

    #[test]
    fn fails_on_broken_json() -> anyhow::Result<()> {
        let dir = TempDir::new("metadata")?;
        let path = dir.path().join("metadata.json");
        std::fs::write(&path, b"not json")?;

        assert!(load_metadata(&path).is_err());
        Ok(())
    }

    #[test]
    fn refuses_an_unknown_field() -> anyhow::Result<()> {
        let dir = TempDir::new("metadata")?;
        let path = dir.path().join("metadata.json");
        std::fs::write(&path, br#"{"last_update":1,"unknown":true}"#)?;

        assert!(load_metadata(&path).is_err());
        Ok(())
    }

    #[test]
    fn a_fresh_timestamp_is_not_stale() {
        let metadata = Metadata {
            last_update: 1000,
            last_version: "0.1.0".to_owned(),
        };

        assert!(!metadata.is_stale(1000, Duration::from_secs(3600)));
        assert!(!metadata.is_stale(1000 + 3599, Duration::from_secs(3600)));
    }

    #[test]
    fn an_old_timestamp_is_stale() {
        let metadata = Metadata {
            last_update: 1000,
            last_version: "0.1.0".to_owned(),
        };

        assert!(metadata.is_stale(1000 + 3600, Duration::from_secs(3600)));
        assert!(metadata.is_stale(1000 + 7200, Duration::from_secs(3600)));
    }

    #[test]
    fn a_timestamp_in_the_future_is_not_stale() {
        let metadata = Metadata {
            last_update: 5000,
            last_version: "0.1.0".to_owned(),
        };

        assert!(!metadata.is_stale(1000, Duration::from_secs(3600)));
    }
}
