/// Provides utilities for interacting with the system clipboard.
pub mod clipboard;

/// The screen that manages the API keys of the user.
pub mod key_manager;

/// Contains custom UI widgets for the terminal interface.
pub mod widgets;

/// Implements text wrapping and layout calculation logic.
pub mod wrap;

pub use clipboard::{copy_to_clipboard, print_copied_message};
pub use key_manager::{Action as KeyAction, KeyManager};
pub use widgets::{
    print_command, print_error, print_in_box, print_uc_warning, print_usage_limits, Spinner,
    StreamingBox,
};
pub use wrap::wrap_text;
