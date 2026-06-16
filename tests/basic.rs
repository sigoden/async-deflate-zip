mod common;

use async_deflate_zip::{EntryOptions, ZipWriter};

#[tokio::test]
async fn basic_single_file() {
    let (_dir, zip_path) = common::zip::zip_path("basic");

    let name = "hello.txt";
    let content: &[u8] = b"Hello, async-deflate-zip!";

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
async fn basic_multiple_files() {
    let (_dir, zip_path) = common::zip::zip_path("basic_multiple");

    let files: [(&str, &[u8]); 4] = [
        ("top.txt", b"top level file"),
        ("sub/a.txt", b"file in subdir a"),
        ("sub/b.txt", b"file in subdir b"),
        ("sub/nested/deep.txt", b"deeply nested"),
    ];

    let file = tokio::fs::File::create(&zip_path).await.unwrap();
    let mut zip = ZipWriter::new(file);
    for (name, content) in &files {
        zip.add_reader(name, *content, &EntryOptions::file())
            .await
            .unwrap();
    }
    zip.finish().await.unwrap();

    common::verify::verify_zip(&zip_path, files.len());
    for (name, content) in &files {
        common::verify::verify_entry_content(&zip_path, name, content);
    }
}

#[tokio::test]
async fn basic_disk_files() {
    let (_dir, zip_path) = common::zip::zip_path("basic_disk");
    let src_dir = tempfile::TempDir::new().unwrap();

    // Create two source files on disk
    let src1_path = src_dir.path().join("from_reader.txt");
    let src2_path = src_dir.path().join("from_copy.txt");
    let content1: &[u8] = b"Added via add_reader with tokio::fs::File";
    let content2: &[u8] = b"Added via start_file + tokio::io::copy";

    tokio::fs::write(&src1_path, content1).await.unwrap();
    tokio::fs::write(&src2_path, content2).await.unwrap();

    // Method 1: add_reader with a tokio::fs::File
    let file = tokio::fs::File::create(&zip_path).await.unwrap();
    let mut zip = ZipWriter::new(file);
    let file1 = tokio::fs::File::open(&src1_path).await.unwrap();
    let opts1 = EntryOptions::from_path(&src1_path).await.unwrap();
    zip.add_reader("from_reader.txt", file1, &opts1)
        .await
        .unwrap();

    // Method 2: start_file + tokio::io::copy
    let opts2 = EntryOptions::from_path(&src2_path).await.unwrap();
    let mut entry = zip.start_file("from_copy.txt", &opts2).await.unwrap();
    let mut file2 = tokio::fs::File::open(&src2_path).await.unwrap();
    tokio::io::copy(&mut file2, &mut entry).await.unwrap();
    entry.finish().await.unwrap();

    zip.finish().await.unwrap();

    common::verify::verify_zip(&zip_path, 2);
    common::verify::verify_entry_content(&zip_path, "from_reader.txt", content1);
    common::verify::verify_entry_content(&zip_path, "from_copy.txt", content2);
}
