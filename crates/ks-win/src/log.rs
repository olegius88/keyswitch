//! The `logging` calls of the Python modules, as one function the application sets.
//!
//! Logs are analysed by their format, so the logger name and the message text are the same as in the Python. The
//! function may run on the keyboard hook's thread, as the Python's handlers did: it must not wait for long.

use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Info,
    Error,
}

/// Where the lines go: the level, the logger's name (`keyswitch.windows_backend`) and the message.
pub type LogSink = fn(Level, &str, &str);

static SINK: OnceLock<LogSink> = OnceLock::new();

/// Sets the function once; says whether this call set it. Until then the lines go nowhere.
pub fn set_sink(sink: LogSink) -> bool {
    SINK.set(sink).is_ok()
}

fn write(level: Level, logger: &str, message: &str) {
    if let Some(sink) = SINK.get() {
        sink(level, logger, message);
    }
}

pub fn info(logger: &str, message: &str) {
    write(Level::Info, logger, message);
}

pub fn error(logger: &str, message: &str) {
    write(Level::Error, logger, message);
}

/// `LOGGER.info(message, exc_info=True)`: the message, then on its own line what went wrong.
pub fn info_with_error(logger: &str, message: &str, error: &dyn std::fmt::Display) {
    if SINK.get().is_some() {
        write(Level::Info, logger, &format!("{message}\n{error}"));
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    /// Before the program sets its journal the lines go nowhere: in a process of its own, where no test has set one.
    #[test]
    fn lines_go_nowhere_until_a_journal_is_set() {
        const CHILD: &str = "KS_TEST_NO_SINK";
        if std::env::var_os(CHILD).is_some() {
            info("logger", "a line");
            error("logger", "a line");
            info_with_error("logger", "a line", &"and why");
            assert!(set_sink(|_level, _logger, _message| {}));
            assert!(!set_sink(|_level, _logger, _message| {}));
            error("logger", "a line");
            return;
        }
        let status = std::process::Command::new(std::env::current_exe().expect("this test"))
            .args(["--exact", "log::tests::lines_go_nowhere_until_a_journal_is_set"])
            .env(CHILD, "1")
            .status()
            .expect("started");
        assert!(status.success());
    }
}

/// The lines every test of the crate wrote, for the tests to look through.
#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
pub(crate) mod capture {
    use std::sync::{Mutex, Once};

    static LINES: Mutex<Vec<String>> = Mutex::new(Vec::new());
    static INSTALL: Once = Once::new();

    pub(crate) fn install() {
        INSTALL.call_once(|| {
            super::set_sink(|_level, logger, message| {
                LINES.lock().unwrap().push(format!("{logger}: {message}"));
            });
        });
    }

    pub(crate) fn contains(fragment: &str) -> bool {
        LINES.lock().unwrap().iter().any(|line| line.contains(fragment))
    }
}
