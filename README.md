# async-deflate-zip
[![Crates.io](https://img.shields.io/crates/v/async_deflate_zip?style=flat-square)](https://crates.io/crates/async_deflate_zip)
[![docs.rs](https://img.shields.io/docsrs/async_deflate_zip?style=flat-square)](https://docs.rs/async_deflate_zip/)
[![GitHub Workflow Status (branch)](https://img.shields.io/github/actions/workflow/status/sigoden/async-deflate-zip/ci.yml?branch=main&style=flat-square)](https://github.com/sigoden/async-deflate-zip/actions?query=branch%3Amain)


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

Licensed under either of

- [Apache License, Version 2.0](LICENSE-APACHE)
- [MIT license](LICENSE-MIT)
