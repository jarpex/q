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
    fn is_valid(&self) -> bool {
        !self.psid.is_empty() && !self.psidts.is_empty()
    }
}

// --- Serde integration for Zeroizing<String> ---.
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

    // SECURITY: Verify file permissions on Unix to prevent local session hijacking.
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

    let dto: CookieDto = serde_json::from_str(&content)
        .with_context(|| format!("Invalid JSON in cookies file: {}", path.display()))?;

    let cookies = CookieSet::from(dto);

    if !cookies.is_valid() {
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
    let dto = CookieDto::from(cookies);
    let json = serde_json::to_string(&dto)?;
    write_atomic(path, json.as_bytes())
}
