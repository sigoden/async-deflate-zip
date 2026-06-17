use crate::zip_format;
use crate::zip_format::ExtraField;

pub(crate) struct StoredEntry {
    pub(crate) name: String,
    pub(crate) crc32: u32,
    pub(crate) compressed_size: u64,
    pub(crate) uncompressed_size: u64,
    pub(crate) local_header_offset: u64,
    pub(crate) is_directory: bool,
    pub(crate) is_symlink: bool,
    pub(crate) is_stored: bool,
    pub(crate) unix_mtime: u64,
    pub(crate) unix_permissions: Option<u32>,
    pub(crate) uid_gid: Option<(u32, u32)>,
    pub(crate) comment: Option<Vec<u8>>,
}

impl StoredEntry {
    pub(crate) fn to_central_dir_entry(&self) -> zip_format::CentralDirEntry {
        let (time, date) = zip_format::unix_secs_to_ms_dos(self.unix_mtime);

        let version_made_by = zip_format::VERSION_UNIX;

        let mut extra = Vec::new();
        if self.unix_mtime != 0 {
            zip_format::ExtendedTimestampExtra::new(self.unix_mtime).serialize(&mut extra);
        }
        if let Some((uid, gid)) = self.uid_gid {
            zip_format::UnixUidGidExtra::new(uid, gid).serialize(&mut extra);
        }

        let use_zip64 = zip_format::entry_needs_zip64(
            self.compressed_size,
            self.uncompressed_size,
            self.local_header_offset,
        );
        if use_zip64 {
            zip_format::Zip64Extra {
                uncompressed_size: self.uncompressed_size,
                compressed_size: self.compressed_size,
                offset: self.local_header_offset,
            }
            .serialize(&mut extra);
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
            (None, true) => (zip_format::DEFAULT_SYMLINK_PERM | file_type_bit) << 16,
            (None, false) => {
                let default_mode = if self.is_directory {
                    zip_format::DEFAULT_DIR_PERM
                } else {
                    zip_format::DEFAULT_FILE_PERM
                };
                (default_mode | file_type_bit) << 16
            }
        };

        let mut flags = if self.is_directory {
            0
        } else {
            zip_format::FLAG_DATA_DESC
        };
        if !self.name.is_ascii() {
            flags |= 1 << 11;
        }

        zip_format::CentralDirEntry {
            version_made_by,
            version_needed: if use_zip64 {
                zip_format::VERSION_ZIP64
            } else if self.is_directory || self.is_symlink || self.is_stored {
                zip_format::VERSION_STORED
            } else {
                zip_format::VERSION_DEFLATE
            },
            flags,
            method: if self.is_directory || self.is_symlink || self.is_stored {
                zip_format::METHOD_STORED
            } else {
                zip_format::METHOD_DEFLATE
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
    use crate::zip_format;

    fn default_entry() -> StoredEntry {
        StoredEntry {
            name: String::new(),
            crc32: 0,
            compressed_size: 0,
            uncompressed_size: 0,
            local_header_offset: 0,
            is_directory: false,
            is_symlink: false,
            is_stored: false,
            unix_mtime: 0,
            unix_permissions: None,
            uid_gid: None,
            comment: None,
        }
    }

    #[test]
    fn test_to_central_dir_entry_basic() {
        let entry = StoredEntry {
            name: "file.txt".to_string(),
            crc32: 0x12345678,
            compressed_size: 100,
            uncompressed_size: 100,
            is_stored: true,
            ..default_entry()
        };

        let cd = entry.to_central_dir_entry();

        assert_eq!(cd.name, b"file.txt");
        assert_eq!(cd.crc32, 0x12345678);
        assert_eq!(cd.method, zip_format::METHOD_STORED);
        assert_eq!(cd.version_needed, zip_format::VERSION_STORED);
        assert_eq!(cd.version_made_by, zip_format::VERSION_UNIX);
        assert!(
            cd.extra.is_empty(),
            "expected no extra fields when unix_mtime=0 and no uid_gid"
        );
    }

    #[test]
    fn test_to_central_dir_entry_directory() {
        let entry = StoredEntry {
            name: "mydir/".to_string(),
            is_directory: true,
            unix_permissions: Some(0o755),
            uid_gid: Some((1000, 1000)),
            ..default_entry()
        };

        let cd = entry.to_central_dir_entry();

        assert_eq!(cd.method, zip_format::METHOD_STORED);
        assert_eq!(cd.version_needed, zip_format::VERSION_STORED);
        assert_eq!(cd.version_made_by, zip_format::VERSION_UNIX);
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
            compressed_size: 8,
            uncompressed_size: 8,
            is_symlink: true,
            uid_gid: Some((1000, 1000)),
            ..default_entry()
        };

        let cd = entry.to_central_dir_entry();

        assert_eq!(cd.method, zip_format::METHOD_STORED);
        assert_eq!(cd.version_needed, zip_format::VERSION_STORED);
        assert_eq!(cd.version_made_by, zip_format::VERSION_UNIX);
        let attrs = cd.external_file_attributes;
        assert_eq!(
            attrs >> 16,
            0o120777,
            "expected S_IFLNK + 0o777, got {:06o}",
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
            is_stored: true,
            unix_mtime: 1234567890,
            unix_permissions: Some(0o644),
            uid_gid: Some((1000, 1000)),
            ..default_entry()
        };

        let cd = entry.to_central_dir_entry();

        assert_eq!(cd.version_made_by, zip_format::VERSION_UNIX);
        assert_eq!(cd.method, zip_format::METHOD_STORED);
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
        let entry = StoredEntry {
            name: "big.bin".to_string(),
            compressed_size: zip_format::U32_MAX + 1,
            uncompressed_size: zip_format::U32_MAX + 1,
            local_header_offset: zip_format::U32_MAX + 1,
            ..default_entry()
        };

        let cd = entry.to_central_dir_entry();

        assert_eq!(
            cd.version_needed,
            zip_format::VERSION_ZIP64,
            "version_needed should be VERSION_ZIP64 (45) for entries needing ZIP64"
        );
        assert_eq!(cd.method, zip_format::METHOD_DEFLATE);
    }
}
