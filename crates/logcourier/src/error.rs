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
    fn from(error: std::io::Error) -> Error {
        use std::io::ErrorKind::*;
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
pub fn not_found(error: &Error) -> bool {
    matches!(error, Error::Os("FileNotFoundError"))
}
