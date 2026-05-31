# async-deflate-zip

Streaming async ZIP archive writer with per-file deflate compression.

## Usage

```rust
use async_deflate_zip::ZipWriter;
use tokio::io::AsyncWriteExt;

let mut buf = Vec::new();
let mut zip = ZipWriter::new(&mut buf);

let mut entry = zip.append_file("hello.txt").await.unwrap();
entry.write_all(b"Hello, World!").await.unwrap();
entry.close().await.unwrap();

zip.finalize().await.unwrap();
```

## License

MIT
