#![no_main]
use libfuzzer_sys::fuzz_target;
use q::tui::wrap_text;
use unicode_width::UnicodeWidthChar;

const MAX_WIDTH: usize = 40;

fuzz_target!(|data: &str| {
    let lines = wrap_text(data, MAX_WIDTH);

    for line in &lines {
        // ANSI sequences have no display width, so they are measured separately.
        if line.contains('\x1b') {
            continue;
        }
        // Control characters have no width, the same as in the wrapping code.
        let width: usize = line
            .chars()
            .map(|ch| UnicodeWidthChar::width(ch).unwrap_or(0))
            .sum();
        assert!(width <= MAX_WIDTH, "line too wide: {line:?}");
    }

    // The content survives the wrapping.
    let keep = |text: &str| -> String {
        text.chars()
            .filter(|c| !c.is_whitespace() && !c.is_control())
            .collect()
    };
    assert_eq!(
        keep(&lines.concat()),
        keep(data),
        "content changed: {data:?}"
    );
});
