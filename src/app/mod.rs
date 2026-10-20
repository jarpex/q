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

        let startup = plan_startup(cli, &metadata, current_version, current_time);
        report_startup(&startup);

        let python_bin = ensure_venv(startup.rebuild, startup.update_dependencies)?;
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

/// What the application does with the Python environment before a run.
#[derive(Debug, PartialEq, Eq)]
struct Startup {
    /// The virtual environment must be recreated.
    rebuild: bool,
    /// The dependencies must be refreshed.
    update_dependencies: bool,
    /// The lines that explain the plan to the user.
    messages: Vec<String>,
}

/// Decides what to do with the Python environment.
///
/// A first run records no version and no timestamp, so it neither reports a
/// version change nor an age of the dependencies.
fn plan_startup(
    cli: &Cli,
    metadata: &Metadata,
    current_version: &str,
    current_time: u64,
) -> Startup {
    let first_run = !metadata.has_previous_version();
    let version_changed = metadata.last_version != current_version;
    let rebuild = cli.rebuild_venv || version_changed;

    let mut messages = Vec::new();
    if first_run {
        messages.push(String::from(
            "First run: setting up the Python environment...",
        ));
    } else if version_changed {
        messages.push(format!(
            "Version changed from {} to {}, rebuilding venv...",
            metadata.last_version, current_version
        ));
    }

    let update_dependencies = !rebuild && metadata.is_stale(current_time, DEPS_UPDATE_INTERVAL);
    if update_dependencies {
        messages.push(metadata.dependency_age_hours(current_time).map_or_else(
            || String::from("Checking for dependency updates..."),
            |hours| format!("Dependencies are {hours} hours old, checking for updates..."),
        ));
    }
    Startup {
        rebuild,
        update_dependencies,
        messages,
    }
}

/// Tells the user what the application is about to do.
#[allow(clippy::print_stdout)]
fn report_startup(startup: &Startup) {
    for message in &startup.messages {
        println!("{message}");
    }
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

#[cfg(test)]
#[allow(clippy::result_large_err)]
mod tests {
    use super::{plan_startup, update_metadata};
    use crate::cli::Cli;
    use crate::config::Metadata;
    use crate::test_support::TempDir;
    use clap::Parser;
    use std::time::Duration;

    fn cli(args: &[&str]) -> Cli {
        let parsed = Cli::try_parse_from(args);
        assert!(parsed.is_ok(), "cannot parse {args:?}");
        let Ok(cli) = parsed else {
            return cli(&["q", "query"]);
        };
        cli
    }

    fn metadata(version: &str, updated_at: u64) -> Metadata {
        Metadata {
            last_update: updated_at,
            last_version: version.to_owned(),
        }
    }

    /// The metadata of a run that never happened.
    fn default_metadata() -> Metadata {
        Metadata::default()
    }

    #[test]
    fn rebuilds_when_the_version_changed() {
        let cli = cli(&["q", "query"]);

        let startup = plan_startup(&cli, &metadata("0.1.0", 1), "0.2.0", 1_700_000_000);

        assert!(startup.rebuild);
    }

    #[test]
    fn does_not_rebuild_when_the_version_is_the_same() {
        let cli = cli(&["q", "query"]);

        let startup = plan_startup(&cli, &metadata("0.2.0", 1), "0.2.0", 1_700_000_000);

        assert!(!startup.rebuild);
    }

    #[test]
    fn rebuilds_when_the_flag_is_given() {
        let cli = cli(&["q", "--rebuild-venv", "query"]);

        let startup = plan_startup(&cli, &metadata("0.2.0", 1), "0.2.0", 1_700_000_000);

        assert!(startup.rebuild);
    }

    #[test]
    fn updates_the_dependencies_when_they_are_old() {
        let now = 1_700_000_000;

        let startup = plan_startup(
            &cli(&["q", "query"]),
            &metadata("0.2.0", now - 100_000),
            "0.2.0",
            now,
        );

        assert!(startup.update_dependencies);
    }

    #[test]
    fn keeps_the_dependencies_when_they_are_fresh() {
        let now = 1_700_000_000;

        let startup = plan_startup(&cli(&["q", "query"]), &metadata("0.2.0", now), "0.2.0", now);

        assert!(!startup.update_dependencies);
    }

    #[test]
    fn reports_a_version_change() {
        let cli = cli(&["q", "query"]);

        let startup = plan_startup(&cli, &metadata("0.1.0", 1), "0.2.0", 1_700_000_000);

        assert_eq!(
            startup.messages,
            ["Version changed from 0.1.0 to 0.2.0, rebuilding venv..."]
        );
    }

    #[test]
    fn reports_the_age_of_the_dependencies() {
        let now = 1_700_000_000;

        let startup = plan_startup(
            &cli(&["q", "query"]),
            &metadata("0.2.0", now - 100_000),
            "0.2.0",
            now,
        );

        assert_eq!(
            startup.messages,
            ["Dependencies are 27 hours old, checking for updates..."]
        );
    }

    #[test]
    fn says_nothing_when_everything_is_current() {
        let now = 1_700_000_000;

        let startup = plan_startup(&cli(&["q", "query"]), &metadata("0.2.0", now), "0.2.0", now);

        assert_eq!(startup.messages, Vec::<String>::new());
    }

    #[test]
    fn does_not_report_a_version_change_on_the_first_run() {
        let cli = cli(&["q", "query"]);

        let startup = plan_startup(&cli, &default_metadata(), "0.2.0", 1_700_000_000);

        assert!(
            !startup
                .messages
                .iter()
                .any(|m| m.contains("Version changed")),
            "{:?}",
            startup.messages
        );
    }

    #[test]
    fn does_not_report_the_age_of_the_dependencies_on_the_first_run() {
        let cli = cli(&["q", "query"]);

        let startup = plan_startup(&cli, &default_metadata(), "0.2.0", 1_700_000_000);

        assert!(
            !startup.messages.iter().any(|m| m.contains("hours old")),
            "{:?}",
            startup.messages
        );
    }

    #[test]
    fn does_not_update_the_dependencies_on_the_first_run() {
        let cli = cli(&["q", "query"]);

        let startup = plan_startup(&cli, &default_metadata(), "0.2.0", 1_700_000_000);

        assert!(!startup.update_dependencies);
    }

    #[test]
    fn announces_the_first_run() {
        let cli = cli(&["q", "query"]);

        let startup = plan_startup(&cli, &default_metadata(), "0.2.0", 1_700_000_000);

        assert_eq!(
            startup.messages,
            ["First run: setting up the Python environment..."]
        );
    }

    #[test]
    fn writes_the_version_and_the_time() -> anyhow::Result<()> {
        let dir = TempDir::new("app")?;
        let path = dir.path().join("metadata.json");
        let mut meta = metadata("0.1.0", 0);

        update_metadata(&path, &mut meta, "0.2.0", 4242)?;

        assert_eq!(meta.last_version, "0.2.0");
        assert_eq!(meta.last_update, 4242);
        let saved = std::fs::read_to_string(&path)?;
        assert!(saved.contains("0.2.0"), "{saved}");
        assert!(saved.contains("4242"), "{saved}");
        Ok(())
    }

    #[test]
    fn the_update_interval_is_a_day() {
        assert_eq!(super::DEPS_UPDATE_INTERVAL, Duration::from_secs(86_400));
    }
}
