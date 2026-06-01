use crate::error::ZipError;
use crate::header;
use crate::types::CompressionLevel;

use async_compression::tokio::write::DeflateEncoder;
use tokio::io::{AsyncWrite, AsyncWriteExt};

use super::directory_writer::DirectoryEntryWriter;
use super::entry_writer::EntryWriter;
use super::helpers::{CountWriter, ShutdownIgnoredWriter};
use super::stored_entry::StoredEntry;

/// A streaming ZIP archive writer with per-file deflate compression.
///
/// Entries are written sequentially — each file produces its own deflate
/// frame with a data descriptor (CRC-32 and sizes) after each entry. The output is a
/// standard ZIP archive compatible with common unzip tools, including
/// Windows Explorer.
///
/// # Example
///
/// ```rust,no_run
/// use async_deflate_zip::ZipWriter;
/// use tokio::io::AsyncWriteExt;
///
/// # async fn example() {
/// let mut buf = Vec::new();
/// let mut zip = ZipWriter::new(&mut buf);
///
/// let mut entry = zip.append_file("hello.txt").await.unwrap();
/// entry.write_all(b"Hello, World!").await.unwrap();
/// entry.close().await.unwrap();
///
/// zip.finalize().await.unwrap();
/// # }
/// ```
pub struct ZipWriter<W: AsyncWrite + Unpin> {
    pub(crate) inner: Option<W>,
    pub(crate) entries: Vec<StoredEntry>,
    level: u8,
    pub(crate) pos: u64,
    pub(crate) poisoned: bool,
}

impl<W: AsyncWrite + Unpin> ZipWriter<W> {
    /// Create a new `ZipWriter` wrapping an async writer.
    ///
    /// Uses the default compression level ([`CompressionLevel::DEFAULT`], level 6).
    /// Use [`with_compression_level`](Self::with_compression_level) to customize.
    pub fn new(inner: W) -> Self {
        Self {
            inner: Some(inner),
            entries: Vec::new(),
            level: CompressionLevel::DEFAULT.level(),
            pos: 0,
            poisoned: false,
        }
    }

    /// Set the compression level for entries added to this archive.
    ///
    /// Must be called before adding any entries. Returns `self` for chaining.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use async_deflate_zip::{ZipWriter, CompressionLevel};
    ///
    /// let mut buf = Vec::new();
    /// let zip = ZipWriter::new(&mut buf)
    ///     .with_compression_level(CompressionLevel::BEST);
    /// ```
    pub fn with_compression_level(mut self, level: CompressionLevel) -> Self {
        self.level = level.level();
        self
    }

    /// Start a new file entry and return an [`EntryWriter`] for streaming data.
    ///
    /// Writes the Local File Header, then returns an `EntryWriter` that
    /// compresses and buffers written data. Call [`EntryWriter::close`]
    /// to finalize the entry and write the trailing CRC-32 and sizes.
    ///
    /// # Errors
    ///
    /// Returns [`ZipError`] if writer is poisoned, or if writing the
    /// Local File Header fails (I/O error or field too long).
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use async_deflate_zip::ZipWriter;
    /// use tokio::io::AsyncWriteExt;
    ///
    /// # async fn example() {
    /// let mut buf = Vec::new();
    /// let mut zip = ZipWriter::new(&mut buf);
    /// let mut entry = zip.append_file("readme.txt").await.unwrap();
    /// entry.write_all(b"content").await.unwrap();
    /// entry.close().await.unwrap();
    /// zip.finalize().await.unwrap();
    /// # }
    /// ```
    pub async fn append_file<'a>(&'a mut self, name: &str) -> Result<EntryWriter<'a, W>, ZipError> {
        let mut inner = self.inner.take().ok_or_else(|| {
            if self.poisoned {
                ZipError::Poisoned("previous entry was dropped without calling close()".to_string())
            } else {
                ZipError::InvalidState("entry writer already active".to_string())
            }
        })?;

        let is_stored = self.level == 0;
        let method = if is_stored {
            header::METHOD_STORED
        } else {
            header::METHOD_DEFLATE
        };

        let needs_zip64 = self.pos > header::U32_MAX;
        let lfh = header::LocalFileHeader::new(name, method, needs_zip64);
        let lfh_bytes = lfh.serialize()?;
        inner.write_all(&lfh_bytes).await?;
        let offset = self.pos;
        self.pos += lfh_bytes.len() as u64;

        let (deflate_encoder, passthrough) = if is_stored {
            (None, Some(CountWriter { inner, count: 0 }))
        } else {
            (
                Some(DeflateEncoder::with_quality(
                    ShutdownIgnoredWriter(CountWriter { inner, count: 0 }),
                    async_compression::Level::Precise(self.level as i32),
                )),
                None,
            )
        };

        Ok(EntryWriter {
            zip: self,
            deflate_encoder,
            passthrough,
            is_stored,
            crc_hasher: crc32fast::Hasher::new(),
            uncompressed_size: 0,
            local_header_offset: offset,
            name: name.to_string(),
            mtime: None,
            unix_permissions: None,
        })
    }

    /// Start a new directory entry.
    ///
    /// Writes the Local File Header and returns a [`DirectoryEntryWriter`] handle.
    /// Call [`close`](DirectoryEntryWriter::close) to finalize the entry.
    /// Directory names should end with `'/'`.
    ///
    /// # Errors
    ///
    /// Returns [`ZipError`] if writer is poisoned, or if writing the
    /// Local File Header fails (I/O error or field too long).
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use async_deflate_zip::ZipWriter;
    ///
    /// # async fn example() {
    /// let mut buf = Vec::new();
    /// let mut zip = ZipWriter::new(&mut buf);
    /// let mut dir = zip.append_directory("mydir/").await.unwrap();
    /// dir.close().await.unwrap();
    /// zip.finalize().await.unwrap();
    /// # }
    /// ```
    pub async fn append_directory<'a>(
        &'a mut self,
        name: &str,
    ) -> Result<DirectoryEntryWriter<'a, W>, ZipError> {
        let mut inner = self.inner.take().ok_or_else(|| {
            if self.poisoned {
                ZipError::Poisoned("previous entry was dropped without calling close()".to_string())
            } else {
                ZipError::InvalidState("entry writer already active".to_string())
            }
        })?;
        let needs_zip64 = self.pos > header::U32_MAX;
        let lfh = header::LocalFileHeader::new(name, header::METHOD_STORED, needs_zip64);
        let lfh_bytes = lfh.serialize()?;
        inner.write_all(&lfh_bytes).await?;
        let offset = self.pos;
        self.pos += lfh_bytes.len() as u64;

        Ok(DirectoryEntryWriter {
            zip: self,
            writer: Some(inner),
            name: name.to_string(),
            local_header_offset: offset,
            mtime: None,
            unix_permissions: None,
        })
    }

    /// Add a symbolic link entry.
    ///
    /// The `name` is the path of the symlink, and `target` is the path
    /// the symlink points to. The target is stored uncompressed as the
    /// entry's data content. The Central Directory entry uses `S_IFLNK`
    /// with `VERSION_UNIX` so Unix unzip tools correctly restore the
    /// symlink.
    ///
    /// # Errors
    ///
    /// Returns [`ZipError`] if writer is poisoned, or if writing the
    /// Local File Header, symlink target, or Data Descriptor fails (I/O error
    /// or field too long).
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use async_deflate_zip::ZipWriter;
    ///
    /// # async fn example() {
    /// let mut buf = Vec::new();
    /// let mut zip = ZipWriter::new(&mut buf);
    /// zip.append_symlink("link.txt", "target.txt").await.unwrap();
    /// zip.finalize().await.unwrap();
    /// # }
    /// ```
    pub async fn append_symlink(&mut self, name: &str, target: &str) -> Result<(), ZipError> {
        let mut inner = self.inner.take().ok_or_else(|| {
            if self.poisoned {
                ZipError::Poisoned("previous entry was dropped without calling close()".to_string())
            } else {
                ZipError::InvalidState("entry writer already active".to_string())
            }
        })?;
        let needs_zip64 = self.pos > header::U32_MAX;
        let lfh = header::LocalFileHeader::new(name, header::METHOD_STORED, needs_zip64);
        let lfh_bytes = lfh.serialize()?;
        inner.write_all(&lfh_bytes).await?;
        let offset = self.pos;
        self.pos += lfh_bytes.len() as u64;

        // Write the symlink target as stored (uncompressed) data
        let target_bytes = target.as_bytes();
        inner.write_all(target_bytes).await?;
        self.pos += target_bytes.len() as u64;

        // CRC-32 of the target path
        let mut hasher = crc32fast::Hasher::new();
        hasher.update(target_bytes);
        let crc32 = hasher.finalize();
        let data_size = target_bytes.len() as u64;

        let dd = header::DataDescriptor {
            crc32,
            compressed_size: data_size,
            uncompressed_size: data_size,
            zip64: data_size > header::U32_MAX || offset > header::U32_MAX,
        };
        let dd_bytes = dd.serialize();
        inner.write_all(&dd_bytes).await?;
        self.pos += dd_bytes.len() as u64;

        self.entries.push(StoredEntry {
            name: name.to_string(),
            crc32,
            compressed_size: data_size,
            uncompressed_size: data_size,
            local_header_offset: offset,
            is_directory: false,
            is_symlink: true,
            is_stored: false,
            mtime: None,
            unix_mtime: None,
            unix_permissions: None,
        });
        self.inner = Some(inner);
        Ok(())
    }

    /// Finalize the archive by writing the Central Directory and EOCDR.
    ///
    /// This writes the Central Directory entries for all file and directory
    /// entries, followed by the End of Central Directory Record (and ZIP64
    /// records if needed). The inner writer is flushed and shut down.
    ///
    /// After calling `finalize`, the `ZipWriter` is consumed and cannot be
    /// used to add more entries.
    ///
    /// # Errors
    ///
    /// Returns [`ZipError`] if an entry writer is still active or the writer is
    /// poisoned, if writing the Central Directory or EOCDR fails (I/O error or
    /// field too long), or if the inner writer's shutdown fails.
    pub async fn finalize(mut self) -> Result<(), ZipError> {
        let mut inner = self.inner.take().ok_or_else(|| {
            if self.poisoned {
                ZipError::Poisoned("previous entry was dropped without calling close()".to_string())
            } else {
                ZipError::InvalidState("entry writer still active".to_string())
            }
        })?;
        let cd_offset = self.pos;

        for entry in &self.entries {
            let cd_entry = entry.to_central_dir_entry();
            let data = cd_entry.serialize()?;
            inner.write_all(&data).await?;
            self.pos += data.len() as u64;
        }

        let cd_size = self.pos - cd_offset;
        let total_entries = self.entries.len() as u64;
        let needs_zip64 =
            total_entries > 0xFFFF || cd_size > header::U32_MAX || cd_offset > header::U32_MAX;

        if needs_zip64 {
            let eocdr64 = header::Zip64Eocdr {
                total_entries,
                cd_size,
                cd_offset,
            };
            let data = eocdr64.serialize();
            let eocdr64_offset = self.pos;
            inner.write_all(&data).await?;
            self.pos += data.len() as u64;

            let locator = header::Zip64EocdrLocator { eocdr64_offset };
            inner.write_all(&locator.serialize()).await?;
            self.pos += 20;
        }

        let eocdr = header::Eocdr {
            total_entries,
            cd_size,
            cd_offset,
        };
        inner.write_all(&eocdr.serialize()).await?;
        inner.shutdown().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::writer::test_utils::lookup_entry;
    use crate::CompressionLevel;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn test_zip_write_single_file() {
        let mut buf = Vec::new();
        let mut zip = ZipWriter::new(&mut buf);
        let mut entry = zip.append_file("hello.txt").await.unwrap();
        entry.write_all(b"Hello, World!").await.unwrap();
        entry.close().await.unwrap();
        zip.finalize().await.unwrap();

        assert!(buf.len() > 30);
        assert!(buf.windows(4).any(|w| w == b"PK\x03\x04"));
        assert!(buf.windows(4).any(|w| w == b"PK\x01\x02"));
        assert!(buf.windows(4).any(|w| w == b"PK\x05\x06"));
    }

    #[tokio::test]
    async fn test_zip_write_multiple_files() {
        let mut buf = Vec::new();
        let mut zip = ZipWriter::new(&mut buf);

        let mut entry = zip.append_file("a.txt").await.unwrap();
        entry.write_all(b"aaa").await.unwrap();
        entry.close().await.unwrap();

        let mut entry = zip.append_file("b.txt").await.unwrap();
        entry.write_all(b"bbb").await.unwrap();
        entry.close().await.unwrap();

        zip.finalize().await.unwrap();
        let cd_count = buf.windows(4).filter(|w| w == b"PK\x01\x02").count();
        assert_eq!(cd_count, 2);
    }

    #[tokio::test]
    async fn test_zip_compression_ratio() {
        let mut buf = Vec::new();
        let mut zip = ZipWriter::new(&mut buf).with_compression_level(CompressionLevel::BEST);

        let data = vec![b'A'; 1024];
        let mut entry = zip.append_file("repeated.txt").await.unwrap();
        entry.write_all(&data).await.unwrap();
        entry.close().await.unwrap();
        zip.finalize().await.unwrap();

        let entry = lookup_entry(&buf, 0);
        assert!(
            entry.compressed_size < entry.uncompressed_size,
            "compressed {} >= uncompressed {}",
            entry.compressed_size,
            entry.uncompressed_size
        );
    }

    #[tokio::test]
    async fn test_symlink_entry() {
        let mut buf = Vec::new();
        let mut zip = ZipWriter::new(&mut buf);
        zip.append_symlink("link.txt", "target.txt").await.unwrap();
        zip.finalize().await.unwrap();

        let pos = buf.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
        let cd = &buf[pos..];

        let vmb = u16::from_le_bytes(cd[4..6].try_into().unwrap());
        assert_eq!(vmb >> 8, 3, "expected Unix host OS for symlink");

        let version_needed = u16::from_le_bytes(cd[6..8].try_into().unwrap());
        assert_eq!(version_needed, 10, "expected VERSION_STORED for symlink");

        let method = u16::from_le_bytes(cd[10..12].try_into().unwrap());
        assert_eq!(method, 0, "expected METHOD_STORED for symlink");

        let efa = u32::from_le_bytes(cd[38..42].try_into().unwrap());
        assert!(
            (efa >> 16) & 0o170000 == 0o120000,
            "expected S_IFLNK in external_file_attributes, got {:06o}",
            efa >> 16
        );

        let lfh_pos = buf.windows(4).position(|w| w == b"PK\x03\x04").unwrap();
        let lfh = &buf[lfh_pos..];
        let lfh_name_len = u16::from_le_bytes(lfh[26..28].try_into().unwrap()) as usize;
        let lfh_extra_len = u16::from_le_bytes(lfh[28..30].try_into().unwrap()) as usize;
        let lfh_total = 30 + lfh_name_len + lfh_extra_len;
        let data = &buf[lfh_pos + lfh_total..lfh_pos + lfh_total + 10];
        assert_eq!(data, b"target.txt", "symlink target mismatch");
    }

    #[tokio::test]
    async fn test_zip64_finalize_many_entries() {
        let num_entries: u16 = 0xFFFF;
        let mut buf = Vec::new();
        let mut zip = ZipWriter::new(&mut buf).with_compression_level(CompressionLevel::NONE);

        for i in 0..=num_entries {
            let name = format!("f{i}");
            let mut entry = zip.append_file(&name).await.unwrap();
            entry.write_all(b"x").await.unwrap();
            entry.close().await.unwrap();
        }

        zip.finalize().await.unwrap();

        let eocdr_pos = buf.windows(4).rposition(|w| w == b"PK\x05\x06").unwrap();
        let eocdr_end = &buf[eocdr_pos..];
        assert_eq!(
            u16::from_le_bytes(eocdr_end[8..10].try_into().unwrap()),
            0xFFFF,
            "EOCDR total_entries should be sentinel 0xFFFF for ZIP64"
        );

        let locator_pos = buf.windows(4).rposition(|w| w == b"PK\x06\x07").unwrap();
        assert_eq!(&buf[locator_pos..locator_pos + 4], b"PK\x06\x07");

        let z64_pos = buf.windows(4).rposition(|w| w == b"PK\x06\x06").unwrap();
        assert_eq!(&buf[z64_pos..z64_pos + 4], b"PK\x06\x06");

        assert!(
            z64_pos < locator_pos && locator_pos < eocdr_pos,
            "expected Zip64Eocdr < Zip64EocdrLocator < Eocdr, got {z64_pos} < {locator_pos} < {eocdr_pos}"
        );

        let cd_count = buf.windows(4).filter(|w| w == b"PK\x01\x02").count();
        assert_eq!(cd_count, num_entries as usize + 1);

        assert_eq!(
            &buf[33..37],
            b"PK\x07\x08",
            "first entry should have DD signature"
        );
        assert_eq!(
            &buf[49..53],
            b"PK\x03\x04",
            "next LFH at offset 49 confirms 16-byte DD (non-ZIP64) for small-entry ZIP64 archive"
        );
    }

    #[tokio::test]
    async fn test_stored_entry_level_zero() {
        let mut buf = Vec::new();
        let mut zip = ZipWriter::new(&mut buf).with_compression_level(CompressionLevel::NONE);

        let data = b"Hello, stored entry!";
        let mut entry = zip.append_file("stored.txt").await.unwrap();
        entry.write_all(data).await.unwrap();
        entry.close().await.unwrap();
        zip.finalize().await.unwrap();

        let pos = buf.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
        let cd = &buf[pos..];
        let method = u16::from_le_bytes(cd[10..12].try_into().unwrap());
        assert_eq!(method, 0, "expected METHOD_STORED for level=0 entry");
        let version_needed = u16::from_le_bytes(cd[6..8].try_into().unwrap());
        assert_eq!(
            version_needed, 10,
            "expected VERSION_STORED for level=0 entry"
        );

        let compressed_size = u32::from_le_bytes(cd[20..24].try_into().unwrap()) as u64;
        let uncompressed_size = u32::from_le_bytes(cd[24..28].try_into().unwrap()) as u64;
        assert_eq!(
            compressed_size, uncompressed_size,
            "stored entry should have equal compressed and uncompressed sizes"
        );
        assert_eq!(compressed_size, data.len() as u64);

        let lfh_pos = buf.windows(4).position(|w| w == b"PK\x03\x04").unwrap();
        let lfh_method = u16::from_le_bytes(buf[lfh_pos + 8..lfh_pos + 10].try_into().unwrap());
        assert_eq!(lfh_method, 0, "LFH method should be STORED for level=0");
    }
}
