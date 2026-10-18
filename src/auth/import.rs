//! Manual cookie import. Critical: cookies expire ~15 min if Gemini tab stays open.
//! Use Firefox (recommended, avoids DBSC), incognito, copy both, close tab.

use crate::config::{cookies_path, save_cookies, CookieSet};
use anyhow::{Context, Result};
use std::io::{self, Write};
use zeroize::Zeroizing;

/// Interactive cookie import. Copy __Secure-1PSID and __Secure-1PSIDTS,
/// then close the Gemini tab immediately to avoid invalidation.
///
/// # Errors
///
/// Returns error if cookie input is empty, stdin read fails, or saving to disk fails.
#[allow(clippy::print_stdout)]
pub fn run_manual_import() -> Result<()> {
    println!("╭──────────────────────────────────────────────────────────────────────────────╮");
    println!("│  MANUAL COOKIE IMPORT                                                        │");
    println!("│  WebView2 extraction is unreliable on Windows. Use manual import.            │");
    println!("├──────────────────────────────────────────────────────────────────────────────┤");
    println!("│  1. Firefox recommended. Open incognito mode.                                │");
    println!("│  2. Open https://gemini.google.com/app  --  log in.                          │");
    println!("│  3. DevTools (F12) -> Cookies -> https://gemini.google.com                   │");
    println!("│  4. Find __Secure-1PSID in Cookies; copy when prompted.                      │");
    println!("│  5. Find __Secure-1PSIDTS in Cookies; copy when prompted.                    │");
    println!("│  6. CRITICAL: Close Gemini tab immediately after copying both.               │");
    println!("│     Otherwise Google updates session and invalidates cookies ~15 min.        │");
    println!("│  7. Separate Google account recommended for keep-alive services.             │");
    println!("╰──────────────────────────────────────────────────────────────────────────────╯");
    println!();

    let psid = prompt_for_cookie("__Secure-1PSID")?;
    println!();
    println!("╭──────────────────────────────────────────────────────────────────────────────╮");
    println!("│  Now paste __Secure-1PSIDTS value.                                           │");
    println!("╰──────────────────────────────────────────────────────────────────────────────╯");
    println!();

    let psidts = prompt_for_cookie("__Secure-1PSIDTS")?;

    let cookie_set = CookieSet {
        psid: Zeroizing::new(psid),
        psidts: Zeroizing::new(psidts),
    };

    let path = cookies_path()?;
    save_cookies(&path, &cookie_set).context("Failed to save cookies to disk")?;

    println!();
    println!("Success! Cookies saved to: {}", path.display());
    println!("IMPORTANT: Close Gemini tab now -- cookies expire if left open.");
    println!("Use q normally.");

    Ok(())
}

#[allow(clippy::print_stdout)]
fn prompt_for_cookie(name: &str) -> Result<String> {
    print!("Paste value for {name} > ");
    io::stdout().flush().context("Failed to flush stdout")?;

    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .context("Failed to read from stdin")?;

    let trimmed = input.trim().to_owned();
    if trimmed.is_empty() {
        anyhow::bail!("Cookie value cannot be empty. Please try again.");
    }

    Ok(trimmed)
}
