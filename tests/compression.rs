mod common;

use async_deflate_zip::{CompressionLevel, EntryOptions, ZipWriter};

async fn write_zip(name: &str, level: CompressionLevel) {
    let content: Vec<u8> = vec![b'A'; 10_000];

    let (_dir, zip_path) = common::zip::zip_path(name);
    let file = tokio::fs::File::create(&zip_path).await.unwrap();
    let mut zip = ZipWriter::new(file).with_compression_level(level);
    zip.add_reader(name, content.as_slice(), &EntryOptions::file())
        .await
        .unwrap();
    zip.finish().await.unwrap();

    common::verify::verify_zip(&zip_path, 1);
    common::verify::verify_entry_content(&zip_path, name, &content);
}

#[tokio::test]
async fn compression_levels() {
    for (level, name) in [
        (CompressionLevel::none(), "compression_none"),
        (CompressionLevel::fast(), "compression_fast"),
        (CompressionLevel::default(), "compression_default"),
        (CompressionLevel::best(), "compression_best"),
    ] {
        write_zip(name, level).await;
    }
}
