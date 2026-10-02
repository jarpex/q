use anyhow::{Context, Result};
use std::path::Path;
use std::process::Stdio;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

use crate::config::CookieSet;

const PYTHON_SCRIPT: &str = include_str!("worker.py");

/// Configuration options for spawning the Gemini API background process.
pub struct AskOptions<'a> {
    /// The path to the Python executable.
    pub python_bin: &'a Path,
    /// The authenticated session cookies.
    pub cookies: &'a CookieSet,
    /// The user's query or prompt.
    pub query: &'a str,
    /// The specific AI model to use.
    pub model: &'a str,
    /// Whether to stream the response or process it in batch.
    pub stream: bool,
    /// Whether to enable debug logging.
    pub debug: bool,
}

/// Events emitted by the Gemini API background process.
pub enum GeminiEvent {
    /// A chunk of text received from the API.
    Chunk(String),
    /// An error occurred during the API interaction.
    Error(String),
    /// The API stream has finished successfully.
    Done,
}

/// Spawns a background Python process to handle Gemini API interactions.
///
/// # Errors
///
/// This function will return an error if:
/// - The Python binary fails to spawn.
/// - The system fails to write the query to the process's standard input.
/// - Standard output or error streams cannot be extracted from the child process.
pub async fn spawn_gemini_stream(options: AskOptions<'_>) -> Result<mpsc::Receiver<GeminiEvent>> {
    let AskOptions {
        python_bin,
        cookies,
        query,
        model,
        stream,
        debug,
    } = options;

    if debug {
        // Allowed explicitly to satisfy `clippy::print_stderr` for debug logs
        #[allow(clippy::print_stderr)]
        {
            eprintln!("[debug] Python: {}", python_bin.display());
            eprintln!("[debug] Model: {model}");
            eprintln!("[debug] Stream: {stream}");
        }
    }

    let mode = if stream { "stream" } else { "batch" };

    let mut child = Command::new(python_bin)
        .arg("-c")
        .arg(PYTHON_SCRIPT)
        .arg(&cookies.psid)
        .arg(&cookies.psidts)
        .arg(model)
        .arg(mode)
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

    let (tx, rx) = mpsc::channel(32);

    let stdout_tx = tx.clone();
    tokio::spawn(async move {
        let mut reader = BufReader::new(stdout);
        let mut buf = vec![0u8; 8192];
        loop {
            match reader.read(&mut buf).await {
                Ok(0) => {
                    let _ = stdout_tx.send(GeminiEvent::Done).await;
                    break;
                }
                Ok(n) => {
                    // Use .get() instead of direct slicing to satisfy `clippy::indexing_slicing`
                    let text = String::from_utf8_lossy(buf.get(..n).unwrap_or(&[])).to_string();
                    if stdout_tx.send(GeminiEvent::Chunk(text)).await.is_err() {
                        break;
                    }
                }
                Err(e) => {
                    let _ = stdout_tx.send(GeminiEvent::Error(e.to_string())).await;
                    break;
                }
            }
        }
    });

    let stderr_tx = tx;
    tokio::spawn(async move {
        let mut reader = BufReader::new(stderr);
        let mut buf = Vec::new();
        let _ = reader.read_to_end(&mut buf).await;

        // Renamed outer variable to satisfy `clippy::shadow_reuse`
        let wait_result = child.wait().await;
        if let Ok(status) = wait_result {
            if !status.success() {
                let stderr = String::from_utf8_lossy(&buf);
                let first_line = stderr
                    .lines()
                    .find(|l| !l.is_empty())
                    .unwrap_or("unknown error");
                let _ = stderr_tx
                    .send(GeminiEvent::Error(format!(
                        "Python gemini_webapi failed: {first_line}"
                    )))
                    .await;
            }
        }
    });

    Ok(rx)
}
