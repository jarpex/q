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

/// The terminal operations the spinner needs.
///
/// The trait is the seam that lets a test count the commands without a real
/// terminal, which matters on Windows, where the cursor commands do not
/// always reach the output as escape sequences.
trait SpinnerTerminal: Send + Sync {
    /// Hides the cursor, so it does not blink while the spinner runs.
    fn hide_cursor(&self);

    /// Draws one frame of the animation.
    fn draw_frame(&self, frame: char, label: &str);

    /// Erases the line the spinner was drawn on and shows the cursor again.
    fn restore(&self);
}

/// The real terminal.
struct Terminal;

impl SpinnerTerminal for Terminal {
    fn hide_cursor(&self) {
        let mut stdout = io::stdout();
        let _ = execute!(stdout, cursor::Hide);
        let _ = stdout.flush();
    }

    fn draw_frame(&self, frame: char, label: &str) {
        let mut stdout = io::stdout();
        let _ = execute!(
            stdout,
            cursor::MoveToColumn(0),
            Clear(ClearType::CurrentLine),
            Print(left_indent()),
            SetForegroundColor(Color::DarkGrey),
            Print(format!("{frame} {label}")),
            ResetColor
        );
        let _ = stdout.flush();
    }

    fn restore(&self) {
        let mut stdout = io::stdout();
        let _ = execute!(
            stdout,
            cursor::MoveToColumn(0),
            Clear(ClearType::CurrentLine),
            cursor::Show
        );
        let _ = stdout.flush();
    }
}

/// An animated terminal spinner widget that displays a label while a background task is running.
pub struct Spinner {
    stop_flag: Arc<AtomicBool>,
    label: Arc<Mutex<String>>,
    handle: Option<std::thread::JoinHandle<()>>,
    terminal: Arc<dyn SpinnerTerminal>,
}

impl Spinner {
    /// Starts a new spinner with the given label.
    ///
    /// If the terminal is not interactive, the spinner is created in an inactive state
    /// and no animation is drawn.
    #[must_use]
    pub fn start(label: &str) -> Self {
        Self::start_with(label, Arc::new(Terminal), is_interactive())
    }

    /// Starts a new spinner that draws on the given terminal.
    ///
    /// An inactive spinner draws nothing and leaves the terminal alone.
    fn start_with(label: &str, terminal: Arc<dyn SpinnerTerminal>, interactive: bool) -> Self {
        let label_arc = Arc::new(Mutex::new(label.to_owned()));

        if !interactive {
            return Self {
                stop_flag: Arc::new(AtomicBool::new(false)),
                label: label_arc,
                handle: None,
                terminal,
            };
        }

        // The cursor is hidden before the first frame is drawn, so the line
        // stays clean while the spinner runs.
        terminal.hide_cursor();

        let stop_flag = Arc::new(AtomicBool::new(false));
        let stop_flag_clone = Arc::clone(&stop_flag);
        let label_clone = Arc::clone(&label_arc);
        let terminal_clone = Arc::clone(&terminal);

        let handle = std::thread::spawn(move || {
            let frames = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
            let mut frame_idx = 0;

            while !stop_flag_clone.load(Ordering::Relaxed) {
                let frame = frames.get(frame_idx % frames.len()).copied().unwrap_or('⠋');
                frame_idx += 1;

                let current = match label_clone.lock() {
                    Ok(lock) => lock.clone(),
                    Err(poisoned) => poisoned.into_inner().clone(),
                };

                terminal_clone.draw_frame(frame, &current);

                std::thread::sleep(Duration::from_millis(80));
            }
        });

        Self {
            stop_flag,
            label: label_arc,
            handle: Some(handle),
            terminal,
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
        self.stop();
    }

    /// Stops the animation and gives the terminal back.
    ///
    /// The terminal is restored only once, no matter how many times the
    /// spinner is stopped.
    fn stop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        let Some(handle) = self.handle.take() else {
            return;
        };
        let _ = handle.join();
        self.terminal.restore();
    }
}

impl Drop for Spinner {
    fn drop(&mut self) {
        self.stop();
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
        self.raw_text.reserve(chunk.len());
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

/// Prints account usage windows and model quotas in a compact terminal report.
#[allow(clippy::print_stdout)]
pub fn print_usage_limits(usage_info: &Value, quotas: &Value) {
    let width = get_max_content_width();
    let entries = quota_entries(quotas);
    let report = Report::new(width, &entries);

    println!();
    for line in usage_limit_lines(usage_info, &report) {
        println!("{line}");
    }

    if !entries.is_empty() {
        println!();
        for line in quota_limit_lines(&entries, &report) {
            println!("{line}");
        }
    }
    println!();
}

/// The width of the column of the labels.
///
/// The labels of the windows are shorter than this, so the labels of the
/// models have room.
const LABEL_WIDTH: usize = 15;

/// The geometry of the report.
///
/// Every block of the report uses it, so the bars of one block stand over the
/// bars of the other.
struct Report {
    /// The width of the column of the labels.
    label_width: usize,
    /// The width of the bar of the percentage.
    bar_width: usize,
}

impl Report {
    /// Builds the geometry that fits the given width and the quotas.
    fn new(width: usize, entries: &[QuotaEntry<'_>]) -> Self {
        let names_width = entries
            .iter()
            .map(|(name, _)| name.chars().count())
            .max()
            .unwrap_or(0)
            .min(MAX_QUOTA_LABEL_WIDTH);
        let label_width = LABEL_WIDTH.max(names_width);

        let credits_width = entries
            .iter()
            .map(|(_, bucket)| quota_credits(bucket).chars().count())
            .max()
            .unwrap_or(0);
        // The quotas of the models add a column of their own at the end.
        let extra = if credits_width == 0 {
            0
        } else {
            2 + credits_width
        };
        let bar_width = width
            .saturating_sub(2 + label_width + 1 + 4 + extra)
            .clamp(3, 24);

        Self {
            label_width,
            bar_width,
        }
    }
}

fn usage_limit_lines(usage_info: &Value, report: &Report) -> Vec<String> {
    let windows = [
        ("5-hour limit", usage_info.get("current_5h")),
        ("Weekly limit", usage_info.get("weekly")),
    ];
    std::iter::once("Usage limits:".to_owned())
        .chain(
            windows
                .iter()
                .map(|(label, value)| format_usage_line(label, *value, report)),
        )
        .collect()
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
fn format_usage_line(label: &str, window: Option<&Value>, report: &Report) -> String {
    let Some(window_value) = window else {
        return format!("  {label:<width$}Unavailable", width = report.label_width);
    };
    let percentage = window_percentage(window_value);
    let bar = usage_bar(percentage, report.bar_width);
    format!(
        "  {label:<width$}{bar} {percentage:>3.0}%",
        width = report.label_width
    )
}

/// Reads the used percentage of a window.
fn window_percentage(window: &Value) -> f64 {
    window
        .get("usage_percentage")
        .and_then(Value::as_f64)
        .or_else(|| {
            window
                .get("usage_level")
                .and_then(Value::as_f64)
                .map(|level| level * 100.0)
        })
        .unwrap_or(0.0)
        .clamp(0.0, 100.0)
}

/// Builds the bar of a percentage.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
fn usage_bar(percentage: f64, bar_width: usize) -> String {
    let filled = ((bar_width as f64) * percentage / 100.0).round() as usize;
    format!(
        "{}{}",
        "█".repeat(filled.min(bar_width)),
        "░".repeat(bar_width.saturating_sub(filled))
    )
}

/// The largest width of a model name in the quota lines.
const MAX_QUOTA_LABEL_WIDTH: usize = 24;

/// The keys of the quota map that hold something else than a model bucket.
const QUOTA_RESERVED_KEYS: [&str; 2] = ["usage_info", "extra"];

/// A model bucket of the quota map, with the name of the model.
type QuotaEntry<'a> = (String, &'a Value);

/// Builds the lines of the model quotas.
fn quota_limit_lines(entries: &[QuotaEntry<'_>], report: &Report) -> Vec<String> {
    let mut lines = vec!["Model quotas:".to_owned()];
    for (name, bucket) in entries {
        let percentage = window_percentage(bucket);
        let bar = usage_bar(percentage, report.bar_width);
        let credits = quota_credits(bucket);
        lines.push(format!(
            "  {:<width$}{bar} {percentage:>3.0}%  {credits}",
            shorten(name, report.label_width),
            width = report.label_width
        ));
    }
    lines
}

/// Collects the buckets of the models out of the quota map.
///
/// The quota map holds one bucket per model. It also holds a copy of the usage
/// info and the caps of the extra features, which have another shape and are
/// left out.
fn quota_entries(quotas: &Value) -> Vec<QuotaEntry<'_>> {
    let Some(buckets) = quotas.as_object() else {
        return Vec::new();
    };

    let mut entries: Vec<QuotaEntry<'_>> = buckets
        .iter()
        .filter(|(key, _)| !QUOTA_RESERVED_KEYS.contains(&key.as_str()))
        .filter(|(_, bucket)| bucket.is_object())
        .map(|(key, bucket)| (model_name(bucket, key), bucket))
        .collect();
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    entries
}

/// The name of a model, without the id of its bucket.
fn model_name(bucket: &Value, key: &str) -> String {
    let label = bucket.get("label").and_then(Value::as_str).unwrap_or(key);
    label.split(" [").next().unwrap_or(label).to_owned()
}

/// The credits left in a quota bucket.
fn quota_credits(bucket: &Value) -> String {
    let total = bucket.get("total").and_then(Value::as_i64);
    let left = bucket.get("remaining").and_then(Value::as_i64);
    match (left, total) {
        (Some(0), Some(0)) => "Unlimited".to_owned(),
        (Some(remaining), Some(credits)) => format!("{remaining}/{credits}"),
        _ => "Unavailable".to_owned(),
    }
}

/// Shortens a name that does not fit the column.
fn shorten(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_owned();
    }
    let mut short: String = text.chars().take(width.saturating_sub(1)).collect();
    short.push('…');
    short
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

#[cfg(test)]
#[allow(clippy::result_large_err)]
mod tests {
    use super::{
        format_usage_line, model_name, quota_credits, quota_entries, quota_limit_lines, shorten,
        top_border_with_title, usage_limit_lines, Report, Spinner, SpinnerTerminal,
    };
    use serde_json::json;
    use std::sync::{Arc, Mutex, PoisonError};

    /// The geometry of a report without any quota of the models.
    fn report(width: usize) -> Report {
        Report::new(width, &[])
    }

    /// The commands the terminal received, in the order they arrived.
    #[derive(Debug, PartialEq, Eq, Clone, Copy)]
    enum Command {
        Hide,
        Frame,
        Restore,
    }

    /// The log of the recorded commands.
    type Log = Arc<Mutex<Vec<Command>>>;

    /// A terminal that records the commands instead of drawing them.
    struct RecordingTerminal {
        log: Log,
    }

    impl SpinnerTerminal for RecordingTerminal {
        fn hide_cursor(&self) {
            self.record(Command::Hide);
        }

        fn draw_frame(&self, _frame: char, _label: &str) {
            self.record(Command::Frame);
        }

        fn restore(&self) {
            self.record(Command::Restore);
        }
    }

    impl RecordingTerminal {
        fn record(&self, command: Command) {
            let mut log = self.log.lock().unwrap_or_else(PoisonError::into_inner);
            log.push(command);
        }
    }

    /// Builds a recording terminal and the log it writes to.
    fn recording_terminal() -> (Arc<RecordingTerminal>, Log) {
        let log: Log = Arc::new(Mutex::new(Vec::new()));
        let terminal = RecordingTerminal {
            log: Arc::clone(&log),
        };
        (Arc::new(terminal), log)
    }

    /// Returns the recorded commands.
    fn recorded(log: &Log) -> Vec<Command> {
        log.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    #[test]
    fn restores_the_terminal_once_after_stop_and_rewind() {
        let (terminal, log) = recording_terminal();
        let spinner = Spinner::start_with("test", terminal, true);
        spinner.stop_and_rewind();

        let commands = recorded(&log);
        let restores = commands.iter().filter(|c| **c == Command::Restore).count();
        assert_eq!(restores, 1, "{commands:?}");
        assert_eq!(commands.first(), Some(&Command::Hide), "{commands:?}");
        assert_eq!(commands.last(), Some(&Command::Restore), "{commands:?}");
    }

    #[test]
    fn restores_the_terminal_once_when_dropped() {
        let (terminal, log) = recording_terminal();
        drop(Spinner::start_with("test", terminal, true));

        let commands = recorded(&log);
        let restores = commands.iter().filter(|c| **c == Command::Restore).count();
        assert_eq!(restores, 1, "{commands:?}");
    }

    #[test]
    fn leaves_the_terminal_alone_when_it_is_not_interactive() {
        let (terminal, log) = recording_terminal();
        let spinner = Spinner::start_with("test", terminal, false);
        spinner.stop_and_rewind();

        assert_eq!(recorded(&log), Vec::new());
    }

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
        let line = format_usage_line("5-hour limit", None, &report(80));
        assert!(line.contains("Unavailable"), "{line}");
    }

    #[test]
    fn reads_the_percentage_when_it_is_given() {
        let value = json!({"usage_percentage": 42.0});
        let line = format_usage_line("5-hour limit", Some(&value), &report(80));
        assert!(line.contains("42%"), "{line}");
    }

    #[test]
    fn derives_the_percentage_from_the_level() {
        let value = json!({"usage_level": 0.25});
        let line = format_usage_line("5-hour limit", Some(&value), &report(80));
        assert!(line.contains("25%"), "{line}");
    }

    #[test]
    fn keeps_the_percentage_within_bounds() {
        let value = json!({"usage_percentage": 150.0});
        let line = format_usage_line("5-hour limit", Some(&value), &report(80));
        assert!(line.contains("100%"), "{line}");
    }

    #[test]
    fn lists_both_windows() {
        let usage = json!({
        "current_5h": {"usage_percentage": 10.0},
        "weekly": {"usage_percentage": 20.0}
        });
        let lines = usage_limit_lines(&usage, &report(80));
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
        let lines = usage_limit_lines(&usage, &report(80));
        assert_eq!(lines.len(), 3);
        assert!(lines.get(1).is_some_and(|l| l.contains("10%")), "{lines:?}");
        assert!(
            lines.get(2).is_some_and(|l| l.contains("Unavailable")),
            "{lines:?}"
        );
    }

    #[test]
    fn the_bars_of_the_two_blocks_stand_over_each_other() {
        let usage = json!({
            "current_5h": {"usage_percentage": 4.0},
            "weekly": {"usage_percentage": 4.0}
        });
        let quotas = json!({
            "None-11": {
                "usage_percentage": 3,
                "total": 12096,
                "remaining": 11627,
                "label": "Gemini Flash [None-11]"
            },
            "None-4": {
                "usage_percentage": 3,
                "total": 12096,
                "remaining": 11627,
                "label": "Gemini Pro [None-4]"
            }
        });

        let entries = quota_entries(&quotas);
        let report = Report::new(80, &entries);
        let limits = usage_limit_lines(&usage, &report);
        let models = quota_limit_lines(&entries, &report);

        // The bar of every line starts at the same column.
        let at = |line: &str| line.find('█').or_else(|| line.find('░'));
        let first = at(limits.get(1).map(String::as_str).unwrap_or_default());
        for line in limits.iter().skip(1).chain(models.iter().skip(1)) {
            assert_eq!(at(line), first, "the bars do not line up: {line:?}");
        }
    }

    #[test]
    fn the_bars_of_the_two_blocks_have_the_same_width() {
        let usage = json!({"current_5h": {"usage_percentage": 4.0}});
        let quotas = json!({
            "None-11": {
                "usage_percentage": 3,
                "total": 12096,
                "remaining": 11627,
                "label": "Gemini Flash [None-11]"
            }
        });

        let entries = quota_entries(&quotas);
        let report = Report::new(80, &entries);
        let limits = usage_limit_lines(&usage, &report);
        let models = quota_limit_lines(&entries, &report);

        let width_of = |line: &str| -> usize {
            line.chars()
                .filter(|character| *character == '█' || *character == '░')
                .count()
        };
        let limit = limits.get(1).map(String::as_str).unwrap_or_default();
        let model = models.get(1).map(String::as_str).unwrap_or_default();
        assert_eq!(width_of(limit), width_of(model), "{limit:?} and {model:?}");
    }

    #[test]
    fn a_long_model_name_is_shortened_to_the_column() {
        let quotas = json!({
            "None-11": {
                "usage_percentage": 3,
                "total": 12096,
                "remaining": 11627,
                "label": "A very very very long model name [None-11]"
            }
        });

        let entries = quota_entries(&quotas);
        let report = Report::new(80, &entries);
        let models = quota_limit_lines(&entries, &report);

        // The name is shortened to the width of the column.
        let line = models.get(1).map(String::as_str).unwrap_or_default();
        assert!(line.contains('…'), "{line}");
    }

    #[test]
    fn a_model_name_that_fits_stays_whole() {
        let quotas = json!({
            "None-11": {
                "usage_percentage": 3,
                "total": 12096,
                "remaining": 11627,
                "label": "Gemini Flash [None-11]"
            }
        });

        let entries = quota_entries(&quotas);
        let report = Report::new(80, &entries);
        let models = quota_limit_lines(&entries, &report);

        let line = models.get(1).map(String::as_str).unwrap_or_default();
        assert!(line.contains("Gemini Flash"), "{line}");
        assert!(!line.contains('…'), "{line}");
    }

    #[test]
    fn lists_the_model_quotas() {
        let quotas = json!({
            "None-11": {
                "usage_percentage": 15,
                "total": 600,
                "remaining": 509,
                "label": "Gemini Flash [None-11]"
            },
            "None-4": {
                "usage_percentage": 15,
                "total": 600,
                "remaining": 509,
                "label": "Gemini Pro [None-4]"
            }
        });

        let lines = quota_limit_lines(&quota_entries(&quotas), &report(80));
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert!(
            lines.first().is_some_and(|l| l.contains("Model quotas")),
            "{lines:?}"
        );
        assert!(
            lines.get(1).is_some_and(|l| l.contains("Gemini Flash")),
            "{lines:?}"
        );
        assert!(
            lines.get(1).is_some_and(|l| l.contains("509/600")),
            "{lines:?}"
        );
        assert!(lines.get(1).is_some_and(|l| l.contains("15%")), "{lines:?}");
        assert!(
            lines.get(2).is_some_and(|l| l.contains("Gemini Pro")),
            "{lines:?}"
        );
    }

    #[test]
    fn leaves_out_the_reserved_keys_of_the_quotas() {
        let quotas = json!({
            "usage_info": {"current_5h": {"usage_percentage": 15}},
            "extra": {"default": {"is_blocked": false}},
            "None-11": {"usage_percentage": 15, "total": 600, "remaining": 509}
        });

        let lines = quota_limit_lines(&quota_entries(&quotas), &report(80));
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(
            lines.get(1).is_some_and(|l| l.contains("None-11")),
            "{lines:?}"
        );
    }

    #[test]
    fn shows_no_quota_lines_without_buckets() {
        assert_eq!(quota_entries(&json!({})).len(), 0);
        assert_eq!(quota_entries(&json!({"usage_info": {}})).len(), 0);
        assert_eq!(quota_entries(&json!({"extra": {}})).len(), 0);
    }

    #[test]
    fn reads_the_credits_of_a_bucket() {
        let bucket = json!({"total": 600, "remaining": 509});
        assert_eq!(quota_credits(&bucket), "509/600");

        let empty = json!({"total": 0, "remaining": 0});
        assert_eq!(quota_credits(&empty), "Unlimited");

        let missing = json!({});
        assert_eq!(quota_credits(&missing), "Unavailable");
    }

    #[test]
    fn drops_the_id_of_a_bucket_from_the_name() {
        let bucket = json!({"label": "Gemini Flash [None-11]"});
        assert_eq!(model_name(&bucket, "None-11"), "Gemini Flash");

        let bare = json!({});
        assert_eq!(model_name(&bare, "None-4"), "None-4");
    }

    #[test]
    fn shortens_a_name_that_does_not_fit() {
        assert_eq!(shorten("Gemini Flash", 20), "Gemini Flash");
        assert_eq!(shorten("Gemini Flash Thinking", 10), "Gemini Fl…");
        assert_eq!(shorten("Gemini Flash Thinking", 10).chars().count(), 10);
    }
}
