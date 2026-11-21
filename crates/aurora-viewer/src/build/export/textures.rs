//! The texture files beside an exported model: each texture fetched at its
//! full resolution, then written in the chosen format next to the file.
//! Port of ColladaExportFloater::saveTextures and
//! CacheReadResponder::saveTexturesWorker / completed
//! (indra/newview/daeexport.cpp, Latif Khalifa and Cinder Biscuits, LGPL
//! 2.1): a texture that does not arrive within a minute is left out, the
//! others are still written. The Legacy Viewer saves one texture per frame
//! from its texture cache; here three at a time, decoded and encoded on
//! workers.
//!
//! A glTF file holds its textures: `TextureSaver::embed` fetches them the
//! same way but keeps each as PNG bytes in memory, written into the file
//! once they are all here.

use super::gltf::Embedded;
use super::{ImageFormat, TextureFiles};
use crate::scene::Scene;
use crossbeam_channel::Receiver;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// TEXTURE_DOWNLOAD_TIMEOUT.
const TIMEOUT: Duration = Duration::from_secs(60);
/// Textures fetched and written at once.
const PARALLEL: usize = 3;
/// The pixels asked of a texture: its full size (16384 is past the
/// largest texture a grid serves).
const FULL_SIZE_PX: f32 = 16384.0;

struct Job {
    id: Uuid,
    path: PathBuf,
    since: Instant,
    writing: Option<Receiver<Result<Option<Embedded>, String>>>,
}

/// The texture files of one export, written over the next frames.
pub struct TextureSaver {
    format: ImageFormat,
    /// Keep the textures in memory (`embedded`) instead of writing files.
    embed: bool,
    queue: VecDeque<(Uuid, PathBuf)>,
    active: Vec<Job>,
    pub total: usize,
    pub written: usize,
    /// The textures left out, by file name.
    pub failed: Vec<String>,
    /// To keep resident while they are written (drained by the app).
    pub wanted_images: HashSet<Uuid>,
    /// The PNG images of an embedding saver, by texture.
    pub embedded: HashMap<Uuid, Embedded>,
}

impl TextureSaver {
    /// The textures of `files`, beside `model` (their folder).
    pub fn new(files: &TextureFiles, model: &Path, format: ImageFormat) -> Self {
        let folder = model.parent().map(Path::to_path_buf).unwrap_or_default();
        let mut queue: Vec<(Uuid, PathBuf)> = files
            .names
            .iter()
            .map(|(id, name)| (*id, folder.join(format!("{name}.{}", files.extension))))
            .collect();
        queue.sort_by(|a, b| a.1.cmp(&b.1));
        Self {
            format,
            embed: false,
            total: queue.len(),
            queue: queue.into(),
            active: Vec::new(),
            written: 0,
            failed: Vec::new(),
            wanted_images: HashSet::new(),
            embedded: HashMap::new(),
        }
    }

    /// The textures of `files` as PNG in memory, for a glTF file (the path
    /// only names a texture in the log and the missing list).
    pub fn embed(files: &TextureFiles) -> Self {
        let mut saver = Self::new(
            &TextureFiles {
                names: files.names.clone(),
                extension: ImageFormat::Png.extension().into(),
            },
            Path::new(""),
            ImageFormat::Png,
        );
        saver.embed = true;
        saver
    }

    pub fn done(&self) -> bool {
        self.queue.is_empty() && self.active.is_empty()
    }

    /// One frame of work: start, watch and finish the textures.
    pub fn poll(&mut self, ctx: &egui::Context, scene: &mut Scene, now: Instant) {
        while self.active.len() < PARALLEL
            && let Some((id, path)) = self.queue.pop_front()
        {
            self.active.push(Job {
                id,
                path,
                since: now,
                writing: None,
            });
        }
        let (format, embed) = (self.format, self.embed);
        let mut finished = Vec::new();
        for (n, job) in self.active.iter_mut().enumerate() {
            if let Some(rx) = &job.writing {
                if let Ok(result) = rx.try_recv() {
                    finished.push((n, result));
                }
                continue;
            }
            if now.duration_since(job.since) > TIMEOUT {
                log::warn!("object export: texture timed out");
                finished.push((n, Err("timed out".into())));
                continue;
            }
            self.wanted_images.insert(job.id);
            scene.textures.note_usage_fixed(&job.id, FULL_SIZE_PX);
            let source = scene.textures.preview_source(&job.id);
            // the original's pixels (discard 0), never a smaller level
            let complete = source.local.is_some() || source.procedural || source.decoded == Some(0);
            if !complete {
                continue;
            }
            let (tx, rx) = crossbeam_channel::bounded(1);
            job.writing = Some(rx);
            let (id, path, wake) = (job.id, job.path.clone(), ctx.clone());
            scene.jobs.execute(move || {
                let result = if embed {
                    encode_png(id, source).map(Some)
                } else {
                    write(id, source, &path, format).map(|()| None)
                };
                let _ = tx.send(result);
                wake.request_repaint();
            });
        }
        for (n, result) in finished.into_iter().rev() {
            let job = self.active.swap_remove(n);
            match result {
                Ok(image) => {
                    if let Some(image) = image {
                        self.embedded.insert(job.id, image);
                    }
                    self.written += 1;
                }
                Err(reason) => {
                    log::warn!("object export: a texture was not saved: {reason}");
                    let name = job.path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
                    self.failed.push(name);
                }
            }
        }
    }
}

/// A texture decoded and encoded as PNG in memory, for a glTF file; `alpha`
/// when a pixel is not opaque (the material then blends).
fn encode_png(id: Uuid, source: crate::scene::textures::PreviewSource) -> Result<Embedded, String> {
    let image = source.inspector_image(id).ok_or("not decoded")?;
    let mut out = std::io::Cursor::new(Vec::new());
    image::write_buffer_with_format(
        &mut out,
        &image.rgba,
        image.size[0],
        image.size[1],
        image::ColorType::Rgba8,
        image::ImageFormat::Png,
    )
    .map_err(|e| e.to_string())?;
    Ok(Embedded {
        png: out.into_inner(),
        alpha: has_alpha(&image.rgba),
    })
}

/// Whether any RGBA pixel is not fully opaque.
fn has_alpha(rgba: &[u8]) -> bool {
    rgba.as_chunks::<4>().0.iter().any(|px| px[3] < 255)
}

/// CacheReadResponder::completed: JPEG2000 as received, else decoded and
/// encoded in the format.
fn write(id: Uuid, source: crate::scene::textures::PreviewSource, path: &Path, format: ImageFormat) -> Result<(), String> {
    let bytes = match format {
        ImageFormat::J2c => {
            if source.decoded != Some(0) {
                return Err("no JPEG2000 data".into());
            }
            let data = if source.bytes.is_empty() {
                Arc::new(crate::cache::read(&source.jpeg).map_err(|e| e.to_string())?)
            } else {
                source.bytes.clone()
            };
            data.to_vec()
        }
        ImageFormat::Png | ImageFormat::Tga => {
            let image = source.inspector_image(id).ok_or("not decoded")?;
            let mut out = std::io::Cursor::new(Vec::new());
            let kind = if format == ImageFormat::Png {
                image::ImageFormat::Png
            } else {
                image::ImageFormat::Tga
            };
            image::write_buffer_with_format(&mut out, &image.rgba, image.size[0], image.size[1], image::ColorType::Rgba8, kind)
                .map_err(|e| e.to_string())?;
            out.into_inner()
        }
    };
    crate::atomic_file::write(path, &bytes).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_embedding_saver_keeps_png_names_and_no_folder() {
        let files = TextureFiles {
            names: HashMap::from([(Uuid::from_u128(1), "Brick".to_owned())]),
            extension: "tga".into(),
        };
        let saver = TextureSaver::embed(&files);
        assert!(saver.embed);
        assert_eq!(saver.format, ImageFormat::Png);
        assert_eq!(saver.total, 1);
        assert_eq!(saver.queue[0].1, PathBuf::from("Brick.png"));
        assert!(!saver.done());
    }

    #[test]
    fn alpha_is_any_pixel_below_opaque() {
        assert!(!has_alpha(&[1, 2, 3, 255, 4, 5, 6, 255]));
        assert!(has_alpha(&[1, 2, 3, 255, 4, 5, 6, 254]));
        assert!(!has_alpha(&[]));
    }
}
