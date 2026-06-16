mod common;

use async_deflate_zip::{EntryOptions, ZipWriter};

#[tokio::test]
async fn unicode_german() {
    let (_dir, zip_path) = common::zip::zip_path("unicode_german");
    let name = "Grüß Gott.txt";
    let content: &[u8] = "Schöne Grüße aus der UTF-8-Welt!".as_bytes();

    let file = tokio::fs::File::create(&zip_path).await.unwrap();
    let mut zip = ZipWriter::new(file);
    zip.add_reader(name, content, &EntryOptions::file())
        .await
        .unwrap();
    zip.finish().await.unwrap();

    common::verify::verify_zip(&zip_path, 1);
    common::verify::verify_entry_content(&zip_path, name, content);
}

#[tokio::test]
async fn unicode_chinese() {
    let (_dir, zip_path) = common::zip::zip_path("unicode_chinese");
    let name = "世界.txt";
    let content: &[u8] = "你好，世界！来自异步压缩库的问候。".as_bytes();

    let file = tokio::fs::File::create(&zip_path).await.unwrap();
    let mut zip = ZipWriter::new(file);
    zip.add_reader(name, content, &EntryOptions::file())
        .await
        .unwrap();
    zip.finish().await.unwrap();

    common::verify::verify_zip(&zip_path, 1);
    common::verify::verify_entry_content(&zip_path, name, content);
}

#[tokio::test]
async fn unicode_directory() {
    let (_dir, zip_path) = common::zip::zip_path("unicode_dir");

    let file = tokio::fs::File::create(&zip_path).await.unwrap();
    let mut zip = ZipWriter::new(file);
    zip.add_directory("목차/", &EntryOptions::directory())
        .await
        .unwrap();
    zip.finish().await.unwrap();

    common::verify::verify_zip(&zip_path, 1);
}
