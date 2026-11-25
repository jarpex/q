//! Real-time streaming bordered output box.

use crossterm::{
    cursor, execute,
    style::{Color, Print, ResetColor, SetForegroundColor},
    terminal::{self, Clear, ClearType},
};
use std::io::{self, Write};

use super::super::wrap::{
    commit_word, display_width, greedy_feed, in_progress_line, longest_prefix_end, WrapState,
};
use super::{get_max_content_width, is_interactive, left_indent, top_border_with_title};

/// A streaming output box that wraps and displays text chunks inside a bordered frame in real time.
#[allow(clippy::struct_excessive_bools)]
pub struct StreamingBox {
    line_buffer: String,
    full_text: String,
    pending: String,
    state: WrapState,
    max_content: usize,
    started: bool,
    interactive: bool,
    title: String,
    lines_drawn: usize,
    buffer_non_interactive: bool,
    tail_active: bool,
    blank_lines_pending: usize,
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
            pending: String::new(),
            state: WrapState::new(),
            max_content,
            started: false,
            interactive,
            title: title.to_owned(),
            lines_drawn: 0,
            buffer_non_interactive: false,
            tail_active: false,
            blank_lines_pending: 0,
        }
    }

    /// Tells whether the output goes to an interactive terminal.
    #[must_use]
    pub const fn is_interactive(&self) -> bool {
        self.interactive
    }

    /// Buffers the text instead of printing it line-by-line when not interactive.
    pub const fn buffer_non_interactive(&mut self) {
        self.buffer_non_interactive = true;
    }

    /// Erases all lines drawn for this box from the terminal.
    pub fn erase(&self) {
        if !self.interactive || self.lines_drawn == 0 {
            return;
        }
        let height = terminal::size().map_or(self.lines_drawn, |(_, h)| h as usize);
        let count = self.lines_drawn.min(height);
        let mut stdout = io::stdout();
        if self.tail_active {
            let _ = execute!(
                stdout,
                cursor::MoveToColumn(0),
                Clear(ClearType::CurrentLine),
            );
        }
        for _ in 0..count {
            let _ = execute!(
                stdout,
                cursor::MoveUp(1),
                cursor::MoveToColumn(0),
                Clear(ClearType::CurrentLine),
            );
        }
        let _ = stdout.flush();
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
        self.lines_drawn += 1;
        self.started = true;
    }

    #[allow(clippy::print_stdout)]
    fn draw_line(&mut self, line: &str) {
        if !self.interactive {
            if !self.buffer_non_interactive {
                println!("{line}");
            }
            return;
        }
        let raw_len = display_width(line);
        let (display_line, visible_len) = if raw_len > self.max_content {
            let cut = longest_prefix_end(line, self.max_content);
            let truncated = &line[..cut];
            (truncated, display_width(truncated))
        } else {
            (line, raw_len)
        };
        let padding = self.max_content.saturating_sub(visible_len);

        let mut stdout = io::stdout();
        let _ = execute!(
            stdout,
            cursor::MoveToColumn(0),
            Clear(ClearType::CurrentLine),
            Print(left_indent()),
            SetForegroundColor(Color::DarkGrey),
            Print("│"),
            ResetColor,
            Print(" "),
            Print(display_line),
            Print(" ".repeat(padding)),
            Print(" "),
            SetForegroundColor(Color::DarkGrey),
            Print("│\n"),
            ResetColor
        );
        let _ = stdout.flush();
        self.lines_drawn += 1;
        self.tail_active = false;
    }

    fn draw_tail(&mut self) {
        if !self.interactive {
            return;
        }
        let tail = in_progress_line(&self.line_buffer, &self.pending, &self.state);
        if tail.is_empty() {
            if self.tail_active {
                self.clear_tail();
            }
            return;
        }
        let raw_len = display_width(&tail);
        let (display_tail, visible_len) = if raw_len > self.max_content {
            let cut = longest_prefix_end(&tail, self.max_content);
            let truncated = &tail[..cut];
            (truncated, display_width(truncated))
        } else {
            (tail.as_str(), raw_len)
        };
        let padding = self.max_content.saturating_sub(visible_len);

        let mut stdout = io::stdout();
        let _ = execute!(
            stdout,
            cursor::MoveToColumn(0),
            Clear(ClearType::CurrentLine),
            Print(left_indent()),
            SetForegroundColor(Color::DarkGrey),
            Print("│"),
            ResetColor,
            Print(" "),
            Print(display_tail),
            Print(" ".repeat(padding)),
            Print(" "),
            SetForegroundColor(Color::DarkGrey),
            Print("│"),
            ResetColor
        );
        let _ = stdout.flush();
        self.tail_active = true;
    }

    fn clear_tail(&mut self) {
        if !self.interactive || !self.tail_active {
            return;
        }
        let mut stdout = io::stdout();
        let _ = execute!(
            stdout,
            cursor::MoveToColumn(0),
            Clear(ClearType::CurrentLine),
        );
        let _ = stdout.flush();
        self.tail_active = false;
    }

    fn flush_blank_lines(&mut self) {
        if self.interactive {
            while self.blank_lines_pending > 0 {
                self.draw_line("");
                self.blank_lines_pending -= 1;
            }
        } else {
            self.blank_lines_pending = 0;
        }
    }

    /// Writes a chunk of text to the box. Wraps it and draws complete lines.
    pub fn write(&mut self, chunk: &str) {
        if !self.started {
            self.start();
        }
        for line in greedy_feed(
            &mut self.line_buffer,
            &mut self.pending,
            &mut self.state,
            &mut self.full_text,
            chunk,
            self.max_content,
        ) {
            if line.is_empty() {
                self.blank_lines_pending += 1;
            } else {
                self.flush_blank_lines();
                self.draw_line(&line);
            }
        }
        if !self.line_buffer.is_empty() || !self.pending.is_empty() {
            self.flush_blank_lines();
        }
        self.draw_tail();
    }

    /// Flushes any remaining text, draws the bottom border, and returns the full text.
    #[must_use]
    pub fn finish(&mut self) -> String {
        if self.started {
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
                    if line.is_empty() {
                        self.blank_lines_pending += 1;
                    } else {
                        self.flush_blank_lines();
                        self.draw_line(&line);
                    }
                }
                self.pending = String::new();
                self.state.pending_width = 0;
            }
            if self.line_buffer.is_empty() {
                self.clear_tail();
            } else {
                self.flush_blank_lines();
                let remaining = std::mem::take(&mut self.line_buffer);
                self.draw_line(&remaining);
                self.full_text.push_str(&remaining);
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
                self.lines_drawn += 1;
            }
        }

        let trimmed_len = self.full_text.trim_end_matches('\n').len();
        self.full_text.truncate(trimmed_len);
        std::mem::take(&mut self.full_text)
    }
}

#[cfg(test)]
mod tests {
    use super::{is_interactive, StreamingBox};

    #[test]
    fn streaming_box_buffers_non_interactive_and_erases() {
        let mut box_printer = StreamingBox::new("test");
        box_printer.buffer_non_interactive();
        assert_eq!(box_printer.is_interactive(), is_interactive());
        box_printer.write("line one\nline two\n");
        let finished = box_printer.finish();
        assert_eq!(finished, "line one\nline two");
        box_printer.erase();
    }

    #[test]
    fn streaming_box_draws_tail_in_interactive_mode() {
        let mut box_printer = StreamingBox::new("test");
        box_printer.interactive = true;
        box_printer.write("hello");
        assert!(box_printer.tail_active);
        box_printer.write(" world");
        assert!(box_printer.tail_active);
        let finished = box_printer.finish();
        assert_eq!(finished, "hello world");
        assert!(!box_printer.tail_active);
        box_printer.erase();
    }

    #[test]
    fn streaming_box_wraps_in_progress_without_overflowing_border() {
        let mut box_printer = StreamingBox::new("test");
        box_printer.interactive = true;
        box_printer.max_content = 10;
        box_printer.write("hello ");
        assert_eq!(box_printer.lines_drawn, 1);
        // "world" exceeds 5 + 1 + 5 = 11 > 10, so "hello" is drawn as a line
        box_printer.write("world");
        assert_eq!(box_printer.lines_drawn, 2);
        assert!(box_printer.tail_active);
        let finished = box_printer.finish();
        assert_eq!(finished, "hello\nworld");
        box_printer.erase();
    }

    #[test]
    fn streaming_box_renders_newlines_with_extra_blank_line_inside_box() {
        let mut box_printer = StreamingBox::new("test");
        box_printer.interactive = true;
        box_printer.write("line one\nline two\n");
        let finished = box_printer.finish();
        assert_eq!(box_printer.lines_drawn, 5);
        assert_eq!(finished, "line one\nline two");
        box_printer.erase();
    }

    #[test]
    fn streaming_box_multiple_newlines_collapse_to_single_blank_line() {
        let mut box_printer = StreamingBox::new("test");
        box_printer.interactive = true;
        box_printer.write("line one\n\nline two");
        let finished = box_printer.finish();
        assert_eq!(box_printer.lines_drawn, 5);
        assert_eq!(finished, "line one\nline two");
        box_printer.erase();
    }
}
