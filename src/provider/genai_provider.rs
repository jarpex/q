//! The provider that works with a token of the user.
//!
//! The token comes from the store of the user, so it never reaches the shell
//! history. The protocol comes from the platform of the token, so an `OpenAI`
//! key talks to `OpenAI` and an `Anthropic` key talks to `Anthropic`. A local
//! server uses the same protocols with a base URL of its own.

use anyhow::{Context, Result};
use futures_util::StreamExt;
use genai::adapter::AdapterKind;
use genai::chat::{ChatMessage, ChatRequest, ChatStreamEvent};
use genai::resolver::{AuthData, Endpoint, ProviderConfig};
use genai::{Client, ModelIden, ServiceTarget};
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::config::ApiKey;

use super::{AccountUsage, ModelList, Provider, ProviderEvent, ProviderFuture, Query};

/// The provider that talks to a platform with the token of the user.
pub struct GenaiProvider {
    /// The name of the platform, as the user wrote it.
    name: String,
    /// The protocol of the platform.
    kind: AdapterKind,
    /// The token of the user.
    token: Arc<str>,
    /// The base URL of a local server.
    endpoint: Option<Arc<str>>,
}

impl GenaiProvider {
    /// Creates the provider from a stored token.
    ///
    /// # Errors
    ///
    /// Returns an error when the platform of the token is unknown.
    pub fn new(key: &ApiKey) -> Result<Self> {
        let kind = AdapterKind::from_lower_str(&key.platform.to_lowercase())
            .with_context(|| format!("Unknown platform: {}", key.platform))?;

        Ok(Self {
            name: key.platform.to_lowercase(),
            kind,
            token: Arc::from(key.token.as_str()),
            endpoint: key.endpoint.as_deref().map(Arc::from),
        })
    }

    /// The configuration of the platform, with the token of the user.
    ///
    /// The configuration carries the token, so the library does not look at
    /// the environment for a token that the user already stored.
    fn provider_config(&self) -> ProviderConfig {
        ProviderConfig {
            endpoint: self
                .endpoint
                .as_deref()
                .map(|url| Endpoint::from_owned(url.to_owned())),
            auth: Some(AuthData::Key(self.token.to_string())),
        }
    }

    /// The target of a call, with the token and the endpoint of the user.
    ///
    /// A platform without an endpoint of its own keeps the one of the library.
    async fn service_target(&self, model: &str) -> Result<ServiceTarget> {
        let endpoint = if let Some(url) = self.endpoint.as_deref() {
            Endpoint::from_owned(url.to_owned())
        } else {
            let client = Client::new().context("Cannot create the client of the provider")?;
            client
                .resolve_service_target(ModelIden::new(self.kind, model))
                .await
                .context("Cannot resolve the endpoint of the platform")?
                .endpoint
        };

        Ok(ServiceTarget {
            endpoint,
            auth: AuthData::Key(self.token.to_string()),
            model: ModelIden::new(self.kind, model),
        })
    }

    /// Starts an answer to the query and returns the channel of the parts.
    async fn start_query_inner(&self, query: Query) -> Result<mpsc::Receiver<ProviderEvent>> {
        let client = Client::new().context("Cannot create the client of the provider")?;
        let target = self.service_target(&query.model).await?;
        let prompt = query.prompt;

        let (sender, receiver) = mpsc::channel(64);
        tokio::spawn(async move {
            let request = ChatRequest::new(vec![ChatMessage::user(prompt)]);
            let response = match client.exec_chat_stream(target, request, None).await {
                Ok(response) => response,
                Err(error) => {
                    let _ = sender.send(ProviderEvent::Error(error.to_string())).await;
                    return;
                }
            };
            let mut stream = response.stream;

            while let Some(part) = stream.next().await {
                let message = match part {
                    Ok(ChatStreamEvent::Chunk(chunk)) => ProviderEvent::Chunk(chunk.content),
                    Ok(_) => continue,
                    Err(error) => {
                        let _ = sender.send(ProviderEvent::Error(error.to_string())).await;
                        return;
                    }
                };
                if message_is_empty(&message) {
                    continue;
                }
                if sender.send(message).await.is_err() {
                    return;
                }
            }
            let _ = sender.send(ProviderEvent::Done).await;
        });

        Ok(receiver)
    }
}

impl Provider for GenaiProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn list_models(&self) -> ProviderFuture<'_, ModelList> {
        Box::pin(async move {
            let client = Client::new().context("Cannot create the client of the provider")?;
            let models = client
                .all_model_names(self.kind, self.provider_config())
                .await
                .with_context(|| format!("Cannot list the models of {}", self.name))?;
            Ok(models)
        })
    }

    fn usage_limits(&self) -> ProviderFuture<'_, Option<AccountUsage>> {
        // The platforms do not report the usage of the account.
        Box::pin(async { Ok(None) })
    }

    fn start_query(&self, query: Query) -> ProviderFuture<'_, mpsc::Receiver<ProviderEvent>> {
        Box::pin(self.start_query_inner(query))
    }
}

/// Tells whether the message carries nothing worth showing.
const fn message_is_empty(message: &ProviderEvent) -> bool {
    matches!(message, ProviderEvent::Chunk(text) if text.is_empty())
}

#[cfg(test)]
#[allow(clippy::result_large_err)]
mod tests {
    use super::{GenaiProvider, Provider};
    use crate::config::ApiKey;
    use zeroize::Zeroizing;

    fn key(platform: &str) -> ApiKey {
        ApiKey {
            platform: platform.to_owned(),
            token: Zeroizing::new("sk-test".to_owned()),
            endpoint: None,
        }
    }

    #[test]
    fn a_known_platform_gives_a_provider() -> anyhow::Result<()> {
        let provider = GenaiProvider::new(&key("openai"))?;

        assert_eq!(provider.name(), "openai");
        Ok(())
    }

    #[test]
    fn the_name_of_the_platform_is_lowercase() -> anyhow::Result<()> {
        let provider = GenaiProvider::new(&key("OpenAI"))?;

        assert_eq!(provider.name(), "openai");
        Ok(())
    }

    #[test]
    fn an_unknown_platform_gives_an_error() {
        assert!(GenaiProvider::new(&key("nosuchplatform")).is_err());
    }

    #[tokio::test]
    async fn the_platform_reports_no_usage() -> anyhow::Result<()> {
        let provider = GenaiProvider::new(&key("openai"))?;

        let limits = provider.usage_limits().await?;
        assert!(limits.is_none());
        Ok(())
    }
}
