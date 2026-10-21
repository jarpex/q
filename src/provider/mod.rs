//! The providers of models that the tool can talk to.
//!
//! Every provider answers the same questions: which models it has, how much
//! of the quota is left, and what it says to a query. The tool picks one
//! provider and works with it the same way, no matter which one it is.

pub mod genai_provider;
pub mod google_cookies;

use anyhow::Result;
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use tokio::sync::mpsc;

pub use genai_provider::GenaiProvider;
pub use google_cookies::GoogleCookiesProvider;

/// The provider that works without an API key, through the cookies of the
/// account of the user.
pub const GOOGLE_COOKIES: &str = "google-cookies";

/// A part of an answer of a model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderEvent {
    /// A chunk of the text of the answer.
    Chunk(String),
    /// The provider failed.
    Error(String),
    /// The answer is complete.
    Done,
}

/// What a provider needs to answer a query.
#[derive(Debug, Clone)]
pub struct Query {
    /// The question of the user.
    pub prompt: String,
    /// The model to use.
    pub model: String,
    /// Whether the answer comes in parts or all at once.
    pub stream: bool,
}

/// A provider of models, as the application holds it.
pub type BoxedProvider = Box<dyn Provider>;

/// The future of a call of a provider.
///
/// The future is boxed, so the trait stays usable with a trait object.
pub type ProviderFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// The list of the models of a provider.
pub type ModelList = Vec<String>;

/// The usage of an account, as the windows and the quotas of it.
pub type AccountUsage = (Value, Value);

/// A source of models and answers.
pub trait Provider: Send + Sync {
    /// The name of the provider, as the user writes it on the command line.
    fn name(&self) -> &str;

    /// The models the provider offers.
    ///
    /// # Errors
    ///
    /// Returns an error when the provider cannot be reached.
    fn list_models(&self) -> ProviderFuture<'_, ModelList>;

    /// The usage of the account, when the provider reports it.
    ///
    /// # Errors
    ///
    /// Returns an error when the provider cannot be reached.
    fn usage_limits(&self) -> ProviderFuture<'_, Option<AccountUsage>>;

    /// Starts an answer to the query.
    ///
    /// The answer comes through the channel, so the caller reads it while the
    /// provider works.
    ///
    /// # Errors
    ///
    /// Returns an error when the provider cannot be reached.
    fn start_query(&self, query: Query) -> ProviderFuture<'_, mpsc::Receiver<ProviderEvent>>;
}

/// The provider the user asked for, or the default one.
///
/// The name is the one of the platform of a token, so the same name picks the
/// token and the protocol.
#[must_use]
pub fn resolve_provider(name: Option<&str>) -> &str {
    name.unwrap_or(GOOGLE_COOKIES)
}

/// The names of the providers that need a token.
///
/// The list comes from the provider library, so it always matches the
/// platforms it can talk to. The names are the ones the library knows, so
/// they work on the command line and in the store of the tokens.
#[must_use]
pub fn token_providers() -> Vec<String> {
    use genai::adapter::AdapterKind;

    let mut names: Vec<String> = AdapterKind::all()
        .iter()
        .map(|kind| kind.as_lower_str().to_owned())
        .collect();
    names.sort_by_key(|name| (sort_key(name), name.clone()));
    names
}

/// The name of a provider, as the library knows it.
///
/// The user writes the name in any way, so `openrouter`, `open_router` and
/// `OpenRouter` all mean the same provider. The name of the library is
/// returned, so the store of the tokens and the command line speak the same
/// language.
#[must_use]
pub fn canonical_provider(name: &str) -> Option<&'static str> {
    use genai::adapter::AdapterKind;

    let wanted = sort_key(name);
    AdapterKind::all()
        .iter()
        .map(genai::adapter::AdapterKind::as_lower_str)
        .find(|candidate| sort_key(candidate) == wanted)
}

/// The name of a provider without the marks that people write differently.
///
/// The letters and the digits are what tell one provider from another, so
/// `open_router` and `openrouter` are the same one.
fn sort_key(name: &str) -> String {
    name.chars()
        .filter(char::is_ascii_alphanumeric)
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
#[allow(clippy::result_large_err)]
mod tests {
    use super::{canonical_provider, resolve_provider, sort_key, token_providers, GOOGLE_COOKIES};

    #[test]
    fn the_default_provider_is_the_cookie_one() {
        assert_eq!(resolve_provider(None), GOOGLE_COOKIES);
    }

    #[test]
    fn the_named_provider_wins() {
        assert_eq!(resolve_provider(Some("openai")), "openai");
    }

    #[test]
    fn the_platform_list_is_not_empty() {
        let providers = token_providers();

        assert!(providers.len() > 10, "{providers:?}");
        assert!(providers.iter().any(|p| p == "openai"), "{providers:?}");
        assert!(providers.iter().any(|p| p == "anthropic"), "{providers:?}");
        assert!(providers.iter().any(|p| p == "ollama"), "{providers:?}");
    }

    #[test]
    fn the_platform_list_is_in_the_alphabet() {
        let providers = token_providers();
        let mut sorted = providers.clone();
        // The order of the list, so the test checks the same thing the list does.
        sorted.sort_by_key(|name| (sort_key(name), name.clone()));

        assert_eq!(providers, sorted, "the list is not sorted");
    }

    #[test]
    fn the_platform_list_reads_like_an_alphabet() {
        let providers = token_providers();
        let names: Vec<String> = providers.iter().map(|name| sort_key(name)).collect();
        let mut sorted = names.clone();
        sorted.sort();

        assert_eq!(names, sorted, "the list is not in the alphabet");
    }

    #[test]
    fn the_platform_list_holds_the_names_of_the_library() {
        let providers = token_providers();

        assert!(
            providers.iter().any(|p| p == "open_router"),
            "{providers:?}"
        );
    }

    #[test]
    fn a_provider_is_found_by_any_spelling() {
        assert_eq!(canonical_provider("openrouter"), Some("open_router"));
        assert_eq!(canonical_provider("open_router"), Some("open_router"));
        assert_eq!(canonical_provider("OpenRouter"), Some("open_router"));
        assert_eq!(canonical_provider("OPENROUTER"), Some("open_router"));
        assert_eq!(canonical_provider("openai"), Some("openai"));
    }

    #[test]
    fn an_unknown_provider_is_not_found() {
        assert_eq!(canonical_provider("nosuchprovider"), None);
    }
}
