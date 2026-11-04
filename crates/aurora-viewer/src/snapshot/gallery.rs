//! The latest photos of the photo folder, newest first, with thumbnails
//! read on a worker (Aurora's own: Firestorm only shows the last one).

use super::save::thumbnail;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Photos shown in the window.
pub const SHOWN: usize = 4;

/// The photo files of `folder`, newest first.
pub fn recent(folder: &Path, count: usize) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut files: Vec<(SystemTime, PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            super::FileFormat::of_path(&path)?;
            let meta = e.metadata().ok()?;
            meta.is_file().then(|| (meta.modified().unwrap_or(SystemTime::UNIX_EPOCH), path))
        })
        .collect();
    files.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    files.into_iter().take(count).map(|(_, p)| p).collect()
}

/// The thumbnail of a photo file (RGBA8), None when it cannot be read.
pub fn load_thumbnail(path: &Path) -> Option<(u32, u32, Vec<u8>)> {
    let img = image::ImageReader::open(path).ok()?.with_guessed_format().ok()?.decode().ok()?;
    let rgba = img.to_rgba8();
    Some(thumbnail(rgba.width(), rgba.height(), rgba.as_raw()))
}

/// Show a folder in the system's file manager, the file selected when one
/// is given.
pub fn reveal(folder: &Path, file: Option<&Path>) {
    let _ = std::fs::create_dir_all(folder);
    let result = if cfg!(windows) {
        let mut cmd = std::process::Command::new("explorer");
        match file {
            Some(f) => select_arg(&mut cmd, f),
            None => {
                cmd.arg(folder);
            }
        }
        crate::open_path::spawn(&mut cmd)
    } else if cfg!(target_os = "macos") {
        let mut cmd = std::process::Command::new("open");
        match file {
            Some(f) => cmd.arg("-R").arg(f),
            None => cmd.arg(folder),
        };
        crate::open_path::spawn(&mut cmd)
    } else {
        crate::open_path::open(folder)
    };
    if let Err(e) = result {
        log::warn!("photo: cannot open {}: {e}", folder.display());
    }
}

/// Explorer's `/select,"<file>"`, written as it reads it: given as one
/// argument, the standard quoting wraps the whole of it in quotes, which
/// Explorer misreads when the path holds a space (the pictures folder's
/// "Aurora Viewer") and opens another folder instead.
#[cfg(windows)]
fn select_arg(cmd: &mut std::process::Command, file: &Path) {
    use std::os::windows::process::CommandExt;
    cmd.raw_arg(format!("/select,\"{}\"", file.display()));
}

#[cfg(not(windows))]
fn select_arg(cmd: &mut std::process::Command, file: &Path) {
    cmd.arg(format!("/select,{}", file.display()));
}

/// Open a photo with the system's viewer.
pub fn open(path: &Path) {
    if let Err(e) = crate::open_path::open(path) {
        log::warn!("photo: cannot open {}: {e}", path.display());
    }
}
