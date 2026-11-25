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
    use super::{collect_extra_cookies, find_session_cookies, CapturedCookie};

    fn pair(name: &str, value: &str) -> CapturedCookie {
        (
            name.to_owned(),
            value.to_owned(),
            Some(".google.com".to_owned()),
        )
    }

    #[test]
    fn finds_both_session_cookies() {
        let cookies = vec![
            pair("__Secure-1PSID", "PSID-VALUE"),
            pair("__Secure-1PSIDTS", "PSIDTS-VALUE"),
        ];

        let (psid, psidts) = find_session_cookies(&cookies);

        assert_eq!(psid.as_deref(), Some("PSID-VALUE"));
        assert_eq!(psidts.as_deref(), Some("PSIDTS-VALUE"));
    }

    #[test]
    fn ignores_unrelated_cookies() {
        let cookies = vec![
            pair("other", "x"),
            pair("__Secure-1PSID", "PSID-VALUE"),
            pair("__Secure-1PSIDTS", "PSIDTS-VALUE"),
            pair("session", "y"),
        ];

        let (psid, psidts) = find_session_cookies(&cookies);

        assert_eq!(psid.as_deref(), Some("PSID-VALUE"));
        assert_eq!(psidts.as_deref(), Some("PSIDTS-VALUE"));
    }

    #[test]
    fn returns_none_when_psid_is_missing() {
        let cookies = vec![pair("__Secure-1PSIDTS", "PSIDTS-VALUE")];

        let (psid, psidts) = find_session_cookies(&cookies);

        assert!(psid.is_none());
        assert_eq!(psidts.as_deref(), Some("PSIDTS-VALUE"));
    }

    #[test]
    fn returns_none_when_psidts_is_missing() {
        let cookies = vec![pair("__Secure-1PSID", "PSID-VALUE")];

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
        let cookies = vec![pair("foo", "bar"), pair("baz", "qux")];

        let (psid, psidts) = find_session_cookies(&cookies);

        assert!(psid.is_none());
        assert!(psidts.is_none());
    }

    #[test]
    fn keeps_empty_values() {
        let cookies = vec![
            pair("__Secure-1PSID", ""),
            pair("__Secure-1PSIDTS", "PSIDTS-VALUE"),
        ];

        let (psid, psidts) = find_session_cookies(&cookies);

        assert_eq!(psid.as_deref(), Some(""));
        assert_eq!(psidts.as_deref(), Some("PSIDTS-VALUE"));
    }

    #[test]
    fn the_last_value_wins_for_a_duplicate_name() {
        let cookies = vec![
            pair("__Secure-1PSID", "FIRST"),
            pair("__Secure-1PSID", "SECOND"),
            pair("__Secure-1PSIDTS", "TS"),
        ];

        let (psid, psidts) = find_session_cookies(&cookies);

        assert_eq!(psid.as_deref(), Some("SECOND"));
        assert_eq!(psidts.as_deref(), Some("TS"));
    }

    #[test]
    fn name_matching_is_case_sensitive() {
        let cookies = vec![
            pair("__secure-1psid", "lower"),
            pair("__Secure-1PSIDTS", "TS"),
        ];

        let (psid, psidts) = find_session_cookies(&cookies);

        assert!(psid.is_none());
        assert_eq!(psidts.as_deref(), Some("TS"));
    }

    #[test]
    fn extra_cookies_drop_the_two_session_ids() {
        let cookies = vec![
            pair("__Secure-1PSID", "P"),
            pair("__Secure-1PSIDTS", "T"),
            pair("SID", "s"),
            pair("OSID", "o"),
        ];

        let extra = collect_extra_cookies(&cookies);

        assert_eq!(extra.len(), 2);
        assert!(extra.iter().all(|c| c.name.as_str() != "__Secure-1PSID"));
        assert!(extra.iter().all(|c| c.name.as_str() != "__Secure-1PSIDTS"));
    }

    #[test]
    fn extra_cookies_keep_their_domain() {
        let cookies = vec![
            (
                "SID".to_owned(),
                "s".to_owned(),
                Some(".google.com".to_owned()),
            ),
            (
                "OSID".to_owned(),
                "o".to_owned(),
                Some("gemini.google.com".to_owned()),
            ),
        ];

        let extra = collect_extra_cookies(&cookies);

        let sid = extra.iter().find(|c| c.name.as_str() == "SID");
        let osid = extra.iter().find(|c| c.name.as_str() == "OSID");
        assert_eq!(sid.map(|c| c.domain.as_str()), Some(".google.com"));
        assert_eq!(osid.map(|c| c.domain.as_str()), Some("gemini.google.com"));
    }

    #[test]
    fn extra_cookies_default_to_google_domain() {
        let cookies = vec![("SID".to_owned(), "s".to_owned(), None)];

        let extra = collect_extra_cookies(&cookies);

        let first = extra.first();
        assert_eq!(first.map(|c| c.domain.as_str()), Some(".google.com"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn detects_ssh_session_without_gui() {
        let _guard = crate::test_support::EnvGuard::new();
        let old_ssh = std::env::var_os("SSH_CONNECTION");
        let old_cg = std::env::var_os("CG_CONTEXT_SHOW_BACKTRACE");
        std::env::set_var("SSH_CONNECTION", "127.0.0.1 12345 127.0.0.1 22");
        std::env::remove_var("CG_CONTEXT_SHOW_BACKTRACE");
        let res = super::authenticate_with_gemini(std::time::Duration::from_millis(10));
        if let Some(val) = old_ssh {
            std::env::set_var("SSH_CONNECTION", val);
        } else {
            std::env::remove_var("SSH_CONNECTION");
        }
        if let Some(val) = old_cg {
            std::env::set_var("CG_CONTEXT_SHOW_BACKTRACE", val);
        } else {
            std::env::remove_var("CG_CONTEXT_SHOW_BACKTRACE");
        }
        let err = match res {
            Err(err) => err.to_string(),
            Ok(_) => String::new(),
        };
        assert!(err.contains("No graphical session detected on macOS"));
        assert!(err.contains("q --import-cookies"));
    }
}

#[cfg(not(target_os = "windows"))]
use crate::config::Cookie;
use crate::config::CookieSet;
use crate::{Error, Result};

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

    crate::python_env::finder::find_system_python().map_err(|e| {
        Error::PythonEnv(format!(
            "cannot find a Python interpreter for the helper scripts: {e}"
        ))
    })
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
type SessionCookiePair = (Option<String>, Option<String>);

/// A captured cookie: name, value and the domain it belongs to.
#[cfg(not(target_os = "windows"))]
type CapturedCookie = (String, String, Option<String>);

#[cfg(not(target_os = "windows"))]
fn find_session_cookies(cookies: &[CapturedCookie]) -> SessionCookiePair {
    let mut psid = None;
    let mut psidts = None;

    for (name, value, _domain) in cookies {
        match name.as_str() {
            TARGET_PSID => psid = Some(value.clone()),
            TARGET_PSIDTS => psidts = Some(value.clone()),
            _ => {}
        }
    }

    (psid, psidts)
}

/// Builds the extra cookies of a session from every captured google.com cookie.
///
/// The two session IDs are kept in their own fields, so they are dropped here.
/// A cookie with no domain of its own is stored on `.google.com`, the domain
/// the library uses for the session cookies.
#[cfg(not(target_os = "windows"))]
fn collect_extra_cookies(cookies: &[CapturedCookie]) -> Vec<Cookie> {
    cookies
        .iter()
        .filter(|(name, _, _)| name.as_str() != TARGET_PSID && name.as_str() != TARGET_PSIDTS)
        .map(|(name, value, domain)| Cookie {
            name: Zeroizing::new(name.clone()),
            value: Zeroizing::new(value.clone()),
            domain: domain
                .clone()
                .filter(|d| !d.is_empty())
                .unwrap_or_else(|| ".google.com".to_owned()),
        })
        .collect()
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
#[allow(
    clippy::print_stdout,
    clippy::missing_errors_doc,
    clippy::too_many_lines
)]
pub fn authenticate_with_gemini(timeout: Duration) -> Result<CookieSet> {
    #[cfg(target_os = "windows")]
    {
        authenticate_on_windows(timeout)
    }

    #[cfg(target_os = "linux")]
    {
        if std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none() {
            return Err(Error::System(
                "No graphical environment detected. \n\
                 Authentication requires a GUI (X11 or Wayland). \n\
                 If you are using SSH, please use X11 forwarding (`ssh -X` or `ssh -Y`) \n\
                 or use `q --import-cookies` to manually paste your cookies."
                    .into(),
            ));
        }
    }

    #[cfg(target_os = "macos")]
    {
        let has_gui = std::env::var_os("SSH_CONNECTION").is_none()
            || std::env::var_os("CG_CONTEXT_SHOW_BACKTRACE").is_some();
        if !has_gui {
            return Err(Error::System(
                "No graphical session detected on macOS (running via SSH).\n\
                 Please use `q --import-cookies` to manually paste cookies."
                    .into(),
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
            .map_err(|e| Error::Auth(format!("failed to create window: {e}")))?;

        let webview = WebViewBuilder::new()
            .with_url("https://gemini.google.com/app")
            .with_incognito(true)
            .build(&window)
            .map_err(|e| Error::Auth(format!("failed to create webview: {e}")))?;

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
                            let pairs: Vec<CapturedCookie> = cookies
                                .iter()
                                .map(|c| {
                                    (
                                        c.name().to_owned(),
                                        c.value().to_owned(),
                                        c.domain().map(str::to_owned),
                                    )
                                })
                                .collect();
                            let (psid, psidts) = find_session_cookies(&pairs);

                            if let (Some(psid_val), Some(psidts_val)) = (psid, psidts) {
                                if !psid_val.is_empty() && !psidts_val.is_empty() {
                                    let extra = collect_extra_cookies(&pairs);
                                    auth_result = Some(CookieSet {
                                        psid: Zeroizing::new(psid_val),
                                        psidts: Zeroizing::new(psidts_val),
                                        extra,
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
            return Err(Error::Auth(format!(
                "Authentication timed out after {} seconds. Please try again, or use `q --import-cookies`.",
                timeout.as_secs()
            )));
        }

        auth_result.ok_or_else(|| {
            Error::Auth(
                "Authentication cancelled or failed. Please ensure you have successfully logged in, or use `q --import-cookies`."
                    .into(),
            )
        })
    }
}
