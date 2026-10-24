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
    let mut login = Login {
        executable: executable.clone(),
        profile: profile.clone(),
        pid: None,
    };

    let session = sign_in(&executable, &profile, timeout, &mut login.pid)?;

    if let Some(pid) = login.pid {
        close_browser(pid);
    }
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
    pid: Option<u32>,
}

impl Drop for Login {
    fn drop(&mut self) {
        if let Some(pid) = self.pid {
            close_browser(pid);
        }
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
fn sign_in(
    executable: &Path,
    profile: &Path,
    timeout: Duration,
    pid_out: &mut Option<u32>,
) -> Result<ProfileSession> {
    println!("Opening the Gemini login page in the portable Firefox...");
    let pid = launch(executable, profile)?;
    *pid_out = Some(pid);

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

        if is_running(pid) {
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
fn launch(executable: &Path, profile: &Path) -> Result<u32> {
    let child = Command::new(executable)
        .arg("-profile")
        .arg(profile)
        .arg("-no-remote")
        .arg(LOGIN_URL)
        .spawn()
        .with_context(|| format!("cannot start {}", executable.display()))?;

    Ok(child.id())
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
fn close_browser(pid: u32) {
    let deadline = Instant::now() + SHUTDOWN_TIMEOUT;

    loop {
        // Kill only the process tree that belongs to this login, by PID, so
        // the Firefox of the user is never reached.
        let _ = Command::new("taskkill")
            .arg("/PID")
            .arg(pid.to_string())
            .arg("/T")
            .output();

        let until = Instant::now() + CLOSE_RETRY_INTERVAL;
        while Instant::now() < until && is_running(pid) {
            std::thread::sleep(Duration::from_millis(250));
        }

        if !is_running(pid) {
            return;
        }

        if Instant::now() >= deadline {
            return;
        }
    }
}

/// Reports whether a process with the given PID is still running.
fn is_running(pid: u32) -> bool {
    let Ok(output) = Command::new("tasklist")
        .arg("/FI")
        .arg(format!("PID eq {pid}"))
        .arg("/NH")
        .arg("/FO")
        .arg("CSV")
        .output()
    else {
        return false;
    };

    let listing = String::from_utf8_lossy(&output.stdout);
    listing.lines().any(|line| {
        line.split(',')
            .next()
            .is_some_and(|field| field.trim_matches('"') == pid.to_string())
    })
}
#[cfg(test)]
#[allow(clippy::result_large_err)]
mod tests {
    use super::LOGIN_URL;

    #[test]
    fn signs_in_through_the_gemini_page() {
        assert_eq!(LOGIN_URL, "https://gemini.google.com/app");
    }
}
