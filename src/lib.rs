//! Library interface for the q tool.
//!
//! This module exposes internal functionality for testing and fuzzing.

/// Authentication via webview.
pub mod auth;
/// Command-line interface parsing and execution.
pub mod cli;
/// Configuration and state management.
pub mod config;
/// Python environment and subprocess management.
pub mod python;
/// Shell command generation and execution.
pub mod shell;
/// Terminal user interface rendering.
pub mod tui;
