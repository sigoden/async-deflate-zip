use async_deflate_zip::{CompressionLevel, ZipWriter};
use tokio::io::AsyncWriteExt;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let mut buf = Vec::new();
    let mut zip = ZipWriter::new(&mut buf).with_compression_level(CompressionLevel::DEFAULT);

    let mut entry = zip.append_file("hello.txt").await.unwrap();
    entry.write_all(b"Hello, World!").await.unwrap();
    entry.close().await.unwrap();

    let mut entry = zip.append_file("nested/path/data.bin").await.unwrap();
    entry.write_all(&[0u8; 256]).await.unwrap();
    entry.close().await.unwrap();

    zip.finalize().await.unwrap();

    std::fs::write("/tmp/test_output.zip", &buf).unwrap();
    eprintln!("ZIP size: {} bytes", buf.len());
    eprintln!(
        "LFH={} CD={} EOCDR={}",
        buf.windows(4).filter(|w| w == b"PK\x03\x04").count(),
        buf.windows(4).filter(|w| w == b"PK\x01\x02").count(),
        buf.windows(4).filter(|w| w == b"PK\x05\x06").count(),
    );
}
