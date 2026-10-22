/// Handles loading and saving of authentication cookies.
pub mod cookies;

/// Provides utility functions for atomic file system operations.
pub mod fs_util;

/// Handles the API keys of the user.
pub mod keys;
/// Handles loading and saving of application metadata.
pub mod metadata;
/// Handles the optional q.conf settings file.
pub mod settings;

pub use cookies::{load_cookies, parse_cookies, remove_cookies, save_cookies, CookieSet};
pub use keys::{load_keys, mask_token, parse_keys, remove_keys, save_keys, ApiKey, KeyStore};
pub use metadata::{load_metadata, save_metadata, Metadata};
pub use settings::{config_path, load_settings, parse_settings, Settings};

use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;

/// Returns the path to the application's configuration directory, creating it if it does not exist.
///
/// The `Q_CONFIG_DIR` environment variable overrides the platform location. It
/// points at the directory that holds the cookies, the metadata, the settings
/// file, and the Python virtual environment.
///
/// # Errors
///
/// Returns an error if:
/// - The system configuration directory cannot be determined.
/// - The configuration directory fails to be created on the filesystem.
pub fn config_dir() -> Result<PathBuf> {
    let dir = resolve_config_dir(std::env::var_os("Q_CONFIG_DIR").map(PathBuf::from))?;
    let display_dir = dir.display();
    fs::create_dir_all(&dir)
        .with_context(|| format!("Failed to create config directory: {display_dir}"))?;
    Ok(dir)
}

/// Returns the configuration directory without creating it.
///
/// The override wins over the platform location. The returned path may not
/// exist yet.
fn resolve_config_dir(override_dir: Option<PathBuf>) -> Result<PathBuf> {
    override_dir.map_or_else(
        || {
            dirs::config_dir()
                .map(|base| base.join("q"))
                .ok_or_else(|| anyhow::anyhow!("Failed to determine config directory"))
        },
        Ok,
    )
}

/// Returns the full path to the `cookies.json` file within the configuration directory.
///
/// # Errors
///
/// Returns an error if the configuration directory cannot be determined or created.
pub fn cookies_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("cookies.json"))
}

/// Returns the full path to the `keys.json` file within the configuration directory.
///
/// # Errors
///
/// Returns an error if the configuration directory cannot be determined or created.
pub fn keys_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("keys.json"))
}

/// Returns the full path to the `metadata.json` file within the configuration directory.
///
/// # Errors
///
/// Returns an error if the configuration directory cannot be determined or created.
pub fn metadata_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("metadata.json"))
}

#[cfg(test)]
#[allow(clippy::result_large_err)]
mod tests {
    use super::{config_dir, cookies_path, metadata_path, resolve_config_dir};
    use crate::test_support::{EnvGuard, TempDir};
    use std::path::PathBuf;

    #[test]
    fn the_override_wins_over_the_platform_location() -> anyhow::Result<()> {
        let dir = PathBuf::from("/tmp/q-override");

        let resolved = resolve_config_dir(Some(dir.clone()))?;

        assert_eq!(resolved, dir);
        Ok(())
    }

    #[test]
    fn the_platform_location_ends_with_the_app_name() -> anyhow::Result<()> {
        let resolved = resolve_config_dir(None)?;

        assert!(resolved.ends_with("q"), "{resolved:?}");
        Ok(())
    }

    #[test]
    fn the_paths_live_in_the_config_directory() -> anyhow::Result<()> {
        let _guard = EnvGuard::new();
        let dir = TempDir::new("config")?;
        std::env::set_var("Q_CONFIG_DIR", dir.path());

        let result = (|| -> anyhow::Result<()> {
            let config = config_dir()?;
            assert_eq!(cookies_path()?, config.join("cookies.json"));
            assert_eq!(metadata_path()?, config.join("metadata.json"));
            Ok(())
        })();
        std::env::remove_var("Q_CONFIG_DIR");
        result
    }

    #[test]
    fn the_config_directory_is_created() -> anyhow::Result<()> {
        let _guard = EnvGuard::new();
        let dir = TempDir::new("config")?;
        let path = dir.path().join("nested");
        std::env::set_var("Q_CONFIG_DIR", &path);

        let result = (|| -> anyhow::Result<()> {
            let config = config_dir()?;
            assert!(config.is_dir(), "{config:?} is not a directory");
            Ok(())
        })();
        std::env::remove_var("Q_CONFIG_DIR");
        result
    }
}
