//! Share the importer's GPU PBR preview. Texture decoding and model assembly
//! happen on workers; visible cards only enqueue bounded, revisioned requests.

use crate::scene::{Scene, textures::PreviewRevision};
use crate::theme::Palette;
use aurora_render::preview::{Image, Model, Part, Vertex, View};
use std::hash::{Hash, Hasher};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, OnceLock},
};
use uuid::Uuid;

type Signature = (u64, [Option<(Uuid, PreviewRevision)>; 4]);

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
    seen: u64,
}
struct Reply {
    asset: Uuid,
    signature: Signature,
    model: Arc<Model>,
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
fn geometry() -> Geometry {
    static GEOMETRY: OnceLock<Geometry> = OnceLock::new();
    GEOMETRY
        .get_or_init(|| {
            let (v, i) = crate::imports::preview::sphere();
            (v.into(), i.into())
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
            }
        }
    }
    pub fn model(&mut self, asset: Uuid, scene: &mut Scene, wanted: &mut HashSet<Uuid>, ctx: &egui::Context) -> Option<Arc<Model>> {
        let material = scene.materials.get(&asset)?;
        self.material_model(asset, material, scene, wanted, ctx)
    }

    pub fn material_model(
        &mut self,
        asset: Uuid,
        material: Arc<aurora_assets::PbrMaterial>,
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
            scene.jobs.execute(move || {
                let textures = sources.map(|s| {
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
                });
                let (vertices, indices) = geometry();
                let mut appearance = crate::imports::preview::material(&material);
                for (rows, t) in appearance.uv.iter_mut().zip(material.transforms) {
                    let (s, c) = t.rotation.sin_cos();
                    *rows = [
                        [c * t.scale[0], -s * t.scale[1], 0.0, t.offset[0]],
                        [s * t.scale[0], c * t.scale[1], 0.0, t.offset[1]],
                    ];
                }
                let model = Arc::new(Model {
                    parts: vec![Part {
                        diagnostic: false,
                        vertices,
                        indices,
                        textures,
                        material: appearance,
                        skin: None,
                    }],
                    center: glam::Vec3::ZERO,
                    radius: 1.0,
                });
                let _ = tx.try_send(Reply { asset, signature, model });
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
