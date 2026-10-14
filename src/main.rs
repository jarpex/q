//! CLI tool for quick, one-shot Gemini queries with no API key required
//!
//! This binary provides a command-line interface to interact with Google Gemini
//! using reverse-engineered web API. It supports both streaming and batch modes,
//! command generation, and automatic authentication via webview.

use std::process::ExitCode;

use clap::Parser;

use q::cli::Cli;
use q::{
    app::App,
    config::{config_path, load_settings},
    tui::print_error,
};

#[tokio::main]
#[allow(clippy::print_stdout)]
async fn main() -> ExitCode {
    let mut cli = Cli::parse();

    let config_path = match config_path() {
        Ok(path) => path,
        Err(error) => {
            print_error(&error.to_string());
            return ExitCode::FAILURE;
        }
    };
    match load_settings(&config_path) {
        Ok(settings) => {
            if cli.model.is_none() {
                cli.model = settings.model;
            }
            if !cli.no_stream {
                cli.no_stream = settings.no_stream.unwrap_or(false);
            }
        }
        Err(error) => {
            print_error(&error.to_string());
            return ExitCode::FAILURE;
        }
    }

    let app = match App::initialize(&cli) {
        Ok(app) => app,
        Err(error) => {
            print_error(&error.to_string());
            return ExitCode::FAILURE;
        }
    };

    match app.run(&cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            print_error(&error.to_string());
            ExitCode::FAILURE
        }
    }
}
