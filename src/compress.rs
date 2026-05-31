use std::io;
use std::path::Path;

use tokio::io::AsyncWrite;

use crate::writer::ZipWriter;

/// Compress a directory into a ZIP file.
///
/// Recursively walks the directory tree asynchronously and compresses each
/// file into a standard ZIP archive. Directories are stored as placeholder
/// entries (using [`ZipWriter::append_directory`]).
///
/// # Arguments
///
/// * `src` — Path to the source directory to compress.
/// * `dest` — Path to the output ZIP file to create.
///
/// # Errors
///
/// Returns [`std::io::Error`] if:
/// - The source directory cannot be read or canonicalized
/// - The destination file cannot be created
/// - Any file within the directory cannot be read or written
///
/// # Example
///
/// ```rust,no_run
/// use async_deflate_zip::compress_dir;
///
/// # async fn example() {
/// compress_dir("./my_folder", "./output.zip").await.unwrap();
/// # }
/// ```
pub async fn compress_dir(src: impl AsRef<Path>, dest: impl AsRef<Path>) -> std::io::Result<()> {
    let file = tokio::fs::File::create(dest.as_ref()).await?;
    let mut zip = ZipWriter::new(file);

    let src = src.as_ref().canonicalize()?;
    compress_dir_recursive(&mut zip, &src, &src).await?;

    zip.finalize().await?;
    Ok(())
}

/// Recursively walk a directory and add all entries to the ZIP archive.
///
/// Internal helper. Visits every file and subdirectory, adding each to
/// the `ZipWriter` with paths relative to `base`.
async fn compress_dir_recursive<W: AsyncWrite + Unpin>(
    zip: &mut ZipWriter<W>,
    base: &Path,
    dir: &Path,
) -> io::Result<()> {
    let mut read_dir = tokio::fs::read_dir(dir).await?;
    while let Some(entry) = read_dir.next_entry().await? {
        let path = entry.path();
        let name = path.strip_prefix(base).unwrap();
        let name_str = name.to_string_lossy().replace('\\', "/");

        if entry.file_type().await?.is_dir() {
            zip.append_directory(&format!("{}/", name_str)).await?;
            Box::pin(compress_dir_recursive(zip, base, &path)).await?;
        } else {
            let mut file = tokio::fs::File::open(&path).await?;
            let mut entry = zip.append_file(&name_str).await?;
            tokio::io::copy(&mut file, &mut entry).await?;
            entry.close().await?;
        }
    }
    Ok(())
}

// === Tests ===

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_compress_dir_integration() {
        use std::path::PathBuf;

        // Create a temp directory structure
        let tmp = PathBuf::from("/tmp/test_compress_dir");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("sub")).unwrap();
        std::fs::write(tmp.join("a.txt"), b"file a").unwrap();
        std::fs::write(tmp.join("sub/b.txt"), b"file b").unwrap();

        let zip_path = PathBuf::from("/tmp/test_compress_dir.zip");
        compress_dir(&tmp, &zip_path).await.unwrap();

        // Read and verify
        let data = std::fs::read(&zip_path).unwrap();
        assert!(data.windows(4).any(|w| w == b"PK\x03\x04"));
        assert!(data.windows(4).any(|w| w == b"PK\x01\x02"));
        assert!(data.windows(4).any(|w| w == b"PK\x05\x06"));

        // Cleanup
        let _ = std::fs::remove_dir_all(&tmp);
        let _ = std::fs::remove_file(&zip_path);
    }
}
