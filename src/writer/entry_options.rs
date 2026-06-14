use std::path::Path;
use std::time::SystemTime;

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, PermissionsExt};

use tokio::fs;

/// Per-entry options for appending to a ZIP archive.
///
/// Passed to [`ZipWriter::append_file`](crate::writer::ZipWriter::append_file),
/// [`ZipWriter::append_directory`](crate::writer::ZipWriter::append_directory),
/// and [`ZipWriter::append_symlink`](crate::writer::ZipWriter::append_symlink)
/// to control metadata such as
/// modification time, Unix permissions, UID/GID, and file comment.
///
/// Use the convenience constructors for common cases:
/// - [`file`](EntryOptions::file) — 0644 + current time
/// - [`directory`](EntryOptions::directory) — 0755 + current time
/// - [`symlink`](EntryOptions::symlink) — 0777 + current time
/// - [`from_path`](EntryOptions::from_path) — read metadata from a real file
///
/// For full control, construct the struct directly:
///
/// ```rust,no_run
/// use async_deflate_zip::EntryOptions;
/// use std::time::SystemTime;
///
/// let opts = EntryOptions {
///     mtime: SystemTime::UNIX_EPOCH,
///     permissions: Some(0o755),
///     uid_gid: Some((1000, 1000)),
///     comment: Some("my file".to_string()),
/// };
/// ```
pub struct EntryOptions {
    /// Last modification time. Stored in MS-DOS format in the fixed CD fields
    /// and as a Unix timestamp in the extended timestamp extra field (0x5455).
    pub mtime: SystemTime,
    /// Unix permission bits (e.g. `0o644`, `0o755`). The file type bit
    /// (`S_IFREG`/`S_IFDIR`/`S_IFLNK`) is added automatically by the writer.
    /// When `None`, no Unix permissions are written.
    pub permissions: Option<u32>,
    /// Unix user and group IDs. Stored in the 0x7875 (Ux) extra field.
    pub uid_gid: Option<(u32, u32)>,
    /// Per-entry comment stored in the Central Directory.
    /// Maximum length is 65535 bytes. Comments longer than this will
    /// cause `finalize` to return [`ZipError::FieldTooLong`](crate::error::ZipError::FieldTooLong).
    pub comment: Option<String>,
}

impl Default for EntryOptions {
    fn default() -> Self {
        Self {
            mtime: SystemTime::now(),
            permissions: None,
            uid_gid: None,
            comment: None,
        }
    }
}

impl EntryOptions {
    /// Read metadata (mtime, permissions, uid/gid) from a real filesystem path.
    ///
    /// The file's content is **not** read — only `symlink_metadata` is queried
    /// so this works on symlinks, directories, and regular files alike.
    ///
    /// On Unix, permission bits are extracted from the file's mode and
    /// UID/GID from the file's owner. On non-Unix platforms, permissions
    /// default to `None` and UID/GID to `None`.
    pub async fn from_path<T: AsRef<Path>>(path: T) -> std::io::Result<Self> {
        let meta = fs::symlink_metadata(&path).await?;
        Ok(Self {
            mtime: meta.modified().unwrap_or_else(|_| SystemTime::now()),
            #[cfg(unix)]
            permissions: Some(meta.permissions().mode() & 0o7777),
            #[cfg(not(unix))]
            permissions: None,
            uid_gid: {
                #[cfg(unix)]
                {
                    Some((meta.uid(), meta.gid()))
                }
                #[cfg(not(unix))]
                {
                    None
                }
            },
            comment: None,
        })
    }

    /// Convenience options for a regular file: permissions `0o644`,
    /// modification time set to now.
    pub fn file() -> Self {
        Self {
            mtime: SystemTime::now(),
            permissions: Some(0o644),
            ..Default::default()
        }
    }

    /// Convenience options for a directory: permissions `0o755`,
    /// modification time set to now.
    pub fn directory() -> Self {
        Self {
            mtime: SystemTime::now(),
            permissions: Some(0o755),
            ..Default::default()
        }
    }

    /// Convenience options for a symbolic link: permissions `0o777`,
    /// modification time set to now.
    pub fn symlink() -> Self {
        Self {
            mtime: SystemTime::now(),
            permissions: Some(0o777),
            ..Default::default()
        }
    }
}
