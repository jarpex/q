//! The provider that works through the cookies of the account of the user.
//!
//! It drives the Python worker, so the tool needs the virtual environment and
//! the cookies of the account. It is the default, because it needs no token.

use anyhow::Result;
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;

use crate::config::CookieSet;
use crate::gemini::{list_models, spawn_gemini_stream, usage_limits, AskOptions, GeminiEvent};

use super::{AccountUsage, ModelList, Provider, ProviderEvent, ProviderFuture, Query};

/// The provider that talks to Gemini through the cookies of the account.
pub struct GoogleCookiesProvider {
    /// The path to the Python executable.
    python_bin: PathBuf,
    /// The authenticated session cookies.
    cookies: CookieSet,
    /// Whether to enable debug logging.
    debug: bool,
}

impl GoogleCookiesProvider {
    /// Creates the provider from the parts the application already has.
    #[must_use]
    pub fn new(python_bin: &Path, cookies: &CookieSet, debug: bool) -> Self {
        Self {
            python_bin: python_bin.to_path_buf(),
            cookies: cookies.clone(),
            debug,
        }
    }

    /// Starts an answer to the query and returns the channel of the parts.
    async fn start_query_inner(&self, query: Query) -> Result<mpsc::Receiver<ProviderEvent>> {
        let options = AskOptions {
            python_bin: &self.python_bin,
            cookies: &self.cookies,
            query: &query.prompt,
            model: &query.model,
            stream: query.stream,
            debug: self.debug,
        };

        let mut source = spawn_gemini_stream(options).await?;
        let (sender, receiver) = mpsc::channel(64);

        tokio::spawn(async move {
            while let Some(event) = source.recv().await {
                let message = match event {
                    GeminiEvent::Chunk(text) => ProviderEvent::Chunk(text),
                    GeminiEvent::Error(error) => ProviderEvent::Error(error),
                    GeminiEvent::Done => ProviderEvent::Done,
                };
                let finished = matches!(message, ProviderEvent::Done | ProviderEvent::Error(_));
                if sender.send(message).await.is_err() || finished {
                    break;
                }
            }
        });

        Ok(receiver)
    }
}

impl Provider for GoogleCookiesProvider {
    fn name(&self) -> &str {
        super::GOOGLE_COOKIES
    }

    fn list_models(&self) -> ProviderFuture<'_, ModelList> {
        Box::pin(async move { list_models(&self.python_bin, &self.cookies, self.debug).await })
    }

    fn usage_limits(&self) -> ProviderFuture<'_, Option<AccountUsage>> {
        Box::pin(async move {
            let limits = usage_limits(&self.python_bin, &self.cookies, self.debug).await?;
            Ok(Some((limits.usage_info, limits.quotas)))
        })
    }

    fn start_query(&self, query: Query) -> ProviderFuture<'_, mpsc::Receiver<ProviderEvent>> {
        Box::pin(self.start_query_inner(query))
    }
}
