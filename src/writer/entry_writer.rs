use super::stored_entry::StoredEntry;
use super::zip_writer::ZipWriter;

use crate::deflate_encoder::DeflateEncoder;
use crate::error::ZipError;
use crate::zip_format;

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::SystemTime;
use tokio::io::{AsyncWrite, AsyncWriteExt};

/// Dispatch an `AsyncWrite` method to the active writer (stored or deflated).
/// Returns `Poisoned` if the entry writer has already been finished.
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
                Poll::Ready(Err(io::Error::other(ZipError::Poisoned)))
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
        pub(crate) deflate_encoder: Option<DeflateEncoder<W>>,
        #[pin]
        pub(crate) passthrough: Option<W>,
        pub(crate) is_stored: bool,
        pub(crate) crc_hasher: crc32fast::Hasher,
        pub(crate) uncompressed_size: u64,
        pub(crate) compressed_size: u64,
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
        let mut inner = if self.is_stored {
            self.compressed_size = self.uncompressed_size;
            self.passthrough.take().ok_or(ZipError::Poisoned)?
        } else {
            let mut encoder = self.deflate_encoder.take().ok_or(ZipError::Poisoned)?;
            encoder.shutdown().await?;
            self.compressed_size = encoder.written();
            encoder.into_inner()
        };

        let crc32 = self.crc_hasher.clone().finalize();

        let dd = zip_format::DataDescriptor {
            crc32,
            compressed_size: self.compressed_size,
            uncompressed_size: self.uncompressed_size,
            zip64: zip_format::data_descriptor_needs_zip64(
                self.compressed_size,
                self.uncompressed_size,
            ),
        };
        self.zip.scratch.clear();
        dd.write_to(&mut self.zip.scratch);
        inner.write_all(&self.zip.scratch).await.map_err(|e| {
            self.zip.poisoned = true;
            ZipError::Io(e)
        })?;

        // Update position tracker: compressed data + data descriptor
        self.zip.pos += self.compressed_size + self.zip.scratch.len() as u64;

        let unix_mtime = zip_format::system_time_to_unix_secs(self.mtime);

        self.zip.entries.push(StoredEntry {
            name: self.name.clone(),
            crc32,
            compressed_size: self.compressed_size,
            uncompressed_size: self.uncompressed_size,
            local_header_offset: self.local_header_offset,
            is_directory: false,
            is_symlink: false,
            is_stored: self.is_stored,
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
    use crate::test_utils::*;
    use std::time::SystemTime;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn test_entry_mtime_epoch() {
        let mut buf = Vec::new();
        let mut zip = ZipWriter::new(&mut buf);
        let mut entry = zip
            .start_file(
                "epoch.txt",
                &EntryOptions::file()
                    .with_mtime(SystemTime::UNIX_EPOCH)
                    .with_unix_permissions(0o644),
            )
            .await
            .unwrap();
        entry.write_all(b"test").await.unwrap();
        entry.finish().await.unwrap();
        zip.finish().await.unwrap();

        let local_epoch =
            time::OffsetDateTime::from(SystemTime::UNIX_EPOCH).to_offset(time::UtcOffset::UTC);

        assert_last_modified(
            &buf,
            "epoch.txt",
            (
                1980,
                1,
                1,
                local_epoch.hour(),
                local_epoch.minute(),
                local_epoch.second().div_ceil(2) * 2,
            ),
        );
    }

    #[tokio::test]
    async fn test_entry_permissions() {
        for (mode, name) in [(0o644, "perm_test.txt"), (0o4755, "setuid_test.txt")] {
            let mut buf = Vec::new();
            let mut zip = ZipWriter::new(&mut buf);
            let mut entry = zip
                .start_file(name, &EntryOptions::file().with_unix_permissions(mode))
                .await
                .unwrap();
            entry.write_all(b"test").await.unwrap();
            entry.finish().await.unwrap();
            zip.finish().await.unwrap();
            assert_unix_mode(&buf, name, mode | 0o100000);
        }
    }

    #[tokio::test]
    async fn test_entry_mtime_appears_in_cd_extra() {
        let mut buf = Vec::new();
        let mut zip = ZipWriter::new(&mut buf);
        let mut entry = zip
            .start_file("mtime_test.txt", &EntryOptions::file())
            .await
            .unwrap();
        entry.write_all(b"hello").await.unwrap();
        entry.finish().await.unwrap();
        zip.finish().await.unwrap();

        assert_extra_has_tag(&buf, "mtime_test.txt", b"UT");
    }

    #[tokio::test]
    async fn test_entry_drop_poisons_zip_writer() {
        let mut buf = Vec::new();
        let mut zip = ZipWriter::new(&mut buf);

        drop(
            zip.start_file("lost.txt", &EntryOptions::file())
                .await
                .unwrap(),
        );

        let result = zip.start_file("another.txt", &EntryOptions::file()).await;
        assert!(result.is_err(), "expected Err, got Ok");
        let err = result.err().unwrap();
        assert!(
            matches!(err, ZipError::Poisoned),
            "expected Poisoned, got: {err}"
        );
    }

    #[tokio::test]
    async fn test_entry_drop_poison_affects_finish() {
        let mut buf = Vec::new();
        let mut zip = ZipWriter::new(&mut buf);

        drop(
            zip.start_file("lost.txt", &EntryOptions::file())
                .await
                .unwrap(),
        );

        let err = zip.finish().await.unwrap_err();
        assert!(
            matches!(err, ZipError::Poisoned),
            "expected Poisoned, got: {err}"
        );
    }
}
