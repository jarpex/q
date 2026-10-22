use anyhow::{bail, Result};

use crate::provider::{Provider, ProviderEvent, Query};
use crate::tui::wrap::display_width;
use crate::tui::{copy_to_clipboard, print_copied_message, print_in_box, Spinner, StreamingBox};
use crate::uc::Uc;

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

/// Configuration options for the chat mode.
pub struct ChatOptions<'a> {
    /// The provider of the model.
    pub provider: &'a dyn Provider,
    /// The user's natural language query.
    pub query: &'a str,
    /// The specific AI model to use for generation.
    pub model: &'a str,
    /// Whether the response arrives all at once.
    pub no_stream: bool,
    /// Whether to enable verbose debug logging.
    pub debug: bool,
    /// The filter that cleans the text of the model.
    pub uc: &'a Uc,
}

/// Runs the chat mode by formatting the prompt and dispatching the request.
///
/// # Errors
/// Returns an error if spawning the subprocess fails, the API returns an error,
/// or the response exceeds the maximum allowed size.
pub async fn run(options: &ChatOptions<'_>) -> Result<()> {
    // Chat mode must not send local files, user data, or PATH to the API.
    let prompt = format!(
        "{SYSTEM_PROMPT}\n\nUser question: {query}",
        query = options.query
    );

    if options.no_stream {
        run_batch(options, prompt).await
    } else {
        run_stream(options, prompt).await
    }
}

async fn run_batch(options: &ChatOptions<'_>, prompt: String) -> Result<()> {
    let model = options.model; // Borrowed &str, no allocation needed
    let query = Query {
        prompt,
        model: options.model.to_owned(),
        stream: false,
    };
    let mut rx = options.provider.start_query(query).await?;
    let spinner = Spinner::start("Thinking...");

    let mut text = String::new();
    let mut err = String::new();

    while let Some(event) = rx.recv().await {
        match event {
            ProviderEvent::Chunk(chunk) => {
                if text.len() + chunk.len() > MAX_RESPONSE_BYTES {
                    err.push_str("Response exceeded maximum allowed size.");
                    break;
                }
                text.push_str(&chunk);
            }
            ProviderEvent::Error(e) => err = e,
            ProviderEvent::Done => break,
        }
    }

    spinner.stop_and_rewind();

    if !err.is_empty() {
        bail!("{err}");
    }
    ensure_response(&text)?;

    // The text is cleaned before it is shown and before it goes to the
    // clipboard, so both hold the same text.
    let cleaned = options.uc.clean(text).await;
    options.uc.report_failure();

    let title = format!("q ─ batch ─ {model}");
    print_in_box(cleaned.trim_end_matches('\n'), &title);
    if copy_to_clipboard(&cleaned) {
        print_copied_message();
    }
    Ok(())
}

async fn run_stream(options: &ChatOptions<'_>, prompt: String) -> Result<()> {
    let model = options.model;
    let query = Query {
        prompt,
        model: options.model.to_owned(),
        stream: true,
    };
    let mut rx = options.provider.start_query(query).await?;
    let mut spinner = Some(Spinner::start("Thinking..."));

    let title = format!("q ─ stream ─ {model}");
    let mut box_printer = StreamingBox::new(&title);
    let mut err = String::new();
    let mut bytes_read = 0;

    while let Some(event) = rx.recv().await {
        match event {
            ProviderEvent::Chunk(chunk) => {
                if let Some(s) = spinner.take() {
                    s.stop_and_rewind();
                }
                bytes_read += chunk.len();
                if bytes_read > MAX_RESPONSE_BYTES {
                    err.push_str("Response exceeded maximum allowed size.");
                    break;
                }
                // The chunk is cleaned before it is shown, so the box and the
                // clipboard hold the same text. The warning comes before the
                // first line of the box, so it stays above the frame.
                let cleaned = options.uc.clean(chunk).await;
                options.uc.report_failure();
                box_printer.write(&cleaned);
            }
            ProviderEvent::Error(e) => err = e,
            ProviderEvent::Done => break,
        }
    }

    if let Some(s) = spinner.take() {
        s.stop_and_rewind();
    }

    let text = box_printer.finish();

    if !err.is_empty() {
        bail!("{err}");
    }
    ensure_response(&text)?;

    if copy_to_clipboard(&text) {
        print_copied_message();
    }
    Ok(())
}

fn ensure_response(text: &str) -> Result<()> {
    if display_width(text) == 0 {
        bail!("Gemini returned an empty response.");
    }
    Ok(())
}
