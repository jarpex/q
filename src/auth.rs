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

/// Authenticates with Gemini by opening a webview and capturing cookies.
///
/// Opens a webview window where the user can sign in to their Google account.
/// The function polls for cookies every 2 seconds and returns once both
/// `__Secure-1PSID` and `__Secure-1PSIDTS` are found.
///
/// The webview runs in incognito mode to ensure no cookies persist after closure.
pub(crate) fn authenticate_with_gemini() -> Result<CookieSet> {
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

    let mut auth_result: Option<Result<CookieSet>> = None;
    let check_interval = Duration::from_secs(2);
    let mut next_check = Instant::now() + check_interval;

    event_loop.run_return(|event, _, control_flow| {
        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                *control_flow = ControlFlow::Exit;
            }
            Event::MainEventsCleared => {
                let now = Instant::now();
                if now >= next_check {
                    next_check = now + check_interval;

                    if let Ok(cookies) = webview.cookies() {
                        let mut psid = None;
                        let mut psidts = None;

                        for cookie in cookies {
                            match cookie.name() {
                                "__Secure-1PSID" => psid = Some(cookie.value().to_string()),
                                "__Secure-1PSIDTS" => psidts = Some(cookie.value().to_string()),
                                _ => {}
                            }
                        }

                        if let (Some(psid_val), Some(psidts_val)) = (psid, psidts) {
                            if !psid_val.is_empty() && !psidts_val.is_empty() {
                                auth_result = Some(Ok(CookieSet {
                                    psid: psid_val,
                                    psidts: psidts_val,
                                }));
                                *control_flow = ControlFlow::Exit;
                                return;
                            }
                        }
                    }
                }
                *control_flow = ControlFlow::WaitUntil(next_check);
            }
            _ => {
                *control_flow = ControlFlow::WaitUntil(next_check);
            }
        }
    });

    auth_result.unwrap_or_else(|| {
        Err(anyhow!(
            "Authentication cancelled or failed. Please ensure you have successfully logged in."
        ))
    })
}