//! Streaming async ZIP archive writer with per-file deflate compression.
//!
//! This crate provides an asynchronous interface for creating ZIP archives
//! from streams of data. Unlike blocking ZIP writers, entries are written
//! incrementally — each file is compressed and written to the output as it
//! arrives, without buffering the entire archive in memory.
//!
//! # Architecture
//!
//! The crate is organized into three modules:
//!
//! - [`ZipWriter`] — Streaming [`ZipWriter`] and per-entry [`EntryWriter`]
//! - [`Compression`] — Deflate compression level (0-9) (re-exported from `flate2`)
//!
//! # Quick Start
//!
//! ```rust,no_run
//! use async_deflate_zip::ZipWriter;
//! use tokio::io::AsyncWriteExt;
//!
//! # async fn example() {
//! let mut buf = Vec::new();
//! let mut zip = ZipWriter::new(&mut buf);
//!
//! let mut entry = zip.append_file("hello.txt").await.unwrap();
//! entry.write_all(b"Hello, World!").await.unwrap();
//! entry.close().await.unwrap();
//!
//! zip.finalize().await.unwrap();
//! # }
//! ```

mod deflate_encoder;
mod error;
mod header;
mod writer;

pub use error::ZipError;
pub use flate2::Compression;
pub use writer::DirectoryWriter;
pub use writer::EntryWriter;
pub use writer::ZipWriter;
