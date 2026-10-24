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
use crate::gemini::runner::verify_session;
use crate::python_env::ensure_venv;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Main application state and configuration.
pub struct App {
    /// Python executable path within managed venv.
    ///
    /// The providers with a token do not need the Python environment, so the
    /// path is empty for them.
    pub python_bin: Option<PathBuf>,
    /// Authenticated session cookies for Gemini API.
    ///
    /// The providers with a token do not need the cookies, so the set is empty
    /// for them.
    pub cookies: Option<CookieSet>,
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

        // The providers with a token talk to their platform over the network,
        // so they need neither the Python environment nor the cookies of the
        // account of the user.
        if !needs_the_account(cli) {
            return Ok(Self {
                python_bin: None,
                cookies: None,
            });
        }

        let current_version = env!("CARGO_PKG_VERSION");
        let meta_path = metadata_path()?;

        let _lock = acquire_metadata_lock(&meta_path)?;

        let mut metadata = load_metadata(&meta_path).unwrap_or_default();

        let current_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let startup = plan_startup(cli, &metadata, current_version, current_time);

        let python_bin = ensure_venv(startup.rebuild, startup.update_dependencies)?;
        let cookies = ensure_cookies(cli, &python_bin)?;

        update_metadata(&meta_path, &mut metadata, current_version, current_time)?;

        Ok(Self {
            python_bin: Some(python_bin),
            cookies: Some(cookies),
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

/// Tells whether the run needs the cookies and the Python environment.
///
/// The default provider works through the cookies of the account. The providers
/// with a token work on their own, and the screen of the tokens needs
/// nothing of it.
fn needs_the_account(cli: &Cli) -> bool {
    !cli.api_management && cli.effective_provider() == crate::provider::GOOGLE_COOKIES
}

/// What the application does with the Python environment before a run.
#[derive(Debug, PartialEq, Eq)]
struct Startup {
    /// The virtual environment must be recreated.
    rebuild: bool,
    /// The dependencies must be refreshed.
    update_dependencies: bool,
}

/// Decides what to do with the Python environment.
///
/// A first run records no version and no timestamp, so it neither reports a
/// version change nor an age of the dependencies.
#[allow(clippy::print_stdout)]
fn plan_startup(
    cli: &Cli,
    metadata: &Metadata,
    current_version: &str,
    current_time: u64,
) -> Startup {
    let first_run = !metadata.has_previous_version();
    let version_changed = metadata.last_version != current_version;
    let rebuild = cli.rebuild_venv || version_changed;

    if first_run {
        println!("First run: setting up the Python environment...");
    } else if version_changed {
        println!(
            "Version changed from {} to {}, rebuilding venv...",
            metadata.last_version, current_version
        );
    }

    let update_dependencies = !rebuild && metadata.is_stale(current_time, DEPS_UPDATE_INTERVAL);
    if update_dependencies {
        if let Some(hours) = metadata.dependency_age_hours(current_time) {
            println!("Dependencies are {hours} hours old, checking for updates...");
        } else {
            println!("Checking for dependency updates...");
        }
    }

    Startup {
        rebuild,
        update_dependencies,
    }
}

const DEPS_UPDATE_INTERVAL: Duration = Duration::from_hours(24);

#[allow(clippy::print_stdout)]
fn ensure_cookies(cli: &Cli, python_bin: &Path) -> Result<CookieSet> {
    let path = cookies_path()?;
    let timeout = cli.effective_auth_timeout();

    if cli.reauth {
        remove_cookies(&path)?;
        return login_and_save_cookies(&path, timeout, python_bin);
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
            login_and_save_cookies(&path, timeout, python_bin)
        }
    }
}

/// Signs in with the browser of the platform and saves the cookies.
#[allow(clippy::print_stdout)]
fn login_and_save_cookies(path: &Path, timeout: Duration, python_bin: &Path) -> Result<CookieSet> {
    if !cfg!(target_os = "windows") {
        println!("Opening Gemini login page in webview...");
        println!("Sign in, then wait ~5-10 seconds after successful login.");
        println!("The window will close automatically once cookies are captured.");
    }

    let cookies = if cfg!(target_os = "windows") {
        authenticate_with_gemini(timeout).context("The sign-in through Firefox failed")?
    } else {
        authenticate_with_gemini(timeout).context("The sign-in through the webview failed")?
    };

    // The cookies are only worth saving if Gemini accepts them. A real request
    // proves that the sign-in worked, instead of a browser that merely exited.
    verify_session(python_bin, &cookies)
        .context("The captured session was not accepted by Gemini")?;

    save_cookies(path, &cookies).context("Failed to save cookies to disk")?;

    println!("Login successful, cookies saved to {}", path.display());
    Ok(cookies)
}

/// Acquires an exclusive lock on the metadata file so that concurrent
/// instances of the application do not read and write it at the same time.
///
/// The lock is a separate file next to the metadata file. It is created with
/// `O_EXCL`, which is atomic on every supported platform, so two instances
/// cannot both believe they hold it. The guard removes the lock file on drop;
/// a stale lock left by a crashed process is removed after a short timeout.
fn acquire_metadata_lock(path: &Path) -> Result<MetadataLock> {
    const STALE_AFTER: Duration = Duration::from_secs(60);
    const RETRY_DELAY: Duration = Duration::from_millis(50);
    const MAX_ATTEMPTS: u32 = 200;

    let lock_path = path.with_extension("lock");

    for _ in 0..MAX_ATTEMPTS {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
        {
            Ok(_file) => return Ok(MetadataLock { lock_path }),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                // Remove a stale lock left by a crashed process.
                if let Ok(metadata) = std::fs::metadata(&lock_path) {
                    if let Ok(modified) = metadata.modified() {
                        if modified.elapsed().unwrap_or_default() > STALE_AFTER {
                            let _ = std::fs::remove_file(&lock_path);
                            continue;
                        }
                    }
                }
                std::thread::sleep(RETRY_DELAY);
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("cannot open metadata lock file: {}", lock_path.display())
                });
            }
        }
    }

    Err(anyhow::anyhow!(
        "cannot acquire metadata lock: {}",
        lock_path.display()
    ))
}

/// RAII guard that releases the metadata lock when dropped.
struct MetadataLock {
    lock_path: PathBuf,
}

impl Drop for MetadataLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.lock_path);
    }
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
        assert_eq!(super::DEPS_UPDATE_INTERVAL, Duration::from_hours(24));
    }
}
