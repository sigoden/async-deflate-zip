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

use async_deflate_zip::{CompressionLevel, ZipWriter};
use std::env;
use std::path::{Path, PathBuf};
use tokio::fs;
use tokio::io::AsyncWriteExt;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 || ["-h", "--help"].contains(&args[1].as_str()) {
        eprintln!("Usage: zip target-dir [-0-9] [output-file]");
        std::process::exit(1);
    }

    let target_dir = PathBuf::from(&args[1]);

    let mut compression_level = CompressionLevel::DEFAULT;
    let mut output_path: Option<PathBuf> = None;

    for arg in &args[2..] {
        if let Some(level_str) = arg.strip_prefix('-') {
            if level_str.len() == 1 && level_str.as_bytes()[0].is_ascii_digit() {
                let level: u8 = level_str.parse().unwrap();
                match CompressionLevel::try_new(level) {
                    Ok(l) => compression_level = l,
                    Err(e) => {
                        eprintln!("Error: Invalid compression level: {e}");
                        std::process::exit(1);
                    }
                }
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
    let mut zip = ZipWriter::new(file).with_compression_level(compression_level);

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
            zip.append_directory(&dir_path).await?.close().await?;
            Box::pin(add_dir(zip, base, &path)).await?;
        } else if file_type.is_file() {
            let mut file = fs::File::open(&path).await?;
            let mut entry = zip.append_file(&relative).await?;
            tokio::io::copy(&mut file, &mut entry).await?;
            entry.close().await?;
        } else if file_type.is_symlink() {
            let target = fs::read_link(&path).await?;
            let mut entry = zip.append_file(&relative).await?;
            entry.write_all(target.to_string_lossy().as_bytes()).await?;
            entry.close().await?;
        }
    }

    Ok(())
}
