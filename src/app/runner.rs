use anyhow::{bail, Result};

use crate::cli::Cli;
use crate::commands::{chat, shell};

use super::App;

/// Runs the main application logic based on the provided CLI arguments.
///
/// # Errors
///
/// Returns an error if:
/// - The query is empty (defensive check, normally prevented by `clap`).
/// - The chat mode fails to process the query or stream the response.
/// - The command mode fails to generate a valid shell command after multiple attempts.
pub async fn run(cli: &Cli, app: &App) -> Result<()> {
    // Defensive check: `clap` normally prevents this via `required_unless_present = "help"`,
    // but we handle it explicitly for direct library usage or future CLI changes.
    if cli.query.is_empty() {
        bail!("No query provided. Usage: q \"your question\"\nRun `q --help` for options.");
    }

    // Joining is O(N) and allocates a new String, which is required since
    // downstream functions expect a contiguous `&str` for the Python subprocess.
    let query = cli.query.join(" ");

    if cli.command_mode {
        let cmd_options = shell::CommandOptions {
            python_bin: &app.python_bin,
            cookies: &app.cookies,
            query: &query,
            model: &cli.model,
            debug: cli.debug,
        };
        shell::run(&cmd_options).await?;
    } else {
        chat::run(cli, &app.python_bin, &app.cookies, &query).await?;
    }

    Ok(())
}
