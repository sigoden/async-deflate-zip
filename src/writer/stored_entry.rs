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
    pub(crate) mtime: Option<(u16, u16)>,
    pub(crate) unix_mtime: Option<u64>,
    pub(crate) unix_permissions: Option<u32>,
}

impl StoredEntry {
    pub(crate) fn to_central_dir_entry(&self) -> header::CentralDirEntry {
        let (time, date) = self.mtime.unwrap_or_else(header::ms_dos_datetime);

        let has_unix_attrs =
            self.unix_permissions.is_some() || self.unix_mtime.is_some() || self.is_symlink;
        let version_made_by = if has_unix_attrs {
            header::VERSION_UNIX
        } else {
            header::VERSION_DEFLATE
        };

        let extra = match self.unix_mtime {
            Some(ts) => header::build_extended_timestamp_extra(ts),
            None => Vec::new(),
        };

        let file_type_bit: u32 = if self.is_symlink {
            0o120000 // S_IFLNK
        } else if self.is_directory {
            0o040000 // S_IFDIR
        } else {
            0o100000 // S_IFREG
        };
        let external_file_attributes = match (self.unix_permissions, self.is_symlink) {
            (Some(mode), _) => (mode | file_type_bit) << 16,
            (None, true) => file_type_bit << 16, // Symlinks always need type bit
            (None, false) => 0,
        };

        let use_zip64 = self.compressed_size > header::U32_MAX
            || self.uncompressed_size > header::U32_MAX
            || self.local_header_offset > header::U32_MAX;

        header::CentralDirEntry {
            version_made_by,
            version_needed: if use_zip64 {
                header::VERSION_ZIP64
            } else if self.is_directory || self.is_symlink || self.is_stored {
                header::VERSION_STORED
            } else {
                header::VERSION_DEFLATE
            },
            flags: header::FLAG_DATA_DESC,
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
            mtime: None,
            unix_mtime: None,
            unix_permissions: None,
        };

        let cd = entry.to_central_dir_entry();

        assert_eq!(cd.name, b"file.txt");
        assert_eq!(cd.crc32, 0x12345678);
        assert_eq!(cd.method, header::METHOD_STORED);
        assert_eq!(cd.version_needed, header::VERSION_STORED);
        assert_eq!(cd.version_made_by, header::VERSION_DEFLATE);
        assert!(cd.extra.is_empty(), "no extra field without metadata");
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
            mtime: None,
            unix_mtime: None,
            unix_permissions: Some(0o755),
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
            mtime: None,
            unix_mtime: None,
            unix_permissions: None,
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
            mtime: Some((0x4A5B, 0x14AF)),
            unix_mtime: Some(1234567890),
            unix_permissions: Some(0o644),
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
            mtime: None,
            unix_mtime: None,
            unix_permissions: None,
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
