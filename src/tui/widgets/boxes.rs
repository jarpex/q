//! Static bordered output boxes and warnings.

use crossterm::{
    execute,
    style::{Color, Print, ResetColor, SetForegroundColor},
};
use std::io::{self, Write};

use super::super::wrap::{display_width, wrap_text};
use super::{get_max_content_width, is_interactive, left_indent, top_border_with_title};

/// Prints the given text inside a bordered box with a title.
///
/// If the terminal is not interactive, the text is printed without the box.
#[allow(clippy::print_stdout)]
pub fn print_in_box(text: &str, title: &str) {
    if !is_interactive() {
        println!("{text}");
        return;
    }

    let max_content = get_max_content_width();
    let mut stdout = io::stdout();

    let _ = execute!(
        stdout,
        Print(left_indent()),
        SetForegroundColor(Color::DarkGrey),
        Print(top_border_with_title(title, max_content + 2)),
        Print("\n"),
        ResetColor
    );

    let trimmed = text.trim_end_matches(['\r', '\n']);
    let paragraphs: Vec<&str> = trimmed.split('\n').collect();
    for (i, para) in paragraphs.iter().enumerate() {
        let wrapped = wrap_text(para.trim_end_matches('\r'), max_content);
        for line in wrapped {
            let visible_len = display_width(&line);
            let padding = max_content.saturating_sub(visible_len);
            let _ = execute!(
                stdout,
                Print(left_indent()),
                SetForegroundColor(Color::DarkGrey),
                Print("│"),
                ResetColor,
                Print(" "),
                Print(line),
                Print(" ".repeat(padding)),
                Print(" "),
                SetForegroundColor(Color::DarkGrey),
                Print("│\n"),
                ResetColor
            );
        }
        if !para.is_empty() && i + 1 < paragraphs.len() {
            let _ = execute!(
                stdout,
                Print(left_indent()),
                SetForegroundColor(Color::DarkGrey),
                Print("│"),
                ResetColor,
                Print(" "),
                Print(" ".repeat(max_content)),
                Print(" "),
                SetForegroundColor(Color::DarkGrey),
                Print("│\n"),
                ResetColor
            );
        }
    }

    let _ = execute!(
        stdout,
        Print(left_indent()),
        SetForegroundColor(Color::DarkGrey),
        Print(format!("╰{}╯\n", "─".repeat(max_content + 2))),
        ResetColor
    );
    let _ = stdout.flush();
}

/// Prints a warning above the output.
///
/// The warning goes to the standard error when the terminal is not
/// interactive, so the piped output stays clean.
#[allow(clippy::print_stderr)]
pub fn print_uc_warning(msg: &str) {
    if !is_interactive() {
        eprintln!("warning: {msg}");
        return;
    }

    let mut stdout = io::stdout();
    let _ = execute!(
        stdout,
        Print(left_indent()),
        SetForegroundColor(Color::DarkYellow),
        Print(format!("warning: {msg}\n")),
        ResetColor
    );
    let _ = stdout.flush();
}

/// Prints the given error message inside a red-bordered box.
///
/// If the terminal is not interactive, the error is printed to stderr without the box.
#[allow(clippy::print_stderr)]
pub fn print_error(msg: &str) {
    let cleaned_msg = msg.replace('\r', "");
    if !is_interactive() {
        eprintln!("error: {cleaned_msg}");
        return;
    }

    let max_content = get_max_content_width();
    let wrapped = wrap_text(&cleaned_msg, max_content);
    let mut stderr = io::stderr();

    let _ = execute!(
        stderr,
        Print(left_indent()),
        SetForegroundColor(Color::DarkRed),
        Print(format!("╭{}╮\n", "─".repeat(max_content + 2))),
        ResetColor
    );

    for line in wrapped {
        let visible_len = display_width(&line);
        let padding = max_content.saturating_sub(visible_len);
        let _ = execute!(
            stderr,
            Print(left_indent()),
            SetForegroundColor(Color::DarkRed),
            Print("│"),
            ResetColor,
            Print(" "),
            SetForegroundColor(Color::Red),
            Print(line),
            ResetColor,
            Print(" ".repeat(padding)),
            Print(" "),
            SetForegroundColor(Color::DarkRed),
            Print("│\n"),
            ResetColor
        );
    }

    let _ = execute!(
        stderr,
        Print(left_indent()),
        SetForegroundColor(Color::DarkRed),
        Print(format!("╰{}╯\n", "─".repeat(max_content + 2))),
        ResetColor
    );
    let _ = stderr.flush();
}

/// Prints the given shell command inside a bordered box with a `$` prompt.
///
/// If the terminal is not interactive, the command is printed as `$ <command>`.
#[allow(clippy::print_stdout)]
pub fn print_command(command: &str, title: &str) {
    if !is_interactive() {
        println!("$ {command}");
        return;
    }

    let max_content = get_max_content_width();
    let wrapped = wrap_text(command, max_content);
    let mut stdout = io::stdout();

    let _ = execute!(
        stdout,
        Print(left_indent()),
        SetForegroundColor(Color::DarkGrey),
        Print(top_border_with_title(title, max_content + 2)),
        Print("\n"),
        ResetColor
    );

    for (idx, line) in wrapped.iter().enumerate() {
        let visible_len = display_width(line);
        let prefix_len = if idx == 0 { 2 } else { 0 };
        let padding = max_content.saturating_sub(visible_len + prefix_len);

        let _ = execute!(
            stdout,
            Print(left_indent()),
            SetForegroundColor(Color::DarkGrey),
            Print("│"),
            ResetColor,
            Print(" "),
        );

        if idx == 0 {
            let _ = execute!(
                stdout,
                SetForegroundColor(Color::DarkGreen),
                Print("$ "),
                ResetColor,
            );
        }

        let _ = execute!(
            stdout,
            Print(line),
            Print(" ".repeat(padding)),
            Print(" "),
            SetForegroundColor(Color::DarkGrey),
            Print("│\n"),
            ResetColor
        );
    }

    let _ = execute!(
        stdout,
        Print(left_indent()),
        SetForegroundColor(Color::DarkGrey),
        Print(format!("╰{}╯\n", "─".repeat(max_content + 2))),
        ResetColor
    );
    let _ = stdout.flush();
}
