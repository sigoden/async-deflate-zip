//! Streaming async ZIP archive writer with per-file deflate compression.
//!
//! This crate provides an asynchronous interface for creating ZIP archives
//! from streams of data. Unlike blocking ZIP writers, entries are written
//! incrementally — each file is compressed and written to the output as it
//! arrives, without buffering the entire archive in memory.
//!
//! # Public API
//!
//! The main entry point is [`ZipWriter`], which accepts per-entry metadata
//! via [`EntryOptions`]. Individual file data is streamed through
//! [`EntryWriter`], obtained from [`ZipWriter::start_file`].
//!
//! # Quick Start
//!
//! ```rust,no_run
//! use async_deflate_zip::{ZipWriter, EntryOptions};
//! use tokio::io::AsyncWriteExt;
//!
//! # async fn example() {
//! let mut buf = Vec::new();
//! let mut zip = ZipWriter::new(&mut buf);
//!
//! let mut entry = zip.start_file("hello.txt", EntryOptions::file()).await.unwrap();
//! entry.write_all(b"Hello, World!").await.unwrap();
//! entry.finish().await.unwrap();
//!
//! zip.finish().await.unwrap();
//! # }
//! ```

mod deflate_encoder;
mod error;
mod header;
mod writer;

pub use error::ZipError;
pub type CompressionLevel = flate2::Compression;
pub use writer::EntryOptions;
pub use writer::EntryWriter;
pub use writer::ZipWriter;
