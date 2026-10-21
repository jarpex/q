//! The API keys of the user, stored the way the cookies are stored.
//!
//! The keys live in a file that only the user can read, so a token never
//! reaches the shell history and never leaks to another account on the
//! machine.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use super::fs_util::write_atomic;

/// One stored token of a provider.
#[derive(Debug, Clone, PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub struct ApiKey {
    /// The provider the token belongs to, such as `openai`.
    pub provider: String,
    /// The token itself.
    pub token: Zeroizing<String>,
    /// The base URL of a local server, when the provider needs one.
    pub endpoint: Option<String>,
}

/// The tokens of every provider the user added.
#[derive(Debug, Clone, Default, PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub struct KeyStore {
    /// The stored tokens.
    pub keys: Vec<ApiKey>,
}

impl KeyStore {
    /// Returns the token of a provider.
    ///
    /// The name of the provider is compared without the marks that people
    /// write differently, so a token of `open_router` is found by the name
    /// `openrouter` too.
    #[must_use]
    pub fn find(&self, provider: &str) -> Option<&ApiKey> {
        self.keys
            .iter()
            .find(|key| same_provider(&key.provider, provider))
    }

    /// Adds a token, replacing the one of the same provider.
    pub fn upsert(&mut self, key: ApiKey) {
        match self
            .keys
            .iter_mut()
            .find(|old| same_provider(&old.provider, &key.provider))
        {
            Some(old) => *old = key,
            None => self.keys.push(key),
        }
    }

    /// Removes the token of a provider and tells whether it was there.
    pub fn remove(&mut self, provider: &str) -> bool {
        let before = self.keys.len();
        self.keys
            .retain(|key| !same_provider(&key.provider, provider));
        before != self.keys.len()
    }
}

/// Tells whether two names mean the same provider.
///
/// The letters and the digits are what tell one provider from another, so
/// `open_router` and `openrouter` are the same one.
fn same_provider(left: &str, right: &str) -> bool {
    let key = |name: &str| -> String {
        name.chars()
            .filter(char::is_ascii_alphanumeric)
            .flat_map(char::to_lowercase)
            .collect()
    };
    key(left) == key(right)
}

/// The shape of the file on disk.
#[derive(Serialize, Deserialize)]
struct KeyDto {
    /// The name of the provider.
    ///
    /// The file of the older versions has the name of the field `platform`,
    /// so it is read as well.
    #[serde(alias = "platform")]
    provider: String,
    token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    endpoint: Option<String>,
}

impl From<KeyDto> for ApiKey {
    fn from(dto: KeyDto) -> Self {
        Self {
            provider: dto.provider,
            token: Zeroizing::new(dto.token),
            endpoint: dto.endpoint,
        }
    }
}

impl From<&ApiKey> for KeyDto {
    fn from(key: &ApiKey) -> Self {
        Self {
            provider: key.provider.clone(),
            token: key.token.to_string(),
            endpoint: key.endpoint.clone(),
        }
    }
}

/// Parses the tokens from the text of the file.
///
/// # Errors
///
/// Returns an error if the JSON is invalid.
pub fn parse_keys(content: &str) -> Result<KeyStore> {
    let dtos: Vec<KeyDto> = serde_json::from_str(content).context("Invalid JSON in keys")?;
    Ok(KeyStore {
        keys: dtos.into_iter().map(ApiKey::from).collect(),
    })
}

/// Loads the tokens from the given path.
///
/// # Errors
///
/// Returns an error if:
/// - The file cannot be read
/// - The file permissions are insecure (readable by others on Unix)
/// - The JSON is invalid
pub fn load_keys<P: AsRef<Path>>(path_arg: P) -> Result<KeyStore> {
    let path = path_arg.as_ref();

    // Verify file permissions on Unix to prevent local token theft.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let metadata = std::fs::metadata(path)
            .with_context(|| format!("Cannot read metadata for {}", path.display()))?;
        let mode = metadata.permissions().mode();
        if mode & 0o077 != 0 {
            anyhow::bail!(
                "Insecure file permissions: {} is readable by others (mode {:o}). \
                 API keys must be private (0600).",
                path.display(),
                mode & 0o777
            );
        }
    }

    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Cannot read keys file: {}", path.display()))?;

    parse_keys(&content).with_context(|| format!("Invalid JSON in keys file: {}", path.display()))
}

/// Saves the tokens to the given path atomically.
///
/// # Errors
///
/// Returns an error if the serialization or the write fails.
pub fn save_keys<P: AsRef<Path>>(path_arg: P, store: &KeyStore) -> Result<()> {
    let path = path_arg.as_ref();
    let dtos: Vec<KeyDto> = store.keys.iter().map(KeyDto::from).collect();
    let json = serde_json::to_string(&dtos)?;
    write_atomic(path, json.as_bytes())
}

/// Removes the saved keys file.
///
/// # Errors
///
/// Returns an error if the file cannot be removed for another reason.
pub fn remove_keys<P: AsRef<Path>>(path_arg: P) -> Result<()> {
    let path = path_arg.as_ref();

    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => {
            Err(error).with_context(|| format!("Failed to remove keys file: {}", path.display()))
        }
    }
}

/// Masks a token, so the screen shows where it starts and where it ends.
///
/// The middle of the token is hidden, because the whole token is a secret.
#[must_use]
pub fn mask_token(token: &str) -> String {
    const VISIBLE: usize = 4;

    let chars: Vec<char> = token.chars().collect();
    if chars.len() <= VISIBLE * 2 {
        return "*".repeat(chars.len());
    }

    let start: String = chars.iter().take(VISIBLE).collect();
    let end: String = chars.iter().skip(chars.len() - VISIBLE).collect();
    let hidden = chars.len() - VISIBLE * 2;
    format!("{start}{}{end}", "*".repeat(hidden))
}

#[cfg(test)]
#[allow(clippy::result_large_err)]
mod tests {
    use super::{load_keys, mask_token, parse_keys, remove_keys, save_keys, ApiKey, KeyStore};
    use crate::test_support::TempDir;
    use zeroize::Zeroizing;

    fn store() -> KeyStore {
        KeyStore {
            keys: vec![ApiKey {
                provider: "openai".to_owned(),
                token: Zeroizing::new("sk-secret-value".to_owned()),
                endpoint: None,
            }],
        }
    }

    #[test]
    fn saves_and_loads_the_tokens() -> anyhow::Result<()> {
        let dir = TempDir::new("keys")?;
        let path = dir.path().join("keys.json");

        save_keys(&path, &store())?;
        let loaded = load_keys(&path)?;

        assert_eq!(loaded.keys.len(), 1);
        let key = loaded.keys.first();
        assert!(key.is_some_and(|item| item.provider == "openai"), "{key:?}");
        assert!(
            key.is_some_and(|item| item.token.as_str() == "sk-secret-value"),
            "{key:?}"
        );
        Ok(())
    }

    #[test]
    fn keeps_the_file_private() -> anyhow::Result<()> {
        let dir = TempDir::new("keys")?;
        let path = dir.path().join("keys.json");

        save_keys(&path, &store())?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path)?.permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "mode: {mode:o}");
        }
        Ok(())
    }

    #[test]
    fn refuses_a_file_that_others_can_read() -> anyhow::Result<()> {
        let dir = TempDir::new("keys")?;
        let path = dir.path().join("keys.json");
        save_keys(&path, &store())?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))?;
            assert!(
                load_keys(&path).is_err(),
                "the insecure file must be refused"
            );
        }
        Ok(())
    }

    #[test]
    fn replaces_the_token_of_the_same_provider() {
        let mut keys = store();
        keys.upsert(ApiKey {
            provider: "openai".to_owned(),
            token: Zeroizing::new("sk-new-value".to_owned()),
            endpoint: None,
        });

        assert_eq!(keys.keys.len(), 1);
        let key = keys.keys.first();
        assert!(
            key.is_some_and(|item| item.token.as_str() == "sk-new-value"),
            "{key:?}"
        );
    }

    #[test]
    fn reads_the_file_of_the_older_versions() {
        let content = r#"[{"platform":"open_router","token":"sk-secret","endpoint":null}]"#;

        let store = parse_keys(content).unwrap_or_default();

        assert_eq!(store.keys.len(), 1, "{store:?}");
        assert_eq!(
            store.keys.first().map(|k| k.provider.as_str()),
            Some("open_router")
        );
        assert_eq!(
            store.keys.first().map(|k| k.token.as_str()),
            Some("sk-secret")
        );
    }

    #[test]
    fn finds_the_token_of_a_provider_by_any_spelling() {
        let store = KeyStore {
            keys: vec![ApiKey {
                provider: "open_router".to_owned(),
                token: Zeroizing::new("sk-secret".to_owned()),
                endpoint: None,
            }],
        };

        assert!(store.find("openrouter").is_some());
        assert!(store.find("open_router").is_some());
        assert!(store.find("OpenRouter").is_some());
    }

    #[test]
    fn removes_the_token_of_a_provider() {
        let mut keys = store();

        assert!(keys.remove("openai"));
        assert!(!keys.remove("openai"));
        assert_eq!(keys.keys, Vec::new());
    }

    #[test]
    fn finds_the_token_of_a_provider() {
        let keys = store();

        assert!(keys.find("openai").is_some());
        assert!(keys.find("anthropic").is_none());
    }

    #[test]
    fn parses_an_empty_list() -> anyhow::Result<()> {
        let parsed = parse_keys("[]")?;
        assert_eq!(parsed.keys, Vec::new());
        Ok(())
    }

    #[test]
    fn rejects_invalid_json() {
        assert!(parse_keys("{").is_err());
    }

    #[test]
    fn removes_a_missing_file_without_a_error() -> anyhow::Result<()> {
        let dir = TempDir::new("keys")?;
        remove_keys(dir.path().join("missing.json"))?;
        Ok(())
    }

    #[test]
    fn masks_the_middle_of_a_token() {
        assert_eq!(mask_token("sk-abcdefghijkl"), "sk-a*******ijkl");
        assert_eq!(mask_token("abcd"), "****");
        assert_eq!(mask_token(""), "");
    }
}
