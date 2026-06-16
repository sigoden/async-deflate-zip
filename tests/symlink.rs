mod common;

use async_deflate_zip::{EntryOptions, ZipWriter};

#[tokio::test]
async fn symlink_basic() {
    let (_dir, zip_path) = common::zip::zip_path("symlink");

    let file = tokio::fs::File::create(&zip_path).await.unwrap();
    let mut zip = ZipWriter::new(file);

    zip.add_symlink("link.txt", "hello.txt", &EntryOptions::symlink())
        .await
        .unwrap();
    zip.add_symlink("sub/alink", "../link.txt", &EntryOptions::symlink())
        .await
        .unwrap();

    zip.finish().await.unwrap();

    common::verify::verify_zip(&zip_path, 2);
}
