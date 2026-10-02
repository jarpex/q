#![no_main]
use libfuzzer_sys::fuzz_target;
use q::commands::shell::parse_command;

fuzz_target!(|data: &str| {
    let _ = parse_command(data);
});