//! The errors of the collector, as the Python exception classes told them apart.
use std::fmt;

use crate::secrets::redact;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// A wrong value: its text is for the user.
    Value(String),
    /// An operating system error, by the name its class had. The text may hold private paths: it is never shown.
    Os(&'static str),
    QueueFull(String),
    Cancelled(String),
    Telegram { message: String, retry_after: i64, status_code: i64 },
    /// A failure whose text is for the user.
    Runtime(String),
    /// A failure shown by its class only.
    Other(&'static str),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub fn telegram(message: impl AsRef<str>) -> Error {
        Error::telegram_with(message, 0, 0)
    }

    pub fn telegram_with(message: impl AsRef<str>, retry_after: i64, status_code: i64) -> Error {
        Error::Telegram { message: redact(message.as_ref()), retry_after, status_code }
    }

    pub fn value(message: impl Into<String>) -> Error {
        Error::Value(message.into())
    }

    /// The name of the Python class, which is what a report may say about a private failure.
    #[must_use]
    pub fn type_name(&self) -> &'static str {
        match self {
            Error::Value(_) => "ValueError",
            Error::Os(name) | Error::Other(name) => name,
            Error::QueueFull(_) => "QueueFull",
            Error::Cancelled(_) => "DeliveryCancelled",
            Error::Telegram { .. } => "TelegramError",
            Error::Runtime(_) => "RuntimeError",
        }
    }

    /// Whether the text is meant for the user (`ValueError` and `RuntimeError` in the Python).
    #[must_use]
    pub fn has_user_text(&self) -> bool {
        !matches!(self, Error::Os(_) | Error::Other(_))
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Value(message) | Error::QueueFull(message) | Error::Cancelled(message) | Error::Runtime(message) => {
                formatter.write_str(message)
            }
            Error::Telegram { message, .. } => formatter.write_str(message),
            Error::Os(name) | Error::Other(name) => formatter.write_str(name),
        }
    }
}

impl From<std::io::Error> for Error {
    #[expect(clippy::wildcard_enum_match_arm, reason = "std::io::ErrorKind is non-exhaustive: a new kind has to land somewhere")]
    fn from(error: std::io::Error) -> Error {
        use std::io::ErrorKind::{NotFound, PermissionDenied, AlreadyExists, IsADirectory, NotADirectory, TimedOut};
        Error::Os(match error.kind() {
            NotFound => "FileNotFoundError",
            PermissionDenied => "PermissionError",
            AlreadyExists => "FileExistsError",
            IsADirectory => "IsADirectoryError",
            NotADirectory => "NotADirectoryError",
            TimedOut => "TimeoutError",
            _ => "OSError",
        })
    }
}

impl From<sqlite::Error> for Error {
    fn from(_: sqlite::Error) -> Error {
        Error::Other("OperationalError")
    }
}

/// Whether an operating system error was a missing file.
#[must_use]
pub fn not_found(error: &Error) -> bool {
    matches!(error, Error::Os("FileNotFoundError"))
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    #[test]
    fn every_error_has_its_python_name_and_shows_only_what_it_may() {
        for (error, name, text) in [
            (Error::value("v"), "ValueError", true),
            (Error::Os("OSError"), "OSError", false),
            (Error::Other("TypeError"), "TypeError", false),
            (Error::QueueFull("q".to_owned()), "QueueFull", true),
            (Error::Cancelled("c".to_owned()), "DeliveryCancelled", true),
            (Error::telegram("t"), "TelegramError", true),
            (Error::Runtime("r".to_owned()), "RuntimeError", true),
        ] {
            assert_eq!((error.type_name(), error.has_user_text()), (name, text));
            assert!(!error.to_string().is_empty());
        }
    }

    #[test]
    fn a_failure_of_the_system_is_named_as_python_names_it() {
        use std::io::{Error as Io, ErrorKind};
        for (kind, name) in [
            (ErrorKind::NotFound, "FileNotFoundError"),
            (ErrorKind::PermissionDenied, "PermissionError"),
            (ErrorKind::AlreadyExists, "FileExistsError"),
            (ErrorKind::IsADirectory, "IsADirectoryError"),
            (ErrorKind::NotADirectory, "NotADirectoryError"),
            (ErrorKind::TimedOut, "TimeoutError"),
            (ErrorKind::Other, "OSError"),
        ] {
            assert_eq!(Error::from(Io::new(kind, "x")).type_name(), name);
        }
        assert_eq!(Error::from(sqlite::Error::InvalidQuery).type_name(), "OperationalError");
    }
}
