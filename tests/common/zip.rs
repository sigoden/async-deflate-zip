use std::path::PathBuf;

use tempfile::TempDir;

/// Return a (TempDir, zip_path) pair. The directory is kept alive until
/// the TempDir is dropped, so the zip file remains accessible.
pub fn zip_path(name: &str) -> (TempDir, PathBuf) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join(format!("{name}.zip"));
    (dir, path)
}
