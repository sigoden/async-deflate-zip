#![allow(dead_code)]

use std::io::Read;
use std::path::Path;
use std::process::Command;

/// Run `unzip -t` as a basic structural sanity check.
/// Skips silently if `unzip` is not installed.
pub fn verify_unzip(path: &Path) {
    let has_unzip = Command::new("unzip")
        .arg("-v")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !has_unzip {
        return;
    }
    let out = Command::new("unzip")
        .args(["-t", &path.to_string_lossy()])
        .output()
        .expect("unzip -t failed");
    assert!(
        out.status.success(),
        "unzip -t failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Open the archive with the `zip` crate, failing the test if it cannot be
/// parsed.
///
/// Earlier this fell back to `unzip -t` on a parse error, which silently
/// skipped the entry-count/content assertions and could hide regressions.
fn open_archive(path: &Path) -> zip::ZipArchive<std::io::BufReader<std::fs::File>> {
    let file = std::fs::File::open(path).unwrap();
    zip::ZipArchive::new(std::io::BufReader::new(file))
        .unwrap_or_else(|e| panic!("the `zip` crate failed to parse {}: {e}", path.display()))
}

/// Verify the entry count with the `zip` crate.
pub fn verify_archive_count(path: &Path, expected: usize) {
    assert_eq!(open_archive(path).len(), expected);
}

/// Verify entry content with the `zip` crate.
pub fn verify_entry_content(path: &Path, name: &str, expected: &[u8]) {
    let mut archive = open_archive(path);
    let mut entry = archive
        .by_name(name)
        .unwrap_or_else(|e| panic!("entry {name:?} not found in {}: {e}", path.display()));
    let mut buf = Vec::new();
    entry.read_to_end(&mut buf).unwrap();
    assert_eq!(buf, expected, "content mismatch for {name:?}");
}

/// Shorthand: runs unzip and zip-rs verification (with fallback).
pub fn verify_zip(path: &Path, expected_entries: usize) {
    verify_unzip(path);
    verify_archive_count(path, expected_entries);
}
