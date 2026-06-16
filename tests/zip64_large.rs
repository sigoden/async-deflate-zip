mod common;

use std::pin::Pin;
use std::task::{Context, Poll};

use async_deflate_zip::{CompressionLevel, EntryOptions, ZipWriter};
use tokio::io::AsyncRead;

/// A reader that yields zeros without allocating or touching disk.
/// Used to exercise ZIP64 code paths without generating multi-GiB files.
struct HugeReader {
    remaining: u64,
}

impl AsyncRead for HugeReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let max = buf.remaining().min(self.remaining as usize);
        buf.initialize_unfilled_to(max);
        buf.advance(max);
        self.remaining -= max as u64;
        Poll::Ready(Ok(()))
    }
}

/// Stream 5 GiB of zeros through the deflate encoder to trigger ZIP64
/// size fields (>4 GiB), without writing anything to disk.
#[tokio::test]
#[ignore]
async fn zip64_large_file() {
    let (_dir, zip_path) = common::zip::zip_path("zip64_large");

    let out_file = tokio::fs::File::create(&zip_path).await.unwrap();
    let mut zip = ZipWriter::new(out_file).with_compression_level(CompressionLevel::fast());

    let reader = HugeReader {
        remaining: 5_000_000_000,
    };
    zip.add_reader("huge.bin", reader, EntryOptions::file())
        .await
        .unwrap();

    let mut entry = zip
        .start_file("readme.txt", EntryOptions::file())
        .await
        .unwrap();
    tokio::io::AsyncWriteExt::write_all(&mut entry, b"This archive contains a >4GB file.")
        .await
        .unwrap();
    entry.finish().await.unwrap();

    zip.finish().await.unwrap();

    common::verify::verify_archive_count(&zip_path, 2);
}
