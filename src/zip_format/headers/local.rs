use crate::error::ZipError;
use crate::zip_format::binary::*;
use crate::zip_format::constants::*;
use crate::zip_format::extra_fields::*;
use crate::zip_format::time::system_time_to_ms_dos;

use std::time::SystemTime;

pub(crate) struct LocalFileHeader {
    pub(crate) version_needed: u16,
    pub(crate) flags: u16,
    pub(crate) method: u16,
    pub(crate) time: u16,
    pub(crate) date: u16,
    pub(crate) name: Vec<u8>,
    pub(crate) extra: Vec<u8>,
}

impl LocalFileHeader {
    pub(crate) fn new(name: &str, method: u16, zip64: bool, mtime: SystemTime) -> Self {
        let (time, date) = system_time_to_ms_dos(mtime);
        let mut flags = FLAG_DATA_DESC;
        if !name.is_ascii() {
            flags |= 1 << 11;
        }
        let unix_secs = mtime
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
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
            extra: {
                let mut extra = Vec::new();
                ExtendedTimestampExtra::new(unix_secs).serialize(&mut extra);
                if !name.is_ascii() {
                    UnicodePathExtra::new(name).serialize(&mut extra);
                }
                if zip64 {
                    Zip64Extra::LocalFileHeader.serialize(&mut extra);
                }
                extra
            },
        }
    }

    pub(crate) fn write_to(&self, buf: &mut Vec<u8>) -> Result<(), ZipError> {
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
        buf.clear();
        buf.reserve(30 + self.name.len() + self.extra.len());
        put_u32(buf, LFH_SIG);
        put_u16(buf, self.version_needed);
        put_u16(buf, self.flags);
        put_u16(buf, self.method);
        put_u16(buf, self.time);
        put_u16(buf, self.date);
        put_u32(buf, 0);
        put_u32(buf, 0);
        put_u32(buf, 0);
        put_u16(buf, self.name.len() as u16);
        put_u16(buf, self.extra.len() as u16);
        buf.extend_from_slice(&self.name);
        buf.extend_from_slice(&self.extra);
        Ok(())
    }
}

pub(crate) struct DataDescriptor {
    pub(crate) crc32: u32,
    pub(crate) compressed_size: u64,
    pub(crate) uncompressed_size: u64,
    pub(crate) zip64: bool,
}

impl DataDescriptor {
    pub(crate) fn write_to(&self, buf: &mut Vec<u8>) {
        buf.clear();
        if self.zip64 {
            buf.reserve(24);
            put_u32(buf, DD_SIG);
            put_u32(buf, self.crc32);
            put_u64(buf, self.compressed_size);
            put_u64(buf, self.uncompressed_size);
        } else {
            buf.reserve(16);
            put_u32(buf, DD_SIG);
            put_u32(buf, self.crc32);
            put_u32(buf, self.compressed_size as u32);
            put_u32(buf, self.uncompressed_size as u32);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::zip_format::binary::test_utils::*;
    use crate::zip_format::*;
    use std::time::SystemTime;

    #[test]
    fn test_lfh_normal_case() {
        let lfh = LocalFileHeader::new("test.txt", METHOD_DEFLATE, false, SystemTime::UNIX_EPOCH);
        let mut buf = Vec::new();
        lfh.write_to(&mut buf).unwrap();
        assert_eq!(buf.len(), 47);
        assert_eq!(read_u32(&buf, 0), LFH_SIG);
        assert_eq!(read_u16(&buf, 8), METHOD_DEFLATE);
        assert!(buf[6] & (1 << 3) != 0);
        assert_eq!(read_u16(&buf, 4), VERSION_DEFLATE);
        assert_eq!(read_u16(&buf, 28) as usize, 9);
    }

    #[test]
    fn test_lfh_zip64_case() {
        let lfh = LocalFileHeader::new("bigfile.bin", METHOD_DEFLATE, true, SystemTime::UNIX_EPOCH);
        let mut buf = Vec::new();
        lfh.write_to(&mut buf).unwrap();
        assert_eq!(read_u16(&buf, 4), VERSION_ZIP64);
        let name_len = read_u16(&buf, 26) as usize;
        let extra_len = read_u16(&buf, 28) as usize;
        assert_eq!(extra_len, 13);
        let extra_start = 30 + name_len;
        let extra = &buf[extra_start..extra_start + extra_len];
        assert_eq!(read_u16(extra, 0), 0x5455);
        assert_eq!(read_u16(extra, 9), 0x0001);

        let lfh = LocalFileHeader::new("bigdir", METHOD_STORED, true, SystemTime::UNIX_EPOCH);
        let mut buf = Vec::new();
        lfh.write_to(&mut buf).unwrap();
        assert_eq!(read_u16(&buf, 4), VERSION_ZIP64);
    }

    #[test]
    fn test_lfh_field_too_long() {
        let name = "a".repeat(65536);
        let lfh = LocalFileHeader::new(&name, METHOD_STORED, false, SystemTime::UNIX_EPOCH);
        let mut buf = Vec::new();
        let result = lfh.write_to(&mut buf);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("filename too long")
        );
    }

    #[test]
    fn test_dd_normal_case() {
        let dd = DataDescriptor {
            crc32: 0x12345678,
            compressed_size: 100,
            uncompressed_size: 200,
            zip64: false,
        };
        let mut buf = Vec::new();
        dd.write_to(&mut buf);
        assert_eq!(buf.len(), 16);
        assert_eq!(read_u32(&buf, 0), DD_SIG);
        assert_eq!(read_u32(&buf, 4), 0x12345678);
        assert_eq!(read_u32(&buf, 8), 100);
        assert_eq!(read_u32(&buf, 12), 200);
    }

    #[test]
    fn test_dd_zip64_case() {
        let mut buf = Vec::new();
        let dd = DataDescriptor {
            crc32: 0x12345678,
            compressed_size: 5_000_000_000,
            uncompressed_size: 10_000_000_000,
            zip64: true,
        };
        dd.write_to(&mut buf);
        assert_eq!(buf.len(), 24);
        assert_eq!(read_u32(&buf, 4), 0x12345678);
        assert_eq!(read_u64(&buf, 8), 5_000_000_000);
        assert_eq!(read_u64(&buf, 16), 10_000_000_000);

        let dd = DataDescriptor {
            crc32: 0xDEADBEEF,
            compressed_size: 100,
            uncompressed_size: 200,
            zip64: true,
        };
        buf.clear();
        dd.write_to(&mut buf);
        assert_eq!(buf.len(), 24);
        assert_eq!(read_u32(&buf, 4), 0xDEADBEEF);
        assert_eq!(read_u64(&buf, 8), 100);
        assert_eq!(read_u64(&buf, 16), 200);
    }
}
