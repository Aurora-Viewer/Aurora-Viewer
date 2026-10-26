//! Writing a photo to disk on a worker, with its progress
//! (LLViewerWindow::saveImageLocal / LLImageFormatted::save): RGBA from the
//! renderer, RGB in the file (RGBA in the PNG of a cut-out photo,
//! `isolate`), PNG row by row so the window can show how far it is, JPEG
//! at its quality, BMP. The file is written beside its final
//! name and renamed once complete: a photo is never seen half written.

use super::FileFormat;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

/// Side of the thumbnail given back for the gallery.
pub const THUMB_SIDE: u32 = 192;

pub struct Job {
    pub width: u32,
    pub height: u32,
    /// RGBA8, rows top to bottom.
    pub rgba: Vec<u8>,
    pub format: FileFormat,
    pub jpeg_quality: u8,
    pub path: PathBuf,
    /// A cut-out photo: the PNG keeps the alpha.
    pub alpha: bool,
}

/// What a finished save gives back.
pub struct Saved {
    pub path: PathBuf,
    pub bytes: u64,
    /// RGBA8 thumbnail (width, height, pixels) for the gallery.
    pub thumb: (u32, u32, Vec<u8>),
    pub millis: u128,
}

/// A save in progress.
pub struct Saving {
    progress: Arc<AtomicU32>,
    rx: crossbeam_channel::Receiver<Result<Saved, String>>,
}

impl Saving {
    pub fn start(job: Job) -> Saving {
        let progress = Arc::new(AtomicU32::new(0));
        let (tx, rx) = crossbeam_channel::bounded(1);
        let p = progress.clone();
        let spawned = std::thread::Builder::new().name("aurora-photo-save".into()).spawn(move || {
            let _ = tx.send(save(job, &p));
        });
        if let Err(e) = spawned {
            log::warn!("photo: cannot start the save thread: {e}");
        }
        Saving { progress, rx }
    }

    /// 0..=1.
    pub fn fraction(&self) -> f32 {
        self.progress.load(Ordering::Relaxed) as f32 / 10_000.0
    }

    pub fn poll(&self) -> Option<Result<Saved, String>> {
        match self.rx.try_recv() {
            Ok(r) => Some(r),
            Err(crossbeam_channel::TryRecvError::Empty) => None,
            Err(crossbeam_channel::TryRecvError::Disconnected) => Some(Err("save thread ended".into())),
        }
    }
}

fn set(progress: &AtomicU32, f: f32) {
    progress.store((f.clamp(0.0, 1.0) * 10_000.0) as u32, Ordering::Relaxed);
}

pub fn save(job: Job, progress: &AtomicU32) -> Result<Saved, String> {
    let t0 = std::time::Instant::now();
    let Job {
        width,
        height,
        rgba,
        format,
        jpeg_quality,
        path,
        alpha,
    } = job;
    if rgba.len() != width as usize * height as usize * 4 {
        return Err(format!("photo: {} bytes for {width} x {height}", rgba.len()));
    }
    let thumb = thumbnail(width, height, &rgba);
    set(progress, 0.05);
    let rgb: Vec<u8> = if alpha && format == FileFormat::Png {
        rgba
    } else {
        rgba.as_chunks::<4>().0.iter().flat_map(|p| [p[0], p[1], p[2]]).collect()
    };
    set(progress, 0.1);
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    }
    let part = part_path(&path);
    let written = (|| -> Result<(), String> {
        let file = std::fs::File::create(&part).map_err(|e| format!("{}: {e}", part.display()))?;
        let mut out = BufWriter::with_capacity(1 << 20, file);
        encode(&mut out, width, height, &rgb, format, jpeg_quality, progress)?;
        out.flush().map_err(|e| e.to_string())?;
        out.into_inner().map_err(|e| e.to_string())?.sync_all().map_err(|e| e.to_string())
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_file(&part);
        return Err(e);
    }
    std::fs::rename(&part, &path).map_err(|e| {
        let _ = std::fs::remove_file(&part);
        format!("{}: {e}", path.display())
    })?;
    set(progress, 1.0);
    let bytes = std::fs::metadata(&path).map_or(0, |m| m.len());
    Ok(Saved {
        path,
        bytes,
        thumb,
        millis: t0.elapsed().as_millis(),
    })
}

fn part_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".part");
    path.with_file_name(name)
}

/// Encode RGB8 rows (or RGBA8 ones for a PNG, by their length); progress
/// from 0.1 to 0.98.
pub fn encode(
    out: &mut impl Write,
    width: u32,
    height: u32,
    rgb: &[u8],
    format: FileFormat,
    jpeg_quality: u8,
    progress: &AtomicU32,
) -> Result<(), String> {
    match format {
        FileFormat::Png => {
            let channels = if rgb.len() == width as usize * height as usize * 4 { 4 } else { 3 };
            let mut enc = png::Encoder::new(&mut *out, width, height);
            enc.set_color(if channels == 4 { png::ColorType::Rgba } else { png::ColorType::Rgb });
            enc.set_depth(png::BitDepth::Eight);
            enc.set_compression(png::Compression::Fast);
            let mut writer = enc.write_header().map_err(|e| e.to_string())?;
            let mut stream = writer.stream_writer().map_err(|e| e.to_string())?;
            let row = width as usize * channels;
            // a band of rows at a time, the progress after each
            for (i, band) in rgb.chunks(row * 64).enumerate() {
                stream.write_all(band).map_err(|e| e.to_string())?;
                let rows = ((i + 1) * 64).min(height as usize);
                set(progress, 0.1 + 0.88 * rows as f32 / height.max(1) as f32);
            }
            stream.finish().map_err(|e| e.to_string())
        }
        FileFormat::Jpeg => {
            let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut *out, jpeg_quality.clamp(1, 100));
            let r = image::ImageEncoder::write_image(enc, rgb, width, height, image::ExtendedColorType::Rgb8);
            set(progress, 0.98);
            r.map_err(|e| e.to_string())
        }
        FileFormat::Bmp => {
            let enc = image::codecs::bmp::BmpEncoder::new(&mut *out);
            let r = image::ImageEncoder::write_image(enc, rgb, width, height, image::ExtendedColorType::Rgb8);
            set(progress, 0.98);
            r.map_err(|e| e.to_string())
        }
    }
}

/// A JPEG in memory: a postcard (LLPanelSnapshotPostcard's
/// SNAPSHOT_FORMAT_JPEG).
pub fn encode_jpeg(width: u32, height: u32, rgba: &[u8], quality: u8) -> Result<Vec<u8>, String> {
    if rgba.len() != width as usize * height as usize * 4 {
        return Err(format!("photo: {} bytes for {width} x {height}", rgba.len()));
    }
    let rgb: Vec<u8> = rgba.as_chunks::<4>().0.iter().flat_map(|p| [p[0], p[1], p[2]]).collect();
    let mut out = Vec::new();
    encode(&mut out, width, height, &rgb, FileFormat::Jpeg, quality, &AtomicU32::new(0))?;
    Ok(out)
}

/// A small copy for the gallery, its long side THUMB_SIDE.
pub fn thumbnail(width: u32, height: u32, rgba: &[u8]) -> (u32, u32, Vec<u8>) {
    downscale(width, height, rgba, THUMB_SIDE)
}

/// A copy whose long side is at most `max_side` (box filter over whole
/// pixels: fast, and a photo reduced several times needs no better). The
/// colour is weighted by the alpha: the empty pixels of a cut-out photo
/// (black, transparent) darken none of its edges.
pub fn downscale(width: u32, height: u32, rgba: &[u8], max_side: u32) -> (u32, u32, Vec<u8>) {
    let scale = max_side as f32 / width.max(height).max(1) as f32;
    if scale >= 1.0 {
        return (width, height, rgba.to_vec());
    }
    let tw = ((width as f32 * scale).round() as u32).max(1);
    let th = ((height as f32 * scale).round() as u32).max(1);
    let mut out = vec![0u8; (tw * th * 4) as usize];
    for ty in 0..th {
        let y0 = ty * height / th;
        let y1 = ((ty + 1) * height / th).max(y0 + 1).min(height);
        for tx in 0..tw {
            let x0 = tx * width / tw;
            let x1 = ((tx + 1) * width / tw).max(x0 + 1).min(width);
            let mut sum = [0u64; 4];
            for y in y0..y1 {
                let start = ((y * width + x0) * 4) as usize;
                for px in rgba[start..start + ((x1 - x0) * 4) as usize].as_chunks::<4>().0 {
                    let a = u64::from(px[3]);
                    for c in 0..3 {
                        sum[c] += u64::from(px[c]) * a;
                    }
                    sum[3] += a;
                }
            }
            let n = u64::from((y1 - y0) * (x1 - x0)).max(1);
            let o = ((ty * tw + tx) * 4) as usize;
            for c in 0..3 {
                out[o + c] = sum[c].checked_div(sum[3]).unwrap_or(0) as u8;
            }
            out[o + 3] = (sum[3] / n) as u8;
        }
    }
    (tw, th, out)
}
