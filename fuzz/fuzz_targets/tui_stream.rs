#![no_main]
use libfuzzer_sys::fuzz_target;
use q::tui::StreamingBox;

fuzz_target!(|data: &str| {
    let mut box_printer = StreamingBox::new("test");
    box_printer.write(data);
    let _ = box_printer.finish();
});