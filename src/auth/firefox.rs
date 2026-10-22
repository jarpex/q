//! Signs in to Gemini through a portable Firefox that the tool provisions.
//!
//! The browser runs under a name of its own, in a profile of its own, and it
//! is closed as soon as the cookies of the sign-in are visible. The Firefox
//! of the user therefore stays untouched, and the session of the user cannot
//! rotate the cookies that are about to be read.
//!
//! The order of the flow is fixed: the login is detected, the browser is
//! closed, the final cookies are read from the closed profile, and only then
//! the caller checks the session against Gemini.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use crate::auth::browser::{self, Firefox};
use crate::auth::session::{self, ProfileSession};
use crate::config::CookieSet;

/// Page the user signs in through.
const LOGIN_URL: &str = "https://gemini.google.com/app";

/// Delay between two reads of the profile.
const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Time the browser may take to write its session store and to exit.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(30);

/// Delay between two attempts to close the browser.
const CLOSE_RETRY_INTERVAL: Duration = Duration::from_secs(3);

/// Authenticates with Gemini through the portable Firefox of the tool.
///
/// Downloads and unpacks the browser on the first run, opens the login page
/// in it, waits for the sign-in, closes the browser and reads the cookies of
/// the closed profile.
///
/// # Errors
///
/// Returns an error if:
/// - The browser cannot be downloaded, verified or unpacked.
/// - The browser cannot be started.
/// - The sign-in does not happen before the timeout.
/// - The profile does not hold a complete Gemini session.
#[allow(clippy::print_stdout)]
pub fn authenticate_with_firefox(timeout: Duration) -> Result<CookieSet> {
    let firefox = prepare_browser()?;
    let _ = browser::remove_stale_profiles();
    browser::clean_traces();
    let profile = browser::create_profile().context("cannot create the Firefox profile")?;
    let executable = firefox
        .unique_executable()
        .context("cannot copy the Firefox executable")?;

    // The browser and the profile are removed however the login ends, so
    // that a failed attempt leaves neither a running browser nor the cookies
    // of the sign-in behind.
    let login = Login {
        executable: executable.clone(),
        profile: profile.clone(),
    };

    let session = sign_in(&executable, &profile, timeout)?;

    close_browser(&executable);
    let after_close = session::read_profile_session(&profile).ok();
    let merged = session::merge(&session, after_close.as_ref());

    browser::clean_traces();
    drop(login);

    merged
        .to_cookie_set()
        .context("the Firefox profile does not hold a complete Gemini session")
}

/// The browser and the profile of one login attempt.
///
/// The profile holds the session of the sign-in, so it is removed when the
/// attempt ends, no matter how it ends.
struct Login {
    executable: PathBuf,
    profile: PathBuf,
}

impl Drop for Login {
    fn drop(&mut self) {
        close_browser(&self.executable);
        let _ = std::fs::remove_file(&self.executable);
        let _ = browser::remove_profile(&self.profile);
        browser::clean_traces();
    }
}

/// Makes sure that the portable Firefox is ready to run.
#[allow(clippy::print_stdout)]
fn prepare_browser() -> Result<Firefox> {
    if !Firefox::is_installed_at_default_location()? {
        println!(
            "Downloading Firefox {} for the sign-in. This happens once.",
            browser::FIREFOX_VERSION
        );
    }

    Firefox::ensure_installed().context("cannot prepare the portable Firefox")
}
/// Opens the login page and waits for the sign-in to complete.
#[allow(clippy::print_stdout)]
fn sign_in(executable: &Path, profile: &Path, timeout: Duration) -> Result<ProfileSession> {
    println!("Opening the Gemini login page in the portable Firefox...");
    launch(executable, profile)?;

    println!("Sign in to Gemini in the Firefox window that opens.");
    println!("The window closes on its own once the session has been read.");
    let deadline = Instant::now() + timeout;
    let mut started = false;
    let mut last_error = None;

    loop {
        match session::read_profile_session(profile) {
            Ok(found) if found.is_complete() => return Ok(found),
            Ok(_) => {}
            Err(error) => last_error = Some(error),
        }

        if is_running(executable) {
            started = true;
        } else if started {
            bail!("the Firefox window was closed before the sign-in completed");
        }

        if Instant::now() >= deadline {
            bail!(
                "timed out after {} seconds waiting for the Gemini cookies. {}",
                timeout.as_secs(),
                last_error
                    .map(|error| format!("The last attempt to read the profile failed: {error}"))
                    .unwrap_or_default()
            );
        }

        std::thread::sleep(POLL_INTERVAL);
    }
}

/// Starts the browser with the profile of this login.
fn launch(executable: &Path, profile: &Path) -> Result<()> {
    Command::new(executable)
        .arg("-profile")
        .arg(profile)
        .arg("-no-remote")
        .arg(LOGIN_URL)
        .spawn()
        .with_context(|| format!("cannot start {}", executable.display()))?;

    Ok(())
}

/// Closes the browser of this login and waits for it to exit.
///
/// The browser is closed without force, so that it writes its session store
/// on the way out. Only the processes that run under the name of this login
/// are closed, so the Firefox of the user stays open.
///
/// The close is repeated while processes are left: Firefox restarts itself
/// when the launcher is closed too early, and a second close catches the
/// browser that came back.
fn close_browser(executable: &Path) {
    let Some(image) = image_name(executable) else {
        return;
    };

    let deadline = Instant::now() + SHUTDOWN_TIMEOUT;

    loop {
        let _ = Command::new("taskkill").arg("/IM").arg(&image).output();

        let until = Instant::now() + CLOSE_RETRY_INTERVAL;
        while Instant::now() < until && is_running(executable) {
            std::thread::sleep(Duration::from_millis(250));
        }

        if !is_running(executable) {
            return;
        }

        if Instant::now() >= deadline {
            return;
        }
    }
}

/// Reports whether a process of this login is still running.
fn is_running(executable: &Path) -> bool {
    let Some(image) = image_name(executable) else {
        return false;
    };

    let Ok(output) = Command::new("tasklist")
        .arg("/FI")
        .arg(format!("IMAGENAME eq {image}"))
        .arg("/NH")
        .arg("/FO")
        .arg("CSV")
        .output()
    else {
        return false;
    };

    let listing = String::from_utf8_lossy(&output.stdout);
    listing
        .lines()
        .any(|line| line.starts_with(&format!("\"{image}\"")))
}

/// Returns the name of the image the browser runs under.
fn image_name(executable: &Path) -> Option<String> {
    executable
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
}

#[cfg(test)]
#[allow(clippy::result_large_err)]
mod tests {
    use super::{image_name, LOGIN_URL};
    use std::path::Path;

    #[test]
    fn signs_in_through_the_gemini_page() {
        assert_eq!(LOGIN_URL, "https://gemini.google.com/app");
    }

    #[test]
    fn the_image_is_the_name_of_the_executable() {
        let name = image_name(Path::new("C:/q/core/q-firefox-123.exe"));

        assert_eq!(name.as_deref(), Some("q-firefox-123.exe"));
    }

    #[test]
    fn a_path_without_a_name_has_no_image() {
        assert!(image_name(Path::new("C:/q/core/")).is_some());
        assert!(image_name(Path::new("")).is_none());
    }
}
