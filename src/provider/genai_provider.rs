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
    use super::{GenaiProvider, Provider, ProviderEvent, Query};
    use crate::config::ApiKey;
    use zeroize::Zeroizing;

    /// A local server that answers the requests of the test.
    struct MockServer {
        /// The address of the server.
        base: String,
        /// The thread that answers the requests.
        handle: std::thread::JoinHandle<()>,
    }

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

    /// A server that answers like a local one that speaks the `OpenAI` protocol.
    ///
    /// The server takes one request and then ends, so the thread does not wait
    /// for more.
    fn mock_server(body: &'static str) -> anyhow::Result<MockServer> {
        mock_server_with_type(body, "application/json")
    }

    /// A server that answers with the given type of body.
    fn mock_server_with_type(
        body: &'static str,
        content_type: &'static str,
    ) -> anyhow::Result<MockServer> {
        use std::io::{BufRead, BufReader, Read, Write};
        use std::net::TcpListener;
        use std::time::Duration;

        let listener = TcpListener::bind("127.0.0.1:0")
            .map_err(|error| anyhow::anyhow!("cannot bind the mock server: {error}"))?;
        let address = listener
            .local_addr()
            .map_err(|error| anyhow::anyhow!("cannot read the address: {error}"))?;

        let handle = std::thread::spawn(move || {
            for connection in listener.incoming().take(1) {
                let Ok(stream) = connection else { continue };
                let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
                let Ok(mut writer) = stream.try_clone() else {
                    continue;
                };
                let mut reader = BufReader::new(stream);

                // The headers end with an empty line.
                let mut length = 0_usize;
                let mut line = String::new();
                while reader.read_line(&mut line).unwrap_or(0) > 0 {
                    let trimmed = line.trim_end();
                    if trimmed.is_empty() {
                        break;
                    }
                    if let Some(value) = trimmed.strip_prefix("Content-Length:") {
                        length = value.trim().parse().unwrap_or(0);
                    }
                    line.clear();
                }

                // The body of the request follows the headers.
                let mut payload = vec![0_u8; length];
                let _ = reader.read_exact(&mut payload);

                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = writer.write_all(response.as_bytes());
                let _ = writer.flush();
            }
        });

        Ok(MockServer {
            base: format!("http://{address}/v1"),
            handle,
        })
    }

    #[tokio::test]
    async fn lists_the_models_of_a_local_server() -> anyhow::Result<()> {
        let server = mock_server(r#"{"data": [{"id": "local-model"}]}"#)?;
        let key = ApiKey {
            platform: "openai".to_owned(),
            token: Zeroizing::new("local-key".to_owned()),
            endpoint: Some(server.base.clone()),
        };

        let provider = GenaiProvider::new(&key)?;
        let models = provider.list_models().await?;

        assert_eq!(models, vec!["local-model".to_owned()]);
        let _ = server.handle.join();
        Ok(())
    }

    #[tokio::test]
    async fn answers_a_query_of_a_local_server() -> anyhow::Result<()> {
        let chunk = r#"{"id":"1","object":"chat.completion.chunk","model":"local-model","choices":[{"index":0,"delta":{"content":"the answer"},"finish_reason":"stop"}]}"#;
        let body: &'static str =
            Box::leak(format!("data: {chunk}\n\ndata: [DONE]\n\n").into_boxed_str());
        let server = mock_server_with_type(body, "text/event-stream")?;
        let key = ApiKey {
            platform: "openai".to_owned(),
            token: Zeroizing::new("local-key".to_owned()),
            endpoint: Some(server.base.clone()),
        };

        let provider = GenaiProvider::new(&key)?;
        let query = Query {
            prompt: "hello".to_owned(),
            model: "local-model".to_owned(),
            stream: true,
        };
        let mut events = provider.start_query(query).await?;

        let mut answer = String::new();
        while let Some(event) = events.recv().await {
            match event {
                ProviderEvent::Chunk(text) => answer.push_str(&text),
                ProviderEvent::Error(error) => return Err(anyhow::anyhow!(error)),
                ProviderEvent::Done => break,
            }
        }

        assert_eq!(answer, "the answer");
        let _ = server.handle.join();
        Ok(())
    }

    #[tokio::test]
    async fn the_platform_reports_no_usage() -> anyhow::Result<()> {
        let provider = GenaiProvider::new(&key("openai"))?;

        let limits = provider.usage_limits().await?;
        assert!(limits.is_none());
        Ok(())
    }
}
