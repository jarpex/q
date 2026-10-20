#![no_main]
use libfuzzer_sys::fuzz_target;
use q::commands::shell::extract_first_command;

fuzz_target!(|data: &str| {
    let first = extract_first_command(data);

    // The result is a prefix of the input.
    assert!(
        data.starts_with(first),
        "not a prefix: {data:?} -> {first:?}"
    );

    // The result must not be longer than the input.
    assert!(
        first.len() <= data.len(),
        "longer than input: {data:?} -> {first:?}"
    );
});
