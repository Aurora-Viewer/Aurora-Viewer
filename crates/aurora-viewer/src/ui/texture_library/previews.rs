//! Share the importer's GPU PBR preview. Texture decoding and model assembly
//! happen on workers; visible cards only enqueue bounded, revisioned requests.

use crate::imports::preview::MaterialShape;
use crate::scene::{Scene, textures::PreviewRevision};
use crate::theme::Palette;
use aurora_render::preview::{Image, Model, Part, Vertex, View};
use std::hash::{Hash, Hasher};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, OnceLock},
};
use uuid::Uuid;

type Signature = (u64, MaterialShape, [Option<(Uuid, PreviewRevision)>; 4]);
pub(super) type Thumbnails = [Option<(Uuid, egui::TextureHandle)>; 4];

fn material_signature(material: &aurora_assets::PbrMaterial) -> u64 {
    let mut hash = std::hash::DefaultHasher::new();
    material.base_color_factor.map(f32::to_bits).hash(&mut hash);
    material.emissive_factor.map(f32::to_bits).hash(&mut hash);
    [material.metallic_factor, material.roughness_factor, material.alpha_cutoff]
        .map(f32::to_bits)
        .hash(&mut hash);
    (material.alpha_mode as u8, material.double_sided).hash(&mut hash);
    for transform in material.transforms {
        transform.offset.map(f32::to_bits).hash(&mut hash);
        transform.scale.map(f32::to_bits).hash(&mut hash);
        transform.rotation.to_bits().hash(&mut hash);
    }
    hash.finish()
}
type Geometry = (Arc<[Vertex]>, Arc<[u32]>);
struct Cached {
    signature: Option<Signature>,
    pending: Option<Signature>,
    model: Option<Arc<Model>>,
    thumbnails: Thumbnails,
    seen: u64,
}
struct Reply {
    asset: Uuid,
    signature: Signature,
    model: Arc<Model>,
    thumbnails: Thumbnails,
}
pub struct Previews {
    entries: HashMap<Uuid, Cached>,
    tx: crossbeam_channel::Sender<Reply>,
    rx: crossbeam_channel::Receiver<Reply>,
    frame: u64,
    requests: usize,
}
impl Default for Previews {
    fn default() -> Self {
        let (tx, rx) = crossbeam_channel::bounded(64);
        Self {
            entries: HashMap::new(),
            tx,
            rx,
            frame: 0,
            requests: 0,
        }
    }
}
fn geometry(shape: MaterialShape) -> Arc<[Geometry]> {
    static GEOMETRY: [OnceLock<Arc<[Geometry]>>; 3] = [const { OnceLock::new() }; 3];
    GEOMETRY[shape as usize]
        .get_or_init(|| {
            crate::imports::preview::material_geometry(shape)
                .unwrap_or_else(|error| {
                    log::warn!("material preview geometry: {error}");
                    let (v, i) = crate::imports::preview::sphere();
                    vec![(v.into(), i.into())]
                })
                .into()
        })
        .clone()
}
impl Previews {
    pub fn poll(&mut self) {
        self.frame = self.frame.wrapping_add(1);
        self.requests = 0;
        while let Ok(reply) = self.rx.try_recv() {
            if let Some(entry) = self.entries.get_mut(&reply.asset).filter(|e| e.pending == Some(reply.signature)) {
                entry.pending = None;
                entry.signature = Some(reply.signature);
                entry.model = Some(reply.model);
                entry.thumbnails = reply.thumbnails;
            }
        }
    }
    pub fn model(&mut self, asset: Uuid, scene: &mut Scene, wanted: &mut HashSet<Uuid>, ctx: &egui::Context) -> Option<Arc<Model>> {
        let material = scene.materials.get(&asset)?;
        self.material_model(asset, material, MaterialShape::Sphere, scene, wanted, ctx)
    }

    pub(super) fn thumbnails(&self, asset: Uuid) -> Thumbnails {
        self.entries.get(&asset).map(|entry| entry.thumbnails.clone()).unwrap_or_default()
    }

    pub fn material_model(
        &mut self,
        asset: Uuid,
        material: Arc<aurora_assets::PbrMaterial>,
        shape: MaterialShape,
        scene: &mut Scene,
        wanted: &mut HashSet<Uuid>,
        ctx: &egui::Context,
    ) -> Option<Arc<Model>> {
        let maps = [
            material.base_color_texture,
            material.normal_texture,
            material.metallic_roughness_texture,
            material.emissive_texture,
        ];
        wanted.extend(maps.iter().flatten().copied());
        let sources = maps.map(|id| id.map(|id| (id, scene.textures.preview_source(&id))));
        let signature = (
            material_signature(&material),
            shape,
            sources.each_ref().map(|s| s.as_ref().map(|(id, s)| (*id, s.revision()))),
        );
        if !self.entries.contains_key(&asset) {
            if self.entries.len() >= 64 {
                let remove = self
                    .entries
                    .iter()
                    .filter(|(_, e)| e.pending.is_none())
                    .min_by_key(|(_, e)| e.seen)
                    .map(|(id, _)| *id);
                self.entries.remove(&remove?);
            }
            self.entries.insert(
                asset,
                Cached {
                    signature: None,
                    pending: None,
                    model: None,
                    thumbnails: Default::default(),
                    seen: self.frame,
                },
            );
        }
        let entry = self.entries.get_mut(&asset)?;
        entry.seen = self.frame;
        if entry.signature != Some(signature) && entry.pending.is_none() && self.requests < 4 {
            entry.pending = Some(signature);
            self.requests += 1;
            let tx = self.tx.clone();
            let wake = ctx.clone();
            let unchanged_maps = entry.signature.is_some_and(|previous| previous.2 == signature.2);
            let previous_textures = unchanged_maps
                .then(|| entry.model.as_ref()?.parts.first().map(|part| part.textures.clone()))
                .flatten();
            let previous_thumbnails = unchanged_maps.then(|| entry.thumbnails.clone());
            scene.jobs.execute(move || {
                let textures = previous_textures.unwrap_or_else(|| {
                    sources.map(|s| {
                        s.and_then(|(id, source)| {
                            source.image(id).or_else(|| {
                                // Demo UI images need no scene texture slot. Decode their
                                // procedural maps here too, off the frame thread.
                                crate::demo::local_texture(&id).map(|(rgba, w, h)| {
                                    Arc::new(Image {
                                        size: [w, h],
                                        rgba: rgba.into(),
                                    })
                                })
                            })
                        })
                    })
                });
                // A map can already be GPU-resident before want_ui_image is
                // called. Build its thumbnail from the same cached source as
                // the PBR preview, without relying on another GPU upload.
                let thumbnails = previous_thumbnails.unwrap_or_else(|| {
                    std::array::from_fn(|slot| {
                        let image = textures[slot].as_ref()?;
                        let id = maps[slot]?;
                        let rgba = image::RgbaImage::from_raw(image.size[0], image.size[1], image.rgba.to_vec())?;
                        let pixels = image::DynamicImage::ImageRgba8(rgba).thumbnail(96, 96).into_rgba8();
                        Some((
                            id,
                            wake.load_texture(
                                format!("material map {id}"),
                                egui::ColorImage::from_rgba_unmultiplied(
                                    [pixels.width() as usize, pixels.height() as usize],
                                    pixels.as_raw(),
                                ),
                                Default::default(),
                            ),
                        ))
                    })
                });
                let geometry = geometry(shape);
                let mut appearance = crate::imports::preview::material(&material);
                for (rows, t) in appearance.uv.iter_mut().zip(material.transforms) {
                    let (s, c) = t.rotation.sin_cos();
                    *rows = [
                        [c * t.scale[0], -s * t.scale[1], 0.0, t.offset[0]],
                        [s * t.scale[0], c * t.scale[1], 0.0, t.offset[1]],
                    ];
                }
                let parts = geometry
                    .iter()
                    .map(|(vertices, indices)| Part {
                        diagnostic: false,
                        vertices: vertices.clone(),
                        indices: indices.clone(),
                        textures: textures.clone(),
                        material: appearance,
                        skin: None,
                    })
                    .collect::<Vec<_>>();
                let (center, radius) = crate::imports::preview::bounds(&parts);
                let model = Arc::new(Model { parts, center, radius });
                let _ = tx.try_send(Reply {
                    asset,
                    signature,
                    model,
                    thumbnails,
                });
                wake.request_repaint();
            });
        }
        entry.model.clone()
    }
}
pub fn view(p: &Palette) -> View {
    View {
        background: [
            p.field.r() as f32 / 255.0,
            p.field.g() as f32 / 255.0,
            p.field.b() as f32 / 255.0,
            1.0,
        ],
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pbr_bom_previews_use_the_coloured_reference_images_for_every_channel() {
        let _inline = crate::scene::jobs::execute_inline_on_this_thread();
        let avatars = Arc::new(crate::scene::avatar::AvatarLibrary::load());
        let mut scene = Scene::new(std::env::temp_dir(), avatars);
        let maps = [0, 1, 2, 8].map(|slot| crate::scene::avatar::BAKES[slot].2);
        let colours = [[240, 80, 20, 255], [30, 190, 170, 255], [150, 110, 235, 255], [220, 180, 40, 255]];
        for (id, rgba) in maps.into_iter().zip(colours) {
            scene.textures.set_local_pixels(
                id,
                Arc::new(Image {
                    size: [1, 1],
                    rgba: Vec::from(rgba).into(),
                }),
            );
        }
        let material = Arc::new(aurora_assets::PbrMaterial {
            base_color_texture: Some(maps[0]),
            normal_texture: Some(maps[1]),
            metallic_roughness_texture: Some(maps[2]),
            emissive_texture: Some(maps[3]),
            ..Default::default()
        });
        let ctx = egui::Context::default();
        let mut previews = Previews::default();
        let mut wanted = HashSet::new();
        let asset = Uuid::from_u128(1793);
        previews.material_model(asset, material, MaterialShape::Sphere, &mut scene, &mut wanted, &ctx);
        previews.poll();
        let model = previews.entries[&asset].model.as_ref().unwrap();
        for (slot, rgba) in colours.iter().enumerate() {
            assert_eq!(model.parts[0].textures[slot].as_ref().unwrap().rgba.as_ref(), rgba);
            assert_eq!(previews.thumbnails(asset)[slot].as_ref().unwrap().0, maps[slot]);
        }
        assert_eq!(wanted, HashSet::from(maps));
    }

    #[test]
    fn material_shape_changes_rebuild_geometry_and_keep_maps_and_uvs_on_every_face() {
        let _inline = crate::scene::jobs::execute_inline_on_this_thread();
        let avatars = Arc::new(crate::scene::avatar::AvatarLibrary::load());
        let mut scene = Scene::new(std::env::temp_dir(), avatars);
        let ctx = egui::Context::default();
        let mut previews = Previews::default();
        let item = Uuid::from_u128(790);
        let mut material = aurora_assets::PbrMaterial {
            base_color_texture: Some(aurora_prim::te::BLANK_TEXTURE),
            normal_texture: Some(aurora_prim::te::TRANSPARENT_TEXTURE),
            metallic_factor: 0.23,
            ..Default::default()
        };
        material.transforms[0].offset = [0.25, 0.5];
        let material = Arc::new(material);
        let mut wanted = HashSet::new();
        for shape in MaterialShape::ALL {
            previews.material_model(item, material.clone(), shape, &mut scene, &mut wanted, &ctx);
            previews.poll();
            let model = previews.entries[&item].model.as_ref().expect("preview job");
            if shape == MaterialShape::Cube {
                assert_eq!(model.parts.len(), 6);
            }
            if shape == MaterialShape::Sphere {
                assert_eq!(model.parts.len(), 1);
            }
            assert!(model.center.is_finite() && model.radius > 0.0);
            for part in &model.parts {
                assert_eq!(part.material.metallic, 0.23);
                assert_eq!(part.material.uv[0][0][3], 0.25);
                assert_eq!(part.material.uv[0][1][3], 0.5);
                assert_eq!(part.textures[0].as_ref().expect("base map").rgba.as_ref(), &[255; 4]);
                assert_eq!(part.textures[1].as_ref().expect("normal map").rgba.as_ref(), &[0; 4]);
            }
        }
        assert_eq!(wanted.len(), 2);
    }
}
