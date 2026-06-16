use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::CompressionLevel;
use flate2::{Compress, FlushCompress, Status};
use tokio::io::AsyncWrite;

/// Async-only Deflate encoder implementing `tokio::io::AsyncWrite`.
///
/// Wraps an inner `AsyncWrite` and compresses data written through it
/// using the DEFLATE algorithm (raw deflate, no zlib header/trailer).
/// Used internally by `ZipWriter` to compress ZIP entry data.
///
/// # Shutdown behavior
///
/// `poll_shutdown` finalizes the deflate stream and drains any remaining
/// compressed output to the inner writer, but does NOT propagate shutdown
/// to the inner writer. This allows the caller (EntryWriter::close) to
/// write a ZIP Data Descriptor after the compressed data stream.
pub(crate) struct DeflateEncoder<W: AsyncWrite + Unpin> {
    inner: W,
    compress: Compress,
    out_buf: Vec<u8>,
    /// Number of bytes from `out_buf[0..out_len]` already written to inner.
    out_pos: usize,
    /// Number of valid compressed bytes in `out_buf`.
    out_len: usize,
    finished: bool,
}

impl<W: AsyncWrite + Unpin> DeflateEncoder<W> {
    /// Create a new `DeflateEncoder` wrapping `inner` with the given compression level.
    pub(crate) fn new(inner: W, level: CompressionLevel) -> Self {
        Self {
            inner,
            // false = raw deflate (no zlib header/trailer)
            compress: Compress::new(level, false),
            out_buf: vec![0u8; 8192],
            out_pos: 0,
            out_len: 0,
            finished: false,
        }
    }

    /// Get a shared reference to the inner writer.
    #[allow(dead_code)]
    pub(crate) fn get_ref(&self) -> &W {
        &self.inner
    }

    /// Consume the encoder and return the inner writer.
    pub(crate) fn into_inner(self) -> W {
        self.inner
    }

    /// Drain pending data in `out_buf[out_pos..out_len]` to the inner writer.
    ///
    /// Returns `Poll::Ready(Ok(()))` once all buffered output has been written,
    /// or `Poll::Pending` if the inner writer cannot accept more data.
    fn poll_drain(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        while self.out_pos < self.out_len {
            let this = self.as_mut().get_mut();
            match Pin::new(&mut this.inner)
                .poll_write(cx, &this.out_buf[this.out_pos..this.out_len])
            {
                Poll::Ready(Ok(0)) => {
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "underlying writer returned 0 bytes",
                    )));
                }
                Poll::Ready(Ok(n)) => this.out_pos += n,
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Pending => return Poll::Pending,
            }
        }
        let this = self.as_mut().get_mut();
        this.out_pos = 0;
        this.out_len = 0;
        Poll::Ready(Ok(()))
    }

    /// Compress `input` into the internal output buffer with the given flush mode.
    ///
    /// Returns the number of bytes consumed from `input` and the flate2 `Status`.
    fn do_compress(&mut self, input: &[u8], flush: FlushCompress) -> io::Result<(usize, Status)> {
        let before_in = self.compress.total_in();
        let before_out = self.compress.total_out();

        let status = self
            .compress
            .compress(input, &mut self.out_buf, flush)
            .map_err(io::Error::other)?;

        let consumed = (self.compress.total_in() - before_in) as usize;
        let produced = (self.compress.total_out() - before_out) as usize;

        // Mark the newly produced bytes for subsequent drain.
        self.out_pos = 0;
        self.out_len = produced;

        if matches!(status, Status::BufError) && consumed == 0 && produced == 0 {
            return Err(io::Error::other("flate2 BufError with no progress"));
        }

        Ok((consumed, status))
    }
}

impl<W: AsyncWrite + Unpin> AsyncWrite for DeflateEncoder<W> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.finished {
            return if buf.is_empty() {
                Poll::Ready(Ok(0))
            } else {
                Poll::Ready(Err(io::Error::other("write after shutdown")))
            };
        }

        if buf.is_empty() {
            return Poll::Ready(Ok(0));
        }

        // Drain any remaining output from a previous call first
        if self.out_pos < self.out_len {
            match self.as_mut().poll_drain(cx) {
                Poll::Ready(Ok(())) => {}
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Pending => return Poll::Pending,
            }
        }

        // Compress and drain until we consume at least some input or would block.
        // flate2 may buffer output internally (especially at low compression
        // levels) and flush it in large batches.  When the output buffer fills
        // up, do_compress can return consumed=0 — we must drain and retry
        // rather than returning Ok(0) (which tokio::io::copy treats as
        // WriteZero).
        loop {
            let (consumed, produced) = {
                let this = self.as_mut().get_mut();
                let (consumed, _status) = this.do_compress(buf, FlushCompress::None)?;
                let produced = this.out_len;
                (consumed, produced)
            };

            // Drain the compressed output (if any) to the inner writer.
            // If the inner writer would block, surface Pending so the caller
            // can retry later.
            if self.out_pos < self.out_len {
                match self.as_mut().poll_drain(cx) {
                    Poll::Ready(Ok(())) => {}
                    Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                    Poll::Pending => return Poll::Pending,
                }
            }

            if consumed > 0 {
                return Poll::Ready(Ok(consumed));
            }

            // consumed == 0 and produced == 0 means no progress possible —
            // return 0 to signal EOF-like condition (shouldn't happen during
            // normal operation unless the compressor is finished).
            if produced == 0 {
                return Poll::Ready(Ok(0));
            }

            // consumed == 0 but produced > 0: the output buffer was full.
            // We already drained it above; loop back and try again.
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.finished {
            return Poll::Ready(Ok(()));
        }

        // Drain existing buffered output
        if self.out_pos < self.out_len {
            match self.as_mut().poll_drain(cx) {
                Poll::Ready(Ok(())) => {}
                other => return other,
            }
        }

        // Flush compressor internal state
        loop {
            let this = self.as_mut().get_mut();
            let (_, status) = this.do_compress(&[], FlushCompress::Sync)?;

            if this.out_pos < this.out_len {
                match self.as_mut().poll_drain(cx) {
                    Poll::Ready(Ok(())) => {}
                    other => return other,
                }
            }

            match status {
                Status::Ok | Status::StreamEnd => break,
                Status::BufError => continue,
            }
        }

        // Do NOT propagate flush to inner writer — the caller manages
        // flush of the inner writer (e.g., ZipWriter finalize).
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if !self.finished {
            // Drain existing buffered output
            if self.out_pos < self.out_len {
                match self.as_mut().poll_drain(cx) {
                    Poll::Ready(Ok(())) => {}
                    other => return other,
                }
            }

            // Finish deflate stream (write end-of-stream markers)
            loop {
                let this = self.as_mut().get_mut();
                let (_, status) = this.do_compress(&[], FlushCompress::Finish)?;

                if this.out_pos < this.out_len {
                    match self.as_mut().poll_drain(cx) {
                        Poll::Ready(Ok(())) => {}
                        other => return other,
                    }
                }

                match status {
                    Status::StreamEnd => break,
                    Status::Ok | Status::BufError => continue,
                }
            }

            self.get_mut().finished = true;
        }

        // Do NOT propagate shutdown to inner writer.
        // The caller (EntryWriter::close) writes the Data Descriptor
        // after the compressed stream, so the inner writer must remain open.
        Poll::Ready(Ok(()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: drain output from encoder into a Vec.
    async fn compress(data: &[u8], level: CompressionLevel) -> Vec<u8> {
        let mut buf = Vec::new();
        let mut encoder = DeflateEncoder::new(&mut buf, level);
        tokio::io::AsyncWriteExt::write_all(&mut encoder, data)
            .await
            .unwrap();
        tokio::io::AsyncWriteExt::shutdown(&mut encoder)
            .await
            .unwrap();
        drop(encoder);
        buf
    }

    fn decompress(compressed: &[u8], expected_len: usize) -> Vec<u8> {
        let mut decompressor = flate2::Decompress::new(false);
        let mut raw_out = vec![0u8; (expected_len + 8192).max(8192)];
        let (mut in_pos, mut out_len) = (0, 0);
        loop {
            if out_len >= raw_out.len() {
                raw_out.resize(raw_out.len() + 65536, 0);
            }
            let in_bytes = &compressed[in_pos..];
            let out_bytes = &mut raw_out[out_len..];
            let result = decompressor
                .decompress(in_bytes, out_bytes, flate2::FlushDecompress::Finish)
                .unwrap();
            in_pos = decompressor.total_in() as usize;
            out_len = decompressor.total_out() as usize;
            match result {
                flate2::Status::StreamEnd => break,
                flate2::Status::Ok | flate2::Status::BufError => continue,
            }
        }
        raw_out[..out_len].to_vec()
    }

    #[tokio::test]
    async fn test_encoder_produces_valid_deflate() {
        let data = b"Hello, World! This is a test of the deflate encoder.";
        let compressed = compress(data, CompressionLevel::default()).await;
        let decompressed = decompress(&compressed, data.len());
        assert_eq!(&decompressed, data);
    }

    #[tokio::test]
    async fn test_encoder_compresses_repeated_data() {
        let data = vec![b'A'; 4096];
        let compressed = compress(&data, CompressionLevel::best()).await;
        assert!(
            compressed.len() < data.len(),
            "compressed size {} should be less than uncompressed {}",
            compressed.len(),
            data.len()
        );
    }

    #[tokio::test]
    async fn test_encoder_no_compression_level_0() {
        let data = vec![b'A'; 1024];
        let compressed = compress(&data, CompressionLevel::none()).await;
        assert!(
            compressed.len() >= data.len(),
            "level 0 should not compress (got {} < {})",
            compressed.len(),
            data.len()
        );
    }

    #[tokio::test]
    async fn test_encoder_empty_input() {
        let data = b"";
        let compressed = compress(data, CompressionLevel::default()).await;
        assert!(
            !compressed.is_empty(),
            "empty input should produce deflate end marker"
        );
        let decompressed = decompress(&compressed, 0);
        assert!(decompressed.is_empty(), "decompressed should be empty");
    }

    #[tokio::test]
    async fn test_encoder_large_data() {
        let data: Vec<u8> = (0..100_000u32).map(|i| (i % 256) as u8).collect();
        let compressed = compress(&data, CompressionLevel::default()).await;
        assert!(
            compressed.len() < data.len(),
            "100KB of cyclic data should compress"
        );
        let decompressed = decompress(&compressed, data.len());
        assert_eq!(decompressed, data);
    }

    #[tokio::test]
    async fn test_shutdown_does_not_propagate_to_inner() {
        // Create a writer that tracks whether shutdown was called
        struct ShutdownTracker {
            data: Vec<u8>,
            shutdown_called: bool,
        }

        impl AsyncWrite for ShutdownTracker {
            fn poll_write(
                mut self: Pin<&mut Self>,
                _cx: &mut Context<'_>,
                buf: &[u8],
            ) -> Poll<io::Result<usize>> {
                self.data.extend_from_slice(buf);
                Poll::Ready(Ok(buf.len()))
            }
            fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
                Poll::Ready(Ok(()))
            }
            fn poll_shutdown(
                mut self: Pin<&mut Self>,
                _cx: &mut Context<'_>,
            ) -> Poll<io::Result<()>> {
                self.shutdown_called = true;
                Poll::Ready(Ok(()))
            }
        }

        let tracker = ShutdownTracker {
            data: Vec::new(),
            shutdown_called: false,
        };
        let mut encoder = DeflateEncoder::new(tracker, CompressionLevel::default());
        tokio::io::AsyncWriteExt::write_all(&mut encoder, b"test data")
            .await
            .unwrap();
        tokio::io::AsyncWriteExt::shutdown(&mut encoder)
            .await
            .unwrap();

        // After shutdown, the inner writer's shutdown should NOT have been called
        assert!(
            !encoder.get_ref().shutdown_called,
            "encoder should not propagate shutdown to inner"
        );

        // The compressed data should be in the inner writer
        assert!(
            !encoder.get_ref().data.is_empty(),
            "compressed data should be available"
        );
    }
}
