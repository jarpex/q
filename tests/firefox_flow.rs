//! End-to-end test of the portable Firefox login flow.
//!
//! The test drives the real browser: it unpacks the official installer, starts
//! the browser under a name of its own, lets it write cookies into a profile
//! of its own, closes it and reads the cookies back.
//!
//! The test needs the network and a graphical session, and it downloads about
//! 100 MB, so it does not run on its own. Run it with:
//!
//! ```text
//! cargo test --test firefox_flow -- --ignored
//! ```

#![cfg(target_os = "windows")]
#![allow(clippy::result_large_err)]

use anyhow::{bail, Context, Result};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use q::auth::browser::{self, Firefox};
use q::auth::session;

/// A page that hands out a cookie, so that the browser has something to store.
const PAGE: &str = "<html><body>hello</body></html>";

/// Serves one page on a port of its own and reports the port.
fn serve() -> Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0").context("cannot bind the probe server")?;
    let port = listener
        .local_addr()
        .context("cannot read the port of the probe server")?
        .port();

    thread::spawn(move || {
        for incoming in listener.incoming() {
            let Ok(mut stream) = incoming else { continue };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nSet-Cookie: PROBE=VALUE; Max-Age=86400; Path=/\r\nContent-Length: {}\r\n\r\n{PAGE}",
                PAGE.len()
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
    });

    Ok(port)
}

/// Waits until the file shows up, and reports whether it did.
fn wait_for_file(path: &Path, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;

    while Instant::now() < deadline {
        if path.exists() {
            return true;
        }
        thread::sleep(Duration::from_millis(250));
    }

    false
}

/// Counts the processes that run under the given image name.
fn running_instances(image: &str) -> usize {
    let Ok(output) = Command::new("tasklist")
        .arg("/FI")
        .arg(format!("IMAGENAME eq {image}"))
        .arg("/NH")
        .arg("/FO")
        .arg("CSV")
        .output()
    else {
        return 0;
    };

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| line.starts_with(&format!("\"{image}\"")))
        .count()
}

/// Closes the browser of the test and waits for it to exit.
fn close(image: &str) {
    let _ = Command::new("taskkill").arg("/IM").arg(image).output();

    let deadline = Instant::now() + Duration::from_secs(30);
    while running_instances(image) > 0 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(250));
    }
}

/// Checks that the browser is unpacked and reports the version it carries.
#[test]
#[ignore = "downloads about 100 MB and needs the network"]
fn unpacks_the_pinned_browser() -> Result<()> {
    let firefox = Firefox::ensure_installed().context("cannot prepare the browser")?;

    if !firefox.executable().is_file() {
        bail!("the browser is missing");
    }

    let version = std::fs::read_to_string(firefox.root().join("core").join("application.ini"))
        .context("cannot read the version of the browser")?;

    if !version.contains(&format!("Version={}", browser::FIREFOX_VERSION)) {
        bail!("the browser carries another version");
    }

    Ok(())
}

/// Checks the whole lifecycle: start, store, close, read back.
#[test]
#[ignore = "needs a graphical session"]
fn signs_in_through_the_portable_browser() -> Result<()> {
    browser::clean_traces();
    let firefox = Firefox::ensure_installed().context("cannot prepare the browser")?;
    let profile = browser::create_profile().context("cannot create the profile")?;
    let executable = firefox
        .unique_executable()
        .context("cannot copy the browser")?;

    let port = serve()?;
    let url = format!("http://127.0.0.1:{port}/page");

    Command::new(&executable)
        .arg("-profile")
        .arg(&profile)
        .arg("-no-remote")
        .arg(&url)
        .spawn()
        .context("cannot start the browser")?;

    let database = profile.join("cookies.sqlite");
    if !wait_for_file(&database, Duration::from_secs(60)) {
        bail!("the browser did not write a cookie database");
    }

    let image = executable
        .file_name()
        .context("the copy of the browser has no name")?
        .to_string_lossy()
        .to_string();

    if running_instances(&image) == 0 {
        bail!("the browser is not running");
    }

    // The browser of the user must not be touched by any of this.
    let user_browser = running_instances("firefox.exe");

    // The browser needs a moment to settle: closing it while it is still
    // starting makes it restart itself.
    thread::sleep(Duration::from_secs(20));

    close(&image);

    if running_instances(&image) != 0 {
        bail!("the browser is still running");
    }

    if running_instances("firefox.exe") != user_browser {
        bail!("the Firefox of the user was touched");
    }

    let store = profile.join("sessionstore.jsonlz4");
    if !store.is_file() {
        bail!("the browser wrote no session store");
    }

    let _ = std::fs::remove_file(&executable);
    let _ = browser::remove_profile(&profile);
    browser::clean_traces();

    Ok(())
}

/// Checks that the cookies of a profile are read out of the database and out
/// of the session store.
#[test]
#[ignore = "needs a graphical session"]
fn reads_the_cookies_of_a_signed_in_profile() -> Result<()> {
    browser::clean_traces();
    let firefox = Firefox::ensure_installed().context("cannot prepare the browser")?;
    let profile = browser::create_profile().context("cannot create the profile")?;
    let executable = firefox
        .unique_executable()
        .context("cannot copy the browser")?;

    let port = serve()?;
    let url = format!("http://127.0.0.1:{port}/page");

    Command::new(&executable)
        .arg("-profile")
        .arg(&profile)
        .arg("-no-remote")
        .arg(&url)
        .spawn()
        .context("cannot start the browser")?;

    let database = profile.join("cookies.sqlite");
    if !wait_for_file(&database, Duration::from_secs(60)) {
        bail!("the browser did not write a cookie database");
    }

    let image = executable
        .file_name()
        .context("the copy of the browser has no name")?
        .to_string_lossy()
        .to_string();

    // The browser needs a moment to settle: closing it while it is still
    // starting makes it restart itself.
    thread::sleep(Duration::from_secs(20));

    close(&image);

    // The cookie of the probe server is not a Google cookie, so the reader
    // filters it out. The database itself has to hold it.
    let mut copy = std::env::temp_dir();
    copy.push(format!("q-probe-{}.sqlite", std::process::id()));
    std::fs::copy(&database, &copy).context("cannot copy the database")?;

    let mut content = Vec::new();
    let mut file = std::fs::File::open(&copy).context("cannot open the copy")?;
    file.read_to_end(&mut content)
        .context("cannot read the copy")?;
    let _ = std::fs::remove_file(&copy);

    if !content.windows(b"PROBE".len()).any(|part| part == b"PROBE") {
        bail!("the database holds no cookie");
    }

    let _ = std::fs::remove_file(&executable);
    let _ = browser::remove_profile(&profile);
    browser::clean_traces();

    Ok(())
}

/// Checks that the session store of a profile is decoded.
#[test]
fn decodes_a_session_store() -> Result<()> {
    let directory = std::env::temp_dir().join(format!("q-store-{}", std::process::id()));
    std::fs::create_dir_all(&directory).context("cannot create the directory")?;
    let path = directory.join("sessionstore.jsonlz4");

    let plain = r#"{"version":1,"cookies":[{"host":".google.com","name":"__Secure-1PSID","value":"VALUE"}]}"#;
    let compressed = lz4_flex::block::compress(plain.as_bytes());

    let mut bytes = Vec::from(b"mozLz40\0");
    let size = u32::try_from(plain.len()).context("the store is too large")?;
    bytes.extend_from_slice(&size.to_le_bytes());
    bytes.extend_from_slice(&compressed);
    std::fs::write(&path, &bytes).context("cannot write the store")?;

    let cookies = session::read_store(&path).context("cannot read the store")?;

    if cookies.len() != 1 {
        bail!("the store holds {} cookies instead of one", cookies.len());
    }
    let value = cookies.first().map(|cookie| cookie.value.as_str());
    if value != Some("VALUE") {
        bail!("the store holds another value");
    }

    let _ = std::fs::remove_dir_all(&directory);

    Ok(())
}
