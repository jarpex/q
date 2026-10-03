use crate::config::CookieSet;
use anyhow::{anyhow, Context, Result};
use std::time::{Duration, Instant};
use tao::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder},
    platform::run_return::EventLoopExtRunReturn,
    window::WindowBuilder,
};
use wry::WebViewBuilder;

const TARGET_PSID: &str = "__Secure-1PSID";
const TARGET_PSIDTS: &str = "__Secure-1PSIDTS";
const POLL_INTERVAL: Duration = Duration::from_secs(2);
const AUTH_TIMEOUT: Duration = Duration::from_secs(300); // 5 минут

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
/// - The authentication times out after 5 minutes.
pub fn authenticate_with_gemini() -> Result<CookieSet> {
    #[cfg(target_os = "linux")]
    {
        if std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none() {
            anyhow::bail!(
                "No graphical environment detected. \n\
                 Authentication requires a GUI (X11 or Wayland). \n\
                 If you are using SSH, please use X11 forwarding (`ssh -X` or `ssh -Y`) \n\
                 or authenticate on a local machine and copy the cookies manually."
            );
        }
    }

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

    let mut auth_result: Option<CookieSet> = None;
    let mut next_check = Instant::now() + POLL_INTERVAL;
    let timeout_deadline = Instant::now() + AUTH_TIMEOUT;

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
                        let mut psid = None;
                        let mut psidts = None;

                        for cookie in cookies {
                            match cookie.name() {
                                TARGET_PSID => psid = Some(cookie.value().to_owned()),
                                TARGET_PSIDTS => psidts = Some(cookie.value().to_owned()),
                                _ => {}
                            }
                        }

                        if let (Some(psid_val), Some(psidts_val)) = (psid, psidts) {
                            if !psid_val.is_empty() && !psidts_val.is_empty() {
                                auth_result = Some(CookieSet {
                                    psid: psid_val,
                                    psidts: psidts_val,
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
        return Err(anyhow!(
            "Authentication timed out after {} minutes. Please try again.",
            AUTH_TIMEOUT.as_secs() / 60
        ));
    }

    auth_result.ok_or_else(|| {
        anyhow!(
            "Authentication cancelled or failed. Please ensure you have successfully logged in."
        )
    })
}
