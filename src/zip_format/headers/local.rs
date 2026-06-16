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
        put_u32(&mut buf, 0);
        put_u32(&mut buf, 0);
        put_u32(&mut buf, 0);
        put_u16(&mut buf, self.name.len() as u16);
        put_u16(&mut buf, self.extra.len() as u16);
        buf.extend_from_slice(&self.name);
        buf.extend_from_slice(&self.extra);
        Ok(buf)
    }
}

pub(crate) struct DataDescriptor {
    pub(crate) crc32: u32,
    pub(crate) compressed_size: u64,
    pub(crate) uncompressed_size: u64,
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
