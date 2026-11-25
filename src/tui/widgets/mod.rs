//! Terminal widgets, boxes, and report formatters.

mod boxes;
mod spinner;
mod streaming_box;
mod usage;

pub use boxes::{print_command, print_error, print_in_box, print_uc_warning};
pub use spinner::Spinner;
pub use streaming_box::StreamingBox;
pub use usage::print_usage_limits;

use crossterm::terminal;
use std::io::{self, IsTerminal};

use super::wrap::display_width;

const MARGIN: usize = 2;

/// Checks whether the current standard output is an interactive terminal.
///
/// Returns `true` if stdout is a TTY, `false` if it is piped or redirected.
#[must_use]
pub fn is_interactive() -> bool {
    io::stdout().is_terminal()
}

#[must_use]
pub(crate) fn left_indent() -> String {
    " ".repeat(MARGIN)
}

#[must_use]
pub(super) fn get_max_content_width() -> usize {
    let term_width = terminal::size().map_or(80, |(w, _)| w as usize);
    let available = term_width.saturating_sub(MARGIN * 2);
    available.saturating_sub(4).max(1)
}

#[must_use]
pub(super) fn top_border_with_title(title: &str, inner_width: usize) -> String {
    let t = format!(" {title} ");
    let t_len = display_width(&t);
    if t_len >= inner_width {
        return format!("╭{}╮", "─".repeat(inner_width));
    }
    let rest = inner_width - t_len;
    let left = rest / 2;
    let right = rest - left;
    format!("╭{}{}{}╮", "─".repeat(left), t, "─".repeat(right))
}

#[cfg(test)]
mod tests {
    use super::top_border_with_title;

    #[test]
    fn draws_a_border_around_the_title() {
        let border = top_border_with_title("q", 20);
        assert!(border.starts_with('╭'), "{border}");
        assert!(border.ends_with('╮'), "{border}");
        assert!(border.contains(" q "), "{border}");
    }

    #[test]
    fn shortens_a_border_when_the_title_does_not_fit() {
        let border = top_border_with_title("a very long title", 4);
        assert_eq!(border.chars().count(), 6);
        assert!(!border.contains("title"), "{border}");
    }
}
