#![no_main]
use libfuzzer_sys::fuzz_target;
use q::tui::wrap_text;

fuzz_target!(|data: &str| {
    let _ = wrap_text(data, 40);
});