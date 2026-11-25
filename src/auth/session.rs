//! Reads the Gemini session cookies out of a Firefox profile.
//!
//! Firefox keeps the persistent cookies of a profile in `cookies.sqlite` and
//! the cookies that carry no expiry date in its session store. A Gemini login
//! can put the session cookies in either place, so both are read and merged.
//!
//! Google now refuses a session that carries only `__Secure-1PSID` and
//! `__Secure-1PSIDTS`, so every google.com cookie of the profile is read, each
//! with its host, not just the two session IDs.
//!
//! The session store is a JSON document behind a small LZ4 header. Firefox
//! writes it under `sessionstore-backups` while the browser runs and under the
//! profile root when it shuts down, so a login can read the cookies of a
//! running browser and the cookies of a closed one.

use crate::{Error, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

use crate::auth::python_bin;
use crate::config::{Cookie, CookieSet};

/// Cookie holding the Gemini session identifier.
pub const PSID: &str = "__Secure-1PSID";
/// Cookie holding the Gemini session timestamp.
pub const PSIDTS: &str = "__Secure-1PSIDTS";

/// Magic bytes in front of the compressed session store.
const STORE_MAGIC: &[u8] = b"mozLz40\0";

/// Bytes in front of the payload: the magic and the size of the plain text.
const HEADER_LEN: usize = 12;

/// Largest session store that is read, so a damaged size header cannot make
/// the process ask for an arbitrary amount of memory.
const MAX_STORE_BYTES: usize = 64 * 1024 * 1024;

/// A cookie as the Firefox session store keeps it.
#[derive(Clone, Debug, Deserialize)]
pub struct StoreCookie {
    /// Host the cookie was set for.
    pub host: String,
    /// Name of the cookie.
    pub name: String,
    /// Value of the cookie.
    pub value: zeroize::Zeroizing<String>,
}

/// The part of the session store the cookie read needs.
#[derive(Debug, Default, Deserialize)]
struct SessionStore {
    /// Cookies the store holds.
    #[serde(default)]
    cookies: Vec<StoreCookie>,
}

/// One cookie of a profile: its name, value and host.
#[derive(Clone, Debug)]
struct ProfileCookie {
    name: String,
    value: zeroize::Zeroizing<String>,
    host: String,
}

/// A cookie of a profile, keyed by its name and host.
type CookieKey = (String, String);

/// The Gemini session cookies found in a Firefox profile.
#[derive(Clone, Debug, Default, zeroize::Zeroize, zeroize::ZeroizeOnDrop)]
pub struct ProfileSession {
    /// Value of the session identifier, if the profile holds it.
    pub psid: Option<zeroize::Zeroizing<String>>,
    /// Value of the session timestamp, if the profile holds it.
    pub psidts: Option<zeroize::Zeroizing<String>>,
    /// The rest of the Google cookies of the profile, each with its host.
    pub extra: Vec<Cookie>,
}

impl ProfileSession {
    /// Reports whether both session cookies are present and filled in.
    pub fn is_complete(&self) -> bool {
        let filled = |value: &Option<zeroize::Zeroizing<String>>| {
            value.as_ref().is_some_and(|text| !text.is_empty())
        };
        filled(&self.psid) && filled(&self.psidts)
    }

    /// Builds the cookie set of the Gemini API.
    ///
    /// # Errors
    ///
    /// Returns an error if one of the two session cookies is missing.
    pub fn to_cookie_set(&self) -> Result<CookieSet> {
        if !self.is_complete() {
            return Err(Error::Auth(
                "the Firefox profile does not hold a complete Gemini session".into(),
            ));
        }

        let psid = self.psid.clone().unwrap_or_default();
        let psidts = self.psidts.clone().unwrap_or_default();

        Ok(CookieSet {
            psid,
            psidts,
            extra: self.extra.clone(),
        })
    }
}

/// Reads the Gemini session cookies out of a Firefox profile.
///
/// The persistent cookies come from the cookie database and the session
/// cookies from the session store. The database wins over the store, because
/// it is written on every change, and the store of a closed browser wins over
/// the backup of a running one, because it is written last.
///
/// # Errors
///
/// Returns an error if the cookie database cannot be copied or read. A profile
/// that holds no cookie database yet is not an error: it simply has no cookies.
pub fn read_profile_session(profile: &Path) -> Result<ProfileSession> {
    let database = read_database_cookies(profile)?;
    let store = read_store_cookies(profile)?;

    // The database wins over the store: it is written on every change.
    let mut merged: HashMap<CookieKey, ProfileCookie> = HashMap::new();
    for cookie in store {
        merged.insert((cookie.name.clone(), cookie.host.clone()), cookie);
    }
    for cookie in database {
        merged.insert((cookie.name.clone(), cookie.host.clone()), cookie);
    }

    let mut psid = None;
    let mut psidts = None;
    let mut extra = Vec::new();

    for cookie in merged.into_values() {
        match cookie.name.as_str() {
            PSID => psid = Some(cookie.value),
            PSIDTS => psidts = Some(cookie.value),
            _ => extra.push(Cookie {
                name: zeroize::Zeroizing::new(cookie.name),
                value: cookie.value,
                domain: cookie.host,
            }),
        }
    }

    Ok(ProfileSession {
        psid,
        psidts,
        extra,
    })
}

/// Merges the session that was read before the close with the one after it.
///
/// The browser writes the final session store while it shuts down, so the
/// values that are read afterwards are the freshest ones. A value that only
/// the earlier read holds is kept, because the final store does not carry
/// every cookie of the profile.
pub fn merge(before: &ProfileSession, after: Option<&ProfileSession>) -> ProfileSession {
    let Some(fresh) = after else {
        return before.clone();
    };

    // Fresh cookies win by (name, domain); a cookie only the earlier read
    // holds is kept.
    let mut merged: HashMap<CookieKey, Cookie> = HashMap::new();
    for cookie in &before.extra {
        merged.insert(
            (cookie.name.to_string(), cookie.domain.clone()),
            cookie.clone(),
        );
    }
    for cookie in &fresh.extra {
        merged.insert(
            (cookie.name.to_string(), cookie.domain.clone()),
            cookie.clone(),
        );
    }

    ProfileSession {
        psid: prefer(
            fresh.psid.as_deref().map(String::as_str),
            before.psid.as_deref().map(String::as_str),
        ),
        psidts: prefer(
            fresh.psidts.as_deref().map(String::as_str),
            before.psidts.as_deref().map(String::as_str),
        ),
        extra: merged.into_values().collect(),
    }
}

/// Returns the first of the two values that is filled in.
fn prefer(fresh: Option<&str>, older: Option<&str>) -> Option<zeroize::Zeroizing<String>> {
    fresh
        .filter(|value| !value.is_empty())
        .or_else(|| older.filter(|value| !value.is_empty()))
        .map(|s| zeroize::Zeroizing::new(s.to_owned()))
}

/// Reads the persistent Gemini cookies out of the cookie database.
///
/// Firefox keeps the database in WAL mode and holds a lock on it while it
/// runs, so the file is copied out (together with its write-ahead log)
/// before it is read.
///
/// # Errors
///
/// Returns an error if the database cannot be copied or read.
fn read_database_cookies(profile: &Path) -> Result<Vec<ProfileCookie>> {
    let database = profile.join("cookies.sqlite");

    if !database.is_file() {
        return Ok(Vec::new());
    }

    // Copy the database into a temporary directory that is removed
    // automatically when the guard goes out of scope, even on an error or
    // a panic, so the session cookies never linger on disk.
    let directory = tempfile::tempdir().map_err(|e| {
        Error::System(format!(
            "cannot create a temporary directory for the cookie database: {e}"
        ))
    })?;

    let copy = directory.path().join("cookies.sqlite");
    std::fs::copy(&database, &copy)
        .map_err(|e| Error::System(format!("cannot copy {}: {e}", database.display())))?;

    let wal = database.with_extension("sqlite-wal");
    if wal.is_file() {
        let _ = std::fs::copy(&wal, copy.with_extension("sqlite-wal"));
    }

    // The guard is still alive here, so the directory is cleaned up after
    // the cookies have been read, whatever the outcome.
    query_cookies(&copy)
}

/// Reads the Gemini cookies from the copied database.
///
/// # Errors
///
/// Returns an error if the Python interpreter cannot be run or reports a
/// failure.
fn query_cookies(database: &Path) -> Result<Vec<ProfileCookie>> {
    use zeroize::Zeroize;

    let mut output = Command::new(python_bin()?)
        .arg("-c")
        .arg(READ_COOKIES_SCRIPT)
        .arg(database)
        .output()
        .map_err(|e| {
            Error::PythonEnv(format!(
                "cannot run the Python interpreter to read the Firefox cookies: {e}"
            ))
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Error::Auth(format!(
            "Failed to read the Firefox cookie database: {stderr}"
        )));
    }

    let stdout = zeroize::Zeroizing::new(String::from_utf8_lossy(&output.stdout).into_owned());
    output.stdout.zeroize();
    let rows: Vec<CookieRow> = serde_json::from_str(&stdout)
        .map_err(|e| Error::Auth(format!("Invalid cookie data returned by Python: {e}")))?;

    Ok(rows
        .into_iter()
        .map(|(name, value, host)| ProfileCookie { name, value, host })
        .collect())
}

/// A cookie name, value and host, as the Python helper reports it.
type CookieRow = (String, zeroize::Zeroizing<String>, String);

/// Reads the Gemini cookies out of the session stores of the profile.
///
/// The store of a running browser lives under `sessionstore-backups` and the
/// store of a closed one in the root of the profile.
///
/// # Errors
///
/// Returns an error if a store cannot be read.
fn read_store_cookies(profile: &Path) -> Result<Vec<ProfileCookie>> {
    let mut cookies = Vec::new();

    let closed = profile.join("sessionstore.jsonlz4");
    let live = profile
        .join("sessionstore-backups")
        .join("recovery.jsonlz4");

    collect_store_cookies(&closed, &mut cookies)?;
    collect_store_cookies(&live, &mut cookies)?;

    Ok(cookies)
}

/// Adds the Google cookies of one session store to the given set.
///
/// The first store that holds a cookie wins, so the store of a closed browser
/// does not overwrite a value that the live store already reported.
///
/// # Errors
///
/// Returns an error if the store cannot be read.
fn collect_store_cookies(path: &Path, cookies: &mut Vec<ProfileCookie>) -> Result<()> {
    for cookie in read_store(path)? {
        if !cookie.host.ends_with("google.com") {
            continue;
        }
        if cookies
            .iter()
            .any(|c| c.name == cookie.name && c.host == cookie.host)
        {
            continue;
        }
        cookies.push(ProfileCookie {
            name: cookie.name,
            value: cookie.value,
            host: cookie.host,
        });
    }

    Ok(())
}

/// Reads the cookies of one session store file.
///
/// A missing file is not an error: Firefox writes the store only when the
/// session holds something worth keeping.
///
/// # Errors
///
/// Returns an error if the file cannot be read or does not hold a valid store.
pub fn read_store(path: &Path) -> Result<Vec<StoreCookie>> {
    if !path.is_file() {
        return Ok(Vec::new());
    }

    let bytes = std::fs::read(path)
        .map_err(|e| Error::Auth(format!("cannot read {}: {e}", path.display())))?;

    decode_store(&bytes)
}

/// Decodes a compressed session store into the cookies it holds.
///
/// # Errors
///
/// Returns an error if the header is missing or the payload does not match the
/// size it announces.
fn decode_store(bytes: &[u8]) -> Result<Vec<StoreCookie>> {
    let header = bytes
        .get(..HEADER_LEN)
        .ok_or_else(|| Error::Auth("the session store is truncated".into()))?;

    if header.get(..STORE_MAGIC.len()) != Some(STORE_MAGIC) {
        return Err(Error::Auth(
            "the file is not a Firefox session store".into(),
        ));
    }

    let announced = u32::from_le_bytes([
        header.get(8).copied().unwrap_or_default(),
        header.get(9).copied().unwrap_or_default(),
        header.get(10).copied().unwrap_or_default(),
        header.get(11).copied().unwrap_or_default(),
    ]);

    // `announced` is a u32 and `usize` is at least 32 bits on every supported
    // target, so the cast cannot truncate. Guard against an oversized or
    // hostile size before allocating.
    let size = announced as usize;
    if size > MAX_STORE_BYTES {
        return Err(Error::Auth("the Firefox session store is too large".into()));
    }

    let payload = bytes.get(HEADER_LEN..).unwrap_or_default();
    let mut buffer = zeroize::Zeroizing::new(vec![0u8; size]);

    let written = lz4_flex::block::decompress_into(payload, &mut buffer)
        .map_err(|e| Error::Auth(format!("cannot decompress the Firefox session store: {e}")))?;
    buffer.truncate(written);

    let store: SessionStore = serde_json::from_slice(&buffer)
        .map_err(|e| Error::Auth(format!("Invalid Firefox session store: {e}")))?;

    Ok(store.cookies)
}

/// Reads the Gemini cookies from a Firefox cookie database.
///
/// Prints the cookies as JSON, one `[name, value, host]` triple per entry.
const READ_COOKIES_SCRIPT: &str = r#"
import json
import sqlite3
import sys

database = sys.argv[1]
try:
    connection = sqlite3.connect(database)
    rows = connection.execute(
        "SELECT name, value, host FROM moz_cookies "
        "WHERE host LIKE '%google.com'"
    ).fetchall()
except sqlite3.Error as error:
    print(f"cannot read the cookie database: {error}", file=sys.stderr)
    sys.exit(1)
print(json.dumps([[name, value, host] for name, value, host in rows]))
"#;

#[cfg(test)]
#[allow(clippy::result_large_err)]
mod tests {
    use super::{
        decode_store, merge, read_profile_session, read_store, ProfileSession, MAX_STORE_BYTES,
        PSID, PSIDTS,
    };
    use crate::config::Cookie;
    use crate::test_support::TempDir;
    use std::path::Path;

    #[allow(clippy::ref_option)]
    fn opt_str(value: &Option<zeroize::Zeroizing<String>>) -> Option<&str> {
        value.as_ref().map(|s| s.as_str())
    }

    fn z(s: &str) -> zeroize::Zeroizing<String> {
        zeroize::Zeroizing::new(s.to_owned())
    }

    /// Builds a session store the way Firefox writes it.
    fn store_bytes(cookies: &str) -> Vec<u8> {
        let plain = format!(r#"{{"version":1,"cookies":[{cookies}]}}"#);
        let compressed = lz4_flex::block::compress(plain.as_bytes());

        let mut bytes = Vec::from(b"mozLz40\0");
        let size = u32::try_from(plain.len()).unwrap_or_default();
        bytes.extend_from_slice(&size.to_le_bytes());
        bytes.extend_from_slice(&compressed);
        bytes
    }

    /// Returns the first cookie of a decoded store.
    fn first_cookie(cookies: &[super::StoreCookie]) -> anyhow::Result<super::StoreCookie> {
        cookies
            .first()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("the store holds no cookie"))
    }

    #[test]
    fn reads_the_cookies_of_a_store() -> anyhow::Result<()> {
        let bytes =
            store_bytes(r#"{"host":".google.com","name":"__Secure-1PSID","value":"STORE-PSID"}"#);

        let cookies = decode_store(&bytes)?;
        let cookie = first_cookie(&cookies)?;

        assert_eq!(cookie.host, ".google.com");
        assert_eq!(cookie.name, "__Secure-1PSID");
        assert_eq!(cookie.value.as_str(), "STORE-PSID");
        Ok(())
    }

    #[test]
    fn reads_an_empty_store() -> anyhow::Result<()> {
        let cookies = decode_store(&store_bytes(""))?;

        assert!(cookies.is_empty());
        Ok(())
    }

    #[test]
    fn rejects_a_file_that_is_not_a_store() {
        let error = decode_store(b"not a session store");

        assert!(error.is_err());
    }

    #[test]
    fn rejects_a_store_with_a_damaged_header() {
        let mut bytes = store_bytes("");
        bytes.truncate(10);

        assert!(decode_store(&bytes).is_err());
    }

    #[test]
    fn rejects_a_store_with_a_damaged_payload() {
        let mut bytes = store_bytes("");
        bytes.truncate(bytes.len().saturating_sub(4));
        bytes.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF]);

        assert!(decode_store(&bytes).is_err());
    }

    #[test]
    fn rejects_a_store_that_announces_too_much_data() {
        let mut bytes = store_bytes("");
        for index in 8..12 {
            if let Some(byte) = bytes.get_mut(index) {
                *byte = 0xFF;
            }
        }

        assert!(decode_store(&bytes).is_err());
    }

    #[test]
    fn reads_a_store_from_a_file() -> anyhow::Result<()> {
        let dir = TempDir::new("store")?;
        let path = dir.path().join("sessionstore.jsonlz4");
        std::fs::write(
            &path,
            store_bytes(
                r#"{"host":".google.com","name":"__Secure-1PSIDTS","value":"STORE-PSIDTS"}"#,
            ),
        )?;

        let cookies = read_store(Path::new(&path))?;
        let cookie = first_cookie(&cookies)?;

        assert_eq!(cookie.name, "__Secure-1PSIDTS");
        Ok(())
    }

    #[test]
    fn reports_a_missing_store_as_empty() -> anyhow::Result<()> {
        let dir = TempDir::new("store")?;
        let path = dir.path().join("sessionstore.jsonlz4");

        let cookies = read_store(Path::new(&path))?;

        assert!(cookies.is_empty());
        Ok(())
    }

    #[test]
    fn finds_the_session_cookies_of_a_profile() -> anyhow::Result<()> {
        let dir = TempDir::new("profile")?;
        let backups = dir.path().join("sessionstore-backups");
        std::fs::create_dir_all(&backups)?;
        std::fs::write(
            backups.join("recovery.jsonlz4"),
            store_bytes(
                r#"{"host":".google.com","name":"__Secure-1PSID","value":"LIVE-PSID"},
                   {"host":".google.com","name":"__Secure-1PSIDTS","value":"LIVE-PSIDTS"}"#,
            ),
        )?;

        let session = read_profile_session(dir.path())?;

        assert_eq!(opt_str(&session.psid), Some("LIVE-PSID"));
        assert_eq!(opt_str(&session.psidts), Some("LIVE-PSIDTS"));
        assert!(session.is_complete());
        Ok(())
    }

    #[test]
    fn a_profile_keeps_the_extra_google_cookies() -> anyhow::Result<()> {
        let dir = TempDir::new("profile")?;
        let backups = dir.path().join("sessionstore-backups");
        std::fs::create_dir_all(&backups)?;
        std::fs::write(
            backups.join("recovery.jsonlz4"),
            store_bytes(
                r#"{"host":".google.com","name":"__Secure-1PSID","value":"P"},
                   {"host":".google.com","name":"__Secure-1PSIDTS","value":"T"},
                   {"host":".google.com","name":"SID","value":"s"},
                   {"host":"gemini.google.com","name":"OSID","value":"o"},
                   {"host":".example.com","name":"SID","value":"other"}"#,
            ),
        )?;

        let session = read_profile_session(dir.path())?;

        assert_eq!(session.extra.len(), 2);
        assert!(session
            .extra
            .iter()
            .any(|c| c.name.as_str() == "SID" && c.domain == ".google.com"));
        assert!(session
            .extra
            .iter()
            .any(|c| c.name.as_str() == "OSID" && c.domain == "gemini.google.com"));
        Ok(())
    }

    #[test]
    fn an_incomplete_session_yields_no_cookies() {
        let session = ProfileSession {
            psid: Some(z("PSID")),
            psidts: None,
            extra: Vec::new(),
        };

        assert!(!session.is_complete());
        assert!(session.to_cookie_set().is_err());
    }

    #[test]
    fn the_store_size_is_capped() {
        const { assert!(MAX_STORE_BYTES > 0) };
    }

    #[test]
    fn targets_the_google_session_cookies() {
        assert_eq!(PSID, "__Secure-1PSID");
        assert_eq!(PSIDTS, "__Secure-1PSIDTS");
    }

    #[test]
    fn merge_keeps_before_when_after_is_none() {
        let before = ProfileSession {
            psid: Some(z("BEFORE-PSID")),
            psidts: Some(z("BEFORE-PSIDTS")),
            extra: Vec::new(),
        };

        let merged = merge(&before, None);

        assert_eq!(opt_str(&merged.psid), Some("BEFORE-PSID"));
        assert_eq!(opt_str(&merged.psidts), Some("BEFORE-PSIDTS"));
    }

    #[test]
    fn merge_prefers_fresh_values() {
        let before = ProfileSession {
            psid: Some(z("OLD-PSID")),
            psidts: Some(z("OLD-PSIDTS")),
            extra: Vec::new(),
        };
        let after = ProfileSession {
            psid: Some(z("NEW-PSID")),
            psidts: Some(z("NEW-PSIDTS")),
            extra: Vec::new(),
        };

        let merged = merge(&before, Some(&after));

        assert_eq!(opt_str(&merged.psid), Some("NEW-PSID"));
        assert_eq!(opt_str(&merged.psidts), Some("NEW-PSIDTS"));
    }

    #[test]
    fn merge_keeps_older_value_when_fresh_is_empty() {
        let before = ProfileSession {
            psid: Some(z("OLD-PSID")),
            psidts: Some(z("OLD-PSIDTS")),
            extra: Vec::new(),
        };
        let after = ProfileSession {
            psid: Some(z("")),
            psidts: None,
            extra: Vec::new(),
        };

        let merged = merge(&before, Some(&after));

        assert_eq!(opt_str(&merged.psid), Some("OLD-PSID"));
        assert_eq!(opt_str(&merged.psidts), Some("OLD-PSIDTS"));
    }

    #[test]
    fn merge_keeps_older_value_when_fresh_is_none() {
        let before = ProfileSession {
            psid: Some(z("OLD-PSID")),
            psidts: None,
            extra: Vec::new(),
        };
        let after = ProfileSession {
            psid: None,
            psidts: Some(z("NEW-PSIDTS")),
            extra: Vec::new(),
        };

        let merged = merge(&before, Some(&after));

        assert_eq!(opt_str(&merged.psid), Some("OLD-PSID"));
        assert_eq!(opt_str(&merged.psidts), Some("NEW-PSIDTS"));
    }

    #[test]
    fn merge_handles_both_empty() {
        let before = ProfileSession::default();
        let after = ProfileSession::default();

        let merged = merge(&before, Some(&after));

        assert!(!merged.is_complete());
        assert!(merged.to_cookie_set().is_err());
    }

    #[test]
    fn merge_result_can_be_complete() -> anyhow::Result<()> {
        let before = ProfileSession::default();
        let after = ProfileSession {
            psid: Some(z("PSID")),
            psidts: Some(z("PSIDTS")),
            extra: Vec::new(),
        };

        let merged = merge(&before, Some(&after));

        assert!(merged.is_complete());
        let cookies = merged.to_cookie_set()?;
        assert_eq!(cookies.psid.as_str(), "PSID");
        assert_eq!(cookies.psidts.as_str(), "PSIDTS");
        Ok(())
    }

    #[test]
    fn merge_unions_the_extra_cookies() {
        let before = ProfileSession {
            psid: Some(z("P")),
            psidts: Some(z("T")),
            extra: vec![Cookie {
                name: z("SID"),
                value: z("old"),
                domain: ".google.com".to_owned(),
            }],
        };
        let after = ProfileSession {
            psid: Some(z("P")),
            psidts: Some(z("T")),
            extra: vec![Cookie {
                name: z("OSID"),
                value: z("new"),
                domain: "gemini.google.com".to_owned(),
            }],
        };

        let merged = merge(&before, Some(&after));

        assert_eq!(merged.extra.len(), 2);
        assert!(merged
            .extra
            .iter()
            .any(|c| c.name.as_str() == "SID" && c.value.as_str() == "old"));
        assert!(merged
            .extra
            .iter()
            .any(|c| c.name.as_str() == "OSID" && c.value.as_str() == "new"));
    }

    #[test]
    fn profile_session_is_complete_requires_both() {
        assert!(ProfileSession {
            psid: Some(z("a")),
            psidts: Some(z("b")),
            extra: Vec::new(),
        }
        .is_complete());

        assert!(!ProfileSession {
            psid: Some(z("a")),
            psidts: Some(z("")),
            extra: Vec::new(),
        }
        .is_complete());

        assert!(!ProfileSession {
            psid: Some(z("")),
            psidts: Some(z("b")),
            extra: Vec::new(),
        }
        .is_complete());

        assert!(!ProfileSession {
            psid: None,
            psidts: Some(z("b")),
            extra: Vec::new(),
        }
        .is_complete());
    }

    #[test]
    fn to_cookie_set_fails_when_incomplete() {
        let session = ProfileSession {
            psid: Some(z("PSID")),
            psidts: None,
            extra: Vec::new(),
        };
        assert!(session.to_cookie_set().is_err());

        let session = ProfileSession {
            psid: None,
            psidts: Some(z("PSIDTS")),
            extra: Vec::new(),
        };
        assert!(session.to_cookie_set().is_err());
    }

    #[test]
    fn to_cookie_set_succeeds_when_complete() -> anyhow::Result<()> {
        let session = ProfileSession {
            psid: Some(z("PSID")),
            psidts: Some(z("PSIDTS")),
            extra: Vec::new(),
        };
        let cookies = session.to_cookie_set()?;
        assert_eq!(cookies.psid.as_str(), "PSID");
        assert_eq!(cookies.psidts.as_str(), "PSIDTS");
        Ok(())
    }

    #[test]
    fn read_store_returns_empty_for_missing_file() -> anyhow::Result<()> {
        let dir = TempDir::new("store")?;
        let path = dir.path().join("missing.jsonlz4");

        let cookies = read_store(Path::new(&path))?;

        assert!(cookies.is_empty());
        Ok(())
    }

    #[test]
    fn collect_store_cookies_filters_by_host() -> anyhow::Result<()> {
        let dir = TempDir::new("profile")?;
        let backups = dir.path().join("sessionstore-backups");
        std::fs::create_dir_all(&backups)?;
        std::fs::write(
            backups.join("recovery.jsonlz4"),
            store_bytes(
                r#"{"host":".google.com","name":"__Secure-1PSID","value":"LIVE-PSID"},
                   {"host":".example.com","name":"__Secure-1PSID","value":"OTHER-PSID"},
                   {"host":".google.com","name":"unrelated","value":"UNRELATED"}"#,
            ),
        )?;

        let session = read_profile_session(dir.path())?;
        assert_eq!(opt_str(&session.psid), Some("LIVE-PSID"));
        Ok(())
    }

    #[test]
    fn read_database_cookies_returns_empty_when_no_database() -> anyhow::Result<()> {
        let dir = TempDir::new("profile")?;
        // No cookies.sqlite file present.
        let session = read_profile_session(dir.path())?;
        assert!(session.psid.is_none());
        assert!(session.psidts.is_none());
        Ok(())
    }

    #[test]
    fn profile_session_default_is_empty() {
        let session = ProfileSession::default();
        assert!(session.psid.is_none());
        assert!(session.psidts.is_none());
        assert!(!session.is_complete());
    }

    #[test]
    fn store_cookie_deserializes() -> anyhow::Result<()> {
        let json = r#"{"host":".google.com","name":"__Secure-1PSID","value":"V"}"#;
        let cookie: super::StoreCookie = serde_json::from_str(json)?;
        assert_eq!(cookie.host, ".google.com");
        assert_eq!(cookie.name, "__Secure-1PSID");
        assert_eq!(cookie.value.as_str(), "V");
        Ok(())
    }
}
