// src/main.rs
//! CLI tool for quick, one-shot Gemini queries with no API key required
//!
//! This binary provides a command-line interface to interact with Google Gemini
//! using reverse-engineered web API. It supports both streaming and batch modes,
//! command generation, and automatic authentication via webview.

use anyhow::Result;
use clap::Parser;

use q::auth::authenticate_with_gemini;
use q::cli::Cli;
use q::config::{
    cookies_path, load_cookies, load_metadata, metadata_path, save_cookies, save_metadata, Metadata,
};
use q::python::{ask_gemini_via_python, ensure_python_venv, AskOptions};
use q::shell::{command_mode, CommandOptions, SystemContext};
use q::tui::{print_copied_message, print_error, Spinner, StreamingBox};

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
    let meta_path = metadata_path()?;

    let current_version = env!("CARGO_PKG_VERSION");

    let metadata = load_metadata(&meta_path).unwrap_or_else(|_| Metadata {
        last_update: 0,
        last_version: String::new(),
    });

    let version_changed = metadata.last_version != current_version;
    let force_rebuild = cli.rebuild_venv || version_changed;

    if version_changed {
        println!(
            "📦 Version changed from {} to {}, rebuilding venv...",
            metadata.last_version, current_version
        );
    }

    let current_time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let days_since_update = (current_time - metadata.last_update) / (24 * 60 * 60);
    let should_update_deps = days_since_update >= 7;

    if should_update_deps && !force_rebuild {
        println!("Dependencies are {days_since_update} days old, checking for updates...");
    }

    let python_bin = ensure_python_venv(force_rebuild, should_update_deps)?;

    let cookies = if cli.login || load_cookies(&path).is_err() {
        println!("Opening Gemini login page in webview...");
        println!("Sign in, then wait ~5-10 seconds after successful login.");
        println!("The window will close automatically once cookies are captured.");
        let cookies = authenticate_with_gemini()?;
        save_cookies(&path, &cookies)?;
        let display_path = path.display();
        println!("Login successful, cookies saved to {display_path}");
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

    let new_metadata = Metadata {
        last_update: current_time,
        last_version: current_version.to_owned(),
    };
    save_metadata(&meta_path, &new_metadata)?;

    if cli.command_mode {
        let cmd_options = CommandOptions {
            python_bin: &python_bin,
            cookies: &cookies,
            query: &query,
            model: &cli.model,
            debug: cli.debug,
        };
        command_mode(&cmd_options).await?;
    } else {
        let sys_ctx = SystemContext::collect().await;
        let system_context_str = sys_ctx.to_prompt_context();

        let plain_query =
            format!("{PLAIN_TEXT_SYSTEM_PROMPT}\n\n{system_context_str}\n\nUser question: {query}");

        let options = AskOptions {
            python_bin: &python_bin,
            cookies: &cookies,
            query: &plain_query,
            model: &cli.model,
            stream: !cli.no_stream,
            debug: cli.debug,
        };

        if cli.no_stream {
            run_batch_mode(&options).await;
        } else {
            run_stream_mode(&options).await;
        }
    }

    Ok(())
}

/// Runs the application in batch mode (non-streaming).
///
/// Shows a spinner while waiting for the response, then displays
/// the complete answer in a box and copies it to the clipboard.
async fn run_batch_mode(options: &AskOptions<'_>) {
    let spinner = Spinner::start("Thinking...");
    let response = ask_gemini_via_python(options).await;
    spinner.stop_and_rewind();

    match response {
        Ok(text) => {
            let title = format!("q ─ batch ─ {}", options.model);
            q::tui::print_in_box(&text, &title);
            if q::tui::copy_to_clipboard(&text) {
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
async fn run_stream_mode(options: &AskOptions<'_>) {
    let spinner = Spinner::start("Thinking...");
    let result = stream_with_indent(options, spinner).await;

    match result {
        Ok(text) => {
            if q::tui::copy_to_clipboard(&text) {
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
async fn stream_with_indent(options: &AskOptions<'_>, spinner: Spinner) -> Result<String> {
    use anyhow::Context;
    use std::process::Stdio;
    use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
    use tokio::process::Command;

    let AskOptions {
        python_bin,
        cookies,
        query,
        model,
        stream: _,
        debug,
    } = options;

    if *debug {
        let display_bin = python_bin.display();
        eprintln!("[debug] Python: {display_bin}");
        eprintln!("[debug] Model: {model}");
        eprintln!("[debug] Stream: true");
    }

    let mut child = Command::new(python_bin)
        .arg("-c")
        .arg(q::python::PYTHON_SCRIPT)
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
    let mut buf = vec![0u8; 8192];
    let mut spinner_stopped = false;
    let mut spinner_opt = Some(spinner);

    let title = format!("q ─ stream ─ {model}");
    let mut box_printer = StreamingBox::new(&title);

    loop {
        let n = reader
            .read(&mut buf)
            .await
            .context("failed to read python stdout")?;
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
        let first_line = stderr
            .lines()
            .find(|l| !l.is_empty())
            .unwrap_or("unknown error");
        anyhow::bail!("Python gemini_webapi failed: {first_line}");
    }

    Ok(full_text)
}
