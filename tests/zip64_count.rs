mod common;

use async_deflate_zip::{CompressionLevel, EntryOptions, ZipWriter};
use tokio::io::AsyncWriteExt;

/// Create enough entries (65536) to force ZIP64 via entry count.
#[tokio::test]
#[ignore]
async fn zip64_many_entries() {
    let (_dir, zip_path) = common::zip::zip_path("zip64_count");

    let file = tokio::fs::File::create(&zip_path).await.unwrap();
    let mut zip = ZipWriter::new(file).with_compression_level(CompressionLevel::none());

    let count = 0xFFFF + 1;
    for i in 0..count {
        let name = format!("files/f{i}");
        let mut entry = zip.start_file(&name, EntryOptions::file()).await.unwrap();
        entry.write_all(b"x").await.unwrap();
        entry.finish().await.unwrap();
    }

    zip.finish().await.unwrap();

    common::verify::verify_zip(&zip_path, count);
}
