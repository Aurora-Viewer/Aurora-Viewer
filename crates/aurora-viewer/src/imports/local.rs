//! Local bitmap/material previews, following LLLocalBitmap and
//! LLLocalGLTFMaterial. File checks, decoding, mips and staging are worker jobs.

use super::*;
use aurora_render::{StagedTexture, StagingPool};
use std::time::{Duration, Instant, SystemTime};

pub struct Texture {
    pub id: Uuid,
    pub staged: StagedTexture,
    pub alpha: crate::scene::jobs::AlphaKind,
    pub has_alpha: bool,
}
pub struct Prepared {
    pub entry: Uuid,
    pub generation: u64,
    pub textures: Vec<Texture>,
    pub material: Option<PbrMaterial>,
}
pub type Stamp = Vec<(PathBuf, Option<(SystemTime, u64)>)>;
pub fn stamp(paths: &[PathBuf]) -> Stamp {
    paths
        .iter()
        .map(|path| {
            (
                path.clone(),
                std::fs::metadata(path)
                    .ok()
                    .and_then(|m| m.modified().ok().map(|time| (time, m.len()))),
            )
        })
        .collect()
}
pub struct Watch {
    pub paths: Vec<PathBuf>,
    pub stamp: Stamp,
    pub next: Instant,
    pub pending: bool,
}
impl Watch {
    pub fn new(mut paths: Vec<PathBuf>) -> Self {
        paths.sort();
        paths.dedup();
        Self {
            stamp: stamp(&paths),
            paths,
            next: Instant::now() + Duration::from_secs(3),
            pending: false,
        }
    }
}
fn prepare(id: Uuid, generation: u64, source: &Source, options: import_texture::Options, pool: &StagingPool) -> Result<Prepared, String> {
    let mut prepared = Prepared {
        entry: id,
        generation,
        textures: Vec::new(),
        material: None,
    };
    let mut textures = Vec::new();
    match source {
        Source::Texture(image) => textures.push((id, image.clone())),
        Source::Material { value, images } => {
            let mut value = value.clone();
            let mut ids = [
                value.base_color_texture,
                value.normal_texture,
                value.metallic_roughness_texture,
                value.emissive_texture,
            ];
            for (slot, image) in images.iter().enumerate() {
                if let Some(image) = image
                    && image.asset.is_none()
                {
                    ids[slot] = Some(image.key);
                    textures.push((image.key, image.pixels.clone()));
                }
            }
            value.base_color_texture = ids[0];
            value.normal_texture = ids[1];
            value.metallic_roughness_texture = ids[2];
            value.emissive_texture = ids[3];
            prepared.material = Some(value);
        }
        _ => return Err(aurora_i18n::tr!("import-local-images-only").into()),
    }
    let mut seen = HashSet::new();
    for (id, pixels) in textures {
        if !seen.insert(id) {
            continue;
        }
        let pixels = import_texture::prepare(&pixels, options).map_err(|e| e.to_string())?;
        let rgba = crate::scene::jobs::to_rgba(pixels.components, &pixels.data, pixels.width as usize * pixels.height as usize);
        let alpha = crate::scene::jobs::classify_alpha(&rgba);
        let has_alpha = pixels.components == 4;
        let mips = aurora_render::build_mips(pixels.width, pixels.height, rgba, 12);
        let refs = mips.iter().map(|(w, h, data)| (*w, *h, data.as_slice())).collect::<Vec<_>>();
        let staged = StagedTexture::new(pool, &refs).ok_or_else(|| aurora_i18n::tr!("import-local-staging-busy").to_owned())?;
        prepared.textures.push(Texture {
            id,
            staged,
            alpha,
            has_alpha,
        });
    }
    Ok(prepared)
}
impl Imports {
    pub fn publish_local(&mut self, id: Uuid, jobs: &crate::scene::jobs::Jobs, ctx: &egui::Context, pool: Arc<StagingPool>) {
        let Some(entry) = self.entries.iter_mut().find(|e| e.id == id && e.local && !e.local_pending) else {
            return;
        };
        let Some(source) = entry.source.clone() else {
            return;
        };
        entry.local_pending = true;
        let generation = entry.preview_generation;
        let options = entry.options;
        let tx = self.tx.clone();
        let wake = ctx.clone();
        jobs.execute(move || {
            let result = prepare(id, generation, &source, options, &pool);
            let _ = tx.send(Worker::Local(id, generation, result));
            wake.request_repaint();
        });
    }
    pub fn watch_local(&mut self, jobs: &crate::scene::jobs::Jobs, ctx: &egui::Context) {
        let now = Instant::now();
        for entry in &mut self.entries {
            if !entry.local_live {
                continue;
            }
            let Some(watch) = &mut entry.watch else {
                continue;
            };
            if watch.pending || watch.next > now {
                continue;
            }
            watch.pending = true;
            watch.next = now + Duration::from_secs(3);
            let paths = watch.paths.clone();
            let id = entry.id;
            let tx = self.tx.clone();
            let wake = ctx.clone();
            jobs.execute(move || {
                let _ = tx.send(Worker::Watch(id, stamp(&paths)));
                wake.request_repaint();
            });
        }
    }
}
