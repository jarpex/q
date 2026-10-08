use anyhow::{bail, Result};
use std::path::Path;

use crate::cli::Cli;
use crate::config::CookieSet;
use crate::gemini::{spawn_gemini_stream, AskOptions, GeminiEvent};
use crate::tui::{
    copy_to_clipboard, print_copied_message, print_error, print_in_box, Spinner, StreamingBox,
};

const SYSTEM_PROMPT: &str = "\
Respond in plain text only. Follow these rules strictly:
1. NO markdown: no **bold**, no *italics*, no _underscores_, no `code`, no # headers, no > quotes, no - lists with markers.
2. NO tables whatsoever.
3. NO bullet points, NO numbered lists.
4. Be EXTREMELY concise: respond in 2-4 SHORT sentences total. One compact paragraph.
5. NO empty lines within the answer — the entire response must be a single block of text.
6. NO service tags like <Image/>, <Elicitation>, <ElicitationsGroup>.
7. Respond in the SAME language as the user's question.
8. Do NOT include any preamble like 'Sure!' or 'Here is...'. Just answer directly.
9. If the question asks for a formula, give the formula inline with a brief explanation.";

/// Hard limit to prevent OOM from runaway LLM streams (1 MB).
/// Suckless philosophy dictates strict bounds on resource consumption.
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

/// Runs the chat mode by formatting the prompt and dispatching the request.
///
/// # Errors
/// Returns an error if spawning the subprocess fails, the API returns an error,
/// or the response exceeds the maximum allowed size.
pub async fn run(cli: &Cli, python_bin: &Path, cookies: &CookieSet, query: &str) -> Result<()> {
    // SECURITY & MINIMALISM:
    // Removed SystemContext::collect(). Chat mode MUST NOT leak local filesystem,
    // user info, or PATH to external APIs. This was a severe privacy flaw.
    let prompt = format!("{SYSTEM_PROMPT}\n\nUser question: {query}");

    let options = AskOptions {
        python_bin,
        cookies,
        query: &prompt,
        model: &cli.model,
        stream: !cli.no_stream,
        debug: cli.debug,
    };

    if cli.no_stream {
        run_batch(options).await
    } else {
        run_stream(options).await
    }
}

async fn run_batch(options: AskOptions<'_>) -> Result<()> {
    let model = options.model; // Borrowed &str, no allocation needed
    let spinner = Spinner::start("Thinking...");
    let mut rx = spawn_gemini_stream(options).await?;

    let mut text = String::new();
    let mut err = String::new();

    while let Some(event) = rx.recv().await {
        match event {
            GeminiEvent::Chunk(chunk) => {
                if text.len() + chunk.len() > MAX_RESPONSE_BYTES {
                    err.push_str("Response exceeded maximum allowed size.");
                    break;
                }
                text.push_str(&chunk);
            }
            GeminiEvent::Error(e) => err = e,
            GeminiEvent::Done => break,
        }
    }

    // Explicit cleanup before processing results
    spinner.stop_and_rewind();

    if !err.is_empty() {
        print_error(&err);
        bail!("{err}");
    }

    let title = format!("q ─ batch ─ {model}");
    print_in_box(&text, &title);
    if copy_to_clipboard(&text) {
        print_copied_message();
    }
    Ok(())
}

async fn run_stream(options: AskOptions<'_>) -> Result<()> {
    let model = options.model;
    let mut spinner = Some(Spinner::start("Thinking..."));
    let mut rx = spawn_gemini_stream(options).await?;

    let title = format!("q ─ stream ─ {model}");
    let mut box_printer = StreamingBox::new(&title);
    let mut err = String::new();
    let mut bytes_read = 0;

    while let Some(event) = rx.recv().await {
        match event {
            GeminiEvent::Chunk(chunk) => {
                if let Some(s) = spinner.take() {
                    s.stop_and_rewind();
                }
                bytes_read += chunk.len();
                if bytes_read > MAX_RESPONSE_BYTES {
                    err.push_str("Response exceeded maximum allowed size.");
                    break;
                }
                box_printer.write(&chunk);
            }
            GeminiEvent::Error(e) => err = e,
            GeminiEvent::Done => break,
        }
    }

    if let Some(s) = spinner.take() {
        s.stop_and_rewind();
    }

    let text = box_printer.finish();

    if !err.is_empty() {
        print_error(&err);
        bail!("{err}");
    }

    if copy_to_clipboard(&text) {
        print_copied_message();
    }
    Ok(())
}
