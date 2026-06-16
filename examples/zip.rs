//! CLI tool: compress files/directories into a ZIP archive.
//!
//! Usage:
//!   zip [-[0-9]] output-file target-path...
//!
//! - `compression`   : `-0` (store) through `-9` (best), default `-6`
//! - `output-file`   : Output ZIP path
//! - `target-path...`: Files or directories to include
//!
//! Examples:
//!   zip -1 test.zip Cargo.toml
//!   zip test.zip examples
//!   zip test.zip Cargo.toml examples

use async_deflate_zip::{CompressionLevel, EntryOptions, ZipWriter};
use std::env;
use std::path::{Path, PathBuf};
use tokio::fs;
use tokio::io::AsyncWriteExt;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 || ["-h", "--help"].contains(&args[1].as_str()) {
        eprintln!("Usage: zip [-[0-9]] output-file target-path...");
        std::process::exit(1);
    }

    let mut compression_level = CompressionLevel::default();
    let mut output_path: Option<PathBuf> = None;
    let mut targets: Vec<PathBuf> = Vec::new();

    let args_iter = args[1..].iter();
    for arg in args_iter {
        if let Some(level_str) = arg.strip_prefix('-') {
            if level_str.len() == 1 && level_str.as_bytes()[0].is_ascii_digit() {
                let level: u8 = level_str.parse().unwrap();
                if level > 9 {
                    eprintln!("Error: Invalid compression level: {level} (valid range 0-9)");
                    std::process::exit(1);
                }
                compression_level = CompressionLevel::new(level);
                continue;
            }
            eprintln!("Error: unrecognized flag '{arg}'");
            std::process::exit(1);
        }
        if output_path.is_none() {
            output_path = Some(PathBuf::from(arg));
        } else {
            targets.push(PathBuf::from(arg));
        }
    }

    let mut output_path = match output_path {
        Some(p) => p,
        None => {
            eprintln!("Error: output file not specified");
            std::process::exit(1);
        }
    };
    if targets.is_empty() && output_path.is_dir() {
        targets.push(output_path.clone());
        output_path = PathBuf::from(format!("{}.zip", output_path.display()));
    }
    if targets.is_empty() {
        eprintln!("Error: no target paths specified");
        std::process::exit(1);
    }

    let file = fs::File::create(&output_path).await.unwrap();
    let mut zip = ZipWriter::new(file).with_compression_level(compression_level);

    if let Err(e) = add_targets(&mut zip, &targets).await {
        eprintln!("Error: {e}");
        let _ = fs::remove_file(&output_path).await;
        std::process::exit(1);
    }

    zip.finish().await.unwrap();

    eprintln!("Created '{}' successfully", output_path.display());
}

async fn add_targets<W: AsyncWriteExt + Unpin>(
    zip: &mut ZipWriter<W>,
    targets: &[PathBuf],
) -> Result<(), Box<dyn std::error::Error>> {
    if targets.len() == 1 && targets[0].is_dir() {
        return Box::pin(add_dir(zip, &targets[0], &targets[0], None)).await;
    }

    for target in targets {
        let entry_options = EntryOptions::from_path(target).await?;
        let target_name = target
            .file_name()
            .unwrap_or_else(|| target.as_os_str())
            .to_string_lossy()
            .into_owned();
        if target.is_dir() {
            zip.add_directory(&target_name, &entry_options).await?;
            Box::pin(add_dir(zip, target, target, Some(&target_name))).await?;
        } else if target.is_file() {
            let mut entry = zip.start_file(&target_name, &entry_options).await?;
            let mut file = fs::File::open(target).await?;
            tokio::io::copy(&mut file, &mut entry).await?;
            entry.finish().await?;
        } else if target.is_symlink() {
            let link_target = fs::read_link(target).await?;
            zip.add_symlink(&target_name, &link_target.to_string_lossy(), &entry_options)
                .await?;
        }
    }

    Ok(())
}

/// Recursively add the contents of `dir` into the ZIP archive.
async fn add_dir<W: AsyncWriteExt + Unpin>(
    zip: &mut ZipWriter<W>,
    base: &Path,
    dir: &Path,
    prefix: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut read_dir = fs::read_dir(dir).await?;

    while let Some(entry) = read_dir.next_entry().await? {
        let path = entry.path();
        let relative = match prefix {
            Some(p) => format!("{}/{}", p, extract_relative_path(base, &path)),
            None => extract_relative_path(base, &path),
        };

        let file_type = entry.file_type().await?;
        let entry_options = EntryOptions::from_path(&path).await?;
        if file_type.is_dir() {
            zip.add_directory(&relative, &entry_options).await?;
            Box::pin(add_dir(zip, base, &path, prefix)).await?;
        } else if file_type.is_file() {
            let mut entry = zip.start_file(&relative, &entry_options).await?;
            let mut file = fs::File::open(&path).await?;
            tokio::io::copy(&mut file, &mut entry).await?;
            entry.finish().await?;
        } else if file_type.is_symlink() {
            let link_target = fs::read_link(&path).await?;
            let target_str = extract_relative_path(base, &link_target);
            zip.add_symlink(&relative, &target_str, &entry_options)
                .await?;
        }
    }

    Ok(())
}

fn extract_relative_path(base: &Path, path: &Path) -> String {
    path.strip_prefix(base)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}
