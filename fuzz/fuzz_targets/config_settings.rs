#![no_main]
use libfuzzer_sys::fuzz_target;
use q::config::parse_settings;

fuzz_target!(|data: &str| {
    let Ok(settings) = parse_settings(data) else {
        return;
    };

    // The model keeps its casing.
    if let Some(model) = &settings.model {
        assert!(!model.is_empty(), "model is empty");
    }

    // Parsing the same input twice gives the same result.
    let again = parse_settings(data);
    assert!(again.is_ok(), "parse is not deterministic");
    assert_eq!(again.ok(), Some(settings), "parse is not deterministic");
});
