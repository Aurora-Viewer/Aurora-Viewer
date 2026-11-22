//! The LiveMesh cards' live thumbnails: a small render of each object as
//! Blender last sent it, redrawn when that object changes (step 2 of #4545).
//! Aurora's own: the Legacy Viewer has no live link with Blender.
//!
//! - One `preview::Model` per source, drawn by `aurora_render::preview`
//!   inside egui, without a readback. A card redraws only when its model's
//!   `Arc` changes, so an object Blender leaves as it was keeps its picture.
//! - Built on the job pool, never on the frame, from the snapshot already
//!   decoded: unskinned (the rest pose Blender binds in, the same for every
//!   frame and every tab), base colour and base colour texture only, images
//!   reduced to `MAX_IMAGE`, and at most `MAX_TRIANGLES` triangles (a sloppy
//!   meshopt reduction past it). With 32 sources at the geometry budget, the
//!   thumbnails stay well under the preview itself.
//! - A source is built once its digest has stayed the same for `SETTLE`:
//!   while the user drags a vertex in Blender, revisions follow one another
//!   and the card keeps its last picture until the hand stops.

use super::protocol::{PreparedImage, PreparedSource};
use aurora_render::preview::{Image, Model, Part, Vertex};
use glam::Vec3;
use std::collections::HashMap;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// A thumbnail never draws more triangles than this.
pub const MAX_TRIANGLES: usize = 20_000;
/// A texture is reduced to this side at most.
pub const MAX_IMAGE: u32 = 128;
/// How long a source must stay unchanged before its thumbnail is built.
pub const SETTLE: Duration = Duration::from_millis(400);

type Built = (Uuid, u64, Arc<Model>);

/// What one card shows: a root object and its linked prims, each at its
/// origin from the root's (`Link::offset`), as one picture.
#[derive(Clone)]
pub struct Group {
    pub id: Uuid,
    /// The digests of its sources, combined.
    pub digest: u64,
    pub sources: Vec<(PreparedSource, Vec3)>,
    /// When one of its sources last changed.
    pub changed: Option<Instant>,
}

impl Group {
    /// The cards of `sources`: each root with its prims; `keep` says which
    /// sources are shown (not removed).
    pub fn of(sources: &[PreparedSource], changed: &HashMap<Uuid, (u64, Instant)>, keep: impl Fn(Uuid) -> bool) -> Vec<Group> {
        use std::hash::{BuildHasher, Hash, Hasher};
        sources
            .iter()
            .filter(|root| root.link.is_none() && keep(root.id))
            .map(|root| {
                let members: Vec<_> = std::iter::once((root.clone(), Vec3::ZERO))
                    .chain(
                        sources
                            .iter()
                            .filter(|s| s.link.is_some_and(|link| link.parent == root.id) && keep(s.id))
                            .map(|s| (s.clone(), s.link.map_or(Vec3::ZERO, |link| Vec3::from_array(link.offset)))),
                    )
                    .collect();
                let mut hasher = foldhash::fast::FixedState::with_seed(0).build_hasher();
                for (source, _) in &members {
                    source.digest.hash(&mut hasher);
                }
                Group {
                    id: root.id,
                    digest: hasher.finish(),
                    changed: members.iter().filter_map(|(s, _)| changed.get(&s.id).map(|c| c.1)).max(),
                    sources: members,
                }
            })
            .collect()
    }
}

/// The thumbnails of the current snapshot, and those being built.
pub struct Thumbnails {
    /// By source: the digest it was built from, and its model.
    models: HashMap<Uuid, (u64, Arc<Model>)>,
    /// By source: the digest a job is building.
    pending: HashMap<Uuid, u64>,
    /// By card: the digest of its sources now (the last `update`).
    current: HashMap<Uuid, u64>,
    tx: mpsc::Sender<Built>,
    rx: mpsc::Receiver<Built>,
}

impl Default for Thumbnails {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            models: HashMap::new(),
            pending: HashMap::new(),
            current: HashMap::new(),
            tx,
            rx,
        }
    }
}

impl Thumbnails {
    /// The thumbnail of `source`, as last built (it may be a revision behind
    /// while the source changes).
    pub fn get(&self, source: Uuid) -> Option<Arc<Model>> {
        self.models.get(&source).map(|(_, model)| model.clone())
    }

    /// Whether `source` has a newer state than its thumbnail shows.
    pub fn stale(&self, source: Uuid, digest: u64) -> bool {
        self.models.get(&source).is_none_or(|(built, _)| *built != digest)
    }

    /// Whether a card's update is on its way: Blender sent a change its
    /// picture does not show yet (settling or being built). The card's
    /// sweep runs while it is.
    pub fn updating(&self, card: Uuid) -> bool {
        self.current.get(&card).is_some_and(|&digest| self.stale(card, digest))
    }

    pub fn clear(&mut self) {
        self.current.clear();
        self.models.clear();
        self.pending.clear();
        while self.rx.try_recv().is_ok() {}
    }

    /// Takes what the jobs built, forgets the sources that left, and starts
    /// a job for each source that changed and has settled. `run` starts a
    /// job (the job pool in the viewer, inline in the tests); the returned
    /// duration is when to look again, while a source is still settling.
    pub fn update(
        &mut self,
        groups: &[Group],
        images: &[PreparedImage],
        now: Instant,
        mut run: impl FnMut(Box<dyn FnOnce() + Send>),
    ) -> Option<Duration> {
        while let Ok((id, digest, model)) = self.rx.try_recv() {
            if self.pending.get(&id) == Some(&digest) {
                self.pending.remove(&id);
            }
            // a late result never replaces a newer picture
            if groups.iter().any(|g| g.id == id && g.digest == digest) {
                self.models.insert(id, (digest, model));
            }
        }
        self.current = groups.iter().map(|g| (g.id, g.digest)).collect();
        self.models.retain(|id, _| groups.iter().any(|g| g.id == *id));
        self.pending.retain(|id, _| groups.iter().any(|g| g.id == *id));
        let mut wait: Option<Duration> = None;
        for group in groups {
            if !self.stale(group.id, group.digest) || self.pending.get(&group.id) == Some(&group.digest) {
                continue;
            }
            let since = group.changed.map_or(Duration::MAX, |at| now.saturating_duration_since(at));
            // the first picture of a card comes at once; a change waits for
            // the hand to stop
            if self.models.contains_key(&group.id) && since < SETTLE {
                let left = SETTLE - since;
                wait = Some(wait.map_or(left, |w| w.min(left)));
                continue;
            }
            self.pending.insert(group.id, group.digest);
            let group = group.clone();
            let images = images.to_vec();
            let tx = self.tx.clone();
            run(Box::new(move || {
                let model = build(&group.sources, &images);
                let _ = tx.send((group.id, group.digest, Arc::new(model)));
            }));
        }
        wait
    }
}

/// The thumbnail model of a card's sources: their geometry in the rest
/// pose, placed by their instances and their offsets from the root, with
/// their base colours and base colour textures.
pub fn build(sources: &[(PreparedSource, Vec3)], images: &[PreparedImage]) -> Model {
    let total: usize = sources
        .iter()
        .flat_map(|(source, _)| {
            let scene = &source.scene;
            scene.instances.iter().filter_map(|instance| scene.models.get(instance.model))
        })
        .map(|model| model.triangles())
        .sum();
    // the share of each face's triangles kept, so the whole fits the cap
    let keep = if total > MAX_TRIANGLES {
        MAX_TRIANGLES as f32 / total as f32
    } else {
        1.0
    };
    let mut textures: HashMap<usize, Option<Arc<Image>>> = HashMap::new();
    let mut parts = Vec::new();
    for (source, offset) in sources {
        let scene = &source.scene;
        for instance in &scene.instances {
            let Some(model) = scene.models.get(instance.model) else {
                continue;
            };
            let transform = glam::Mat4::from_translation(*offset) * instance.transform;
            let normals = transform.inverse().transpose();
            for (slot, face) in model.faces.iter().enumerate() {
                let indices = reduced(face, keep);
                if indices.is_empty() {
                    continue;
                }
                let value = scene.materials.get(*instance.materials.get(slot).unwrap_or(&face.material));
                let mut material = value
                    .map(|value| crate::imports::preview::material(&value.value))
                    .unwrap_or_default();
                // as the importer's preview: no blending, masks kept
                if material.alpha_mode == 1 {
                    material.alpha_mode = 0;
                }
                let base = value.and_then(|value| value.images[0]).and_then(|index| {
                    textures
                        .entry(index)
                        .or_insert_with(|| images.get(index).map(|image| Arc::new(small_image(&image.pixels))))
                        .clone()
                });
                let vertices: Vec<Vertex> = face
                    .vertices
                    .iter()
                    .map(|v| {
                        Vertex::new(
                            transform.transform_point3(Vec3::from_array(v.position)).to_array(),
                            normals.transform_vector3(Vec3::from_array(v.normal)).normalize_or_zero().to_array(),
                            [v.uv[0], 1.0 - v.uv[1]],
                        )
                    })
                    .collect();
                parts.push(Part {
                    diagnostic: false,
                    vertices: vertices.into(),
                    indices: indices.into(),
                    textures: [base, None, None, None],
                    material,
                    skin: None,
                });
            }
        }
    }
    let (center, radius) = crate::imports::preview::bounds(&parts);
    Model { parts, center, radius }
}

/// A face's indices, reduced to `keep` of its triangles past the cap.
fn reduced(face: &aurora_assets::import_mesh::Face, keep: f32) -> Vec<u32> {
    if keep >= 1.0 || face.indices.len() < 3 * 16 {
        return face.indices.clone();
    }
    let target = ((face.indices.len() as f32 * keep) as usize / 3 * 3).max(3);
    let mut out = meshopt::simplify_sloppy_decoder(&face.indices, &face.vertices, target, 0.05, None);
    if out.len() < 3 {
        // nothing usable: every third triangle, which keeps the outline
        out = face.indices.as_chunks::<3>().0.iter().step_by(3).flatten().copied().collect();
    }
    // the reduction may overshoot its target a little: the cap is firm
    out.truncate(target);
    out
}

/// `pixels` reduced to `MAX_IMAGE` at most (an average of each block), RGBA.
fn small_image(pixels: &aurora_assets::DecodedImage) -> Image {
    let (width, height) = (pixels.width.max(1), pixels.height.max(1));
    let rgba = crate::scene::jobs::to_rgba(pixels.components, &pixels.data, width as usize * height as usize);
    let step = width.max(height).div_ceil(MAX_IMAGE).max(1);
    if step == 1 {
        return Image {
            size: [width, height],
            rgba: rgba.into(),
        };
    }
    let (w, h) = (width.div_ceil(step), height.div_ceil(step));
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let mut sum = [0u32; 4];
            let mut count = 0;
            for sy in y * step..((y + 1) * step).min(height) {
                for sx in x * step..((x + 1) * step).min(width) {
                    let i = ((sy * width + sx) * 4) as usize;
                    for (c, total) in sum.iter_mut().enumerate() {
                        *total += u32::from(rgba[i + c]);
                    }
                    count += 1;
                }
            }
            out.extend(sum.map(|total| (total / count.max(1)) as u8));
        }
    }
    Image {
        size: [w, h],
        rgba: out.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sources(value: &serde_json::Value) -> super::super::protocol::Prepared {
        super::super::tests::prepare(value).expect("snapshot")
    }

    fn inline(job: Box<dyn FnOnce() + Send>) {
        job();
    }

    fn groups(prepared: &super::super::protocol::Prepared, at: Instant) -> Vec<Group> {
        let changed = prepared.sources.iter().map(|s| (s.id, (s.digest, at))).collect();
        Group::of(&prepared.sources, &changed, |_| true)
    }

    #[test]
    fn a_thumbnail_is_built_once_per_state_and_waits_for_changes_to_settle() {
        let mut value = super::super::tests::static_triangle();
        let first = sources(&value);
        let id = first.sources[0].id;
        let start = Instant::now();
        let mut thumbnails = Thumbnails::default();
        // the first picture comes at once
        assert_eq!(thumbnails.update(&groups(&first, start), &first.images, start, inline), None);
        thumbnails.update(&groups(&first, start), &first.images, start, inline);
        let model = thumbnails.get(id).expect("built");
        assert!(!model.parts.is_empty() && model.radius > 0.0);
        assert!(!thumbnails.updating(id), "a card that shows its state has no sweep");
        // the same state: the same model, no new job
        let mut jobs = 0;
        thumbnails.update(&groups(&first, start), &first.images, start, |_| jobs += 1);
        assert_eq!(jobs, 0);
        assert!(Arc::ptr_eq(&model, &thumbnails.get(id).expect("kept")));
        // a vertex moves: the card keeps its picture until the change settles
        value["meshes"][0]["faces"][0]["vertices"][0]["position"][0] = 0.25.into();
        let second = sources(&value);
        let wait = thumbnails.update(&groups(&second, start), &second.images, start, |_| jobs += 1);
        assert_eq!(jobs, 0, "nothing is built during a drag");
        assert_eq!(wait, Some(SETTLE));
        assert!(Arc::ptr_eq(&model, &thumbnails.get(id).expect("kept while settling")));
        assert!(thumbnails.updating(id), "the sweep runs while the update is on its way");
        let later = start + SETTLE;
        thumbnails.update(&groups(&second, start), &second.images, later, inline);
        thumbnails.update(&groups(&second, start), &second.images, later, inline);
        assert!(!Arc::ptr_eq(&model, &thumbnails.get(id).expect("rebuilt")));
        assert!(!thumbnails.updating(id), "the sweep stops with the new picture");
        // a card that leaves is forgotten
        thumbnails.update(&[], &[], later, inline);
        assert!(thumbnails.get(id).is_none());
        assert!(!thumbnails.updating(id));
    }

    #[test]
    fn a_linkset_is_one_picture_with_its_prims_at_their_offsets() {
        let lib = crate::scene::avatar::AvatarLibrary::load();
        let prepared = crate::blender_live::demo_static_fixture(&lib.rig, true).expect("linked demo");
        let cards = groups(&prepared, Instant::now());
        let linkset = cards.iter().find(|g| g.sources.len() > 1).expect("a root with its prim");
        assert!(cards.iter().all(|g| g.sources[0].0.link.is_none()), "one card per root");
        let child = &linkset.sources[1];
        assert_eq!(child.1, Vec3::from_array(child.0.link.expect("linked").offset));
        let alone = build(&linkset.sources[..1], &prepared.images);
        let whole = build(&linkset.sources, &prepared.images);
        assert!(whole.parts.len() > alone.parts.len(), "the prim is drawn with its root");
        assert!(whole.radius > alone.radius, "and beside it");
        // a prim's change changes its card's digest
        let mut other = prepared.sources.clone();
        if let Some(source) = other.iter_mut().find(|s| s.id == child.0.id) {
            source.digest ^= 1;
        }
        let changed: HashMap<_, _> = HashMap::new();
        let moved = Group::of(&other, &changed, |_| true);
        let moved = moved.iter().find(|g| g.id == linkset.id).expect("card");
        assert_ne!(moved.digest, linkset.digest);
    }

    #[test]
    fn a_heavy_source_is_reduced_to_the_cap() {
        let mut value = super::super::tests::static_triangle();
        // a 160 × 160 grid: 51,200 triangles in one face
        let n = 160;
        let mut vertices = Vec::new();
        for y in 0..=n {
            for x in 0..=n {
                vertices.push(serde_json::json!({
                    "position": [x as f32 / n as f32, y as f32 / n as f32, ((x * y) % 7) as f32 * 0.001],
                    "normal": [0.0, 0.0, 1.0], "uv": [0.0, 0.0], "joints": [0, 0, 0, 0], "weights": [0.0, 0.0, 0.0, 0.0]
                }));
            }
        }
        let mut indices = Vec::new();
        for y in 0..n {
            for x in 0..n {
                let a = y * (n + 1) + x;
                indices.extend([a, a + 1, a + n + 1, a + 1, a + n + 2, a + n + 1]);
            }
        }
        value["meshes"][0]["faces"][0]["vertices"] = vertices.into();
        value["meshes"][0]["faces"][0]["indices"] = indices.into();
        let prepared = sources(&value);
        let total: usize = prepared.sources[0].scene.models.iter().map(|m| m.triangles()).sum();
        assert!(total > MAX_TRIANGLES, "{total}");
        let model = build(&[(prepared.sources[0].clone(), Vec3::ZERO)], &prepared.images);
        let drawn: usize = model.parts.iter().map(|p| p.indices.len() / 3).sum();
        assert!(drawn <= MAX_TRIANGLES && drawn > MAX_TRIANGLES / 4, "{drawn} triangles drawn");
    }

    #[test]
    fn a_large_texture_is_averaged_down() {
        let pixels = aurora_assets::DecodedImage {
            width: 512,
            height: 256,
            components: 4,
            data: [255u8, 0, 0, 255].repeat(512 * 256),
            discard: 0,
        };
        let image = small_image(&pixels);
        assert_eq!(image.size, [128, 64]);
        assert_eq!(&image.rgba[..4], &[255, 0, 0, 255]);
    }
}
