#![cfg(test)]

use std::io::Cursor;

/// Open an in-memory ZIP buffer for inspection using the `zip` crate.
pub(crate) fn open_zip(buf: &[u8]) -> zip::ZipArchive<Cursor<&[u8]>> {
    zip::ZipArchive::new(Cursor::new(buf)).expect("invalid ZIP archive")
}

/// Assert the archive contains exactly `expected` entries.
pub(crate) fn assert_entry_count(buf: &[u8], expected: usize) {
    let archive = open_zip(buf);
    assert_eq!(
        archive.len(),
        expected,
        "archive should contain {expected} entries"
    );
}

/// Assert that an entry has a specific compression method.
pub(crate) fn assert_entry_compression(buf: &[u8], name: &str, method: zip::CompressionMethod) {
    let mut archive = open_zip(buf);
    let entry = archive
        .by_name(name)
        .unwrap_or_else(|_| panic!("entry {name:?} not found"));
    assert_eq!(
        entry.compression(),
        method,
        "compression method for {name:?}"
    );
}

/// Assert that an entry's compressed and uncompressed sizes are equal (stored entry).
pub(crate) fn assert_entry_sizes_match(buf: &[u8], name: &str) {
    let mut archive = open_zip(buf);
    let entry = archive
        .by_name(name)
        .unwrap_or_else(|_| panic!("entry {name:?} not found"));
    assert_eq!(
        entry.compressed_size(),
        entry.size(),
        "compressed and uncompressed sizes should match for stored entry {name:?}"
    );
}

/// Assert compressed size is strictly less than uncompressed size.
pub(crate) fn assert_compressed_smaller(buf: &[u8], name: &str) {
    let mut archive = open_zip(buf);
    let entry = archive
        .by_name(name)
        .unwrap_or_else(|_| panic!("entry {name:?} not found"));
    assert!(
        entry.compressed_size() < entry.size(),
        "compressed {} >= uncompressed {} for {name:?}",
        entry.compressed_size(),
        entry.size()
    );
}

/// Assert an entry's comment.
pub(crate) fn assert_entry_comment(buf: &[u8], name: &str, expected: &str) {
    let mut archive = open_zip(buf);
    let entry = archive
        .by_name(name)
        .unwrap_or_else(|_| panic!("entry {name:?} not found"));
    assert_eq!(entry.comment(), expected, "comment for {name:?}");
}

/// Assert the archive-level comment.
pub(crate) fn assert_archive_comment(buf: &[u8], expected: &str) {
    let archive = open_zip(buf);
    assert_eq!(archive.comment(), expected.as_bytes(), "archive comment");
}

/// Assert that an entry's extra data contains a specific tag (e.g. b"UT" or b"Ux").
pub(crate) fn assert_extra_has_tag(buf: &[u8], name: &str, tag: &[u8]) {
    let mut archive = open_zip(buf);
    let entry = archive
        .by_name(name)
        .unwrap_or_else(|_| panic!("entry {name:?} not found"));
    let extra = entry.extra_data().expect("entry has no extra data");
    assert!(
        extra.windows(2).any(|w| w == tag),
        "extra data for {name:?} should contain tag {tag:?}, got {extra:02x?}"
    );
}

/// Assert that an entry's unix mode matches expected (file type + permissions).
pub(crate) fn assert_unix_mode(buf: &[u8], name: &str, expected: u32) {
    let mut archive = open_zip(buf);
    let entry = archive
        .by_name(name)
        .unwrap_or_else(|_| panic!("entry {name:?} not found"));
    assert_eq!(entry.unix_mode(), Some(expected), "unix mode for {name:?}");
}

/// Assert that an entry's last_modified matches the expected MS-DOS local time.
pub(crate) fn assert_last_modified(buf: &[u8], name: &str, datetime: (u16, u8, u8, u8, u8, u8)) {
    let mut archive = open_zip(buf);
    let entry = archive
        .by_name(name)
        .unwrap_or_else(|_| panic!("entry {name:?} not found"));
    let dt = entry.last_modified().expect("entry has no last_modified");
    assert_eq!(dt.year(), datetime.0, "year for {name:?}");
    assert_eq!(dt.month(), datetime.1, "month for {name:?}");
    assert_eq!(dt.day(), datetime.2, "day for {name:?}");
    assert_eq!(dt.hour(), datetime.3, "hour for {name:?}");
    assert_eq!(dt.minute(), datetime.4, "minute for {name:?}");
    assert_eq!(dt.second(), datetime.5, "second for {name:?}");
}

/// Check entry content.
pub(crate) fn assert_entry_content(buf: &[u8], name: &str, expected: &[u8]) {
    let mut archive = open_zip(buf);
    let mut entry = archive
        .by_name(name)
        .unwrap_or_else(|_| panic!("entry {name:?} not found"));
    let mut content = Vec::new();
    std::io::Read::read_to_end(&mut entry, &mut content)
        .unwrap_or_else(|_| panic!("read entry {name:?}"));
    assert_eq!(content, expected, "content for {name:?}");
}

/// Assert that the archive contains all three PK signatures (basic validity).
pub(crate) fn assert_has_pk_signatures(buf: &[u8]) {
    assert!(
        buf.windows(4).any(|w| w == b"PK\x03\x04"),
        "missing Local File Header signature"
    );
    assert!(
        buf.windows(4).any(|w| w == b"PK\x01\x02"),
        "missing Central Directory signature"
    );
    assert!(
        buf.windows(4).any(|w| w == b"PK\x05\x06"),
        "missing EOCDR signature"
    );
}
