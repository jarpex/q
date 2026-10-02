use arboard::Clipboard;
use crossterm::{
    execute,
    style::{Color, Print, ResetColor, SetForegroundColor},
};
use std::io::{self, Write};

use super::widgets::{is_interactive, left_indent};

/// Copies the given text to the system clipboard.
///
/// Returns `true` if the text was successfully copied, `false` otherwise.
#[must_use]
pub fn copy_to_clipboard(text: &str) -> bool {
    Clipboard::new().is_ok_and(|mut c| c.set_text(text).is_ok())
}

/// Prints a message indicating that the output was copied to the clipboard.
pub fn print_copied_message() {
    if !is_interactive() {
        return;
    }
    let mut stdout = io::stdout();
    let _ = execute!(
        stdout,
        Print(left_indent()),
        SetForegroundColor(Color::DarkGrey),
        Print("(copied to clipboard · ⌘V to paste)\n"),
        ResetColor
    );
    let _ = stdout.flush();
}
