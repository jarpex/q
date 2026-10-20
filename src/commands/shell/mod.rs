/// Manages system context and environment details for shell command generation.
pub mod context;

/// Provides validation helpers for checking if shell tools exist on the system.
pub mod tools;

use anyhow::Result;
use std::fmt::Write as FmtWrite;
use std::path::Path;

use crate::config::CookieSet;
use crate::gemini::{spawn_gemini_stream, AskOptions, GeminiEvent};
use crate::tui::{print_command, print_copied_message, Spinner};

use self::context::SystemContext;
use self::tools::validate_tool;

/// Maximum number of retry attempts for command generation.
const MAX_ATTEMPTS: u32 = 3;

/// Known markdown language identifiers to skip when parsing LLM responses.
const MARKDOWN_LANGS: &[&str] = &[
    "bash",
    "sh",
    "zsh",
    "powershell",
    "cmd",
    "shell",
    "ksh",
    "fish",
    "bat",
    "dos",
];

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
    let mut last_error = String::new();

    let mut spinner = Some(Spinner::start("Thinking..."));

    for attempt in 1..=MAX_ATTEMPTS {
        if attempt > 1 {
            if let Some(s) = spinner.as_ref() {
                s.set_label(&format!("Retrying ({attempt}/{MAX_ATTEMPTS})..."));
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

    anyhow::bail!("Failed to generate a valid command after {MAX_ATTEMPTS} attempts")
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

    // Skip environment variable assignments (e.g., FOO=bar) to find the executable.
    let first_tool = first_command
        .split_whitespace()
        .find(|token| !(token.contains('=') && !token.starts_with('-') && !token.starts_with('=')))
        .unwrap_or("");

    if first_tool.is_empty() {
        anyhow::bail!("Generated command was empty or contained only assignments.");
    }

    // The check runs on the blocking pool. It is fast, but the executor must not
    // stall on filesystem I/O.
    let tool_to_validate = first_tool.to_owned();
    let is_valid = tokio::task::spawn_blocking(move || validate_tool(&tool_to_validate))
        .await
        .unwrap_or(false);

    if !is_valid {
        anyhow::bail!("Tool '{first_tool}' does not exist on this system.");
    }

    Ok(command)
}

fn build_prompt(ctx: &SystemContext, query: &str, last_error: &str) -> String {
    let system_context = ctx.to_prompt_context();
    let shell = &ctx.shell;
    let os_info = &ctx.os_info;

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
         - The command MUST work when executed. Test it mentally before responding."
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

/// Checks whether the text is a markdown language identifier, such as `bash`.
fn is_language(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    MARKDOWN_LANGS.contains(&lower.as_str())
}

/// Checks whether the text after a fence marker holds the command itself.
fn is_attached_code(text: &str) -> bool {
    !text.is_empty() && !is_language(text) && text.contains(char::is_whitespace)
}

/// Removes a leading markdown language identifier from the code.
///
/// A flag after the identifier means the identifier is the command itself,
/// so `zsh -c 'ls'` stays intact.
fn strip_language(text: &str) -> &str {
    let Some((head, rest)) = text.split_once(char::is_whitespace) else {
        return text;
    };
    if !is_language(head) {
        return text;
    }
    let code = rest.trim_start();
    if code.is_empty() || code.starts_with('-') {
        return text;
    }
    code
}

/// Removes leading shell prompts such as `$ ` and `# `.
fn strip_prompt(line: &str) -> &str {
    let mut rest = line.trim();
    loop {
        let head = if let Some(dollar) = rest.strip_prefix("$ ") {
            dollar
        } else if let Some(hash) = rest.strip_prefix("# ") {
            hash
        } else {
            return rest;
        };
        rest = head.trim_start();
    }
}

/// Parses a raw LLM response into a clean shell command string.
///
/// Robustly extracts content from markdown code blocks using an iterative approach (no regex).
/// Handles edge cases like attached language identifiers (e.g., ` ```bash ls -la `)
/// and strips leading `$ ` or `# ` prompts.
///
/// Returns an empty string when the response has no command. A code block that
/// holds no content yields an empty string, never the raw fence markers.
pub fn parse_command(response: &str) -> String {
    let mut in_block = false;
    let mut block_content = String::new();
    let mut found_block = false;
    let mut plain_line = String::new();

    for line in response.lines() {
        let trimmed = line.trim();
        // The prompt can hide a fence marker, so it is removed before the check.
        let candidate = strip_prompt(trimmed);

        if candidate.starts_with("```") {
            if in_block {
                break; // Closing backticks found
            }
            found_block = true;

            // The fence can carry the code on the same line: ```ls -la```
            let run = candidate.chars().take_while(|c| *c == '`').count();
            let rest = &candidate[run..];
            let Some(end) = rest.find("```") else {
                // The block continues on the next lines.
                in_block = true;
                let attached = rest.trim();
                if is_attached_code(attached) {
                    block_content.push_str(strip_language(attached));
                    block_content.push('\n');
                }
                continue;
            };

            // The fence closes on the same line, so the block is complete.
            let attached = rest[..end].trim();
            if is_attached_code(attached) {
                block_content.push_str(strip_language(attached));
                block_content.push('\n');
            }
            break;
        }

        if in_block {
            block_content.push_str(trimmed);
            block_content.push('\n');
        } else if plain_line.is_empty() && !trimmed.is_empty() {
            trimmed.clone_into(&mut plain_line);
        }
    }

    if !found_block {
        return strip_prompt(&plain_line).to_owned();
    }

    let mut lines = block_content
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty());
    let Some(first_line) = lines.next() else {
        return String::new();
    };
    strip_prompt(first_line).to_owned()
}

#[cfg(test)]
#[allow(clippy::result_large_err)]
mod tests {
    use super::{extract_first_command, parse_command};

    #[test]
    fn extracts_command_from_fenced_block() {
        assert_eq!(parse_command("```bash\nls -la\n```"), "ls -la");
        assert_eq!(parse_command("```\nls -la\n```"), "ls -la");
        assert_eq!(parse_command("```\nls -la"), "ls -la");
    }

    #[test]
    fn keeps_plain_response_without_fence() {
        assert_eq!(parse_command("ls -la"), "ls -la");
        assert_eq!(parse_command("just a note"), "just a note");
    }

    #[test]
    fn drops_prompt_prefix() {
        assert_eq!(parse_command("```\n$ ls -la\n```"), "ls -la");
        assert_eq!(parse_command("```\n# ls -la\n```"), "ls -la");
    }

    #[test]
    fn empty_block_yields_no_command() {
        assert_eq!(parse_command("```\n```"), "");
        assert_eq!(parse_command("```bash\n```"), "");
        assert_eq!(parse_command("```bash"), "");
        assert_eq!(parse_command(""), "");
    }

    #[test]
    fn fence_never_reaches_the_command() {
        for response in [
            "```\n```",
            "```bash\n```",
            "```ls -la```",
            "```bash ls -la```",
            "```\n```\nmore text",
            "```",
            "``` ```foo",
            " $ $ ````",
            "````",
            "``````",
        ] {
            let command = parse_command(response);
            assert!(!command.starts_with("```"), "fence leaked: {command:?}");
        }
    }

    #[test]
    fn drops_the_language_identifier() {
        assert_eq!(parse_command("```bash ls -la```"), "ls -la");
        assert_eq!(parse_command("```bash ls -la"), "ls -la");
    }

    #[test]
    fn keeps_a_flag_after_the_language() {
        assert_eq!(parse_command("```zsh -c 'ls'```"), "zsh -c 'ls'");
    }

    #[test]
    fn finds_fence_hidden_behind_a_prompt() {
        assert_eq!(parse_command("$ ```bash\nls -la\n```"), "ls -la");
        assert_eq!(parse_command("# ```\nls -la\n```"), "ls -la");
    }

    #[test]
    fn takes_first_line_of_a_plain_response() {
        assert_eq!(parse_command("line1\nline2"), "line1");
        assert_eq!(parse_command("\n\nls -la\nmore"), "ls -la");
    }

    #[test]
    fn command_is_always_one_line() {
        for response in ["```\na\nb\n```", "```\nls -la\n```", "plain\ntext"] {
            assert!(!parse_command(response).contains('\n'));
        }
    }

    #[test]
    fn extracts_first_command_before_separator() {
        assert_eq!(extract_first_command("ls -la | tee out"), "ls -la ");
        assert_eq!(extract_first_command("ls -la && echo done"), "ls -la ");
        assert_eq!(extract_first_command("ls -la; echo done"), "ls -la");
        assert_eq!(extract_first_command("ls -la\nnext"), "ls -la");
    }

    #[test]
    fn keeps_separator_inside_quotes() {
        assert_eq!(extract_first_command("echo 'a | b'"), "echo 'a | b'");
        assert_eq!(extract_first_command("echo \"a | b\""), "echo \"a | b\"");
    }

    #[test]
    fn returns_whole_input_without_separator() {
        assert_eq!(extract_first_command("ls -la"), "ls -la");
        assert_eq!(extract_first_command(""), "");
    }
}
