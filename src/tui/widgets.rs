use crossterm::{
    cursor, execute,
    style::{Color, Print, ResetColor, SetForegroundColor},
    terminal::{self, Clear, ClearType},
};
use serde_json::Value;
use std::io::{self, IsTerminal, Write};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::Duration;

use super::wrap::{commit_word, display_width, greedy_feed, wrap_text, WrapState};

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
fn get_max_content_width() -> usize {
    let term_width = terminal::size().map_or(80, |(w, _)| w as usize);
    let available = term_width.saturating_sub(MARGIN * 2);
    available.saturating_sub(4).max(1)
}

#[must_use]
fn top_border_with_title(title: &str, inner_width: usize) -> String {
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

/// An animated terminal spinner widget that displays a label while a background task is running.
pub struct Spinner {
    stop_flag: Arc<AtomicBool>,
    label: Arc<Mutex<String>>,
    handle: Option<std::thread::JoinHandle<()>>,
    active: bool,
}

impl Spinner {
    /// Starts a new spinner with the given label.
    ///
    /// If the terminal is not interactive, the spinner is created in an inactive state
    /// and no animation is drawn.
    #[must_use]
    pub fn start(label: &str) -> Self {
        let label_arc = Arc::new(Mutex::new(label.to_owned()));

        if !is_interactive() {
            return Self {
                stop_flag: Arc::new(AtomicBool::new(false)),
                label: label_arc,
                handle: None,
                active: false,
            };
        }

        let stop_flag = Arc::new(AtomicBool::new(false));
        let stop_flag_clone = Arc::clone(&stop_flag);
        let label_clone = Arc::clone(&label_arc);

        let handle = std::thread::spawn(move || {
            let frames = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
            let mut frame_idx = 0;

            let mut stdout = io::stdout();
            let _ = execute!(stdout, cursor::Hide);

            while !stop_flag_clone.load(Ordering::Relaxed) {
                let frame = frames.get(frame_idx % frames.len()).copied().unwrap_or('⠋');
                frame_idx += 1;

                let current = match label_clone.lock() {
                    Ok(lock) => lock.clone(),
                    Err(poisoned) => poisoned.into_inner().clone(),
                };

                let _ = execute!(
                    stdout,
                    cursor::MoveToColumn(0),
                    Clear(ClearType::CurrentLine),
                    Print(left_indent()),
                    SetForegroundColor(Color::DarkGrey),
                    Print(format!("{frame} {current}")),
                    ResetColor
                );
                let _ = stdout.flush();

                std::thread::sleep(Duration::from_millis(80));
            }

            let _ = execute!(
                stdout,
                cursor::MoveToColumn(0),
                Clear(ClearType::CurrentLine),
                cursor::Show
            );
            let _ = stdout.flush();
        });

        Self {
            stop_flag,
            label: label_arc,
            handle: Some(handle),
            active: true,
        }
    }

    /// Updates the label text displayed next to the spinner.
    pub fn set_label(&self, new_label: &str) {
        if let Ok(mut lock) = self.label.lock() {
            new_label.clone_into(&mut lock);
        }
    }

    /// Stops the spinner animation and clears the line it was drawn on.
    pub fn stop_and_rewind(mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for Spinner {
    fn drop(&mut self) {
        if self.active {
            self.stop_flag.store(true, Ordering::Relaxed);
            if let Some(h) = self.handle.take() {
                let _ = h.join();
            }
            if is_interactive() {
                let mut stdout = io::stdout();
                let _ = execute!(stdout, cursor::Show);
                let _ = stdout.flush();
            }
        }
    }
}

/// A streaming output box that wraps and displays text chunks inside a bordered frame in real time.
pub struct StreamingBox {
    line_buffer: String,
    full_text: String,
    raw_text: String,
    pending: String,
    state: WrapState,
    max_content: usize,
    started: bool,
    interactive: bool,
    title: String,
}

impl StreamingBox {
    /// Creates a new streaming box with the given title.
    #[must_use]
    pub fn new(title: &str) -> Self {
        let interactive = is_interactive();
        let max_content = get_max_content_width().max(1);

        Self {
            line_buffer: String::new(),
            full_text: String::new(),
            raw_text: String::new(),
            pending: String::new(),
            state: WrapState::new(),
            max_content,
            started: false,
            interactive,
            title: title.to_owned(),
        }
    }

    /// Draws the top border of the box.
    ///
    /// This is called automatically by `write` if the box has not been started yet.
    pub fn start(&mut self) {
        if !self.interactive {
            self.started = true;
            return;
        }
        let mut stdout = io::stdout();
        let _ = execute!(
            stdout,
            Print(left_indent()),
            SetForegroundColor(Color::DarkGrey),
            Print(top_border_with_title(&self.title, self.max_content + 2)),
            Print("\n"),
            ResetColor
        );
        let _ = stdout.flush();
        self.started = true;
    }

    #[allow(clippy::print_stdout)]
    fn draw_line(&self, line: &str) {
        if !self.interactive {
            println!("{line}");
            return;
        }
        let visible_len = display_width(line);
        let padding = self.max_content.saturating_sub(visible_len);

        let mut stdout = io::stdout();
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
        let _ = stdout.flush();
    }

    /// Writes a chunk of text to the box. Wraps it and draws complete lines.
    pub fn write(&mut self, chunk: &str) {
        if !self.started {
            self.start();
        }
        self.raw_text.push_str(chunk);
        for line in greedy_feed(
            &mut self.line_buffer,
            &mut self.pending,
            &mut self.state,
            &mut self.full_text,
            chunk,
            self.max_content,
        ) {
            self.draw_line(&line);
        }
    }

    /// Flushes any remaining text, draws the bottom border, and returns the full raw text.
    #[must_use]
    pub fn finish(mut self) -> String {
        if !self.started {
            return self.raw_text.trim_end_matches('\n').to_owned();
        }

        if !self.pending.is_empty() || self.state.sep_pending {
            let mut drained: Vec<String> = Vec::new();
            let tail_width = self.state.pending_width;
            commit_word(
                &mut self.line_buffer,
                &mut self.state,
                &mut self.full_text,
                &mut drained,
                &self.pending,
                tail_width,
                false,
                self.max_content,
            );
            for line in drained {
                self.draw_line(&line);
            }
            self.pending = String::new();
            self.state.pending_width = 0;
        }
        if !self.line_buffer.is_empty() {
            let remaining = std::mem::take(&mut self.line_buffer);
            self.draw_line(&remaining);
        }

        if self.interactive {
            let mut stdout = io::stdout();
            let _ = execute!(
                stdout,
                Print(left_indent()),
                SetForegroundColor(Color::DarkGrey),
                Print(format!("╰{}╯\n", "─".repeat(self.max_content + 2))),
                ResetColor
            );
            let _ = stdout.flush();
        }

        self.raw_text.trim_end_matches('\n').to_owned()
    }
}

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
    let wrapped = wrap_text(text, max_content);
    let mut stdout = io::stdout();

    let _ = execute!(
        stdout,
        Print(left_indent()),
        SetForegroundColor(Color::DarkGrey),
        Print(top_border_with_title(title, max_content + 2)),
        Print("\n"),
        ResetColor
    );

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

    let _ = execute!(
        stdout,
        Print(left_indent()),
        SetForegroundColor(Color::DarkGrey),
        Print(format!("╰{}╯\n", "─".repeat(max_content + 2))),
        ResetColor
    );
    let _ = stdout.flush();
}

/// Prints account usage windows in a compact terminal report.
#[allow(clippy::print_stdout)]
pub fn print_usage_limits(usage_info: &Value, _quotas: &Value) {
    let width = get_max_content_width();
    println!();
    for line in usage_limit_lines(usage_info, width) {
        println!("{line}");
    }
    println!();
}

fn usage_limit_lines(usage_info: &Value, width: usize) -> Vec<String> {
    let windows = [
        ("5-hour limit", usage_info.get("current_5h")),
        ("Weekly limit", usage_info.get("weekly")),
    ];
    std::iter::once("Usage limits:".to_owned())
        .chain(
            windows
                .iter()
                .map(|(label, value)| format_usage_line(label, *value, width)),
        )
        .collect()
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
fn format_usage_line(label: &str, window: Option<&Value>, width: usize) -> String {
    const LABEL_WIDTH: usize = 15;
    let bar_width = width.saturating_sub(2 + LABEL_WIDTH + 1 + 4).clamp(3, 24);
    let Some(window_value) = window else {
        return format!("  {label:<LABEL_WIDTH$}Unavailable");
    };
    let percentage = window_value
        .get("usage_percentage")
        .and_then(Value::as_f64)
        .unwrap_or_else(|| {
            window_value
                .get("usage_level")
                .and_then(Value::as_f64)
                .unwrap_or(0.0)
                * 100.0
        })
        .clamp(0.0, 100.0);
    let filled = ((bar_width as f64) * percentage / 100.0).round() as usize;
    let bar = format!(
        "{}{}",
        "█".repeat(filled.min(bar_width)),
        "░".repeat(bar_width.saturating_sub(filled))
    );
    format!("  {label:<LABEL_WIDTH$}{bar} {percentage:>3.0}%")
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

#[cfg(test)]
#[allow(clippy::result_large_err)]
mod tests {
    use super::{format_usage_line, top_border_with_title, usage_limit_lines};
    use serde_json::json;

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

    #[test]
    fn reports_a_missing_window_as_unavailable() {
        let line = format_usage_line("5-hour limit", None, 80);
        assert!(line.contains("Unavailable"), "{line}");
    }

    #[test]
    fn reads_the_percentage_when_it_is_given() {
        let value = json!({"usage_percentage": 42.0});
        let line = format_usage_line("5-hour limit", Some(&value), 80);
        assert!(line.contains("42%"), "{line}");
    }

    #[test]
    fn derives_the_percentage_from_the_level() {
        let value = json!({"usage_level": 0.25});
        let line = format_usage_line("5-hour limit", Some(&value), 80);
        assert!(line.contains("25%"), "{line}");
    }

    #[test]
    fn keeps_the_percentage_within_bounds() {
        let value = json!({"usage_percentage": 150.0});
        let line = format_usage_line("5-hour limit", Some(&value), 80);
        assert!(line.contains("100%"), "{line}");
    }

    #[test]
    fn lists_both_windows() {
        let usage = json!({
        "current_5h": {"usage_percentage": 10.0},
        "weekly": {"usage_percentage": 20.0}
        });
        let lines = usage_limit_lines(&usage, 80);
        assert_eq!(lines.len(), 3);
        assert!(
            lines.first().is_some_and(|l| l.contains("Usage limits")),
            "{lines:?}"
        );
        assert!(
            lines.get(1).is_some_and(|l| l.contains("5-hour limit")),
            "{lines:?}"
        );
        assert!(
            lines.get(2).is_some_and(|l| l.contains("Weekly limit")),
            "{lines:?}"
        );
    }

    #[test]
    fn marks_a_missing_window_as_unavailable() {
        let usage = json!({"current_5h": {"usage_percentage": 10.0}});
        let lines = usage_limit_lines(&usage, 80);
        assert_eq!(lines.len(), 3);
        assert!(lines.get(1).is_some_and(|l| l.contains("10%")), "{lines:?}");
        assert!(
            lines.get(2).is_some_and(|l| l.contains("Unavailable")),
            "{lines:?}"
        );
    }
}
