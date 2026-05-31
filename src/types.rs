//! Core types for the async-deflate-zip crate.
//!
//! This module defines the public type system:
//!
//! - [`CompressionLevel`] — DEFLATE compression level (0-9) with predefined constants

// === CompressionLevel ===

/// Deflate compression level (0-9).
///
/// Controls the trade-off between compression speed and ratio.
/// Higher levels produce smaller output but take longer to compress.
///
/// Provides predefined constants [`NONE`](CompressionLevel::NONE),
/// [`DEFAULT`](CompressionLevel::DEFAULT), and [`BEST`](CompressionLevel::BEST).
///
/// # Examples
///
/// ```rust,no_run
/// use async_deflate_zip::CompressionLevel;
///
/// let level = CompressionLevel::new(4);
/// assert_eq!(level.level(), 4);
/// ```
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

    /// Create a new `CompressionLevel`.
    ///
    /// Panics if the level is not in the range 0-9.
    pub fn new(level: u8) -> Self {
        assert!(level <= 9, "compression level must be 0-9");
        Self(level)
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
