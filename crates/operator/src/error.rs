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

/// How a CLI or daemon caller reports an operator failure: an `AppError` as
/// is, any other exception through the generic `internal_error` with
/// Python's `str(exc)` (`[Errno N] text` for an `OSError`, `timed out` for a
/// socket timeout).
pub fn into_app_error(error: Error) -> AppError {
    let message = match error {
        Error::App(error) => return error,
        Error::Io(error) => os_error_text(&error),
        Error::Eof(message) => message.to_owned(),
        Error::Value(message) => message,
    };
    AppError::new("internal_error", message)
}

/// `str(OSError)`.
pub fn os_error_text(error: &io::Error) -> String {
    if let Some(number) = error.raw_os_error() {
        let text = error.to_string();
        let suffix = format!(" (os error {number})");
        let label = if cfg!(windows) { "WinError" } else { "Errno" };
        return format!(
            "[{label} {number}] {}",
            text.strip_suffix(&suffix).unwrap_or(&text)
        );
    }
    if error.kind() == io::ErrorKind::TimedOut {
        return "timed out".to_owned();
    }
    error.to_string()
}

/// `AppError(message, code=code)`: not retryable unless set.
pub(crate) fn app(code: &str, message: impl Into<String>) -> AppError {
    AppError::new(code, message)
}

/// `AppError(message, code="operator_protocol_error")`.
pub(crate) fn protocol(message: &str) -> Error {
    Error::App(app("operator_protocol_error", message))
}

/// A storage-crate error as the `AppError` Python raises for it.
pub fn from_paths(error: kassiber_paths::Error) -> Error {
    let details = if error.details.is_null() {
        None
    } else {
        kassiber_pytext::loads(&error.details.to_string()).ok()
    };
    let mut app = AppError::new(error.code, error.message).retryable(error.retryable);
    app.hint = error.hint.map(|hint| Box::from(hint.as_ref()));
    app.details = details;
    Error::App(app)
}
