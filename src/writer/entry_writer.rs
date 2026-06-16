use super::stored_entry::StoredEntry;
use super::zip_writer::ZipWriter;
use crate::count_writer::CountWriter;

use crate::deflate_encoder::DeflateEncoder;
use crate::error::ZipError;
use crate::zip_format;

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::SystemTime;
use tokio::io::{AsyncWrite, AsyncWriteExt};

/// Dispatch an `AsyncWrite` method to the active writer (stored or deflated).
/// Returns `EntryWriterCorrupted` error if the entry has already been closed.
macro_rules! poll_active_writer {
    ($this:expr, $cx:expr, $method:ident $(, $args:expr)*) => {{
        let writer: Option<Pin<&mut dyn AsyncWrite>> = if *$this.is_stored {
            $this.passthrough
                .as_pin_mut()
                .map(|w| w as Pin<&mut dyn AsyncWrite>)
        } else {
            $this.deflate_encoder
                .as_pin_mut()
                .map(|e| e as Pin<&mut dyn AsyncWrite>)
        };
        match writer {
            Some(mut w) => w.as_mut().$method($cx $(, $args)*),
            None => {
                $this.zip.poisoned = true;
                Poll::Ready(Err(io::Error::other(ZipError::EntryWriterCorrupted)))
            }
        }
    }};
}

pin_project_lite::pin_project! {
    /// A streaming writer for a single file entry in a ZIP archive.
    ///
    /// Obtained from [`ZipWriter::start_file`]. Data written through this
    /// writer is compressed with DEFLATE and streamed to the underlying output.
    ///
    /// # Important
    ///
    /// The [`finish`](EntryWriter::finish) method **must** be called after all
    /// data is written to finish the deflate frame and write the Data
    /// Descriptor. Dropping without finishing will lose the entry.
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
                // finish() was never called — mark the ZipWriter as poisoned
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
    /// Returns [`ZipError`] if `finish` is called more than once, if the deflate
    /// encoder fails to shut down (I/O error), or if writing the Data
    /// Descriptor fails (I/O error).
    pub async fn finish(mut self) -> Result<(), ZipError> {
        let (compressed_size, mut inner) = if self.is_stored {
            let cw = self
                .passthrough
                .take()
                .ok_or(ZipError::EntryWriterCorrupted)?;
            (cw.count, cw.inner)
        } else {
            let mut encoder = self
                .deflate_encoder
                .take()
                .ok_or(ZipError::EntryWriterCorrupted)?;
            encoder.shutdown().await?;

            // Extract the inner writer from the encoder stack
            let count_writer: CountWriter<W> = encoder.into_inner();
            let compressed_size = count_writer.count;
            (compressed_size, count_writer.inner)
        };

        let crc32 = self.crc_hasher.clone().finalize();

        let dd = zip_format::DataDescriptor {
            crc32,
            compressed_size,
            uncompressed_size: self.uncompressed_size,
            // Use ZIP64 DD when any entry field exceeds 32 bits, consistent with
            // CentralDirEntry::serialize() which also checks local_header_offset.
            zip64: compressed_size > zip_format::U32_MAX
                || self.uncompressed_size > zip_format::U32_MAX
                || self.local_header_offset > zip_format::U32_MAX,
        };
        let dd_bytes = dd.serialize();
        inner.write_all(&dd_bytes).await.map_err(|e| {
            self.zip.poisoned = true;
            ZipError::Io(e)
        })?;

        // Update position tracker: compressed data + data descriptor
        self.zip.pos += compressed_size + dd_bytes.len() as u64;

        let (mtime_msdos, unix_mtime) = zip_format::mtime_to_ms_dos_and_unix(self.mtime);

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
        let result = poll_active_writer!(this, cx, poll_write, buf);
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
        poll_active_writer!(this, cx, poll_flush)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.project();
        poll_active_writer!(this, cx, poll_shutdown)
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use crate::ZipError;
    use std::time::SystemTime;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn test_entry_mtime_epoch() {
        let mut buf = Vec::new();
        let mut zip = ZipWriter::new(&mut buf);
        let mut entry = zip
            .start_file(
                "epoch.txt",
                EntryOptions::file()
                    .with_mtime(SystemTime::UNIX_EPOCH)
                    .with_unix_permissions(0o644),
            )
            .await
            .unwrap();
        entry.write_all(b"test").await.unwrap();
        entry.finish().await.unwrap();
        zip.finish().await.unwrap();

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
        for (mode, name) in [(0o644, "perm_test.txt"), (0o4755, "setuid_test.txt")] {
            let mut buf = Vec::new();
            let mut zip = ZipWriter::new(&mut buf);
            let mut entry = zip
                .start_file(name, EntryOptions::file().with_unix_permissions(mode))
                .await
                .unwrap();
            entry.write_all(b"test").await.unwrap();
            entry.finish().await.unwrap();
            zip.finish().await.unwrap();
            let pos = buf.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
            let cd = &buf[pos..];
            let efa = u32::from_le_bytes(cd[38..42].try_into().unwrap());
            assert_eq!(efa, (mode | 0o100000) << 16, "mode {mode:04o}");
            let vmb = u16::from_le_bytes(cd[4..6].try_into().unwrap());
            assert!(vmb >> 8 == 3, "expected Unix host OS for mode {mode:04o}");
        }
    }

    #[tokio::test]
    async fn test_entry_mtime_appears_in_cd_extra() {
        let mut buf = Vec::new();
        let mut zip = ZipWriter::new(&mut buf);
        let mut entry = zip
            .start_file("mtime_test.txt", EntryOptions::file())
            .await
            .unwrap();
        entry.write_all(b"hello").await.unwrap();
        entry.finish().await.unwrap();
        zip.finish().await.unwrap();

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
            zip.start_file("lost.txt", EntryOptions::file())
                .await
                .unwrap(),
        );

        let result = zip.start_file("another.txt", EntryOptions::file()).await;
        assert!(result.is_err(), "expected Err, got Ok");
        let err = result.err().unwrap();
        assert!(
            matches!(err, ZipError::EntryWriterCorrupted),
            "expected EntryWriterCorrupted, got: {err}"
        );
    }

    #[tokio::test]
    async fn test_entry_drop_poison_affects_finish() {
        let mut buf = Vec::new();
        let mut zip = ZipWriter::new(&mut buf);

        drop(
            zip.start_file("lost.txt", EntryOptions::file())
                .await
                .unwrap(),
        );

        let err = zip.finish().await.unwrap_err();
        assert!(
            matches!(err, ZipError::EntryWriterCorrupted),
            "expected EntryWriterCorrupted, got: {err}"
        );
    }
}
