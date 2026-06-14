use crate::header;

pub(crate) struct StoredEntry {
    pub(crate) name: String,
    pub(crate) crc32: u32,
    pub(crate) compressed_size: u64,
    pub(crate) uncompressed_size: u64,
    pub(crate) local_header_offset: u64,
    pub(crate) is_directory: bool,
    pub(crate) is_symlink: bool,
    pub(crate) is_stored: bool,
    pub(crate) mtime: (u16, u16),
    pub(crate) unix_mtime: u64,
    pub(crate) unix_permissions: Option<u32>,
    pub(crate) uid_gid: Option<(u32, u32)>,
    pub(crate) comment: Option<Vec<u8>>,
}

impl StoredEntry {
    pub(crate) fn to_central_dir_entry(&self) -> header::CentralDirEntry {
        let (time, date) = self.mtime;

        let has_unix_attrs =
            self.unix_permissions.is_some() || self.is_symlink || self.unix_mtime != 0;
        let version_made_by = if has_unix_attrs {
            header::VERSION_UNIX
        } else {
            header::VERSION_DEFLATE
        };

        let mut extra = if !self.name.is_ascii() {
            header::build_unicode_extra_field(&self.name)
        } else {
            Vec::new()
        };
        extra.extend(header::build_extended_timestamp_extra(self.unix_mtime));
        if let Some((uid, gid)) = self.uid_gid {
            extra.extend(header::build_unix_uid_gid_extra(uid, gid));
        }

        let file_type_bit: u32 = if self.is_symlink {
            0o120000 // S_IFLNK
        } else if self.is_directory {
            0o040000 // S_IFDIR
        } else {
            0o100000 // S_IFREG
        };
        let external_file_attributes = match (self.unix_permissions, self.is_symlink) {
            (Some(mode), _) => (mode | file_type_bit) << 16,
            (None, true) => file_type_bit << 16,
            (None, false) if has_unix_attrs => {
                let default_mode = if self.is_directory { 0o755 } else { 0o644 };
                (default_mode | file_type_bit) << 16
            }
            (None, false) => 0,
        };

        let use_zip64 = self.compressed_size > header::U32_MAX
            || self.uncompressed_size > header::U32_MAX
            || self.local_header_offset > header::U32_MAX;

        let mut flags = header::FLAG_DATA_DESC;
        if !self.name.is_ascii() {
            flags |= 1 << 11; // EFS / UTF-8 flag (bit 11), consistent with LocalFileHeader::new()
        }

        header::CentralDirEntry {
            version_made_by,
            version_needed: if use_zip64 {
                header::VERSION_ZIP64
            } else if self.is_directory || self.is_symlink || self.is_stored {
                header::VERSION_STORED
            } else {
                header::VERSION_DEFLATE
            },
            flags,
            method: if self.is_directory || self.is_symlink || self.is_stored {
                header::METHOD_STORED
            } else {
                header::METHOD_DEFLATE
            },
            time,
            date,
            crc32: self.crc32,
            compressed_size: self.compressed_size,
            uncompressed_size: self.uncompressed_size,
            name: self.name.as_bytes().to_vec(),
            extra,
            local_header_offset: self.local_header_offset,
            external_file_attributes,
            comment: self.comment.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header;

    #[test]
    fn test_to_central_dir_entry_basic() {
        let entry = StoredEntry {
            name: "file.txt".to_string(),
            crc32: 0x12345678,
            compressed_size: 100,
            uncompressed_size: 100,
            local_header_offset: 0,
            is_directory: false,
            is_symlink: false,
            is_stored: true,
            mtime: (0, 0),
            unix_mtime: 0,
            unix_permissions: None,
            uid_gid: None,
            comment: None,
        };

        let cd = entry.to_central_dir_entry();

        assert_eq!(cd.name, b"file.txt");
        assert_eq!(cd.crc32, 0x12345678);
        assert_eq!(cd.method, header::METHOD_STORED);
        assert_eq!(cd.version_needed, header::VERSION_STORED);
        assert_eq!(cd.version_made_by, header::VERSION_DEFLATE);
        assert_eq!(
            cd.extra.len(),
            9,
            "expected 9-byte extended timestamp extra, got {}",
            cd.extra.len()
        );
        assert!(
            cd.extra.windows(2).any(|w| w == b"UT"),
            "extra should contain UT (0x5455) tag"
        );
    }

    #[test]
    fn test_to_central_dir_entry_directory() {
        let entry = StoredEntry {
            name: "mydir/".to_string(),
            crc32: 0,
            compressed_size: 0,
            uncompressed_size: 0,
            local_header_offset: 0,
            is_directory: true,
            is_symlink: false,
            is_stored: false,
            mtime: (0, 0),
            unix_mtime: 0,
            unix_permissions: Some(0o755),
            uid_gid: Some((1000, 1000)),
            comment: None,
        };

        let cd = entry.to_central_dir_entry();

        assert_eq!(cd.method, header::METHOD_STORED);
        assert_eq!(cd.version_needed, header::VERSION_STORED);
        assert_eq!(cd.version_made_by, header::VERSION_UNIX);
        let attrs = cd.external_file_attributes;
        assert_eq!(
            attrs >> 16,
            0o040000 | 0o755,
            "expected S_IFDIR + 0o755, got {:06o}",
            attrs >> 16
        );
        assert!(
            cd.extra.windows(2).any(|w| w == [0x75, 0x78]),
            "CD extra should contain 0x7875 (Ux) tag for Unix entries"
        );
    }

    #[test]
    fn test_to_central_dir_entry_symlink() {
        let entry = StoredEntry {
            name: "link".to_string(),
            crc32: 0,
            compressed_size: 8,
            uncompressed_size: 8,
            local_header_offset: 0,
            is_directory: false,
            is_symlink: true,
            is_stored: false,
            mtime: (0, 0),
            unix_mtime: 0,
            unix_permissions: None,
            uid_gid: Some((1000, 1000)),
            comment: None,
        };

        let cd = entry.to_central_dir_entry();

        assert_eq!(cd.method, header::METHOD_STORED);
        assert_eq!(cd.version_needed, header::VERSION_STORED);
        assert_eq!(cd.version_made_by, header::VERSION_UNIX);
        let attrs = cd.external_file_attributes;
        assert_eq!(
            attrs >> 16,
            0o120000,
            "expected S_IFLNK, got {:06o}",
            attrs >> 16
        );
        assert!(
            cd.extra.windows(2).any(|w| w == [0x75, 0x78]),
            "CD extra should contain 0x7875 (Ux) tag for symlink entries"
        );
    }

    #[test]
    fn test_to_central_dir_entry_with_metadata() {
        let entry = StoredEntry {
            name: "meta.txt".to_string(),
            crc32: 0xDEADBEEF,
            compressed_size: 50,
            uncompressed_size: 50,
            local_header_offset: 42,
            is_directory: false,
            is_symlink: false,
            is_stored: true,
            mtime: (0x4A5B, 0x14AF),
            unix_mtime: 1234567890,
            unix_permissions: Some(0o644),
            uid_gid: Some((1000, 1000)),
            comment: None,
        };

        let cd = entry.to_central_dir_entry();

        assert_eq!(cd.version_made_by, header::VERSION_UNIX);
        assert_eq!(cd.method, header::METHOD_STORED);
        assert_eq!(cd.crc32, 0xDEADBEEF);
        assert_eq!(cd.compressed_size, 50);
        assert_eq!(cd.uncompressed_size, 50);
        assert!(!cd.extra.is_empty(), "expected extended timestamp extra");
        assert!(
            cd.extra.windows(2).any(|w| w == b"UT"),
            "extra should contain UT (0x5455) tag"
        );
        assert!(
            cd.extra.windows(2).any(|w| w == [0x75, 0x78]),
            "extra should contain Ux (0x7875) tag"
        );
        let attrs = cd.external_file_attributes;
        assert_eq!(
            attrs >> 16,
            0o100000 | 0o644,
            "expected S_IFREG + 0o644, got {:06o}",
            attrs >> 16
        );
    }

    #[test]
    fn test_to_central_dir_entry_zip64() {
        // When compressed/uncompressed/local_header exceed U32_MAX, version_needed
        // should be VERSION_ZIP64.
        let entry = StoredEntry {
            name: "big.bin".to_string(),
            crc32: 0,
            compressed_size: header::U32_MAX + 1,
            uncompressed_size: header::U32_MAX + 1,
            local_header_offset: header::U32_MAX + 1,
            is_directory: false,
            is_symlink: false,
            is_stored: false,
            mtime: (0, 0),
            unix_mtime: 0,
            unix_permissions: None,
            uid_gid: None,
            comment: None,
        };

        let cd = entry.to_central_dir_entry();

        assert_eq!(
            cd.version_needed,
            header::VERSION_ZIP64,
            "version_needed should be VERSION_ZIP64 (20) for entries needing ZIP64"
        );
        assert_eq!(cd.method, header::METHOD_DEFLATE);
    }
}
