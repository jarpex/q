use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const CONTENT_COL: usize = 4;

#[must_use]
pub(crate) fn display_width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// Wraps the given text into lines that fit within the specified maximum display width.
///
/// Handles Unicode-aware width calculation, line-breaking opportunities, and long words.
#[must_use]
pub fn wrap_text(text: &str, max_width: usize) -> Vec<String> {
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
        if !current.is_empty() && cur_w + display_width(sep) + w <= max_width {
            current.push_str(sep);
            current.push_str(word);
            return;
        }
    }
    out.push(word.to_owned());
}

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
        out.push(rest[..cut].to_owned());
        rest = &rest[cut..];
    }
    if !rest.is_empty() {
        out.push(rest.to_owned());
    }
}

pub(crate) fn push_emitted(out: &mut Vec<String>, full: &mut String, line: &str) {
    out.push(line.to_owned());
    full.push_str(line);
    full.push('\n');
}

pub(crate) struct WrapState {
    pub(crate) width: usize,
    pub(crate) pending_width: usize,
    pub(crate) sep_pending: bool,
}

impl WrapState {
    #[allow(clippy::missing_const_for_fn)]
    pub(crate) fn new() -> Self {
        Self {
            width: 0,
            pending_width: 0,
            sep_pending: false,
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn commit_word(
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

#[must_use]
pub(crate) fn longest_prefix_end(s: &str, max_width: usize) -> usize {
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
pub(crate) fn first_char_end(s: &str) -> usize {
    let (first_pos, first_ch) = s.char_indices().next().unwrap_or((0, ' '));
    first_pos + first_ch.len_utf8()
}

#[must_use]
#[allow(clippy::too_many_arguments)]
pub(crate) fn greedy_feed(
    buf: &mut String,
    pending: &mut String,
    st: &mut WrapState,
    full: &mut String,
    chunk: &str,
    max_width: usize,
) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();

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
            let emit = word[..cut].to_owned();
            push_emitted(&mut out, full, &emit);
            word.drain(..cut);
            word_width -= display_width(&emit);
        }
    }

    if !word.is_empty() {
        *pending = word;
        st.pending_width = word_width;
    }

    out
}

#[cfg(test)]
#[allow(clippy::result_large_err)]
mod tests {
    use super::*;

    #[must_use]
    fn wrapped(text: &str, max_width: usize) -> String {
        wrap_text(text, max_width).join("\n")
    }

    #[test]
    fn wrap_keeps_short_lines_whole() {
        assert_eq!(wrapped("hello world", 40), "hello world");
    }

    #[test]
    fn wrap_breaks_at_spaces_when_they_dont_fit() {
        assert_eq!(wrapped("a b c", 3), "a b\nc");
    }

    #[test]
    fn wrap_packs_words_greedily() {
        assert_eq!(wrapped("a bb cccc", 4), "a bb\ncccc");
    }

    #[test]
    fn wrap_does_not_split_words_that_fit_in_the_window() {
        assert_eq!(wrapped("aaaa bbbb cccc", 4), "aaaa\nbbbb\ncccc");
    }

    #[test]
    fn wrap_splits_word_only_when_wider_than_window() {
        assert_eq!(wrapped("abcdef", 4), "abcd\nef");
    }

    #[test]
    fn wrap_breaks_after_hyphens() {
        assert_eq!(wrapped("well-ready", 5), "well-\nready");
        assert_eq!(wrapped("well-known", 11), "well-known");
        assert_eq!(wrapped("- item one -", 40), "- item one -");
    }

    #[test]
    fn wrap_breaks_at_cjk_ideograph_boundaries() {
        assert_eq!(wrapped("我们", 2), "我\n们");
    }

    #[test]
    fn wrap_measures_display_width_for_fullwidth_chars() {
        assert_eq!(wrapped("话话话", 4), "话话\n话");
    }

    #[test]
    fn wrap_measures_display_width_for_emoji() {
        assert_eq!(wrapped("👉👉", 2), "👉\n👉");
    }

    #[test]
    fn wrap_preserves_blank_lines_between_paragraphs() {
        assert_eq!(wrapped("one\n\ntwo", 40), "one\n\ntwo");
    }

    #[test]
    fn wrap_non_breaking_space_keeps_text_together() {
        assert_eq!(wrapped("\u{00A0}абв", 8), "\u{00A0}абв");
    }

    #[test]
    fn wrap_empty_string_yields_single_blank_line() {
        assert_eq!(wrapped("", 40), "");
    }

    #[must_use]
    fn feed_chunks(chunks: Vec<&str>, max_width: usize) -> (Vec<String>, String) {
        let mut buf = String::new();
        let mut full = String::new();
        let mut pending = String::new();
        let mut st = WrapState::new();
        let mut out: Vec<String> = Vec::new();
        for c in chunks {
            for line in greedy_feed(&mut buf, &mut pending, &mut st, &mut full, c, max_width) {
                out.push(line);
            }
        }
        if !pending.is_empty() || st.sep_pending {
            let tail_width = st.pending_width;
            commit_word(
                &mut buf, &mut st, &mut full, &mut out, &pending, tail_width, false, max_width,
            );
        }
        let remaining = std::mem::take(&mut buf);
        if !remaining.is_empty() {
            full.push_str(&remaining);
        }
        (out, full)
    }

    #[test]
    fn stream_wraps_at_words_without_mid_word_cuts() {
        let (out, full) = feed_chunks(vec!["hello world this is a long line"], 10);
        assert_eq!(full, "hello\nworld this\nis a long\nline");
        assert_eq!(out.join("\n"), "hello\nworld this\nis a long");
    }

    #[test]
    fn stream_hard_splits_overwide_word() {
        let (out, full) = feed_chunks(vec!["abcdefghij"], 4);
        assert_eq!(full, "abcd\nefgh\nij");
        assert_eq!(out.join("\n"), "abcd\nefgh");
    }

    #[test]
    fn stream_expands_tabs_and_drops_cr() {
        let (out, full) = feed_chunks(vec!["a\tb\r\nc"], 40);
        assert_eq!(out.join("\n"), "a   b");
        assert_eq!(full, "a   b\nc");
    }

    #[test]
    fn stream_full_text_matches_display_except_trailing() {
        let (out, full) = feed_chunks(vec!["hello world"], 40);
        assert_eq!(out.join("\n"), "");
        assert_eq!(full, "hello world");
    }

    #[test]
    fn stream_multiple_chunks_keep_state() {
        let (out, full) = feed_chunks(vec!["abcde", "fghij"], 6);
        assert_eq!(full, "abcdef\nghij");
        assert_eq!(out.join("\n"), "abcdef");
    }
}
