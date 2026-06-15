use std::time::SystemTime;

use crate::error::ZipError;

use super::constants::*;
use super::serialize::*;

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
                extra.extend(build_extended_timestamp_extra(unix_secs));
                if !name.is_ascii() {
                    extra.extend(build_unicode_extra_field(name));
                }
                if zip64 {
                    extra.extend(build_zip64_extra_lfh());
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

pub(crate) struct CentralDirEntry {
    pub(crate) version_made_by: u16,
    pub(crate) version_needed: u16,
    pub(crate) flags: u16,
    pub(crate) method: u16,
    pub(crate) time: u16,
    pub(crate) date: u16,
    pub(crate) crc32: u32,
    pub(crate) compressed_size: u64,
    pub(crate) uncompressed_size: u64,
    pub(crate) name: Vec<u8>,
    pub(crate) extra: Vec<u8>,
    pub(crate) local_header_offset: u64,
    pub(crate) external_file_attributes: u32,
    pub(crate) comment: Option<Vec<u8>>,
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
        let comment = self.comment.as_deref().unwrap_or_default();
        if comment.len() > u16::MAX as usize {
            return Err(ZipError::FieldTooLong {
                field: "CentralDirEntry comment",
                len: comment.len(),
                max: u16::MAX as usize,
            });
        }

        let mut buf = Vec::with_capacity(46 + self.name.len() + extra.len() + comment.len());
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
        put_u16(&mut buf, comment.len() as u16);
        put_u16(&mut buf, 0);
        put_u16(&mut buf, 0);
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
        buf.extend_from_slice(comment);
        Ok(buf)
    }

    fn zip64_extra(compressed_size: u64, uncompressed_size: u64, offset: u64) -> Vec<u8> {
        let mut data = Vec::with_capacity(28);
        put_u16(&mut data, 0x0001);
        put_u16(&mut data, 24);
        put_u64(&mut data, uncompressed_size);
        put_u64(&mut data, compressed_size);
        put_u64(&mut data, offset);
        data
    }
}

pub(crate) struct Eocdr {
    pub(crate) total_entries: u64,
    pub(crate) cd_size: u64,
    pub(crate) cd_offset: u64,
    pub(crate) comment: Option<Vec<u8>>,
}

impl Eocdr {
    pub(crate) fn serialize(&self) -> Vec<u8> {
        let use_zip64 =
            self.total_entries > 0xFFFF || self.cd_size > U32_MAX || self.cd_offset > U32_MAX;

        let comment = self.comment.as_deref().unwrap_or_default();

        let mut buf = Vec::with_capacity(22 + comment.len());
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
        put_u16(&mut buf, comment.len() as u16);
        buf.extend_from_slice(comment);
        buf
    }
}

pub(crate) struct Zip64Eocdr {
    pub(crate) total_entries: u64,
    pub(crate) cd_size: u64,
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

pub(crate) struct Zip64EocdrLocator {
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
