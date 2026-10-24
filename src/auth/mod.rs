//! Authentication with Gemini.
//!
//! On Windows the login runs in a portable Firefox that the tool provisions
//! itself: the official installer of a pinned release is downloaded, checked
//! against a pinned digest and unpacked, so the browser of the user is never
//! used and no extra program has to be installed. Google refuses to sign in
//! to an automated browser, so the user signs in by hand in a real Firefox.
//!
//! On the other platforms the login runs in a webview.

pub mod import;

#[cfg(target_os = "windows")]
pub mod browser;
#[cfg(target_os = "windows")]
pub mod firefox;
#[cfg(target_os = "windows")]
pub mod session;

#[cfg(all(test, not(target_os = "windows")))]
#[allow(clippy::result_large_err)]
mod tests {
    use super::find_session_cookies;

    #[test]
    fn finds_both_session_cookies() {
        let cookies = vec![
            ("__Secure-1PSID".to_owned(), "PSID-VALUE".to_owned()),
            ("__Secure-1PSIDTS".to_owned(), "PSIDTS-VALUE".to_owned()),
        ];

        let (psid, psidts) = find_session_cookies(&cookies);

        assert_eq!(psid.as_deref(), Some("PSID-VALUE"));
        assert_eq!(psidts.as_deref(), Some("PSIDTS-VALUE"));
    }

    #[test]
    fn ignores_unrelated_cookies() {
        let cookies = vec![
            ("other".to_owned(), "x".to_owned()),
            ("__Secure-1PSID".to_owned(), "PSID-VALUE".to_owned()),
            ("__Secure-1PSIDTS".to_owned(), "PSIDTS-VALUE".to_owned()),
            ("session".to_owned(), "y".to_owned()),
        ];

        let (psid, psidts) = find_session_cookies(&cookies);

        assert_eq!(psid.as_deref(), Some("PSID-VALUE"));
        assert_eq!(psidts.as_deref(), Some("PSIDTS-VALUE"));
    }

    #[test]
    fn returns_none_when_psid_is_missing() {
        let cookies = vec![("__Secure-1PSIDTS".to_owned(), "PSIDTS-VALUE".to_owned())];

        let (psid, psidts) = find_session_cookies(&cookies);

        assert!(psid.is_none());
        assert_eq!(psidts.as_deref(), Some("PSIDTS-VALUE"));
    }

    #[test]
    fn returns_none_when_psidts_is_missing() {
        let cookies = vec![("__Secure-1PSID".to_owned(), "PSID-VALUE".to_owned())];

        let (psid, psidts) = find_session_cookies(&cookies);

        assert_eq!(psid.as_deref(), Some("PSID-VALUE"));
        assert!(psidts.is_none());
    }

    #[test]
    fn returns_none_for_both_when_no_cookies() {
        let (psid, psidts) = find_session_cookies(&[]);

        assert!(psid.is_none());
        assert!(psidts.is_none());
    }

    #[test]
    fn returns_none_for_both_when_only_unrelated_cookies() {
        let cookies = vec![
            ("foo".to_owned(), "bar".to_owned()),
            ("baz".to_owned(), "qux".to_owned()),
        ];

        let (psid, psidts) = find_session_cookies(&cookies);

        assert!(psid.is_none());
        assert!(psidts.is_none());
    }

    #[test]
    fn keeps_empty_values() {
        let cookies = vec![
            ("__Secure-1PSID".to_owned(), String::new()),
            ("__Secure-1PSIDTS".to_owned(), "PSIDTS-VALUE".to_owned()),
        ];

        let (psid, psidts) = find_session_cookies(&cookies);

        assert_eq!(psid.as_deref(), Some(""));
        assert_eq!(psidts.as_deref(), Some("PSIDTS-VALUE"));
    }

    #[test]
    fn the_last_value_wins_for_a_duplicate_name() {
        let cookies = vec![
            ("__Secure-1PSID".to_owned(), "FIRST".to_owned()),
            ("__Secure-1PSID".to_owned(), "SECOND".to_owned()),
            ("__Secure-1PSIDTS".to_owned(), "TS".to_owned()),
        ];

        let (psid, psidts) = find_session_cookies(&cookies);

        assert_eq!(psid.as_deref(), Some("SECOND"));
        assert_eq!(psidts.as_deref(), Some("TS"));
    }

    #[test]
    fn name_matching_is_case_sensitive() {
        let cookies = vec![
            ("__secure-1psid".to_owned(), "lower".to_owned()),
            ("__Secure-1PSIDTS".to_owned(), "TS".to_owned()),
        ];

        let (psid, psidts) = find_session_cookies(&cookies);

        assert!(psid.is_none());
        assert_eq!(psidts.as_deref(), Some("TS"));
    }
}

use crate::config::CookieSet;
use anyhow::{Context, Result};

#[cfg(target_os = "windows")]
use std::path::PathBuf;
use std::time::Duration;
#[cfg(not(target_os = "windows"))]
use std::time::Instant;
#[cfg(not(target_os = "windows"))]
use tao::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder},
    platform::run_return::EventLoopExtRunReturn,
    window::WindowBuilder,
};
#[cfg(not(target_os = "windows"))]
use wry::WebViewBuilder;
#[cfg(not(target_os = "windows"))]
use zeroize::Zeroizing;

#[cfg(not(target_os = "windows"))]
const TARGET_PSID: &str = "__Secure-1PSID";
#[cfg(not(target_os = "windows"))]
const TARGET_PSIDTS: &str = "__Secure-1PSIDTS";
#[cfg(not(target_os = "windows"))]
const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Returns the Python interpreter that runs the helper scripts.
///
/// The `Q_PYTHON` environment variable wins over the interpreter of the
/// system, so a user can point the tool at a specific Python.
///
/// # Errors
///
/// Returns an error if no usable Python interpreter can be found.
#[cfg(target_os = "windows")]
pub(crate) fn python_bin() -> Result<PathBuf> {
    if let Some(python) = std::env::var_os("Q_PYTHON") {
        return Ok(PathBuf::from(python));
    }

    crate::python_env::finder::find_system_python()
        .context("cannot find a Python interpreter for the helper scripts")
}

/// Authenticates on Windows, where the wry webview path is unavailable.
///
/// The login runs in a portable Firefox that the tool downloads and unpacks
/// itself. Google refuses to sign in to an automated browser, so a real
/// Firefox qualifies, and its session cookies stay usable outside the browser.
#[cfg(target_os = "windows")]
fn authenticate_on_windows(timeout: Duration) -> Result<CookieSet> {
    firefox::authenticate_with_firefox(timeout)
}

#[cfg(not(target_os = "windows"))]
type CookiePair = (String, String);
#[cfg(not(target_os = "windows"))]
type SessionCookiePair = (Option<String>, Option<String>);

#[cfg(not(target_os = "windows"))]
fn find_session_cookies(cookies: &[CookiePair]) -> SessionCookiePair {
    let mut psid = None;
    let mut psidts = None;

    for (name, value) in cookies {
        match name.as_str() {
            TARGET_PSID => psid = Some(value.clone()),
            TARGET_PSIDTS => psidts = Some(value.clone()),
            _ => {}
        }
    }

    (psid, psidts)
}

/// Authenticates with Gemini by opening a webview and capturing cookies.
///
/// Opens a webview window where the user can sign in to their Google account.
/// The function polls for cookies every 2 seconds and returns once both
/// `__Secure-1PSID` and `__Secure-1PSIDTS` are found.
///
/// The webview runs in incognito mode to ensure no cookies persist after closure.
///
/// # Errors
///
/// Returns an error if:
/// - No graphical environment is detected (e.g., SSH without X11 forwarding).
/// - The window or webview fails to create.
/// - The user closes the window without logging in.
/// - The required cookies are not found after login.
/// - The authentication times out.
#[allow(clippy::print_stdout, clippy::missing_errors_doc)]
pub fn authenticate_with_gemini(timeout: Duration) -> Result<CookieSet> {
    #[cfg(target_os = "windows")]
    {
        authenticate_on_windows(timeout)
    }

    #[cfg(target_os = "linux")]
    {
        if std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none() {
            return Err(anyhow::anyhow!(
                "No graphical environment detected. \n\
                 Authentication requires a GUI (X11 or Wayland). \n\
                 If you are using SSH, please use X11 forwarding (`ssh -X` or `ssh -Y`) \n\
                 or use `q --import-cookies` to manually paste your cookies."
            ));
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        let mut event_loop = EventLoopBuilder::<()>::with_user_event().build();

        let window = WindowBuilder::new()
            .with_title("Sign in to Gemini")
            .with_inner_size(tao::dpi::LogicalSize::new(900, 700))
            .build(&event_loop)
            .context("failed to create window")?;

        let webview = WebViewBuilder::new()
            .with_url("https://gemini.google.com/app")
            .with_incognito(true)
            .build(&window)
            .context("failed to create webview")?;

        println!("Waiting for Gemini login...");

        let mut auth_result: Option<CookieSet> = None;
        let mut next_check = Instant::now() + POLL_INTERVAL;
        let timeout_deadline = Instant::now() + timeout;

        event_loop.run_return(|event, _, control_flow| {
            let now = Instant::now();

            if now >= timeout_deadline {
                *control_flow = ControlFlow::Exit;
                return;
            }

            match event {
                Event::WindowEvent {
                    event: WindowEvent::CloseRequested,
                    ..
                } => {
                    *control_flow = ControlFlow::Exit;
                }

                Event::MainEventsCleared => {
                    if now >= next_check {
                        next_check = now + POLL_INTERVAL;

                        if let Ok(cookies) = webview.cookies() {
                            let pairs: Vec<CookiePair> = cookies
                                .iter()
                                .map(|c| (c.name().to_owned(), c.value().to_owned()))
                                .collect();
                            let (psid, psidts) = find_session_cookies(&pairs);

                            if let (Some(psid_val), Some(psidts_val)) = (psid, psidts) {
                                if !psid_val.is_empty() && !psidts_val.is_empty() {
                                    auth_result = Some(CookieSet {
                                        psid: Zeroizing::new(psid_val),
                                        psidts: Zeroizing::new(psidts_val),
                                    });
                                    *control_flow = ControlFlow::Exit;
                                    return;
                                }
                            }
                        }
                    }

                    let next_wake = next_check.min(timeout_deadline);
                    *control_flow = ControlFlow::WaitUntil(next_wake);
                }

                _ => {
                    let next_wake = next_check.min(timeout_deadline);
                    *control_flow = ControlFlow::WaitUntil(next_wake);
                }
            }
        });

        if Instant::now() >= timeout_deadline {
            return Err(anyhow::anyhow!(
                "Authentication timed out after {} seconds. Please try again, or use `q --import-cookies`.",
                timeout.as_secs()
            ));
        }

        auth_result.ok_or_else(|| {
            anyhow::anyhow!(
                "Authentication cancelled or failed. Please ensure you have successfully logged in, or use `q --import-cookies`."
            )
        })
    }
}
