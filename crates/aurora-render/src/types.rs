//! Plain data shared between the world and the renderer.

use crate::HudView;
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3, Vec4};

/// 24-byte vertex used by every mesh (prims, meshes, avatars, terrain).
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable, Default)]
pub struct Vertex {
    pub pos: [f32; 3],
    /// snorm8 normal; `w` carries extra data (terrain composition).
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

/// Per-vertex skinning data (parallel stream): 4 joint indices and 4
/// normalized weights.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable, Default)]
pub struct SkinVertex {
    /// Mesh joint indices into the record's skin bindings (`flags[2]`).
    pub joints: [u8; 4],
    pub weights: [u8; 4],
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
}

/// Layout of a skeleton's palette in the palettes buffer: its joints, then
/// its anchors. The first anchor places the skeleton itself (skeleton space
/// to world); the scene gives the others to the attachment points.
pub const SKELETON_ANCHOR: u32 = 160;

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

/// Visible draws for a frame, built by the world.
#[derive(Default)]
pub struct DrawLists {
    /// Own HUD attachments, drawn after post-processing with their own depth.
    pub hud_view: Option<HudView>,
    pub hud_opaque: Vec<DrawCmd>,
    pub hud_blend: Vec<DrawCmd>,
    pub terrain: Vec<DrawCmd>,
    pub opaque: Vec<DrawCmd>,
    pub opaque_two_sided: Vec<DrawCmd>,
    pub mask: Vec<DrawCmd>,
    pub mask_two_sided: Vec<DrawCmd>,
    pub water: Vec<DrawCmd>,
    /// Sorted back to front.
    pub blend: Vec<DrawCmd>,
    /// Per `blend` entry: the face has glow (drawn into the glow right
    /// after it, as LLDrawPoolAlpha's emissives).
    pub blend_glow: Vec<bool>,
    pub shadow_casters: Vec<ShadowCaster>,
    /// Opaque and alpha-masked draws for the planar reflection passes
    /// (water and mirror); drawn without face culling, alpha tested.
    /// With their bounds: each reflection pass culls them to its own frustum.
    pub reflection: Vec<ShadowCaster>,
    /// `reflection` without avatars, their attachments and animesh: what a
    /// static reflection probe captures (LLViewerWindow::cubeSnapshot).
    pub probe: Vec<ShadowCaster>,
    pub reflection_terrain: Vec<DrawCmd>,
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
    /// Debug « Afficher la transparence » (LLDrawPoolAlpha::renderDebugAlpha):
    /// blended, alpha-masked and invisible faces in translucent red...
    pub debug_red: Vec<DrawCmd>,
    /// ... and faces masked by their material in blue.
    pub debug_blue: Vec<DrawCmd>,
    /// Build tools: wireframe of the selected mesh / sculpted objects
    /// (LLSelectMgr::renderOneSilhouette draws meshes as wireframes), root
    /// in yellow, other prims of the linkset in blue.
    pub select_root: Vec<DrawCmd>,
    pub select_child: Vec<DrawCmd>,
    /// Selected texture faces (LLPipeline::renderSelectedFaces).
    pub select_faces: Vec<DrawCmd>,
    pub hud_select_faces: Vec<DrawCmd>,
    pub select_edges: Vec<DrawCmd>,
    pub hud_select_edges: Vec<DrawCmd>,
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
    /// Make `self` a copy of `src`, keeping the memory `self` already has:
    /// the lists of a frame packet, filled again every frame.
    pub fn copy_from(&mut self, src: &DrawLists) {
        self.hud_view = src.hud_view;
        self.hud_opaque.clone_from(&src.hud_opaque);
        self.hud_blend.clone_from(&src.hud_blend);
        self.terrain.clone_from(&src.terrain);
        self.opaque.clone_from(&src.opaque);
        self.opaque_two_sided.clone_from(&src.opaque_two_sided);
        self.mask.clone_from(&src.mask);
        self.mask_two_sided.clone_from(&src.mask_two_sided);
        self.water.clone_from(&src.water);
        self.blend.clone_from(&src.blend);
        self.blend_glow.clone_from(&src.blend_glow);
        self.shadow_casters.clone_from(&src.shadow_casters);
        self.reflection.clone_from(&src.reflection);
        self.probe.clone_from(&src.probe);
        self.reflection_terrain.clone_from(&src.reflection_terrain);
        self.particles.clone_from(&src.particles);
        self.glow.clone_from(&src.glow);
        self.glow_alpha.clone_from(&src.glow_alpha);
        self.debug_red.clone_from(&src.debug_red);
        self.debug_blue.clone_from(&src.debug_blue);
        self.select_root.clone_from(&src.select_root);
        self.select_child.clone_from(&src.select_child);
        self.select_faces.clone_from(&src.select_faces);
        self.hud_select_faces.clone_from(&src.hud_select_faces);
        self.select_edges.clone_from(&src.select_edges);
        self.hud_select_edges.clone_from(&src.hud_select_edges);
        self.impostor_captures.clone_from(&src.impostor_captures);
        self.impostor_sprites.clone_from(&src.impostor_sprites);
        self.gpu = src.gpu;
    }

    pub fn clear(&mut self) {
        self.hud_view = None;
        self.hud_opaque.clear();
        self.hud_blend.clear();
        self.terrain.clear();
        self.opaque.clear();
        self.opaque_two_sided.clear();
        self.mask.clear();
        self.mask_two_sided.clear();
        self.water.clear();
        self.blend.clear();
        self.shadow_casters.clear();
        self.reflection.clear();
        self.probe.clear();
        self.reflection_terrain.clear();
        self.particles.clear();
        self.glow.clear();
        self.blend_glow.clear();
        self.glow_alpha.clear();
        self.debug_red.clear();
        self.debug_blue.clear();
        self.select_root.clear();
        self.select_child.clear();
        self.select_faces.clear();
        self.hud_select_faces.clear();
        self.select_edges.clear();
        self.hud_select_edges.clear();
        self.impostor_captures.clear();
        self.impostor_sprites.clear();
        self.gpu = None;
    }

    pub fn total(&self) -> usize {
        self.hud_opaque.len()
            + self.hud_blend.len()
            + self.terrain.len()
            + self.opaque.len()
            + self.opaque_two_sided.len()
            + self.mask.len()
            + self.mask_two_sided.len()
            + self.water.len()
            + self.blend.len()
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tonemapper {
    /// Khronos PBR Neutral (Second Life's tone mapper).
    KhronosNeutral,
    Aces,
}

/// Quality settings applied with `Renderer::apply_settings`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderSettings {
    pub aa: AntiAliasing,
    pub tonemapper: Tonemapper,
    /// Shadow map resolution per cascade (0 = shadows off).
    pub shadow_resolution: u32,
    /// Screen-space ambient occlusion: 0 off, 1 low, 2 high.
    pub ssao: u8,
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
}

impl Default for RenderSettings {
    fn default() -> Self {
        RenderSettings {
            aa: AntiAliasing::Msaa(4),
            tonemapper: Tonemapper::KhronosNeutral,
            shadow_resolution: 2048,
            ssao: 1,
            water_reflection_scale: 0.25,
            mirror_scale: 0.25,
            ssr: false,
            anisotropy: 8,
            probe_slots: 32,
            occlusion: true,
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
            sun_dir: sun,
            moon_dir: -sun,
            sun_scale: 1.0,
            moon_scale: 1.0,
            moon_brightness: 0.5,
            star_brightness: 250.0,
            cloud_texture: 0,
            sun_texture: 0,
            moon_texture: 0,
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
#[derive(Debug, Clone)]
pub struct FrameParams {
    /// Linear RGBA from the UI palette for selected-face overlays.
    pub selection_color: [f32; 4],
    pub view: Mat4,
    pub proj: Mat4,
    pub camera_pos: Vec3,
    pub near: f32,
    pub far: f32,
    pub fov_y: f32,
    pub time: f32,
    pub sun_dir: Vec3,
    pub sun_visible: f32,
    pub sun_color: Vec3,
    pub sky_zenith: Vec3,
    pub sky_horizon: Vec3,
    pub ground_color: Vec3,
    pub fog_color: Vec3,
    pub fog_density: f32,
    pub exposure: f32,
    /// Contrast adaptive sharpening strength (0 = off; Firestorm RenderCASSharpness).
    pub sharpen: f32,
    /// Screen-space glow of faces with a glow value (RenderGlow).
    pub glow: bool,
    pub water_height: f32,
    pub shadows: bool,
    /// Distance covered by the shadow cascades.
    pub shadow_distance: f32,
    pub lights: Vec<PointLight>,
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
        scene.terrain.push(cmd(1));
        scene.blend.extend([cmd(7), cmd(8), cmd(9)]);
        scene.blend_glow.extend([false, true, false]);
        scene.glow.push(cmd(4));
        scene.select_root.push(cmd(5));
        scene.select_faces.push(cmd(6));
        scene.hud_select_faces.push(cmd(22));
        scene.select_edges.push(cmd(23));
        scene.hud_select_edges.push(cmd(24));
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
        assert_eq!(packet.blend.iter().map(|c| c.record).collect::<Vec<_>>(), [7, 8, 9]);
        assert_eq!(packet.blend_glow, [false, true, false]);
        assert_eq!((packet.terrain.len(), packet.glow.len(), packet.select_root.len()), (1, 1, 1));
        assert_eq!(packet.select_faces[0].record, 6);
        assert_eq!(packet.hud_select_faces[0].record, 22);
        assert_eq!(packet.select_edges[0].record, 23);
        assert_eq!(packet.hud_select_edges[0].record, 24);
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
        assert!(scene.hud_opaque.is_empty());
        assert!(scene.select_faces.is_empty());
        assert!(scene.hud_select_faces.is_empty());
        assert!(scene.select_edges.is_empty());
        assert!(scene.hud_select_edges.is_empty());
        assert!(packet.hud_view.is_some());
        assert_eq!(packet.hud_opaque[0].record, 20);
        assert!(packet.gpu.is_some());
    }

    /// `DrawRecord` must match the WGSL struct (std430: 4 × 4 floats for the
    /// matrix, then one 16-byte vector per field).
    #[test]
    fn draw_record_layout_matches_wgsl() {
        assert_eq!(std::mem::size_of::<DrawRecord>(), 64 + 12 * 16);
        assert_eq!(std::mem::offset_of!(DrawRecord, anim), 64 + 10 * 16);
        assert_eq!(std::mem::offset_of!(DrawRecord, anim_xf), 64 + 11 * 16);
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
                "anim_xf"
            ]
        );
    }

    #[test]
    fn anchored_flag_matches_wgsl_and_spares_skinned_records() {
        let wgsl = include_str!("shaders/common.wgsl");
        assert!(wgsl.contains(&format!("const FLAG_ANCHORED: u32 = {}u;", flags::ANCHORED)));
        assert!(wgsl.contains(&format!("const FLAG_SKINNED: u32 = {}u;", flags::SKINNED)));
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
