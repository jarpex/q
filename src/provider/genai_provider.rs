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
use url::Url;

use crate::config::ApiKey;

use super::{
    canonical_provider, AccountUsage, ModelList, Provider, ProviderEvent, ProviderFuture, Query,
};

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
        let name = canonical_provider(&key.provider).with_context(|| {
            format!(
                "Unknown provider: {}. Run with --api-management to see them.",
                key.provider
            )
        })?;
        let kind = AdapterKind::from_lower_str(name)
            .with_context(|| format!("Unknown provider: {}", key.provider))?;

        Ok(Self {
            name: name.to_owned(),
            kind,
            token: Arc::from(key.token.as_str()),
            endpoint: key.endpoint.as_deref().map(Arc::from),
        })
    }

    /// The configuration of the platform, with the token of the user.
    ///
    /// The configuration carries the token, so the library does not look at
    /// the environment for a token that the user already stored.
    fn provider_config(&self, endpoint: Endpoint) -> ProviderConfig {
        ProviderConfig {
            endpoint: Some(endpoint),
            auth: Some(AuthData::Key(self.token.to_string())),
        }
    }

    /// The base URL of the platform, with the address of the user when there
    /// is one.
    ///
    /// A platform without an address of its own keeps the one of the library.
    /// The address of a local server replaces the host of the platform, but
    /// keeps its path, so the server speaks the same protocol.
    async fn resolve_endpoint(&self) -> Result<Endpoint> {
        let client = Client::new().context("Cannot create the client of the provider")?;
        let default = client
            .resolve_service_target(ModelIden::new(self.kind, ""))
            .await
            .context("Cannot resolve the endpoint of the platform")?
            .endpoint;

        let Some(given) = self.endpoint.as_deref() else {
            return Ok(default);
        };

        Ok(Endpoint::from_owned(combine_endpoint(
            given,
            default.base_url(),
        )?))
    }

    /// The target of a call, with the token and the endpoint of the user.
    async fn service_target(&self, model: &str) -> Result<ServiceTarget> {
        Ok(ServiceTarget {
            endpoint: self.resolve_endpoint().await?,
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
            let endpoint = self.resolve_endpoint().await?;
            let models = client
                .all_model_names(self.kind, self.provider_config(endpoint))
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

/// Puts the address of a local server onto the path of the platform.
///
/// The library adds the path of the request to the base address as a string,
/// so the address has to end with a slash. Without it the request goes to a
/// wrong path, for example the models go to `v1models` instead of `v1/models`.
///
/// The address of a local server carries no path of its own, since the path
/// belongs to the protocol. The path of the platform is used then, so that
/// the server speaks the same protocol as the platform it stands for.
fn combine_endpoint(given: &str, default: &str) -> Result<String> {
    let mut url = Url::parse(given.trim())
        .with_context(|| format!("Cannot read the address of the server: {given}"))?;
    let default_url = Url::parse(default).context("Cannot read the address of the platform")?;

    if url.path() == "/" || url.path().is_empty() {
        url.set_path(default_url.path());
    }

    let path = url.path().to_owned();
    if !path.ends_with('/') {
        url.set_path(&format!("{path}/"));
    }

    Ok(url.to_string())
}

/// Tells whether the message carries nothing worth showing.
const fn message_is_empty(message: &ProviderEvent) -> bool {
    matches!(message, ProviderEvent::Chunk(text) if text.is_empty())
}

#[cfg(test)]
#[allow(clippy::result_large_err)]
mod tests {
    use super::{combine_endpoint, GenaiProvider, Provider, ProviderEvent, Query};
    use crate::config::ApiKey;
    use zeroize::Zeroizing;

    /// The place where the paths of the requests are kept.
    type Paths = std::sync::Arc<std::sync::Mutex<Vec<String>>>;

    /// A local server that answers the requests of the test.
    struct MockServer {
        /// The address of the server.
        base: String,
        /// The thread that answers the requests.
        handle: std::thread::JoinHandle<()>,
        /// The paths of the requests that the server answered.
        paths: Paths,
    }

    impl MockServer {
        /// The paths of the requests that the server answered.
        fn paths(&self) -> Vec<String> {
            self.paths
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
        }
    }

    fn key(provider: &str) -> ApiKey {
        ApiKey {
            provider: provider.to_owned(),
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

        let paths: Paths = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let collected = std::sync::Arc::clone(&paths);

        let handle = std::thread::spawn(move || {
            for connection in listener.incoming().take(1) {
                let Ok(stream) = connection else { continue };
                let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
                let Ok(mut writer) = stream.try_clone() else {
                    continue;
                };
                let mut reader = BufReader::new(stream);

                // The first line of the request holds the path.
                let mut request_line = String::new();
                let _ = reader.read_line(&mut request_line);
                let path = request_line
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or_default()
                    .to_owned();

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

                collected
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(path);
            }
        });

        Ok(MockServer {
            base: format!("http://{address}/v1"),
            handle,
            paths,
        })
    }

    #[tokio::test]
    async fn lists_the_models_of_a_local_server() -> anyhow::Result<()> {
        let server = mock_server(r#"{"data": [{"id": "local-model"}]}"#)?;
        let key = ApiKey {
            provider: "openai".to_owned(),
            token: Zeroizing::new("local-key".to_owned()),
            endpoint: Some(server.base.clone()),
        };

        let provider = GenaiProvider::new(&key)?;
        let models = provider.list_models().await?;

        assert_eq!(models, vec!["local-model".to_owned()]);
        let paths = server.paths();
        let _ = server.handle.join();
        assert_eq!(paths, vec!["/v1/models".to_owned()]);
        Ok(())
    }

    #[tokio::test]
    async fn answers_a_query_of_a_local_server() -> anyhow::Result<()> {
        let chunk = r#"{"id":"1","object":"chat.completion.chunk","model":"local-model","choices":[{"index":0,"delta":{"content":"the answer"},"finish_reason":"stop"}]}"#;
        let body: &'static str =
            Box::leak(format!("data: {chunk}\n\ndata: [DONE]\n\n").into_boxed_str());
        let server = mock_server_with_type(body, "text/event-stream")?;
        let key = ApiKey {
            provider: "openai".to_owned(),
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
        let paths = server.paths();
        let _ = server.handle.join();
        assert_eq!(paths, vec!["/v1/chat/completions".to_owned()]);
        Ok(())
    }

    #[test]
    fn the_address_of_a_local_server_keeps_the_path_of_the_platform() {
        let given = combine_endpoint("http://127.0.0.1:1234", "https://api.openai.com/v1/");
        assert_eq!(
            given.unwrap_or_default(),
            "http://127.0.0.1:1234/v1/",
            "the path of the platform is missing"
        );
    }

    #[test]
    fn the_address_of_a_local_server_with_a_path_of_its_own_keeps_it() {
        let given = combine_endpoint(
            "http://127.0.0.1:1234/custom/v1",
            "https://api.openai.com/v1/",
        );
        assert_eq!(
            given.unwrap_or_default(),
            "http://127.0.0.1:1234/custom/v1/"
        );
    }

    #[test]
    fn the_address_of_a_local_server_ends_with_a_slash() {
        let given = combine_endpoint("http://127.0.0.1:1234/v1", "https://api.openai.com/v1/");
        assert_eq!(given.unwrap_or_default(), "http://127.0.0.1:1234/v1/");
    }

    #[test]
    fn the_address_of_a_local_ollama_keeps_the_path_of_the_platform() {
        let given = combine_endpoint("http://127.0.0.1:11434", "http://localhost:11434/");
        assert_eq!(given.unwrap_or_default(), "http://127.0.0.1:11434/");
    }

    #[test]
    fn a_wrong_address_gives_an_error() {
        let given = combine_endpoint("not an address", "https://api.openai.com/v1/");
        assert!(given.is_err(), "{given:?}");
    }

    #[tokio::test]
    async fn the_platform_reports_no_usage() -> anyhow::Result<()> {
        let provider = GenaiProvider::new(&key("openai"))?;

        let limits = provider.usage_limits().await?;
        assert!(limits.is_none());
        Ok(())
    }
}
