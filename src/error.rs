#![allow(clippy::result_large_err)]

//! Typed errors for the q crate.
//!
//! Replaces `anyhow::Error` with domain variants so callers and CLI consumers
//! can match on failure kinds and exit codes.

use std::process::ExitCode;
use thiserror::Error;

/// Convenience result alias using [`Error`].
pub type Result<T> = std::result::Result<T, Error>;

/// Typed errors for the q crate.
#[allow(clippy::result_large_err)]
#[derive(Debug, Error)]
pub enum Error {
    /// I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON (de)serialization error.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// The configuration directory could not be determined.
    #[error("configuration directory could not be determined")]
    ConfigDir,

    /// Configuration file or setting error.
    #[error("configuration error: {0}")]
    Config(String),

    /// Metadata file error.
    #[error("metadata file error: {0}")]
    Metadata(String),

    /// Cookie error.
    #[error("cookie error: {0}")]
    Cookie(String),

    /// Authentication error.
    #[error("authentication error: {0}")]
    Auth(String),

    /// Network or transport error.
    #[error("network error: {0}")]
    Network(String),

    /// Rate limit or quota error.
    #[error("rate limit error: {0}")]
    RateLimit(String),

    /// Model refusal or invalid output.
    #[error("model refusal: {0}")]
    ModelRefusal(String),

    /// Provider error.
    #[error("provider error: {0}")]
    Provider(String),

    /// Gemini API error.
    #[error("Gemini error: {0}")]
    Gemini(String),

    /// Python environment error.
    #[error("Python environment error: {0}")]
    PythonEnv(String),

    /// Archive extraction or checksum error.
    #[error("archive error: {0}")]
    Archive(String),

    /// Operating system or subprocess error.
    #[error("system error: {0}")]
    System(String),

    /// Other error.
    #[error("{0}")]
    Other(String),
}

impl Error {
    /// Returns the numeric exit code for this error kind.
    #[must_use]
    pub const fn code(&self) -> u8 {
        match self {
            Self::Other(_) => 1,
            Self::ConfigDir | Self::Config(_) | Self::Metadata(_) | Self::Json(_) => 2,
            Self::Auth(_) | Self::Cookie(_) => 3,
            Self::Network(_) => 4,
            Self::RateLimit(_) => 5,
            Self::ModelRefusal(_) => 6,
            Self::Provider(_) | Self::Gemini(_) => 7,
            Self::Io(_) | Self::PythonEnv(_) | Self::Archive(_) | Self::System(_) => 8,
        }
    }

    /// Returns the process [`ExitCode`] for this error kind.
    pub fn exit_code(&self) -> ExitCode {
        ExitCode::from(self.code())
    }
}

#[cfg(test)]
mod tests {
    use super::Error;

    #[test]
    fn exit_codes_distinguish_domains() {
        assert_eq!(Error::Config("bad".into()).code(), 2);
        assert_eq!(Error::Auth("denied".into()).code(), 3);
        assert_eq!(Error::Network("timeout".into()).code(), 4);
        assert_eq!(Error::RateLimit("429".into()).code(), 5);
        assert_eq!(Error::ModelRefusal("empty".into()).code(), 6);
        assert_eq!(Error::Provider("missing key".into()).code(), 7);
        assert_eq!(Error::System("missing tool".into()).code(), 8);
    }
}
