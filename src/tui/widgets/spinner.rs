//! Animated terminal spinner widget.

use crossterm::{
    cursor, execute,
    style::{Color, Print, ResetColor, SetForegroundColor},
    terminal::{Clear, ClearType},
};
use std::io::{self, Write};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::Duration;

use super::{is_interactive, left_indent};

/// The terminal operations the spinner needs.
///
/// The trait is the seam that lets a test count the commands without a real
/// terminal, which matters on Windows, where the cursor commands do not
/// always reach the output as escape sequences.
trait SpinnerTerminal: Send + Sync {
    /// Hides the cursor, so it does not blink while the spinner runs.
    fn hide_cursor(&self);

    /// Draws one frame of the animation.
    fn draw_frame(&self, frame: char, label: &str);

    /// Erases the line the spinner was drawn on and shows the cursor again.
    fn restore(&self);
}

/// The real terminal.
struct Terminal;

impl SpinnerTerminal for Terminal {
    fn hide_cursor(&self) {
        let mut stdout = io::stdout();
        let _ = execute!(stdout, cursor::Hide);
        let _ = stdout.flush();
    }

    fn draw_frame(&self, frame: char, label: &str) {
        let mut stdout = io::stdout();
        let _ = execute!(
            stdout,
            cursor::MoveToColumn(0),
            Clear(ClearType::CurrentLine),
            Print(left_indent()),
            SetForegroundColor(Color::DarkGrey),
            Print(format!("{frame} {label}")),
            ResetColor
        );
        let _ = stdout.flush();
    }

    fn restore(&self) {
        let mut stdout = io::stdout();
        let _ = execute!(
            stdout,
            cursor::MoveToColumn(0),
            Clear(ClearType::CurrentLine),
            cursor::Show
        );
        let _ = stdout.flush();
    }
}

/// An animated terminal spinner widget that displays a label while a background task is running.
pub struct Spinner {
    stop_flag: Arc<AtomicBool>,
    label: Arc<Mutex<String>>,
    handle: Option<std::thread::JoinHandle<()>>,
    terminal: Arc<dyn SpinnerTerminal>,
}

impl Spinner {
    /// Starts a new spinner with the given label.
    ///
    /// If the terminal is not interactive, the spinner is created in an inactive state
    /// and no animation is drawn.
    #[must_use]
    pub fn start(label: &str) -> Self {
        Self::start_with(label, Arc::new(Terminal), is_interactive())
    }

    /// Starts a new spinner that draws on the given terminal.
    ///
    /// An inactive spinner draws nothing and leaves the terminal alone.
    fn start_with(label: &str, terminal: Arc<dyn SpinnerTerminal>, interactive: bool) -> Self {
        let label_arc = Arc::new(Mutex::new(label.to_owned()));

        if !interactive {
            return Self {
                stop_flag: Arc::new(AtomicBool::new(false)),
                label: label_arc,
                handle: None,
                terminal,
            };
        }

        // The cursor is hidden before the first frame is drawn, so the line
        // stays clean while the spinner runs.
        terminal.hide_cursor();

        let stop_flag = Arc::new(AtomicBool::new(false));
        let stop_flag_clone = Arc::clone(&stop_flag);
        let label_clone = Arc::clone(&label_arc);
        let terminal_clone = Arc::clone(&terminal);

        let handle = std::thread::spawn(move || {
            let frames = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
            let mut frame_idx = 0;

            while !stop_flag_clone.load(Ordering::Relaxed) {
                let frame = frames.get(frame_idx % frames.len()).copied().unwrap_or('⠋');
                frame_idx += 1;

                let current = match label_clone.lock() {
                    Ok(lock) => lock.clone(),
                    Err(poisoned) => poisoned.into_inner().clone(),
                };

                terminal_clone.draw_frame(frame, &current);

                std::thread::sleep(Duration::from_millis(80));
            }
        });

        Self {
            stop_flag,
            label: label_arc,
            handle: Some(handle),
            terminal,
        }
    }

    /// Updates the label text displayed next to the spinner.
    pub fn set_label(&self, new_label: &str) {
        if let Ok(mut lock) = self.label.lock() {
            new_label.clone_into(&mut lock);
        }
    }

    /// Stops the spinner animation and clears the line it was drawn on.
    pub fn stop_and_rewind(mut self) {
        self.stop();
    }

    /// Stops the animation and gives the terminal back.
    ///
    /// The terminal is restored only once, no matter how many times the
    /// spinner is stopped.
    fn stop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        let Some(handle) = self.handle.take() else {
            return;
        };
        let _ = handle.join();
        self.terminal.restore();
    }
}

impl Drop for Spinner {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::{Spinner, SpinnerTerminal};
    use std::sync::{Arc, Mutex, PoisonError};

    /// The commands the terminal received, in the order they arrived.
    #[derive(Debug, PartialEq, Eq, Clone, Copy)]
    enum Command {
        Hide,
        Frame,
        Restore,
    }

    /// The log of the recorded commands.
    type Log = Arc<Mutex<Vec<Command>>>;

    /// A terminal that records the commands instead of drawing them.
    struct RecordingTerminal {
        log: Log,
    }

    impl SpinnerTerminal for RecordingTerminal {
        fn hide_cursor(&self) {
            self.record(Command::Hide);
        }

        fn draw_frame(&self, _frame: char, _label: &str) {
            self.record(Command::Frame);
        }

        fn restore(&self) {
            self.record(Command::Restore);
        }
    }

    impl RecordingTerminal {
        fn record(&self, command: Command) {
            let mut log = self.log.lock().unwrap_or_else(PoisonError::into_inner);
            log.push(command);
        }
    }

    /// Builds a recording terminal and the log it writes to.
    fn recording_terminal() -> (Arc<RecordingTerminal>, Log) {
        let log: Log = Arc::new(Mutex::new(Vec::new()));
        let terminal = RecordingTerminal {
            log: Arc::clone(&log),
        };
        (Arc::new(terminal), log)
    }

    /// Returns the recorded commands.
    fn recorded(log: &Log) -> Vec<Command> {
        log.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    #[test]
    fn restores_the_terminal_once_after_stop_and_rewind() {
        let (terminal, log) = recording_terminal();
        let spinner = Spinner::start_with("test", terminal, true);
        spinner.stop_and_rewind();

        let commands = recorded(&log);
        let restores = commands.iter().filter(|c| **c == Command::Restore).count();
        assert_eq!(restores, 1, "{commands:?}");
        assert_eq!(commands.first(), Some(&Command::Hide), "{commands:?}");
        assert_eq!(commands.last(), Some(&Command::Restore), "{commands:?}");
    }

    #[test]
    fn restores_the_terminal_once_when_dropped() {
        let (terminal, log) = recording_terminal();
        drop(Spinner::start_with("test", terminal, true));

        let commands = recorded(&log);
        let restores = commands.iter().filter(|c| **c == Command::Restore).count();
        assert_eq!(restores, 1, "{commands:?}");
    }

    #[test]
    fn leaves_the_terminal_alone_when_it_is_not_interactive() {
        let (terminal, log) = recording_terminal();
        let spinner = Spinner::start_with("test", terminal, false);
        spinner.stop_and_rewind();

        assert_eq!(recorded(&log), Vec::new());
    }
}
