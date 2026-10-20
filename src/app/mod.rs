/// Command runners for application modes (chat, shell).
pub mod runner;

use crate::auth::authenticate_with_gemini;
use crate::auth::import::run_manual_import;
use crate::cli::Cli;
use crate::config::save_cookies;
use crate::config::{
    cookies_path, load_cookies, load_metadata, metadata_path, remove_cookies, save_metadata,
    CookieSet, Metadata,
};
use crate::python_env::ensure_venv;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Main application state and configuration.
pub struct App {
    /// Python executable path within managed venv.
    pub python_bin: PathBuf,
    /// Authenticated session cookies for Gemini API.
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
    /// - The authentication process fails.
    /// - Saving the updated metadata fails.
    pub fn initialize(cli: &Cli) -> Result<Self> {
        if cli.import_cookies {
            run_manual_import()?;
            std::process::exit(0);
        }

        let current_version = env!("CARGO_PKG_VERSION");
        let meta_path = metadata_path()?;

        let mut metadata = load_metadata(&meta_path).unwrap_or_default();

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

    /// Runs the main application logic, delegating to the appropriate command runner.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying command runner fails.
    pub async fn run(&self, cli: &Cli) -> Result<()> {
        runner::run(cli, self).await
    }
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
    let is_stale = metadata.is_stale(current_time, DEPS_UPDATE_INTERVAL);
    if is_stale {
        let hours = current_time.saturating_sub(metadata.last_update) / 3600;
        println!("Dependencies are {hours} hours old, checking for updates...");
    }
    is_stale
}

const DEPS_UPDATE_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

#[allow(clippy::print_stdout)]
fn ensure_cookies(cli: &Cli) -> Result<CookieSet> {
    let path = cookies_path()?;
    let timeout = cli.effective_auth_timeout();

    if cli.reauth {
        remove_cookies(&path)?;
        return login_and_save_cookies(&path, timeout);
    }

    match load_cookies(&path) {
        Ok(cookies) => Ok(cookies),
        Err(e) => {
            let is_not_found = e
                .downcast_ref::<std::io::Error>()
                .is_some_and(|io_err| io_err.kind() == std::io::ErrorKind::NotFound);

            if is_not_found {
                println!("No existing session found.");
            } else {
                println!(
                    "Existing cookies are corrupted or unreadable ({e}). Re-authenticating..."
                );
            }
            login_and_save_cookies(&path, timeout)
        }
    }
}

/// Authenticates via webview login or returns manual import instruction on Windows.
#[allow(clippy::print_stdout)]
fn login_and_save_cookies(path: &Path, timeout: Duration) -> Result<CookieSet> {
    if cfg!(target_os = "windows") {
        return Err(anyhow::anyhow!(
            "Automatic cookie extraction via WebView2 is currently unreliable on Windows \
             due to Google's Device Bound Session Credentials (DBSC). \n\n\
             Please run the following command to import cookies manually: \n\
             q --import-cookies"
        ));
    }

    println!("Opening Gemini login page in webview...");
    println!("Sign in, then wait ~5-10 seconds after successful login.");
    println!("The window will close automatically once cookies are captured.");

    let cookies = authenticate_with_gemini(timeout).context("Webview authentication failed")?;
    save_cookies(path, &cookies).context("Failed to save cookies to disk")?;

    println!("Login successful, cookies saved to {}", path.display());
    Ok(cookies)
}

/// Updates the application metadata file.
fn update_metadata(
    path: &Path,
    metadata: &mut Metadata,
    current_version: &str,
    current_time: u64,
) -> Result<()> {
    metadata.last_update = current_time;
    current_version.clone_into(&mut metadata.last_version);
    save_metadata(path, metadata).context("Failed to update metadata file")
}
