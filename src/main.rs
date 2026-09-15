//! CLI tool for quick, one-shot Gemini queries with no API key required
//!
//! This binary provides a command-line interface to interact with Google Gemini
//! using reverse-engineered web API. It supports both streaming and batch modes,
//! command generation, and automatic authentication via webview.

use anyhow::Result;
use clap::Parser;

mod auth;
mod cli;
mod config;
mod python;
mod shell;
mod tui;

use auth::authenticate_with_gemini;
use cli::Cli;
use config::{cookies_path, load_cookies, save_cookies};
use python::{ask_gemini_via_python, ensure_python_venv};
use shell::{command_mode, SystemContext};
use tui::{print_copied_message, print_error, Spinner, StreamingBox};

const PLAIN_TEXT_SYSTEM_PROMPT: &str = "Respond in plain text only. Follow these rules strictly:
1. NO markdown: no **bold**, no *italics*, no _underscores_, no `code`, no # headers, no > quotes, no - lists with markers.
2. NO tables whatsoever.
3. NO bullet points, NO numbered lists.
4. Be EXTREMELY concise: respond in 2-4 SHORT sentences total. One compact paragraph.
5. NO empty lines within the answer — the entire response must be a single block of text.
6. NO service tags like <Image/>, <Elicitation>, <ElicitationsGroup>.
7. Respond in the SAME language as the user's question.
8. Do NOT include any preamble like 'Sure!' or 'Here is...'. Just answer directly.
9. If the question asks for a formula, give the formula inline with a brief explanation.";

#[tokio::main]
#[allow(clippy::print_stdout)]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let path = cookies_path()?;

    let python_bin = ensure_python_venv(cli.rebuild_venv)?;

    let cookies = if cli.login || load_cookies(&path).is_err() {
        println!("🔐 Opening Gemini login page in webview...");
        println!("   Sign in, then wait ~5-10 seconds after successful login.");
        println!("   The window will close automatically once cookies are captured.");
        let cookies = authenticate_with_gemini()?;
        save_cookies(&path, &cookies)?;
        let display_path = path.display();
        println!("✅ Login successful, cookies saved to {display_path}");
        cookies
    } else {
        load_cookies(&path)?
    };

    if cli.query.is_empty() {
        println!("No query provided. Usage: q \"your question\"");
        println!("Run `q --help` for options.");
        return Ok(());
    }

    let query = cli.query.join(" ");

    if cli.command_mode {
        command_mode(&python_bin, &cookies, &query, &cli.model, cli.debug).await?;
    } else {
        let sys_ctx = SystemContext::collect().await;
        let system_context_str = sys_ctx.to_prompt_context();

        let plain_query = format!(
            "{PLAIN_TEXT_SYSTEM_PROMPT}\n\n{system_context_str}\n\nUser question: {query}"
        );

        if cli.no_stream {
            run_batch_mode(&python_bin, &cookies, &plain_query, &cli.model, cli.debug).await;
        } else {
            run_stream_mode(&python_bin, &cookies, &plain_query, &cli.model, cli.debug).await;
        }
    }

    Ok(())
}

/// Runs the application in batch mode (non-streaming).
///
/// Shows a spinner while waiting for the response, then displays
/// the complete answer in a box and copies it to the clipboard.
async fn run_batch_mode(
    python_bin: &std::path::Path,
    cookies: &config::CookieSet,
    query: &str,
    model: &str,
    debug: bool,
) {
    let spinner = Spinner::start("Thinking...");
    let response = ask_gemini_via_python(python_bin, cookies, query, model, false, debug).await;
    spinner.stop_and_rewind();

    match response {
        Ok(text) => {
            let title = format!("q ─ batch ─ {model}");
            tui::print_in_box(&text, &title);
            if tui::copy_to_clipboard(&text) {
                print_copied_message();
            }
        }
        Err(e) => print_error(&format!("{e:#}")),
    }
}

/// Runs the application in streaming mode.
///
/// Shows a spinner initially, then streams the response character by character
/// inside a box, and copies the complete text to the clipboard when finished.
async fn run_stream_mode(
    python_bin: &std::path::Path,
    cookies: &config::CookieSet,
    query: &str,
    model: &str,
    debug: bool,
) {
    let spinner = Spinner::start("Thinking...");
    let result = stream_with_indent(python_bin, cookies, query, model, debug, spinner).await;

    match result {
        Ok(text) => {
            if tui::copy_to_clipboard(&text) {
                print_copied_message();
            }
        }
        Err(e) => print_error(&format!("{e:#}")),
    }
}

/// Streams response from Gemini with proper spinner handling.
///
/// The spinner is stopped and rewound when the first chunk arrives,
/// allowing the streaming box to appear seamlessly in its place.
#[allow(clippy::print_stderr)]
async fn stream_with_indent(
    python_bin: &std::path::Path,
    cookies: &config::CookieSet,
    query: &str,
    model: &str,
    debug: bool,
    spinner: Spinner,
) -> Result<String> {
    use anyhow::Context;
    use std::process::Stdio;
    use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
    use tokio::process::Command;

    if debug {
        let display_bin = python_bin.display();
        eprintln!("[debug] Python: {display_bin}");
        eprintln!("[debug] Model: {model}");
        eprintln!("[debug] Stream: true");
    }

    let mut child = Command::new(python_bin)
        .arg("-c")
        .arg(python::PYTHON_SCRIPT)
        .arg(&cookies.psid)
        .arg(&cookies.psidts)
        .arg(model)
        .arg("stream")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("Failed to spawn {}", python_bin.display()))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(query.as_bytes())
            .await
            .context("Failed to write query to Python stdin")?;
    }

    let stdout = child.stdout.take().context("no stdout from python")?;
    let stderr = child.stderr.take().context("no stderr from python")?;

    let stderr_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        let mut reader = BufReader::new(stderr);
        let _ = reader.read_to_end(&mut buf).await;
        buf
    });

    let mut reader = BufReader::new(stdout);
    let mut buf = [0u8; 8192];
    let mut spinner_stopped = false;
    let mut spinner_opt = Some(spinner);

    let title = format!("q ─ stream ─ {model}");
    let mut box_printer = StreamingBox::new(&title);

    loop {
        let n = reader.read(&mut buf).await.context("failed to read python stdout")?;
        if n == 0 {
            break;
        }

        if !spinner_stopped {
            if let Some(s) = spinner_opt.take() {
                s.stop_and_rewind();
            }
            spinner_stopped = true;
        }

        if let Some(slice) = buf.get(..n) {
            let text = String::from_utf8_lossy(slice);
            box_printer.write(&text);
        }
    }

    let full_text = box_printer.finish();

    let status = child.wait().await.context("failed to wait for python")?;
    let stderr_bytes = stderr_task.await.unwrap_or_default();

    if !status.success() {
        let stderr = String::from_utf8_lossy(&stderr_bytes);
        if !stderr.trim().is_empty() {
            eprint!("{stderr}");
        }
        let first_line = stderr.lines().find(|l| !l.is_empty()).unwrap_or("unknown error");
        anyhow::bail!("Python gemini_webapi failed: {first_line}");
    }

    Ok(full_text)
}
