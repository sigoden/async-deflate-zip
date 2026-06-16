use crate::error::ZipError;
use crate::zip_format::binary::*;
use crate::zip_format::constants::*;
use crate::zip_format::extra_fields::*;

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
            Zip64Extra::CentralDirectory {
                uncompressed_size: self.uncompressed_size,
                compressed_size: self.compressed_size,
                offset: self.local_header_offset,
            }
            .serialize(&mut extra);
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
}
