//! Manual cookie import. Critical: cookies expire ~15 min if Gemini tab stays open.
//!
//! Use Firefox (recommended, avoids DBSC), incognito, copy the full Cookie
//! header, close tab.

use crate::config::{cookies_path, save_cookies, Cookie, CookieSet};
use crate::{Error, Result};
use std::io::{self, Write};
use zeroize::Zeroizing;

/// Interactive cookie import.
///
/// Paste the full Cookie header of a signed-in gemini.google.com request, then
/// close the Gemini tab immediately to avoid invalidation. The two session IDs
/// are required; the rest of the header is kept so Google accepts the session.
///
/// # Errors
///
/// Returns error if cookie input is empty, stdin read fails, or saving to disk fails.
#[allow(clippy::print_stdout)]
pub fn run_manual_import() -> Result<()> {
    println!("╭──────────────────────────────────────────────────────────────────────────────╮");
    println!("│  MANUAL COOKIE IMPORT                                                        │");
    println!("│  This is the fallback when the automatic sign-in does not work.               │");
    println!("├──────────────────────────────────────────────────────────────────────────────┤");
    println!("│  1. Firefox recommended. Open incognito mode.                                │");
    println!("│  2. Open https://gemini.google.com/app  --  log in.                          │");
    println!("│  3. DevTools (F12) -> Network -> any request to gemini.google.com            │");
    println!("│  4. Copy the whole `Cookie:` request header (name=value; name=value; ...).   │");
    println!("│  5. CRITICAL: Close Gemini tab immediately after copying.                     │");
    println!("│     Otherwise Google updates session and invalidates cookies ~15 min.        │");
    println!("│  7. Separate Google account recommended for keep-alive services.             │");
    println!("╰──────────────────────────────────────────────────────────────────────────────╯");
    println!();

    let header = prompt_for_cookie("Cookie header")?;
    let cookie_set = parse_cookie_header(&header)?;

    let path = cookies_path()?;
    save_cookies(&path, &cookie_set)
        .map_err(|e| Error::Cookie(format!("Failed to save cookies to disk: {e}")))?;

    println!();
    println!("Success! Cookies saved to: {}", path.display());
    println!("IMPORTANT: Close Gemini tab now -- cookies expire if left open.");
    println!("Use q normally.");

    Ok(())
}

/// Parses a Cookie request header into a cookie set.
///
/// The header is a `name=value; name=value` list. `__Secure-1PSID` and
/// `__Secure-1PSIDTS` fill the session fields; every other google.com cookie
/// is kept with the domain it was set for. A cookie with no domain in the
/// header is stored on `.google.com`, the domain the library uses.
///
/// # Errors
///
/// Returns an error if the header does not carry both session cookies.
fn parse_cookie_header(header: &str) -> Result<CookieSet> {
    let mut found_psid = None;
    let mut found_psidts = None;
    let mut extra = Vec::new();

    for raw_segment in header.split(';') {
        let segment = raw_segment.trim();
        if segment.is_empty() {
            continue;
        }
        let Some((raw_name, raw_value)) = segment.split_once('=') else {
            continue;
        };
        let cookie_name = raw_name.trim();
        let cookie_value = raw_value.trim();
        if cookie_name.is_empty() {
            continue;
        }

        match cookie_name {
            "__Secure-1PSID" => found_psid = Some(Zeroizing::new(cookie_value.to_owned())),
            "__Secure-1PSIDTS" => found_psidts = Some(Zeroizing::new(cookie_value.to_owned())),
            _ => extra.push(Cookie {
                name: Zeroizing::new(cookie_name.to_owned()),
                value: Zeroizing::new(cookie_value.to_owned()),
                domain: ".google.com".to_owned(),
            }),
        }
    }

    let psid = found_psid
        .ok_or_else(|| Error::Cookie("the Cookie header is missing __Secure-1PSID".into()))?;
    let psidts = found_psidts
        .ok_or_else(|| Error::Cookie("the Cookie header is missing __Secure-1PSIDTS".into()))?;

    Ok(CookieSet {
        psid,
        psidts,
        extra,
    })
}

#[allow(clippy::print_stdout)]
fn prompt_for_cookie(name: &str) -> Result<Zeroizing<String>> {
    print!("Paste value for {name} > ");
    io::stdout()
        .flush()
        .map_err(|e| Error::System(format!("Failed to flush stdout: {e}")))?;

    let mut input = Zeroizing::new(String::new());
    io::stdin()
        .read_line(&mut input)
        .map_err(|e| Error::System(format!("Failed to read from stdin: {e}")))?;

    let trimmed = Zeroizing::new(input.trim().to_owned());
    if trimmed.is_empty() {
        return Err(Error::Cookie(
            "Cookie value cannot be empty. Please try again.".into(),
        ));
    }

    Ok(trimmed)
}

#[cfg(test)]
#[allow(clippy::result_large_err)]
mod tests {
    use super::parse_cookie_header;

    fn parse(header: &str) -> anyhow::Result<crate::config::CookieSet> {
        Ok(parse_cookie_header(header)?)
    }

    #[test]
    fn parses_the_two_session_ids() -> anyhow::Result<()> {
        let set = parse("__Secure-1PSID=abc; __Secure-1PSIDTS=def")?;

        assert_eq!(set.psid.as_str(), "abc");
        assert_eq!(set.psidts.as_str(), "def");
        assert!(set.extra.is_empty());
        Ok(())
    }

    #[test]
    fn keeps_the_extra_cookies() -> anyhow::Result<()> {
        let set = parse("__Secure-1PSID=abc; __Secure-1PSIDTS=def; SID=s; OSID=o")?;

        assert_eq!(set.extra.len(), 2);
        assert!(set.extra.iter().any(|c| c.name.as_str() == "SID"));
        assert!(set.extra.iter().any(|c| c.name.as_str() == "OSID"));
        Ok(())
    }

    #[test]
    fn extra_cookies_default_to_google_domain() -> anyhow::Result<()> {
        let set = parse("__Secure-1PSID=abc; __Secure-1PSIDTS=def; SID=s")?;

        let first = set.extra.first();
        assert_eq!(first.map(|c| c.domain.as_str()), Some(".google.com"));
        Ok(())
    }

    #[test]
    fn rejects_a_header_without_psid() {
        assert!(parse_cookie_header("__Secure-1PSIDTS=def").is_err());
    }

    #[test]
    fn rejects_a_header_without_psidts() {
        assert!(parse_cookie_header("__Secure-1PSID=abc").is_err());
    }

    #[test]
    fn ignores_empty_segments() -> anyhow::Result<()> {
        let set = parse(";; __Secure-1PSID=abc ;; __Secure-1PSIDTS=def ;")?;

        assert_eq!(set.psid.as_str(), "abc");
        assert_eq!(set.psidts.as_str(), "def");
        Ok(())
    }
}
