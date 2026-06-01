use crate::error::ZipError;
use crate::header;

use tokio::io::{AsyncWrite, AsyncWriteExt};

use super::stored_entry::StoredEntry;
use super::zip_writer::ZipWriter;

/// A handle for finalizing a directory entry in a ZIP archive.
///
/// Obtained from [`ZipWriter::append_directory`]. Use [`set_mtime`](Self::set_mtime)
/// and/or [`set_permissions`](Self::set_permissions) to attach metadata, then call
/// [`close`](Self::close) to finalize the entry.
///
/// Dropping without calling `close` leaves the archive in an inconsistent state
/// and poisons the parent [`ZipWriter`].
pub struct DirectoryWriter<'a, W: AsyncWrite + Unpin> {
    pub(crate) zip: &'a mut ZipWriter<W>,
    pub(crate) writer: Option<W>,
    pub(crate) name: String,
    pub(crate) local_header_offset: u64,
    pub(crate) mtime: Option<std::time::SystemTime>,
    pub(crate) unix_permissions: Option<u32>,
}

impl<W: AsyncWrite + Unpin> DirectoryWriter<'_, W> {
    /// Set the modification time for this directory entry.
    pub fn set_mtime(&mut self, mtime: std::time::SystemTime) -> &mut Self {
        self.mtime = Some(mtime);
        self
    }

    /// Set Unix file permissions for this directory entry.
    ///
    /// Provide permission bits including setuid/setgid/sticky (e.g., `0o755`).
    /// The crate automatically adds the `S_IFDIR` file type bit.
    pub fn set_permissions(&mut self, mode: u32) -> &mut Self {
        self.unix_permissions = Some(mode & 0o7777);
        self
    }

    /// Finalize the directory entry by writing the Data Descriptor.
    ///
    /// This consumes the `DirectoryWriter`, writes the trailing Data Descriptor
    /// (CRC-32 and zero sizes), and returns the inner writer to the parent
    /// [`ZipWriter`].
    ///
    /// # Errors
    ///
    /// Returns [`ZipError`] if `close` is called more than once.
    pub async fn close(mut self) -> Result<(), ZipError> {
        let mut inner = self
            .writer
            .take()
            .ok_or_else(|| ZipError::InvalidState("directory entry already closed".to_string()))?;

        let dd = header::DataDescriptor {
            crc32: 0,
            compressed_size: 0,
            uncompressed_size: 0,
            zip64: self.local_header_offset > header::U32_MAX,
        };
        let dd_bytes = dd.serialize();
        inner.write_all(&dd_bytes).await.map_err(|e| {
            self.zip.poisoned = true;
            ZipError::Io(e)
        })?;
        self.zip.pos += dd_bytes.len() as u64;

        let (mtime_msdos, unix_mtime) = header::mtime_to_ms_dos_and_unix(self.mtime);

        self.zip.entries.push(StoredEntry {
            name: self.name.clone(),
            crc32: 0,
            compressed_size: 0,
            uncompressed_size: 0,
            local_header_offset: self.local_header_offset,
            is_directory: true,
            is_symlink: false,
            is_stored: false,
            mtime: mtime_msdos,
            unix_mtime,
            unix_permissions: self.unix_permissions,
        });

        self.zip.inner = Some(inner);
        Ok(())
    }
}

impl<W: AsyncWrite + Unpin> Drop for DirectoryWriter<'_, W> {
    fn drop(&mut self) {
        if self.writer.is_some() {
            // close() was never called — mark the ZipWriter as poisoned
            self.zip.poisoned = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn test_directory_entry() {
        let mut buf = Vec::new();
        let mut zip = ZipWriter::new(&mut buf);
        zip.append_directory("emptydir/")
            .await
            .unwrap()
            .close()
            .await
            .unwrap();

        let mut entry = zip.append_file("emptydir/hello.txt").await.unwrap();
        entry.write_all(b"nested").await.unwrap();
        entry.close().await.unwrap();

        zip.finalize().await.unwrap();

        assert!(buf.windows(4).any(|w| w == b"PK\x03\x04"));
    }

    #[tokio::test]
    async fn test_directory_metadata() {
        let mut buf = Vec::new();
        let mut zip = ZipWriter::new(&mut buf);

        {
            let mut dir = zip.append_directory("meta_dir/").await.unwrap();
            dir.set_mtime(std::time::SystemTime::UNIX_EPOCH);
            dir.set_permissions(0o755);
            dir.close().await.unwrap();
        }

        zip.finalize().await.unwrap();

        let pos = buf.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
        let cd = &buf[pos..];

        let vmb = u16::from_le_bytes(cd[4..6].try_into().unwrap());
        assert_eq!(vmb >> 8, 3, "expected Unix host OS when metadata is set");

        let version_needed = u16::from_le_bytes(cd[6..8].try_into().unwrap());
        assert_eq!(version_needed, 10, "expected VERSION_STORED for directory");

        let method = u16::from_le_bytes(cd[10..12].try_into().unwrap());
        assert_eq!(method, 0, "expected METHOD_STORED for directory");

        let efa = u32::from_le_bytes(cd[38..42].try_into().unwrap());
        assert_eq!(efa, ((0o755 | 0o040000) as u32) << 16);

        let name_len = u16::from_le_bytes(cd[28..30].try_into().unwrap()) as usize;
        let extra_len = u16::from_le_bytes(cd[30..32].try_into().unwrap()) as usize;
        assert!(
            extra_len >= 4,
            "expected non-empty extra field when mtime is set, got {extra_len}"
        );
        let extra_start = 46 + name_len;
        let extra = &cd[extra_start..extra_start + extra_len];
        assert!(
            extra.windows(2).any(|w| w == b"UT"),
            "expected extended timestamp extra (0x5455/UT) in directory entry"
        );
    }

    #[tokio::test]
    async fn test_directory_version_needed() {
        let mut buf = Vec::new();
        let mut zip = ZipWriter::new(&mut buf);
        zip.append_directory("mydir/")
            .await
            .unwrap()
            .close()
            .await
            .unwrap();
        zip.finalize().await.unwrap();

        let pos = buf.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
        let cd = &buf[pos..];
        let version_needed = u16::from_le_bytes(cd[6..8].try_into().unwrap());
        assert_eq!(
            version_needed, 10,
            "directory CD version_needed should be 10 (STORED), got {version_needed}"
        );
    }
}
