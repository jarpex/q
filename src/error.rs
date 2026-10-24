#![allow(clippy::result_large_err)]

//! Typed errors for the q crate.
//!
//! Replaces the catch-all `anyhow::Error` with specific variants so callers
//! can match on the kind of failure. `anyhow::Error` is kept as a fallback
//! variant for parts of the codebase that have not been migrated yet.

use thiserror::Error;

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

    /// Metadata file error.
    #[error("metadata file error: {0}")]
    Metadata(String),

    /// Cookie error.
    #[error("cookie error: {0}")]
    Cookie(String),

    /// Authentication error.
    #[error("authentication error: {0}")]
    Auth(String),

    /// Provider error.
    #[error("provider error: {0}")]
    Provider(String),

    /// Gemini API error.
    #[error("Gemini error: {0}")]
    Gemini(String),

    /// Python environment error.
    #[error("Python environment error: {0}")]
    PythonEnv(String),

    /// Other error.
    #[error("{0}")]
    Other(String),

    /// Wrapped anyhow error (for unmigrated code).
    #[error(transparent)]
    Anyhow(#[from] anyhow::Error),
}
