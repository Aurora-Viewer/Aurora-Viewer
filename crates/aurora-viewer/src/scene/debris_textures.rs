//! Aurora Weather's debris: the user's own pictures for the leaves and the
//! paper (the Debris card's "Pictures", up to [`MAX`] each), image files on
//! this computer, kept by path in the Weather settings and never sent
//! anywhere. Each is read and made small on a worker thread (at most
//! [`SIDE`] texels a side, with its mips), then given a bindless slot; the
//! debris draws its cut-out (its alpha) instead of the drawn shapes, each
//! particle taking one of them at random. With none, the drawn shapes.

use aurora_render::{MipLevel, Renderer};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, mpsc};

/// The most pictures of each kind.
pub const MAX: usize = 8;
/// Their largest side once read (texels): a leaf or a sheet of paper is
/// small on screen.
const SIDE: u32 = 256;

type Mips = Vec<(u32, u32, Vec<u8>)>;

#[derive(Default)]
pub struct DebrisTextures {
    /// The slots of the pictures read, by path.
    slots: HashMap<PathBuf, u32>,
    /// Those being read (their results come through `done`).
    reading: Vec<PathBuf>,
    /// Those that could not be read (not tried again until the list
    /// changes).
    failed: Vec<PathBuf>,
    /// (the receiver in a mutex: the scene is shared between threads)
    done: Option<(
        mpsc::Sender<(PathBuf, Option<Mips>)>,
        Mutex<mpsc::Receiver<(PathBuf, Option<Mips>)>>,
    )>,
}

impl DebrisTextures {
    /// The slots of `leaves` and `paper` as far as they are read (0 for
    /// those still being read or that failed), reading the new ones and
    /// letting go of the pictures no longer listed.
    pub fn update(&mut self, renderer: &mut Renderer, leaves: &[PathBuf], paper: &[PathBuf]) -> [[u32; MAX]; 2] {
        let (tx, rx) = self.done.get_or_insert_with(|| {
            let (tx, rx) = mpsc::channel();
            (tx, Mutex::new(rx))
        });
        let rx = rx.get_mut().unwrap_or_else(|e| e.into_inner());
        // what was read
        while let Ok((path, mips)) = rx.try_recv() {
            self.reading.retain(|p| *p != path);
            let slot = mips.and_then(|mips| {
                let levels: Vec<MipLevel> = mips
                    .iter()
                    .map(|(width, height, data)| MipLevel {
                        width: *width,
                        height: *height,
                        data,
                    })
                    .collect();
                renderer.create_texture(&levels)
            });
            match slot {
                Some(slot) => {
                    self.slots.insert(path, slot);
                }
                None => {
                    log::warn!("debris pictures: {} could not be read", path.display());
                    self.failed.push(path);
                }
            }
        }
        let wanted: Vec<&PathBuf> = leaves.iter().take(MAX).chain(paper.iter().take(MAX)).collect();
        // gone from the lists: let go
        let unwanted: Vec<PathBuf> = self.slots.keys().filter(|p| !wanted.contains(p)).cloned().collect();
        for path in unwanted {
            if let Some(slot) = self.slots.remove(&path) {
                renderer.free_texture(slot);
            }
        }
        self.failed.retain(|p| wanted.contains(&p));
        // new: read them
        for path in wanted {
            if self.slots.contains_key(path) || self.reading.contains(path) || self.failed.contains(path) {
                continue;
            }
            self.reading.push(path.clone());
            let (path, tx) = (path.clone(), tx.clone());
            rayon::spawn(move || {
                let mips = read(&path);
                let _ = tx.send((path, mips));
            });
        }
        let pick = |list: &[PathBuf]| {
            let mut out = [0u32; MAX];
            let mut n = 0;
            for p in list.iter().take(MAX) {
                if let Some(&slot) = self.slots.get(p) {
                    out[n] = slot;
                    n += 1;
                }
            }
            out
        };
        [pick(leaves), pick(paper)]
    }

    /// Everything goes (the renderer's slots with it).
    pub fn clear(&mut self, renderer: &mut Renderer) {
        for (_, slot) in self.slots.drain() {
            renderer.free_texture(slot);
        }
        self.reading.clear();
        self.failed.clear();
        self.done = None;
    }
}

/// An image file read and made small, with its mips (RGBA8); None if it
/// cannot be read.
fn read(path: &Path) -> Option<Mips> {
    let image = image::ImageReader::open(path).ok()?.with_guessed_format().ok()?.decode().ok()?;
    let image = if image.width().max(image.height()) > SIDE {
        image.resize(SIDE, SIDE, image::imageops::FilterType::Triangle)
    } else {
        image
    };
    let rgba = image.to_rgba8();
    let (w, h) = rgba.dimensions();
    if w == 0 || h == 0 {
        return None;
    }
    Some(aurora_render::build_mips(w, h, rgba.into_raw(), u32::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_is_read_small_with_its_mips_and_a_bad_one_is_not() {
        let dir = std::env::temp_dir().join(format!("aurora-debris-pic-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("leaf.png");
        image::RgbaImage::from_pixel(600, 300, image::Rgba([40, 120, 30, 200]))
            .save(&path)
            .expect("written");
        let mips = read(&path).expect("read");
        assert_eq!((mips[0].0, mips[0].1), (256, 128));
        assert_eq!(mips.last().map(|m| (m.0, m.1)), Some((1, 1)));
        let bad = dir.join("not-an-image.png");
        std::fs::write(&bad, b"nothing").expect("written");
        assert!(read(&bad).is_none());
        assert!(read(&dir.join("missing.png")).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
