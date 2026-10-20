use arboard::Clipboard;
use crossterm::{
    execute,
    style::{Color, Print, ResetColor, SetForegroundColor},
};
use std::io::{self, Write};

use super::widgets::{is_interactive, left_indent};

#[cfg(target_os = "macos")]
const PASTE_SHORTCUT: &str = "⌘V";
#[cfg(not(target_os = "macos"))]
const PASTE_SHORTCUT: &str = "Ctrl+V";

/// Copies the given text to the system clipboard.
///
/// Returns `true` if the text was successfully copied, `false` otherwise.
/// The `Q_DISABLE_CLIPBOARD` environment variable turns the copy off, which
/// keeps headless runs from touching the clipboard of the user.
#[must_use]
pub fn copy_to_clipboard(text: &str) -> bool {
    if std::env::var_os("Q_DISABLE_CLIPBOARD").is_some() {
        return false;
    }
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
        Print(format!(
            "(copied to clipboard · {PASTE_SHORTCUT} to paste)\n"
        )),
        ResetColor
    );
    let _ = stdout.flush();
}

#[cfg(test)]
#[allow(clippy::result_large_err)]
mod tests {
    use super::copy_to_clipboard;
    use crate::test_support::EnvGuard;

    #[test]
    fn does_not_copy_when_the_clipboard_is_disabled() {
        let _guard = EnvGuard::new();
        std::env::set_var("Q_DISABLE_CLIPBOARD", "1");

        assert!(!copy_to_clipboard("text"));

        std::env::remove_var("Q_DISABLE_CLIPBOARD");
    }

    #[test]
    fn never_panics_when_the_clipboard_is_unavailable() {
        // The result depends on the environment, so only the absence of a
        // panic is checked here.
        let _guard = EnvGuard::new();
        let _ = copy_to_clipboard("text");
    }
}
