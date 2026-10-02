/// Handles loading and saving of authentication cookies.
pub mod cookies;

/// Provides utility functions for atomic file system operations.
pub mod fs_util;

/// Handles loading and saving of application metadata.
pub mod metadata;

pub use cookies::{load_cookies, save_cookies, CookieSet};
pub use metadata::{load_metadata, save_metadata, Metadata};

use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;

/// Returns the path to the application's configuration directory, creating it if it does not exist.
///
/// # Errors
///
/// This function will return an error if:
/// - The system's configuration directory cannot be determined.
/// - The configuration directory fails to be created on the filesystem.
pub fn config_dir() -> Result<PathBuf> {
    let base = dirs::config_dir()
        .ok_or_else(|| anyhow::anyhow!("Failed to determine config directory"))?;
    let dir = base.join("q");
    let display_dir = dir.display();
    fs::create_dir_all(&dir)
        .with_context(|| format!("Failed to create config directory: {display_dir}"))?;
    Ok(dir)
}

/// Returns the full path to the `cookies.json` file within the configuration directory.
///
/// # Errors
///
/// This function will return an error if the base configuration directory cannot be determined or created.
pub fn cookies_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("cookies.json"))
}

/// Returns the full path to the `metadata.json` file within the configuration directory.
///
/// # Errors
///
/// This function will return an error if the base configuration directory cannot be determined or created.
pub fn metadata_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("metadata.json"))
}
