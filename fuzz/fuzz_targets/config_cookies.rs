#![no_main]
use libfuzzer_sys::fuzz_target;
use q::config::parse_cookies;

fuzz_target!(|data: &str| {
    let Ok(cookies) = parse_cookies(data) else {
        return;
    };

    // A parsed session has both cookies set.
    assert!(!cookies.psid.is_empty(), "psid is empty");
    assert!(!cookies.psidts.is_empty(), "psidts is empty");

    // Parsing the same input twice gives the same cookies.
    let again = parse_cookies(data).expect("reparse");
    assert_eq!(again.psid.as_str(), cookies.psid.as_str());
    assert_eq!(again.psidts.as_str(), cookies.psidts.as_str());
});
