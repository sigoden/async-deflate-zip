//! ZIP binary format header structures and serialization helpers.
//!
//! This module implements the low-level on-disk structures that make up a
//! standard ZIP archive. The ZIP format organizes an archive into three
//! sections:
//!
//! 1. **Entries** — Each file or directory entry starts with a [`LocalFileHeader`]
//!    (signature `PK\x03\x04`), followed by the (optionally compressed) file data,
//!    followed by an optional Data Descriptor containing CRC-32 and sizes.
//! 2. **Central Directory** — A sequence of [`CentralDirEntry`] records (signature
//!    `PK\x01\x02`), one per entry, providing a table of contents.
//! 3. **End of Central Directory** — An [`Eocdr`] record (signature `PK\x05\x06`)
//!    pointing to the central directory. For large archives (ZIP64), additional
//!    [`Zip64Eocdr`] and [`Zip64EocdrLocator`] records are written.
//!
//! All multi-byte values in the ZIP format are little-endian. Compression is
//! indicated by the `method` field in each header.

mod constants;
mod serialize;
mod types;

pub(crate) use constants::*;
pub(crate) use serialize::*;
pub(crate) use types::*;

#[cfg(test)]
mod tests;
