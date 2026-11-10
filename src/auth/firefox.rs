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

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use crate::auth::browser::{self, Browser, Firefox};
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
#[allow(clippy::print_stdout, clippy::exit)]
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
    let job = JobGuard::new().context("cannot create the Firefox job object")?;
    let mut login = Login {
        executable: executable.clone(),
        profile: profile.clone(),
        pid: None,
        job,
    };

    let running = Arc::new(AtomicBool::new(true));
    let shared_pid = Arc::new(AtomicU32::new(0));
    let cleanup_running = Arc::clone(&running);
    let cleanup_pid = Arc::clone(&shared_pid);
    let cleanup_exec = executable.clone();
    let cleanup_profile = profile.clone();

    let ctrlc_task = tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() && cleanup_running.load(Ordering::SeqCst) {
            let pid = cleanup_pid.load(Ordering::SeqCst);
            if pid != 0 {
                close_browser(pid);
            }
            let _ = std::fs::remove_file(&cleanup_exec);
            let _ = browser::remove_profile(&cleanup_profile);
            browser::clean_traces();
            std::process::exit(130);
        }
    });

    println!("Opening the Gemini login page in the portable Firefox...");
    let pid = launch(&executable, &profile, &login.job)?;
    shared_pid.store(pid, Ordering::SeqCst);
    login.pid = Some(pid);

    let session_result = sign_in(&profile, timeout, pid);
    running.store(false, Ordering::SeqCst);
    ctrlc_task.abort();
    let session = session_result?;

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
    job: JobGuard,
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

/// Windows Job Object that terminates all associated processes when dropped.
#[derive(Debug)]
struct JobGuard(windows_sys::Win32::Foundation::HANDLE);

impl JobGuard {
    #[allow(unsafe_code)]
    fn new() -> std::io::Result<Self> {
        use windows_sys::Win32::System::JobObjects::{
            CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        };

        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return Err(std::io::Error::last_os_error());
            }

            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;

            let info_size = u32::try_from(std::mem::size_of_val(&info))
                .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidInput, err))?;

            let ok = SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                (&raw const info).cast(),
                info_size,
            );
            if ok == 0 {
                let err = std::io::Error::last_os_error();
                windows_sys::Win32::Foundation::CloseHandle(job);
                return Err(err);
            }

            Ok(Self(job))
        }
    }

    #[allow(unsafe_code)]
    fn assign_process(
        &self,
        process_handle: std::os::windows::io::RawHandle,
    ) -> std::io::Result<()> {
        use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;

        let res = unsafe { AssignProcessToJobObject(self.0, process_handle.cast()) };
        if res == 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

impl Drop for JobGuard {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

/// Makes sure that a browser is ready to run.
///
/// Prefers an existing system Firefox installation to avoid unnecessary downloads
/// and benefit from system security updates. Falls back to the tool's portable Firefox.
#[allow(clippy::print_stdout)]
fn prepare_browser() -> Result<Browser> {
    if let Some(system) = browser::find_system_firefox() {
        return Ok(Browser::System { executable: system });
    }

    if !Firefox::is_installed_at_default_location()? {
        println!(
            "Downloading Firefox {} for the sign-in. This happens once.",
            browser::FIREFOX_VERSION
        );
    }

    let portable = Firefox::ensure_installed().context("cannot prepare the portable Firefox")?;
    Ok(Browser::Portable(portable))
}
/// Waits for the sign-in to complete in the opened browser window.
#[allow(clippy::print_stdout)]
fn sign_in(profile: &Path, timeout: Duration, pid: u32) -> Result<ProfileSession> {
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
fn launch(executable: &Path, profile: &Path, job: &JobGuard) -> Result<u32> {
    use std::os::windows::io::AsRawHandle;

    let child = Command::new(executable)
        .arg("-profile")
        .arg(profile)
        .arg("-no-remote")
        .arg(LOGIN_URL)
        .spawn()
        .with_context(|| format!("cannot start {}", executable.display()))?;

    let pid = child.id();
    job.assign_process(child.as_raw_handle())
        .context("cannot assign Firefox process to Windows job object")?;

    Ok(pid)
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
        let _ = browser::system32_cmd("taskkill.exe")
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
    let Ok(output) = browser::system32_cmd("tasklist.exe")
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
    tasklist_contains_pid(&listing, pid)
}

fn tasklist_contains_pid(listing: &str, pid: u32) -> bool {
    let pid_str = pid.to_string();
    listing.lines().any(|line| {
        line.split(',')
            .nth(1)
            .is_some_and(|field| field.trim_matches('"') == pid_str)
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

    #[test]
    fn tasklist_contains_pid_matches_second_column() {
        let sample = "\"firefox.exe\",\"14032\",\"Console\",\"1\",\"8,924 K\"\n";
        assert!(super::tasklist_contains_pid(sample, 14032));
        assert!(!super::tasklist_contains_pid(sample, 9999));
    }

    #[test]
    fn tasklist_contains_pid_handles_empty_or_info_output() {
        let sample = "INFO: No tasks are running which match the specified criteria.\n";
        assert!(!super::tasklist_contains_pid(sample, 14032));
    }
}
