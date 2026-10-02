use anyhow::Result;

use crate::cli::Cli;
use crate::commands::{chat, shell};

use super::App;

/// Runs the main application logic based on the provided CLI arguments.
///
/// # Errors
///
/// Returns an error if:
/// - The chat mode fails to process the query or stream the response.
/// - The command mode fails to generate a valid shell command after multiple attempts.
pub async fn run(cli: &Cli, app: &App) -> Result<()> {
    if cli.query.is_empty() {
        print_usage_and_exit();
        return Ok(());
    }

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

#[allow(clippy::print_stdout)]
fn print_usage_and_exit() {
    println!("No query provided. Usage: q \"your question\"");
    println!("Run `q --help` for options.");
}
