#![no_main]

use libfuzzer_sys::fuzz_target;
use q::config::CookieSet;

fuzz_target!(|data: &str| {
    let _: Result<CookieSet, _> = serde_json::from_str(data);
});