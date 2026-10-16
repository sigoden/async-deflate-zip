mod common;

use async_deflate_zip::{EntryOptions, ZipWriter};

#[tokio::test]
async fn unicode_roundtrip() {
    let (_dir, zip_path) = common::zip::zip_path("unicode");

    let german_name = "Grüß Gott.txt";
    let german_content: &[u8] = "Schöne Grüße aus der UTF-8-Welt!".as_bytes();
    let chinese_name = "世界.txt";
    let chinese_content: &[u8] = "你好，世界！来自异步压缩库的问候。".as_bytes();

    let file = tokio::fs::File::create(&zip_path).await.unwrap();
    let mut zip = ZipWriter::new(file);
    zip.add_directory("목차/", &EntryOptions::directory())
        .await
        .unwrap();
    zip.add_reader(german_name, german_content, &EntryOptions::file())
        .await
        .unwrap();
    zip.add_reader(chinese_name, chinese_content, &EntryOptions::file())
        .await
        .unwrap();
    zip.finish().await.unwrap();

    common::verify::verify_zip(&zip_path, 3);
    common::verify::verify_entry_content(&zip_path, german_name, german_content);
    common::verify::verify_entry_content(&zip_path, chinese_name, chinese_content);
}
