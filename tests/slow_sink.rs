//! Regression tests for reading behavior under a slow (back-pressured) sink.
//!
//! These model an inner writer that is not always ready to accept data, like
//! a TCP socket whose send buffer is momentarily full.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use async_deflate_zip::{EntryOptions, ZipWriter};
use tokio::io::AsyncWrite;

/// Inner writer that answers `Pending` for every second `poll_write` call,
/// modeling a consumer with a fill-and-drain send buffer.
struct SlowSink {
    data: Vec<u8>,
    call: usize,
}

impl AsyncWrite for SlowSink {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.call += 1;
        if self.call.is_multiple_of(2) {
            cx.waker().wake_by_ref();
            return Poll::Pending;
        }
        self.data.extend_from_slice(buf);
        Poll::Ready(Ok(buf.len()))
    }
    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

/// Enough data to force flate2's internal output buffer to spill during
/// `poll_write` (once roughly 1.6 MB has been consumed), exercising the
/// drain-write path in the middle of a stream.
const N: usize = 1_600_000;

fn pseudo_random(n: u64) -> u8 {
    ((n.wrapping_mul(6364136223846793005) >> 33) & 0xFF) as u8
}

#[tokio::test]
async fn slow_sink_produces_valid_archive() {
    let data: Vec<u8> = (0..N as u64).map(pseudo_random).collect();

    let mut zip = ZipWriter::new(SlowSink {
        data: Vec::new(),
        call: 0,
    });
    let mut entry = zip
        .start_file("probe.bin", &EntryOptions::file())
        .await
        .unwrap();
    // Write in two large chunks: the second `poll_write` triggers flate2's
    // output spill and hits a Pending drain after input was consumed. The
    // encoder must report success for the consumed bytes instead of failing.
    tokio::io::AsyncWriteExt::write_all(&mut entry, &data[..N / 2])
        .await
        .unwrap();
    tokio::io::AsyncWriteExt::write_all(&mut entry, &data[N / 2..])
        .await
        .unwrap();
    entry.finish().await.unwrap();

    let mut sink = zip.finish().await.unwrap();

    // Verify the archive with the `zip` crate (CRC + sizes are validated).
    use std::io::Read;
    let reader = std::io::Cursor::new(std::mem::take(&mut sink.data));
    let mut zf = zip::ZipArchive::new(reader).unwrap();
    let mut entry = zf.by_name("probe.bin").unwrap();
    assert_eq!(entry.size(), N as u64);
    let mut out = Vec::new();
    entry
        .read_to_end(&mut out)
        .expect("archive must be readable");
    assert_eq!(out, data, "payload content must match original");
}
