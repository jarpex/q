#![no_main]
use libfuzzer_sys::fuzz_target;
use q::config::parse_cookies;

fuzz_target!(|data: &str| {
    let _ = parse_cookies(data);
});