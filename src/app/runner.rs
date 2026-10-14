use anyhow::{bail, Result};

use crate::cli::Cli;
use crate::commands::{chat, shell};
use crate::gemini::{list_models, usage_limits};
use crate::tui::print_usage_limits;

use super::App;

/// Runs the main application logic based on the provided CLI arguments.
///
/// # Errors
///
/// Returns an error if:
/// - The query is empty (defensive check, normally prevented by `clap`).
/// - The chat mode fails to process the query or stream the response.
/// - The command mode fails to generate a valid shell command after multiple attempts.
#[allow(clippy::print_stdout)]
pub async fn run(cli: &Cli, app: &App) -> Result<()> {
    if cli.list_models {
        let models = list_models(&app.python_bin, &app.cookies, cli.debug).await?;
        println!("Available models: {}", models.join(", "));
        return Ok(());
    }

    if cli.usage_limits {
        let limits = usage_limits(&app.python_bin, &app.cookies, cli.debug).await?;
        print_usage_limits(&limits.usage_info, &limits.quotas);
        return Ok(());
    }

    // Action-only flags initialize the application and then exit without running a query.
    if cli.query.is_empty() {
        if cli.reauth || cli.rebuild_venv || cli.usage_limits {
            return Ok(());
        }

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
            model: cli.effective_model(),
            debug: cli.debug,
        };
        shell::run(&cmd_options).await?;
    } else {
        chat::run(cli, &app.python_bin, &app.cookies, &query).await?;
    }

    Ok(())
}
