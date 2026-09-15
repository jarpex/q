use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CookieSet {
    pub psid: String,
    pub psidts: String,
}

pub(crate) fn config_dir() -> Result<PathBuf> {
    let base = dirs::config_dir()
        .ok_or_else(|| anyhow::anyhow!("Failed to determine config directory"))?;
    let dir = base.join("q");
    let display_dir = dir.display();
    fs::create_dir_all(&dir)
        .with_context(|| format!("Failed to create config directory: {display_dir}"))?;
    Ok(dir)
}

pub(crate) fn cookies_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("cookies.json"))
}

pub(crate) fn load_cookies<P: AsRef<Path>>(path_arg: P) -> Result<CookieSet> {
    let path = path_arg.as_ref();
    let display_path = path.display();
    let content = fs::read_to_string(path)
        .with_context(|| format!("Cannot read cookies file: {display_path}"))?;
    
    let cookies: CookieSet = serde_json::from_str(&content)
        .with_context(|| format!("Invalid JSON in cookies file: {display_path}"))?;
    
    if cookies.psid.is_empty() || cookies.psidts.is_empty() {
        anyhow::bail!("Stored cookies are invalid or empty");
    }
    
    Ok(cookies)
}

pub(crate) fn save_cookies<P: AsRef<Path>>(path_arg: P, cookies: &CookieSet) -> Result<()> {
    let path = path_arg.as_ref();
    let display_path = path.display();
    let json = serde_json::to_string_pretty(cookies)?;
    
    // Atomic write: write to temp file, then rename
    let temp_path = path.with_extension("json.tmp");
    let display_temp = temp_path.display();
    let mut file = fs::File::create(&temp_path)
        .with_context(|| format!("Failed to create temp file: {display_temp}"))?;
    
    file.write_all(json.as_bytes())
        .with_context(|| format!("Failed to write to temp file: {display_temp}"))?;
    
    // Sync to disk to ensure data is written
    file.sync_all()
        .with_context(|| "Failed to sync temp file to disk")?;
    
    drop(file);
    
    // Set permissions BEFORE rename for security
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temp_path, fs::Permissions::from_mode(0o600))
            .with_context(|| format!("Failed to set permissions on temp file: {display_temp}"))?;
    }
    
    // Atomic rename
    fs::rename(&temp_path, path)
        .with_context(|| format!("Failed to rename temp file to: {display_path}"))?;
    
    Ok(())
}