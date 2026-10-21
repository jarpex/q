use anyhow::{bail, Context, Result};

use crate::cli::Cli;
use crate::commands::{chat, shell};
use crate::config::{keys_path, load_keys};
use crate::provider::{
    canonical_provider, token_providers, BoxedProvider, GenaiProvider, GoogleCookiesProvider,
};
use crate::tui::{print_usage_limits, KeyAction, KeyManager};
use crate::uc::Uc;

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
    if cli.api_management {
        return manage_keys();
    }

    let provider = build_provider(cli, app)?;

    if cli.list_models {
        let models = provider.list_models().await?;
        println!("Available models: {}", models.join(", "));
        return Ok(());
    }

    if cli.usage_limits {
        let Some((usage_info, quotas)) = provider.usage_limits().await? else {
            println!(
                "The provider {} does not report the usage.",
                provider.name()
            );
            return Ok(());
        };
        print_usage_limits(&usage_info, &quotas);
        return Ok(());
    }

    // Action-only flags initialize the application and then exit without running a query.
    if cli.query.is_empty() {
        if cli.reauth || cli.rebuild_venv || cli.usage_limits {
            return Ok(());
        }

        bail!("No query provided. Usage: q \"your question\"\nRun `q --help` for options.");
    }

    // The join allocates a new String. Downstream functions need a contiguous
    // `&str` for the Python subprocess.
    let query = cli.query.join(" ");

    // The filter cleans the text of the model before it is shown and before it
    // goes to the clipboard.
    let uc = Uc::new(cli.uc.as_deref(), cli.uc_path.as_deref());

    if cli.command_mode {
        let cmd_options = shell::CommandOptions {
            provider: provider.as_ref(),
            query: &query,
            model: cli.effective_model(),
            debug: cli.debug,
            uc: &uc,
        };
        shell::run(&cmd_options).await?;
    } else {
        let chat_options = chat::ChatOptions {
            provider: provider.as_ref(),
            query: &query,
            model: cli.effective_model(),
            no_stream: cli.no_stream,
            debug: cli.debug,
            uc: &uc,
        };
        chat::run(&chat_options).await?;
    }

    Ok(())
}

/// Builds the provider that the user asked for.
///
/// The default provider works through the cookies of the account. Every other
/// provider needs a token, which the user adds with the flag
/// `--api-management`.
fn build_provider(cli: &Cli, app: &App) -> Result<BoxedProvider> {
    let name = cli.effective_provider();
    if name == crate::provider::GOOGLE_COOKIES {
        let python_bin = app
            .python_bin
            .as_ref()
            .context("The Python environment is missing")?;
        let cookies = app
            .cookies
            .as_ref()
            .context("The cookies of the account are missing")?;
        return Ok(Box::new(GoogleCookiesProvider::new(
            python_bin, cookies, cli.debug,
        )));
    }

    // The name is checked first, so an unknown provider is told apart from a
    // known one that has no token yet.
    let canonical = canonical_provider(name).with_context(|| {
        format!("Unknown provider: {name}. Run with `q --api-management` to see the list of them.")
    })?;

    let path = keys_path()?;
    let store = load_keys(&path).unwrap_or_default();
    let key = store.find(canonical).with_context(|| {
        format!("No token for the provider {canonical}. Add one with `q --api-management`.")
    })?;

    Ok(Box::new(GenaiProvider::new(key)?))
}

/// Opens the screen that manages the API keys and saves the result.
fn manage_keys() -> Result<()> {
    let path = keys_path()?;
    let store = load_keys(&path).unwrap_or_default();
    let mut manager = KeyManager::new(store, token_providers());

    let mut terminal = ratatui::try_init().context("Cannot open the terminal")?;
    let action = manager.run(&mut terminal);
    ratatui::restore();

    match action? {
        KeyAction::Cancel => Ok(()),
        KeyAction::Save(store) => {
            crate::config::save_keys(&path, &store).context("Failed to save the API keys")?;
            Ok(())
        }
    }
}
