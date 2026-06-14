use super::helpers::CountWriter;
use super::stored_entry::StoredEntry;
use super::zip_writer::ZipWriter;

use crate::deflate_encoder::DeflateEncoder;
use crate::error::ZipError;
use crate::header;

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::SystemTime;
use tokio::io::{AsyncWrite, AsyncWriteExt};

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
        pub(crate) zip: &'a mut ZipWriter<W>,
        #[pin]
        pub(crate) deflate_encoder: Option<DeflateEncoder<CountWriter<W>>>,
        #[pin]
        pub(crate) passthrough: Option<CountWriter<W>>,
        pub(crate) is_stored: bool,
        pub(crate) crc_hasher: crc32fast::Hasher,
        pub(crate) uncompressed_size: u64,
        pub(crate) local_header_offset: u64,
        pub(crate) name: String,
        pub(crate) mtime: SystemTime,
        pub(crate) unix_permissions: Option<u32>,
        pub(crate) uid_gid: Option<(u32, u32)>,
        pub(crate) comment: Option<Vec<u8>>,
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
    /// Returns [`ZipError`] if `close` is called more than once, if the deflate
    /// encoder fails to shut down (I/O error), or if writing the Data
    /// Descriptor fails (I/O error).
    pub async fn close(mut self) -> Result<(), ZipError> {
        let (compressed_size, mut inner) = if self.is_stored {
            let cw = self
                .passthrough
                .take()
                .ok_or_else(|| ZipError::InvalidState("entry already closed".to_string()))?;
            (cw.count, cw.inner)
        } else {
            let mut encoder = self
                .deflate_encoder
                .take()
                .ok_or_else(|| ZipError::InvalidState("entry already closed".to_string()))?;
            encoder.shutdown().await?;

            // Extract the inner writer from the encoder stack
            let count_writer: CountWriter<W> = encoder.into_inner();
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
        inner.write_all(&dd_bytes).await.map_err(|e| {
            self.zip.poisoned = true;
            ZipError::Io(e)
        })?;

        // Update position tracker: compressed data + data descriptor
        self.zip.pos += compressed_size + dd_bytes.len() as u64;

        let (mtime_msdos, unix_mtime) = header::mtime_to_ms_dos_and_unix(self.mtime);

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
            uid_gid: self.uid_gid,
            comment: self.comment.clone(),
        });

        // Return the inner writer to ZipWriter
        self.zip.inner = Some(inner);
        Ok(())
    }
}

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
                None => {
                    this.zip.poisoned = true;
                    return Poll::Ready(Err(ZipError::Poisoned(
                        "write after entry closed".to_string(),
                    )
                    .into()));
                }
            }
        } else {
            match this.deflate_encoder.as_pin_mut() {
                Some(e) => e.poll_write(cx, buf),
                None => {
                    this.zip.poisoned = true;
                    return Poll::Ready(Err(ZipError::Poisoned(
                        "write after entry closed".to_string(),
                    )
                    .into()));
                }
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
                None => {
                    this.zip.poisoned = true;
                    Poll::Ready(Err(ZipError::Poisoned(
                        "flush after entry closed".to_string(),
                    )
                    .into()))
                }
            }
        } else {
            match this.deflate_encoder.as_pin_mut() {
                Some(e) => e.poll_flush(cx),
                None => {
                    this.zip.poisoned = true;
                    Poll::Ready(Err(ZipError::Poisoned(
                        "flush after entry closed".to_string(),
                    )
                    .into()))
                }
            }
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.project();
        if *this.is_stored {
            match this.passthrough.as_pin_mut() {
                Some(w) => w.poll_shutdown(cx),
                None => {
                    this.zip.poisoned = true;
                    Poll::Ready(Err(ZipError::Poisoned(
                        "shutdown after entry closed".to_string(),
                    )
                    .into()))
                }
            }
        } else {
            match this.deflate_encoder.as_pin_mut() {
                Some(e) => e.poll_shutdown(cx),
                None => {
                    this.zip.poisoned = true;
                    Poll::Ready(Err(ZipError::Poisoned(
                        "shutdown after entry closed".to_string(),
                    )
                    .into()))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use std::time::SystemTime;
    use tokio::io::AsyncWriteExt;

    fn opts_mtime(t: SystemTime) -> WriterOptions {
        WriterOptions {
            mtime: t,
            permissions: None,
            uid_gid: None,
            comment: None,
        }
    }

    fn opts_perms(mode: u32) -> WriterOptions {
        WriterOptions {
            mtime: SystemTime::now(),
            permissions: Some(mode),
            uid_gid: None,
            comment: None,
        }
    }

    #[tokio::test]
    async fn test_entry_mtime_epoch() {
        let mut buf = Vec::new();
        let mut zip = ZipWriter::new(&mut buf);
        let mut entry = zip
            .append_file("epoch.txt", opts_mtime(SystemTime::UNIX_EPOCH))
            .await
            .unwrap();
        entry.write_all(b"test").await.unwrap();
        entry.close().await.unwrap();
        zip.finalize().await.unwrap();

        let pos = buf.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
        let cd = &buf[pos..];

        let time = u16::from_le_bytes(cd[12..14].try_into().unwrap());
        let date = u16::from_le_bytes(cd[14..16].try_into().unwrap());
        let local_offset = time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC);
        let local_epoch =
            time::OffsetDateTime::from(SystemTime::UNIX_EPOCH).to_offset(local_offset);
        let expected_time = (local_epoch.hour() as u16) << 11
            | (local_epoch.minute() as u16) << 5
            | (local_epoch.second() as u16).div_ceil(2);
        assert_eq!(time, expected_time, "expected local time for epoch");
        assert_eq!(date, (1 << 5) | 1, "expected MS-DOS date for 1980-01-01");
    }

    #[tokio::test]
    async fn test_entry_permissions() {
        let mut buf = Vec::new();
        let mut zip = ZipWriter::new(&mut buf);
        let mut entry = zip
            .append_file("perm_test.txt", opts_perms(0o644))
            .await
            .unwrap();
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
        let mut buf = Vec::new();
        let mut zip = ZipWriter::new(&mut buf);
        let mut entry = zip
            .append_file("setuid_test.txt", opts_perms(0o4755))
            .await
            .unwrap();
        entry.write_all(b"test").await.unwrap();
        entry.close().await.unwrap();
        zip.finalize().await.unwrap();

        let pos = buf.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
        let cd = &buf[pos..];
        let efa = u32::from_le_bytes(cd[38..42].try_into().unwrap());
        assert_eq!(efa, ((0o4755 | 0o100000) as u32) << 16);
    }

    #[tokio::test]
    async fn test_entry_mtime_appears_in_cd_extra() {
        let mut buf = Vec::new();
        let mut zip = ZipWriter::new(&mut buf);
        let mut entry = zip
            .append_file("mtime_test.txt", opts_mtime(SystemTime::now()))
            .await
            .unwrap();
        entry.write_all(b"hello").await.unwrap();
        entry.close().await.unwrap();
        zip.finalize().await.unwrap();

        let pos = buf.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
        let cd = &buf[pos..];
        let name_len = u16::from_le_bytes(cd[28..30].try_into().unwrap()) as usize;
        let extra_len = u16::from_le_bytes(cd[30..32].try_into().unwrap()) as usize;

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
        let vmb = u16::from_le_bytes(cd[4..6].try_into().unwrap());
        assert_eq!(vmb >> 8, 3, "expected Unix host OS when mtime is set");
    }

    #[tokio::test]
    async fn test_entry_drop_poisons_zip_writer() {
        let mut buf = Vec::new();
        let mut zip = ZipWriter::new(&mut buf);

        drop(
            zip.append_file("lost.txt", WriterOptions::file())
                .await
                .unwrap(),
        );

        let result = zip.append_file("another.txt", WriterOptions::file()).await;
        assert!(result.is_err(), "expected Err, got Ok");
        let err = result.err().unwrap();
        assert!(
            err.to_string().contains("archive corrupted"),
            "expected 'archive corrupted', got: {err}"
        );
    }

    #[tokio::test]
    async fn test_entry_drop_poison_affects_finalize() {
        let mut buf = Vec::new();
        let mut zip = ZipWriter::new(&mut buf);

        drop(
            zip.append_file("lost.txt", WriterOptions::file())
                .await
                .unwrap(),
        );

        let err = zip.finalize().await.unwrap_err();
        assert!(
            err.to_string().contains("archive corrupted"),
            "expected 'archive corrupted', got: {err}"
        );
    }
}
