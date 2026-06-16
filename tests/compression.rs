mod common;

use async_deflate_zip::{CompressionLevel, EntryOptions, ZipWriter};

async fn write_zip(level: CompressionLevel) -> (tempfile::TempDir, std::path::PathBuf) {
    let name = "data.bin";
    let content: Vec<u8> = vec![b'A'; 10_000];

    let (_dir, zip_path) = common::zip::zip_path("compression");
    let file = tokio::fs::File::create(&zip_path).await.unwrap();
    let mut zip = ZipWriter::new(file).with_compression_level(level);
    zip.add_reader(name, content.as_slice(), &EntryOptions::file())
        .await
        .unwrap();
    zip.finish().await.unwrap();

    common::verify::verify_zip(&zip_path, 1);
    common::verify::verify_entry_content(&zip_path, name, &content);
    (_dir, zip_path)
}

#[tokio::test]
async fn compression_none() {
    write_zip(CompressionLevel::none()).await;
}

#[tokio::test]
async fn compression_fast() {
    write_zip(CompressionLevel::fast()).await;
}

#[tokio::test]
async fn compression_default() {
    write_zip(CompressionLevel::default()).await;
}

#[tokio::test]
async fn compression_best() {
    write_zip(CompressionLevel::best()).await;
}

#[tokio::test]
async fn compression_stored() {
    let (_dir, zip_path) = common::zip::zip_path("compression_stored");
    let name1 = "stored.bin";
    let content1: &[u8] = &[0xFF; 5000];
    let name2 = "stored2.bin";
    let content2: &[u8] = b"small stored data";

    let file = tokio::fs::File::create(&zip_path).await.unwrap();
    let mut zip = ZipWriter::new(file).with_compression_level(CompressionLevel::none());
    zip.add_reader(name1, content1, &EntryOptions::file())
        .await
        .unwrap();
    zip.add_reader(name2, content2, &EntryOptions::file())
        .await
        .unwrap();
    zip.finish().await.unwrap();

    common::verify::verify_zip(&zip_path, 2);
    common::verify::verify_entry_content(&zip_path, name1, content1);
    common::verify::verify_entry_content(&zip_path, name2, content2);
}
