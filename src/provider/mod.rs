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
/// platforms it can talk to.
#[must_use]
pub fn token_providers() -> Vec<String> {
    use genai::adapter::AdapterKind;

    AdapterKind::all()
        .iter()
        .map(|kind| kind.to_string().to_lowercase())
        .collect()
}

#[cfg(test)]
#[allow(clippy::result_large_err)]
mod tests {
    use super::{resolve_provider, token_providers, GOOGLE_COOKIES};

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
}
