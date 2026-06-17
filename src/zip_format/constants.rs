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

/// Version made by: Unix host OS (upper byte = 3) + version 3.0 (lower byte = 30, industry standard).
pub(crate) const VERSION_UNIX: u16 = (3 << 8) | 30;

/// Version needed: 4.5 — supports ZIP64 extensions.
pub(crate) const VERSION_ZIP64: u16 = 45;

/// Maximum value that fits in a ZIP 32-bit size/offset field.
pub(crate) const U32_MAX: u64 = u32::MAX as u64;

/// Maximum entries that fit in a 16-bit EOCDR field.
pub(crate) const U16_ENTRY_MAX: u64 = 0xFFFF;

/// Default Unix permissions for regular files.
pub(crate) const DEFAULT_FILE_PERM: u32 = 0o644;

/// Default Unix permissions for directories.
pub(crate) const DEFAULT_DIR_PERM: u32 = 0o755;

/// Default Unix permissions for symlink files.
pub(crate) const DEFAULT_SYMLINK_PERM: u32 = 0o777;

/// Whether a data descriptor needs ZIP64 extensions.
///
/// ZIP64 is required when compressed or uncompressed size exceeds `U32_MAX`.
/// Unlike entry headers, data descriptors do not carry a local header offset.
pub(crate) const fn data_descriptor_needs_zip64(
    compressed_size: u64,
    uncompressed_size: u64,
) -> bool {
    compressed_size > U32_MAX || uncompressed_size > U32_MAX
}

/// Whether an individual entry needs ZIP64 extensions.
///
/// ZIP64 is required when any of the three 32-bit fields (compressed size,
/// uncompressed size, local header offset) exceeds `U32_MAX`.
pub(crate) const fn entry_needs_zip64(
    compressed_size: u64,
    uncompressed_size: u64,
    local_header_offset: u64,
) -> bool {
    compressed_size > U32_MAX || uncompressed_size > U32_MAX || local_header_offset > U32_MAX
}

/// Whether the archive itself needs ZIP64 records (Zip64Eocdr + Locator).
///
/// ZIP64 is required when the total entry count exceeds 0xFFFF, or the
/// Central Directory size or offset exceeds `U32_MAX`.
pub(crate) const fn archive_needs_zip64(total_entries: u64, cd_size: u64, cd_offset: u64) -> bool {
    total_entries > U16_ENTRY_MAX || cd_size > U32_MAX || cd_offset > U32_MAX
}

/// Data Descriptor signature: `PK\x07\x08` (`0x08074b50`).
pub(crate) const DD_SIG: u32 = 0x08074b50;
