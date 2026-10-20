#![no_main]
use libfuzzer_sys::fuzz_target;
use q::config::Metadata;
use std::time::Duration;

fuzz_target!(|data: &str| {
    let Ok(metadata) = serde_json::from_str::<Metadata>(data) else {
        return;
    };

    // A missing field falls back to the default.
    let default = Metadata::default();
    assert_eq!(default.last_update, 0);
    assert_eq!(default.last_version, "");

    // The version round-trips through JSON.
    let json = serde_json::to_string(&metadata).expect("serialize");
    let parsed: Metadata = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(parsed.last_update, metadata.last_update);
    assert_eq!(parsed.last_version, metadata.last_version);

    // The current time equal to the update time is never stale.
    let max_age = Duration::from_secs(3600);
    assert!(
        !metadata.is_stale(metadata.last_update, max_age),
        "fresh is stale"
    );

    // The current time past the limit is stale.
    if let Some(past) = metadata.last_update.checked_add(max_age.as_secs()) {
        assert!(metadata.is_stale(past, max_age), "past is not stale");
        assert!(!metadata.is_stale(past - 1, max_age), "edge is stale");
    }
});
