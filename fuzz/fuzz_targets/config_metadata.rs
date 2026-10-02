#![no_main]
use libfuzzer_sys::fuzz_target;
use q::config::Metadata;

fuzz_target!(|data: &str| {
    let _: Result<Metadata, _> = serde_json::from_str(data);
});