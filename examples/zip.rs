//! CLI tool: compress a directory into a ZIP archive.
//!
//! Usage:
//!   zip target-dir [-0-9] [output-file]
//!
//! - `target-dir`    : Directory to compress (required)
//! - `compression`   : `-0` (store) through `-9` (best), default `-6`
//! - `output-file`   : Output ZIP path; defaults to `<dirname>.zip`
//!
//! Examples:
//!   zip ./my-project                 → my-project.zip  (level 6)
//!   zip ./my-project -0              → my-project.zip  (store, no compression)
//!   zip ./my-project -9 ./out.zip    → out.zip         (max compression)

use async_deflate_zip::{Compression, ZipWriter};
use std::env;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 || ["-h", "--help"].contains(&args[1].as_str()) {
        eprintln!("Usage: zip target-dir [-0-9] [output-file]");
        std::process::exit(1);
    }

    let target_dir = PathBuf::from(&args[1]);

    let mut compression = Compression::default();
    let mut output_path: Option<PathBuf> = None;

    for arg in &args[2..] {
        if let Some(level_str) = arg.strip_prefix('-') {
            if level_str.len() == 1 && level_str.as_bytes()[0].is_ascii_digit() {
                let level: u8 = level_str.parse().unwrap();
                if level > 9 {
                    eprintln!("Error: Invalid compression level: {level} (valid range 0-9)");
                    std::process::exit(1);
                }
                compression = Compression::new(level as u32);
                continue;
            }
            eprintln!("Warning: ignore unrecognized flag '{arg}'");
            std::process::exit(1);
        }
        output_path = Some(PathBuf::from(arg));
    }

    let output_path = match output_path {
        Some(p) => p,
        None => {
            let dir_name = target_dir
                .file_name()
                .unwrap_or_else(|| target_dir.as_os_str())
                .to_string_lossy()
                .into_owned();
            PathBuf::from(format!("{dir_name}.zip"))
        }
    };

    if !target_dir.is_dir() {
        eprintln!("Error: '{}' is not a directory", target_dir.display());
        std::process::exit(1);
    }

    let file = fs::File::create(&output_path).await.unwrap();
    let mut zip = ZipWriter::new(file).with_level(compression);

    if let Err(e) = add_dir(&mut zip, &target_dir, &target_dir).await {
        eprintln!("Error: {e}");
        let _ = fs::remove_file(&output_path).await;
        std::process::exit(1);
    }

    zip.finalize().await.unwrap();

    eprintln!("Created '{}' successfully", output_path.display());
}

/// Recursively add the contents of `dir` into the ZIP archive.
async fn add_dir<W: AsyncWriteExt + Unpin>(
    zip: &mut ZipWriter<W>,
    base: &Path,
    dir: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut read_dir = fs::read_dir(dir).await?;

    while let Some(entry) = read_dir.next_entry().await? {
        let path = entry.path();
        let relative = path
            .strip_prefix(base)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");

        let file_type = entry.file_type().await?;

        if file_type.is_dir() {
            let dir_path = format!("{relative}/");
            if dir_path == "./" {
                continue;
            }
            let mut dir = zip.append_directory(&dir_path).await?;
            if let Ok(meta) = fs::metadata(&path).await {
                if let Ok(mtime) = meta.modified() {
                    dir.set_mtime(mtime);
                }
                #[cfg(unix)]
                {
                    dir.set_permissions(meta.mode() & 0o7777);
                    dir.set_uid_gid(meta.uid(), meta.gid());
                }
            }
            dir.close().await?;
            Box::pin(add_dir(zip, base, &path)).await?;
        } else if file_type.is_file() {
            let mut file = fs::File::open(&path).await?;
            let mut entry = zip.append_file(&relative).await?;
            if let Ok(meta) = file.metadata().await {
                if let Ok(mtime) = meta.modified() {
                    entry.set_mtime(mtime);
                }
                #[cfg(unix)]
                {
                    entry.set_permissions(meta.mode() & 0o7777);
                    entry.set_uid_gid(meta.uid(), meta.gid());
                }
            }
            // Mark as text if no null bytes in first 8 KB
            let mut probe = [0u8; 8192];
            let n = file.read(&mut probe).await.unwrap_or(0);
            if n > 0 {
                entry.set_text(!probe[..n].contains(&0));
            }
            entry.write_all(&probe[..n]).await?;
            tokio::io::copy(&mut file, &mut entry).await?;
            entry.close().await?;
        } else if file_type.is_symlink() {
            let target = fs::read_link(&path).await?;
            let mut entry = zip.append_file(&relative).await?;
            if let Ok(meta) = fs::symlink_metadata(&path).await {
                if let Ok(mtime) = meta.modified() {
                    entry.set_mtime(mtime);
                }
                #[cfg(unix)]
                {
                    entry.set_permissions(meta.mode() & 0o7777);
                    entry.set_uid_gid(meta.uid(), meta.gid());
                }
            }
            entry.write_all(target.to_string_lossy().as_bytes()).await?;
            entry.close().await?;
        }
    }

    Ok(())
}
