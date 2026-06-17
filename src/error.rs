//! Custom error types for async-deflate-zip.

use std::fmt;
use std::io;

/// Errors that can occur when creating a ZIP archive.
#[derive(Debug)]
pub enum ZipError {
    /// An underlying I/O error occurred.
    Io(io::Error),

    /// A filename or extra field exceeds the maximum allowed length.
    FieldTooLong {
        /// Name of the field (e.g., "filename", "extra").
        field: &'static str,
        /// Actual length in bytes.
        len: usize,
        /// Maximum allowed length.
        max: usize,
    },

    /// Invalid input was provided (e.g. empty name, null byte).
    InvalidInput {
        /// Description of the validation failure.
        reason: &'static str,
    },

    /// An entry writer is still active; finish it before starting another.
    WriterBusy,

    /// The writer is in a poisoned state (e.g. entry dropped without finish).
    Poisoned,
}

impl fmt::Display for ZipError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O error: {e}"),
            Self::FieldTooLong { field, len, max } => {
                write!(f, "{field} too long: {len} bytes (max {max})")
            }
            Self::InvalidInput { reason } => write!(f, "invalid input: {reason}"),
            Self::WriterBusy => write!(f, "an entry writer is still active"),
            Self::Poisoned => {
                write!(f, "writer is poisoned (entry dropped without finish)")
            }
        }
    }
}

impl std::error::Error for ZipError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for ZipError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<ZipError> for io::Error {
    fn from(err: ZipError) -> Self {
        match err {
            ZipError::Io(e) => e,
            ZipError::FieldTooLong { .. } | ZipError::InvalidInput { .. } => {
                io::Error::new(io::ErrorKind::InvalidInput, err)
            }
            ZipError::WriterBusy | ZipError::Poisoned => io::Error::other(err),
        }
    }
}
