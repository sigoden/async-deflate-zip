# async-deflate-zip
[![Crates.io](https://img.shields.io/crates/v/async-deflate-zip?style=flat-square)](https://crates.io/crates/async-deflate-zip)
[![docs.rs](https://img.shields.io/docsrs/async-deflate-zip?style=flat-square)](https://docs.rs/async-deflate-zip/)
[![GitHub Workflow Status (branch)](https://img.shields.io/github/actions/workflow/status/sigoden/async-deflate-zip/ci.yml?branch=main&style=flat-square)](https://github.com/sigoden/async-deflate-zip/actions?query=branch%3Amain)


Streaming async ZIP archive writer with per-file deflate compression.

# Usage

```toml
[dependencies]
async-deflate-zip = "0.1.0"
```

```rust
use async_deflate_zip::{ZipWriter, WriterOptions};
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

let file = File::create("tmp/output.zip").await?;
let mut zip = ZipWriter::new(file);

let mut entry = zip.append_file("hello.txt", WriterOptions::file()).await?;
entry.write_all(b"Hello, World!").await?;
entry.close().await?;

zip.finalize().await?;
```

## Why Deflate Only?

**Performance balance.** Deflate offers the best trade-off between compression ratio and CPU cost for async streaming. Heavier algorithms like LZMA or BZip2 create backpressure that degrades I/O throughput, while Deflate keeps the pipeline responsive with sufficient compression for most real-world data.

**Universal compatibility.** Deflate (method 8) is the universal baseline — every ZIP reader on every platform supports it. Non-Deflate methods (LZMA, BZip2, PPMd) have fragmented tool and OS support. By committing to Deflate-only, this library guarantees every archive it produces is readable anywhere.

## License

Licensed under either of

- [Apache License, Version 2.0](LICENSE-APACHE)
- [MIT license](LICENSE-MIT)
