/// Manages system context and environment details for shell command generation.
pub mod context;

/// Provides validation helpers for checking if shell tools exist on the system.
pub mod tools;

use anyhow::Result;
use std::fmt::Write as FmtWrite;
use std::path::Path;

use crate::config::CookieSet;
use crate::gemini::{spawn_gemini_stream, AskOptions, GeminiEvent};
use crate::tui::{print_command, print_copied_message, print_error, Spinner};

use self::context::SystemContext;
use self::tools::validate_tool;

/// Configuration options for generating shell commands via the Gemini API.
pub struct CommandOptions<'a> {
    /// Path to the Python executable used for API interactions.
    pub python_bin: &'a Path,
    /// The authenticated session cookies for the Gemini API.
    pub cookies: &'a CookieSet,
    /// The user's natural language query.
    pub query: &'a str,
    /// The specific AI model to use for generation.
    pub model: &'a str,
    /// Whether to enable verbose debug logging.
    pub debug: bool,
}

/// Runs command mode: generates a shell command from the user's query.
///
/// # Errors
///
/// Returns an error if:
/// - The Gemini API call fails repeatedly.
/// - No valid command can be generated after 3 attempts.
/// - The generated tool does not exist on the system.
pub async fn run(options: &CommandOptions<'_>) -> Result<()> {
    let ctx = SystemContext::collect().await;
    let title = format!("q ─ command ─ {}", options.model);
    let max_attempts = 3;
    let mut last_error = String::new();

    let mut spinner = Some(Spinner::start("Thinking..."));

    for attempt in 1..=max_attempts {
        if attempt > 1 {
            if let Some(s) = spinner.as_ref() {
                s.set_label(&format!("Retrying ({attempt}/{max_attempts})..."));
            }
        }

        match attempt_generation(options, &ctx, &last_error).await {
            Ok(command) => {
                if let Some(s) = spinner.take() {
                    s.stop_and_rewind();
                }
                print_command(&command, &title);
                if crate::tui::copy_to_clipboard(&command) {
                    print_copied_message();
                }
                return Ok(());
            }
            Err(e) => {
                last_error = e.to_string();
            }
        }
    }

    if let Some(s) = spinner.take() {
        s.stop_and_rewind();
    }
    print_error(&format!(
        "Failed to generate a valid command after {max_attempts} attempts. Last error: {last_error}"
    ));

    anyhow::bail!("Failed to generate a valid command after {max_attempts} attempts")
}

async fn attempt_generation(
    options: &CommandOptions<'_>,
    ctx: &SystemContext,
    last_error: &str,
) -> Result<String> {
    let prompt = build_prompt(ctx, options.query, last_error);

    let response = fetch_command_response(options, &prompt).await?;

    let command = parse_command(&response);
    if command.is_empty() {
        anyhow::bail!("Generated command was empty or invalid markdown.");
    }

    let first_command = extract_first_command(&command);
    let first_tool = first_command.split_whitespace().next().unwrap_or("");

    if first_tool.is_empty() {
        anyhow::bail!("Generated command was empty or invalid markdown.");
    }

    if !validate_tool(first_tool).await {
        anyhow::bail!("Tool '{first_tool}' does not exist on this system.");
    }

    Ok(command)
}

fn build_prompt(ctx: &SystemContext, query: &str, last_error: &str) -> String {
    let system_context = ctx.to_prompt_context();
    let mut prompt = format!(
        "You are a strict {shell} command generator for {os_info}.\n\
         Respond with EXACTLY one raw shell command.\n\
         NO markdown, NO backticks, NO explanation, NO prefix like '$'.\n\
         Provide the COMPLETE command with all necessary flags and options.\n\
         Do NOT truncate or abbreviate the response.\n\n\
         Generate a single, correct {shell} command to fulfill this request:\n\
         Request: \"{query}\"\n\n\
         {system_context}\n\n\
         CRITICAL RULES:\n\
         - Analyze the available tools list and CHOOSE the BEST tool for the task.\n\
         - If modern tools like fd, rg (ripgrep), fzf, bat, exa are available, PREFER them over traditional tools.\n\
         - Generate a command that USES the available tools to accomplish the task, NOT a command to check if tools exist.\n\
         - Use ONLY tools from the available tools list above.\n\
         - Use syntax appropriate for {shell} on {os_info}.\n\
         - Do not invent flags or options. Use standard, well-documented flags.\n\
         - Include ALL necessary flags to fulfill the request completely.\n\
         - Return the FULL command, not abbreviated.\n\
         - The command MUST work when executed. Test it mentally before responding.",
        shell = ctx.shell,
        os_info = ctx.os_info,
        query = query,
        system_context = system_context,
    );

    if !last_error.is_empty() {
        let _ = write!(
            prompt,
            "\n\nPREVIOUS ATTEMPT FAILED: {last_error}\nGenerate a DIFFERENT command."
        );
    }
    prompt
}

async fn fetch_command_response(options: &CommandOptions<'_>, prompt: &str) -> Result<String> {
    let ask_opts = AskOptions {
        python_bin: options.python_bin,
        cookies: options.cookies,
        query: prompt,
        model: options.model,
        stream: false,
        debug: options.debug,
    };

    let mut rx = spawn_gemini_stream(ask_opts)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to spawn stream: {e}"))?;

    let mut response = String::new();
    let mut error_msg = String::new();

    while let Some(event) = rx.recv().await {
        match event {
            GeminiEvent::Chunk(text) => response.push_str(&text),
            GeminiEvent::Error(e) => error_msg = e,
            GeminiEvent::Done => break,
        }
    }

    if !error_msg.is_empty() {
        anyhow::bail!("Request failed: {error_msg}");
    }

    Ok(response)
}

/// Extracts the first command from a pipeline by splitting at unquoted `|`, `&`, `;`, or `\n`.
///
/// Correctly handles shell quoting (`'`, `"`), escapes (`\`), backticks (`` ` ``),
/// and parentheses (`$()`) to avoid splitting inside subshells or strings.
///
/// Returns the original string if no such unquoted delimiters are found.
pub fn extract_first_command(cmd: &str) -> &str {
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut in_backtick = false;
    let mut escape_next = false;
    let mut paren_depth = 0;

    for (i, c) in cmd.char_indices() {
        if escape_next {
            escape_next = false;
            continue;
        }

        if c == '\\' {
            escape_next = true;
            continue;
        }

        if in_single_quote {
            if c == '\'' {
                in_single_quote = false;
            }
            continue;
        }

        if in_double_quote {
            if c == '"' {
                in_double_quote = false;
            }
            continue;
        }

        if in_backtick {
            if c == '`' {
                in_backtick = false;
            }
            continue;
        }

        match c {
            '\'' => in_single_quote = true,
            '"' => in_double_quote = true,
            '`' => in_backtick = true,
            '(' => paren_depth += 1,
            ')' => {
                if paren_depth > 0 {
                    paren_depth -= 1;
                }
            }
            '|' | '&' | ';' | '\n' if paren_depth == 0 => {
                return &cmd[..i];
            }
            _ => {}
        }
    }

    cmd
}

/// Parses a raw LLM response into a clean shell command string.
///
/// Robustly extracts content from markdown code blocks (even if the LLM adds
/// conversational filler before the block) and strips leading `$ ` prompts.
pub fn parse_command(response: &str) -> String {
    // If the response contains a markdown code block, extract its content
    if let Some(start_idx) = response.find("```") {
        let after_marker = &response[start_idx + 3..];
        // Skip the optional language identifier line (e.g., "bash\n")
        let content_start = after_marker.find('\n').map_or(0, |i| i + 1);
        let content = &after_marker[content_start..];

        let end_idx = content.find("```").unwrap_or(content.len());
        let block = &content[..end_idx];

        return block
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or("")
            .trim_start_matches("$ ")
            .trim()
            .to_owned();
    }

    // Fallback: just take the first non-empty line
    response
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .trim_start_matches("$ ")
        .trim()
        .to_owned()
}
