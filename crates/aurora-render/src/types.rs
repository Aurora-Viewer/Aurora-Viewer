//! Plain data shared between the world and the renderer.

use crate::HudView;
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3, Vec4};

/// 24-byte vertex used by every mesh (prims, meshes, avatars, terrain).
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable, Default)]
pub struct Vertex {
    pub pos: [f32; 3],
    /// snorm8 normal; `w` carries extra data: the tangent of an object's
    /// vertex ([`pack_tangent`]), the handedness of the terrain's frame.
    pub normal: [i8; 4],
    pub uv: [f32; 2],
}

impl Vertex {
    #[inline]
    pub fn new(pos: [f32; 3], n: [f32; 3], uv: [f32; 2]) -> Vertex {
        Vertex {
            pos,
            normal: [pack_snorm(n[0]), pack_snorm(n[1]), pack_snorm(n[2]), 0],
            uv,
        }
    }
}

#[inline]
pub fn pack_snorm(v: f32) -> i8 {
    if !v.is_finite() {
        return 0;
    }
    (v.clamp(-1.0, 1.0) * 127.0).round() as i8
}

/// Steps of a packed tangent's angle around its normal.
const TANGENT_STEPS: f32 = 127.0;

/// The normal a vertex shader reads from a snorm8 normal, unit length.
fn unpacked_normal(normal: [i8; 4]) -> Option<Vec3> {
    let n = Vec3::new(normal[0] as f32, normal[1] as f32, normal[2] as f32) / 127.0;
    n.max(Vec3::splat(-1.0)).try_normalize()
}

/// Two unit axes across `n` (Duff et al. 2017, "Building an Orthonormal
/// Basis, Revisited"); object.wgsl vertex_tangent builds the same pair.
fn tangent_basis(n: Vec3) -> (Vec3, Vec3) {
    let s = if n.z >= 0.0 { 1.0 } else { -1.0 };
    let a = -1.0 / (s + n.z);
    let b = n.x * n.y * a;
    (
        Vec3::new(1.0 + s * n.x * n.x * a, s * b, -s * n.x),
        Vec3::new(b, s + n.y * n.y * a, -n.y),
    )
}

/// A vertex tangent in one byte, the `w` of the packed `normal`: the
/// tangent's angle around the normal, in 127 steps (1.4° at most off), and
/// its handedness as the sign (`hand` < 0: the bitangent is -cross(N, T),
/// a mirrored texture). 0 is no tangent (normal maps on the per-pixel frame
/// of perturb_normal). The tangent is taken across the packed normal, the
/// one the vertex shader reads; the Legacy Viewer's are across theirs
/// (LLCalculateTangentArray, MikkTSpace), so the vertex keeps its 24 bytes
/// for what a 16-byte float tangent tells.
pub fn pack_tangent(normal: [i8; 4], t: [f32; 3], hand: f32) -> i8 {
    let Some(n) = unpacked_normal(normal) else {
        return 0;
    };
    let t = Vec3::from_array(t);
    if !t.is_finite() {
        return 0;
    }
    let (b1, b2) = tangent_basis(n);
    let (x, y) = (t.dot(b1), t.dot(b2));
    let angle = if x * x + y * y > 1e-12 { y.atan2(x) } else { 0.0 };
    let step = (angle.rem_euclid(std::f32::consts::TAU) / std::f32::consts::TAU * TANGENT_STEPS).round() as i32 % TANGENT_STEPS as i32;
    let code = (step + 1) as i8;
    if hand < 0.0 { -code } else { code }
}

/// The tangent and handedness of [`pack_tangent`], as object.wgsl
/// vertex_tangent reads them; `None` without one.
pub fn unpack_tangent(normal: [i8; 4]) -> Option<(Vec3, f32)> {
    let code = normal[3].max(-127);
    if code == 0 {
        return None;
    }
    let n = unpacked_normal(normal)?;
    let (b1, b2) = tangent_basis(n);
    let angle = (code.unsigned_abs() - 1) as f32 * (std::f32::consts::TAU / TANGENT_STEPS);
    Some((b1 * angle.cos() + b2 * angle.sin(), if code < 0 { -1.0 } else { 1.0 }))
}

/// Per-vertex skinning data (parallel stream): 4 joint indices and 4
/// normalized weights. Terrain uses these same eight bytes for its frame
/// ([`pack_terrain_frame`]); its records never enable skinning.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable, Default)]
pub struct SkinVertex {
    /// Mesh joint indices into the record's skin bindings (`flags[2]`).
    pub joints: [u8; 4],
    pub weights: [u8; 4],
}

/// Terrain has no skeleton: its parallel 8-byte stream holds octahedral
/// normal/tangent directions (two snorm16 values each). This keeps the
/// Legacy Viewer's terrain frame without reducing it to snorm8 or deriving
/// the tangent from the normal. No vertex stride or GPU binding is added.
pub fn pack_terrain_frame(normal: Vec3, tangent: Vec3) -> SkinVertex {
    let pack = |v: Vec3| {
        let v = v / v.abs().element_sum().max(1e-20);
        let mut p = v.truncate();
        if v.z < 0.0 {
            p = (glam::Vec2::ONE - glam::Vec2::new(p.y, p.x).abs())
                * glam::Vec2::new(if p.x < 0.0 { -1.0 } else { 1.0 }, if p.y < 0.0 { -1.0 } else { 1.0 });
        }
        let x = ((p.x.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes();
        let y = ((p.y.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes();
        [x[0], x[1], y[0], y[1]]
    };
    SkinVertex {
        joints: pack(normal),
        weights: pack(tangent),
    }
}

pub fn unpack_terrain_frame(frame: SkinVertex) -> (Vec3, Vec3) {
    let unpack = |bytes: [u8; 4]| {
        let x = f32::from(i16::from_le_bytes([bytes[0], bytes[1]])) / 32767.0;
        let y = f32::from(i16::from_le_bytes([bytes[2], bytes[3]])) / 32767.0;
        let mut n = Vec3::new(x, y, 1.0 - x.abs() - y.abs());
        if n.z < 0.0 {
            n.x = (1.0 - y.abs()) * if x < 0.0 { -1.0 } else { 1.0 };
            n.y = (1.0 - x.abs()) * if y < 0.0 { -1.0 } else { 1.0 };
        }
        n.normalize_or(Vec3::Z)
    };
    (unpack(frame.joints), unpack(frame.weights))
}

/// One mesh joint of a skinned mesh (LLMeshSkinInfo): its inverse bind
/// matrix and the joint of the owner's palette that drives it. The
/// skinning matrix is `palette[flags[1] + joint] * inverse_bind`, as
/// LLSkinningUtil::initSkinningMatrixPalette.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct SkinBind {
    pub inverse_bind: [[f32; 4]; 4],
    /// x: palette joint index.
    pub joint: [u32; 4],
}

pub mod flags {
    pub const FULLBRIGHT: u32 = 1;
    pub const ALPHA_MASK: u32 = 2;
    pub const ALPHA_BLEND: u32 = 4;
    pub const PBR: u32 = 8;
    pub const HIGHLIGHT: u32 = 16;
    pub const PLANAR: u32 = 32;
    /// Terrain records use four shared material record indices for PBR.
    pub const TERRAIN: u32 = 64;
    /// Vertex positions are skinned with palette `flags[1]`.
    pub const SKINNED: u32 = 128;
    /// Legacy material (LLMaterial): normal / specular maps with their own
    /// transforms (`mat_uv`, `spec_uv`, `legacy`).
    pub const LEGACY_MAT: u32 = 256;
    /// Legacy "emissive mask" alpha mode: texture alpha = emissive amount.
    pub const EMISSIVE_MASK: u32 = 512;
    /// Texture animation (llSetTextureAnim): `anim`, `anim_xf` and `flags[3]`
    /// (see `tex_anim::RecordAnim`), evaluated by the vertex shaders.
    pub const TEX_ANIM: u32 = 1024;
    /// glTF doubleSided: HUD fragments keep back faces only when requested.
    pub const DOUBLE_SIDED: u32 = 2048;
    /// Placed by the GPU. Not skinned: `model` is relative to the anchor
    /// matrix `palettes[flags[1]]` (the posed attachment point of the
    /// wearer), composed by the vertex shaders. Skinned: the skeleton is
    /// placed by its own anchor, `palettes[flags[1] + SKELETON_ANCHOR]`,
    /// and `model` (the identity) is not read.
    pub const ANCHORED: u32 = 4096;
    /// System avatar live morphs (lip sync, body physics, facial
    /// expressions, hand poses), evaluated before skinning.
    pub const LIP_SYNC: u32 = 8192;
    pub const PORTAL_CLIP: u32 = 16384;
    /// Procedural Aurora portal surface; uses the ordinary sorted blend/glow.
    pub const PORTAL_EFFECT: u32 = 32768;
}

pub const PORTAL_HALF_WIDTH: f32 = 1.0;
pub const PORTAL_HALF_HEIGHT: f32 = 1.5;
pub const PORTAL_SURFACE_SCALE: f32 = 1.16;

/// One vector keeps the cut local without growing every world-face record:
/// center.xyz, and the horizontal entry normal on a signed diamond perimeter.
/// Decoding needs only normalization, no per-fragment trigonometry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PortalClip {
    pub data: [f32; 4],
}

impl PortalClip {
    pub fn new(center: Vec3, normal: Vec3) -> Option<Self> {
        if !center.is_finite() || !normal.is_finite() || normal.z.abs() > 1e-5 {
            return None;
        }
        let normal = normal.try_normalize()?;
        let mut direction = normal.y / (normal.x.abs() + normal.y.abs());
        if normal.x < 0.0 {
            direction = if normal.y < 0.0 { -1.0 } else { 1.0 } * (2.0 - direction.abs());
        }
        Some(Self {
            data: center.extend(direction).to_array(),
        })
    }

    pub fn normal(self) -> Vec3 {
        let code = self.data[3];
        let y = if code.abs() > 1.0 {
            code.signum() * (2.0 - code.abs())
        } else {
            code
        };
        Vec3::new(1.0 - code.abs(), y, 0.0).normalize()
    }

    /// CPU picking uses the same finite aperture as every GPU surface pass.
    pub fn keeps_point(self, point: Vec3) -> bool {
        let local = point - Vec3::from_slice(&self.data[..3]);
        let normal = self.normal();
        if local.dot(normal) >= -0.0001 {
            return true;
        }
        let across = local.dot(Vec3::new(-normal.y, normal.x, 0.0)) / PORTAL_HALF_WIDTH;
        let height = local.z / PORTAL_HALF_HEIGHT;
        across * across + height * height > 1.0
    }
}

/// Layout of a skeleton's palette in the palettes buffer: its joints (the
/// 159 bones and collision volumes, the 47 body attachment points, which
/// meshes are skinned to and animations turn as in the Legacy Viewer, and
/// the identity slot), then its anchors. The first anchor places the
/// skeleton itself (skeleton space to world); the scene gives the others to
/// the attachment points.
pub const SKELETON_ANCHOR: u32 = 208;

/// First matrix of the weights of a skeleton's live morphs, after its 48
/// anchors: column `k` is `palettes[base + LIVE_MORPH_WEIGHTS + k / 16][k / 4 % 4][k % 4]`.
pub const LIVE_MORPH_WEIGHTS: u32 = 256;

/// Per-face GPU record (must match `DrawRecord` in common.wgsl).
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct DrawRecord {
    pub model: [[f32; 4]; 4],
    pub base_color: [f32; 4],
    pub emissive: [f32; 4],
    pub uv_st: [f32; 4],
    pub params: [f32; 4],
    pub tex: [u32; 4],
    pub flags: [u32; 4],
    /// Legacy and PBR materials: normal map scale s, t, offset s, t.
    pub mat_uv: [f32; 4],
    /// Legacy material: specular map scale s, t, offset s, t (PBR: the
    /// metallic-roughness map's).
    pub spec_uv: [f32; 4],
    /// Legacy material: normal rotation, specular rotation, glossiness
    /// (exponent / 255), environment intensity (/ 255). PBR: normal,
    /// metallic-roughness and emissive map rotations.
    pub legacy: [f32; 4],
    /// Legacy material: specular light color (rgb, linear). PBR: emissive
    /// map scale s, t, offset s, t.
    pub spec_color: [f32; 4],
    /// Texture animation (flag `TEX_ANIM`): time origin on the renderer's
    /// `AnimClock` (ms; the frozen counter's f32 bits at rate 0), start,
    /// length, rate (f32 bits). `flags[3]` holds mode | size_x << 8 | size_y << 16.
    pub anim: [u32; 4],
    /// Texture animation: the texture entry's transform parts the animation
    /// leaves alone (`tex_anim::RecordAnim::constants`).
    pub anim_xf: [f32; 4],
    /// The rez effect of an object appearing (the viewer's `scene/rez.rs`):
    /// x the `AnimClock` ms it started at, y its mode (`rez_mode`) in the
    /// low byte, the line's length in ms above it (12 bits) and a look of
    /// its own in the 4 bits above (0: the frame's), z and w the world
    /// height of the object's bottom and its height (f32 bits).
    pub rez: [u32; 4],
    /// Finite portal cut: center.xyz and encoded horizontal entry direction.
    /// Only active with PORTAL_CLIP; zero is also a valid opening at the origin.
    pub portal_clip: [f32; 4],
}

/// Modes of `DrawRecord::rez` (`rez_apply` in object.wgsl).
pub mod rez_mode {
    /// Drawn as usual.
    pub const NONE: u32 = 0;
    /// Still loading: a glowing grid in the accent color.
    pub const HOLOGRAM: u32 = 1;
    /// Loaded: a line rising from its bottom reveals it.
    pub const SWEEP: u32 = 2;
    /// Leaving: the line comes down and leaves the grid.
    pub const DEREZ: u32 = 3;
    /// Loaded with no line for it: from the grid to the textures.
    pub const FADE: u32 = 4;
    /// Arriving: the line passes over it and hides nothing (a face still
    /// loading stays the grid, the others show their surface).
    pub const GLINT: u32 = 5;
}

impl Default for DrawRecord {
    fn default() -> Self {
        DrawRecord {
            model: Mat4::IDENTITY.to_cols_array_2d(),
            base_color: [1.0; 4],
            emissive: [0.0; 4],
            uv_st: [1.0, 1.0, 0.0, 0.0],
            params: [0.0, 0.0, 0.8, 0.5],
            tex: [
                crate::textures::WHITE,
                crate::textures::FLAT_NORMAL,
                crate::textures::WHITE,
                crate::textures::WHITE,
            ],
            flags: [0; 4],
            mat_uv: [1.0, 1.0, 0.0, 0.0],
            spec_uv: [1.0, 1.0, 0.0, 0.0],
            legacy: [0.0; 4],
            spec_color: [1.0, 1.0, 1.0, 0.0],
            anim: [0; 4],
            anim_xf: [0.0; 4],
            rez: [0; 4],
            portal_clip: [0.0; 4],
        }
    }
}

impl DrawRecord {
    /// Upper 16 bits of flags.x: visual HUD zoom in units of 1/4096.
    /// Zero means 1. The shader removes it from planar texture generation,
    /// before materials and animations, without enlarging every GPU record.
    pub fn set_hud_zoom(&mut self, zoom: f32) {
        let packed = if zoom == 1.0 {
            0
        } else {
            (zoom.clamp(0.1, 8.0) * 4096.0).round() as u32
        };
        self.flags[0] = (self.flags[0] & 0xffff) | (packed << 16);
    }

    /// Index of the anchor matrix that places the record (flag `ANCHORED`):
    /// the one its model is relative to, or its skeleton's when skinned.
    pub fn anchor(&self) -> Option<u32> {
        if self.flags[0] & flags::ANCHORED == 0 {
            return None;
        }
        Some(if self.flags[0] & flags::SKINNED != 0 {
            self.flags[1] + SKELETON_ANCHOR
        } else {
            self.flags[1]
        })
    }

    /// Place the record from anchor matrix `anchor`, or in the world. A
    /// skinned record keeps its palette in `flags[1]`: its anchor is that
    /// palette's own (`SKELETON_ANCHOR`), whatever is asked.
    pub fn set_anchor(&mut self, anchor: Option<u32>) {
        let skinned = self.flags[0] & flags::SKINNED != 0;
        match anchor {
            Some(a) => {
                self.flags[0] |= flags::ANCHORED;
                if !skinned {
                    self.flags[1] = a;
                }
            }
            None => {
                self.flags[0] &= !flags::ANCHORED;
                if !skinned {
                    self.flags[1] = 0;
                }
            }
        }
    }
}

/// One indexed draw referring to a record slot.
#[derive(Debug, Clone, Copy)]
pub struct DrawCmd {
    pub index_count: u32,
    pub first_index: u32,
    pub base_vertex: i32,
    pub record: u32,
    /// World bounding sphere (center, radius) for the GPU occlusion test;
    /// radius 0 = never culled.
    pub bounds: [f32; 4],
}

impl DrawCmd {
    pub fn with_bounds(mut self, center: Vec3, radius: f32) -> Self {
        self.bounds = center.extend(radius).to_array();
        self
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub(crate) struct DrawIndexedIndirect {
    pub index_count: u32,
    pub instance_count: u32,
    pub first_index: u32,
    pub base_vertex: i32,
    pub first_instance: u32,
}

impl From<&DrawCmd> for DrawIndexedIndirect {
    fn from(d: &DrawCmd) -> Self {
        DrawIndexedIndirect {
            index_count: d.index_count,
            instance_count: 1,
            first_index: d.first_index,
            base_vertex: d.base_vertex,
            first_instance: d.record,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ShadowCaster {
    pub cmd: DrawCmd,
    pub center: Vec3,
    pub radius: f32,
    /// Shadow casters: the face is masked or blended, its shadow is alpha
    /// tested (`fs_shadow`); the others are drawn depth only.
    pub alpha: bool,
}

/// How the blended faces of rigged meshes (`DrawLists::blend_depth`) write
/// depth.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RiggedBlendDepth {
    /// From 1/255 of alpha as they are drawn, into the scene depth, as the
    /// Legacy Viewer (LLDrawPoolAlpha::forwardRender(true), #499).
    #[default]
    Write,
    /// Only where at least half covered, in a depth pass before their
    /// colour, which is then drawn without depth (#556). A layer of mesh
    /// hair drawn first is then hidden behind every strand of the layers
    /// over it that is half covered or more: a strand at 70 % shows what
    /// lies behind the hair through it, not the layer under it (#935).
    HalfCovered,
    /// From 1/255 as they are drawn, as the Legacy Viewer, but into a copy
    /// of the scene depth (#948): the layers of one hair compose in their
    /// drawing order, and the scene depth then takes them only where at
    /// least half covered, so that the glass, water and particles drawn
    /// after them are not hidden by a soft fringe (#556). But a soft strand
    /// edge drawn first then hides the strands drawn after it behind it: the
    /// triangles of one face, not drawn back to front (the Legacy Viewer
    /// reorders them for the vertex cache, LLVolumeFace::cacheOptimize),
    /// leave shards where the scene behind shows through the hair (#935).
    Layered,
    /// Their depth where nearly opaque (95 %) before any of their colour;
    /// then run by run (`DrawLists::blend_depth_runs`: the meshes worn on
    /// one attachment point of one skeleton) their colour, depth tested,
    /// and their depth where at least half covered, which the runs drawn
    /// after them and the rest of the scene see. A layer is hidden only
    /// behind what nearly covers it, the triangles of one face and the
    /// meshes of one point never hide each other by a faint edge (#935),
    /// a mesh of another point drawn after them (a body under the hair) is
    /// hidden behind their strands from 50 % on, as with `HalfCovered`;
    /// no halo (#556). But a strand between 50 and 95 % hides nothing drawn
    /// after it: an inner layer drawn after the outer one, darker, showed
    /// over it in patches the shape of its triangles (#935).
    Runs,
    /// In their drawing order (`DrawCmd`s in the Legacy Viewer's order, the
    /// triangles of a mesh in its order, `aurora_assets::legacy_triangle_order`),
    /// object by object (`DrawLists::blend_depth_runs`): first its texels at
    /// least half covered, each writing its depth as it is drawn, as the
    /// Legacy Viewer from 1/255; then its fainter texels, depth tested,
    /// writing nothing. A strand hides what is drawn after it behind it only
    /// where it is at least half covered: hair made inner layer first
    /// composes as its creator saw it, a layer drawn after another and
    /// behind it no longer shows over it (#935), a soft edge hides nothing
    /// (no shards, #935 after #948), and the scene depth holds the hair
    /// where half covered only (no halo, #556). Within one object the
    /// fringes come after all of its strands. But a fainter texel hides
    /// nothing drawn after it: under a sheer dress whose alpha wavers around
    /// one half, a layer drawn after it (a body add-on) showed through in
    /// speckles, hidden where the dress is half covered, over it elsewhere
    /// (#1300).
    Split,
    /// `Split`, one object per run, into a copy of the scene depth
    /// (`Targets::blend_layers`), each run's fringes then writing their depth
    /// there from 1/255 (#1300): a mesh drawn after another and behind it is
    /// hidden by any of its texels, as with the Legacy Viewer's 1/255, while
    /// the texels of one object still never hide each other by a soft edge
    /// (no shards, #935). The scene depth then takes the rigged faces where
    /// half covered only, for the glass, water and particles drawn after
    /// them (no halo, #556).
    Layers,
}

/// Visible draws for a frame, built by the world.
#[derive(Default)]
pub struct DrawLists {
    /// Select clipped depth/shadow variants only while an avatar is entering.
    pub portal_clipping: bool,
    /// Invisible water exclusion surfaces, in the CPU culling fallback.
    pub water_exclusion: Vec<DrawCmd>,
    /// Own HUD attachments, drawn after post-processing with their own depth.
    pub hud_view: Option<HudView>,
    pub hud_opaque: Vec<DrawCmd>,
    pub hud_blend: Vec<DrawCmd>,
    pub hud_debug_red: Vec<DrawCmd>,
    pub hud_debug_blue: Vec<DrawCmd>,
    /// Classic terrain (detail textures).
    pub terrain: Vec<DrawCmd>,
    /// PBR terrain (glTF materials), opaque: its own shader.
    pub terrain_pbr: Vec<DrawCmd>,
    /// PBR terrain whose alpha needs to be tested in the depth prepass.
    pub terrain_mask: Vec<DrawCmd>,
    pub opaque: Vec<DrawCmd>,
    /// Local creation hologram: depth tested, without writing pick depth,
    /// casting shadows or entering reflections / probes.
    pub creation_preview: Vec<DrawCmd>,
    /// Collision shapes of the build selection, depth tested without
    /// affecting world depth, picking, shadows or reflections.
    pub physics_shapes: Vec<DrawCmd>,
    pub opaque_two_sided: Vec<DrawCmd>,
    pub mask: Vec<DrawCmd>,
    pub mask_two_sided: Vec<DrawCmd>,
    pub water: Vec<DrawCmd>,
    pub water_events: Vec<crate::WaterEvent>,
    pub water_contacts: Vec<crate::WaterEvent>,
    pub water_ground: crate::WaterGround,
    /// Sorted back to front, the first `blend_depth` apart (below).
    pub blend: Vec<DrawCmd>,
    /// The blended faces under the water while the camera is above it, back
    /// to front, the rigged first: drawn before the water and the scene copy
    /// it refracts, writing depth, without glow (POOL_ALPHA_PRE_WATER). A
    /// face crossing the surface is in both lists, clipped to each side.
    pub blend_under_water: Vec<DrawCmd>,
    /// The first `blend_depth` entries of `blend` are rigged faces (worn
    /// rigged meshes, animesh): drawn before the other blended faces and
    /// writing depth, so that a layer in front (mesh hair) hides what is
    /// behind it (eyelashes) whatever the order of the two objects
    /// (LLDrawPoolAlpha::render: forwardRender(true) first, depth written).
    pub blend_depth: usize,
    /// How those rigged faces write their depth.
    pub blend_depth_mode: RiggedBlendDepth,
    /// `RiggedBlendDepth::Runs` (one attachment point), `Split` (one object
    /// per point) and `Layers` (one object): where each run of faces ends
    /// in the first `blend_depth` entries of `blend` (increasing, the last
    /// one `blend_depth`).
    pub blend_depth_runs: Vec<u32>,
    /// Per `blend` entry: the face has glow (drawn into the glow right
    /// after it, as LLDrawPoolAlpha's emissives).
    pub blend_glow: Vec<bool>,
    pub shadow_casters: Vec<ShadowCaster>,
    /// Terrain chunks casting shadows: drawn depth only like the opaque
    /// casters, else with the terrain's own alpha test (`fs_shadow_terrain`),
    /// never through `fs_shadow`.
    pub terrain_shadow_casters: Vec<ShadowCaster>,
    /// Opaque and alpha-masked draws for the planar reflection passes
    /// (water and mirror); drawn without face culling, alpha tested.
    /// With their bounds: each reflection pass culls them to its own frustum.
    pub reflection: Vec<ShadowCaster>,
    /// `reflection` without avatars, their attachments and animesh: what a
    /// static reflection probe captures (LLViewerWindow::cubeSnapshot).
    pub probe: Vec<ShadowCaster>,
    pub reflection_terrain: Vec<DrawCmd>,
    /// `reflection_terrain` of the PBR regions (their own shader).
    pub reflection_terrain_pbr: Vec<DrawCmd>,
    /// Particles, sorted back to front.
    pub particles: Vec<ParticleInstance>,
    /// Non-blended faces with glow (LL PASS_GLOW): drawn again to
    /// accumulate the glow amount in the scene alpha, before the blended
    /// faces (which dim the glow behind them).
    pub glow: Vec<DrawCmd>,
    /// Glowing faces drawn opaque here but in LL's alpha pool (rigged or
    /// glowing faces whose texture has an alpha channel, LLFace::canRenderAsMask):
    /// each first dims the glow under it by its alpha, then adds its own,
    /// so stacked body / clothing layers do not add up.
    pub glow_alpha: Vec<DrawCmd>,
    /// Debug "Show transparency" (LLDrawPoolAlpha::renderDebugAlpha):
    /// blended, alpha-masked and invisible faces in translucent red...
    pub debug_red: Vec<DrawCmd>,
    /// ... and faces masked by their material in blue.
    pub debug_blue: Vec<DrawCmd>,
    /// Dev Toolbox "Pass and alpha per face": opaque faces in green, faces
    /// left hidden (fully transparent) in yellow; masked ones go to
    /// `debug_blue`, blended ones to `debug_red`.
    pub debug_green: Vec<DrawCmd>,
    pub debug_yellow: Vec<DrawCmd>,
    /// Parcel ownership and property lines, on existing terrain geometry.
    pub land_overlay: Vec<DrawCmd>,
    /// Build tools: wireframe of the selected mesh / sculpted objects
    /// (LLSelectMgr::renderOneSilhouette draws meshes as wireframes), root
    /// in yellow, other prims of the linkset in blue.
    pub select_root: Vec<DrawCmd>,
    pub select_child: Vec<DrawCmd>,
    /// Selected texture faces (LLPipeline::renderSelectedFaces).
    pub select_faces: Vec<DrawCmd>,
    pub hud_select_faces: Vec<DrawCmd>,
    /// Impostor pictures to take this frame (at most `IMPOSTOR_CAPTURES`).
    pub impostor_captures: Vec<ImpostorCapture>,
    /// Impostor cards to draw.
    pub impostor_sprites: Vec<ImpostorSprite>,
    /// GPU draw lists this frame (gpu_cull.rs): the opaque and masked main
    /// draws, the object shadow casters and the reflection / probe draws
    /// are culled by the GPU from the scene's tables, so those lists hold
    /// only what the CPU still draws (terrain). None: CPU lists only.
    pub gpu: Option<crate::gpu_cull::GpuCullView>,
}

/// Impostor atlas: square tiles of `IMPOSTOR_TILE` pixels.
pub const IMPOSTOR_TILE: u32 = 256;
pub const IMPOSTOR_TILES_PER_ROW: u32 = 8;
pub const IMPOSTOR_TILES: u32 = IMPOSTOR_TILES_PER_ROW * IMPOSTOR_TILES_PER_ROW;
/// At most four 32 MiB atlas pages; offscreen pictures are reclaimed first.
pub const IMPOSTOR_MAX_TILES: u32 = IMPOSTOR_TILES * 4;
/// A visible avatar waiting for a picture uses a cheap silhouette card.
pub const IMPOSTOR_PENDING: u32 = 1 << 30;
/// Impostor pictures taken per frame.
pub const IMPOSTOR_CAPTURES: usize = 2;

/// Picture of an avatar for its impostor: an orthographic view framing its
/// bounding sphere, seen from the camera.
#[derive(Debug, Clone)]
pub struct ImpostorCapture {
    pub tile: u32,
    pub eye: Vec3,
    pub view: Mat4,
    /// Half size of the framed square (m).
    pub half: f32,
    /// Depth range from the eye (m).
    pub depth: f32,
    pub opaque: Vec<DrawCmd>,
    pub blend: Vec<DrawCmd>,
}

/// Flag of `ImpostorSprite::tile`: the picture is all fullbright (the
/// silhouette of an avatar over the complexity limit), so its card takes no
/// haze on a classic sky, as the silhouette drawn in full.
pub const IMPOSTOR_FULLBRIGHT: u32 = 1 << 31;

/// Impostor card: `center ± right ± up` (half extents, world space).
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct ImpostorSprite {
    pub center: [f32; 3],
    /// Atlas tile, with `IMPOSTOR_FULLBRIGHT`.
    pub tile: u32,
    pub right: [f32; 3],
    pub up: [f32; 3],
}

impl DrawLists {
    /// Remove viewer dressing while retaining geometry, particles and required vision restrictions.
    pub(crate) fn strip_viewer_overlays(&mut self) {
        self.hud_view = None;
        self.hud_opaque.clear();
        self.hud_blend.clear();
        self.hud_debug_red.clear();
        self.hud_debug_blue.clear();
        self.creation_preview.clear();
        self.physics_shapes.clear();
        self.debug_red.clear();
        self.debug_blue.clear();
        self.debug_green.clear();
        self.debug_yellow.clear();
        self.land_overlay.clear();
        self.select_root.clear();
        self.select_child.clear();
        self.select_faces.clear();
        self.hud_select_faces.clear();
    }
    /// Make `self` a copy of `src`, keeping the memory `self` already has:
    /// the lists of a frame packet, filled again every frame.
    pub fn copy_from(&mut self, src: &DrawLists) {
        self.hud_view = src.hud_view;
        self.hud_opaque.clone_from(&src.hud_opaque);
        self.hud_blend.clone_from(&src.hud_blend);
        self.hud_debug_red.clone_from(&src.hud_debug_red);
        self.hud_debug_blue.clone_from(&src.hud_debug_blue);
        self.terrain.clone_from(&src.terrain);
        self.terrain_pbr.clone_from(&src.terrain_pbr);
        self.terrain_mask.clone_from(&src.terrain_mask);
        self.portal_clipping = src.portal_clipping;
        self.opaque.clone_from(&src.opaque);
        self.creation_preview.clone_from(&src.creation_preview);
        self.physics_shapes.clone_from(&src.physics_shapes);
        self.opaque_two_sided.clone_from(&src.opaque_two_sided);
        self.mask.clone_from(&src.mask);
        self.mask_two_sided.clone_from(&src.mask_two_sided);
        self.water.clone_from(&src.water);
        self.water_exclusion.clone_from(&src.water_exclusion);
        self.blend.clone_from(&src.blend);
        self.blend_under_water.clone_from(&src.blend_under_water);
        self.blend_glow.clone_from(&src.blend_glow);
        self.blend_depth = src.blend_depth;
        self.blend_depth_mode = src.blend_depth_mode;
        self.blend_depth_runs.clone_from(&src.blend_depth_runs);
        self.shadow_casters.clone_from(&src.shadow_casters);
        self.terrain_shadow_casters.clone_from(&src.terrain_shadow_casters);
        self.reflection.clone_from(&src.reflection);
        self.probe.clone_from(&src.probe);
        self.reflection_terrain.clone_from(&src.reflection_terrain);
        self.reflection_terrain_pbr.clone_from(&src.reflection_terrain_pbr);
        self.particles.clone_from(&src.particles);
        self.water_events.clone_from(&src.water_events);
        self.water_contacts.clone_from(&src.water_contacts);
        self.water_ground.clone_from(&src.water_ground);
        self.glow.clone_from(&src.glow);
        self.glow_alpha.clone_from(&src.glow_alpha);
        self.debug_red.clone_from(&src.debug_red);
        self.debug_blue.clone_from(&src.debug_blue);
        self.debug_green.clone_from(&src.debug_green);
        self.debug_yellow.clone_from(&src.debug_yellow);
        self.land_overlay.clone_from(&src.land_overlay);
        self.select_root.clone_from(&src.select_root);
        self.select_child.clone_from(&src.select_child);
        self.select_faces.clone_from(&src.select_faces);
        self.hud_select_faces.clone_from(&src.hud_select_faces);
        self.impostor_captures.clone_from(&src.impostor_captures);
        self.impostor_sprites.clone_from(&src.impostor_sprites);
        self.gpu = src.gpu;
    }

    pub fn clear(&mut self) {
        self.hud_view = None;
        self.hud_opaque.clear();
        self.hud_blend.clear();
        self.hud_debug_red.clear();
        self.hud_debug_blue.clear();
        self.terrain.clear();
        self.terrain_pbr.clear();
        self.terrain_mask.clear();
        self.portal_clipping = false;
        self.opaque.clear();
        self.creation_preview.clear();
        self.physics_shapes.clear();
        self.opaque_two_sided.clear();
        self.mask.clear();
        self.mask_two_sided.clear();
        self.water.clear();
        self.water_exclusion.clear();
        self.blend.clear();
        self.blend_under_water.clear();
        self.blend_depth = 0;
        self.blend_depth_mode = RiggedBlendDepth::default();
        self.blend_depth_runs.clear();
        self.shadow_casters.clear();
        self.terrain_shadow_casters.clear();
        self.reflection.clear();
        self.probe.clear();
        self.reflection_terrain.clear();
        self.reflection_terrain_pbr.clear();
        self.particles.clear();
        self.water_events.clear();
        self.water_contacts.clear();
        self.water_ground = Default::default();
        self.glow.clear();
        self.blend_glow.clear();
        self.glow_alpha.clear();
        self.debug_red.clear();
        self.debug_blue.clear();
        self.debug_green.clear();
        self.debug_yellow.clear();
        self.land_overlay.clear();
        self.select_root.clear();
        self.select_child.clear();
        self.select_faces.clear();
        self.hud_select_faces.clear();
        self.impostor_captures.clear();
        self.impostor_sprites.clear();
        self.gpu = None;
    }

    pub fn total(&self) -> usize {
        self.water_exclusion.len()
            + self.hud_opaque.len()
            + self.hud_blend.len()
            + self.hud_debug_red.len()
            + self.hud_debug_blue.len()
            + self.terrain.len()
            + self.terrain_pbr.len()
            + self.terrain_mask.len()
            + self.opaque.len()
            + self.creation_preview.len()
            + self.physics_shapes.len()
            + self.opaque_two_sided.len()
            + self.mask.len()
            + self.mask_two_sided.len()
            + self.water.len()
            + self.blend.len()
            + self.blend_under_water.len()
    }
}

/// One camera-facing particle quad (instance data, 48 bytes).
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable, Default)]
pub struct ParticleInstance {
    pub pos: [f32; 3],
    /// Bindless texture slot.
    pub texture: u32,
    pub size: [f32; 2],
    /// `particle_flags` bits.
    pub flags: u32,
    /// Straight RGBA, sRGB encoded.
    pub color: [u8; 4],
    /// Orientation axis for ribbons / velocity-aligned particles.
    pub axis: [f32; 3],
    pub glow: f32,
}

pub mod particle_flags {
    /// Not lit by the environment.
    pub const EMISSIVE: u32 = 1;
    /// Additive blending (destination factor ONE).
    pub const ADDITIVE: u32 = 2;
    /// Quad stretched along its world-space `axis` (ribbons).
    pub const AXIS: u32 = 4;
    /// Velocity-oriented billboard: project its axis onto the viewing plane.
    pub const FOLLOW_VELOCITY: u32 = 8;
}

#[derive(Debug, Clone, Copy)]
pub struct PointLight {
    pub position: Vec3,
    pub radius: f32,
    /// Linear color premultiplied by intensity.
    pub color: Vec3,
    pub falloff: f32,
    /// A light with a texture lights its frustum only, through the texture.
    pub projector: Option<Projector>,
}

/// Frustum and texture of a projector (a prim light with a light texture),
/// as LLPipeline::setupSpotLight gives them to spotLightF.glsl (originally
/// LGPL 2.1, Linden Research / Firestorm), in world space instead of screen
/// space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Projector {
    /// World to the projected texture: after the divide by w, x and y are
    /// in 0..1 inside the frustum; z < 0 (before the divide) behind its
    /// near plane.
    pub matrix: Mat4,
    /// Apex of the frustum (angular attenuation is taken from it).
    pub origin: Vec3,
    /// Direction the projector points to (the prim's -Z).
    pub direction: Vec3,
    /// Distance from the apex to the face of the prim the light leaves.
    pub near: f32,
    /// Depth of the frustum, from that face to its far plane.
    pub range: f32,
    /// Distance from the light at which the texture starts to blur.
    pub focus: f32,
    pub ambiance: f32,
    /// Texture table slot of the projected texture.
    pub texture: u32,
    /// World to the clip space of the projector's shadow map (depth 0 at
    /// its near plane, 1 at its reach), as LLPipeline::generateSunShadow
    /// draws the shadow of a projector.
    pub shadow_matrix: Mat4,
}

/// A projector of the frame's light list that casts shadows: Firestorm draws
/// the shadows of two projectors at a time (LLPipeline::mShadowSpotLight).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpotShadow {
    /// Index of the projector in `FrameParams::lights`.
    pub light: u32,
    /// 0 to 1: its shadows fade in when the projector takes one of the two
    /// shadow maps, and out when another one takes it.
    pub fade: f32,
}

impl Projector {
    /// The frustum of a prim at `position` turned by `rotation`, of size
    /// `scale`: it leaves the prim's -Z face and reaches `reach` (the
    /// light's radius x 1.5) from the prim. `params`: field of view
    /// (radians, vertical), focus and ambiance (LLLightImageParams).
    pub fn new(position: Vec3, rotation: glam::Quat, scale: Vec3, reach: f32, params: Vec3, texture: u32) -> Projector {
        // a null field of view has no frustum (LL divides by its tangent)
        let fov = params.x.clamp(0.01, 3.1);
        let scale = scale.max(Vec3::splat(1e-3));
        let direction = (rotation * Vec3::NEG_Z).normalize_or(Vec3::NEG_Z);
        // near plane: the -Z face; the apex is where a frustum of this
        // field of view has the face's height
        let face = position + direction * (scale.z * 0.5);
        let near = (scale.y * 0.5) / (fov * 0.5).tan();
        let origin = face - direction * near;
        let far = (reach + near - scale.z).max(near + 1e-3);
        let light_to_world = Mat4::from_rotation_translation(rotation, origin);
        // glm::perspective(fov, width / height, near, far)
        let f = 1.0 / (fov * 0.5).tan();
        let aspect = scale.x / scale.y;
        let perspective = Mat4::from_cols(
            Vec4::new(f / aspect, 0.0, 0.0, 0.0),
            Vec4::new(0.0, f, 0.0, 0.0),
            Vec4::new(0.0, 0.0, (far + near) / (near - far), -1.0),
            Vec4::new(0.0, 0.0, 2.0 * far * near / (near - far), 0.0),
        );
        // clip space (-1..1) to texture space (0..1)
        let bias = Mat4::from_translation(Vec3::splat(0.5)) * Mat4::from_scale(Vec3::splat(0.5));
        // the shadow map: from just past the face to the light's reach
        let (sn, sf) = (near + 0.01, near + reach.max(0.02));
        let shadow_perspective = Mat4::from_cols(
            Vec4::new(f / aspect, 0.0, 0.0, 0.0),
            Vec4::new(0.0, f, 0.0, 0.0),
            Vec4::new(0.0, 0.0, sf / (sn - sf), -1.0),
            Vec4::new(0.0, 0.0, sn * sf / (sn - sf), 0.0),
        );
        Projector {
            shadow_matrix: shadow_perspective * light_to_world.inverse(),
            matrix: bias * perspective * light_to_world.inverse(),
            origin,
            direction,
            near,
            range: far - near,
            focus: params.y,
            ambiance: params.z,
            texture,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AntiAliasing {
    None,
    Fxaa,
    /// Multisampling with the given sample count (2, 4 or 8).
    Msaa(u32),
    Taa,
    /// SMAA 1x, preset high (Firestorm RenderFSAAType 2).
    Smaa,
}

/// A photo drawn at its own size (snapshot window): the frame that carries
/// it is drawn offscreen at `width` x `height` instead of the window, with
/// the same passes, then read back (LLViewerWindow::rawSnapshot, without
/// its tiles: one pass while the card holds the size).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotSpec {
    pub width: u32,
    pub height: u32,
    /// The interface drawn over the scene, scaled from the window by
    /// `height / window height` (RenderUIInSnapshot).
    pub interface: bool,
    /// HUD attachments drawn (RenderHUDInSnapshot).
    pub huds: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tonemapper {
    /// Khronos PBR Neutral (Second Life's tone mapper).
    KhronosNeutral,
    Aces,
}

/// Quality settings applied with `Renderer::apply_settings`.
/// The ambient occlusion method (Preferences › Graphics).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AoMethod {
    /// The Legacy Viewer's screen-space occlusion (ssao.wgsl), its two
    /// levels from `ssao`.
    Legacy,
    /// Ground-truth ambient occlusion (gtao.rs), quality 0 low … 3 ultra.
    Gtao { quality: u8 },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderSettings {
    /// Percentage of the window resolution used by the world; UI stays native.
    pub render_scale_percent: u32,
    pub water_effects: crate::WaterEffects,
    pub aa: AntiAliasing,
    pub tonemapper: Tonemapper,
    /// "HDR and Emissive" (RenderDisableVintageMode, which drives
    /// RenderHDREnabled and RenderEnableEmissiveBuffer). Off, the Legacy
    /// Viewer's "vintage" image: no exposure, tone mapping nor sharpening
    /// (LLPipeline::gammaCorrect), no emissive on the PBR faces drawn into
    /// its G-buffer (HAS_EMISSIVE) and reflection probes in 8 bits.
    pub hdr: bool,
    /// Shadow map resolution per cascade (0 = shadows off).
    pub shadow_resolution: u32,
    /// Screen-space ambient occlusion: 0 off, 1 low, 2 high.
    pub ssao: u8,
    /// How the occlusion `ssao` turns on is computed.
    pub ao_method: AoMethod,
    /// With GTAO: its occlusion as a visibility bitmask and one bounce of
    /// light from the surfaces around (gtao.rs, `main_bitmask`).
    pub screen_indirect: bool,
    /// Its strength (1 = one physical bounce of a Lambert surface).
    pub screen_indirect_intensity: f32,
    /// Screen-space contact shadows of the sun or the moon (contact_shadows.rs),
    /// with the sun shadows only.
    pub contact_shadows: bool,
    /// Geometric specular anti-aliasing of the objects' highlights
    /// (common.wgsl `specular_aa_roughness`, AURORA_SPECULAR_AA).
    pub specular_aa: bool,
    /// Planar water reflections: fraction of the screen resolution (0 = off).
    pub water_reflection_scale: f32,
    /// Mirrors (reflection probes flagged as mirror): fraction of the screen
    /// resolution (0 = off).
    pub mirror_scale: f32,
    /// Screen-space reflections on glossy surfaces.
    pub ssr: bool,
    /// Anisotropic texture filtering (1 = off, 2, 4, 8, 16).
    pub anisotropy: u16,
    /// Reflection probe cube slots, the default probe included
    /// (RenderReflectionProbeCount; 1..=probes::MAX_PROBES).
    pub probe_slots: u32,
    /// GPU occlusion culling (UseOcclusion).
    pub occlusion: bool,
    /// Order-independent transparency (oit.rs): the blended faces are sorted
    /// per pixel rather than per object. Off, or on a GPU without writable
    /// storage in fragment shaders, they are blended in the drawing order.
    pub oit: bool,
    /// Its node pool: blended fragments per screen pixel on average
    /// (`oit::MIN_NODES_PER_PIXEL..=oit::MAX_NODES_PER_PIXEL`), within
    /// `oit::MAX_POOL_BYTES`.
    pub oit_nodes_per_pixel: u32,
    /// Layers sorted exactly in each pixel (`1..=oit::MAX_SORTED_LAYERS`);
    /// the farther ones are blended as a weighted average.
    pub oit_max_layers: u32,
}

impl Default for RenderSettings {
    fn default() -> Self {
        RenderSettings {
            render_scale_percent: 100,
            water_effects: crate::WaterEffects::default(),
            // the state `Renderer::new` builds its pipelines and targets
            // for, not the viewer's default (SMAA, in its own settings)
            aa: AntiAliasing::Msaa(4),
            tonemapper: Tonemapper::KhronosNeutral,
            hdr: true,
            shadow_resolution: 2048,
            ssao: 1,
            ao_method: AoMethod::Legacy,
            screen_indirect: false,
            screen_indirect_intensity: 1.0,
            contact_shadows: false,
            specular_aa: true,
            water_reflection_scale: 0.25,
            mirror_scale: 0.25,
            ssr: false,
            anisotropy: 8,
            probe_slots: 32,
            occlusion: true,
            oit: true,
            oit_nodes_per_pixel: crate::oit::DEFAULT_NODES_PER_PIXEL,
            oit_max_layers: crate::oit::DEFAULT_SORTED_LAYERS,
        }
    }
}

/// Atmospheric sky parameters (EEP "legacy haze" model, the one Second
/// Life's sky shaders use). Colors are in the shaders' sRGB-like space.
#[derive(Debug, Clone, Copy)]
pub struct SkyParams {
    /// Direction towards the dominant light (sun, or moon at night), z-up,
    /// with z clamped to >= -0.1.
    pub light_norm: Vec3,
    /// Light color of the sky dome and clouds (sunlight, or moonlight * 0.7
    /// in skyV.glsl).
    pub sunlight: Vec3,
    /// Light color of objects, terrain and water (LLPipeline mSunDiffuse /
    /// mMoonDiffuse: the sky's sunlight scaled so its largest component is
    /// at most 1, then clamped; black when neither sun nor moon is up).
    pub object_sunlight: Vec3,
    /// Sky gamma: Firestorm's legacyGamma on skies without reflection probe
    /// ambiance (post process, 1 = no change).
    pub gamma: f32,
    pub ambient: Vec3,
    pub blue_horizon: Vec3,
    pub blue_density: Vec3,
    pub haze_horizon: f32,
    pub haze_density: f32,
    pub density_multiplier: f32,
    pub distance_multiplier: f32,
    pub max_y: f32,
    pub glow: Vec3,
    pub sun_moon_glow_factor: f32,
    pub cloud_shadow: f32,
    pub cloud_color: Vec3,
    pub cloud_scale: f32,
    pub cloud_variance: f32,
    /// xy: scrolled position, z: density.
    pub cloud_pos_density1: Vec3,
    pub cloud_pos_density2: Vec3,
    pub dome_offset: f32,
    pub dome_radius: f32,
    /// Sky brightness scale (`sky_hdr_scale`).
    pub hdr_scale: f32,
    /// Classic sky (no reflection probe ambiance): SL lights objects with
    /// its legacy model and does not tone map.
    pub classic: bool,
    /// Reflection probe ambiance is 0: no tone mapping (Firestorm no_post).
    pub no_post: bool,
    /// The sky ambient keeps its colour at full strength, as the first PBR
    /// release lit it (an experimental setting): the classic and linear
    /// shading paths read it (common.wgsl
    /// `colored_sky_ambient`), the HDR one gets it in the hemisphere colours.
    pub colored_ambient: bool,
    /// True sun / moon directions (for their discs).
    pub sun_dir: Vec3,
    pub moon_dir: Vec3,
    pub sun_scale: f32,
    pub moon_scale: f32,
    pub moon_brightness: f32,
    pub star_brightness: f32,
    /// Bindless slots (0 = none): cloud noise, sun, moon.
    pub cloud_texture: u32,
    pub sun_texture: u32,
    pub moon_texture: u32,
    /// Rainbow strength (moisture level, 0 with neither sun nor moon up),
    /// droplet radius and 22° halo strength (ice level), as skyF.glsl.
    pub moisture_level: f32,
    pub droplet_radius: f32,
    pub ice_level: f32,
    /// Bindless slots (0 = none) of the rainbow and halo textures.
    pub rainbow_texture: u32,
    pub halo_texture: u32,
    /// Night sky: which stars and how they are turned (stars.rs).
    pub stars: crate::stars::StarFrame,
}

impl Default for SkyParams {
    /// Second Life's default midday sky.
    fn default() -> Self {
        let sun = Vec3::new(0.0, 0.7, 0.7).normalize();
        SkyParams {
            light_norm: sun,
            sunlight: Vec3::new(0.7342, 0.7815, 0.8999),
            object_sunlight: Vec3::new(0.7342, 0.7815, 0.8999),
            gamma: 1.0,
            ambient: Vec3::new(0.25, 0.25, 0.25),
            blue_horizon: Vec3::new(0.4954, 0.4954, 0.6399),
            blue_density: Vec3::new(0.2447, 0.4487, 0.7599),
            haze_horizon: 0.1899,
            haze_density: 0.6999,
            density_multiplier: 0.0001799,
            distance_multiplier: 0.8,
            max_y: 1605.0,
            glow: Vec3::new(5.0, 0.001, -0.4799),
            sun_moon_glow_factor: 1.0,
            cloud_shadow: 0.2699,
            cloud_color: Vec3::splat(0.4099),
            cloud_scale: 0.4199,
            cloud_variance: 0.0,
            cloud_pos_density1: Vec3::new(1.0, 0.526, 1.0),
            cloud_pos_density2: Vec3::new(1.0, 0.526, 1.0),
            dome_offset: 0.96,
            dome_radius: 15000.0,
            hdr_scale: 1.0,
            classic: true,
            no_post: true,
            colored_ambient: false,
            sun_dir: sun,
            moon_dir: -sun,
            sun_scale: 1.0,
            moon_scale: 1.0,
            moon_brightness: 0.5,
            star_brightness: 250.0,
            cloud_texture: 0,
            sun_texture: 0,
            moon_texture: 0,
            moisture_level: 0.0,
            droplet_radius: 800.0,
            ice_level: 0.0,
            rainbow_texture: 0,
            halo_texture: 0,
            stars: crate::stars::StarFrame::default(),
        }
    }
}

/// Water appearance (EEP water track).
#[derive(Debug, Clone, Copy)]
pub struct WaterParams {
    /// Fog color (sRGB) and density (`water_fog_density`).
    pub fog_color: Vec3,
    pub fog_density: f32,
    pub underwater_fog_mod: f32,
    pub fresnel_scale: f32,
    pub fresnel_offset: f32,
    pub blur_multiplier: f32,
    pub normal_scale: Vec3,
    pub wave1: glam::Vec2,
    pub wave2: glam::Vec2,
    /// Bindless slot of the wave normal map (0 = analytic waves).
    pub normal_texture: u32,
}

impl Default for WaterParams {
    fn default() -> Self {
        WaterParams {
            fog_color: Vec3::new(0.0156, 0.149, 0.2509),
            fog_density: 2.0,
            underwater_fog_mod: 0.25,
            fresnel_scale: 0.3999,
            fresnel_offset: 0.5,
            blur_multiplier: 0.04,
            normal_scale: Vec3::splat(2.0),
            wave1: glam::Vec2::new(1.05, -0.42),
            wave2: glam::Vec2::new(1.11, -1.16),
            normal_texture: 0,
        }
    }
}

/// A planar mirror (reflection probe flagged as mirror, SL "hero probe").
#[derive(Debug, Clone, Copy)]
pub struct MirrorParams {
    pub point: Vec3,
    /// Plane normal (the probe's local +Z), facing the camera.
    pub normal: Vec3,
    /// World -> probe box local space ([-0.5, 0.5]^3 inside).
    pub world_to_box: Mat4,
}

/// Camera and environment for one frame.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct OverlayEffect {
    pub color: [f32; 4],
    pub texture: [u32; 4],
}

/// Camera and environment for one frame.
#[derive(Debug, Clone)]
pub struct FrameParams {
    /// Session-local scripted vision effects; empty keeps the normal render path.
    pub rlva_spheres: Vec<SphereEffect>,
    pub rlva_resolution_divisor: u32,
    pub rlva_overlays: Vec<OverlayEffect>,
    /// Linear RGBA from the UI palette for selected-face overlays.
    pub selection_color: [f32; 4],
    /// The rez effect's colors (`rez_apply` in object.wgsl): the brand
    /// gradient of the UI palette, linear, from the line's leading edge to
    /// its trail; the first entry's w is the look (1 grid, 2 scanlines,
    /// 3 wireframe).
    pub rez_gradient: [[f32; 4]; 3],
    /// Brief coverage highlight, zero once the material can be read unaltered.
    pub selection_pulse: f32,
    /// Logical halo width scaled to display pixels.
    pub selection_scale: f32,
    pub view: Mat4,
    pub proj: Mat4,
    /// A photo drawn in tiles: the part of it this frame draws, `proj`
    /// being this times the whole photo's projection (the viewer's
    /// `snapshot::tile_matrix`); identity otherwise. The sun's cascades
    /// and their casters are fitted to the whole photo, so that every tile
    /// reads the shadow maps of one pass.
    pub photo_tile: Mat4,
    /// How many times taller, in pixels, the image is than the viewfinder's
    /// view of the same field: 1 in the window, the photo's height over
    /// the viewfinder's in a photo. The screen-space effects measured in
    /// pixels (the contact shadows) keep their size in the image with it.
    pub photo_scale: f32,
    pub camera_pos: Vec3,
    pub near: f32,
    pub far: f32,
    pub fov_y: f32,
    pub time: f32,
    /// The instant the rez effects and the portal swirl are drawn at
    /// (`AnimClock`): held for a photo in tiles, else None for now.
    pub anim_at: Option<std::time::Instant>,
    /// The instant the animated textures are drawn at: held for a photo
    /// in tiles and while the photo mode freezes the world (whose objects
    /// still rez in as the camera finds them), else None for now.
    pub tex_anim_at: Option<std::time::Instant>,
    pub sun_dir: Vec3,
    pub sun_visible: f32,
    pub sun_color: Vec3,
    pub sky_zenith: Vec3,
    pub sky_horizon: Vec3,
    pub ground_color: Vec3,
    pub fog_color: Vec3,
    pub fog_density: f32,
    pub exposure: f32,
    /// The automatic exposure held where it is (the photo viewfinder's
    /// AE-L), as for a photo.
    pub exposure_lock: bool,
    /// Share of the tone mapped color over the exposed linear one on skies
    /// that are not classic (RenderTonemapMix, 0.7 in the Legacy Viewer).
    pub tonemap_mix: f32,
    /// Contrast adaptive sharpening strength (0 = off; Firestorm RenderCASSharpness).
    pub sharpen: f32,
    /// Screen-space glow of faces with a glow value (RenderGlow).
    pub glow: bool,
    /// Aurora's bloom (bloom.rs): the share of the light over the
    /// threshold spread around the bright parts, 0 = off.
    pub bloom: f32,
    pub water_height: f32,
    pub shadows: bool,
    /// Distance covered by the shadow cascades.
    pub shadow_distance: f32,
    pub lights: Vec<PointLight>,
    /// The projectors of `lights` that cast shadows (drawn with `shadows`).
    pub spot_shadows: [Option<SpotShadow>; 2],
    pub clear_color: Vec4,
    pub sky: SkyParams,
    pub water: WaterParams,
    pub mirror: Option<MirrorParams>,
    pub probes: ProbeFrame,
    /// Debug: triangle edges over the scene.
    pub wireframe: bool,
    /// Debug: glow amount view (red = glow x10, green x2) instead of the image.
    pub debug_glow: bool,
    /// Comparison ([`BlendDiscard::Alternate`]): this frame lights the
    /// invisible fragments of the blended faces before dropping them.
    pub blend_late_discard: bool,
    /// Comparison ([`crate::light_tiles::LightCulling::Alternate`]): this
    /// frame's fragments walk the whole light list.
    pub all_lights: bool,
    /// Comparison ([`crate::probes::ProbeWalk::Alternate`]): this frame's
    /// fragments walk the whole probe list.
    pub probe_walk_list: bool,
    /// Debug views and checks asked from the interface.
    pub debug: DebugRender,
    /// Depth of field (RenderDepthOfField); None: off, at no cost.
    pub dof: Option<DepthOfField>,
    /// A cut-out photo (cutout.rs): no sky, terrain or water, no glow, and
    /// what is drawn keeps its coverage, in the snapshot's alpha or over a
    /// checkerboard on the live view.
    pub cutout: bool,
    /// The photo's look (looks.rs), over the image once tone mapped; None:
    /// none, at no cost.
    pub look: Option<crate::looks::PhotoLook>,
}

#[derive(Debug, Clone, Copy)]
pub struct SphereEffect {
    pub origin: Vec3,
    pub mode: u32,
    pub extend: u32,
    pub params: [f32; 4],
    pub range: [f32; 4],
}

/// The lens of the depth of field, worked out by the viewer for the frame
/// (LLPipeline::renderDoF's uniforms of cofF.glsl).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DepthOfField {
    /// Circle of confusion in pixels of the image for (s - z) / z, s the
    /// focus distance and z a point's distance along the view axis.
    pub coc_scale: f32,
    /// Focus distance along the view axis (m).
    pub focus: f32,
    /// The far end of the zone kept sharp (m), `focus` for a single plane:
    /// the photo viewfinder's group focus keeps every avatar between its
    /// nearest (`focus`) and its farthest sharp, the blur growing beyond
    /// each end as a lens's from its plane.
    pub focus_far: f32,
    /// Largest circle (pixels of the image; CameraMaxCoF at the window's
    /// height).
    pub max_coc: f32,
    /// Samples of the blur's gather.
    pub samples: u32,
    /// Debug: near field red, far field blue, sharp grey.
    pub debug: bool,
    /// Focus peaking (the photo viewfinder's manual focus): the sharp
    /// edges painted this linear colour.
    pub peaking: Option<[f32; 3]>,
}

/// Views shown instead of the image, and checks painted over it
/// (Preferences › Debug). The environment variables that ask for the same
/// still do, whatever this says.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DebugRender {
    /// The shadow cascade each pixel reads, darkened by its shadow
    /// (AURORA_DEBUG_SHADOWS).
    pub shadows: bool,
    /// The ambient occlusion of the screen alone.
    pub ambient_occlusion: bool,
    /// Magenta where a local light reaches a fragment without being in
    /// its sets (AURORA_LIGHT_TILES=check).
    pub check_light_sets: bool,
    /// Magenta where a reflection probe covers a fragment without being
    /// in its sets (AURORA_PROBE_WALK=check).
    pub check_probe_sets: bool,
}

/// Where the blended faces drop their invisible fragments (`fs_blend` in
/// object.wgsl; AURORA_BLEND_EARLY_DISCARD). The image is the same.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlendDiscard {
    /// Before the lighting (the default).
    Early,
    /// After it, as before (`0`).
    Late,
    /// Both pipelines, each frame choosing with
    /// [`FrameParams::blend_late_discard`] (`alt`).
    Alternate,
}

impl BlendDiscard {
    pub fn from_env() -> BlendDiscard {
        static MODE: std::sync::OnceLock<BlendDiscard> = std::sync::OnceLock::new();
        *MODE.get_or_init(|| match std::env::var("AURORA_BLEND_EARLY_DISCARD").as_deref().map(str::trim) {
            Ok("0") => BlendDiscard::Late,
            Ok("alt") => BlendDiscard::Alternate,
            _ => BlendDiscard::Early,
        })
    }
}

/// One reflection probe as the scene shaders see it (reflectionProbeF.glsl
/// refSphere / refParams / refIndex / refBox, in world space).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, Pod, Zeroable)]
pub struct ProbeGpu {
    /// Influence sphere center and radius.
    pub center_radius: [f32; 4],
    /// x irradiance (ambiance) scale, y radiance scale, z fade in (0..1),
    /// w kind: 0 automatic, 1 manual sphere, -1 manual box.
    pub params: [f32; 4],
    /// x cube slot in the probe array.
    pub slot: [u32; 4],
    /// World -> unit box ([-1, 1]^3 inside) for box probes.
    pub box_inv: [[f32; 4]; 4],
}

/// One cube face to capture this frame.
#[derive(Debug, Clone, Copy)]
pub struct ProbeCapture {
    pub slot: u32,
    pub origin: Vec3,
    /// Near clip of the capture (LLReflectionMap::getNearClip).
    pub near: f32,
    /// Cube face 0..5 (+X, -X, +Y, -Y, +Z, -Z).
    pub face: u32,
    /// Default probe: sky and terrain only.
    pub sky_only: bool,
    /// Last face of the cube: filter the radiance mips into `slot` afterwards.
    pub finish: bool,
}

/// Reflection probes for one frame (LLReflectionMapManager).
#[derive(Debug, Clone, Default)]
pub struct ProbeFrame {
    pub enabled: bool,
    /// Index 0 is the default (sky) probe.
    pub probes: Vec<ProbeGpu>,
    pub capture: Option<ProbeCapture>,
}

/// The shadow pass of a frame, by cascade (near to far).
#[derive(Debug, Clone, Copy, Default)]
pub struct ShadowStats {
    /// Casters drawn in each cascade.
    pub draws: [u32; 3],
    /// Of which those of the CPU lists (with the GPU lists: the terrain).
    pub cpu_draws: [u32; 3],
    /// With AURORA_PROFILE and the GPU lists: triangles of the GPU casters,
    /// then the rigged ones (avatars, rigged meshes) and their triangles.
    pub triangles: [u32; 3],
    pub rigged_draws: [u32; 3],
    pub rigged_triangles: [u32; 3],
    /// What `GpuElement::ShadowsDepth` is made of (ms): the GPU culling,
    /// the three cascades, then the depth prepass with the occlusion; None
    /// without in-pass timestamps.
    pub gpu_ms: Option<[f32; 5]>,
}

/// Per-frame statistics reported by the renderer.
#[derive(Debug, Clone, Copy, Default)]
pub struct RenderStats {
    /// A swapchain image was presented, including the egui frame.
    pub presented: bool,
    /// Presentation mode the swapchain is configured with (present.rs);
    /// None until the backend has answered a frame.
    pub present_mode: Option<wgpu::PresentMode>,
    /// The settings the frame was drawn with: the MSAA samples and SSR of
    /// the previous ones while the pipelines for new ones compile (#1746).
    /// None until the backend has answered a frame.
    pub settings: Option<RenderSettings>,
    pub draws: u32,
    pub shadow_draws: u32,
    pub shadow: ShadowStats,
    pub triangles: u64,
    pub textures: u32,
    pub texture_bytes: u64,
    /// Texture pages (textures.rs) and their allocated memory.
    pub texture_pages: u32,
    pub texture_page_bytes: u64,
    pub geometry_bytes: u64,
    pub vertex_used: u64,
    pub index_used: u64,
    pub records: u32,
    pub gpu_ms: Option<f32>,
    /// The world pixels of the frame `gpu_ms` measures (a photo frame's are
    /// the photo's).
    pub gpu_pixels: u64,
    /// CPU time the main thread spent on the frame's rendering (ms): the
    /// whole encoding when the renderer runs on it, only the frame packet
    /// with a render thread.
    pub cpu_encode_ms: f32,
    pub particles: u32,
    pub water_reflection: bool,
    pub mirror: bool,
    /// Draws hidden by the GPU occlusion (a frame or two late; None = off).
    pub occluded: Option<u32>,
    /// The draw lists were culled on the GPU this frame (gpu_cull.rs).
    pub gpu_cull: bool,
    /// GPU draw lists: objects in view (a frame or two late).
    pub visible_objects: Option<u32>,
    /// GPU time by kind of element (ms), in [`GpuElement::ALL`] order;
    /// None without in-pass timestamps.
    pub gpu_elements: Option<[f32; GpuElement::ALL.len()]>,
    /// Triangles of the blended faces of the scene pass.
    pub blend_triangles: u64,
    /// Local lights of the frame's list and its reflection probes: what a
    /// lit fragment may have to walk.
    pub lights: u32,
    pub probes: u32,
    /// The frame's fragments walked the lights of their screen tile and
    /// depth only (light_tiles.rs).
    pub light_tiles: bool,
    /// The frame's blended faces dropped their invisible fragments before
    /// the lighting ([`BlendDiscard`]).
    pub blend_early_discard: bool,
    /// GPU time of the depth prepasses alone (ms), both part of
    /// [`GpuElement::ShadowsDepth`]: the single-sample prepass with the
    /// occlusion culling, and with MSAA the prepass of the multisampled
    /// depth the scene pass starts from (0 otherwise).
    pub gpu_depth: Option<[f32; 2]>,
    /// GPU time of the ambient occlusion alone (ms), part of
    /// [`GpuElement::Effects`]: the Legacy pass or GTAO's.
    pub gpu_ssao: Option<f32>,
    /// GPU time of the contact shadows alone (ms), part of
    /// [`GpuElement::Effects`].
    pub gpu_contact: Option<f32>,
    /// GPU time of the bloom levels alone (ms), part of
    /// [`GpuElement::Post`] (0 while off).
    pub gpu_bloom: Option<f32>,
    /// GPU time of the rest of [`GpuElement::Effects`] (ms): the planar
    /// reflections (with the refracted water light), the reflection probe,
    /// the scene copy (with the cut-out pass and the submerged water
    /// particles), the SSR.
    pub gpu_effects: Option<[f32; 4]>,
    /// CPU time of each step of the renderer's frame (ms), in
    /// [`RENDER_PHASES`] order; "acquire" is the wait for the swapchain.
    /// Spent on the render thread when there is one (then they are those
    /// of the frame before: its result comes back with the next hand-over).
    pub cpu_phases: [f32; RENDER_PHASES.len()],
    /// Part of each step above its thread spent off the processor (ms):
    /// waiting for a lock, or for a core. Measured with AURORA_PROFILE,
    /// where the thread's CPU time is known (thread_cpu.rs); 0 otherwise.
    pub cpu_waits: [f32; RENDER_PHASES.len()],
    /// Interface: egui shapes of the frame, the meshes tessellated from
    /// them (one draw each) and their vertices.
    pub ui_shapes: u32,
    pub ui_meshes: u32,
    pub ui_vertices: u32,
    /// Draw commands recorded (multi-draws, fullscreen and sprite draws).
    pub draw_calls: u32,
    /// Bytes of draw records and joint palettes sent to the GPU this frame.
    pub records_uploaded: u64,
    pub palettes_uploaded: u64,
    /// Bytes of the culling tables (faces, objects, joint boxes) and of the
    /// vertices deformed in place (flexible prims) sent this frame.
    pub cull_uploaded: u64,
    pub vertices_uploaded: u64,
    /// The frame was drawn by the render thread (render_thread.rs); false
    /// with AURORA_RENDER_THREAD=0.
    pub render_thread: bool,
    /// Main thread: time to build the frame packet and hand it over (ms),
    /// without the wait below.
    pub packet_ms: f32,
    /// Main thread: time waited for the render thread to finish the frame
    /// before (back-pressure, ms); also the whole wait of a frame that
    /// carries a capture.
    pub wait_render_ms: f32,
    /// Render thread: its whole time on the frame (ms), from the packet to
    /// the end of the presentation, swapchain wait included.
    pub thread_ms: f32,
    /// Render thread: time it waited for the main thread's packet (ms).
    pub thread_idle_ms: f32,
    /// From the moment the frame began reading its input to its
    /// presentation (ms; None when it was not presented). What the GPU
    /// still has to draw after `present` is not in it.
    pub input_ms: Option<f32>,
    /// Main thread: the sleep the frame pacing asks before the next frame
    /// reads its input (pacing.rs, ms).
    pub pace_ms: f32,
    /// Bytes of the frame's write journal (writes.rs).
    pub journal_bytes: u64,
}

/// Steps of the renderer's frame (`Backend::render`), in frame order
/// (AURORA_PROFILE).
pub const RENDER_PHASES: [&str; 17] = [
    "acquire",
    "tessellate",
    "resources",
    "lists",
    "setup",
    "shadows",
    "prepass",
    "ssao",
    "reflections",
    "impostors",
    "scene_a",
    "scene_b",
    "post",
    "egui",
    "finish",
    "submit",
    "present",
];

/// What the GPU time of a frame goes to (performance panel). Each one
/// gathers the passes and draw groups that render it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GpuElement {
    /// Shadow cascades, depth prepass and occlusion culling.
    ShadowsDepth,
    /// SSAO, planar reflections, reflection probes, SSR.
    Effects,
    TerrainSky,
    /// Opaque and alpha-masked faces (prims, meshes, avatars), impostors.
    Objects,
    Water,
    /// Blended faces and particles.
    Transparent,
    /// Glow, TAA, tone mapping, SMAA.
    Post,
}

impl GpuElement {
    /// In frame order.
    pub const ALL: [GpuElement; 7] = [
        GpuElement::ShadowsDepth,
        GpuElement::Effects,
        GpuElement::TerrainSky,
        GpuElement::Objects,
        GpuElement::Water,
        GpuElement::Transparent,
        GpuElement::Post,
    ];

    /// Short key for the profile log.
    pub fn key(self) -> &'static str {
        match self {
            GpuElement::ShadowsDepth => "shadows_depth",
            GpuElement::Effects => "effects",
            GpuElement::TerrainSky => "terrain_sky",
            GpuElement::Objects => "objects",
            GpuElement::Water => "water",
            GpuElement::Transparent => "transparent",
            GpuElement::Post => "post",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terrain_frames_keep_direction_precision_in_the_existing_eight_bytes() {
        for x in -3..=3 {
            for y in -3..=3 {
                for z in -3..=3 {
                    let n = Vec3::new(x as f32, y as f32, z as f32);
                    if n.length_squared() == 0.0 {
                        continue;
                    }
                    let n = n.normalize();
                    let t = n.cross(if n.z.abs() > 0.9 { Vec3::X } else { Vec3::Z }).normalize();
                    let frame = pack_terrain_frame(n, t);
                    let (dn, dt) = unpack_terrain_frame(frame);
                    assert!(dn.distance(n) < 0.0002 && dt.distance(t) < 0.0002, "{n} {t} -> {dn} {dt}");
                }
            }
        }
        assert_eq!(std::mem::size_of::<SkinVertex>(), 8);
        assert_eq!(std::mem::size_of::<Vertex>(), 24);
    }

    /// Texture coordinates of a world point, None behind the near plane.
    fn projected(p: &Projector, at: Vec3) -> Option<Vec3> {
        let c = p.matrix * at.extend(1.0);
        (c.z >= 0.0).then(|| c.truncate() / c.w)
    }

    #[test]
    fn a_projector_lights_the_frustum_in_front_of_its_face() {
        // a prim 0.5 m wide and high, 0.2 m deep, pointing down
        let turn = glam::Quat::IDENTITY;
        let fov = std::f32::consts::FRAC_PI_2;
        let p = Projector::new(
            Vec3::new(10.0, 20.0, 30.0),
            turn,
            Vec3::new(0.5, 0.5, 0.2),
            15.0,
            Vec3::new(fov, 2.0, 0.3),
            7,
        );
        assert!((p.direction - Vec3::NEG_Z).length() < 1e-6);
        // the apex is behind the face: half its height at 45 degrees
        assert!((p.near - 0.25).abs() < 1e-6);
        assert!((p.origin - Vec3::new(10.0, 20.0, 30.0 - 0.1 + 0.25)).length() < 1e-5);
        assert!((p.range - (15.0 - 0.2)).abs() < 1e-5);
        assert_eq!((p.focus, p.ambiance, p.texture), (2.0, 0.3, 7));
        // on the axis: the middle of the texture
        let mid = projected(&p, Vec3::new(10.0, 20.0, 25.0)).expect("in front");
        assert!((mid.truncate() - glam::Vec2::splat(0.5)).length() < 1e-5, "{mid:?}");
        // at 45 degrees from the axis: its edges
        let depth = p.origin.z - 25.0;
        let edge = projected(&p, Vec3::new(10.0 + depth, 20.0, 25.0)).expect("in front");
        assert!((edge.x - 1.0).abs() < 1e-4 && (edge.y - 0.5).abs() < 1e-4, "{edge:?}");
        let low = projected(&p, Vec3::new(10.0, 20.0 - depth, 25.0)).expect("in front");
        assert!((low.x - 0.5).abs() < 1e-4 && low.y.abs() < 1e-4, "{low:?}");
        // beyond the edge: outside 0..1; above the prim: behind the near plane
        let out = projected(&p, Vec3::new(10.0 + 2.0 * depth, 20.0, 25.0)).expect("in front");
        assert!(out.x > 1.0);
        assert!(projected(&p, Vec3::new(10.0, 20.0, 31.0)).is_none());
    }

    #[test]
    fn a_projector_follows_its_prim() {
        // turned to point east (+X), twice as wide as high
        let turn = glam::Quat::from_rotation_y(-std::f32::consts::FRAC_PI_2);
        let p = Projector::new(Vec3::ZERO, turn, Vec3::new(1.0, 0.5, 0.1), 10.0, Vec3::new(1.0, 0.0, 0.0), 0);
        assert!((p.direction - Vec3::X).length() < 1e-5, "{:?}", p.direction);
        let mid = projected(&p, Vec3::new(5.0, 0.0, 0.0)).expect("in front");
        assert!((mid.truncate() - glam::Vec2::splat(0.5)).length() < 1e-4);
        assert!(projected(&p, Vec3::new(-5.0, 0.0, 0.0)).is_none());
        // the frustum is as wide as the prim's face is (aspect 2): at the
        // same angle, the height leaves it and the width does not
        let d = 5.0 - p.origin.x;
        let half = d * (0.5f32).tan();
        let high = projected(&p, Vec3::new(5.0, half * 1.5, 0.0)).expect("in front");
        let wide = projected(&p, Vec3::new(5.0, 0.0, half * 1.5)).expect("in front");
        let outside = |t: Vec3| !(0.0..=1.0).contains(&t.x) || !(0.0..=1.0).contains(&t.y);
        assert!(outside(high) != outside(wide), "{high:?} {wide:?}");
        // a null field of view stays finite
        let thin = Projector::new(Vec3::ZERO, turn, Vec3::ONE, 10.0, Vec3::ZERO, 0);
        assert!(thin.matrix.is_finite() && thin.near.is_finite());
        // the shadow map: depth 0 just past the face, 1 at the reach, and
        // the same axis as the projected texture
        let depth = |at: Vec3| {
            let c = p.shadow_matrix * at.extend(1.0);
            c.truncate() / c.w
        };
        let face = p.origin.x + p.near;
        assert!(depth(Vec3::new(face + 0.01, 0.0, 0.0)).z.abs() < 1e-3);
        assert!((depth(Vec3::new(face + 10.0, 0.0, 0.0)).z - 1.0).abs() < 1e-3);
        let mid = depth(Vec3::new(5.0, 0.0, 0.0));
        assert!(mid.x.abs() < 1e-4 && mid.y.abs() < 1e-4 && mid.z > 0.0 && mid.z < 1.0, "{mid:?}");
    }

    fn cmd(record: u32) -> DrawCmd {
        DrawCmd {
            index_count: 6,
            first_index: record * 6,
            base_vertex: 0,
            record,
            bounds: [0.0; 4],
        }
    }

    /// The lists of a frame packet: a full copy of the scene's lists, which
    /// the main thread rebuilds for the next frame while the render thread
    /// still reads the copy.
    #[test]
    fn packet_lists_are_a_copy_that_keeps_its_memory() {
        let mut scene = DrawLists {
            hud_view: Some(HudView::new([1600, 900], 0.5, -1.0, 2.0)),
            ..Default::default()
        };
        scene.hud_opaque.push(cmd(20));
        scene.hud_blend.push(cmd(21));
        scene.hud_debug_red.push(cmd(23));
        scene.hud_debug_blue.push(cmd(24));
        scene.terrain.push(cmd(1));
        scene.terrain_mask.push(cmd(27));
        scene.terrain_pbr.push(cmd(28));
        scene.creation_preview.push(cmd(25));
        scene.physics_shapes.push(cmd(26));
        scene.blend.extend([cmd(7), cmd(8), cmd(9)]);
        scene.blend_under_water.push(cmd(29));
        scene.blend_glow.extend([false, true, false]);
        scene.blend_depth = 1;
        scene.glow.push(cmd(4));
        scene.select_root.push(cmd(5));
        scene.select_faces.push(cmd(6));
        scene.hud_select_faces.push(cmd(22));
        scene.particles.push(ParticleInstance::default());
        scene.impostor_sprites.push(ImpostorSprite {
            center: [1.0, 2.0, 3.0],
            tile: 3,
            right: [1.0, 0.0, 0.0],
            up: [0.0, 0.0, 1.0],
        });
        scene.impostor_captures.push(ImpostorCapture {
            tile: 3,
            eye: Vec3::ZERO,
            view: Mat4::IDENTITY,
            half: 1.0,
            depth: 4.0,
            opaque: vec![cmd(11)],
            blend: vec![cmd(12)],
        });
        scene.gpu = Some(crate::gpu_cull::GpuCullView {
            first_person: None,
            planes: [Vec4::ZERO; 6],
            eye: Vec3::ONE,
            draw_distance: 128.0,
            shadow_distance: 64.0,
            reflection_distance: 128.0,
            pixel_scale: 900.0,
            shadows: true,
            reflections: false,
        });
        // a packet buffer coming back from an earlier, larger frame
        let mut packet = DrawLists::default();
        packet.blend.extend((0..64).map(cmd));
        packet.opaque.push(cmd(99));
        let (blend_memory, blend_room) = (packet.blend.as_ptr(), packet.blend.capacity());
        packet.copy_from(&scene);
        assert_eq!(packet.total(), scene.total());
        assert_eq!(packet.hud_view.map(HudView::matrix), scene.hud_view.map(HudView::matrix));
        assert_eq!(packet.hud_opaque[0].record, 20);
        assert_eq!(packet.hud_blend[0].record, 21);
        assert_eq!(packet.hud_debug_red[0].record, 23);
        assert_eq!(packet.hud_debug_blue[0].record, 24);
        assert_eq!(packet.creation_preview[0].record, 25);
        assert_eq!(packet.physics_shapes[0].record, 26);
        assert_eq!(packet.blend.iter().map(|c| c.record).collect::<Vec<_>>(), [7, 8, 9]);
        assert_eq!(packet.blend_under_water.iter().map(|c| c.record).collect::<Vec<_>>(), [29]);
        assert_eq!(packet.blend_glow, [false, true, false]);
        assert_eq!(packet.blend_depth, 1);
        assert_eq!((packet.terrain.len(), packet.glow.len(), packet.select_root.len()), (1, 1, 1));
        assert_eq!(packet.terrain_mask[0].record, 27);
        assert_eq!(packet.terrain_pbr[0].record, 28);
        assert_eq!(packet.select_faces[0].record, 6);
        assert_eq!(packet.hud_select_faces[0].record, 22);
        assert_eq!((packet.particles.len(), packet.impostor_sprites.len()), (1, 1));
        assert_eq!(packet.impostor_captures[0].blend[0].record, 12);
        assert_eq!(packet.gpu.map(|g| g.draw_distance), Some(128.0));
        // what the earlier frame held is gone, its memory is reused
        assert!(packet.opaque.is_empty());
        assert_eq!((packet.blend.as_ptr(), packet.blend.capacity()), (blend_memory, blend_room));
        // the scene builds the next frame: the packet does not follow
        scene.clear();
        scene.blend.push(cmd(1));
        assert_eq!(packet.blend.len(), 3);
        assert!(scene.hud_view.is_none());
        assert!(scene.blend_under_water.is_empty());
        assert!(scene.hud_opaque.is_empty());
        assert!(scene.hud_debug_red.is_empty());
        assert!(scene.creation_preview.is_empty());
        assert!(scene.physics_shapes.is_empty());
        assert_eq!(packet.physics_shapes[0].record, 26);
        assert_eq!(packet.creation_preview[0].record, 25);
        assert!(scene.hud_debug_blue.is_empty());
        assert!(scene.select_faces.is_empty());
        assert!(scene.hud_select_faces.is_empty());
        assert!(packet.hud_view.is_some());
        assert_eq!(packet.hud_opaque[0].record, 20);
        assert!(packet.gpu.is_some());
        packet.copy_from(&scene);
        assert!(
            packet.physics_shapes.is_empty(),
            "Disabling physics display clears reused frame packets"
        );
        assert!(
            packet.creation_preview.is_empty(),
            "A reused packet must not retain a hidden creation hologram"
        );
    }

    /// `DrawRecord` must match the WGSL struct (std430: 4 × 4 floats for the
    /// matrix, then one 16-byte vector per field).
    #[test]
    fn draw_record_layout_matches_wgsl() {
        assert_eq!(std::mem::size_of::<DrawRecord>(), 64 + 14 * 16);
        assert_eq!(std::mem::offset_of!(DrawRecord, anim), 64 + 10 * 16);
        assert_eq!(std::mem::offset_of!(DrawRecord, anim_xf), 64 + 11 * 16);
        assert_eq!(std::mem::offset_of!(DrawRecord, rez), 64 + 12 * 16);
        assert_eq!(std::mem::offset_of!(DrawRecord, portal_clip), 64 + 13 * 16);
        let wgsl = include_str!("shaders/common.wgsl");
        let start = wgsl.find("struct DrawRecord {").expect("DrawRecord in common.wgsl");
        let body = &wgsl[start..start + wgsl[start..].find("};").expect("end of DrawRecord")];
        let fields: Vec<&str> = body
            .lines()
            .skip(1)
            .filter_map(|l| l.trim().split(':').next())
            .filter(|n| !n.is_empty())
            .collect();
        assert_eq!(
            fields,
            [
                "model",
                "base_color",
                "emissive",
                "uv_st",
                "params",
                "tex",
                "flags",
                "mat_uv",
                "spec_uv",
                "legacy",
                "spec_color",
                "anim",
                "anim_xf",
                "rez",
                "portal_clip"
            ]
        );
    }

    #[test]
    fn portal_clipping_keeps_the_default_record_disabled_and_survives_frame_handoff() {
        let record = DrawRecord::default();
        assert_eq!(record.portal_clip, [0.0; 4]);
        assert_eq!(record.flags[0] & flags::PORTAL_CLIP, 0);
        assert!(include_str!("shaders/common.wgsl").contains(&format!("const FLAG_PORTAL_CLIP: u32 = {}u;", flags::PORTAL_CLIP)));
        let source = DrawLists {
            portal_clipping: true,
            ..Default::default()
        };
        let mut destination = DrawLists::default();
        destination.copy_from(&source);
        assert!(destination.portal_clipping);
        destination.clear();
        assert!(!destination.portal_clipping);
    }

    #[test]
    fn portal_aperture_keeps_geometry_outside_the_oval_for_both_sides_and_rotated_centers() {
        let wgsl = include_str!("shaders/common.wgsl");
        assert!(wgsl.contains(&format!("const PORTAL_HALF_WIDTH: f32 = {PORTAL_HALF_WIDTH:.1};")));
        assert!(wgsl.contains(&format!("const PORTAL_HALF_HEIGHT: f32 = {PORTAL_HALF_HEIGHT:.1};")));
        assert!(wgsl.contains(&format!("const PORTAL_SURFACE_SCALE: f32 = {PORTAL_SURFACE_SCALE:.2};")));
        assert!(wgsl.contains(&format!("const FLAG_PORTAL_EFFECT: u32 = {}u;", flags::PORTAL_EFFECT)));
        for center in [Vec3::ZERO, Vec3::new(214.0, 185.0, 60.0)] {
            for step in 0..32 {
                let angle = step as f32 * std::f32::consts::TAU / 32.0;
                let normal = Vec3::new(angle.cos(), angle.sin(), 0.0);
                let across = Vec3::new(-normal.y, normal.x, 0.0);
                for side in [-1.0, 1.0] {
                    let clip = PortalClip::new(center, normal * side).unwrap();
                    assert!(clip.normal().dot(normal * side) > 0.999999);
                    let front = center + normal * side * 0.2;
                    let back = center - normal * side * 0.2;
                    assert!(clip.keeps_point(front));
                    assert!(!clip.keeps_point(back));
                    assert!(!clip.keeps_point(back + across * (PORTAL_HALF_WIDTH * 0.99)));
                    assert!(clip.keeps_point(back + across * (PORTAL_HALF_WIDTH * 1.01)));
                    assert!(clip.keeps_point(back + Vec3::Z * (PORTAL_HALF_HEIGHT * 1.01)));
                    assert!(!clip.keeps_point(back + across * 0.5 + Vec3::Z * 0.5));
                    // Inside the bounding rectangle, outside the ellipse.
                    assert!(clip.keeps_point(back + across * 0.8 + Vec3::Z * (PORTAL_HALF_HEIGHT * 0.8)));
                }
            }
        }
        assert_eq!(PortalClip::new(Vec3::NAN, Vec3::X), None);
        assert_eq!(PortalClip::new(Vec3::ZERO, Vec3::ZERO), None);
        assert_eq!(PortalClip::new(Vec3::ZERO, Vec3::Z), None);
        assert_eq!(PortalClip::new(Vec3::ZERO, Vec3::X).unwrap().data, [0.0; 4]);
    }

    #[test]
    fn anchored_flag_matches_wgsl_and_spares_skinned_records() {
        let wgsl = include_str!("shaders/common.wgsl");
        assert!(wgsl.contains(&format!("const FLAG_ANCHORED: u32 = {}u;", flags::ANCHORED)));
        assert!(wgsl.contains(&format!("const FLAG_SKINNED: u32 = {}u;", flags::SKINNED)));
        assert!(wgsl.contains(&format!("const FLAG_LIP_SYNC: u32 = {}u;", flags::LIP_SYNC)));
        let mut record = DrawRecord::default();
        assert_eq!(record.anchor(), None);
        record.set_anchor(Some(371));
        assert_eq!(record.anchor(), Some(371));
        record.set_hud_zoom(2.0);
        assert_eq!(record.anchor(), Some(371));
        record.set_anchor(None);
        assert_eq!((record.anchor(), record.flags[1]), (None, 0));
        // a skinned record keeps its palette base: its anchor is its
        // skeleton's
        assert!(wgsl.contains(&format!("const SKELETON_ANCHOR: u32 = {SKELETON_ANCHOR}u;")));
        assert!(wgsl.contains(&format!("const LIVE_MORPH_WEIGHTS: u32 = {LIVE_MORPH_WEIGHTS}u;")));
        let mut skinned = DrawRecord {
            flags: [flags::SKINNED, 208, 7, 0],
            ..Default::default()
        };
        assert_eq!(skinned.anchor(), None);
        skinned.set_anchor(Some(208 + SKELETON_ANCHOR));
        assert_eq!(skinned.flags, [flags::SKINNED | flags::ANCHORED, 208, 7, 0]);
        assert_eq!(skinned.anchor(), Some(208 + SKELETON_ANCHOR));
        skinned.set_anchor(None);
        assert_eq!(skinned.flags, [flags::SKINNED, 208, 7, 0]);
    }

    #[test]
    fn hud_zoom_keeps_material_skin_and_animation_flags() {
        let mut record = DrawRecord {
            flags: [0x0fff, 42, 128, 0x123456],
            ..Default::default()
        };
        for zoom in [0.1, 0.5, 1.61051, 8.0] {
            record.set_hud_zoom(zoom);
            assert_eq!(record.flags[0] & 0xffff, 0x0fff);
            assert_eq!(&record.flags[1..], &[42, 128, 0x123456]);
            let decoded = (record.flags[0] >> 16) as f32 / 4096.0;
            assert!((decoded - zoom).abs() <= 1.0 / 8192.0);
        }
        record.set_hud_zoom(1.0);
        assert_eq!(record.flags[0], 0x0fff);
    }
}
