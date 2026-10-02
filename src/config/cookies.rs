use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

use super::fs_util::write_atomic;

/// Holds the authentication cookies required to interact with the Gemini API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CookieSet {
    /// The primary session ID cookie (e.g., `__Secure-1PSID`).
    pub psid: String,
    /// The session ID timestamp/token cookie (e.g., `__Secure-1PSIDTS`).
    pub psidts: String,
}

/// Loads cookies from the specified path.
///
/// # Errors
///
/// Returns an error if:
/// - The file cannot be read
/// - The JSON is invalid
/// - The cookies are empty or malformed
pub fn load_cookies<P: AsRef<Path>>(path_arg: P) -> Result<CookieSet> {
    let path = path_arg.as_ref();
    let display_path = path.display();
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Cannot read cookies file: {display_path}"))?;

    let cookies: CookieSet = serde_json::from_str(&content)
        .with_context(|| format!("Invalid JSON in cookies file: {display_path}"))?;

    if cookies.psid.is_empty() || cookies.psidts.is_empty() {
        anyhow::bail!("Stored cookies are invalid or empty");
    }

    Ok(cookies)
}

/// Saves cookies to the specified path atomically.
///
/// # Errors
///
/// Returns an error if:
/// - Serialization to JSON fails
/// - The temporary file cannot be created or written to
/// - Setting permissions fails (on Unix)
/// - The atomic rename fails
pub fn save_cookies<P: AsRef<Path>>(path_arg: P, cookies: &CookieSet) -> Result<()> {
    let path = path_arg.as_ref();
    let json = serde_json::to_string_pretty(cookies)?;
    write_atomic(path, json.as_bytes())
}
