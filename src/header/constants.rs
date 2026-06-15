/// Local File Header signature: `PK\x03\x04` (`0x04034b50`).
pub(crate) const LFH_SIG: u32 = 0x04034b50;

/// Central Directory Entry signature: `PK\x01\x02` (`0x02014b50`).
pub(crate) const CD_SIG: u32 = 0x02014b50;

/// End of Central Directory Record signature: `PK\x05\x06` (`0x06054b50`).
pub(crate) const EOCDR_SIG: u32 = 0x06054b50;

/// ZIP64 End of Central Directory Record signature: `PK\x06\x06` (`0x06064b50`).
pub(crate) const EOCDR64_SIG: u32 = 0x06064b50;

/// ZIP64 End of Central Directory Locator signature: `PK\x07\x06` (`0x07064b50`).
pub(crate) const EOCDR64L_SIG: u32 = 0x07064b50;

/// No compression (stored method).
pub(crate) const METHOD_STORED: u16 = 0;

/// DEFLATE compression method.
pub(crate) const METHOD_DEFLATE: u16 = 8;

/// Flags bit: Data Descriptor follows the file data.
pub(crate) const FLAG_DATA_DESC: u16 = 1 << 3;

/// Version needed: 1.0 (10) — supports stored (uncompressed) entries.
pub(crate) const VERSION_STORED: u16 = 10;

/// Version needed: 2.0 (20) — supports DEFLATE compression.
pub(crate) const VERSION_DEFLATE: u16 = 20;

/// Version made by: Unix host OS (upper byte = 3) + version 3.0 (lower byte = 30).
pub(crate) const VERSION_UNIX: u16 = (3 << 8) | 30;

/// Version needed: 4.5 — supports ZIP64 extensions.
pub(crate) const VERSION_ZIP64: u16 = 45;

/// Maximum value that fits in a ZIP 32-bit size/offset field.
pub(crate) const U32_MAX: u64 = u32::MAX as u64;

/// Data Descriptor signature: `PK\x07\x08` (`0x08074b50`).
pub(crate) const DD_SIG: u32 = 0x08074b50;
