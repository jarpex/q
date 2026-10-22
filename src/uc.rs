//! Runs the model output through the external `uc` program.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;

use crate::tui::print_uc_warning;

/// The program that cleans the text when the user gives no path.
const DEFAULT_PROGRAM: &str = "uc";

/// The flags that pick the mode of the program.
///
/// The program always clears the text, so these flags never reach it.
const MODE_FLAGS: [&str; 4] = ["-c", "--clear", "-s", "--show"];

/// The flags that make the program print something else than the text.
const OUTPUT_FLAGS: [&str; 4] = ["-h", "--help", "-v", "--version"];

/// The flag that tells the program to clear the text.
const CLEAR_FLAG: &str = "-c";

/// The flag that picks the encoding of the text.
const ENCODING_FLAG: &str = "-e";

/// The long form of the encoding flag.
const ENCODING_LONG_FLAG: &str = "--encoding";

/// The flag that makes the program report the problems it found.
const CHECK_FLAG: &str = "--check";

/// The flag that ends the options of the program.
const SEPARATOR: &str = "--";

/// What the filter knows about the program.
#[derive(Default)]
struct State {
    /// The reason of the failure, if the program failed.
    reason: Option<Box<str>>,
    /// Tells whether the reason was already shown.
    reported: bool,
}

impl State {
    /// Takes the reason of the failure and marks it as shown.
    fn take_reason(&mut self) -> Option<String> {
        if self.reported {
            return None;
        }
        let reason = self.reason.as_deref().map(str::to_owned)?;
        self.reported = true;
        Some(reason)
    }
}

/// Cleans the text with the external `uc` program.
pub struct Uc {
    /// The program to run.
    program: PathBuf,
    /// The arguments of the program.
    args: Vec<String>,
    /// Tells whether the user asked for the filter.
    enabled: bool,
    /// What the filter knows about the program.
    state: Mutex<State>,
}

impl Uc {
    /// Creates the filter from the arguments and the program of the user.
    ///
    /// The program always clears the text, so the mode flags of the user are
    /// dropped and the clear flag comes first. The program is the one in the
    /// path, unless the user gave a path of their own.
    #[must_use]
    pub fn new(args: Option<&str>, program: Option<&Path>) -> Self {
        Self {
            program: program.map_or_else(|| PathBuf::from(DEFAULT_PROGRAM), Path::to_path_buf),
            args: build_args(args.unwrap_or_default()),
            enabled: args.is_some() || program.is_some(),
            state: Mutex::new(State::default()),
        }
    }

    /// Tells whether the user asked for the filter.
    #[must_use]
    pub const fn enabled(&self) -> bool {
        self.enabled
    }

    /// Runs the text through the program.
    ///
    /// The text comes back unchanged when the program is missing or fails.
    /// The reason of the failure waits in the filter, so the caller can show
    /// it above the output.
    pub async fn clean(&self, text: String) -> String {
        if !self.enabled || self.failed() {
            return text;
        }

        match self.run(&text).await {
            Ok(cleaned) => cleaned,
            Err(reason) => {
                self.set_failure(reason);
                text
            }
        }
    }

    /// Shows the reason of the failure above the output, once.
    pub fn report_failure(&self) {
        let reason = self.lock().take_reason();
        let Some(message) = reason else {
            return;
        };

        print_uc_warning(&message);
    }

    /// Tells whether the program failed already.
    fn failed(&self) -> bool {
        self.lock().reason.is_some()
    }

    /// Runs the program with the text on its standard input.
    #[allow(clippy::result_large_err)]
    async fn run(&self, text: &str) -> Result<String, String> {
        let mut child = Command::new(&self.program)
            .args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| match error.kind() {
                ErrorKind::NotFound => {
                    if self.program == Path::new(DEFAULT_PROGRAM) {
                        "uc not found".to_owned()
                    } else {
                        format!("uc not found: {}", self.program.display())
                    }
                }
                _ => format!("uc failed to start: {error}"),
            })?;

        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        // The text goes in while the output comes out, so a long text does
        // not fill the pipe of the program and block it.
        let feed = async move {
            if let Some(mut pipe) = stdin {
                let _ = pipe.write_all(text.as_bytes()).await;
            }
        };
        let collect = async move {
            let mut buffer = Vec::new();
            if let Some(mut pipe) = stdout {
                let _ = pipe.read_to_end(&mut buffer).await;
            }
            buffer
        };
        let collect_error = async move {
            let mut buffer = Vec::new();
            if let Some(mut pipe) = stderr {
                let _ = pipe.read_to_end(&mut buffer).await;
            }
            buffer
        };

        let ((), output, error) = tokio::join!(feed, collect, collect_error);

        let status = child
            .wait()
            .await
            .map_err(|error| format!("uc failed: {error}"))?;

        // The program reports the problems it found with a status of one when
        // the check flag is given. The text is still clean in that case.
        let found_problems = status.code() == Some(1) && self.checks_problems();
        if !status.success() && !found_problems {
            return Err(
                first_line(&error).unwrap_or_else(|| format!("uc failed with status {status}"))
            );
        }

        Ok(String::from_utf8_lossy(&output).into_owned())
    }

    /// Tells whether the user asked the program to report the problems.
    fn checks_problems(&self) -> bool {
        self.args.iter().any(|arg| arg == CHECK_FLAG)
    }

    /// Locks the state of the filter.
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Records the failure of the program.
    fn set_failure(&self, reason: String) {
        let mut state = self.lock();
        if state.reason.is_none() {
            state.reason = Some(reason.into_boxed_str());
            state.reported = false;
        }
    }
}

/// Builds the arguments of the program.
///
/// The clear flag comes first, so the mode of the program does not depend on
/// the order of the arguments of the user. The mode flags of the user are
/// dropped, because the program always clears the text. The flags that print
/// something else than the text are dropped as well. The separator ends the
/// options, because the text goes to the program on its standard input. A word
/// that is not an option is dropped as well, because the program would read it
/// as the text to clean instead of the standard input.
#[must_use]
pub fn build_args(given: &str) -> Vec<String> {
    let mut argv = vec![CLEAR_FLAG.to_owned()];
    let mut take_value = false;

    for arg in given.split_whitespace() {
        // The separator ends the options, so the flag that waits for a value
        // loses it and goes away with them.
        if arg == SEPARATOR {
            if take_value {
                argv.pop();
            }
            break;
        }

        if take_value {
            take_value = false;
            if !MODE_FLAGS.contains(&arg) && !OUTPUT_FLAGS.contains(&arg) {
                argv.push(arg.to_owned());
            }
            continue;
        }

        if MODE_FLAGS.contains(&arg) || OUTPUT_FLAGS.contains(&arg) {
            continue;
        }
        if arg == ENCODING_FLAG || arg == ENCODING_LONG_FLAG {
            take_value = true;
            argv.push(arg.to_owned());
            continue;
        }
        if arg.starts_with('-') {
            argv.push(arg.to_owned());
        }
    }

    argv
}

/// Tells whether the argument takes a value after it.
#[must_use]
pub fn takes_value(arg: &str) -> bool {
    arg == ENCODING_FLAG || arg == ENCODING_LONG_FLAG
}

/// Returns the first line of the error output of the program.
fn first_line(bytes: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(bytes);
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
#[allow(clippy::result_large_err)]
mod tests {
    use super::{build_args, takes_value, Uc};
    use std::path::Path;

    #[test]
    fn always_clears_the_text() {
        assert_eq!(build_args(""), ["-c"]);
        assert_eq!(build_args("-e cp1251"), ["-c", "-e", "cp1251"]);
    }

    #[test]
    fn drops_the_mode_flags_of_the_user() {
        assert_eq!(build_args("-c -e cp1251"), ["-c", "-e", "cp1251"]);
        assert_eq!(build_args("-s -c"), ["-c"]);
        assert_eq!(build_args("--clear --show"), ["-c"]);
    }

    #[test]
    fn drops_the_flags_that_print_something_else() {
        assert_eq!(build_args("-h -v"), ["-c"]);
        assert_eq!(build_args("--help --version"), ["-c"]);
    }

    #[test]
    fn keeps_the_other_options() {
        assert_eq!(build_args("--check"), ["-c", "--check"]);
        assert_eq!(build_args("--encoding=cp1251"), ["-c", "--encoding=cp1251"]);
    }

    #[test]
    fn drops_the_arguments_after_the_separator() {
        assert_eq!(build_args("-e cp1251 -- -c"), ["-c", "-e", "cp1251"]);
    }

    #[test]
    fn the_separator_ends_the_options_of_a_flag() {
        assert_eq!(build_args("-e -- -c"), ["-c"]);
        assert_eq!(build_args("-c -- -e cp1251"), ["-c"]);
    }

    #[test]
    fn drops_the_words_that_would_replace_the_text() {
        assert_eq!(build_args("hello"), ["-c"]);
        assert_eq!(build_args("-e cp1251 hello"), ["-c", "-e", "cp1251"]);
    }

    #[test]
    fn only_the_encoding_flag_takes_a_value() {
        assert!(takes_value("-e"));
        assert!(takes_value("--encoding"));
        assert!(!takes_value("--check"));
    }

    #[test]
    fn the_filter_stays_off_without_options() {
        assert!(!Uc::new(None, None).enabled());
        assert!(Uc::new(None, Some(Path::new("/opt/uc"))).enabled());
        assert!(Uc::new(Some("-c"), None).enabled());
    }

    #[tokio::test]
    async fn reports_a_missing_program() {
        let filter = Uc::new(Some("-c"), Some(Path::new("/nonexistent/uc")));

        let outcome = filter.run("hello").await;

        assert!(
            matches!(outcome, Err(ref reason) if reason == "uc not found: /nonexistent/uc"),
            "unexpected outcome: {outcome:?}"
        );
    }

    #[tokio::test]
    async fn keeps_the_text_when_the_program_is_missing() {
        let filter = Uc::new(Some("-c"), Some(Path::new("/nonexistent/uc")));

        let text = filter.clean("hello".to_owned()).await;

        assert_eq!(text, "hello");
    }
}
