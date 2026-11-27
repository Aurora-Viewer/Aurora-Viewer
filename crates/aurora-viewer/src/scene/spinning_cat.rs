//! The spinning cat of the easter egg (`crate::oiia`): a mesh of the
//! viewer's own, drawn in the world for this viewer only. Nothing of it
//! reaches the grid, and it is no object of the world: it cannot be
//! selected, touched or seen by anyone else.
//!
//! The model is "Oiiaioooooiai Cat" by Zhuier, CC BY 4.0
//! (assets/models/README.md). Its one morph target, the cat flattened into a
//! loaf, is blended here on the vertices and written back to the mesh when
//! its weight moves; the cats circling the first share that mesh. The
//! circle on the ground is drawn here, from rings, paw prints and a swirl.

use crate::oiia::show::{self, Frame};
use aurora_render::types::flags;
use aurora_render::{DrawCmd, DrawLists, DrawRecord, MeshAlloc, MipLevel, Renderer, ShadowCaster, Vertex, build_mips};
use crossbeam_channel::{Receiver, TryRecvError};
use glam::{Mat4, Quat, Vec3};

const MODEL: &[u8] = include_bytes!("../../../../assets/models/spinning-cat.glb");
/// Metres per unit of the model (centimetres).
const UNIT: f32 = 0.01;
/// Radius of the circle on the ground (m), and its height over it.
const CIRCLE_RADIUS: f32 = 1.6;
const CIRCLE_LIFT: f32 = 0.04;
/// How much of its own colours the cat gives off, so that it is seen in a
/// dark place as in daylight; the light of the place adds to it.
const CAT_LIGHT: f32 = 0.5;
const CIRCLE_COLOR: [f32; 3] = [1.0, 0.45, 0.9];
const CIRCLE_TEXELS: u32 = 512;
/// Half the cat's height and its reach from its feet (m), for its bounds.
const CAT_HALF_HEIGHT: f32 = 0.2;
const CAT_REACH: f32 = 0.6;
/// The morph is written again when its weight moved by this much.
const MORPH_STEP: f32 = 1.0 / 255.0;

/// Where the show is and what it looks like this frame.
pub struct Stage {
    /// The circle's centre, on the ground.
    pub centre: Vec3,
    /// The way the cat faces at rest (radians around the vertical).
    pub facing: f32,
    pub frame: Frame,
    /// Seconds into the music, for the cats that circle.
    pub time: f32,
}

/// The model as the viewer draws it: metres, x ahead, z up.
struct Model {
    base: Vec<([f32; 3], [f32; 3])>,
    /// What the morph target adds to each vertex's position and normal.
    morph: Vec<([f32; 3], [f32; 3])>,
    uv: Vec<[f32; 2]>,
    indices: Vec<u16>,
}

impl Model {
    /// The vertices with the morph target at `weight`.
    fn vertices(&self, weight: f32) -> Vec<Vertex> {
        self.base
            .iter()
            .zip(&self.morph)
            .zip(&self.uv)
            .map(|(((p, n), (dp, dn)), uv)| {
                let position = Vec3::from_array(*p) + Vec3::from_array(*dp) * weight;
                let normal = (Vec3::from_array(*n) + Vec3::from_array(*dn) * weight).normalize_or(Vec3::Z);
                Vertex::new(position.to_array(), normal.to_array(), *uv)
            })
            .collect()
    }
}

type Mips = Vec<(u32, u32, Vec<u8>)>;

struct Loaded {
    model: Model,
    fur: Mips,
    circle: Mips,
}

/// The model's axes (x right, y up, z ahead) to the viewer's (x ahead,
/// y left, z up): a turn of the axes, which keeps the faces' winding.
fn to_world(v: [f32; 3]) -> [f32; 3] {
    [v[2], v[0], v[1]]
}

fn load() -> Result<Loaded, String> {
    use aurora_assets::AssetError;
    use aurora_assets::import_gltf::{Document, Image};
    use aurora_assets::import_mesh::gltf::attribute;
    let bundled = |_: &str| Err(AssetError::invalid("the model is self-contained"));
    let doc = Document::parse(MODEL, bundled).map_err(|e| e.to_string())?;
    let primitive = &doc.json["meshes"][0]["primitives"][0];
    let accessor = |value: &serde_json::Value, what: &str| value.as_u64().map(|i| i as usize).ok_or_else(|| format!("no {what}"));
    let vec3 = |value: &serde_json::Value, what: &str| attribute::<3>(&doc, accessor(value, what)?).map_err(|e| e.to_string());
    let positions = vec3(&primitive["attributes"]["POSITION"], "positions")?;
    let normals = vec3(&primitive["attributes"]["NORMAL"], "normals")?;
    let target = &primitive["targets"][0];
    let moved = vec3(&target["POSITION"], "morph positions")?;
    let turned = vec3(&target["NORMAL"], "morph normals")?;
    let uv = attribute::<2>(&doc, accessor(&primitive["attributes"]["TEXCOORD_0"], "texture coordinates")?).map_err(|e| e.to_string())?;
    let indices = attribute::<1>(&doc, accessor(&primitive["indices"], "indices")?).map_err(|e| e.to_string())?;
    let count = positions.len();
    if count == 0 || count > usize::from(u16::MAX) || [normals.len(), moved.len(), turned.len(), uv.len()] != [count; 4] {
        return Err("the model's vertex lists do not match".into());
    }
    if !indices.len().is_multiple_of(3) || indices.iter().any(|i| i[0] as usize >= count) {
        return Err("the model's indices are out of range".into());
    }
    let scaled = |v: [f32; 3]| to_world(v).map(|c| c * UNIT);
    let model = Model {
        base: positions.into_iter().zip(normals).map(|(p, n)| (scaled(p), to_world(n))).collect(),
        morph: moved.into_iter().zip(turned).map(|(p, n)| (scaled(p), to_world(n))).collect(),
        uv,
        indices: indices.into_iter().map(|i| i[0] as u16).collect(),
    };
    let Image::Bytes(bytes) = doc.image(0, bundled).map_err(|e| e.to_string())? else {
        return Err("the model has no texture of its own".into());
    };
    let fur = image::load_from_memory(&bytes).map_err(|e| e.to_string())?.to_rgba8();
    let (width, height) = fur.dimensions();
    Ok(Loaded {
        model,
        fur: build_mips(width, height, fur.into_raw(), 12),
        circle: build_mips(CIRCLE_TEXELS, CIRCLE_TEXELS, circle_texels(CIRCLE_TEXELS), 10),
    })
}

/// How much of the circle's drawing covers a point of the unit disc: two
/// rings with ticks between them, ten paw prints, and a swirl of three arms
/// in the middle.
fn circle_ink(x: f32, y: f32) -> bool {
    let radius = x.hypot(y);
    if radius > 1.0 {
        return false;
    }
    let angle = y.atan2(x).rem_euclid(std::f32::consts::TAU);
    let turn = angle / std::f32::consts::TAU;
    if (radius - 0.97).abs() < 0.012 || (radius - 0.64).abs() < 0.008 || radius < 0.045 {
        return true;
    }
    if (0.64..0.69).contains(&radius) && (turn * 36.0).fract() < 0.14 {
        return true;
    }
    if (0.10..0.56).contains(&radius) && ((turn * 3.0 + radius * 1.6).fract() - 0.5).abs() < 0.07 {
        return true;
    }
    // A paw print every tenth of a turn: a pad, and four toes outside it.
    let spoke = ((turn * 10.0).round() / 10.0) * std::f32::consts::TAU;
    let out = radius * (angle - spoke).cos() - 0.82;
    let across = radius * (angle - spoke).sin();
    let pad = (out + 0.018) / 0.040;
    let pad = pad * pad + (across / 0.052) * (across / 0.052) < 1.0;
    let toe = |at: f32, far: f32| (out - far).hypot(across - at) < 0.019;
    pad || toe(-0.058, 0.040) || toe(-0.021, 0.062) || toe(0.021, 0.062) || toe(0.058, 0.040)
}

/// The circle as white texels whose alpha is the drawing, each texel the
/// share of nine points inside it that are inked.
fn circle_texels(size: u32) -> Vec<u8> {
    let mut texels = vec![255u8; (size * size * 4) as usize];
    for (i, texel) in texels.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let (px, py) = ((i as u32 % size) as f32, (i as u32 / size) as f32);
        let mut inked = 0u32;
        for sub in 0..9u32 {
            let x = (px + ((sub % 3) as f32 + 0.5) / 3.0) / size as f32 * 2.0 - 1.0;
            let y = (py + ((sub / 3) as f32 + 0.5) / 3.0) / size as f32 * 2.0 - 1.0;
            inked += u32::from(circle_ink(x, y));
        }
        texel[3] = (inked * 255 / 9) as u8;
    }
    texels
}

fn texture(renderer: &mut Renderer, mips: &Mips) -> Option<u32> {
    let levels: Vec<MipLevel> = mips
        .iter()
        .map(|(width, height, data)| MipLevel {
            width: *width,
            height: *height,
            data,
        })
        .collect();
    renderer.create_texture(&levels)
}

struct Gpu {
    model: Model,
    cat: MeshAlloc,
    circle: MeshAlloc,
    fur: u32,
    ink: u32,
    /// The weight of the morph the mesh holds.
    morph: f32,
}

enum State {
    Idle,
    Loading(Receiver<Result<Loaded, String>>),
    Ready(Box<Gpu>),
    /// The model could not be read or the GPU has no room: the show goes
    /// on without the cat, and is not tried again until the next one.
    Failed,
}

pub struct SpinningCat {
    state: State,
    /// One record per cat drawn, the first one's first, then the circle's.
    records: Vec<u32>,
    circle_record: Option<u32>,
    cats: Vec<(DrawCmd, Vec3, f32)>,
    glowing: bool,
    /// The circle, blended: sorted with the world's other blended faces.
    pub cmds: Vec<(Vec3, DrawCmd)>,
}

impl Default for SpinningCat {
    fn default() -> Self {
        Self {
            state: State::Idle,
            records: Vec::new(),
            circle_record: None,
            cats: Vec::new(),
            glowing: false,
            cmds: Vec::new(),
        }
    }
}

impl SpinningCat {
    /// Frees everything: the show is over.
    pub fn clear(&mut self, renderer: &mut Renderer) {
        for record in self.records.drain(..).chain(self.circle_record.take()) {
            renderer.records.free(record);
        }
        if let State::Ready(gpu) = std::mem::replace(&mut self.state, State::Idle) {
            renderer.free_mesh(gpu.cat);
            renderer.free_mesh(gpu.circle);
            renderer.free_texture(gpu.fur);
            renderer.free_texture(gpu.ink);
        }
        self.cats.clear();
        self.cmds.clear();
    }

    /// The renderer could not stage the texels of these slots: when one is
    /// the cat's, it is loaded again at the next update, and both of its
    /// slots leave `lost`.
    pub fn texture_lost(&mut self, lost: &mut Vec<u32>, renderer: &mut Renderer) {
        let State::Ready(gpu) = &self.state else {
            return;
        };
        let mine = [gpu.fur, gpu.ink];
        if !mine.iter().any(|s| lost.contains(s)) {
            return;
        }
        lost.retain(|s| !mine.contains(s));
        self.clear(renderer);
    }

    /// The draws of this frame; `None` ends the show and frees it.
    pub fn update(&mut self, renderer: &mut Renderer, stage: Option<&Stage>) {
        self.cats.clear();
        self.cmds.clear();
        let Some(stage) = stage else {
            if !matches!(self.state, State::Idle) {
                self.clear(renderer);
            }
            return;
        };
        if matches!(self.state, State::Idle) {
            // Reading the model and its texture takes a few tens of
            // milliseconds: not on the frame's thread.
            let (send, receive) = crossbeam_channel::bounded(1);
            rayon::spawn(move || {
                let _ = send.send(load());
            });
            self.state = State::Loading(receive);
        }
        if let State::Loading(receive) = &self.state {
            self.state = match receive.try_recv() {
                Ok(Ok(loaded)) => Self::upload(renderer, loaded),
                Ok(Err(error)) => {
                    log::warn!("spinning cat: {error}");
                    State::Failed
                }
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => State::Failed,
            };
        }
        let State::Ready(gpu) = &mut self.state else {
            return;
        };
        let frame = &stage.frame;

        // ---- the circle
        let record = DrawRecord {
            model: Mat4::from_scale_rotation_translation(
                Vec3::new(CIRCLE_RADIUS, CIRCLE_RADIUS, 1.0),
                Quat::from_rotation_z(frame.circle_turn),
                stage.centre + Vec3::Z * CIRCLE_LIFT,
            )
            .to_cols_array_2d(),
            base_color: [CIRCLE_COLOR[0], CIRCLE_COLOR[1], CIRCLE_COLOR[2], frame.circle],
            emissive: [0.0, 0.0, 0.0, 0.6 * frame.circle],
            tex: [
                gpu.ink,
                aurora_render::textures::FLAT_NORMAL,
                aurora_render::textures::WHITE,
                aurora_render::textures::WHITE,
            ],
            flags: [flags::FULLBRIGHT | flags::ALPHA_BLEND | flags::DOUBLE_SIDED, 0, 0, 0],
            identity: [aurora_render::oit_tie(*b"aurora-cat-ring!", 0), 0, 0, 0],
            ..Default::default()
        };
        let slot = match self.circle_record {
            Some(slot) => {
                renderer.records.set(slot, record);
                slot
            }
            None => *self.circle_record.insert(renderer.records.alloc(record)),
        };
        if frame.circle > 0.003 {
            self.cmds.push((
                stage.centre,
                DrawCmd {
                    index_count: gpu.circle.index_count,
                    first_index: gpu.circle.index_offset,
                    base_vertex: gpu.circle.vertex_offset as i32,
                    record: slot,
                    // Never culled, as the other blended effects of the scene.
                    bounds: [0.0; 4],
                },
            ));
        }

        // ---- the cats
        let Some(cat) = frame.cat else {
            return;
        };
        if (cat.squash - gpu.morph).abs() >= MORPH_STEP && renderer.update_mesh_vertices(gpu.cat, &gpu.model.vertices(cat.squash)) {
            gpu.morph = cat.squash;
        }
        self.glowing = cat.glow > 0.01;
        let mut place = |index: usize, feet: Vec3, yaw: f32, scale: f32| {
            let record = DrawRecord {
                model: Mat4::from_scale_rotation_translation(Vec3::splat(scale), Quat::from_rotation_z(yaw), feet).to_cols_array_2d(),
                // Its own texture as its light, brighter on the kicks.
                emissive: [CAT_LIGHT + cat.glow, CAT_LIGHT + 0.6 * cat.glow, CAT_LIGHT + cat.glow, cat.glow],
                // The model's material: not metallic, rough like fur.
                params: [0.0, 0.0, 0.85, 0.5],
                tex: [
                    gpu.fur,
                    aurora_render::textures::FLAT_NORMAL,
                    aurora_render::textures::WHITE,
                    gpu.fur,
                ],
                // The emissive map as the colour map: scale 1, no offset.
                spec_color: [1.0, 1.0, 0.0, 0.0],
                flags: [flags::PBR | flags::DOUBLE_SIDED, 0, 0, 0],
                ..Default::default()
            };
            let slot = match self.records.get(index) {
                Some(slot) => {
                    renderer.records.set(*slot, record);
                    *slot
                }
                None => {
                    let slot = renderer.records.alloc(record);
                    self.records.push(slot);
                    slot
                }
            };
            let centre = feet + Vec3::Z * (CAT_HALF_HEIGHT * scale);
            let cmd = DrawCmd {
                index_count: gpu.cat.index_count,
                first_index: gpu.cat.index_offset,
                base_vertex: gpu.cat.vertex_offset as i32,
                record: slot,
                bounds: [0.0; 4],
            }
            .with_bounds(centre, CAT_REACH * scale);
            self.cats.push((cmd, centre, CAT_REACH * scale));
        };
        let feet = stage.centre + Vec3::Z * cat.lift;
        place(0, feet, stage.facing + cat.yaw, cat.scale);
        for index in 0..frame.clones.ceil() as usize {
            let (offset, size, way) = show::clone_place(index, stage.time, frame.clones - index as f32);
            if size > 0.0 {
                let around = offset[1].atan2(offset[0]);
                place(index + 1, feet + Vec3::from_array(offset), around + way * cat.yaw, cat.scale * size);
            }
        }
    }

    fn upload(renderer: &mut Renderer, loaded: Loaded) -> State {
        let corners = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
            .map(|(x, y): (f32, f32)| Vertex::new([x, y, 0.0], [0.0, 0.0, 1.0], [x * 0.5 + 0.5, y * 0.5 + 0.5]));
        let cat = renderer.upload_mesh(&loaded.model.vertices(0.0), &loaded.model.indices);
        let circle = renderer.upload_mesh(&corners, &[0, 1, 2, 0, 2, 3]);
        let fur = texture(renderer, &loaded.fur);
        let ink = texture(renderer, &loaded.circle);
        match (cat, circle, fur, ink) {
            (Some(cat), Some(circle), Some(fur), Some(ink)) => State::Ready(Box::new(Gpu {
                model: loaded.model,
                cat,
                circle,
                fur,
                ink,
                morph: 0.0,
            })),
            (cat, circle, fur, ink) => {
                log::warn!("spinning cat: no room for its mesh or its textures");
                for mesh in [cat, circle].into_iter().flatten() {
                    renderer.free_mesh(mesh);
                }
                for slot in [fur, ink].into_iter().flatten() {
                    renderer.free_texture(slot);
                }
                State::Failed
            }
        }
    }

    /// Adds the cats to the frame's lists, once the world's are built.
    pub fn append(&self, lists: &mut DrawLists) {
        for (cmd, center, radius) in &self.cats {
            lists.opaque_two_sided.push(*cmd);
            lists.shadow_casters.push(ShadowCaster {
                cmd: *cmd,
                center: *center,
                radius: *radius,
                alpha: false,
            });
            if self.glowing {
                lists.glow.push(*cmd);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_model_loads_with_its_morph_and_its_texture() {
        let loaded = load().expect("the bundled model");
        assert_eq!(loaded.model.base.len(), 9_167);
        assert_eq!(loaded.model.indices.len(), 47_322);
        assert_eq!(loaded.fur[0].0, 1_024);
        assert!(loaded.fur.len() > 8 && loaded.circle[0].0 == CIRCLE_TEXELS);
        // In metres, z up: a cat about 0.4 m tall and 0.7 m long, standing
        // on the ground.
        let (mut low, mut high) = (Vec3::MAX, Vec3::MIN);
        for (p, _) in &loaded.model.base {
            low = low.min(Vec3::from_array(*p));
            high = high.max(Vec3::from_array(*p));
        }
        assert!((0.3..0.5).contains(&(high.z - low.z)) && low.z.abs() < 0.03, "{low} {high}");
        assert!((0.6..0.8).contains(&(high.x - low.x)) && (0.2..0.3).contains(&(high.y - low.y)));
    }

    #[test]
    fn the_morph_flattens_the_cat_and_keeps_its_normals_unit() {
        let model = load().expect("the bundled model").model;
        let (standing, loaf) = (model.vertices(0.0), model.vertices(1.0));
        let moved = standing
            .iter()
            .zip(&loaf)
            .map(|(a, b)| Vec3::from_array(a.pos).distance(Vec3::from_array(b.pos)))
            .fold(0.0, f32::max);
        assert!((0.05..0.5).contains(&moved), "The loaf is another shape: {moved} m");
        // Halfway there is halfway between the two.
        let half = model.vertices(0.5);
        let between = (Vec3::from_array(standing[0].pos) + Vec3::from_array(loaf[0].pos)) * 0.5;
        assert!(Vec3::from_array(half[0].pos).distance(between) < 1e-5);
        for v in &loaf {
            let n = Vec3::new(f32::from(v.normal[0]), f32::from(v.normal[1]), f32::from(v.normal[2])) / 127.0;
            assert!((n.length() - 1.0).abs() < 0.05, "{n}");
        }
    }

    #[test]
    fn the_circle_is_drawn_inside_its_disc() {
        let size = 64;
        let texels = circle_texels(size);
        let alpha = |x: u32, y: u32| texels[((y * size + x) * 4 + 3) as usize];
        assert_eq!(alpha(0, 0), 0, "Nothing in the corners");
        assert!(alpha(size / 2, size / 2) > 200, "The dot in the middle");
        let (texel, _) = texels.as_chunks::<4>();
        let inked = texel.iter().filter(|t| t[3] > 127).count() as f32 / (size * size) as f32;
        assert!((0.05..0.4).contains(&inked), "{inked}");
        assert!(texel.iter().all(|t| t[..3] == [255; 3]));
        // The outer ring, on the right of the centre.
        assert!(circle_ink(0.97, 0.0) && !circle_ink(1.01, 0.0) && !circle_ink(0.90, 0.3));
    }
}
