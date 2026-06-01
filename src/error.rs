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

    /// The archive is in an inconsistent state because a previous entry
    /// was dropped without being properly closed.
    Poisoned(String),

    /// The archive writer is in a state that does not allow the requested operation.
    InvalidState(String),
}

impl fmt::Display for ZipError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O error: {e}"),
            Self::FieldTooLong { field, len, max } => {
                write!(f, "{field} too long: {len} bytes (max {max})")
            }
            Self::Poisoned(msg) => write!(f, "archive corrupted: {msg}"),
            Self::InvalidState(msg) => write!(f, "invalid state: {msg}"),
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

// Allow seamless conversion back to io::Error for backward compatibility
// or for use in contexts that require io::Result.
impl From<ZipError> for io::Error {
    fn from(err: ZipError) -> Self {
        match err {
            ZipError::Io(e) => e,
            ZipError::FieldTooLong { .. } => io::Error::new(io::ErrorKind::InvalidInput, err),
            ZipError::Poisoned(_) => io::Error::other(err),
            ZipError::InvalidState(_) => io::Error::new(io::ErrorKind::InvalidInput, err),
        }
    }
}
