mod common;

use async_deflate_zip::{EntryOptions, ZipWriter};

#[tokio::test]
async fn metadata_with_mtime() {
    let (_dir, zip_path) = common::zip::zip_path("with_mtime");
    let name = "with_mtime.txt";
    let content: &[u8] = b"epoch";
    let opts = EntryOptions::default().with_mtime(std::time::SystemTime::UNIX_EPOCH);

    let file = tokio::fs::File::create(&zip_path).await.unwrap();
    let mut zip = ZipWriter::new(file);
    zip.add_reader(name, content, opts).await.unwrap();
    zip.finish().await.unwrap();

    common::verify::verify_zip(&zip_path, 1);
    common::verify::verify_entry_content(&zip_path, name, content);
}

#[tokio::test]
async fn metadata_with_unix_permissions() {
    let (_dir, zip_path) = common::zip::zip_path("with_unix_permissions");
    let name = "with_unix_permissions.txt";
    let content: &[u8] = b"readable";
    let opts = EntryOptions::default().with_unix_permissions(0o600);

    let file = tokio::fs::File::create(&zip_path).await.unwrap();
    let mut zip = ZipWriter::new(file);
    zip.add_reader(name, content, opts).await.unwrap();
    zip.finish().await.unwrap();

    common::verify::verify_zip(&zip_path, 1);
    common::verify::verify_entry_content(&zip_path, name, content);
}

#[tokio::test]
async fn metadata_with_uid_gid() {
    let (_dir, zip_path) = common::zip::zip_path("with_uid_gid");
    let name = "with_uid_gid.txt";
    let content: &[u8] = b"uid/gid";
    let opts = EntryOptions::file().with_uid_gid(0, 0);

    let file = tokio::fs::File::create(&zip_path).await.unwrap();
    let mut zip = ZipWriter::new(file);
    zip.add_reader(name, content, opts).await.unwrap();
    zip.finish().await.unwrap();

    common::verify::verify_zip(&zip_path, 1);
    common::verify::verify_entry_content(&zip_path, name, content);
}
