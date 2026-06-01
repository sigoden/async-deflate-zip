//! Core types for the async-deflate-zip crate.
//!
//! This module defines the public type system:
//!
//! - [`CompressionLevel`] — DEFLATE compression level (0-9) with predefined constants

use crate::error::ZipError;

// === CompressionLevel ===

/// Deflate compression level (0-9).
///
/// Controls the trade-off between compression speed and ratio.
/// Higher levels produce smaller output but take longer to compress.
///
/// Provides predefined constants [`NONE`](CompressionLevel::NONE),
/// [`DEFAULT`](CompressionLevel::DEFAULT), and [`BEST`](CompressionLevel::BEST).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompressionLevel(u8);

impl CompressionLevel {
    /// No compression (store only).
    pub const NONE: Self = Self(0);

    /// Fast compression (level 1).
    ///
    /// Optimize for the best speed of encoding.
    pub const FAST: Self = Self(1);

    /// Maximum compression (level 9).
    ///
    /// Optimize for the size of data being encoded.
    pub const BEST: Self = Self(9);

    /// Default compression (level 6).
    ///
    /// Balanced trade-off between compression speed and ratio.
    pub const DEFAULT: Self = Self(6);

    /// Create a new `CompressionLevel` with validation.
    ///
    /// Returns `Err(ZipError::InvalidCompressionLevel)` if the level is not
    /// in the valid range 0–9.
    ///
    /// # Example
    ///
    /// ```rust
    /// use async_deflate_zip::CompressionLevel;
    ///
    /// let level = CompressionLevel::try_new(4).unwrap();
    /// assert_eq!(level.level(), 4);
    /// assert!(CompressionLevel::try_new(42).is_err());
    /// ```
    pub fn try_new(level: u8) -> Result<Self, ZipError> {
        if level <= 9 {
            Ok(Self(level))
        } else {
            Err(ZipError::InvalidCompressionLevel(level))
        }
    }

    /// Return the raw compression level value (0-9).
    pub fn level(&self) -> u8 {
        self.0
    }
}

impl Default for CompressionLevel {
    fn default() -> Self {
        Self::DEFAULT
    }
}
