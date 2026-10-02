/// Contains the logic for spawning and managing the Gemini API background process.
pub mod runner;

pub use runner::{spawn_gemini_stream, AskOptions, GeminiEvent};
