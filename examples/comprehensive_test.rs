//! Comprehensive test for async-deflate-zip library.
//!
//! Tests all public APIs:
//! - ZipWriter::new / ZipWriter::with_level
//! - ZipWriter::append_file → EntryWriter (write, set_mtime, set_permissions, close)
//! - ZipWriter::append_directory → DirectoryWriter (set_mtime, set_permissions, close)
//! - ZipWriter::append_symlink
//! - ZipWriter::finalize
//!
//! Includes a large-file test (>4 GiB) that exercises ZIP64 code paths.
//!
//! Every test writes the archive directly to a temp file and validates it
//! with system `unzip -t` / `zipinfo` tools — no in-memory buffer.

use async_deflate_zip::{Compression, ZipWriter};
use std::path::{Path, PathBuf};
use std::process::Command;
use tokio::io::AsyncWriteExt;

// ============================================================
// Test 1: Basic single file with default compression
// ============================================================
async fn test_basic_single_file() {
    println!("--- Test 1: Basic single file (default compression) ---");
    let path = zip_out_path("01_test_basic_single_file");
    let file = tokio::fs::File::create(&path).await.unwrap();
    let mut zip = ZipWriter::new(file);

    let mut entry = zip.append_file("hello.txt").await.unwrap();
    entry.write_all(b"Hello, async-deflate-zip!").await.unwrap();
    entry.close().await.unwrap();
    zip.finalize().await.unwrap();

    verify_zip_structure(&path, 1).await;
    println!("  PASS\n");
}

// ============================================================
// Test 2: All Compression constants
// ============================================================
async fn test_compression_levels() {
    println!("--- Test 2: All compression levels ---");

    let levels: Vec<(&str, Compression)> = vec![
        ("NONE", Compression::none()),
        ("FAST", Compression::fast()),
        ("DEFAULT", Compression::default()),
        ("BEST", Compression::best()),
    ];

    let data = vec![b'A'; 10_000]; // highly compressible

    for (label, level) in &levels {
        let path = zip_out_path(&format!("02_test_compression_levels_{label}"));
        let file = tokio::fs::File::create(&path).await.unwrap();
        let mut zip = ZipWriter::new(file).with_level(*level);
        let mut entry = zip.append_file(&format!("data_{label}.bin")).await.unwrap();
        entry.write_all(&data).await.unwrap();
        entry.close().await.unwrap();
        zip.finalize().await.unwrap();

        let size = tokio::fs::metadata(&path).await.unwrap().len();
        let ratio = size as f64 / data.len() as f64;
        println!(
            "  {label:8} level={}  ZIP size={:>8}  ratio={:.3}",
            level.level(),
            size,
            ratio
        );
        verify_zip_structure(&path, 1).await;
    }
    println!("  PASS\n");
}

// ============================================================
// Test 3: Multiple files + nested paths
// ============================================================
async fn test_multiple_files_nested() {
    println!("--- Test 3: Multiple files with nested paths ---");
    let path = zip_out_path("03_test_multiple_files_nested");
    let file = tokio::fs::File::create(&path).await.unwrap();
    let mut zip = ZipWriter::new(file);

    let files: [(&str, &[u8]); 4] = [
        ("top.txt", b"top level file" as &[u8]),
        ("sub/a.txt", b"file in subdir a" as &[u8]),
        ("sub/b.txt", b"file in subdir b" as &[u8]),
        ("sub/nested/deep.txt", b"deeply nested" as &[u8]),
    ];

    for (name, content) in &files {
        let mut entry = zip.append_file(name).await.unwrap();
        entry.write_all(content).await.unwrap();
        entry.close().await.unwrap();
    }

    zip.finalize().await.unwrap();
    verify_zip_structure(&path, files.len()).await;
    println!("  PASS\n");
}

// ============================================================
// Test 4: Directory entries with metadata
// ============================================================
async fn test_directory_entries() {
    println!("--- Test 4: Directory entries ---");
    let path = zip_out_path("04_test_directory_entries");
    let file = tokio::fs::File::create(&path).await.unwrap();
    let mut zip = ZipWriter::new(file);

    // Simple directory
    let dir = zip.append_directory("emptydir/").await.unwrap();
    dir.close().await.unwrap();

    // Directory with mtime
    let mut dir = zip.append_directory("dated_dir/").await.unwrap();
    dir.set_mtime(std::time::SystemTime::UNIX_EPOCH);
    dir.close().await.unwrap();

    // Directory with permissions
    let mut dir = zip.append_directory("protected_dir/").await.unwrap();
    dir.set_permissions(0o755);
    dir.close().await.unwrap();

    // Directory with both
    let mut dir = zip.append_directory("full_meta_dir/").await.unwrap();
    dir.set_mtime(std::time::SystemTime::now());
    dir.set_permissions(0o700);
    dir.close().await.unwrap();

    // File inside a directory (demonstrates the directory/file relationship)
    let mut entry = zip.append_file("emptydir/hello.txt").await.unwrap();
    entry.write_all(b"nested").await.unwrap();
    entry.close().await.unwrap();

    zip.finalize().await.unwrap();
    // 4 directories + 1 file
    verify_zip_structure(&path, 5).await;
    println!("  PASS\n");
}

// ============================================================
// Test 5: Symlink entries
// ============================================================
async fn test_symlink_entries() {
    println!("--- Test 5: Symlink entries ---");
    let path = zip_out_path("05_test_symlink_entries");
    let file = tokio::fs::File::create(&path).await.unwrap();
    let mut zip = ZipWriter::new(file);

    zip.append_symlink("link.txt", "hello.txt").await.unwrap();
    zip.append_symlink("sub/alink", "../link.txt")
        .await
        .unwrap();

    zip.finalize().await.unwrap();
    verify_zip_structure(&path, 2).await;
    println!("  PASS\n");
}

// ============================================================
// Test 6: File with mtime and permissions
// ============================================================
async fn test_entry_metadata() {
    println!("--- Test 6: Entry metadata (mtime + permissions) ---");
    let path = zip_out_path("06_test_entry_metadata");
    let file = tokio::fs::File::create(&path).await.unwrap();
    let mut zip = ZipWriter::new(file);

    // File with mtime only
    {
        let mut entry = zip.append_file("mtime_only.txt").await.unwrap();
        entry.set_mtime(std::time::SystemTime::UNIX_EPOCH);
        entry.write_all(b"epoch").await.unwrap();
        entry.close().await.unwrap();
    }

    // File with permissions only
    {
        let mut entry = zip.append_file("perm_only.txt").await.unwrap();
        entry.set_permissions(0o644);
        entry.write_all(b"readable").await.unwrap();
        entry.close().await.unwrap();
    }

    // File with both
    {
        let mut entry = zip.append_file("both_meta.txt").await.unwrap();
        entry.set_mtime(std::time::SystemTime::now());
        entry.set_permissions(0o755);
        entry.write_all(b"executable").await.unwrap();
        entry.close().await.unwrap();
    }

    // File with setuid
    {
        let mut entry = zip.append_file("setuid.txt").await.unwrap();
        entry.set_permissions(0o4755);
        entry.write_all(b"setuid").await.unwrap();
        entry.close().await.unwrap();
    }

    zip.finalize().await.unwrap();
    verify_zip_structure(&path, 4).await;
    println!("  PASS\n");
}

// ============================================================
// Test 7: Write file from filesystem into archive
// ============================================================
async fn test_from_filesystem() {
    println!("--- Test 7: Add files from filesystem ---");
    let tmp_dir = zip_out_dir().join("07_test_from_filesystem");
    ensure_dir_clean(&tmp_dir).await;

    // Create some test files on disk
    tokio::fs::write(tmp_dir.join("readme.txt"), b"Hello from disk!\n")
        .await
        .unwrap();
    tokio::fs::write(tmp_dir.join("data.bin"), &[0xABu8; 1024])
        .await
        .unwrap();
    tokio::fs::write(
        tmp_dir.join("code.rs"),
        b"fn main() { println!(\"hi\"); }\n",
    )
    .await
    .unwrap();

    let path = zip_out_path("07_test_from_filesystem");
    let file = tokio::fs::File::create(&path).await.unwrap();
    let mut zip = ZipWriter::new(file);

    // Read each file from disk and add to zip
    let mut dir = tokio::fs::read_dir(&tmp_dir).await.unwrap();
    while let Some(entry) = dir.next_entry().await.unwrap() {
        let fpath = entry.path();
        if fpath.is_file() {
            let name = fpath.file_name().unwrap().to_str().unwrap().to_string();
            let content = tokio::fs::read(&fpath).await.unwrap();
            let mut zw = zip.append_file(&name).await.unwrap();
            zw.write_all(&content).await.unwrap();
            zw.close().await.unwrap();
            println!("  added: {} ({} bytes)", name, content.len());
        }
    }

    zip.finalize().await.unwrap();

    verify_zip_structure(&path, 3).await;
    println!("  PASS\n");
}

// ============================================================
// Test 8: Stored entry (compression level 0)
// ============================================================
async fn test_stored_entry() {
    println!("--- Test 8: Stored entry (level=NONE / method=STORED) ---");
    let path = zip_out_path("08_test_stored_entry");
    let file = tokio::fs::File::create(&path).await.unwrap();
    let mut zip = ZipWriter::new(file).with_level(Compression::none());

    let mut entry = zip.append_file("stored.bin").await.unwrap();
    entry.write_all(&[0xFF; 500]).await.unwrap();
    entry.close().await.unwrap();

    let mut entry = zip.append_file("stored2.bin").await.unwrap();
    entry.write_all(b"small stored data").await.unwrap();
    entry.close().await.unwrap();

    zip.finalize().await.unwrap();
    verify_zip_structure(&path, 2).await;
    println!("  PASS\n");
}

// ============================================================
// Test 9: Chain API (builder pattern)
// ============================================================
async fn test_builder_chain() {
    println!("--- Test 9: Builder chain ---");
    let path = zip_out_path("09_test_builder_chain");
    let file = tokio::fs::File::create(&path).await.unwrap();
    // Chaining: new → with_level → append_file
    let mut zip = ZipWriter::new(file).with_level(Compression::best());
    let mut entry = zip.append_file("chained.txt").await.unwrap();
    entry.write_all(b"builder pattern test").await.unwrap();
    entry.close().await.unwrap();
    zip.finalize().await.unwrap();
    verify_zip_structure(&path, 1).await;
    println!("  PASS\n");
}

// ============================================================
// Test 10: Multiple small entries to trigger ZIP64 via entry count
// ============================================================
async fn test_zip64_many_entries() {
    println!("--- Test 10: Many entries (>=65535) to trigger ZIP64 via count ---");
    let path = zip_out_path("10_test_zip64_many_entries");
    let file = tokio::fs::File::create(&path).await.unwrap();
    let mut zip = ZipWriter::new(file).with_level(Compression::none());

    let count = 0xFFFF + 1; // 65536 entries — exceeds the 16-bit limit
    for i in 0..count {
        let name = format!("files/f{i}");
        let mut entry = zip.append_file(&name).await.unwrap();
        entry.write_all(b"x").await.unwrap();
        entry.close().await.unwrap();
    }

    zip.finalize().await.unwrap();

    // Verify entry count via zipinfo/unzip or self-contained parser
    verify_zip_structure(&path, count).await;
}

// ============================================================
// Test 11: Large file >4GiB (ZIP64)
// ============================================================
async fn test_large_file_zip64() {
    println!("--- Test 11: Large file >4GiB (ZIP64) ---");

    let large_file = Path::new("/tmp")
        .join("comprehensive_test")
        .join("bigfile.bin");
    let output_zip = zip_out_path("11_test_large_file_zip64");

    println!("  generating 4.5 GiB test file with dd...");

    // Generate a 4.5 GiB sparse file using dd
    // If dd is not found (ExitStatusError), skip the test
    let status = Command::new("dd")
        .args([
            "if=/dev/zero",
            &format!("of={}", large_file.display()),
            "bs=1M",
            "count=4608",
            "status=progress",
        ])
        .status();
    match status {
        Ok(s) if s.success() => {}
        _ => {
            println!("  SKIP: dd unavailable, skipping ZIP64 large file test\n");
            return;
        }
    }

    let file_size = tokio::fs::metadata(&large_file).await.unwrap().len();
    println!(
        "  file size: {} bytes ({:.2} GiB)",
        file_size,
        file_size as f64 / (1 << 30) as f64
    );
    assert!(
        file_size > 4_294_967_296,
        "file must be >4GiB for ZIP64 test, got {file_size}"
    );

    // Open the output zip as a file, then wrap in ZipWriter
    let out_file = tokio::fs::File::create(&output_zip).await.unwrap();
    let mut zip = ZipWriter::new(out_file).with_level(Compression::default());

    // Add the large file with a descriptive name
    {
        let mut entry = zip
            .append_file("large_data_4GB.bin")
            .await
            .expect("append_file for large entry");
        // Stream the large file in chunks to avoid reading 4.5 GiB into memory
        let mut in_file = tokio::fs::File::open(&large_file).await.unwrap();
        let mut buffer = vec![0u8; 64 * 1024]; // 64 KB chunks
        loop {
            let n = tokio::io::AsyncReadExt::read(&mut in_file, &mut buffer)
                .await
                .unwrap();
            if n == 0 {
                break;
            }
            entry.write_all(&buffer[..n]).await.unwrap();
        }
        entry.close().await.expect("close large entry");
    }

    // Also add a small file to verify the archive is well-formed end-to-end
    {
        let mut entry = zip.append_file("readme.txt").await.unwrap();
        entry
            .write_all(b"This archive contains a >4GB file.")
            .await
            .unwrap();
        entry.close().await.unwrap();
    }

    zip.finalize().await.expect("finalize");

    // Verify output archive size is reasonable
    let zip_meta = tokio::fs::metadata(&output_zip).await.unwrap();
    println!(
        "  output ZIP: {} bytes ({:.2} GiB)",
        zip_meta.len(),
        zip_meta.len() as f64 / (1 << 30) as f64
    );

    // Verify structure with unzip/zipinfo or self-contained parser
    verify_zip_structure(&output_zip, 2).await;
    println!("  PASS\n");
}

// ============================================================
// Test runner
// ============================================================
#[tokio::main(flavor = "current_thread")]
async fn main() {
    println!("================================================");
    println!("  async-deflate-zip — Comprehensive Test Suite");
    println!("================================================\n");

    let out_dir = zip_out_dir();
    ensure_dir_clean(&out_dir).await;

    test_basic_single_file().await;
    test_compression_levels().await;
    test_multiple_files_nested().await;
    test_directory_entries().await;
    test_symlink_entries().await;
    test_entry_metadata().await;
    test_from_filesystem().await;
    test_stored_entry().await;
    test_builder_chain().await;

    // ZIP64 via many entries (memory-safe)
    test_zip64_many_entries().await;

    // ZIP64 via large file >4GiB (requires dd)
    test_large_file_zip64().await;

    println!("================================================");
    println!("  ALL TESTS PASSED");
    println!("================================================");
}

// ============================================================
// Helper
// ============================================================

async fn verify_zip_structure(path: &Path, expected_entries: usize) {
    // Check if unzip is available via exit code of `unzip -v`
    let has_unzip = Command::new("unzip")
        .arg("-v")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if has_unzip {
        // Use system unzip for thorough verification (CRC, headers, ordering)
        let zip_str = path.to_string_lossy().into_owned();
        let out = Command::new("unzip")
            .args(["-t", &zip_str])
            .output()
            .expect("unzip -t failed");
        assert!(
            out.status.success(),
            "unzip -t failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let output = unsafe { std::str::from_utf8_unchecked(&out.stdout) };

        let count = output
            .lines()
            .filter(|l| l.starts_with("    testing:"))
            .count();

        assert_eq!(
            count, expected_entries,
            "unzip reports {count} entries, expected {expected_entries}"
        );
    } else {
        // Self-contained verification: scan for local file headers
        let buf = tokio::fs::read(path)
            .await
            .expect("failed to read zip file");
        let lfh_count = buf.windows(4).filter(|w| w == b"PK\x03\x04").count();

        // Also check for EOCDR signature to ensure it's a valid archive
        let has_eocdr = buf.windows(4).rposition(|w| w == b"PK\x05\x06").is_some();
        assert!(
            has_eocdr,
            "no EOCDR signature found — not a valid ZIP archive"
        );

        assert_eq!(
            lfh_count, expected_entries,
            "found {lfh_count} local file headers, expected {expected_entries}"
        );
    }
}

/// Directory for output zip files. Each test writes to a separate file here.
fn zip_out_dir() -> PathBuf {
    Path::new("/tmp").join("comprehensive_test")
}

/// Full path to the zip archive for a given label.
fn zip_out_path(label: &str) -> PathBuf {
    zip_out_dir().join(format!("{label}.zip"))
}

/// Ensure the output directory exists and is empty before running tests.
async fn ensure_dir_clean(out_dir: &PathBuf) {
    if out_dir.exists() {
        let mut dir = tokio::fs::read_dir(out_dir)
            .await
            .expect("failed to read output directory");
        while let Some(entry) = dir
            .next_entry()
            .await
            .expect("failed to read directory entry")
        {
            let path = entry.path();
            if path.is_dir() {
                tokio::fs::remove_dir_all(&path)
                    .await
                    .expect("failed to remove subdirectory");
            } else {
                tokio::fs::remove_file(&path)
                    .await
                    .expect("failed to remove file");
            }
        }
    } else {
        tokio::fs::create_dir_all(out_dir)
            .await
            .expect("failed to create output directory");
    }
}
