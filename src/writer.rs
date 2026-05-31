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
    mtime: Option<(u16, u16)>,
    unix_mtime: Option<u64>,
    unix_permissions: Option<u32>,
}

impl StoredEntry {
    fn to_central_dir_entry(&self) -> header::CentralDirEntry {
        let (time, date) = self.mtime.unwrap_or_else(header::ms_dos_datetime);

        let has_unix_attrs = self.unix_permissions.is_some() || self.unix_mtime.is_some();
        let version_made_by = if has_unix_attrs {
            header::VERSION_UNIX
        } else {
            header::VERSION_DEFLATE
        };

        let extra = match self.unix_mtime {
            Some(ts) => header::build_extended_timestamp_extra(ts),
            None => Vec::new(),
        };

        let file_type_bit: u32 = if self.is_directory {
            0o040000
        } else {
            0o100000
        };
        let external_file_attributes = self
            .unix_permissions
            .map(|mode| (mode | file_type_bit) << 16)
            .unwrap_or(0);

        header::CentralDirEntry {
            version_made_by,
            version_needed: header::VERSION_DEFLATE,
            flags: header::FLAG_DATA_DESC,
            method: if self.is_directory {
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
        let mut inner = self.inner.take().unwrap();
        let lfh = header::LocalFileHeader::new(name, header::METHOD_DEFLATE);
        let lfh_bytes = lfh.serialize();
        inner.write_all(&lfh_bytes).await?;
        let offset = self.pos;
        self.pos += lfh_bytes.len() as u64;

        let encoder = DeflateEncoder::with_quality(
            ShutdownIgnoredWriter(CountWriter { inner, count: 0 }),
            async_compression::Level::Precise(self.level as i32),
        );

        Ok(EntryWriter {
            zip: self,
            encoder: Some(encoder),
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
        let mut inner = self.inner.take().unwrap();
        let lfh = header::LocalFileHeader::new(name, header::METHOD_STORED);
        let lfh_bytes = lfh.serialize();
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
    /// # Panics
    ///
    /// Panics if called when the inner writer has not been returned
    /// (i.e., an `EntryWriter` is still active and hasn't been closed).
    pub async fn finalize(mut self) -> io::Result<()> {
        let mut inner = self.inner.expect("zip already finalized");
        let cd_offset = self.pos;

        for entry in &self.entries {
            let cd_entry = entry.to_central_dir_entry();
            let data = cd_entry.serialize();
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
        encoder: Option<DeflateEncoder<ShutdownIgnoredWriter<CountWriter<W>>>>,
        crc_hasher: crc32fast::Hasher,
        uncompressed_size: u64,
        local_header_offset: u64,
        name: String,
        mtime: Option<std::time::SystemTime>,
        unix_permissions: Option<u32>,
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
    /// Provide permission bits only (e.g., `0o644`, `0o755`). The crate
    /// automatically adds the file type bit (`S_IFREG` for files,
    /// `S_IFDIR` for directories).
    pub fn set_permissions(&mut self, mode: u32) -> &mut Self {
        self.unix_permissions = Some(mode & 0o777);
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
    /// # Panics
    ///
    /// Panics if `close` is called more than once on the same entry.
    pub async fn close(mut self) -> io::Result<()> {
        let mut encoder = self.encoder.take().unwrap();
        encoder.shutdown().await?;

        // Extract the inner writer from the encoder stack
        let shutdown_writer: ShutdownIgnoredWriter<CountWriter<W>> = encoder.into_inner();
        let count_writer: CountWriter<W> = shutdown_writer.0;
        let compressed_size = count_writer.count;
        let mut inner: W = count_writer.inner;

        let crc32 = self.crc_hasher.clone().finalize();

        let dd = header::DataDescriptor {
            crc32,
            compressed_size,
            uncompressed_size: self.uncompressed_size,
            zip64: compressed_size > header::U32_MAX || self.uncompressed_size > header::U32_MAX,
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
        match this.encoder.as_pin_mut().unwrap().poll_write(cx, buf) {
            Poll::Ready(Ok(n)) => {
                this.crc_hasher.update(&buf[..n]);
                *this.uncompressed_size += n as u64;
                Poll::Ready(Ok(n))
            }
            other => other,
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.project().encoder.as_pin_mut().unwrap().poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.project()
            .encoder
            .as_pin_mut()
            .unwrap()
            .poll_shutdown(cx)
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
        // Unix epoch (1970-01-01) clamped to MS-DOS range: 1980-01-01 00:00:00
        assert_eq!(time, 0, "expected midnight");
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
        assert_eq!(time, 0);
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
}
