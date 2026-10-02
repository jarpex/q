/// Provides utilities for interacting with the system clipboard.
pub mod clipboard;

/// Contains custom UI widgets for the terminal interface.
pub mod widgets;

/// Implements text wrapping and layout calculation logic.
pub mod wrap;

pub use clipboard::{copy_to_clipboard, print_copied_message};
pub use widgets::{print_command, print_error, print_in_box, Spinner, StreamingBox};
pub use wrap::wrap_text;
