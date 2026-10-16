use super::stored_entry::StoredEntry;
use super::zip_writer::ZipWriter;

use crate::deflate_encoder::DeflateEncoder;
use crate::error::ZipError;
use crate::zip_format;

use std::fmt;
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
            // If the writer was not handed back to the `ZipWriter`, the entry
            // was never completed (dropped or cancelled): poison the archive so
            // later operations report `Poisoned` rather than silently
            // continuing. When `finish()` succeeds it stores the writer back
            // before this runs, so a completed entry is not poisoned.
            if this.zip.inner.is_none() {
                this.zip.poisoned = true;
            }
        }
    }
}

impl<W: AsyncWrite + Unpin> fmt::Debug for EntryWriter<'_, W> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EntryWriter")
            .field("name", &self.name)
            .field("is_stored", &self.is_stored)
            .field("uncompressed_size", &self.uncompressed_size)
            .field("compressed_size", &self.compressed_size)
            .field("local_header_offset", &self.local_header_offset)
            .field("comment_len", &self.comment.as_ref().map(Vec::len))
            .field(
                "active",
                &(self.deflate_encoder.is_some() || self.passthrough.is_some()),
            )
            .finish()
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
            if let Err(e) = encoder.shutdown().await {
                // The inner writer was moved into the encoder and is dropped
                // with it, so it can no longer be recovered: poison the parent
                // so later calls report `Poisoned` rather than `WriterBusy`.
                self.zip.poisoned = true;
                return Err(ZipError::Io(e));
            }
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
        self.zip.poisoned = false;
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
        // Do NOT propagate `shutdown` to the shared archive writer for stored
        // entries: it would close the archive before the entry's data
        // descriptor and the central directory are written. `DeflateEncoder`
        // already avoids this by not forwarding shutdown to its inner writer,
        // so mirror that here by flushing the passthrough instead.
        if *this.is_stored {
            poll_active_writer!(this, cx, poll_flush)
        } else {
            poll_active_writer!(this, cx, poll_shutdown)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use crate::CompressionLevel;
    use crate::ZipError;
    use crate::test_utils::*;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::task::{Context, Poll, Waker};
    use std::time::SystemTime;
    use tokio::io::{AsyncWrite, AsyncWriteExt};

    /// A writer that stalls (returns `Pending`) while `gate` is closed, and
    /// otherwise accepts everything. Closing the gate between two operations
    /// lets a test target exactly which `.await` gets cancelled.
    #[derive(Debug)]
    struct GatedWriter {
        gate: Arc<AtomicBool>,
        data: Vec<u8>,
    }

    impl AsyncWrite for GatedWriter {
        fn poll_write(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            if !self.gate.load(Ordering::SeqCst) {
                cx.waker().wake_by_ref();
                return Poll::Pending;
            }
            self.data.extend_from_slice(buf);
            Poll::Ready(Ok(buf.len()))
        }
        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
        fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

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
    async fn test_entry_drop_poisons_zip_writer() {
        let mut buf = Vec::new();
        let mut zip = ZipWriter::new(&mut buf);

        drop(
            zip.start_file("lost.txt", &EntryOptions::file())
                .await
                .unwrap(),
        );

        let err = zip
            .start_file("another.txt", &EntryOptions::file())
            .await
            .expect_err("expected Err, got Ok");
        assert!(
            matches!(err, ZipError::Poisoned),
            "expected Poisoned, got: {err}"
        );

        let err = zip.finish().await.unwrap_err();
        assert!(
            matches!(err, ZipError::Poisoned),
            "expected Poisoned, got: {err}"
        );
    }

    /// `shutdown` on a per-entry writer must not shut down the shared archive
    /// writer, or the data descriptor / central directory would be written to
    /// an already closed writer.
    #[tokio::test]
    async fn test_stored_entry_shutdown_does_not_close_archive() {
        struct Tracker {
            shutdown_called: bool,
            writes_after_shutdown: usize,
        }

        impl AsyncWrite for Tracker {
            fn poll_write(
                mut self: Pin<&mut Self>,
                _cx: &mut Context<'_>,
                buf: &[u8],
            ) -> Poll<std::io::Result<usize>> {
                if self.shutdown_called {
                    self.writes_after_shutdown += 1;
                }
                Poll::Ready(Ok(buf.len()))
            }
            fn poll_flush(
                self: Pin<&mut Self>,
                _cx: &mut Context<'_>,
            ) -> Poll<std::io::Result<()>> {
                Poll::Ready(Ok(()))
            }
            fn poll_shutdown(
                mut self: Pin<&mut Self>,
                _cx: &mut Context<'_>,
            ) -> Poll<std::io::Result<()>> {
                self.shutdown_called = true;
                Poll::Ready(Ok(()))
            }
        }

        let mut zip = ZipWriter::new(Tracker {
            shutdown_called: false,
            writes_after_shutdown: 0,
        })
        .with_compression_level(CompressionLevel::none());
        let mut entry = zip
            .start_file("a.txt", &EntryOptions::file())
            .await
            .unwrap();
        entry.write_all(b"hello").await.unwrap();
        entry.shutdown().await.unwrap();
        entry.finish().await.unwrap();
        let tracker = zip.finish().await.unwrap();

        assert_eq!(
            tracker.writes_after_shutdown, 0,
            "data descriptor / central directory written after the archive writer was shut down"
        );
    }

    /// Poll a future once with a no-op waker and require it to stall.
    fn poll_once_stalling<F: Future>(fut: &mut Pin<Box<F>>) {
        let mut cx = Context::from_waker(Waker::noop());
        assert!(
            fut.as_mut().poll(&mut cx).is_pending(),
            "operation should stall (writer gate closed)"
        );
    }

    /// Dropping a cancelled `start_file` future loses the inner writer; the
    /// archive must report `Poisoned`, not the misleading `WriterBusy`.
    #[tokio::test]
    async fn test_cancelled_start_file_poisons_writer() {
        let gate = Arc::new(AtomicBool::new(false)); // stall on the LFH
        let mut zip = ZipWriter::new(GatedWriter {
            gate,
            data: Vec::new(),
        });

        let options = EntryOptions::file();
        let mut fut = Box::pin(zip.start_file("a.txt", &options));
        poll_once_stalling(&mut fut);
        drop(fut); // cancellation

        let err = zip.abort().unwrap_err();
        assert!(
            matches!(err, ZipError::Poisoned),
            "expected Poisoned after cancellation, got {err}"
        );
    }

    /// Dropping a cancelled `EntryWriter::finish` (stored path) must poison the
    /// archive, because `passthrough` was already taken out of the entry.
    #[tokio::test]
    async fn test_cancelled_stored_finish_poisons_writer() {
        let gate = Arc::new(AtomicBool::new(true));
        let mut zip = ZipWriter::new(GatedWriter {
            gate: gate.clone(),
            data: Vec::new(),
        })
        .with_compression_level(CompressionLevel::none());
        let mut entry = zip
            .start_file("a.txt", &EntryOptions::file())
            .await
            .unwrap();
        entry.write_all(b"hello").await.unwrap();

        gate.store(false, Ordering::SeqCst); // stall on the Data Descriptor
        let mut fut = Box::pin(entry.finish());
        poll_once_stalling(&mut fut);
        drop(fut); // cancellation

        let err = zip.finish().await.unwrap_err();
        assert!(
            matches!(err, ZipError::Poisoned),
            "expected Poisoned after cancellation, got {err}"
        );
    }

    /// Same, for the deflated path where the writer is hidden inside the
    /// deflate encoder's shutdown.
    #[tokio::test]
    async fn test_cancelled_deflate_finish_poisons_writer() {
        let gate = Arc::new(AtomicBool::new(true));
        let mut zip = ZipWriter::new(GatedWriter {
            gate: gate.clone(),
            data: Vec::new(),
        });
        let mut entry = zip
            .start_file("a.txt", &EntryOptions::file())
            .await
            .unwrap();
        entry.write_all(&vec![b'A'; 65536]).await.unwrap();

        gate.store(false, Ordering::SeqCst); // stall on the deflate drain
        let mut fut = Box::pin(entry.finish());
        poll_once_stalling(&mut fut);
        drop(fut); // cancellation

        let err = zip.finish().await.unwrap_err();
        assert!(
            matches!(err, ZipError::Poisoned),
            "expected Poisoned after cancellation, got {err}"
        );
    }
}
