use anyhow::{Context, Result};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::path::Path;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use super::fs_util::write_atomic;

/// Holds the authentication cookies required to interact with the Gemini API.
/// Secrets are cryptographically zeroed from memory on drop to prevent
/// leakage via core dumps, swap, or hibernation.
#[derive(Debug, Clone, Zeroize, ZeroizeOnDrop)]
pub struct CookieSet {
    /// The primary session ID cookie (e.g., `__Secure-1PSID`).
    pub psid: Zeroizing<String>,
    /// The session ID timestamp/token cookie (e.g., `__Secure-1PSIDTS`).
    pub psidts: Zeroizing<String>,
}

impl CookieSet {
    /// Validates that neither cookie is empty.
    #[inline]
    #[allow(clippy::missing_const_for_fn)]
    fn is_valid(&self) -> bool {
        !self.psid.is_empty() && !self.psidts.is_empty()
    }
}

#[allow(clippy::type_complexity)]
fn serialize_zeroizing<S: Serializer>(s: &Zeroizing<String>, ser: S) -> Result<S::Ok, S::Error> {
    ser.serialize_str(s.as_str())
}

struct ZeroizingVisitor;

impl serde::de::Visitor<'_> for ZeroizingVisitor {
    type Value = Zeroizing<String>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a secure string")
    }

    fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
        Ok(Zeroizing::new(v.to_owned()))
    }

    fn visit_string<E: serde::de::Error>(self, v: String) -> Result<Self::Value, E> {
        // Zeroizing::new takes ownership and will zeroize the buffer on drop
        Ok(Zeroizing::new(v))
    }
}

#[allow(clippy::type_complexity)]
fn deserialize_zeroizing<'de, D: Deserializer<'de>>(de: D) -> Result<Zeroizing<String>, D::Error> {
    de.deserialize_string(ZeroizingVisitor)
}

/// Wrapper struct for JSON (de)serialization.
/// Using an explicit DTO prevents accidental logging of raw secrets by serde
/// and gives us full control over memory allocation.
#[derive(Serialize, Deserialize)]
struct CookieDto {
    #[serde(
        serialize_with = "serialize_zeroizing",
        deserialize_with = "deserialize_zeroizing"
    )]
    psid: Zeroizing<String>,
    #[serde(
        serialize_with = "serialize_zeroizing",
        deserialize_with = "deserialize_zeroizing"
    )]
    psidts: Zeroizing<String>,
}

impl From<CookieDto> for CookieSet {
    fn from(dto: CookieDto) -> Self {
        Self {
            psid: dto.psid,
            psidts: dto.psidts,
        }
    }
}

impl From<&CookieSet> for CookieDto {
    fn from(set: &CookieSet) -> Self {
        Self {
            psid: set.psid.clone(),
            psidts: set.psidts.clone(),
        }
    }
}

/// Parses cookies from JSON text.
///
/// # Errors
///
/// Returns an error if the JSON is invalid or the cookies are empty.
pub fn parse_cookies(content: &str) -> Result<CookieSet> {
    let dto: CookieDto = serde_json::from_str(content).context("Invalid JSON in cookies")?;
    let cookies = CookieSet::from(dto);

    if !cookies.is_valid() {
        anyhow::bail!("Stored cookies are invalid or empty");
    }

    Ok(cookies)
}

/// Loads cookies from the specified path.
///
/// # Errors
///
/// Returns an error if:
/// - The file cannot be read
/// - The file permissions are insecure (readable by others on Unix)
/// - The JSON is invalid
/// - The cookies are empty or malformed
pub fn load_cookies<P: AsRef<Path>>(path_arg: P) -> Result<CookieSet> {
    let path = path_arg.as_ref();

    // Verify file permissions on Unix to prevent local session hijacking.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let metadata = std::fs::metadata(path)
            .with_context(|| format!("Cannot read metadata for {}", path.display()))?;
        let mode = metadata.permissions().mode();
        if mode & 0o077 != 0 {
            anyhow::bail!(
                "Insecure file permissions: {} is readable by others (mode {:o}). \
                 Session cookies must be private (0600).",
                path.display(),
                mode & 0o777
            );
        }
    }

    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Cannot read cookies file: {}", path.display()))?;

    parse_cookies(&content)
        .with_context(|| format!("Invalid JSON in cookies file: {}", path.display()))
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
    let dto = CookieDto::from(cookies);
    let json = serde_json::to_string(&dto)?;
    write_atomic(path, json.as_bytes())
}

/// Removes the saved cookies file.
///
/// # Errors
///
/// Returns an error if the file cannot be removed for another reason.
pub fn remove_cookies<P: AsRef<Path>>(path_arg: P) -> Result<()> {
    let path = path_arg.as_ref();

    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => {
            Err(error).with_context(|| format!("Failed to remove cookies file: {}", path.display()))
        }
    }
}

#[cfg(test)]
#[allow(clippy::result_large_err)]
mod tests {
    use super::{load_cookies, parse_cookies, remove_cookies, save_cookies, CookieSet};
    use crate::test_support::TempDir;
    use std::path::PathBuf;
    use zeroize::Zeroizing;

    fn cookies() -> CookieSet {
        CookieSet {
            psid: Zeroizing::new("psid-value".to_owned()),
            psidts: Zeroizing::new("psidts-value".to_owned()),
        }
    }

    fn test_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("q-{name}-{}.json", std::process::id()))
    }

    #[test]
    fn remove_cookies_succeeds_when_file_is_missing() {
        let path = test_path("missing-cookies");
        let _ = std::fs::remove_file(&path);

        assert!(remove_cookies(&path).is_ok());
    }

    #[test]
    fn remove_cookies_deletes_existing_file() {
        let path = test_path("existing-cookies");
        assert!(std::fs::write(&path, b"test cookies").is_ok());

        assert!(remove_cookies(&path).is_ok());

        assert!(!path.exists());
    }

    #[test]
    fn parses_a_valid_pair() {
        let result = parse_cookies("{\"psid\":\"a\",\"psidts\":\"b\"}");

        assert!(result.is_ok());
        let Ok(parsed) = result else { return };
        assert_eq!(parsed.psid.as_str(), "a");
        assert_eq!(parsed.psidts.as_str(), "b");
    }

    #[test]
    fn rejects_an_empty_cookie() {
        assert!(parse_cookies("{\"psid\":\"\",\"psidts\":\"b\"}").is_err());
        assert!(parse_cookies("{\"psid\":\"a\",\"psidts\":\"\"}").is_err());
    }

    #[test]
    fn rejects_broken_json() {
        assert!(parse_cookies("not json").is_err());
        assert!(parse_cookies("{}").is_err());
    }

    #[test]
    fn saves_and_loads_the_same_cookies() -> anyhow::Result<()> {
        let dir = TempDir::new("cookies")?;
        let path = dir.path().join("cookies.json");

        save_cookies(&path, &cookies())?;
        let loaded = load_cookies(&path)?;

        assert_eq!(loaded.psid.as_str(), "psid-value");
        assert_eq!(loaded.psidts.as_str(), "psidts-value");
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn refuses_a_file_that_others_can_read() -> anyhow::Result<()> {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new("cookies")?;
        let path = dir.path().join("cookies.json");
        save_cookies(&path, &cookies())?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))?;

        let loaded = load_cookies(&path);
        assert!(loaded.is_err(), "readable cookies must be refused");
        Ok(())
    }
}
