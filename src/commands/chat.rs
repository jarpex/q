use anyhow::Result;
use std::path::Path;

use crate::cli::Cli;
use crate::config::CookieSet;
use crate::gemini::{spawn_gemini_stream, AskOptions, GeminiEvent};
use crate::tui::{print_copied_message, print_error, Spinner, StreamingBox};

use super::shell::context::SystemContext;

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

/// Runs the chat mode by collecting system context, formatting the prompt,
/// and dispatching the request to either batch or streaming mode.
///
/// # Errors
///
/// Returns an error if:
/// - Spawning the Gemini subprocess fails.
/// - The Python subprocess returns an error during execution.
/// - Reading from the subprocess stream fails.
pub async fn run(cli: &Cli, python_bin: &Path, cookies: &CookieSet, query: &str) -> Result<()> {
    let sys_ctx = SystemContext::collect().await;
    let system_context_str = sys_ctx.to_prompt_context();

    let plain_query =
        format!("{PLAIN_TEXT_SYSTEM_PROMPT}\n\n{system_context_str}\n\nUser question: {query}");

    let options = AskOptions {
        python_bin,
        cookies,
        query: &plain_query,
        model: &cli.model,
        stream: !cli.no_stream,
        debug: cli.debug,
    };

    if cli.no_stream {
        run_batch_mode(options).await?;
    } else {
        run_stream_mode(options).await?;
    }
    Ok(())
}

async fn run_batch_mode(options: AskOptions<'_>) -> Result<()> {
    let model = options.model.to_owned();
    let spinner = Spinner::start("Thinking...");
    let mut rx = spawn_gemini_stream(options).await?;

    let mut full_text = String::new();
    let mut error_msg = String::new();

    while let Some(event) = rx.recv().await {
        match event {
            GeminiEvent::Chunk(text) => full_text.push_str(&text),
            GeminiEvent::Error(e) => error_msg = e,
            GeminiEvent::Done => break,
        }
    }
    spinner.stop_and_rewind();

    if !error_msg.is_empty() {
        print_error(&error_msg);
        anyhow::bail!(error_msg);
    }

    let title = format!("q ─ batch ─ {model}");
    crate::tui::print_in_box(&full_text, &title);
    if crate::tui::copy_to_clipboard(&full_text) {
        print_copied_message();
    }
    Ok(())
}

async fn run_stream_mode(options: AskOptions<'_>) -> Result<()> {
    let model = options.model.to_owned();
    let mut spinner = Some(Spinner::start("Thinking..."));
    let mut rx = spawn_gemini_stream(options).await?;

    let title = format!("q ─ stream ─ {model}");
    let mut box_printer = StreamingBox::new(&title);
    let mut error_msg = String::new();

    while let Some(event) = rx.recv().await {
        match event {
            GeminiEvent::Chunk(text) => {
                if let Some(s) = spinner.take() {
                    s.stop_and_rewind();
                }
                box_printer.write(&text);
            }
            GeminiEvent::Error(e) => {
                error_msg = e;
            }
            GeminiEvent::Done => break,
        }
    }

    if let Some(s) = spinner.take() {
        s.stop_and_rewind();
    }

    let full_text = box_printer.finish();

    if !error_msg.is_empty() {
        print_error(&error_msg);
        anyhow::bail!(error_msg);
    }

    if crate::tui::copy_to_clipboard(&full_text) {
        print_copied_message();
    }
    Ok(())
}
