//! The four ways an operator call fails, as the Python code distinguishes
//! them: `AppError`, `OSError` (with `TimeoutError` as its `TimedOut`
//! kind), `EOFError`, and a programming error Python would raise as
//! `ValueError`/`TypeError` and never catch.

use kassiber_envelope::AppError;
use std::fmt;
use std::io;

#[derive(Debug)]
pub enum Error {
    /// A typed, user-facing failure (`AppError`).
    App(AppError),
    /// `OSError`; `io::ErrorKind::TimedOut` is Python's `TimeoutError`.
    Io(io::Error),
    /// `EOFError`: the peer closed the connection mid-frame.
    Eof(&'static str),
    /// An exception Python does not catch on these paths (`ValueError`,
    /// `TypeError`, `RecursionError`, `OverflowError`).
    Value(String),
}

impl Error {
    /// The `AppError`, if this is one.
    pub fn app(&self) -> Option<&AppError> {
        match self {
            Error::App(error) => Some(error),
            _ => None,
        }
    }

    /// The `AppError` code, if this is one.
    pub fn code(&self) -> Option<&str> {
        self.app().map(|error| error.code.as_str())
    }

    /// `except (OSError, EOFError)`: the connection-level failures the
    /// client treats as "no broker answered".
    pub fn is_connection(&self) -> bool {
        matches!(self, Error::Io(_) | Error::Eof(_))
    }

    /// Whether this is `TimeoutError`.
    pub fn is_timeout(&self) -> bool {
        matches!(self, Error::Io(error) if error.kind() == io::ErrorKind::TimedOut)
    }

    /// `exc.__class__.__name__` for the details Python reports
    /// (`operator_broker_start_failed`).
    pub fn python_class(&self) -> &'static str {
        match self {
            Error::App(_) => "AppError",
            Error::Eof(_) => "EOFError",
            Error::Value(_) => "ValueError",
            Error::Io(error) => match error.kind() {
                io::ErrorKind::NotFound => "FileNotFoundError",
                io::ErrorKind::ConnectionRefused => "ConnectionRefusedError",
                io::ErrorKind::ConnectionReset => "ConnectionResetError",
                io::ErrorKind::ConnectionAborted => "ConnectionAbortedError",
                io::ErrorKind::BrokenPipe => "BrokenPipeError",
                io::ErrorKind::TimedOut => "TimeoutError",
                io::ErrorKind::PermissionDenied => "PermissionError",
                io::ErrorKind::WouldBlock => "BlockingIOError",
                io::ErrorKind::AlreadyExists => "FileExistsError",
                io::ErrorKind::Interrupted => "InterruptedError",
                io::ErrorKind::NotADirectory => "NotADirectoryError",
                io::ErrorKind::IsADirectory => "IsADirectoryError",
                _ => "OSError",
            },
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::App(error) => f.write_str(&error.message),
            Error::Io(error) => write!(f, "{error}"),
            Error::Eof(message) => f.write_str(message),
            Error::Value(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for Error {}

impl From<AppError> for Error {
    fn from(error: AppError) -> Self {
        Error::App(error)
    }
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Error::Io(error)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// `AppError(message, code=code)`: not retryable unless set.
pub(crate) fn app(code: &str, message: impl Into<String>) -> AppError {
    AppError::new(code, message)
}

/// `AppError(message, code="operator_protocol_error")`.
pub(crate) fn protocol(message: &str) -> Error {
    Error::App(app("operator_protocol_error", message))
}
