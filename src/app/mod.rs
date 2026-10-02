/// Contains the command runners for the application's different modes (e.g., chat, shell).
pub mod runner;

use anyhow::Result;
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

        let force_rebuild = should_force_rebuild(cli, &metadata, current_version);
        let should_update_deps = should_update_dependencies(&metadata);

        let python_bin = ensure_venv(force_rebuild, should_update_deps)?;
        let cookies = ensure_cookies(cli)?;

        update_metadata(&meta_path, &mut metadata, current_version)?;

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
            "📦 Version changed from {} to {}, rebuilding venv...",
            metadata.last_version, current_version
        );
    }
    cli.rebuild_venv || version_changed
}

#[allow(clippy::print_stdout)]
fn should_update_dependencies(metadata: &Metadata) -> bool {
    let current_time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days_since_update = (current_time - metadata.last_update) / (24 * 60 * 60);
    if days_since_update >= 7 {
        println!("Dependencies are {days_since_update} days old, checking for updates...");
    }
    days_since_update >= 7
}

#[allow(clippy::print_stdout)]
fn ensure_cookies(cli: &Cli) -> Result<CookieSet> {
    let path = cookies_path()?;
    if cli.login || load_cookies(&path).is_err() {
        println!("Opening Gemini login page in webview...");
        println!("Sign in, then wait ~5-10 seconds after successful login.");
        println!("The window will close automatically once cookies are captured.");
        let cookies = authenticate_with_gemini()?;
        save_cookies(&path, &cookies)?;
        println!("Login successful, cookies saved to {}", path.display());
        Ok(cookies)
    } else {
        load_cookies(&path)
    }
}

fn update_metadata(path: &Path, metadata: &mut Metadata, current_version: &str) -> Result<()> {
    let current_time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    metadata.last_update = current_time;
    current_version.clone_into(&mut metadata.last_version);
    save_metadata(path, metadata)
}
