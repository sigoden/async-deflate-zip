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

    /// The writer is in a corrupted state (e.g. entry dropped without finish).
    EntryWriterCorrupted,

    /// An entry writer is already active; finish it before starting another.
    WriterCorrupted,
}

impl fmt::Display for ZipError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O error: {e}"),
            Self::FieldTooLong { field, len, max } => {
                write!(f, "{field} too long: {len} bytes (max {max})")
            }
            Self::EntryWriterCorrupted => write!(f, "entry writer corrupted"),
            Self::WriterCorrupted => write!(f, "writer corrupted"),
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
            ZipError::FieldTooLong { .. } => io::Error::new(io::ErrorKind::InvalidInput, err),
            ZipError::EntryWriterCorrupted => io::Error::other(err),
            ZipError::WriterCorrupted => io::Error::other(err),
        }
    }
}
