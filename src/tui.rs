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
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const MARGIN: usize = 2;
const CONTENT_COL: usize = MARGIN + 2;

#[must_use]
pub(crate) fn is_interactive() -> bool {
    io::stdout().is_terminal()
}

#[must_use]
fn left_indent() -> String {
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

#[must_use]
fn display_width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// Wraps text into lines no wider than `max_width` terminal columns.
/// Words are kept whole unless wider than the window. Uses UAX#14 break rules.
#[must_use]
fn wrap_text(text: &str, max_width: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let width = max_width.max(1);
    for line in text.split('\n') {
        wrap_line(&mut out, line, width);
    }
    out
}

fn wrap_line(out: &mut Vec<String>, text: &str, max_width: usize) {
    if text.is_empty() {
        out.push(String::new());
        return;
    }

    let mut breaks: Vec<usize> = Vec::new();
    for (i, _) in unicode_linebreak::linebreaks(text) {
        breaks.push(i);
    }
    if breaks.last().is_none_or(|l| *l != text.len()) {
        breaks.push(text.len());
    }

    let mut prev_end = 0;
    let mut prev_had_space = false;
    for br in breaks {
        let raw = &text[prev_end..br];
        prev_end = br;

        let trimmed = raw.trim_end_matches(' ');
        if trimmed.is_empty() {
            continue;
        }
        let sep: &str = if prev_had_space { " " } else { "" };
        pack_word_sep(out, trimmed, sep, max_width);
        prev_had_space = raw.len() != trimmed.len();
    }
}

fn pack_word_sep(out: &mut Vec<String>, word: &str, sep: &str, max_width: usize) {
    let w = display_width(word);
    if w > max_width {
        pack_long_word(out, word, max_width);
        return;
    }
    if let Some(current) = out.last_mut() {
        let cur_w = display_width(current);
        if !current.is_empty()
            && cur_w + display_width(sep) + w <= max_width
        {
            current.push_str(sep);
            current.push_str(word);
            return;
        }
    }
    out.push(word.to_string());
}

/// Hard-cuts a word that doesn't fit the window, splitting by display width.
fn pack_long_word(out: &mut Vec<String>, word: &str, max_width: usize) {
    let mut rest: &str = word;
    while display_width(rest) > max_width {
        let mut cut = 0;
        let mut width = 0;
        for (i, ch) in rest.char_indices() {
            let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
            if width + cw > max_width && cut > 0 {
                break;
            }
            width += cw;
            cut = i + ch.len_utf8();
        }
        if cut == 0 || cut >= rest.len() {
            break;
        }
        out.push(rest[..cut].to_string());
        rest = &rest[cut..];
    }
    if !rest.is_empty() {
        out.push(rest.to_string());
    }
}

// ============== Wrapping tests ==============

#[cfg(test)]
#[must_use]
fn wrapped(text: &str, max_width: usize) -> String {
    wrap_text(text, max_width).join("\n")
}

#[cfg(test)]
#[test]
fn wrap_keeps_short_lines_whole() {
    assert_eq!(wrapped("hello world", 40), "hello world");
}

#[cfg(test)]
#[test]
fn wrap_breaks_at_spaces_when_they_dont_fit() {
    assert_eq!(wrapped("a b c", 3), "a b\nc");
}

#[cfg(test)]
#[test]
fn wrap_packs_words_greedily() {
    assert_eq!(wrapped("a bb cccc", 4), "a bb\ncccc");
}

#[cfg(test)]
#[test]
fn wrap_does_not_split_words_that_fit_in_the_window() {
    assert_eq!(wrapped("aaaa bbbb cccc", 4), "aaaa\nbbbb\ncccc");
}

#[cfg(test)]
#[test]
fn wrap_splits_word_only_when_wider_than_window() {
    assert_eq!(wrapped("abcdef", 4), "abcd\nef");
}

#[cfg(test)]
#[test]
fn wrap_breaks_after_hyphens() {
    assert_eq!(wrapped("well-ready", 5), "well-\nready");
    assert_eq!(wrapped("well-known", 11), "well-known");
    assert_eq!(wrapped("- item one -", 40), "- item one -");
}

#[cfg(test)]
#[test]
fn wrap_breaks_at_cjk_ideograph_boundaries() {
    assert_eq!(wrapped("我们", 2), "我\n们");
}

#[cfg(test)]
#[test]
fn wrap_measures_display_width_for_fullwidth_chars() {
    assert_eq!(wrapped("话话话", 4), "话话\n话");
}

#[cfg(test)]
#[test]
fn wrap_measures_display_width_for_emoji() {
    assert_eq!(wrapped("👉👉", 2), "👉\n👉");
}

#[cfg(test)]
#[test]
fn wrap_preserves_blank_lines_between_paragraphs() {
    assert_eq!(wrapped("one\n\ntwo", 40), "one\n\ntwo");
}

#[cfg(test)]
#[test]
fn wrap_non_breaking_space_keeps_text_together() {
    assert_eq!(wrapped("\u{00A0}абв", 8), "\u{00A0}абв");
}

#[cfg(test)]
#[test]
fn wrap_empty_string_yields_single_blank_line() {
    assert_eq!(wrapped("", 40), "");
}

// ============== Streaming feed tests ==============

#[cfg(test)]
#[must_use]
fn feed_chunks(chunks: Vec<&str>, max_width: usize) -> (Vec<String>, String) {
    let mut buf = String::new();
    let mut full = String::new();
    let mut pending = String::new();
    let mut st = WrapState::new();
    let mut out: Vec<String> = Vec::new();
    for c in chunks {
        for line in StreamingBox::greedy_feed(&mut buf, &mut pending, &mut st, &mut full, c, max_width) {
            out.push(line);
        }
    }
    if !pending.is_empty() || st.sep_pending {
        let tail_width = st.pending_width;
        commit_word(&mut buf, &mut st, &mut full, &mut out, &pending, tail_width, false, max_width);
    }
    let remaining = std::mem::take(&mut buf);
    if !remaining.is_empty() {
        full.push_str(&remaining);
    }
    (out, full)
}

#[cfg(test)]
#[test]
fn stream_wraps_at_words_without_mid_word_cuts() {
    let (out, full) = feed_chunks(vec!["hello world this is a long line"], 10);
    assert_eq!(full, "hello\nworld this\nis a long\nline");
    assert_eq!(out.join("\n"), "hello\nworld this\nis a long");
}

#[cfg(test)]
#[test]
fn stream_hard_splits_overwide_word() {
    let (out, full) = feed_chunks(vec!["abcdefghij"], 4);
    assert_eq!(full, "abcd\nefgh\nij");
    assert_eq!(out.join("\n"), "abcd\nefgh");
}

#[cfg(test)]
#[test]
fn stream_expands_tabs_and_drops_cr() {
    let (out, full) = feed_chunks(vec!["a\tb\r\nc"], 40);
    // "a" at CONTENT_COL (4), width 1, so at column 5. Next tab stop is 8, so 3 spaces.
    assert_eq!(out.join("\n"), "a   b");
    assert_eq!(full, "a   b\nc");
}

#[cfg(test)]
#[test]
fn stream_full_text_matches_display_except_trailing() {
    let (out, full) = feed_chunks(vec!["hello world"], 40);
    assert_eq!(out.join("\n"), "");
    assert_eq!(full, "hello world");
}

#[cfg(test)]
#[test]
fn stream_multiple_chunks_keep_state() {
    let (out, full) = feed_chunks(vec!["abcde", "fghij"], 6);
    assert_eq!(full, "abcdef\nghij");
    assert_eq!(out.join("\n"), "abcdef");
}

#[cfg(test)]
#[test]
fn finish_returns_original_backend_text() {
    let mut sb = StreamingBox::new("test");
    sb.write("a\tb\r\n");
    sb.write("abcdefghij");
    let raw = sb.finish();
    assert_eq!(raw, "a\tb\r\nabcdefghij");
}

#[cfg(test)]
#[test]
fn finish_skips_box_when_nothing_written() {
    let sb = StreamingBox::new("test");
    let raw = sb.finish();
    assert_eq!(raw, "");
}

// ============== Clipboard ==============

#[must_use]
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
    /// Spawns a spinner thread that animates the label in the terminal.
    /// In non-interactive mode (stdout not a tty), returns a no-op spinner
    /// that does nothing on `stop_and_rewind`.
    #[must_use]
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

fn push_emitted(out: &mut Vec<String>, full: &mut String, line: &str) {
    out.push(line.to_string());
    full.push_str(line);
    full.push('\n');
}

struct WrapState {
    width: usize,
    pending_width: usize,
    sep_pending: bool,
}

impl WrapState {
    #[allow(clippy::missing_const_for_fn)]
    fn new() -> Self {
        Self { width: 0, pending_width: 0, sep_pending: false }
    }
}

/// Commits a finalized word to the current line, wrapping to a new line
/// if it doesn't fit.
#[allow(clippy::too_many_arguments)]
fn commit_word(
    buf: &mut String,
    st: &mut WrapState,
    full: &mut String,
    out: &mut Vec<String>,
    word: &str,
    ww: usize,
    sep_after: bool,
    max_width: usize,
) {
    let lead = usize::from(st.sep_pending);
    let total = st.width + lead + ww;
    if total <= max_width {
        if st.sep_pending {
            buf.push(' ');
        }
        buf.push_str(word);
        st.width = total;
    } else {
        let line = std::mem::take(buf);
        if !line.is_empty() {
            push_emitted(out, full, &line);
        }
        buf.push_str(word);
        st.width = ww;
    }
    st.sep_pending = sep_after;
    st.pending_width = 0;
}

/// Byte offset just past the longest prefix of `s` fitting in `max_width`.
/// Used to hard-cut over-wide words during streaming.
#[must_use]
fn longest_prefix_end(s: &str, max_width: usize) -> usize {
    let mut end = 0;
    let mut width = 0;
    for (i, ch) in s.char_indices() {
        let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
        if width + cw > max_width && end > 0 {
            break;
        }
        width += cw;
        end = i + ch.len_utf8();
    }
    if end == 0 {
        end = first_char_end(s);
    }
    end
}

#[must_use]
fn first_char_end(s: &str) -> usize {
    let (first_pos, first_ch) = s.char_indices().next().unwrap_or((0, ' '));
    first_pos + first_ch.len_utf8()
}

pub(crate) struct StreamingBox {
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
    pub(crate) fn new(title: &str) -> Self {
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

    /// Feeds raw response text into the streaming buffer.
    /// Word-greedy with one word lookahead: incomplete words accumulate in `pending`
    /// until a terminator arrives, then a wrap decision is made.
    /// Tabs expand relative to `CONTENT_COL`; control chars (except `\n`, `\t`) are dropped.
    #[must_use]
    fn greedy_feed(
        buf: &mut String,
        pending: &mut String,
        st: &mut WrapState,
        full: &mut String,
        chunk: &str,
        max_width: usize,
    ) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();

        // Take pending into `word` without reallocation, preserving capacity.
        let mut word = std::mem::take(pending);
        let mut word_width = st.pending_width;

        for (_, ch) in chunk.char_indices() {
            let cw = UnicodeWidthChar::width(ch);

            if ch == '\n' {
                if !word.is_empty() {
                    commit_word(buf, st, full, &mut out, &word, word_width, false, max_width);
                    word.clear();
                    word_width = 0;
                }
                let line = std::mem::take(buf);
                if !line.is_empty() {
                    push_emitted(&mut out, full, &line);
                }
                st.width = 0;
                st.sep_pending = false;
                continue;
            }
            if ch == '\t' {
                if !word.is_empty() {
                    commit_word(buf, st, full, &mut out, &word, word_width, false, max_width);
                    word.clear();
                    word_width = 0;
                }
                let abs_col = CONTENT_COL + st.width;
                let next_stop = (abs_col / 8 + 1) * 8;
                let spaces = next_stop - abs_col;
                buf.push_str(&" ".repeat(spaces));
                st.width += spaces;
                continue;
            }
            if cw.is_none() {
                continue;
            }
            let w = cw.unwrap_or(0);

            if ch == ' ' {
                if !word.is_empty() {
                    commit_word(buf, st, full, &mut out, &word, word_width, true, max_width);
                    word.clear();
                    word_width = 0;
                }
                st.sep_pending = true;
                continue;
            }

            word.push(ch);
            word_width += w;
            while word_width > max_width {
                let line = std::mem::take(buf);
                if !line.is_empty() {
                    push_emitted(&mut out, full, &line);
                }
                st.width = 0;
                st.sep_pending = false;
                let cut = longest_prefix_end(&word, max_width);
                let emit = word[..cut].to_string();
                push_emitted(&mut out, full, &emit);
                let rest = &word[cut..];
                word = rest.to_string();
                word_width -= display_width(&emit);
            }
        }

        // Keep the incomplete word for the next chunk, moving `word` into
        // `pending` to preserve its capacity across `write` calls.
        if !word.is_empty() {
            *pending = word;
            st.pending_width = word_width;
        }

        out
    }

    pub(crate) fn write(&mut self, chunk: &str) {
        if !self.started {
            self.start();
        }
        self.raw_text.push_str(chunk);
        for line in Self::greedy_feed(
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

    #[must_use]
    pub(crate) fn finish(mut self) -> String {
        // If nothing was ever written, don't draw an empty box.
        if !self.started {
            return self.raw_text.trim_end_matches('\n').to_string();
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

        self.raw_text.trim_end_matches('\n').to_string()
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