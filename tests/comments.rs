mod common;

use async_deflate_zip::{EntryOptions, ZipWriter};

#[tokio::test]
async fn comments_entry_file() {
    let (_dir, zip_path) = common::zip::zip_path("comment_file");
    let name = "hello.txt";
    let content: &[u8] = b"Hello!";
    let opts = EntryOptions::default().with_comment("a short file comment");

    let file = tokio::fs::File::create(&zip_path).await.unwrap();
    let mut zip = ZipWriter::new(file);
    zip.add_reader(name, content, opts).await.unwrap();
    zip.finish().await.unwrap();

    common::verify::verify_zip(&zip_path, 1);
    common::verify::verify_entry_content(&zip_path, name, content);
}

#[tokio::test]
async fn comments_archive() {
    let (_dir, zip_path) = common::zip::zip_path("comment_archive");
    let name = "hello.txt";
    let content: &[u8] = b"Hello!";

    let file = tokio::fs::File::create(&zip_path).await.unwrap();
    let mut zip = ZipWriter::new(file).with_comment("async-deflate-zip archive");
    zip.add_reader(name, content, EntryOptions::file())
        .await
        .unwrap();
    zip.finish().await.unwrap();

    common::verify::verify_zip(&zip_path, 1);
    common::verify::verify_entry_content(&zip_path, name, content);
}

#[tokio::test]
async fn comments_directory() {
    let (_dir, zip_path) = common::zip::zip_path("comment_dir");

    let file = tokio::fs::File::create(&zip_path).await.unwrap();
    let mut zip = ZipWriter::new(file);
    zip.add_directory(
        "mydir/",
        EntryOptions::directory().with_comment("directory comment"),
    )
    .await
    .unwrap();
    zip.finish().await.unwrap();

    common::verify::verify_zip(&zip_path, 1);
}

#[tokio::test]
async fn comments_symlink() {
    let (_dir, zip_path) = common::zip::zip_path("comment_symlink");

    let file = tokio::fs::File::create(&zip_path).await.unwrap();
    let mut zip = ZipWriter::new(file);
    zip.add_symlink(
        "link.txt",
        "hello.txt",
        EntryOptions::symlink().with_comment("symlink comment"),
    )
    .await
    .unwrap();
    zip.finish().await.unwrap();

    common::verify::verify_zip(&zip_path, 1);
}
