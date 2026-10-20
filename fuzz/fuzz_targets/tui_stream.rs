#![no_main]
use libfuzzer_sys::fuzz_target;
use q::tui::StreamingBox;

fuzz_target!(|data: &str| {
    let mut box_printer = StreamingBox::new("test");
    box_printer.write(data);
    let whole = box_printer.finish();

    // The output is the input without trailing newlines.
    assert_eq!(
        whole,
        data.trim_end_matches('\n'),
        "output differs: {data:?}"
    );

    // Splitting the input into two writes gives the same result.
    let mid = data
        .char_indices()
        .nth(data.chars().count() / 2)
        .map(|(index, _)| index)
        .unwrap_or(0);
    let mut split_printer = StreamingBox::new("test");
    split_printer.write(&data[..mid]);
    split_printer.write(&data[mid..]);
    let split = split_printer.finish();
    assert_eq!(split, whole, "split write differs: {data:?}");
});
