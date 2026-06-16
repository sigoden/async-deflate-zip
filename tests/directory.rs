mod common;

use async_deflate_zip::{EntryOptions, ZipWriter};

#[tokio::test]
async fn directory_basic() {
    let (_dir, zip_path) = common::zip::zip_path("dir_basic");

    let file = tokio::fs::File::create(&zip_path).await.unwrap();
    let mut zip = ZipWriter::new(file);

    zip.add_directory("emptydir/", &EntryOptions::directory())
        .await
        .unwrap();

    zip.add_directory(
        "dated_dir/",
        &EntryOptions::directory().with_mtime(std::time::SystemTime::UNIX_EPOCH),
    )
    .await
    .unwrap();

    zip.add_directory("protected_dir/", &EntryOptions::directory())
        .await
        .unwrap();

    zip.add_directory(
        "full_meta_dir/",
        &EntryOptions::directory().with_unix_permissions(0o700),
    )
    .await
    .unwrap();

    let name = "emptydir/hello.txt";
    let content: &[u8] = b"nested";
    zip.add_reader(name, content, &EntryOptions::file())
        .await
        .unwrap();

    zip.finish().await.unwrap();

    common::verify::verify_zip(&zip_path, 5);
    common::verify::verify_entry_content(&zip_path, name, content);
}
