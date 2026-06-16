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

    // Create three source files on disk
    let name1 = "add_file.txt";
    let name2 = "add_reader.txt";
    let name3 = "start_file.txt";
    let content1: &[u8] = b"Added via add_file";
    let content2: &[u8] = b"Added via add_reader with tokio::fs::File";
    let content3: &[u8] = b"Added via start_file + tokio::io::copy";

    let src1_path = src_dir.path().join(name1);
    let src2_path = src_dir.path().join(name2);
    let src3_path = src_dir.path().join(name3);

    tokio::fs::write(&src1_path, content1).await.unwrap();
    tokio::fs::write(&src2_path, content2).await.unwrap();
    tokio::fs::write(&src3_path, content3).await.unwrap();

    // Method 1: add_file
    let file = tokio::fs::File::create(&zip_path).await.unwrap();
    let mut zip = ZipWriter::new(file);
    zip.add_file(name1, &src1_path).await.unwrap();

    // Method 2: add_reader with a tokio::fs::File
    let file2 = tokio::fs::File::open(&src2_path).await.unwrap();
    let opts2 = EntryOptions::from_path(&src2_path).await.unwrap();
    zip.add_reader(name2, file2, &opts2).await.unwrap();

    // Method 3: start_file + tokio::io::copy
    let opts3 = EntryOptions::from_path(&src3_path).await.unwrap();
    let mut entry = zip.start_file(name3, &opts3).await.unwrap();
    let mut file3 = tokio::fs::File::open(&src3_path).await.unwrap();
    tokio::io::copy(&mut file3, &mut entry).await.unwrap();
    entry.finish().await.unwrap();

    zip.finish().await.unwrap();

    common::verify::verify_zip(&zip_path, 3);
    common::verify::verify_entry_content(&zip_path, name1, content1);
    common::verify::verify_entry_content(&zip_path, name2, content2);
    common::verify::verify_entry_content(&zip_path, name3, content3);
}
