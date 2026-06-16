/// Compression level for entries.
///
/// Maps to standard zlib compression levels 0–9:
/// - [`none()`](Self::none)   → level 0 (ZIP **stored**, no deflate compression)
/// - [`fast()`](Self::fast)   → level 1 (fastest deflate compression)
/// - [`default()`](Self::default) → level 6 (default deflate compression)
/// - [`best()`](Self::best)   → level 9 (maximum deflate compression)
///
/// Level 0 is special: it bypasses the deflate encoder entirely and stores the
/// entry data verbatim (ZIP method 0 — stored), rather than running deflate at
/// level 0. This means stored entries are neither compressed nor wrapped in a
/// deflate frame, which is the correct behavior for the ZIP format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompressionLevel(u8);

impl CompressionLevel {
    /// Level 0: ZIP **stored** (no deflate compression at all).
    ///
    /// Unlike levels 1–9, this bypasses the deflate encoder entirely. Entry
    /// data is written as-is with ZIP method 0 (stored), not wrapped in a
    /// deflate frame.
    pub const fn none() -> Self {
        CompressionLevel(0)
    }

    /// Level 1: fastest compression.
    pub const fn fast() -> Self {
        CompressionLevel(1)
    }

    /// Level 9: maximum compression.
    pub const fn best() -> Self {
        CompressionLevel(9)
    }

    /// Create a level from an arbitrary 0–9 value (values > 9 are clamped).
    pub const fn new(level: u8) -> Self {
        CompressionLevel(if level > 9 { 9 } else { level })
    }

    /// Return the raw compression level (0–9).
    pub const fn value(&self) -> u8 {
        self.0
    }
}

impl Default for CompressionLevel {
    fn default() -> Self {
        CompressionLevel(6)
    }
}

impl From<CompressionLevel> for flate2::Compression {
    fn from(level: CompressionLevel) -> Self {
        flate2::Compression::new(level.value() as u32)
    }
}
