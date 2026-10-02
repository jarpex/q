//! Library interface for the q tool.
//!
//! This module exposes internal functionality for testing and fuzzing.

/// Application orchestration and runner.
pub mod app;
/// Authentication via webview.
pub mod auth;
/// Command-line interface parsing and execution.
pub mod cli;
/// Command modes (chat, shell).
pub mod commands;
/// Configuration and state management.
pub mod config;
/// Gemini API interaction via Python subprocess.
pub mod gemini;
/// Python environment and virtual environment management.
pub mod python_env;
/// Terminal user interface rendering.
pub mod tui;
