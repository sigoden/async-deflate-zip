//! ZIP binary format header structures and serialization helpers.
//!
//! This module implements the low-level on-disk structures that make up a
//! standard ZIP archive. The ZIP format organizes an archive into three
//! sections:
//!
//! 1. **Entries** — Each file or directory entry starts with a [`LocalFileHeader`]
//!    (signature `PK\x03\x04`), followed by the (optionally compressed) file data,
//!    followed by an optional Data Descriptor containing CRC-32 and sizes.
//! 2. **Central Directory** — A sequence of [`CentralDirEntry`] records (signature
//!    `PK\x01\x02`), one per entry, providing a table of contents.
//! 3. **End of Central Directory** — An [`Eocdr`] record (signature `PK\x05\x06`)
//!    pointing to the central directory. For large archives (ZIP64), additional
//!    [`Zip64Eocdr`] and [`Zip64EocdrLocator`] records are written.
//!
//! All multi-byte values in the ZIP format are little-endian. Compression is
//! indicated by the `method` field in each header.

use crate::error::ZipError;

// === Constants ===

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
///
/// Data is stored as-is without any compression.
pub(crate) const METHOD_STORED: u16 = 0;

/// DEFLATE compression method.
///
/// Standard compression method used in ZIP archives.
pub(crate) const METHOD_DEFLATE: u16 = 8;

/// Flags bit: Data Descriptor follows the file data.
///
/// When set, CRC-32 and sizes are written in a [`DataDescriptor`]
/// after the compressed data rather than in the local file header.
pub(crate) const FLAG_DATA_DESC: u16 = 1 << 3;

/// Version needed: 1.0 (10) — supports stored (uncompressed) entries.
pub(crate) const VERSION_STORED: u16 = 10;

/// Version needed: 2.0 (20) — supports DEFLATE compression.
pub(crate) const VERSION_DEFLATE: u16 = 20;

/// Version made by: Unix host OS (upper byte = 3) + deflate support (lower byte = 20).
pub(crate) const VERSION_UNIX: u16 = (3 << 8) | VERSION_DEFLATE;

/// Version needed: 4.5 — supports ZIP64 extensions.
pub(crate) const VERSION_ZIP64: u16 = 45;

/// Maximum value that fits in a ZIP 32-bit size/offset field.
pub(crate) const U32_MAX: u64 = u32::MAX as u64;

/// Data Descriptor signature: `PK\x07\x08` (`0x08074b50`).
pub(crate) const DD_SIG: u32 = 0x08074b50;

// === Helper: write little-endian integers ===

/// Write a `u16` in little-endian order into a byte buffer.
pub(crate) fn put_u16(buf: &mut Vec<u8>, v: u16) {
    buf.extend_from_slice(&v.to_le_bytes());
}

/// Write a `u32` in little-endian order into a byte buffer.
pub(crate) fn put_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_le_bytes());
}

/// Write a `u64` in little-endian order into a byte buffer.
pub(crate) fn put_u64(buf: &mut Vec<u8>, v: u64) {
    buf.extend_from_slice(&v.to_le_bytes());
}

/// Write a `u8` into a byte buffer.
pub(crate) fn put_u8(buf: &mut Vec<u8>, v: u8) {
    buf.push(v);
}

// === MS-DOS date/time ===

/// Return the current time and date in MS-DOS format.
///
/// MS-DOS date/time packs into two 16-bit values:
/// - **Time**: hours (5 bits), minutes (6 bits), seconds/2 (5 bits)
/// - **Date**: year-1980 (7 bits), month (4 bits), day (5 bits)
pub(crate) fn ms_dos_datetime() -> (u16, u16) {
    system_time_to_ms_dos(std::time::SystemTime::now())
}

/// Convert a `SystemTime` to MS-DOS date/time format.
///
/// Converts the UTC input to local time using the `time` crate before packing
/// into MS-DOS format. Clamps year to the valid MS-DOS range [1980, 2107].
pub(crate) fn system_time_to_ms_dos(t: std::time::SystemTime) -> (u16, u16) {
    let local_offset = time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC);
    let dt = time::OffsetDateTime::from(t).to_offset(local_offset);
    let y = dt.year().clamp(1980, 2107);
    let m = u8::from(dt.month()) as u16;
    let day = dt.day() as u16;
    let hour = dt.hour() as u16;
    let minute = dt.minute() as u16;
    let second = (dt.second() / 2) as u16;
    let date = ((y - 1980) as u16) << 9 | m << 5 | day;
    let time = hour << 11 | minute << 5 | second;
    (time, date)
}

/// Build the Info-ZIP extended timestamp extra field (ID `0x5455`) with mtime only.
///
/// Format: header_id (2) + data_size (2) + flags (1) + mtime (4) = 9 bytes.
/// The mtime field is 32-bit, so values exceeding `u32::MAX` (2106-02-07) are
/// clamped to `u32::MAX` to avoid silent wrap-around.
pub(crate) fn build_extended_timestamp_extra(mtime: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(9);
    put_u16(&mut buf, 0x5455);
    put_u16(&mut buf, 5);
    put_u8(&mut buf, 1);
    put_u32(&mut buf, mtime.min(u32::MAX as u64) as u32);
    buf
}

/// Convert an optional `SystemTime` to MS-DOS date/time and Unix timestamp pair.
///
/// Used by both `EntryWriter::close` and `DirectoryWriter::close` to convert
/// the user-set mtime into the MS-DOS format stored in the Central Directory
/// header and the Unix seconds stored in the extended timestamp extra field.
pub(crate) fn mtime_to_ms_dos_and_unix(
    mtime: Option<std::time::SystemTime>,
) -> (Option<(u16, u16)>, Option<u64>) {
    match mtime {
        Some(t) => {
            let (time, date) = system_time_to_ms_dos(t);
            let secs = t
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            (Some((time, date)), Some(secs))
        }
        None => (None, None),
    }
}

// === Header structures ===

/// Local File Header (30 bytes + filename + extra).
///
/// Precedes each file entry in the archive. Written before the
/// (optionally compressed) file data.
pub(crate) struct LocalFileHeader {
    /// Minimum ZIP version needed to extract this entry (e.g. 20 for deflate, 45 for ZIP64).
    pub(crate) version_needed: u16,
    /// General purpose bit flag (bit 3 = data descriptor follows).
    pub(crate) flags: u16,
    /// Compression method: [`METHOD_STORED`] (0) or [`METHOD_DEFLATE`] (8).
    pub(crate) method: u16,
    /// Last modification time in MS-DOS format.
    pub(crate) time: u16,
    /// Last modification date in MS-DOS format.
    pub(crate) date: u16,
    /// Entry filename (UTF-8 bytes).
    pub(crate) name: Vec<u8>,
    /// Extra field data (ZIP64 info, etc.).
    pub(crate) extra: Vec<u8>,
}

/// Build a ZIP64 extra field for a Local File Header where sizes are
/// recorded via data descriptor (0 in the header fields).
///
/// Since the data descriptor flag is set, the 32-bit size fields in the
/// LFH are 0 (not 0xFFFFFFFF sentinels). The ZIP64 extra field includes
/// the 0x0001 tag header with data_size=0 to signal ZIP64 usage without
/// carrying redundant zero-size fields. This satisfies parsers that check
/// for the ZIP64 extra field tag to determine ZIP64 capability.
pub(crate) fn build_zip64_extra_lfh() -> Vec<u8> {
    let mut data = Vec::with_capacity(4);
    put_u16(&mut data, 0x0001); // ZIP64 extra ID
    put_u16(&mut data, 0); // data size = 0 (sizes in data descriptor)
    data
}

impl LocalFileHeader {
    pub(crate) fn new(name: &str, method: u16, zip64: bool) -> Self {
        let (time, date) = ms_dos_datetime();
        let mut flags = FLAG_DATA_DESC;
        if !name.is_ascii() {
            flags |= 1 << 11; // EFS / Language encoding flag (bit 11)
        }
        Self {
            version_needed: if zip64 {
                VERSION_ZIP64
            } else {
                match method {
                    METHOD_STORED => VERSION_STORED,
                    METHOD_DEFLATE => VERSION_DEFLATE,
                    _ => VERSION_DEFLATE,
                }
            },
            flags,
            method,
            time,
            date,
            name: name.as_bytes().to_vec(),
            extra: if zip64 {
                build_zip64_extra_lfh()
            } else {
                Vec::new()
            },
        }
    }

    pub(crate) fn serialize(&self) -> Result<Vec<u8>, ZipError> {
        if self.name.len() > u16::MAX as usize {
            return Err(ZipError::FieldTooLong {
                field: "LocalFileHeader filename",
                len: self.name.len(),
                max: u16::MAX as usize,
            });
        }
        if self.extra.len() > u16::MAX as usize {
            return Err(ZipError::FieldTooLong {
                field: "LocalFileHeader extra",
                len: self.extra.len(),
                max: u16::MAX as usize,
            });
        }
        let mut buf = Vec::with_capacity(30 + self.name.len() + self.extra.len());
        put_u32(&mut buf, LFH_SIG);
        put_u16(&mut buf, self.version_needed);
        put_u16(&mut buf, self.flags);
        put_u16(&mut buf, self.method);
        put_u16(&mut buf, self.time);
        put_u16(&mut buf, self.date);
        put_u32(&mut buf, 0); // crc32 — in data descriptor
        put_u32(&mut buf, 0); // compressed size — in data descriptor
        put_u32(&mut buf, 0); // uncompressed size — in data descriptor
        put_u16(&mut buf, self.name.len() as u16);
        put_u16(&mut buf, self.extra.len() as u16);
        buf.extend_from_slice(&self.name);
        buf.extend_from_slice(&self.extra);
        Ok(buf)
    }
}

/// Data Descriptor (12 or 20 bytes).
///
/// Written after the compressed file data when the `FLAG_DATA_DESC` bit
/// is set in the local file header. Contains the CRC-32 and sizes that
/// were unknown at the time the local file header was written.
pub(crate) struct DataDescriptor {
    /// CRC-32 checksum of the uncompressed data.
    pub(crate) crc32: u32,
    /// Size of the compressed data in bytes.
    pub(crate) compressed_size: u64,
    /// Size of the original uncompressed data in bytes.
    pub(crate) uncompressed_size: u64,
    /// If `true`, sizes are serialized as 8-byte (ZIP64); otherwise 4-byte.
    pub(crate) zip64: bool,
}

impl DataDescriptor {
    pub(crate) fn serialize(&self) -> Vec<u8> {
        if self.zip64 {
            let mut buf = Vec::with_capacity(24);
            put_u32(&mut buf, DD_SIG);
            put_u32(&mut buf, self.crc32);
            put_u64(&mut buf, self.compressed_size);
            put_u64(&mut buf, self.uncompressed_size);
            buf
        } else {
            let mut buf = Vec::with_capacity(16);
            put_u32(&mut buf, DD_SIG);
            put_u32(&mut buf, self.crc32);
            put_u32(&mut buf, self.compressed_size as u32);
            put_u32(&mut buf, self.uncompressed_size as u32);
            buf
        }
    }
}

/// Central Directory Entry (46 bytes + filename + extra).
///
/// One entry per file/directory in the Central Directory, which serves
/// as the archive's table of contents.
pub(crate) struct CentralDirEntry {
    /// ZIP version that created this entry.
    pub(crate) version_made_by: u16,
    /// Minimum ZIP version needed to extract this entry.
    pub(crate) version_needed: u16,
    /// General purpose bit flag.
    pub(crate) flags: u16,
    /// Compression method.
    pub(crate) method: u16,
    /// Last modification time in MS-DOS format.
    pub(crate) time: u16,
    /// Last modification date in MS-DOS format.
    pub(crate) date: u16,
    /// CRC-32 checksum of the uncompressed data.
    pub(crate) crc32: u32,
    /// Size of compressed data in bytes.
    pub(crate) compressed_size: u64,
    /// Size of original uncompressed data in bytes.
    pub(crate) uncompressed_size: u64,
    /// Entry filename (UTF-8 bytes).
    pub(crate) name: Vec<u8>,
    /// Extra field data (ZIP64 info, etc.).
    pub(crate) extra: Vec<u8>,
    /// Offset of this entry's Local File Header from the start of the archive.
    pub(crate) local_header_offset: u64,
    /// External file attributes, host-OS dependent.
    /// For Unix (version_made_by upper byte = 3), upper 16 bits hold st_mode.
    pub(crate) external_file_attributes: u32,
}

impl CentralDirEntry {
    pub(crate) fn serialize(&self) -> Result<Vec<u8>, ZipError> {
        if self.name.len() > u16::MAX as usize {
            return Err(ZipError::FieldTooLong {
                field: "CentralDirEntry filename",
                len: self.name.len(),
                max: u16::MAX as usize,
            });
        }

        let use_zip64 = self.compressed_size > U32_MAX
            || self.uncompressed_size > U32_MAX
            || self.local_header_offset > U32_MAX;

        let extra = if use_zip64 {
            let mut extra = self.extra.clone();
            extra.extend(Self::zip64_extra(
                self.compressed_size,
                self.uncompressed_size,
                self.local_header_offset,
            ));
            extra
        } else {
            self.extra.clone()
        };

        if extra.len() > u16::MAX as usize {
            return Err(ZipError::FieldTooLong {
                field: "CentralDirEntry extra",
                len: extra.len(),
                max: u16::MAX as usize,
            });
        }

        let mut buf = Vec::with_capacity(46 + self.name.len() + extra.len());
        put_u32(&mut buf, CD_SIG);
        put_u16(&mut buf, self.version_made_by);
        put_u16(
            &mut buf,
            if use_zip64 {
                VERSION_ZIP64
            } else {
                self.version_needed
            },
        );
        put_u16(&mut buf, self.flags);
        put_u16(&mut buf, self.method);
        put_u16(&mut buf, self.time);
        put_u16(&mut buf, self.date);
        put_u32(&mut buf, self.crc32);
        put_u32(
            &mut buf,
            if use_zip64 {
                u32::MAX
            } else {
                self.compressed_size as u32
            },
        );
        put_u32(
            &mut buf,
            if use_zip64 {
                u32::MAX
            } else {
                self.uncompressed_size as u32
            },
        );
        put_u16(&mut buf, self.name.len() as u16);
        put_u16(&mut buf, extra.len() as u16);
        put_u16(&mut buf, 0); // file comment length
        put_u16(&mut buf, 0); // disk number start
        put_u16(&mut buf, 0); // internal file attributes
        put_u32(&mut buf, self.external_file_attributes);
        put_u32(
            &mut buf,
            if use_zip64 {
                u32::MAX
            } else {
                self.local_header_offset as u32
            },
        );
        buf.extend_from_slice(&self.name);
        buf.extend_from_slice(&extra);
        Ok(buf)
    }

    fn zip64_extra(compressed_size: u64, uncompressed_size: u64, offset: u64) -> Vec<u8> {
        // Always write all three u64 fields in fixed order.
        // Info-ZIP unzip expects the full 24-byte body when ZIP64 is used,
        // even if some values fit in 32 bits. Omitting fields causes
        // positional misalignment and "extra field corrupt" warnings.
        let mut data = Vec::with_capacity(28);
        put_u16(&mut data, 0x0001); // ZIP64 extra ID
        put_u16(&mut data, 24); // data size = 24 (3 × u64)
        put_u64(&mut data, uncompressed_size);
        put_u64(&mut data, compressed_size);
        put_u64(&mut data, offset);
        data
    }
}

/// End of Central Directory Record (22 bytes).
///
/// The final record in a ZIP archive. Points to the Central Directory
/// and records the total number of entries. For ZIP64 archives, this
/// record uses sentinel values (`0xFFFF`, `u32::MAX`) and is followed
/// by ZIP64 end-of-central-directory records.
pub(crate) struct Eocdr {
    /// Total number of entries in the central directory.
    pub(crate) total_entries: u64,
    /// Size of the central directory in bytes.
    pub(crate) cd_size: u64,
    /// Offset of the central directory from the start of the archive.
    pub(crate) cd_offset: u64,
}

impl Eocdr {
    pub(crate) fn serialize(&self) -> Vec<u8> {
        let use_zip64 =
            self.total_entries > 0xFFFF || self.cd_size > U32_MAX || self.cd_offset > U32_MAX;

        let mut buf = Vec::with_capacity(22);
        put_u32(&mut buf, EOCDR_SIG);
        put_u16(&mut buf, 0);
        put_u16(&mut buf, 0);
        let total = if use_zip64 {
            0xFFFF
        } else {
            self.total_entries as u16
        };
        put_u16(&mut buf, total);
        put_u16(&mut buf, total);
        put_u32(
            &mut buf,
            if use_zip64 {
                u32::MAX
            } else {
                self.cd_size as u32
            },
        );
        put_u32(
            &mut buf,
            if use_zip64 {
                u32::MAX
            } else {
                self.cd_offset as u32
            },
        );
        put_u16(&mut buf, 0);
        buf
    }
}

/// ZIP64 End of Central Directory Record (56 bytes).
///
/// Used when the archive contains more than 65535 entries, or the
/// central directory is larger than 4 GiB, or the central directory
/// offset exceeds 4 GiB.
pub(crate) struct Zip64Eocdr {
    /// Total number of entries in the central directory.
    pub(crate) total_entries: u64,
    /// Size of the central directory in bytes.
    pub(crate) cd_size: u64,
    /// Offset of the central directory from the start of the archive.
    pub(crate) cd_offset: u64,
}

impl Zip64Eocdr {
    pub(crate) fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(56);
        put_u32(&mut buf, EOCDR64_SIG);
        put_u64(&mut buf, 44);
        put_u16(&mut buf, VERSION_ZIP64);
        put_u16(&mut buf, VERSION_ZIP64);
        put_u32(&mut buf, 0);
        put_u32(&mut buf, 0);
        put_u64(&mut buf, self.total_entries);
        put_u64(&mut buf, self.total_entries);
        put_u64(&mut buf, self.cd_size);
        put_u64(&mut buf, self.cd_offset);
        buf
    }
}

/// ZIP64 End of Central Directory Locator (20 bytes).
///
/// Points from the original EOCDR location to the ZIP64 EOCDR.
/// Always written before the standard EOCDR when ZIP64 is used.
pub(crate) struct Zip64EocdrLocator {
    /// Offset of the ZIP64 EOCDR from the start of the archive.
    pub(crate) eocdr64_offset: u64,
}

impl Zip64EocdrLocator {
    pub(crate) fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(20);
        put_u32(&mut buf, EOCDR64L_SIG);
        put_u32(&mut buf, 0);
        put_u64(&mut buf, self.eocdr64_offset);
        put_u32(&mut buf, 1);
        buf
    }
}

// === Tests ===

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_local_file_header_size() {
        let lfh = LocalFileHeader::new("test.txt", METHOD_DEFLATE, false);
        let data = lfh.serialize().unwrap();
        assert_eq!(data.len(), 30 + 8);
        assert_eq!(&data[0..4], &0x04034b50u32.to_le_bytes());
        assert_eq!(&data[8..10], &METHOD_DEFLATE.to_le_bytes());
        assert!(data[6] & (1 << 3) != 0);
        assert_eq!(
            u16::from_le_bytes(data[4..6].try_into().unwrap()),
            20,
            "expected VERSION_DEFLATE"
        );
        let extra_len = u16::from_le_bytes(data[28..30].try_into().unwrap()) as usize;
        assert_eq!(extra_len, 0, "expected no extra field, got {extra_len}");
    }

    #[test]
    fn test_local_file_header_zip64() {
        // When zip64=true, version_needed should be 45 and extra should contain
        // the ZIP64 extra ID (0x0001) to signal ZIP64 capability.
        let lfh = LocalFileHeader::new("bigfile.bin", METHOD_DEFLATE, true);
        let data = lfh.serialize().unwrap();
        // version_needed at offset 4
        assert_eq!(
            u16::from_le_bytes(data[4..6].try_into().unwrap()),
            VERSION_ZIP64,
            "expected VERSION_ZIP64 (45) for ZIP64 LFH"
        );
        // extra field should not be empty
        let name_len = u16::from_le_bytes(data[26..28].try_into().unwrap()) as usize;
        let extra_len = u16::from_le_bytes(data[28..30].try_into().unwrap()) as usize;
        assert!(
            extra_len >= 4,
            "expected non-empty ZIP64 extra field, got {extra_len}"
        );
        let extra_start = 30 + name_len;
        let extra = &data[extra_start..extra_start + extra_len];
        // Should contain ZIP64 extra ID 0x0001
        assert_eq!(
            u16::from_le_bytes(extra[0..2].try_into().unwrap()),
            0x0001,
            "expected ZIP64 extra ID (0x0001)"
        );
        // Also test with stored method + zip64 to confirm VERSION_ZIP64 takes priority
        let lfh_stored = LocalFileHeader::new("bigdir", METHOD_STORED, true);
        let data2 = lfh_stored.serialize().unwrap();
        assert_eq!(
            u16::from_le_bytes(data2[4..6].try_into().unwrap()),
            VERSION_ZIP64,
            "expected VERSION_ZIP64 (45) for ZIP64 LFH even with METHOD_STORED"
        );
    }

    #[test]
    fn test_data_descriptor() {
        let dd = DataDescriptor {
            crc32: 0x12345678,
            compressed_size: 100,
            uncompressed_size: 200,
            zip64: false,
        };
        let data = dd.serialize();
        assert_eq!(data.len(), 16);
        assert_eq!(&data[0..4], &0x08074b50u32.to_le_bytes());
        assert_eq!(&data[4..8], &0x12345678u32.to_le_bytes());
        assert_eq!(&data[8..12], &100u32.to_le_bytes());
        assert_eq!(&data[12..16], &200u32.to_le_bytes());
    }

    #[test]
    fn test_central_dir_entry() {
        let cde = CentralDirEntry {
            version_made_by: VERSION_DEFLATE,
            version_needed: VERSION_DEFLATE,
            flags: FLAG_DATA_DESC,
            method: METHOD_DEFLATE,
            time: 0,
            date: 0,
            crc32: 0xDEADBEEF,
            compressed_size: 500,
            uncompressed_size: 1000,
            name: b"test.txt".to_vec(),
            extra: Vec::new(),
            local_header_offset: 0,
            external_file_attributes: 0,
        };
        let data = cde.serialize().unwrap();
        assert_eq!(&data[0..4], &0x02014b50u32.to_le_bytes());
        assert_eq!(&data[16..20], &0xDEADBEEFu32.to_le_bytes());
        assert_eq!(&data[20..24], &500u32.to_le_bytes());
        assert_eq!(&data[24..28], &1000u32.to_le_bytes());
    }

    #[test]
    fn test_central_dir_entry_zip64() {
        let cde = CentralDirEntry {
            version_made_by: VERSION_DEFLATE,
            version_needed: VERSION_DEFLATE,
            flags: FLAG_DATA_DESC,
            method: METHOD_DEFLATE,
            time: 0,
            date: 0,
            crc32: 0,
            compressed_size: 5_000_000_000,
            uncompressed_size: 10_000_000_000,
            name: b"big_file.bin".to_vec(),
            extra: Vec::new(),
            local_header_offset: 0,
            external_file_attributes: 0,
        };
        let data = cde.serialize().unwrap();

        assert_eq!(&data[0..4], &0x02014b50u32.to_le_bytes());
        assert_eq!(&data[20..24], &u32::MAX.to_le_bytes());
        assert_eq!(&data[24..28], &u32::MAX.to_le_bytes());
        assert_eq!(&data[6..8], &VERSION_ZIP64.to_le_bytes());

        // Verify ZIP64 extra field is exactly 28 bytes with all 3 fields
        let name_len = u16::from_le_bytes(data[28..30].try_into().unwrap()) as usize;
        let extra_len = u16::from_le_bytes(data[30..32].try_into().unwrap()) as usize;
        assert_eq!(
            extra_len, 28,
            "expected 28-byte ZIP64 extra, got {extra_len}"
        );

        let extra_start = 46 + name_len;
        let extra = &data[extra_start..extra_start + extra_len];
        assert_eq!(u16::from_le_bytes(extra[0..2].try_into().unwrap()), 0x0001);
        assert_eq!(u16::from_le_bytes(extra[2..4].try_into().unwrap()), 24);
        assert_eq!(
            u64::from_le_bytes(extra[4..12].try_into().unwrap()),
            10_000_000_000
        );
        assert_eq!(
            u64::from_le_bytes(extra[12..20].try_into().unwrap()),
            5_000_000_000
        );
        assert_eq!(u64::from_le_bytes(extra[20..28].try_into().unwrap()), 0);
    }

    #[test]
    fn test_data_descriptor_zip64() {
        let dd = DataDescriptor {
            crc32: 0x12345678,
            compressed_size: 5_000_000_000,
            uncompressed_size: 10_000_000_000,
            zip64: true,
        };
        let data = dd.serialize();
        assert_eq!(data.len(), 24);
        assert_eq!(&data[0..4], &0x08074b50u32.to_le_bytes());
        assert_eq!(&data[4..8], &0x12345678u32.to_le_bytes());
        assert_eq!(&data[8..16], &5_000_000_000u64.to_le_bytes());
        assert_eq!(&data[16..24], &10_000_000_000u64.to_le_bytes());
    }

    #[test]
    fn test_data_descriptor_zip64_small_sizes() {
        // Simulate a small file at local_header_offset > U32_MAX.
        // Sizes fit in 32 bits, but zip64=true (triggered by offset in close()).
        // DD must still be 24 bytes with 8-byte size fields.
        let dd = DataDescriptor {
            crc32: 0xDEADBEEF,
            compressed_size: 100,
            uncompressed_size: 200,
            zip64: true,
        };
        let data = dd.serialize();
        assert_eq!(data.len(), 24);
        assert_eq!(&data[0..4], &0x08074b50u32.to_le_bytes());
        assert_eq!(&data[4..8], &0xDEADBEEFu32.to_le_bytes());
        assert_eq!(&data[8..16], &100u64.to_le_bytes());
        assert_eq!(&data[16..24], &200u64.to_le_bytes());
    }

    #[test]
    fn test_eocdr() {
        let eocdr = Eocdr {
            total_entries: 10,
            cd_size: 5000,
            cd_offset: 10000,
        };
        let data = eocdr.serialize();
        assert_eq!(&data[0..4], &0x06054b50u32.to_le_bytes());
        assert_eq!(&data[8..10], &10u16.to_le_bytes());
        assert_eq!(&data[10..12], &10u16.to_le_bytes());
        assert_eq!(&data[12..16], &5000u32.to_le_bytes());
        assert_eq!(&data[16..20], &10000u32.to_le_bytes());
    }

    #[test]
    fn test_eocdr_zip64() {
        // ZIP64 sentinels: entries > 0xFFFF, cd_size/cd_offset > U32_MAX
        let eocdr = Eocdr {
            total_entries: 70000,
            cd_size: 5_000_000_000,
            cd_offset: 6_000_000_000,
        };
        let data = eocdr.serialize();
        assert_eq!(&data[8..10], &0xFFFFu16.to_le_bytes());
        assert_eq!(&data[10..12], &0xFFFFu16.to_le_bytes());
        assert_eq!(&data[12..16], &u32::MAX.to_le_bytes());
        assert_eq!(&data[16..20], &u32::MAX.to_le_bytes());
    }

    #[test]
    fn test_zip64_eocdr() {
        let z64 = Zip64Eocdr {
            total_entries: 70000,
            cd_size: 5_000_000_000,
            cd_offset: 6_000_000_000,
        };
        let data = z64.serialize();
        assert_eq!(data.len(), 56);
        assert_eq!(&data[0..4], &0x06064b50u32.to_le_bytes());
        assert_eq!(&data[4..12], &44u64.to_le_bytes()); // size of remaining record
        assert_eq!(&data[12..14], &VERSION_ZIP64.to_le_bytes());
        assert_eq!(&data[14..16], &VERSION_ZIP64.to_le_bytes());
        assert_eq!(&data[24..32], &70000u64.to_le_bytes()); // total entries
        assert_eq!(&data[32..40], &70000u64.to_le_bytes()); // same
        assert_eq!(&data[40..48], &5_000_000_000u64.to_le_bytes());
        assert_eq!(&data[48..56], &6_000_000_000u64.to_le_bytes());
    }

    #[test]
    fn test_zip64_eocdr_locator() {
        let loc = Zip64EocdrLocator {
            eocdr64_offset: 6_000_000_000,
        };
        let data = loc.serialize();
        assert_eq!(data.len(), 20);
        assert_eq!(&data[0..4], &0x07064b50u32.to_le_bytes());
        assert_eq!(&data[4..8], &0u32.to_le_bytes()); // disk number
        assert_eq!(&data[8..16], &6_000_000_000u64.to_le_bytes());
        assert_eq!(&data[16..20], &1u32.to_le_bytes()); // total disks
    }

    #[test]
    fn test_central_dir_entry_zip64_with_extra_field() {
        // Bug 1 regression: ZIP64 extra must NOT replace existing extra (e.g. extended timestamp)
        let timestamp_extra = build_extended_timestamp_extra(1700000000);
        let cde = CentralDirEntry {
            version_made_by: VERSION_UNIX,
            version_needed: VERSION_DEFLATE,
            flags: FLAG_DATA_DESC,
            method: METHOD_DEFLATE,
            time: 0,
            date: 0,
            crc32: 0x12345678,
            compressed_size: 5_000_000_000,
            uncompressed_size: 10_000_000_000,
            name: b"big_with_ts.bin".to_vec(),
            extra: timestamp_extra.clone(),
            local_header_offset: 0,
            external_file_attributes: 0,
        };
        let data = cde.serialize().unwrap();

        // Verify ZIP64 sentinels in main fields
        assert_eq!(&data[20..24], &u32::MAX.to_le_bytes());
        assert_eq!(&data[24..28], &u32::MAX.to_le_bytes());

        // Parse extra length
        let name_len = u16::from_le_bytes(data[28..30].try_into().unwrap()) as usize;
        let extra_len = u16::from_le_bytes(data[30..32].try_into().unwrap()) as usize;

        // UT (9 bytes) + ZIP64 (28 bytes) = 37 bytes
        assert_eq!(
            extra_len, 37,
            "expected 37-byte combined extra, got {extra_len}"
        );

        let extra_start = 46 + name_len;
        let extra_data = &data[extra_start..extra_start + extra_len];

        // UT should come first
        assert_eq!(&extra_data[0..2], b"UT");
        // ZIP64 should follow at offset 9
        assert_eq!(
            u16::from_le_bytes(extra_data[9..11].try_into().unwrap()),
            0x0001
        );
        assert_eq!(
            u16::from_le_bytes(extra_data[11..13].try_into().unwrap()),
            24
        );
    }

    #[test]
    fn test_local_file_header_name_too_long() {
        // Filename exceeding u16::MAX should return an error, not silently truncate
        let name = "a".repeat(65536);
        let lfh = LocalFileHeader::new(&name, METHOD_STORED, false);
        let result = lfh.serialize();
        assert!(result.is_err(), "expected Err for oversized filename");
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("filename too long"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn test_central_dir_entry_name_too_long() {
        // Filename exceeding u16::MAX should return an error in CD entry as well
        let cde = CentralDirEntry {
            version_made_by: VERSION_DEFLATE,
            version_needed: VERSION_DEFLATE,
            flags: FLAG_DATA_DESC,
            method: METHOD_STORED,
            time: 0,
            date: 0,
            crc32: 0,
            compressed_size: 0,
            uncompressed_size: 0,
            name: "a".repeat(65536).into_bytes(),
            extra: Vec::new(),
            local_header_offset: 0,
            external_file_attributes: 0,
        };
        let result = cde.serialize();
        assert!(result.is_err(), "expected Err for oversized filename");
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("filename too long"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn test_ms_dos_datetime() {
        let (time, date) = ms_dos_datetime();
        let hour = time >> 11;
        let min = (time >> 5) & 0x3F;
        assert!(hour <= 23);
        assert!(min <= 59);
        let year = (date >> 9) + 1980;
        let month = (date >> 5) & 0x0F;
        let day = date & 0x1F;
        assert!(year >= 2026);
        assert!(month >= 1 && month <= 12);
        assert!(day >= 1 && day <= 31);
    }
}
