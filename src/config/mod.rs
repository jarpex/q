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
use std::path::Path;
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
use std::io;

/// Creates a directory with mode 0700 on Unix, so it is never world-readable
/// even for the instant between creation and a permission change.
fn create_dir_secure(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new().mode(0o700).create(path)
    }
    #[cfg(not(unix))]
    {
        fs::create_dir(path)
    }
}

/// Encrypts sensitive data using the Windows Data Protection API (DPAPI).
///
/// # Errors
///
/// Returns an error if the data exceeds 4 GB or `CryptProtectData` fails.
#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
pub fn protect_secret(data: &[u8]) -> Result<Vec<u8>> {
    use std::ptr::null_mut;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{CryptProtectData, CRYPT_INTEGER_BLOB};

    let Ok(len) = u32::try_from(data.len()) else {
        anyhow::bail!("secret data too large");
    };

    let in_blob = CRYPT_INTEGER_BLOB {
        cbData: len,
        pbData: data.as_ptr().cast_mut(),
    };
    let mut out_blob = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: null_mut(),
    };

    let success = unsafe {
        CryptProtectData(
            &raw const in_blob,
            null_mut(),
            null_mut(),
            null_mut(),
            null_mut(),
            0,
            &raw mut out_blob,
        )
    };
    if success == 0 {
        anyhow::bail!("CryptProtectData failed");
    }
    let res =
        unsafe { std::slice::from_raw_parts(out_blob.pbData, out_blob.cbData as usize).to_vec() };
    let _ = unsafe { LocalFree(out_blob.pbData.cast()) };
    Ok(res)
}

/// Decrypts sensitive data using the Windows Data Protection API (DPAPI).
///
/// # Errors
///
/// Returns an error if the data exceeds 4 GB or `CryptUnprotectData` fails.
#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
pub fn unprotect_secret(data: &[u8]) -> Result<Vec<u8>> {
    use std::ptr::null_mut;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{CryptUnprotectData, CRYPT_INTEGER_BLOB};

    let Ok(len) = u32::try_from(data.len()) else {
        anyhow::bail!("secret data too large");
    };

    let in_blob = CRYPT_INTEGER_BLOB {
        cbData: len,
        pbData: data.as_ptr().cast_mut(),
    };
    let mut out_blob = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: null_mut(),
    };

    let success = unsafe {
        CryptUnprotectData(
            &raw const in_blob,
            null_mut(),
            null_mut(),
            null_mut(),
            null_mut(),
            0,
            &raw mut out_blob,
        )
    };
    if success == 0 {
        anyhow::bail!("CryptUnprotectData failed");
    }
    let res =
        unsafe { std::slice::from_raw_parts(out_blob.pbData, out_blob.cbData as usize).to_vec() };
    let _ = unsafe { LocalFree(out_blob.pbData.cast()) };
    Ok(res)
}

#[cfg(windows)]
fn verify_not_reparse_point(path: &Path) -> Result<()> {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    let meta = fs::symlink_metadata(path)?;
    if (meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT) != 0 {
        anyhow::bail!(
            "Configuration directory is a junction or reparse point: {}",
            path.display()
        );
    }
    Ok(())
}

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

    // Create the parent directories (the system location, which is trusted).
    if let Some(parent) = dir.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create config directory: {display_dir}"))?;
        }
    }

    // Create the final component atomically. `create_dir` (mkdir) fails if
    // anything already exists at that path, including a symlink, so an
    // attacker cannot substitute a symlink between a check and the creation.
    // On Unix the directory is created with mode 0700 in a single mkdir
    // call, so there is no window where it is world-readable.
    match create_dir_secure(&dir) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            // The directory already exists: make sure it is a real directory
            // and not a symlink planted by an attacker.
            let metadata = fs::symlink_metadata(&dir)
                .with_context(|| format!("Failed to inspect config directory: {display_dir}"))?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(anyhow::anyhow!(
                    "configuration directory is not a directory: {display_dir}"
                ));
            }
            #[cfg(windows)]
            verify_not_reparse_point(&dir)?;
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("Failed to create config directory: {display_dir}"));
        }
    }

    // Make sure the directory is private (0700) so other users cannot plant
    // files or symlinks inside it.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let permissions = fs::Permissions::from_mode(0o700);
        fs::set_permissions(&dir, permissions)
            .with_context(|| format!("Failed to secure config directory: {display_dir}"))?;
    }

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

    #[cfg(target_os = "windows")]
    #[test]
    fn regular_directory_is_not_reparse_point() -> anyhow::Result<()> {
        let dir = TempDir::new("regular_dir")?;
        super::verify_not_reparse_point(dir.path())?;
        Ok(())
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn protects_and_unprotects_secrets_with_dpapi() -> anyhow::Result<()> {
        let secret = b"my-secret-token";
        let protected = super::protect_secret(secret)?;
        assert_ne!(protected, secret);
        let unprotected = super::unprotect_secret(&protected)?;
        assert_eq!(unprotected, secret);
        Ok(())
    }
}
