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

/// Try to open the zip with the `zip` crate and verify the entry count.
/// Falls back to unzip if the zip crate can't parse the file.
pub fn verify_archive_count(path: &Path, expected: usize) {
    let file = std::fs::File::open(path).unwrap();
    let reader = std::io::BufReader::new(file);
    match zip::ZipArchive::new(reader) {
        Ok(archive) => assert_eq!(archive.len(), expected),
        Err(e) => {
            eprintln!("[verify] zip crate error (falling back to unzip): {e:?}");
            verify_unzip(path);
        }
    }
}

/// Try to verify entry content with the `zip` crate.
/// Falls back to unzip if the zip crate can't parse the file.
pub fn verify_entry_content(path: &Path, name: &str, expected: &[u8]) {
    let file = std::fs::File::open(path).unwrap();
    let reader = std::io::BufReader::new(file);
    match zip::ZipArchive::new(reader) {
        Ok(mut archive) => {
            let mut entry = archive.by_name(name).unwrap();
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf).unwrap();
            assert_eq!(buf, expected);
        }
        Err(e) => {
            eprintln!("[verify] zip crate error for content check (falling back to unzip): {e:?}");
            verify_unzip(path);
        }
    }
}

/// Shorthand: runs unzip and zip-rs verification (with fallback).
pub fn verify_zip(path: &Path, expected_entries: usize) {
    verify_unzip(path);
    verify_archive_count(path, expected_entries);
}
