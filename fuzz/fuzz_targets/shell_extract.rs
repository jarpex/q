#![no_main]
use libfuzzer_sys::fuzz_target;
use q::commands::shell::extract_first_command;

fuzz_target!(|data: &str| {
    let _ = extract_first_command(data);
});