use crossterm::{
    cursor, execute,
    style::{Color, Print, ResetColor, SetForegroundColor},
    terminal::{self, Clear, ClearType},
};
use std::io::{self, IsTerminal, Write};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::Duration;

const MARGIN: usize = 2;

pub(crate) fn is_interactive() -> bool {
    io::stdout().is_terminal()
}

fn left_indent() -> String {
    " ".repeat(MARGIN)
}

fn get_max_content_width() -> usize {
    let term_width = terminal::size().map_or(80, |(w, _)| w as usize);
    let available = term_width.saturating_sub(MARGIN * 2);
    available.saturating_sub(4).max(1)
}

/// Top border of the box with a centered title.
fn top_border_with_title(title: &str, inner_width: usize) -> String {
    let t = format!(" {title} ");
    let t_len = t.chars().count();
    if t_len >= inner_width {
        return format!("╭{}╮", "─".repeat(inner_width));
    }
    let rest = inner_width - t_len;
    let left = rest / 2;
    let right = rest - left;
    format!("╭{}{}{}╮", "─".repeat(left), t, "─".repeat(right))
}

/// Zero-allocation text wrapper that respects character boundaries.
fn wrap_text(text: &str, max_width: usize) -> Vec<&str> {
    let mut lines = Vec::new();
    for line in text.split('\n') {
        if line.is_empty() {
            lines.push("");
            continue;
        }

        let mut start_byte = 0;
        let mut char_count = 0;

        for (i, _) in line.char_indices() {
            if char_count == max_width {
                lines.push(&line[start_byte..i]);
                start_byte = i;
                char_count = 0;
            }
            char_count += 1;
        }
        
        if start_byte < line.len() {
            lines.push(&line[start_byte..]);
        }
    }
    lines
}

// ============== Clipboard ==============

pub(crate) fn copy_to_clipboard(text: &str) -> bool {
    use arboard::Clipboard;
    Clipboard::new().is_ok_and(|mut c| c.set_text(text).is_ok())
}

pub(crate) fn print_copied_message() {
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

// ============== Spinner ==============

pub(crate) struct Spinner {
    stop_flag: Arc<AtomicBool>,
    label: Arc<Mutex<String>>,
    handle: Option<std::thread::JoinHandle<()>>,
    active: bool,
}

impl Spinner {
    pub(crate) fn start(label: &str) -> Self {
        let label_arc = Arc::new(Mutex::new(label.to_string()));

        if !is_interactive() {
            return Self {
                stop_flag: Arc::new(AtomicBool::new(false)),
                label: label_arc,
                handle: None,
                active: false,
            };
        }

        let mut stdout = io::stdout();
        let _ = execute!(stdout, Print(left_indent()), Print("\n"));
        let _ = stdout.flush();

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

    pub(crate) fn set_label(&self, new_label: &str) {
        if let Ok(mut lock) = self.label.lock() {
            *lock = new_label.to_string();
        }
    }

    pub(crate) fn stop_and_rewind(mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }

        if self.active && is_interactive() {
            let mut stdout = io::stdout();
            let _ = execute!(
                stdout,
                cursor::MoveUp(1),
                cursor::MoveToColumn(0),
                Clear(ClearType::CurrentLine)
            );
            let _ = stdout.flush();
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

// ============== StreamingBox ==============

pub(crate) struct StreamingBox {
    line_buffer: String,
    full_text: String,
    max_content: usize,
    started: bool,
    interactive: bool,
    title: String,
}

impl StreamingBox {
    pub(crate) fn new(title: &str) -> Self {
        let interactive = is_interactive();
        let max_content = get_max_content_width();

        Self {
            line_buffer: String::new(),
            full_text: String::new(),
            max_content,
            started: false,
            interactive,
            title: title.to_string(),
        }
    }

    pub(crate) fn start(&mut self) {
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
        let visible_len = line.chars().count();
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

    pub(crate) fn write(&mut self, chunk: &str) {
        self.full_text.push_str(chunk);

        if !self.started {
            self.start();
        }

        self.line_buffer.push_str(chunk);

        while let Some(pos) = self.line_buffer.find('\n') {
            let line: String = self.line_buffer.drain(..pos).collect();
            self.line_buffer.drain(..1);
            self.wrap_and_draw(&line);
        }

        while self.line_buffer.chars().count() > self.max_content {
            let byte_len = self.line_buffer.char_indices().nth(self.max_content).map_or(0, |(i, _)| i);
            let take: String = self.line_buffer.drain(..byte_len).collect();
            self.draw_line(&take);
        }
    }

    fn wrap_and_draw(&self, line: &str) {
        let wrapped = wrap_text(line, self.max_content);
        for part in wrapped {
            self.draw_line(part);
        }
    }

    pub(crate) fn finish(mut self) -> String {
        if !self.line_buffer.is_empty() {
            let remaining = std::mem::take(&mut self.line_buffer);
            self.wrap_and_draw(&remaining);
        }

        if !self.started {
            self.start();
            self.draw_line("");
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

        self.full_text.trim_end_matches('\n').to_string()
    }
}

// ============== Batch box ==============

#[allow(clippy::print_stdout)]
pub(crate) fn print_in_box(text: &str, title: &str) {
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
        let visible_len = line.chars().count();
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

// ============== Error ==============

#[allow(clippy::print_stderr)]
pub(crate) fn print_error(msg: &str) {
    if !is_interactive() {
        eprintln!("error: {msg}");
        return;
    }

    let max_content = get_max_content_width();
    let wrapped = wrap_text(msg, max_content);
    let mut stderr = io::stderr();

    let _ = execute!(
        stderr,
        Print(left_indent()),
        SetForegroundColor(Color::DarkRed),
        Print(format!("╭{}╮\n", "─".repeat(max_content + 2))),
        ResetColor
    );

    for line in wrapped {
        let visible_len = line.chars().count();
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

// ============== Command output ==============

#[allow(clippy::print_stdout)]
pub(crate) fn print_command(command: &str, title: &str) {
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
        let visible_len = line.chars().count();
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