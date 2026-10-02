//! Engine errors and the classes they cross the boundary with.
//!
//! Every failure the engine reports carries a class and RP2's exact message,
//! so the Python layer can raise the same text RP2 would. The classes mirror
//! the Python exception types RP2 raises for the same condition.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::decimal::DecimalError;

/// The Python exception class an engine error maps to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ErrorClass {
    /// RP2's `RP2ValueError`.
    ValueError,
    /// RP2's `RP2TypeError`.
    TypeError,
    /// RP2's `RP2RuntimeError`.
    RuntimeError,
    /// `decimal.InvalidOperation` (for example a comparison of values of
    /// 1E+19 or more, which RP2's 13-place quantize cannot represent).
    InvalidOperation,
    /// `decimal.DivisionByZero`.
    DivisionByZero,
    /// `decimal.Overflow`.
    Overflow,
    /// The request is malformed: a bug in the caller, not an RP2 condition.
    RequestError,
    /// The engine does not implement the requested method or operation yet.
    Unsupported,
}

/// A failure with its boundary class and message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineError {
    /// The Python exception class.
    pub class: ErrorClass,
    /// The message, identical to RP2's for the same condition.
    pub message: String,
}

impl EngineError {
    /// Builds an error of `class`.
    pub fn new(class: ErrorClass, message: impl Into<String>) -> Self {
        EngineError {
            class,
            message: message.into(),
        }
    }

    /// RP2's `RP2ValueError`.
    pub fn value(message: impl Into<String>) -> Self {
        Self::new(ErrorClass::ValueError, message)
    }

    /// RP2's `RP2TypeError`.
    pub fn type_error(message: impl Into<String>) -> Self {
        Self::new(ErrorClass::TypeError, message)
    }

    /// RP2's `RP2RuntimeError`.
    pub fn runtime(message: impl Into<String>) -> Self {
        Self::new(ErrorClass::RuntimeError, message)
    }

    /// A malformed request.
    pub fn request(message: impl Into<String>) -> Self {
        Self::new(ErrorClass::RequestError, message)
    }

    /// A method or operation the engine does not implement yet.
    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::new(ErrorClass::Unsupported, message)
    }
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.class, self.message)
    }
}

impl std::error::Error for EngineError {}

impl From<DecimalError> for EngineError {
    /// The exception CPython's `decimal` module raises for the condition,
    /// with its message (`str(exc)`).
    fn from(error: DecimalError) -> Self {
        let (class, signal) = match error {
            DecimalError::InvalidOperation => (ErrorClass::InvalidOperation, "InvalidOperation"),
            DecimalError::DivisionUndefined => (ErrorClass::InvalidOperation, "DivisionUndefined"),
            DecimalError::DivisionByZero => (ErrorClass::DivisionByZero, "DivisionByZero"),
            DecimalError::Overflow => (ErrorClass::Overflow, "Overflow"),
            DecimalError::ConversionSyntax
            | DecimalError::NonFinite
            | DecimalError::InvalidContext
            | DecimalError::FormatTooLong => {
                return EngineError::request(format!("invalid decimal: {error}"));
            }
        };
        EngineError::new(class, format!("[<class 'decimal.{signal}'>]"))
    }
}

/// Engine results.
pub type EngineResult<T> = Result<T, EngineError>;
