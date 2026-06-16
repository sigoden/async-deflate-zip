use super::entry_options::EntryOptions;
use super::entry_writer::EntryWriter;
use super::stored_entry::StoredEntry;
use crate::count_writer::CountWriter;
use crate::zip_format::sanitize_path;

use crate::deflate_encoder::DeflateEncoder;
use crate::error::ZipError;
use crate::validate::{validate_comment, validate_input};
use crate::zip_format;

use crate::CompressionLevel;

use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};

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
/// use async_deflate_zip::{ZipWriter, EntryOptions};
/// use tokio::io::AsyncWriteExt;
///
/// # async fn example() {
/// let mut buf = Vec::new();
/// let mut zip = ZipWriter::new(&mut buf);
///
/// let mut entry = zip.start_file("hello.txt", &EntryOptions::file()).await.unwrap();
/// entry.write_all(b"Hello, World!").await.unwrap();
/// entry.finish().await.unwrap();
///
/// let _inner = zip.finish().await.unwrap();
/// # }
/// ```
pub struct ZipWriter<W: AsyncWrite + Unpin> {
    pub(crate) inner: Option<W>,
    pub(crate) entries: Vec<StoredEntry>,
    level: CompressionLevel,
    pub(crate) pos: u64,
    pub(crate) poisoned: bool,
    comment: Option<Vec<u8>>,
}

impl<W: AsyncWrite + Unpin> ZipWriter<W> {
    /// Create a new `ZipWriter` wrapping an async writer.
    ///
    /// Uses the default compression level ([`CompressionLevel::default`], level 6).
    pub fn new(inner: W) -> Self {
        Self {
            inner: Some(inner),
            entries: Vec::new(),
            level: CompressionLevel::default(),
            pos: 0,
            poisoned: false,
            comment: None,
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
    ///     .with_compression_level(CompressionLevel::best());
    /// ```
    pub fn with_compression_level(mut self, level: CompressionLevel) -> Self {
        self.level = level;
        self
    }

    /// Set the archive-level comment.
    ///
    /// The comment is embedded in the End of Central Directory Record and can
    /// be up to 65535 bytes when encoded as UTF-8. Returns `self` for chaining.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use async_deflate_zip::ZipWriter;
    ///
    /// let mut buf = Vec::new();
    /// let zip = ZipWriter::new(&mut buf)
    ///     .with_comment("Hello, archive!");
    /// ```
    pub fn with_comment(mut self, comment: &str) -> Self {
        self.comment = Some(comment.as_bytes().to_vec());
        self
    }

    /// Take ownership of the inner writer, returning an error if it's already taken.
    fn take_inner(&mut self) -> Result<W, ZipError> {
        self.inner.take().ok_or({
            if self.poisoned {
                ZipError::EntryWriterCorrupted
            } else {
                ZipError::WriterCorrupted
            }
        })
    }

    /// Start a new file entry and return an [`EntryWriter`] for streaming data.
    ///
    /// Writes the Local File Header, then returns an `EntryWriter` that
    /// compresses and buffers written data. Call [`EntryWriter::finish`]
    /// to finish the entry and write the trailing CRC-32 and sizes.
    ///
    /// # Errors
    ///
    /// Returns [`ZipError`] if writer is poisoned, or if writing the
    /// Local File Header fails (I/O error or field too long).
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use async_deflate_zip::{ZipWriter, EntryOptions};
    /// use tokio::io::AsyncWriteExt;
    ///
    /// # async fn example() {
    /// let mut buf = Vec::new();
    /// let mut zip = ZipWriter::new(&mut buf);
    /// let mut entry = zip.start_file("readme.txt", &EntryOptions::file()).await.unwrap();
    /// entry.write_all(b"content").await.unwrap();
    /// entry.finish().await.unwrap();
    /// let _inner = zip.finish().await.unwrap();
    /// # }
    /// ```
    pub async fn start_file<'a>(
        &'a mut self,
        name: &str,
        options: &EntryOptions,
    ) -> Result<EntryWriter<'a, W>, ZipError> {
        let name = sanitize_path(name);
        validate_input(&name, options.comment())?;

        let mut inner = self.take_inner()?;

        let is_stored = self.level.value() == 0;
        let method = if is_stored {
            zip_format::METHOD_STORED
        } else {
            zip_format::METHOD_DEFLATE
        };

        let needs_zip64 = self.pos > zip_format::U32_MAX;
        let lfh = zip_format::LocalFileHeader::new(&name, method, needs_zip64, *options.mtime());
        let lfh_bytes = lfh.serialize()?;
        inner.write_all(&lfh_bytes).await?;
        let offset = self.pos;
        self.pos += lfh_bytes.len() as u64;

        let (deflate_encoder, passthrough) = if is_stored {
            (None, Some(CountWriter { inner, count: 0 }))
        } else {
            (
                Some(DeflateEncoder::new(
                    CountWriter { inner, count: 0 },
                    self.level,
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
            name: name.into_owned(),
            mtime: *options.mtime(),
            unix_permissions: options.unix_permissions(),
            uid_gid: options.uid_gid(),
            comment: options.comment().map(|c| c.to_vec()),
        })
    }

    /// Stream data from an [`AsyncRead`] into a new file entry.
    ///
    /// Reads from `reader` until EOF, writing all data into the entry, then
    /// finishes it.
    ///
    /// # Errors
    ///
    /// Returns [`ZipError`] if the writer is poisoned, or if any I/O
    /// operation fails.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use async_deflate_zip::{ZipWriter, EntryOptions};
    /// use tokio::io::AsyncReadExt;
    ///
    /// # async fn example() {
    /// let mut buf = Vec::new();
    /// let mut zip = ZipWriter::new(&mut buf);
    /// let data: &[u8] = b"streamed data";
    /// zip.add_reader("stream.txt", data, &EntryOptions::file()).await.unwrap();
    /// let _inner = zip.finish().await.unwrap();
    /// # }
    /// ```
    pub async fn add_reader<R>(
        &mut self,
        path: &str,
        mut reader: R,
        options: &EntryOptions,
    ) -> Result<(), ZipError>
    where
        R: AsyncRead + Unpin,
    {
        let mut entry = self.start_file(path, options).await?;
        tokio::io::copy(&mut reader, &mut entry)
            .await
            .map_err(ZipError::Io)?;
        entry.finish().await
    }

    /// Start a new directory entry.
    ///
    /// Writes the Local File Header and Data Descriptor, then registers
    /// the entry in the archive. Directory names should end with `'/'`.
    ///
    /// # Errors
    ///
    /// Returns [`ZipError`] if writer is poisoned, or if writing fails.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use async_deflate_zip::{ZipWriter, EntryOptions};
    ///
    /// # async fn example() {
    /// let mut buf = Vec::new();
    /// let mut zip = ZipWriter::new(&mut buf);
    /// zip.add_directory("mydir/", &EntryOptions::directory()).await.unwrap();
    /// let _inner = zip.finish().await.unwrap();
    /// # }
    /// ```
    pub async fn add_directory(
        &mut self,
        name: &str,
        options: &EntryOptions,
    ) -> Result<(), ZipError> {
        let mut name = sanitize_path(name).into_owned();
        if !name.ends_with('/') {
            name.push('/');
        }
        validate_input(&name, options.comment())?;

        let mut inner = self.take_inner()?;

        let needs_zip64 = self.pos > zip_format::U32_MAX;
        let lfh = zip_format::LocalFileHeader::new(
            &name,
            zip_format::METHOD_STORED,
            needs_zip64,
            *options.mtime(),
        );
        let lfh_bytes = lfh.serialize()?;
        inner.write_all(&lfh_bytes).await?;
        let offset = self.pos;
        self.pos += lfh_bytes.len() as u64;

        let dd = zip_format::DataDescriptor {
            crc32: 0,
            compressed_size: 0,
            uncompressed_size: 0,
            zip64: offset > zip_format::U32_MAX,
        };
        let dd_bytes = dd.serialize();
        inner.write_all(&dd_bytes).await.map_err(|e| {
            self.poisoned = true;
            ZipError::Io(e)
        })?;
        self.pos += dd_bytes.len() as u64;

        let unix_mtime = zip_format::system_time_to_unix_secs(*options.mtime());

        self.entries.push(StoredEntry {
            name: name.to_string(),
            crc32: 0,
            compressed_size: 0,
            uncompressed_size: 0,
            local_header_offset: offset,
            is_directory: true,
            is_symlink: false,
            is_stored: true,
            unix_mtime,
            unix_permissions: options.unix_permissions(),
            uid_gid: options.uid_gid(),
            comment: options.comment().map(|c| c.to_vec()),
        });

        self.inner = Some(inner);
        Ok(())
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
    /// use async_deflate_zip::{ZipWriter, EntryOptions};
    ///
    /// # async fn example() {
    /// let mut buf = Vec::new();
    /// let mut zip = ZipWriter::new(&mut buf);
    /// zip.add_symlink("link.txt", "target.txt", &EntryOptions::symlink()).await.unwrap();
    /// let _inner = zip.finish().await.unwrap();
    /// # }
    /// ```
    pub async fn add_symlink(
        &mut self,
        name: &str,
        target: &str,
        options: &EntryOptions,
    ) -> Result<(), ZipError> {
        let name = sanitize_path(name).into_owned();
        validate_input(&name, options.comment())?;

        let mut inner = self.take_inner()?;

        let needs_zip64 = self.pos > zip_format::U32_MAX;
        let lfh = zip_format::LocalFileHeader::new(
            &name,
            zip_format::METHOD_STORED,
            needs_zip64,
            *options.mtime(),
        );
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

        let dd = zip_format::DataDescriptor {
            crc32,
            compressed_size: data_size,
            uncompressed_size: data_size,
            zip64: data_size > zip_format::U32_MAX || offset > zip_format::U32_MAX,
        };
        let dd_bytes = dd.serialize();
        inner.write_all(&dd_bytes).await.map_err(|e| {
            self.poisoned = true;
            ZipError::Io(e)
        })?;
        self.pos += dd_bytes.len() as u64;

        let unix_mtime = zip_format::system_time_to_unix_secs(*options.mtime());

        self.entries.push(StoredEntry {
            name,
            crc32,
            compressed_size: data_size,
            uncompressed_size: data_size,
            local_header_offset: offset,
            is_directory: false,
            is_symlink: true,
            is_stored: true,
            unix_mtime,
            unix_permissions: options.unix_permissions(),
            uid_gid: options.uid_gid(),
            comment: options.comment().map(|c| c.to_vec()),
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
    /// After calling `finish`, the `ZipWriter` is consumed and the inner
    /// writer is returned.
    ///
    /// # Errors
    ///
    /// Returns [`ZipError`] if an entry writer is still active or the writer is
    /// poisoned, if writing the Central Directory or EOCDR fails (I/O error or
    /// field too long), or if the inner writer's shutdown fails.
    pub async fn finish(mut self) -> Result<W, ZipError> {
        let mut inner = self.take_inner()?;
        let cd_offset = self.pos;

        for entry in &self.entries {
            let cd_entry = entry.to_central_dir_entry();
            let data = cd_entry.serialize()?;
            inner.write_all(&data).await?;
            self.pos += data.len() as u64;
        }

        let cd_size = self.pos - cd_offset;
        let total_entries = self.entries.len() as u64;
        let needs_zip64 = total_entries > 0xFFFF
            || cd_size > zip_format::U32_MAX
            || cd_offset > zip_format::U32_MAX;

        if needs_zip64 {
            let eocdr64 = zip_format::Zip64Eocdr {
                total_entries,
                cd_size,
                cd_offset,
            };
            let data = eocdr64.serialize();
            let eocdr64_offset = self.pos;
            inner.write_all(&data).await?;
            self.pos += data.len() as u64;

            let locator = zip_format::Zip64EocdrLocator { eocdr64_offset };
            inner.write_all(&locator.serialize()).await?;
            self.pos += 20;
        }

        if let Some(ref comment) = self.comment {
            validate_comment(comment)?;
        }
        let eocdr = zip_format::Eocdr {
            total_entries,
            cd_size,
            cd_offset,
            comment: self.comment,
        };
        inner.write_all(&eocdr.serialize()).await?;
        inner.shutdown().await?;
        Ok(inner)
    }

    /// Abort the archive and return the inner writer without writing
    /// any finalization data (Central Directory, EOCDR).
    ///
    /// This is useful for recovering the underlying writer when an
    /// error occurs during entry writing. The ZIP output will be
    /// incomplete and should be discarded.
    ///
    /// # Errors
    ///
    /// Returns [`ZipError::WriterCorrupted`] if an [`EntryWriter`] is still
    /// active (the inner writer has been moved into that entry), or
    /// [`ZipError::EntryWriterCorrupted`] if the writer is in a poisoned
    /// state (an entry was dropped without finishing).
    pub fn abort(self) -> Result<W, ZipError> {
        self.inner.ok_or(if self.poisoned {
            ZipError::EntryWriterCorrupted
        } else {
            ZipError::WriterCorrupted
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CompressionLevel;
    use crate::test_utils::*;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn test_zip_write_single_file() {
        let mut buf = Vec::new();
        let mut zip = ZipWriter::new(&mut buf);
        let mut entry = zip
            .start_file("hello.txt", &EntryOptions::file())
            .await
            .unwrap();
        entry.write_all(b"Hello, World!").await.unwrap();
        entry.finish().await.unwrap();
        zip.finish().await.unwrap();

        assert_has_pk_signatures(&buf);
        assert_entry_count(&buf, 1);
        assert_entry_content(&buf, "hello.txt", b"Hello, World!");
    }

    #[tokio::test]
    async fn test_zip_write_multiple_files() {
        let mut buf = Vec::new();
        let mut zip = ZipWriter::new(&mut buf);

        let mut entry = zip
            .start_file("a.txt", &EntryOptions::file())
            .await
            .unwrap();
        entry.write_all(b"aaa").await.unwrap();
        entry.finish().await.unwrap();

        let mut entry = zip
            .start_file("b.txt", &EntryOptions::file())
            .await
            .unwrap();
        entry.write_all(b"bbb").await.unwrap();
        entry.finish().await.unwrap();

        zip.finish().await.unwrap();
        assert_entry_count(&buf, 2);
        assert_entry_content(&buf, "a.txt", b"aaa");
        assert_entry_content(&buf, "b.txt", b"bbb");
    }

    #[tokio::test]
    async fn test_zip_compression_ratio() {
        let mut buf = Vec::new();
        let mut zip = ZipWriter::new(&mut buf).with_compression_level(CompressionLevel::best());

        let data = vec![b'A'; 1024];
        let mut entry = zip
            .start_file("repeated.txt", &EntryOptions::file())
            .await
            .unwrap();
        entry.write_all(&data).await.unwrap();
        entry.finish().await.unwrap();
        zip.finish().await.unwrap();

        assert_compressed_smaller(&buf, "repeated.txt");
    }

    #[tokio::test]
    async fn test_symlink_entry() {
        let mut buf = Vec::new();
        let mut zip = ZipWriter::new(&mut buf);
        zip.add_symlink("link.txt", "target.txt", &EntryOptions::symlink())
            .await
            .unwrap();
        zip.finish().await.unwrap();

        assert_entry_compression(&buf, "link.txt", zip::CompressionMethod::Stored);
        assert_entry_content(&buf, "link.txt", b"target.txt");
    }

    #[tokio::test]
    async fn test_zip64_finish_many_entries() {
        let num_entries: u16 = 0xFFFF;
        let mut buf = Vec::new();
        let mut zip = ZipWriter::new(&mut buf).with_compression_level(CompressionLevel::none());

        for i in 0..=num_entries {
            let name = format!("f{i}");
            let mut entry = zip.start_file(&name, &EntryOptions::file()).await.unwrap();
            entry.write_all(b"x").await.unwrap();
            entry.finish().await.unwrap();
        }

        zip.finish().await.unwrap();

        assert_entry_count(&buf, num_entries as usize + 1);

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

        assert!(
            buf.windows(4).any(|w| w == b"PK\x07\x08"),
            "first entry should have DD signature"
        );
    }

    #[tokio::test]
    async fn test_stored_entry_level_zero() {
        let mut buf = Vec::new();
        let mut zip = ZipWriter::new(&mut buf).with_compression_level(CompressionLevel::none());

        let data = b"Hello, stored entry!";
        let mut entry = zip
            .start_file("stored.txt", &EntryOptions::file())
            .await
            .unwrap();
        entry.write_all(data).await.unwrap();
        entry.finish().await.unwrap();
        zip.finish().await.unwrap();

        assert_entry_compression(&buf, "stored.txt", zip::CompressionMethod::Stored);
        assert_entry_sizes_match(&buf, "stored.txt");
        assert_entry_content(&buf, "stored.txt", data);
    }

    #[tokio::test]
    async fn test_directory_entry() {
        let mut buf = Vec::new();
        let mut zip = ZipWriter::new(&mut buf);
        zip.add_directory("mydir/", &EntryOptions::directory())
            .await
            .unwrap();
        zip.finish().await.unwrap();

        assert_entry_compression(&buf, "mydir/", zip::CompressionMethod::Stored);
        assert_unix_mode(&buf, "mydir/", 0o040755);
        assert_extra_has_tag(&buf, "mydir/", b"UT");
    }

    #[tokio::test]
    async fn test_entry_comment() {
        let mut buf = Vec::new();
        let mut zip = ZipWriter::new(&mut buf);
        let mut entry = zip
            .start_file(
                "commented.txt",
                &EntryOptions::file().with_comment("file comment"),
            )
            .await
            .unwrap();
        entry.write_all(b"data").await.unwrap();
        entry.finish().await.unwrap();
        zip.finish().await.unwrap();
        assert_entry_comment(&buf, "commented.txt", "file comment");

        let mut buf = Vec::new();
        let mut zip = ZipWriter::new(&mut buf);
        zip.add_directory(
            "dir/",
            &EntryOptions::directory().with_comment("dir comment"),
        )
        .await
        .unwrap();
        zip.finish().await.unwrap();
        assert_entry_comment(&buf, "dir/", "dir comment");
    }

    #[tokio::test]
    async fn test_archive_comment() {
        let mut buf = Vec::new();
        let mut zip = ZipWriter::new(&mut buf).with_comment("archive comment");
        let mut entry = zip
            .start_file("f.txt", &EntryOptions::file())
            .await
            .unwrap();
        entry.write_all(b"data").await.unwrap();
        entry.finish().await.unwrap();
        zip.finish().await.unwrap();

        assert_archive_comment(&buf, "archive comment");
    }
}
