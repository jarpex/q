//! CLI tool for quick, one-shot Gemini queries with no API key required
//!
//! This binary provides a command-line interface to interact with Google Gemini
//! using reverse-engineered web API. It supports both streaming and batch modes,
//! command generation, and automatic authentication via webview.

use anyhow::Result;
use clap::Parser;

use q::app::App;
use q::cli::Cli;

#[tokio::main]
#[allow(clippy::print_stdout)]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let app = App::initialize(&cli)?;
    app.run(&cli).await?;

    Ok(())
}
