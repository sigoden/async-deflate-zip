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

#[cfg(test)]
mod tests {
    use crate::zip_format::binary::*;
    use crate::zip_format::*;

    #[test]
    fn test_cd_normal_case() {
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
            comment: None,
        };
        let data = cde.serialize().unwrap();
        assert_eq!(read_u32(&data, 0), CD_SIG);
        assert_eq!(read_u32(&data, 16), 0xDEADBEEF);
        assert_eq!(read_u32(&data, 20), 500);
        assert_eq!(read_u32(&data, 24), 1000);
    }

    #[test]
    fn test_cd_zip64_case() {
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
            comment: None,
        };
        let data = cde.serialize().unwrap();
        assert_eq!(read_u32(&data, 0), CD_SIG);
        assert_eq!(read_u32(&data, 20), u32::MAX);
        assert_eq!(read_u32(&data, 24), u32::MAX);
        assert_eq!(read_u16(&data, 6), VERSION_ZIP64);
        let name_len = read_u16(&data, 28) as usize;
        let extra_len = read_u16(&data, 30) as usize;
        assert_eq!(extra_len, 28);
        let extra_start = 46 + name_len;
        let extra = &data[extra_start..extra_start + extra_len];
        assert_eq!(read_u16(extra, 0), 0x0001);
        assert_eq!(read_u16(extra, 2), 24);
        assert_eq!(read_u64(extra, 4), 10_000_000_000);
        assert_eq!(read_u64(extra, 12), 5_000_000_000);
        assert_eq!(read_u64(extra, 20), 0);
    }

    #[test]
    fn test_cd_zip64_with_extra_field() {
        let mut timestamp_extra = Vec::new();
        ExtendedTimestampExtra::new(1700000000).serialize(&mut timestamp_extra);
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
            comment: None,
        };
        let data = cde.serialize().unwrap();

        assert_eq!(read_u32(&data, 20), u32::MAX);
        assert_eq!(read_u32(&data, 24), u32::MAX);

        let name_len = read_u16(&data, 28) as usize;
        let extra_len = read_u16(&data, 30) as usize;
        assert_eq!(extra_len, 37);

        let extra_start = 46 + name_len;
        let extra_data = &data[extra_start..extra_start + extra_len];
        assert_eq!(&extra_data[0..2], b"UT");
        assert_eq!(read_u16(extra_data, 9), 0x0001);
        assert_eq!(read_u16(extra_data, 11), 24);
    }

    #[test]
    fn test_cd_field_too_long() {
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
            comment: None,
        };
        let result = cde.serialize();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("filename too long")
        );
    }
}
