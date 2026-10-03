/// Contains the command runners for the application's different modes (e.g., chat, shell).
pub mod runner;

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use crate::auth::authenticate_with_gemini;
use crate::cli::Cli;
use crate::config::{
    cookies_path, load_cookies, load_metadata, metadata_path, save_cookies, save_metadata,
    CookieSet, Metadata,
};
use crate::python_env::ensure_venv;

/// The main application state and configuration, holding paths and authentication details.
pub struct App {
    /// The path to the Python executable within the managed virtual environment.
    pub python_bin: PathBuf,
    /// The authenticated session cookies required to interact with the Gemini API.
    pub cookies: CookieSet,
}

impl App {
    /// Initializes the application by setting up the Python virtual environment,
    /// loading or refreshing authentication cookies, and updating metadata.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The configuration or metadata paths cannot be determined.
    /// - The Python virtual environment fails to initialize or update.
    /// - The webview authentication process fails or is cancelled.
    /// - Saving the updated metadata fails.
    pub fn initialize(cli: &Cli) -> Result<Self> {
        let current_version = env!("CARGO_PKG_VERSION");
        let meta_path = metadata_path()?;
        let mut metadata = load_metadata_or_default(&meta_path);

        let current_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let force_rebuild = should_force_rebuild(cli, &metadata, current_version);
        let should_update_deps = should_update_dependencies(&metadata, current_time);

        let python_bin = ensure_venv(force_rebuild, should_update_deps)?;
        let cookies = ensure_cookies(cli)?;

        update_metadata(&meta_path, &mut metadata, current_version, current_time)?;

        Ok(Self {
            python_bin,
            cookies,
        })
    }

    /// Runs the main application logic, delegating to the appropriate command runner
    /// based on the CLI arguments.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying command runner (chat or shell) fails
    /// to execute or complete its task successfully.
    pub async fn run(&self, cli: &Cli) -> Result<()> {
        runner::run(cli, self).await
    }
}

fn load_metadata_or_default(path: &Path) -> Metadata {
    // NOTE: If implement #[derive(Default)] for Metadata in config/metadata.rs,
    // this function can be replaced with load_metadata(path).unwrap_or_default()
    load_metadata(path).unwrap_or_else(|_| Metadata {
        last_update: 0,
        last_version: String::new(),
    })
}

#[allow(clippy::print_stdout)]
fn should_force_rebuild(cli: &Cli, metadata: &Metadata, current_version: &str) -> bool {
    let version_changed = metadata.last_version != current_version;
    if version_changed {
        println!(
            "Version changed from {} to {}, rebuilding venv...",
            metadata.last_version, current_version
        );
    }
    cli.rebuild_venv || version_changed
}

#[allow(clippy::print_stdout)]
fn should_update_dependencies(metadata: &Metadata, current_time: u64) -> bool {
    let seconds_since_update = current_time.saturating_sub(metadata.last_update);
    let days_since_update = seconds_since_update / (24 * 60 * 60);

    if days_since_update >= 7 {
        println!("Dependencies are {days_since_update} days old, checking for updates...");
    }
    days_since_update >= 7
}

#[allow(clippy::print_stdout)]
fn ensure_cookies(cli: &Cli) -> Result<CookieSet> {
    let path = cookies_path()?;

    if cli.login {
        return login_and_save_cookies(&path);
    }

    match load_cookies(&path) {
        Ok(cookies) => Ok(cookies),
        Err(e) => {
            let is_not_found = e
                .downcast_ref::<std::io::Error>()
                .is_some_and(|io_err| io_err.kind() == std::io::ErrorKind::NotFound);

            if is_not_found {
                println!("No existing session found. Opening Gemini login page...");
            } else {
                println!(
                    "⚠️ Existing cookies are corrupted or unreadable ({e}). Re-authenticating..."
                );
            }
            login_and_save_cookies(&path)
        }
    }
}

#[allow(clippy::print_stdout)]
fn login_and_save_cookies(path: &Path) -> Result<CookieSet> {
    println!("Opening Gemini login page in webview...");
    println!("Sign in, then wait ~5-10 seconds after successful login.");
    println!("The window will close automatically once cookies are captured.");

    let cookies = authenticate_with_gemini().context("Webview authentication failed")?;
    save_cookies(path, &cookies).context("Failed to save cookies to disk")?;

    println!("Login successful, cookies saved to {}", path.display());
    Ok(cookies)
}

fn update_metadata(
    path: &Path,
    metadata: &mut Metadata,
    current_version: &str,
    current_time: u64,
) -> Result<()> {
    metadata.last_update = current_time;

    metadata.last_version.clear();
    metadata.last_version.push_str(current_version);

    save_metadata(path, metadata).context("Failed to update metadata file")
}
