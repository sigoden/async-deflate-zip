use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use async_compression::tokio::write::DeflateEncoder;
use tokio::io::{AsyncWrite, AsyncWriteExt};

use crate::header;
use crate::types::CompressionLevel;

// === Writer wrappers ===

/// Wraps an AsyncWrite and ignores `poll_shutdown`.
struct ShutdownIgnoredWriter<W>(W);

impl<W: AsyncWrite + Unpin> AsyncWrite for ShutdownIgnoredWriter<W> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.0).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

/// Counts bytes written through it.
struct CountWriter<W> {
    inner: W,
    count: u64,
}

impl<W: AsyncWrite + Unpin> AsyncWrite for CountWriter<W> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = &mut *self;
        match Pin::new(&mut this.inner).poll_write(cx, buf) {
            Poll::Ready(Ok(n)) => {
                this.count += n as u64;
                Poll::Ready(Ok(n))
            }
            other => other,
        }
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

// === StoredEntry ===

pub(crate) struct StoredEntry {
    name: String,
    crc32: u32,
    compressed_size: u64,
    uncompressed_size: u64,
    local_header_offset: u64,
    is_directory: bool,
    is_symlink: bool,
    is_stored: bool,
    mtime: Option<(u16, u16)>,
    unix_mtime: Option<u64>,
    unix_permissions: Option<u32>,
}

impl StoredEntry {
    fn to_central_dir_entry(&self) -> header::CentralDirEntry {
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

        header::CentralDirEntry {
            version_made_by,
            version_needed: if self.is_directory || self.is_symlink || self.is_stored {
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

// === ZipWriter ===

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
    poisoned: bool,
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
    /// Returns `std::io::Error` if writing the Local File Header fails.
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
    pub async fn append_file<'a>(&'a mut self, name: &str) -> io::Result<EntryWriter<'a, W>> {
        let mut inner = self.inner.take().ok_or_else(|| {
            if self.poisoned {
                io::Error::other(
                    "archive corrupted: previous entry was dropped without calling close()",
                )
            } else {
                io::Error::other("entry writer already active")
            }
        })?;

        let is_stored = self.level == 0;
        let method = if is_stored {
            header::METHOD_STORED
        } else {
            header::METHOD_DEFLATE
        };

        let lfh = header::LocalFileHeader::new(name, method);
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

    /// Add a directory entry (no data, just a placeholder).
    ///
    /// Directory names should end with `'/'`. The entry is written as a
    /// stored (uncompressed) entry with zero data size.
    ///
    /// # Errors
    ///
    /// Returns [`std::io::Error`] if writing the Local File Header or Data Descriptor fails.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use async_deflate_zip::ZipWriter;
    ///
    /// # async fn example() {
    /// let mut buf = Vec::new();
    /// let mut zip = ZipWriter::new(&mut buf);
    /// zip.append_directory("mydir/").await.unwrap();
    /// zip.finalize().await.unwrap();
    /// # }
    /// ```
    pub async fn append_directory(&mut self, name: &str) -> io::Result<()> {
        let mut inner = self.inner.take().ok_or_else(|| {
            if self.poisoned {
                io::Error::other(
                    "archive corrupted: previous entry was dropped without calling close()",
                )
            } else {
                io::Error::other("entry writer already active")
            }
        })?;
        let lfh = header::LocalFileHeader::new(name, header::METHOD_STORED);
        let lfh_bytes = lfh.serialize()?;
        inner.write_all(&lfh_bytes).await?;
        let offset = self.pos;
        self.pos += lfh_bytes.len() as u64;

        let dd = header::DataDescriptor {
            crc32: 0,
            compressed_size: 0,
            uncompressed_size: 0,
            zip64: false,
        };
        let dd_bytes = dd.serialize();
        inner.write_all(&dd_bytes).await?;
        self.pos += dd_bytes.len() as u64;

        self.entries.push(StoredEntry {
            name: name.to_string(),
            crc32: 0,
            compressed_size: 0,
            uncompressed_size: 0,
            local_header_offset: offset,
            is_directory: true,
            is_symlink: false,
            is_stored: false,
            mtime: None,
            unix_mtime: None,
            unix_permissions: None,
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
    /// Returns [`std::io::Error`] if writing the Local File Header, the symlink
    /// target data, or the Data Descriptor fails.
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
    pub async fn append_symlink(&mut self, name: &str, target: &str) -> io::Result<()> {
        let mut inner = self.inner.take().ok_or_else(|| {
            if self.poisoned {
                io::Error::other(
                    "archive corrupted: previous entry was dropped without calling close()",
                )
            } else {
                io::Error::other("entry writer already active")
            }
        })?;
        let lfh = header::LocalFileHeader::new(name, header::METHOD_STORED);
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
            zip64: false,
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
    /// Returns [`std::io::Error`] if writing the Central Directory or EOCDR fails,
    /// or if the inner writer's shutdown fails.
    ///
    /// # Errors
    ///
    /// Returns an error if the inner writer is not available
    /// (i.e., an `EntryWriter` is still active and hasn't been closed).
    pub async fn finalize(mut self) -> io::Result<()> {
        let mut inner = self.inner.take().ok_or_else(|| {
            if self.poisoned {
                io::Error::other(
                    "archive corrupted: previous entry was dropped without calling close()",
                )
            } else {
                io::Error::other("entry writer still active")
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

// === EntryWriter ===

pin_project_lite::pin_project! {
    /// A streaming writer for a single file entry in a ZIP archive.
    ///
    /// Obtained from [`ZipWriter::append_file`]. Data written through this
    /// writer is compressed with DEFLATE and streamed to the underlying output.
    ///
    /// # Important
    ///
    /// The [`close`](EntryWriter::close) method **must** be called after all
    /// data is written to finalize the deflate frame and write the Data
    /// Descriptor. Dropping without closing will lose the entry.
    pub struct EntryWriter<'a, W>
    where
        W: AsyncWrite,
        W: Unpin,
    {
        zip: &'a mut ZipWriter<W>,
        #[pin]
        deflate_encoder: Option<DeflateEncoder<ShutdownIgnoredWriter<CountWriter<W>>>>,
        #[pin]
        passthrough: Option<CountWriter<W>>,
        is_stored: bool,
        crc_hasher: crc32fast::Hasher,
        uncompressed_size: u64,
        local_header_offset: u64,
        name: String,
        mtime: Option<std::time::SystemTime>,
        unix_permissions: Option<u32>,
    }

    impl<W> PinnedDrop for EntryWriter<'_, W>
    where
        W: AsyncWrite,
        W: Unpin,
    {
        fn drop(this: Pin<&mut Self>) {
            let this = this.project();
            if this.deflate_encoder.is_some() || this.passthrough.is_some() {
                // close() was never called — mark the ZipWriter as poisoned
                this.zip.poisoned = true;
            }
        }
    }
}

impl<W: AsyncWrite + Unpin> EntryWriter<'_, W> {
    /// Set the modification time for this entry.
    ///
    /// The timestamp is stored in the Central Directory using MS-DOS format
    /// and as an extended timestamp extra field (0x5455).
    pub fn set_mtime(&mut self, mtime: std::time::SystemTime) -> &mut Self {
        self.mtime = Some(mtime);
        self
    }

    /// Set Unix file permissions for this entry.
    ///
    /// Provide permission bits including setuid/setgid/sticky (e.g., `0o4755`, `0o2755`).
    /// The crate automatically adds the file type bit (`S_IFREG` for files,
    /// `S_IFDIR` for directories).
    pub fn set_permissions(&mut self, mode: u32) -> &mut Self {
        self.unix_permissions = Some(mode & 0o7777);
        self
    }

    /// Finalize the deflate frame, compute the CRC-32 checksum, and write
    /// the Data Descriptor.
    ///
    /// This consumes the `EntryWriter`, flushes the deflate encoder, extracts
    /// the compressed size, computes the CRC-32 of the uncompressed data, and
    /// writes the trailing CRC-32 and sizes (Data Descriptor) after the compressed
    /// data. The inner writer is returned to the parent [`ZipWriter`].
    ///
    /// # Errors
    ///
    /// Returns [`std::io::Error`] if the deflate encoder fails to shut down, or if
    /// writing the Data Descriptor fails.
    ///
    /// # Errors
    ///
    /// Returns an error if `close` is called more than once on the same entry.
    pub async fn close(mut self) -> io::Result<()> {
        let (compressed_size, mut inner) = if self.is_stored {
            let cw = self
                .passthrough
                .take()
                .ok_or_else(|| io::Error::other("entry already closed"))?;
            (cw.count, cw.inner)
        } else {
            let mut encoder = self
                .deflate_encoder
                .take()
                .ok_or_else(|| io::Error::other("entry already closed"))?;
            encoder.shutdown().await?;

            // Extract the inner writer from the encoder stack
            let shutdown_writer: ShutdownIgnoredWriter<CountWriter<W>> = encoder.into_inner();
            let count_writer: CountWriter<W> = shutdown_writer.0;
            let compressed_size = count_writer.count;
            (compressed_size, count_writer.inner)
        };

        let crc32 = self.crc_hasher.clone().finalize();

        let dd = header::DataDescriptor {
            crc32,
            compressed_size,
            uncompressed_size: self.uncompressed_size,
            // Use ZIP64 DD when any entry field exceeds 32 bits, consistent with
            // CentralDirEntry::serialize() which also checks local_header_offset.
            zip64: compressed_size > header::U32_MAX
                || self.uncompressed_size > header::U32_MAX
                || self.local_header_offset > header::U32_MAX,
        };
        let dd_bytes = dd.serialize();
        inner.write_all(&dd_bytes).await?;

        // Update position tracker: compressed data + data descriptor
        self.zip.pos += compressed_size + dd_bytes.len() as u64;

        let (mtime_msdos, unix_mtime) = match self.mtime {
            Some(t) => {
                let (time, date) = header::system_time_to_ms_dos(t);
                let secs = t
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                (Some((time, date)), Some(secs))
            }
            None => (None, None),
        };

        self.zip.entries.push(StoredEntry {
            name: self.name.clone(),
            crc32,
            compressed_size,
            uncompressed_size: self.uncompressed_size,
            local_header_offset: self.local_header_offset,
            is_directory: false,
            is_symlink: false,
            is_stored: self.is_stored,
            mtime: mtime_msdos,
            unix_mtime,
            unix_permissions: self.unix_permissions,
        });

        // Return the inner writer to ZipWriter
        self.zip.inner = Some(inner);
        Ok(())
    }
}

// === AsyncWrite impl for EntryWriter ===

impl<W: AsyncWrite + Unpin> AsyncWrite for EntryWriter<'_, W> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.project();
        let result = if *this.is_stored {
            match this.passthrough.as_pin_mut() {
                Some(w) => w.poll_write(cx, buf),
                None => return Poll::Ready(Err(io::Error::other("write after entry closed"))),
            }
        } else {
            match this.deflate_encoder.as_pin_mut() {
                Some(e) => e.poll_write(cx, buf),
                None => return Poll::Ready(Err(io::Error::other("write after entry closed"))),
            }
        };
        match result {
            Poll::Ready(Ok(n)) => {
                this.crc_hasher.update(&buf[..n]);
                *this.uncompressed_size += n as u64;
                Poll::Ready(Ok(n))
            }
            other => other,
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.project();
        if *this.is_stored {
            match this.passthrough.as_pin_mut() {
                Some(w) => w.poll_flush(cx),
                None => Poll::Ready(Err(io::Error::other("flush after entry closed"))),
            }
        } else {
            match this.deflate_encoder.as_pin_mut() {
                Some(e) => e.poll_flush(cx),
                None => Poll::Ready(Err(io::Error::other("flush after entry closed"))),
            }
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.project();
        if *this.is_stored {
            match this.passthrough.as_pin_mut() {
                Some(w) => w.poll_shutdown(cx),
                None => Poll::Ready(Err(io::Error::other("shutdown after entry closed"))),
            }
        } else {
            match this.deflate_encoder.as_pin_mut() {
                Some(e) => e.poll_shutdown(cx),
                None => Poll::Ready(Err(io::Error::other("shutdown after entry closed"))),
            }
        }
    }
}

// === Tests ===

#[cfg(test)]
mod tests {
    use super::*;

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

        // Find CD entry and check compressed_size < uncompressed_size
        let entry = lookup_entry(&buf, 0);
        assert!(
            entry.compressed_size < entry.uncompressed_size,
            "compressed {} >= uncompressed {}",
            entry.compressed_size,
            entry.uncompressed_size
        );
    }

    #[tokio::test]
    async fn test_directory_entry() {
        let mut buf = Vec::new();
        let mut zip = ZipWriter::new(&mut buf);
        zip.append_directory("emptydir/").await.unwrap();

        let mut entry = zip.append_file("emptydir/hello.txt").await.unwrap();
        entry.write_all(b"nested").await.unwrap();
        entry.close().await.unwrap();

        zip.finalize().await.unwrap();

        assert!(buf.windows(4).any(|w| w == b"PK\x03\x04"));
    }

    #[tokio::test]
    async fn test_symlink_entry() {
        let mut buf = Vec::new();
        let mut zip = ZipWriter::new(&mut buf);
        zip.append_symlink("link.txt", "target.txt").await.unwrap();
        zip.finalize().await.unwrap();

        // Find first CD entry
        let pos = buf.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
        let cd = &buf[pos..];

        // version_made_by at offset 4: upper byte should be 3 (Unix)
        let vmb = u16::from_le_bytes(cd[4..6].try_into().unwrap());
        assert_eq!(vmb >> 8, 3, "expected Unix host OS for symlink");

        // version_needed at offset 6: should be 10 (STORED)
        let version_needed = u16::from_le_bytes(cd[6..8].try_into().unwrap());
        assert_eq!(version_needed, 10, "expected VERSION_STORED for symlink");

        // method at offset 10: should be 0 (STORED)
        let method = u16::from_le_bytes(cd[10..12].try_into().unwrap());
        assert_eq!(method, 0, "expected METHOD_STORED for symlink");

        // external_file_attributes at offset 38: should have S_IFLNK (0o120000)
        let efa = u32::from_le_bytes(cd[38..42].try_into().unwrap());
        assert!(
            (efa >> 16) & 0o170000 == 0o120000,
            "expected S_IFLNK in external_file_attributes, got {:06o}",
            efa >> 16
        );

        // Verify symlink target content via LFH + data
        // Find the LFH and read the data after it
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
        // Trigger ZIP64 via total_entries > 0xFFFF, exercising:
        //   - Zip64Eocdr / Zip64EocdrLocator writing in finalize()
        //   - Eocdr with ZIP64 sentinels
        //   - CentralDirEntry version_needed upgrade to VERSION_ZIP64
        let num_entries: u16 = 0xFFFF;
        let mut buf = Vec::new();
        let mut zip = ZipWriter::new(&mut buf).with_compression_level(CompressionLevel::NONE);

        for i in 0..=num_entries {
            let name = format!("f{i}");
            let mut entry = zip.append_file(&name).await.unwrap();
            // Write a small payload so compressed_size is non-zero and CD entry
            // correctly records stored size.
            entry.write_all(b"x").await.unwrap();
            entry.close().await.unwrap();
        }

        zip.finalize().await.unwrap();

        // EOCDR should use ZIP64 sentinels
        let eocdr_pos = buf.windows(4).rposition(|w| w == b"PK\x05\x06").unwrap();
        let eocdr_end = &buf[eocdr_pos..];
        assert_eq!(
            u16::from_le_bytes(eocdr_end[8..10].try_into().unwrap()),
            0xFFFF,
            "EOCDR total_entries should be sentinel 0xFFFF for ZIP64"
        );

        // ZIP64 EOCD locator should exist
        let locator_pos = buf.windows(4).rposition(|w| w == b"PK\x06\x07").unwrap();
        assert_eq!(&buf[locator_pos..locator_pos + 4], b"PK\x06\x07");

        // ZIP64 EOCD should exist
        let z64_pos = buf.windows(4).rposition(|w| w == b"PK\x06\x06").unwrap();
        assert_eq!(&buf[z64_pos..z64_pos + 4], b"PK\x06\x06");

        // Verify ordering: Zip64Eocdr < Zip64EocdrLocator < Eocdr
        assert!(
            z64_pos < locator_pos && locator_pos < eocdr_pos,
            "expected Zip64Eocdr < Zip64EocdrLocator < Eocdr, got {z64_pos} < {locator_pos} < {eocdr_pos}"
        );

        // Verify correct number of CD entries
        let cd_count = buf.windows(4).filter(|w| w == b"PK\x01\x02").count();
        assert_eq!(cd_count, num_entries as usize + 1);

        // Individual entries have small sizes and offsets, so each DD should be
        // 16 bytes (non-ZIP64), even though the overall archive uses ZIP64 for
        // the EOCD (due to total_entries > 0xFFFF).
        // Entry "f0": LFH(30 + name_len=2 + extra_len=0 = 32) + data(1) = 33.
        // DD at offset 33 must be 16 bytes — if ZIP64 (24 bytes), next LFH would
        // be at 57 instead of 49.
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

    fn lookup_entry(buf: &[u8], index: usize) -> StoredEntry {
        let sig = b"PK\x01\x02";
        let pos: Vec<usize> = buf
            .windows(4)
            .enumerate()
            .filter(|(_, w)| w == sig)
            .map(|(i, _)| i)
            .collect();
        let pos = *pos.get(index).expect("entry not found");
        let cd = &buf[pos..];

        let crc32 = u32::from_le_bytes(cd[16..20].try_into().unwrap());
        let compressed_size = u32::from_le_bytes(cd[20..24].try_into().unwrap()) as u64;
        let uncompressed_size = u32::from_le_bytes(cd[24..28].try_into().unwrap()) as u64;
        let name_len = u16::from_le_bytes(cd[28..30].try_into().unwrap()) as usize;
        let name = String::from_utf8_lossy(&cd[46..46 + name_len]).to_string();

        StoredEntry {
            name,
            crc32,
            compressed_size,
            uncompressed_size,
            local_header_offset: 0,
            is_directory: false,
            is_symlink: false,
            is_stored: false,
            mtime: None,
            unix_mtime: None,
            unix_permissions: None,
        }
    }

    #[tokio::test]
    async fn test_entry_mtime_epoch() {
        let mut buf = Vec::new();
        let mut zip = ZipWriter::new(&mut buf);
        let mut entry = zip.append_file("epoch.txt").await.unwrap();
        entry.set_mtime(std::time::SystemTime::UNIX_EPOCH);
        entry.write_all(b"test").await.unwrap();
        entry.close().await.unwrap();
        zip.finalize().await.unwrap();

        // Find first CD entry
        let pos = buf.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
        let cd = &buf[pos..];

        // CD entry offsets: 4=version_made_by, 6=version_needed, 8=flags,
        // 10=method, 12=time, 14=date, 16=crc32, 20=compressed_size,
        // 24=uncompressed_size, 28=name_len, 30=extra_len, 38=external_attrs
        let time = u16::from_le_bytes(cd[12..14].try_into().unwrap());
        let date = u16::from_le_bytes(cd[14..16].try_into().unwrap());
        // Unix epoch (1970-01-01) converted to local time, year clamped to 1980
        let local_offset = time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC);
        let local_epoch =
            time::OffsetDateTime::from(std::time::SystemTime::UNIX_EPOCH).to_offset(local_offset);
        let expected_time = (local_epoch.hour() as u16) << 11
            | (local_epoch.minute() as u16) << 5
            | (local_epoch.second() as u16 / 2);
        assert_eq!(time, expected_time, "expected local time for epoch");
        assert_eq!(
            date,
            ((1980 - 1980) << 9) | (1 << 5) | 1,
            "expected MS-DOS date for 1980-01-01"
        );
    }

    #[tokio::test]
    async fn test_entry_permissions() {
        let mut buf = Vec::new();
        let mut zip = ZipWriter::new(&mut buf);
        let mut entry = zip.append_file("perm_test.txt").await.unwrap();
        entry.set_permissions(0o644);
        entry.write_all(b"test").await.unwrap();
        entry.close().await.unwrap();
        zip.finalize().await.unwrap();

        let pos = buf.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
        let cd = &buf[pos..];
        let efa = u32::from_le_bytes(cd[38..42].try_into().unwrap());
        assert_eq!(efa, ((0o644 | 0o100000) as u32) << 16);
        let vmb = u16::from_le_bytes(cd[4..6].try_into().unwrap());
        assert!(vmb >> 8 == 3, "expected Unix host OS");
    }

    #[tokio::test]
    async fn test_entry_setuid_permissions() {
        // setuid (0o4000) should be preserved, not stripped by 0o777 masking
        let mut buf = Vec::new();
        let mut zip = ZipWriter::new(&mut buf);
        let mut entry = zip.append_file("setuid_test.txt").await.unwrap();
        entry.set_permissions(0o4755);
        entry.write_all(b"test").await.unwrap();
        entry.close().await.unwrap();
        zip.finalize().await.unwrap();

        let pos = buf.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
        let cd = &buf[pos..];
        let efa = u32::from_le_bytes(cd[38..42].try_into().unwrap());
        // (0o4755 | S_IFREG) << 16
        assert_eq!(efa, ((0o4755 | 0o100000) as u32) << 16);
    }

    #[tokio::test]
    async fn test_entry_mtime_and_permissions() {
        let mut buf = Vec::new();
        let mut zip = ZipWriter::new(&mut buf);
        let mut entry = zip.append_file("both.txt").await.unwrap();
        entry.set_mtime(std::time::SystemTime::UNIX_EPOCH);
        entry.set_permissions(0o755);
        entry.write_all(b"test").await.unwrap();
        entry.close().await.unwrap();
        zip.finalize().await.unwrap();

        let pos = buf.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
        let cd = &buf[pos..];
        // CD time at offset 12
        let time = u16::from_le_bytes(cd[12..14].try_into().unwrap());
        let local_offset = time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC);
        let local_epoch =
            time::OffsetDateTime::from(std::time::SystemTime::UNIX_EPOCH).to_offset(local_offset);
        let expected_time = (local_epoch.hour() as u16) << 11
            | (local_epoch.minute() as u16) << 5
            | (local_epoch.second() as u16 / 2);
        assert_eq!(time, expected_time);
        // external_file_attributes at offset 38
        let efa = u32::from_le_bytes(cd[38..42].try_into().unwrap());
        assert_eq!(efa, ((0o755 | 0o100000) as u32) << 16);
        // version_made_by at offset 4
        let vmb = u16::from_le_bytes(cd[4..6].try_into().unwrap());
        assert!(
            vmb >> 8 == 3,
            "expected version_made_by upper byte = 3 (Unix), got {}",
            vmb >> 8
        );
    }

    #[tokio::test]
    async fn test_directory_version_needed() {
        // Bug 2 regression: CD version_needed for directory entries must be 10 (STORED), not 20
        let mut buf = Vec::new();
        let mut zip = ZipWriter::new(&mut buf);
        zip.append_directory("mydir/").await.unwrap();
        zip.finalize().await.unwrap();

        let pos = buf.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
        let cd = &buf[pos..];
        let version_needed = u16::from_le_bytes(cd[6..8].try_into().unwrap());
        assert_eq!(
            version_needed, 10,
            "directory CD version_needed should be 10 (STORED), got {version_needed}"
        );
    }

    #[tokio::test]
    async fn test_entry_mtime_appears_in_cd_extra() {
        // Bug 1 integration: verify extended timestamp extra field appears in CD entry
        let mut buf = Vec::new();
        let mut zip = ZipWriter::new(&mut buf);
        let mut entry = zip.append_file("mtime_test.txt").await.unwrap();
        entry.set_mtime(std::time::SystemTime::UNIX_EPOCH);
        entry.write_all(b"hello").await.unwrap();
        entry.close().await.unwrap();
        zip.finalize().await.unwrap();

        let pos = buf.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
        let cd = &buf[pos..];
        let name_len = u16::from_le_bytes(cd[28..30].try_into().unwrap()) as usize;
        let extra_len = u16::from_le_bytes(cd[30..32].try_into().unwrap()) as usize;

        // Extra field should contain the extended timestamp header 0x5455
        let extra_start = 46 + name_len;
        let extra = &cd[extra_start..extra_start + extra_len];
        let has_ts_extra = extra.windows(2).any(|w| w == b"UT");
        assert!(
            has_ts_extra,
            "CD entry extra should contain extended timestamp (0x5455/UT) when mtime is set"
        );
        assert!(
            extra_len >= 4,
            "extra_len should be >= 4 when mtime is set, got {extra_len}"
        );
        // version_made_by should indicate Unix
        let vmb = u16::from_le_bytes(cd[4..6].try_into().unwrap());
        assert_eq!(vmb >> 8, 3, "expected Unix host OS when mtime is set");
    }

    #[tokio::test]
    async fn test_entry_default_no_metadata() {
        let mut buf = Vec::new();
        let mut zip = ZipWriter::new(&mut buf);
        let mut entry = zip.append_file("default.txt").await.unwrap();
        entry.write_all(b"test").await.unwrap();
        entry.close().await.unwrap();
        zip.finalize().await.unwrap();

        let pos = buf.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
        let cd = &buf[pos..];
        let efa = u32::from_le_bytes(cd[38..42].try_into().unwrap());
        assert_eq!(efa, 0);
        let vmb = u16::from_le_bytes(cd[4..6].try_into().unwrap());
        assert_eq!(vmb, super::header::VERSION_DEFLATE);
    }

    #[tokio::test]
    async fn test_entry_drop_poisons_zip_writer() {
        let mut buf = Vec::new();
        let mut zip = ZipWriter::new(&mut buf);

        // Drop EntryWriter without calling close() — should poison the ZipWriter
        drop(zip.append_file("lost.txt").await.unwrap());

        // append_file should fail with "archive corrupted" error
        let result = zip.append_file("another.txt").await;
        assert!(result.is_err(), "expected Err, got Ok");
        let err = result.err().unwrap();
        assert!(
            err.to_string().contains("archive corrupted"),
            "expected 'archive corrupted', got: {err}"
        );
    }

    #[tokio::test]
    async fn test_stored_entry_level_zero() {
        // CompressionLevel::NONE should produce method=0 (stored), not deflate at level 0
        let mut buf = Vec::new();
        let mut zip = ZipWriter::new(&mut buf).with_compression_level(CompressionLevel::NONE);

        let data = b"Hello, stored entry!";
        let mut entry = zip.append_file("stored.txt").await.unwrap();
        entry.write_all(data).await.unwrap();
        entry.close().await.unwrap();
        zip.finalize().await.unwrap();

        // Find CD entry and verify method=0 (STORED)
        let pos = buf.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
        let cd = &buf[pos..];
        let method = u16::from_le_bytes(cd[10..12].try_into().unwrap());
        assert_eq!(method, 0, "expected METHOD_STORED for level=0 entry");
        let version_needed = u16::from_le_bytes(cd[6..8].try_into().unwrap());
        assert_eq!(
            version_needed, 10,
            "expected VERSION_STORED for level=0 entry"
        );

        // compressed_size == uncompressed_size (stored as-is)
        let compressed_size = u32::from_le_bytes(cd[20..24].try_into().unwrap()) as u64;
        let uncompressed_size = u32::from_le_bytes(cd[24..28].try_into().unwrap()) as u64;
        assert_eq!(
            compressed_size, uncompressed_size,
            "stored entry should have equal compressed and uncompressed sizes"
        );
        assert_eq!(compressed_size, data.len() as u64);

        // Also check LFH method
        let lfh_pos = buf.windows(4).position(|w| w == b"PK\x03\x04").unwrap();
        let lfh_method = u16::from_le_bytes(buf[lfh_pos + 8..lfh_pos + 10].try_into().unwrap());
        assert_eq!(lfh_method, 0, "LFH method should be STORED for level=0");
    }

    #[tokio::test]
    async fn test_entry_drop_poison_affects_finalize() {
        let mut buf = Vec::new();
        let mut zip = ZipWriter::new(&mut buf);

        // Drop EntryWriter without calling close()
        drop(zip.append_file("lost.txt").await.unwrap());

        // finalize should also fail with "archive corrupted" error
        let err = zip.finalize().await.unwrap_err();
        assert!(
            err.to_string().contains("archive corrupted"),
            "expected 'archive corrupted', got: {err}"
        );
    }
}
