use std::path::Path;
use std::time::SystemTime;

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, PermissionsExt};

use tokio::fs;

/// Per-entry options for appending to a ZIP archive.
///
/// Passed to [`ZipWriter::start_file`](crate::writer::ZipWriter::start_file),
/// [`ZipWriter::add_directory`](crate::writer::ZipWriter::add_directory),
/// and [`ZipWriter::add_symlink`](crate::writer::ZipWriter::add_symlink)
/// to control metadata such as
/// modification time, Unix permissions, UID/GID, and file comment.
///
/// Use the convenience constructors for common cases:
/// - [`file`](EntryOptions::file) — 0644 + current time
/// - [`directory`](EntryOptions::directory) — 0755 + current time
/// - [`symlink`](EntryOptions::symlink) — 0777 + current time
/// - [`from_path`](EntryOptions::from_path) — read metadata from a real file
///
/// For full control, use the builder methods on a convenience constructor:
///
/// ```rust,no_run
/// use async_deflate_zip::EntryOptions;
/// use std::time::SystemTime;
///
/// let opts = EntryOptions::default()
///     .with_unix_permissions(0o644)
///     .with_comment("my file");
/// ```
pub struct EntryOptions {
    mtime: SystemTime,
    permissions: Option<u32>,
    uid_gid: Option<(u32, u32)>,
    comment: Option<String>,
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
            permissions: Some(0o644),
            ..Default::default()
        }
    }

    /// Convenience options for a directory: permissions `0o755`,
    /// modification time set to now.
    pub fn directory() -> Self {
        Self {
            permissions: Some(0o755),
            ..Default::default()
        }
    }

    /// Convenience options for a symbolic link: permissions `0o777`,
    /// modification time set to now.
    pub fn symlink() -> Self {
        Self {
            permissions: Some(0o777),
            ..Default::default()
        }
    }

    // --- Builder methods -------------------------------------------------

    /// Set the modification time.
    pub fn with_mtime(mut self, mtime: SystemTime) -> Self {
        self.mtime = mtime;
        self
    }

    /// Set the Unix permission bits.
    ///
    /// The file type bit (`S_IFREG`/`S_IFDIR`/`S_IFLNK`) is added
    /// automatically by the writer.
    pub fn with_unix_permissions(mut self, permissions: u32) -> Self {
        self.permissions = Some(permissions);
        self
    }

    /// Set the Unix user and group IDs.
    pub fn with_uid_gid(mut self, uid: u32, gid: u32) -> Self {
        self.uid_gid = Some((uid, gid));
        self
    }

    /// Set the per-entry comment.
    ///
    /// Maximum length is 65535 bytes when encoded as UTF-8. Comments
    /// longer than this will cause [`crate::writer::ZipWriter::finish`] to return
    /// [`crate::error::ZipError::FieldTooLong`].
    pub fn with_comment(mut self, comment: &str) -> Self {
        self.comment = Some(comment.to_string());
        self
    }

    // --- Getters ----------------------------------------------------------

    /// Last modification time.
    pub fn mtime(&self) -> &SystemTime {
        &self.mtime
    }

    /// Unix permission bits, if set.
    pub fn unix_permissions(&self) -> Option<u32> {
        self.permissions
    }

    /// Unix user and group IDs, if set.
    pub fn uid_gid(&self) -> Option<(u32, u32)> {
        self.uid_gid
    }

    /// Per-entry comment, if set.
    pub fn comment(&self) -> Option<&str> {
        self.comment.as_deref()
    }
}
