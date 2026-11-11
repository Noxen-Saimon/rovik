use std::fs::File;
use std::io::{Read, Write};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

const CREATE_NO_WINDOW: u32 = 0x08000000;

pub fn find_7z_binary() -> Option<PathBuf> {
    let candidates = [
        r"C:\Program Files\7-Zip\7z.exe",
        r"C:\Program Files (x86)\7-Zip\7z.exe",
        r"C:\Windows\System32\7z.exe",
    ];
    for c in &candidates {
        let p = PathBuf::from(c);
        if p.exists() {
            return Some(p);
        }
    }
    if let Ok(out) = Command::new("7z.exe")
        .arg("/?")
        .creation_flags(CREATE_NO_WINDOW)
        .output()
    {
        if out.status.success() || !out.stdout.is_empty() {
            return Some(PathBuf::from("7z.exe"));
        }
    }
    None
}

pub fn is_archive_file(path: &Path) -> bool {
    let ext = match path.extension().and_then(|e| e.to_str()) {
        Some(e) => e.to_ascii_lowercase(),
        None => return false,
    };
    matches!(
        ext.as_str(),
        "zip" | "7z" | "rar" | "tar" | "gz" | "xz" | "bz2" | "tgz" | "iso" | "cab"
    )
}

pub fn compress(sources: &[PathBuf], dest_archive: &Path) -> Result<(), String> {
    if sources.is_empty() {
        return Err("No files selected for compression.".to_string());
    }

    if let Some(seven_z) = find_7z_binary() {
        let mut cmd = Command::new(seven_z);
        cmd.arg("a");
        if dest_archive
            .extension()
            .map_or(false, |e| e.eq_ignore_ascii_case("zip"))
        {
            cmd.arg("-tzip");
        } else {
            cmd.arg("-t7z");
        }
        cmd.arg(dest_archive);
        for s in sources {
            cmd.arg(s);
        }
        cmd.arg("-y");
        cmd.creation_flags(CREATE_NO_WINDOW);

        if let Ok(status) = cmd.status() {
            if status.success() {
                return Ok(());
            }
        }
    }

    compress_to_zip(sources, dest_archive)
}

pub fn extract(archive: &Path, dest_folder: &Path) -> Result<(), String> {
    if let Some(seven_z) = find_7z_binary() {
        let mut cmd = Command::new(seven_z);
        cmd.arg("x")
            .arg(archive)
            .arg(format!("-o{}", dest_folder.display()))
            .arg("-y")
            .creation_flags(CREATE_NO_WINDOW);

        if let Ok(status) = cmd.status() {
            if status.success() {
                return Ok(());
            }
        }
    }

    extract_zip(archive, dest_folder)
}

pub fn compress_to_zip(sources: &[PathBuf], dest_zip: &Path) -> Result<(), String> {
    if sources.is_empty() {
        return Err("No files selected for compression.".to_string());
    }

    let file = File::create(dest_zip).map_err(|e| format!("Failed to create zip file: {e}"))?;
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    let base_dir = if sources.len() == 1 && sources[0].is_dir() {
        sources[0].parent().unwrap_or(Path::new("")).to_path_buf()
    } else {
        sources[0].parent().unwrap_or(Path::new("")).to_path_buf()
    };

    let mut buffer = vec![0u8; 64 * 1024];

    fn add_dir_recursive(
        dir: &Path,
        base_dir: &Path,
        zip: &mut ZipWriter<File>,
        options: SimpleFileOptions,
        buf: &mut [u8],
    ) -> Result<(), String> {
        let entries = std::fs::read_dir(dir).map_err(|e| format!("Read dir error: {e}"))?;
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            let relative = path
                .strip_prefix(base_dir)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");

            if relative.is_empty() {
                continue;
            }

            if path.is_dir() {
                let dir_name = if relative.ends_with('/') {
                    relative
                } else {
                    format!("{relative}/")
                };
                zip.add_directory(&dir_name, options)
                    .map_err(|e| format!("Zip error: {e}"))?;
                add_dir_recursive(&path, base_dir, zip, options, buf)?;
            } else if path.is_file() {
                zip.start_file(&relative, options)
                    .map_err(|e| format!("Zip error: {e}"))?;
                let mut f = File::open(&path).map_err(|e| format!("Cannot read file: {e}"))?;
                loop {
                    let n = f.read(buf).map_err(|e| format!("Read error: {e}"))?;
                    if n == 0 {
                        break;
                    }
                    zip.write_all(&buf[..n])
                        .map_err(|e| format!("Write error: {e}"))?;
                }
            }
        }
        Ok(())
    }

    for src in sources {
        if src.is_dir() {
            let rel = src
                .strip_prefix(&base_dir)
                .unwrap_or(src)
                .to_string_lossy()
                .replace('\\', "/");
            let dir_name = if rel.ends_with('/') {
                rel
            } else {
                format!("{rel}/")
            };
            zip.add_directory(&dir_name, options)
                .map_err(|e| format!("Zip error: {e}"))?;
            add_dir_recursive(src, &base_dir, &mut zip, options, &mut buffer)?;
        } else if src.is_file() {
            let relative = src
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .replace('\\', "/");
            zip.start_file(&relative, options)
                .map_err(|e| format!("Zip error: {e}"))?;
            let mut f = File::open(src).map_err(|e| format!("Cannot read file: {e}"))?;
            loop {
                let n = f.read(&mut buffer).map_err(|e| format!("Read error: {e}"))?;
                if n == 0 {
                    break;
                }
                zip.write_all(&buffer[..n])
                    .map_err(|e| format!("Write error: {e}"))?;
            }
        }
    }

    zip.finish().map_err(|e| format!("Finalize zip error: {e}"))?;
    Ok(())
}

pub fn extract_zip(zip_file: &Path, dest_folder: &Path) -> Result<(), String> {
    let file = File::open(zip_file).map_err(|e| format!("Failed to open zip file: {e}"))?;
    let mut archive = ZipArchive::new(file).map_err(|e| format!("Invalid zip archive: {e}"))?;

    std::fs::create_dir_all(dest_folder).map_err(|e| format!("Failed to create folder: {e}"))?;

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("Failed reading entry {i}: {e}"))?;
        let outpath = match entry.enclosed_name() {
            Some(path) => dest_folder.join(path),
            None => continue,
        };

        if entry.is_dir() {
            std::fs::create_dir_all(&outpath)
                .map_err(|e| format!("Failed creating directory: {e}"))?;
        } else {
            if let Some(p) = outpath.parent() {
                if !p.exists() {
                    std::fs::create_dir_all(p)
                        .map_err(|e| format!("Failed creating parent directory: {e}"))?;
                }
            }
            let mut outfile =
                File::create(&outpath).map_err(|e| format!("Failed creating file: {e}"))?;
            std::io::copy(&mut entry, &mut outfile)
                .map_err(|e| format!("Failed extracting file: {e}"))?;
        }
    }

    Ok(())
}
