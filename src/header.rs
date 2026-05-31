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
/// Clamps year to the valid MS-DOS range [1980, 2107].
pub(crate) fn system_time_to_ms_dos(t: std::time::SystemTime) -> (u16, u16) {
    let d = t
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days = d / 86400;
    let secs = d % 86400;
    let hour = (secs / 3600) as u16;
    let minute = ((secs % 3600) / 60) as u16;
    let second = (secs % 60 / 2) as u16;

    let (y, m, day) = epoch_days_to_date(days as i64);
    let y = y.clamp(1980, 2107);
    let date = ((y - 1980) as u16) << 9 | (m as u16) << 5 | day;
    let time = hour << 11 | minute << 5 | second;
    (time, date)
}

/// Build the Info-ZIP extended timestamp extra field (ID `0x5455`) with mtime only.
///
/// Format: header_id (2) + data_size (2) + flags (1) + mtime (4) = 9 bytes.
pub(crate) fn build_extended_timestamp_extra(mtime: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(9);
    put_u16(&mut buf, 0x5455);
    put_u16(&mut buf, 5);
    put_u8(&mut buf, 1);
    put_u32(&mut buf, mtime as u32);
    buf
}

/// Convert days since Unix epoch to (year, month, day) in the Gregorian calendar.
pub(crate) fn epoch_days_to_date(mut days: i64) -> (i64, i64, u16) {
    let mut y = 1970i64;
    loop {
        let yd = if is_leap(y) { 366 } else { 365 };
        if days < yd {
            break;
        }
        days -= yd;
        y += 1;
    }
    let leap = is_leap(y);
    let mdays = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut m = 1i64;
    for &md in &mdays {
        if days < md {
            break;
        }
        days -= md;
        m += 1;
    }
    (y, m, (days + 1) as u16)
}

/// Return `true` if `year` is a leap year in the Gregorian calendar.
pub(crate) fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0)
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

impl LocalFileHeader {
    pub(crate) fn new(name: &str, method: u16) -> Self {
        let (time, date) = ms_dos_datetime();
        Self {
            version_needed: if method == METHOD_DEFLATE {
                VERSION_DEFLATE
            } else {
                10
            },
            flags: FLAG_DATA_DESC,
            method,
            time,
            date,
            name: name.as_bytes().to_vec(),
            extra: Vec::new(),
        }
    }

    pub(crate) fn serialize(&self) -> Vec<u8> {
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
        assert!(
            self.name.len() <= u16::MAX as usize,
            "LocalFileHeader filename too long: {} bytes",
            self.name.len()
        );
        assert!(
            self.extra.len() <= u16::MAX as usize,
            "LocalFileHeader extra too long: {} bytes",
            self.extra.len()
        );
        put_u16(&mut buf, self.name.len() as u16);
        put_u16(&mut buf, self.extra.len() as u16);
        buf.extend_from_slice(&self.name);
        buf.extend_from_slice(&self.extra);
        buf
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
            let mut buf = Vec::with_capacity(20);
            put_u32(&mut buf, self.crc32);
            put_u64(&mut buf, self.compressed_size);
            put_u64(&mut buf, self.uncompressed_size);
            buf
        } else {
            let mut buf = Vec::with_capacity(12);
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
    pub(crate) fn serialize(&self) -> Vec<u8> {
        let use_zip64 = self.compressed_size > U32_MAX
            || self.uncompressed_size > U32_MAX
            || self.local_header_offset > U32_MAX;

        let extra = if use_zip64 {
            let mut z64 = Self::zip64_extra(
                self.compressed_size,
                self.uncompressed_size,
                self.local_header_offset,
            );
            z64.extend_from_slice(&self.extra);
            z64
        } else {
            self.extra.clone()
        };

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
        assert!(
            self.name.len() <= u16::MAX as usize,
            "CentralDirEntry filename too long: {} bytes",
            self.name.len()
        );
        assert!(
            extra.len() <= u16::MAX as usize,
            "CentralDirEntry extra too long: {} bytes",
            extra.len()
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
        buf
    }

    fn zip64_extra(compressed_size: u64, uncompressed_size: u64, offset: u64) -> Vec<u8> {
        let mut data = Vec::new();
        put_u16(&mut data, 0x0001);
        let mut size = 0u16;
        if uncompressed_size > U32_MAX {
            size += 8;
        }
        if compressed_size > U32_MAX {
            size += 8;
        }
        if offset > U32_MAX {
            size += 8;
        }
        put_u16(&mut data, size);
        if uncompressed_size > U32_MAX {
            put_u64(&mut data, uncompressed_size);
        }
        if compressed_size > U32_MAX {
            put_u64(&mut data, compressed_size);
        }
        if offset > U32_MAX {
            put_u64(&mut data, offset);
        }
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
        let lfh = LocalFileHeader::new("test.txt", METHOD_DEFLATE);
        let data = lfh.serialize();
        assert_eq!(data.len(), 30 + 8);
        assert_eq!(&data[0..4], &0x04034b50u32.to_le_bytes());
        assert_eq!(&data[8..10], &METHOD_DEFLATE.to_le_bytes());
        assert!(data[6] & (1 << 3) != 0);
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
        assert_eq!(data.len(), 12);
        assert_eq!(&data[0..4], &0x12345678u32.to_le_bytes());
        assert_eq!(&data[4..8], &100u32.to_le_bytes());
        assert_eq!(&data[8..12], &200u32.to_le_bytes());
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
        let data = cde.serialize();
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
        let data = cde.serialize();
        assert_eq!(&data[0..4], &0x02014b50u32.to_le_bytes());
        assert_eq!(&data[20..24], &u32::MAX.to_le_bytes());
        assert_eq!(&data[24..28], &u32::MAX.to_le_bytes());
        assert_eq!(&data[6..8], &VERSION_ZIP64.to_le_bytes());
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
        let data = cde.serialize();

        // Verify ZIP64 sentinels in main fields
        assert_eq!(&data[20..24], &u32::MAX.to_le_bytes());
        assert_eq!(&data[24..28], &u32::MAX.to_le_bytes());

        // Parse extra length
        let name_len = u16::from_le_bytes(data[28..30].try_into().unwrap()) as usize;
        let extra_len = u16::from_le_bytes(data[30..32].try_into().unwrap()) as usize;

        // Extra must be larger than ZIP64 alone (i.e., timestamp is preserved)
        assert!(
            extra_len > 20,
            "expected extra_len > 20 (ZIP64 only), got {extra_len}"
        );

        let extra_start = 46 + name_len;
        let extra_data = &data[extra_start..extra_start + extra_len];

        // Find ZIP64 extra field header (0x0001)
        assert_eq!(
            u16::from_le_bytes(extra_data[0..2].try_into().unwrap()),
            0x0001
        );

        // Find extended timestamp extra field header (0x5455)
        let has_ts = extra_data.windows(4).any(|w| w[0] == 0x55 && w[1] == 0x54);
        assert!(
            has_ts,
            "extended timestamp extra (0x5455) missing from serialized extra field"
        );
    }

    #[test]
    #[should_panic(expected = "filename too long")]
    fn test_local_file_header_name_too_long() {
        // Bug 3: filename exceeding u16::MAX should panic, not silently truncate
        let name = "a".repeat(65536);
        let lfh = LocalFileHeader::new(&name, METHOD_STORED);
        lfh.serialize();
    }

    #[test]
    #[should_panic(expected = "filename too long")]
    fn test_central_dir_entry_name_too_long() {
        // Bug 3: filename exceeding u16::MAX should panic in CD entry as well
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
        cde.serialize();
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
