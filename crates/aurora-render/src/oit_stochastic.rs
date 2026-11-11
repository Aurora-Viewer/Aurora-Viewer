//! Order-independent transparency by depth-based stochastic transparency
//! (the OIT setting; `AURORA_OIT` in docs/TESTING.md). An Aurora addition:
//! the Legacy Viewer sorts its alpha (LLDrawPoolAlpha).
//!
//! Enderton, Sintorn, Shirley and Luebke, "Stochastic Transparency", I3D
//! 2010, section 3.4, the final anti-aliased depth-based algorithm:
//!
//! 1. the opaque scene in its multisampled depth (the renderer's);
//! 2. the total alpha of every sample, `1 - prod(1 - alpha)`, into a
//!    multisampled target (`fs_st_alpha`, one pass, order independent);
//! 3. the blended faces into a copy of the opaque depth, each fragment
//!    covering a random subset of its samples of size `alpha * S`, depth
//!    only (`fs_st_depth_mask`, or `fs_st_depth_index`);
//! 4. the accumulation: `alpha * colour` (and alpha) added into the samples
//!    where the fragment is at or in front of the stochastic depth
//!    (`fs_blend`, additive, GreaterEqual);
//! 5. the composite: every scene sample dimmed by its transparency, plus
//!    the accumulated colour averaged over the pixel's samples, times the
//!    alpha correction (total alpha over accumulated alpha).
//!
//! The glow of the blended faces that have some is accumulated like their
//! colour (`fs_st_glow`, after the accumulation), corrected the same way and
//! added to the scene alpha after the composite: on average each face's
//! glow dimmed by the layers in front of it, as the Legacy Viewer's glow
//! drawn after each face in order and suppressed by the ones after it.
//!
//! The passes 3 and 4 run at their own sample count, the most the device
//! has up to 8 (the paper's, section 4), whatever the scene's: their depth
//! starts as the opaque depth spread over those samples
//! (`oit_stochastic_expand.wgsl`). With MSAA the total alpha and the
//! composite stay at the scene's samples (each scene sample dimmed by its
//! own transparency). Without (SMAA, FXAA, TAA, FSR or none: one sample) the
//! total alpha runs at the passes' samples against that spread depth and the
//! composite dims each pixel by its resolved transparency; the scene's
//! anti-aliasing then runs over the result as over the sorted order. Under
//! FSR that is the render size, before the upscale. The 8 samples are
//! averaged over the frames by a temporal blend (weight 1/8, the seeds
//! changing every frame), off under TAA (`temporal_weight`).
//!
//! Everything here is made the first frame the method is on, on the render
//! thread, and nothing while it is off: the passes' shaders are a module of
//! their own (the `ST_BEGIN` to `ST_END` part of object.wgsl is left out of
//! the scene module, `without_st`), their pipelines, buffers and targets
//! come with the first use, and the targets go when the method is off or
//! unused for a while. The masks come from a table of random S-choose-R
//! subsets (`mask_table`), fixed for every launch; the seed is the pixel,
//! the face's tie-break and the triangle. Needs multisampled targets of at
//! least 2 samples (every device has 4), and `PRIMITIVE_INDEX` for the
//! triangle in the seed (without it the seed is per face: coplanar
//! fragments of one face then occlude instead of blending).

use crate::pipeline_cache::CachedPipelines;
use bytemuck::{Pod, Zeroable};

/// Seeds per alpha level in the mask table (the paper's width, 2 047).
pub const SEEDS: u32 = 2048;
/// The most samples a stochastic pass takes (the expand map's size).
pub const MAX_PASS_SAMPLES: u32 = 8;
/// The temporal blend's weight of the current frame.
pub const TEMPORAL: f32 = 1.0 / 8.0;
/// Frames without a blended face after which the targets are freed.
const IDLE_FRAMES: u32 = 120;

/// How the coverage mask reaches the samples of the stochastic depth.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Route {
    /// one invocation per sample (`@builtin(sample_index)`), the samples
    /// off the mask discarded: the fallback
    SampleIndex,
    /// the mask written once per fragment (`@builtin(sample_mask)`): the
    /// default, a quarter of the invocations at 4x. On Vulkan only through
    /// spirv_fix (naga 30's SPIR-V declares the built-in wrong); when that
    /// cannot make the module, the index route is taken
    SampleMask,
}

/// The scene's own accumulation over the frames, after the composite
/// (`temporal_weight`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum SceneHistory {
    /// SMAA, FXAA, MSAA or none: the blend averages the noise
    None,
    /// the scene's TAA
    Taa,
    /// FSR 3.1 (fsr.rs), in TAA's place: its own history and locks
    Fsr,
}

/// The temporal blend's weight under `history`. 0 under TAA, whose history
/// would hold the blend's trail a second time (a double ghost); the seeds
/// then stay the same from frame to frame, and TAA keeps that grain steady.
/// `wanted` otherwise, FSR included: its reactive mask (the scene copy
/// against the composited colour) marks the blended faces, where FSR takes
/// this frame's colour rather than its history, so a static grain would
/// stay, enlarged by the upscale and sharpened by RCAS, while the blend
/// averages it as without a scene history.
pub(crate) fn temporal_weight(wanted: f32, history: SceneHistory) -> f32 {
    match history {
        SceneHistory::None | SceneHistory::Fsr => wanted,
        SceneHistory::Taa => 0.0,
    }
}

/// How the blended faces are drawn: the OIT setting on means stochastic
/// transparency, off the sorted order (per face, as `scene/blend_order.rs`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Method {
    /// the sorted order
    Off,
    Stochastic,
}

/// `AURORA_OIT` read once.
fn env_value() -> Option<&'static str> {
    static VALUE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| std::env::var("AURORA_OIT").ok()).as_deref()
}

/// The method for an `AURORA_OIT` value (`0` off, `1` or `stochastic` on,
/// whatever the setting; anything else, or none, leaves it to the setting)
/// and the OIT setting.
pub(crate) fn method_with(env: Option<&str>, setting_oit: bool) -> Method {
    match env.map(str::trim) {
        Some("0") => Method::Off,
        Some("1" | "stochastic") => Method::Stochastic,
        _ => {
            if setting_oit {
                Method::Stochastic
            } else {
                Method::Off
            }
        }
    }
}

/// The method this frame, from the environment or the OIT setting.
pub(crate) fn method(setting_oit: bool) -> Method {
    method_with(env_value(), setting_oit)
}

/// The stochastic method can be the frame's (not ruled out by the
/// environment): the GPU timer logs its passes then.
pub(crate) fn wanted() -> bool {
    method_with(env_value(), true) == Method::Stochastic
}

/// Whether a frame tries to draw its blended faces with the method: it is
/// on, the frame has some, and it is the plain main view (not a photo, one
/// of its tiles or a snapshot, whose size and history the method would mix
/// up). The targets may still be refused (`Stochastic::prepare`).
pub(crate) fn frame_wants_method(method: Method, blended: u32, plain_view: bool) -> bool {
    method == Method::Stochastic && blended > 0 && plain_view
}

/// The scene module's source: `src` (object.wgsl) without the passes of
/// the method, for the scene module.
pub(crate) fn without_st(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut skipping = false;
    for line in src.split_inclusive('\n') {
        let t = line.trim();
        if t == "// ST_BEGIN" {
            skipping = true;
        } else if t == "// ST_END" {
            skipping = false;
        } else if !skipping {
            out.push_str(line);
        }
    }
    out
}

/// object.wgsl with the triangle index in the fragment seeds when the
/// device has `PRIMITIVE_INDEX` (the `ST_PRIM` markers), for the method's
/// own module.
pub(crate) fn st_object_source(device: &wgpu::Device, src: &str) -> String {
    st_object_source_with(src, device.features().contains(wgpu::Features::PRIMITIVE_INDEX))
}

pub(crate) fn st_object_source_with(src: &str, prim: bool) -> String {
    if prim {
        src.replace("/*ST_PRIM_PARAM*/", "@builtin(primitive_index) prim_id: u32,")
            .replace("/*ST_PRIM*/0u", "prim_id")
    } else {
        src.replace("/*ST_PRIM_PARAM*/", "")
    }
}

/// The directive the method's module starts with when the triangle index
/// is read (`enable primitive_index;`), empty otherwise.
pub(crate) fn enable_directive(device: &wgpu::Device) -> &'static str {
    enable_directive_with(device.features().contains(wgpu::Features::PRIMITIVE_INDEX))
}

pub(crate) fn enable_directive_with(prim: bool) -> &'static str {
    if prim {
        "enable primitive_index;
"
    } else {
        ""
    }
}

/// The scene module's source with the method's passes in (the common
/// part, object.wgsl, env.wgsl): the module the method's pipelines use.
pub(crate) fn st_scene_source(device: &wgpu::Device) -> String {
    format!(
        "{}{}\n{}\n{}",
        enable_directive(device),
        crate::common_shader_source(),
        st_object_source(
            device,
            &crate::water_shader_source(&["object.wgsl"], include_str!("shaders/object.wgsl"))
        ),
        crate::water_shader_source(&["env.wgsl"], include_str!("shaders/env.wgsl"))
    )
}

/// The table of masks for `samples` bits: `[r * SEEDS + seed]` a random
/// r-subset of the bits, from a fixed generator (the same every launch).
pub fn mask_table(samples: u32) -> Vec<u32> {
    let s = samples.clamp(1, MAX_PASS_SAMPLES);
    let mut state: u64 = 0x9e37_79b9_7f4a_7c15 ^ u64::from(s);
    let mut next = move || {
        // splitmix64
        state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    };
    let mut table = Vec::with_capacity(((s + 1) * SEEDS) as usize);
    for r in 0..=s {
        for _ in 0..SEEDS {
            // a partial Fisher-Yates shuffle: the first r of a permutation
            let mut bits: Vec<u32> = (0..s).collect();
            let mut mask = 0u32;
            for i in 0..r as usize {
                let j = i + (next() % (s as u64 - i as u64)) as usize;
                bits.swap(i, j);
                mask |= 1 << bits[i];
            }
            table.push(mask);
        }
    }
    table
}

/// The samples per stochastic pass for a scene of `scene` samples: the most
/// the device has up to 8, never fewer than the scene's (whose targets the
/// passes then share a count with). `supported`: the counts the colour, the
/// depth and the transmittance formats all take. 1 (the method declines)
/// when the scene's own count is not among them or none fits.
pub fn pass_samples(scene: u32, supported: &[u32]) -> u32 {
    if scene > 1 && !supported.contains(&scene) {
        return 1;
    }
    supported
        .iter()
        .copied()
        .filter(|&n| n >= scene && n <= MAX_PASS_SAMPLES)
        .max()
        .unwrap_or(1)
}

/// The standard sample positions of Vulkan and Direct3D (in sixteenths of
/// a pixel from its centre), for the counts a pass or a scene can have.
fn standard_positions(n: u32) -> &'static [(i32, i32)] {
    match n {
        2 => &[(4, 4), (-4, -4)],
        4 => &[(-2, -6), (6, -2), (-6, 2), (2, 6)],
        8 => &[(1, -3), (-1, 3), (5, 1), (-3, -5), (-5, 5), (-7, -1), (3, 7), (7, -7)],
        _ => &[(0, 0)],
    }
}

/// For each sample of a pass of `pass` samples, the scene sample nearest to
/// it (oit_stochastic_expand.wgsl): the opaque depth it starts from.
pub fn expand_map(scene: u32, pass: u32) -> [u32; MAX_PASS_SAMPLES as usize] {
    let (from, to) = (standard_positions(scene), standard_positions(pass));
    let mut map = [0u32; MAX_PASS_SAMPLES as usize];
    for (m, &(x, y)) in map.iter_mut().zip(to) {
        let d = |&(a, b): &(i32, i32)| (a - x) * (a - x) + (b - y) * (b - y);
        *m = (0..from.len()).min_by_key(|&k| d(&from[k])).unwrap_or(0) as u32;
    }
    map
}

/// The samples of the total alpha's target: the scene's with MSAA (the
/// composite dims each scene sample by its own), the passes' without (the
/// composite dims the pixel by the resolved transparency).
pub fn trans_samples(scene: u32, pass: u32) -> u32 {
    if scene > 1 { scene } else { pass }
}

/// The bytes of the targets `make_targets` allocates, from their formats
/// (Rgba16Float colour, Depth32Float depth): the total alpha, the
/// accumulation, the glow and the stochastic depth multisampled, the
/// resolved colour (two with the history) and the total alpha and glow.
pub fn target_bytes(width: u32, height: u32, samples: u32, pass: u32, history: bool) -> u64 {
    let px = u64::from(width.max(1)) * u64::from(height.max(1));
    let trans = 2 * u64::from(trans_samples(samples, pass));
    let per_pass = (8 + 2 + 4) * u64::from(pass);
    let resolved = 8 * (1 + u64::from(history));
    px * (trans + per_pass + resolved + 8)
}

/// object.wgsl `StParams`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct StParams {
    samples: u32,
    /// bit 0: the seeds change every frame
    flags: u32,
    frame_seed: u32,
    _pad: u32,
}

/// oit_stochastic.wgsl `ResolveParams`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ResolveParams {
    width: u32,
    height: u32,
    /// samples per stochastic pass
    samples_hw: u32,
    /// bit 0 temporal blend, bit 1 glowing faces, bit 2 the scene
    /// single-sampled
    flags: u32,
    temporal_weight: f32,
    /// samples of the scene (the total alpha's target)
    trans_samples: u32,
    _pad: [u32; 2],
    /// this frame's clip space to the last frame's (frame.motion_reproj)
    reproj: [[f32; 4]; 4],
}

/// The scene module's pipelines of the passes over the faces. The total
/// alpha and the stochastic depth test the non-rigged faces against the
/// opaque depth with GreaterEqual like the sorted order does (a decal at
/// the depth of the surface under it stays), the rigged ones with Greater.
pub(crate) struct ScenePipes {
    /// total alpha, by rigged (0) or not (1), then by sidedness
    pub alpha: [[wgpu::RenderPipeline; 2]; 2],
    /// stochastic depth, by rigged or not, then by sidedness
    pub depth: [[wgpu::RenderPipeline; 2]; 2],
    /// accumulation, by sidedness
    pub accum: [wgpu::RenderPipeline; 2],
    /// the glowing faces' glow accumulated (no culling: fs_glow's own test
    /// of the backs, as the Legacy glow pass)
    pub glow: wgpu::RenderPipeline,
}

/// Everything a scene sample count needs: the passes' pipelines and the
/// resolve's.
pub(crate) struct Pipes {
    /// the scene's samples these were made for
    pub samples: u32,
    pub scene: ScenePipes,
    pub resolve: Resolve,
}

/// What the renderer's pipelines are made with.
#[derive(Clone)]
pub(crate) struct Shared {
    /// group 3 of the alpha and depth passes: the masks and the parameters
    pub face_layout: wgpu::BindGroupLayout,
    resolve_layout: wgpu::BindGroupLayout,
    module: wgpu::ShaderModule,
    hdr_format: wgpu::TextureFormat,
    depth_format: wgpu::TextureFormat,
    /// the sample mask route: naga's SampleMask fixed on Vulkan
    /// (spirv_fix.rs), with the scene's group layouts (frame, records,
    /// textures) the shader is written against
    mask_shaders: std::sync::Arc<(crate::spirv_fix::SampleMaskShaders, Vec<Vec<wgpu::BindGroupLayoutEntry>>)>,
    /// the sample counts the device takes for the stochastic passes'
    /// targets (the colour, the depth and the transmittance formats)
    pass_supported: Vec<u32>,
    /// the opaque depth spread over a pass's samples, from a multisampled
    /// scene depth and from a single-sampled one
    expand_layout: wgpu::BindGroupLayout,
    expand_layout_ss: wgpu::BindGroupLayout,
    expand_module: wgpu::ShaderModule,
}

/// The resolve's pipelines for one sample count of the scene.
pub(crate) struct Resolve {
    resolve: wgpu::ComputePipeline,
    composite: wgpu::RenderPipeline,
    /// the blended faces' glow added to the scene alpha
    composite_glow: wgpu::RenderPipeline,
    /// the opaque depth spread over the pass's samples (more than the
    /// scene's only)
    expand: Option<wgpu::RenderPipeline>,
}

struct Targets {
    width: u32,
    height: u32,
    /// the scene's samples, the stochastic passes'
    samples: u32,
    pass: u32,
    /// the last frame's resolved colour is kept (the temporal blend)
    history: bool,
    /// the total alpha's samples (`trans_samples`)
    trans_samples: u32,
    trans: wgpu::TextureView,
    accum: wgpu::TextureView,
    /// the glow accumulated (red), at the passes' samples
    glow: wgpu::TextureView,
    depth_tex: wgpu::Texture,
    depth: wgpu::TextureView,
    /// per parity of the frame: the resolve's and the composite's groups
    groups: [(wgpu::BindGroup, wgpu::BindGroup); 2],
}

/// What a frame tells `prepare`.
pub(crate) struct Frame {
    pub width: u32,
    pub height: u32,
    /// the scene's samples (1 without MSAA)
    pub samples: u32,
    /// this frame's clip space to the last frame's
    pub reproj: [[f32; 4]; 4],
    /// glowing blended faces will be accumulated
    pub glow: bool,
    pub history: SceneHistory,
    /// the camera jumped (a teleport, a cut): the history is not this view's
    pub cut: bool,
}

pub(crate) struct Stochastic {
    shared: Shared,
    /// the temporal weight the scene's history leaves (0 under TAA)
    temporal: f32,
    /// the scene history the last `prepare` saw (its log line on a change)
    scene_history: SceneHistory,
    masks: wgpu::Buffer,
    /// the passes' samples the mask table in `masks` was written for
    mask_pass: u32,
    params: wgpu::Buffer,
    face_group: wgpu::BindGroup,
    resolve_params: wgpu::Buffer,
    /// oit_stochastic_expand.wgsl `ExpandParams`
    expand_params: wgpu::Buffer,
    /// 1 x 1 stand-ins for the bindings a step does not use (a texture
    /// written as storage and read as sampled in one group conflicts):
    /// rgba16float read and written, rg32float read and written
    dummy_rgba_r: wgpu::TextureView,
    dummy_rgba_w: wgpu::TextureView,
    dummy_rg_r: wgpu::TextureView,
    dummy_rg_w: wgpu::TextureView,
    targets: Option<Targets>,
    /// the targets of this key were refused by the memory: not tried again
    /// until the size or the samples change
    refused: Option<(u32, u32, u32, u32, bool)>,
    frame: u32,
    /// the targets hold a resolved colour of an earlier frame of this view
    history_ready: bool,
    /// frames in a row the method was on without a blended face
    idle: u32,
    /// glowing faces are accumulated this frame
    glow: bool,
}

fn entry(binding: u32, visibility: wgpu::ShaderStages, ty: wgpu::BindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty,
        count: None,
    }
}

fn buffer(ty: wgpu::BufferBindingType) -> wgpu::BindingType {
    wgpu::BindingType::Buffer {
        ty,
        has_dynamic_offset: false,
        min_binding_size: None,
    }
}

fn texture(ms: bool) -> wgpu::BindingType {
    wgpu::BindingType::Texture {
        sample_type: wgpu::TextureSampleType::Float { filterable: false },
        view_dimension: wgpu::TextureViewDimension::D2,
        multisampled: ms,
    }
}

fn storage(format: wgpu::TextureFormat) -> wgpu::BindingType {
    wgpu::BindingType::StorageTexture {
        access: wgpu::StorageTextureAccess::WriteOnly,
        format,
        view_dimension: wgpu::TextureViewDimension::D2,
    }
}

fn bind(binding: u32, resource: wgpu::BindingResource<'_>) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry { binding, resource }
}

/// The entries of group 3 of the alpha and depth passes (object.wgsl).
fn face_entries() -> [wgpu::BindGroupLayoutEntry; 2] {
    [
        entry(
            30,
            wgpu::ShaderStages::FRAGMENT,
            buffer(wgpu::BufferBindingType::Storage { read_only: true }),
        ),
        entry(31, wgpu::ShaderStages::FRAGMENT, buffer(wgpu::BufferBindingType::Uniform)),
    ]
}

impl Stochastic {
    /// `pass_supported`: the sample counts the device takes for the passes'
    /// formats (`pass_samples`; `prepare` declines a frame when none fits).
    /// `scene_groups`: the scene's bind group layouts (frame, records,
    /// textures) the sample mask route's shader is written against.
    pub fn new(
        device: &wgpu::Device,
        hdr_format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
        pass_supported: Vec<u32>,
        mask_shaders: crate::spirv_fix::SampleMaskShaders,
        scene_groups: Vec<Vec<wgpu::BindGroupLayoutEntry>>,
    ) -> Self {
        use wgpu::ShaderStages as S;
        let face_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("oit stochastic faces"),
            entries: &face_entries(),
        });
        let all = S::COMPUTE | S::FRAGMENT;
        let resolve_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("oit stochastic resolve"),
            entries: &[
                entry(0, all | S::VERTEX, buffer(wgpu::BufferBindingType::Uniform)),
                entry(1, all, texture(true)),
                entry(2, all, texture(true)),
                entry(3, S::COMPUTE, storage(wgpu::TextureFormat::Rgba16Float)),
                entry(4, S::COMPUTE, storage(wgpu::TextureFormat::Rg32Float)),
                entry(5, all, texture(false)),
                entry(6, all, texture(false)),
                entry(7, all, texture(false)),
                entry(
                    10,
                    S::COMPUTE,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: true,
                    },
                ),
                entry(11, S::COMPUTE, texture(true)),
            ],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("oit stochastic"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/oit_stochastic.wgsl").into()),
        });
        let expand_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("oit stochastic expand"),
            entries: &[
                entry(
                    0,
                    S::FRAGMENT,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: true,
                    },
                ),
                entry(1, S::FRAGMENT, buffer(wgpu::BufferBindingType::Uniform)),
            ],
        });
        let expand_layout_ss = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("oit stochastic expand single sample"),
            entries: &[
                entry(1, S::FRAGMENT, buffer(wgpu::BufferBindingType::Uniform)),
                entry(
                    2,
                    S::FRAGMENT,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
            ],
        });
        let expand_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("oit stochastic expand"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/oit_stochastic_expand.wgsl").into()),
        });
        let uniform = |label, size| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let expand_params = uniform("oit stochastic expand parameters", u64::from(MAX_PASS_SAMPLES * 4));
        let params = uniform("oit stochastic parameters", std::mem::size_of::<StParams>() as u64);
        let resolve_params = uniform("oit stochastic resolve parameters", std::mem::size_of::<ResolveParams>() as u64);
        // the table of the most samples a pass has: written for the pass's
        // own count when the targets are made
        let masks = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("oit stochastic masks"),
            size: u64::from((MAX_PASS_SAMPLES + 1) * SEEDS * 4),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let face_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("oit stochastic faces"),
            layout: &face_layout,
            entries: &[bind(30, masks.as_entire_binding()), bind(31, params.as_entire_binding())],
        });
        let single = wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING;
        let small = |label, format| crate::renderer::make_view(device, label, 1, 1, format, 1, single).1;
        log::info!("oit stochastic: made; pass sample counts the device takes: {pass_supported:?}");
        Stochastic {
            shared: Shared {
                face_layout,
                resolve_layout,
                module,
                hdr_format,
                depth_format,
                mask_shaders: std::sync::Arc::new((mask_shaders, scene_groups)),
                pass_supported,
                expand_layout,
                expand_layout_ss,
                expand_module,
            },
            temporal: TEMPORAL,
            scene_history: SceneHistory::None,
            masks,
            mask_pass: 0,
            params,
            face_group,
            resolve_params,
            expand_params,
            dummy_rgba_r: small("oit stochastic dummy rgba r", wgpu::TextureFormat::Rgba16Float),
            dummy_rgba_w: small("oit stochastic dummy rgba w", wgpu::TextureFormat::Rgba16Float),
            dummy_rg_r: small("oit stochastic dummy rg32 r", wgpu::TextureFormat::Rg32Float),
            dummy_rg_w: small("oit stochastic dummy rg32 w", wgpu::TextureFormat::Rg32Float),
            targets: None,
            refused: None,
            frame: 0,
            history_ready: false,
            idle: 0,
            glow: false,
        }
    }

    pub fn shared(&self) -> &Shared {
        &self.shared
    }

    /// The frame's targets and parameters. `false` when the passes cannot
    /// run (no multisampled count for the scene, or the memory refused the
    /// targets): the frame then draws the sorted order.
    pub fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, f: &Frame) -> bool {
        let pass = self.shared.pass_samples(f.samples);
        if pass < 2 {
            return false;
        }
        // the scene's TAA accumulates already: a second history would ghost
        // twice, so the blend (and its seeds changing every frame) is off
        // there; on under FSR (`temporal_weight`)
        let temporal = temporal_weight(TEMPORAL, f.history);
        if temporal != self.temporal || f.history != self.scene_history {
            let why = match f.history {
                SceneHistory::Taa => "off, the scene's TAA accumulates",
                SceneHistory::Fsr => "on, before FSR (its reactive mask leaves the blended faces to this frame)",
                SceneHistory::None => "on",
            };
            log::info!("oit stochastic: temporal blend {why}");
            self.temporal = temporal;
        }
        self.scene_history = f.history;
        let history = temporal > 0.0;
        let key = (f.width, f.height, f.samples, pass, history);
        if self.refused == Some(key) {
            return false;
        }
        let stale = self
            .targets
            .as_ref()
            .is_none_or(|t| t.width != f.width || t.height != f.height || t.samples != f.samples || t.pass != pass || t.history != history);
        if stale {
            // the old ones go first: both sets do not have to fit
            self.targets = None;
            self.history_ready = false;
            let (made, refused) =
                crate::gpu_alloc::scoped(device, || self.make_targets(device, f.width, f.height, f.samples, pass, history));
            if refused {
                log::warn!(
                    "oit stochastic: the memory refused the targets ({}x{}, {} MB): the sorted order is drawn",
                    f.width,
                    f.height,
                    target_bytes(f.width, f.height, f.samples, pass, history) / (1024 * 1024)
                );
                self.refused = Some(key);
                return false;
            }
            self.refused = None;
            self.targets = Some(made);
            queue.write_buffer(&self.expand_params, 0, bytemuck::cast_slice(&expand_map(f.samples, pass)));
            if self.mask_pass != pass {
                queue.write_buffer(&self.masks, 0, bytemuck::cast_slice(&mask_table(pass)));
                self.mask_pass = pass;
            }
        }
        self.idle = 0;
        self.frame = self.frame.wrapping_add(1);
        let sp = StParams {
            samples: pass,
            flags: u32::from(history),
            frame_seed: self.frame.wrapping_mul(0x9e37_79b9),
            _pad: 0,
        };
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&sp));
        // the blend takes this frame alone when the targets are new or the
        // camera jumped: what they hold is not this view's
        let weight = if self.history_ready && !f.cut { self.temporal } else { 1.0 };
        let rp = ResolveParams {
            width: f.width,
            height: f.height,
            samples_hw: pass,
            flags: u32::from(history) | (u32::from(f.glow) << 1) | (u32::from(f.samples == 1) << 2),
            temporal_weight: weight,
            trans_samples: trans_samples(f.samples, pass),
            _pad: [0; 2],
            reproj: f.reproj,
        };
        queue.write_buffer(&self.resolve_params, 0, bytemuck::bytes_of(&rp));
        self.history_ready = true;
        self.glow = f.glow;
        true
    }

    fn make_targets(&self, device: &wgpu::Device, width: u32, height: u32, samples: u32, pass: u32, history: bool) -> Targets {
        let make = |label, format, samples, usage| crate::renderer::make_view(device, label, width, height, format, samples, usage);
        use wgpu::TextureUsages as U;
        let trans_n = trans_samples(samples, pass);
        let (_, trans) = make(
            "oit stochastic transmittance",
            wgpu::TextureFormat::R16Float,
            trans_n,
            U::RENDER_ATTACHMENT | U::TEXTURE_BINDING,
        );
        let (_, accum) = make(
            "oit stochastic accumulation",
            self.shared.hdr_format,
            pass,
            U::RENDER_ATTACHMENT | U::TEXTURE_BINDING,
        );
        let (_, glow) = make(
            "oit stochastic glow",
            wgpu::TextureFormat::R16Float,
            pass,
            U::RENDER_ATTACHMENT | U::TEXTURE_BINDING,
        );
        let (depth_tex, depth) = make(
            "oit stochastic depth",
            self.shared.depth_format,
            pass,
            U::RENDER_ATTACHMENT | U::COPY_DST | U::TEXTURE_BINDING,
        );
        let single = U::STORAGE_BINDING | U::TEXTURE_BINDING;
        // the resolved colour: the last frame's is read by the temporal
        // blend alone (otherwise one, the placeholder as its history)
        let resolved0 = make("oit stochastic resolved 0", wgpu::TextureFormat::Rgba16Float, 1, single).1;
        let resolved1 = history.then(|| make("oit stochastic resolved 1", wgpu::TextureFormat::Rgba16Float, 1, single).1);
        // the total alpha (red) and the corrected glow (green)
        let (_, total) = make("oit stochastic total alpha", wgpu::TextureFormat::Rg32Float, 1, single);
        let groups = std::array::from_fn(|parity| {
            let (cur, old) = match &resolved1 {
                Some(r1) if parity == 1 => (r1, &resolved0),
                Some(r1) => (&resolved0, r1),
                None => (&resolved0, &self.dummy_rgba_r),
            };
            let group = |label, entries: [(u32, &wgpu::TextureView); 5]| {
                let mut all = vec![
                    bind(0, self.resolve_params.as_entire_binding()),
                    bind(1, wgpu::BindingResource::TextureView(&accum)),
                    bind(2, wgpu::BindingResource::TextureView(&trans)),
                    bind(10, wgpu::BindingResource::TextureView(&depth)),
                    bind(11, wgpu::BindingResource::TextureView(&glow)),
                ];
                all.extend(entries.iter().map(|&(b, v)| bind(b, wgpu::BindingResource::TextureView(v))));
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some(label),
                    layout: &self.shared.resolve_layout,
                    entries: &all,
                })
            };
            // writes cur and total, reads old
            let resolve = group(
                "oit stochastic resolve",
                [(3, cur), (4, &total), (5, old), (6, &self.dummy_rgba_r), (7, &self.dummy_rg_r)],
            );
            // reads cur and total
            let composite = group(
                "oit stochastic composite",
                [
                    (3, &self.dummy_rgba_w),
                    (4, &self.dummy_rg_w),
                    (5, &self.dummy_rgba_r),
                    (6, cur),
                    (7, &total),
                ],
            );
            (resolve, composite)
        });
        log::info!(
            "oit stochastic: targets {width}x{height}, the scene at {samples} samples, the passes at {pass}, {:.0} MB",
            target_bytes(width, height, samples, pass, history) as f64 / (1024.0 * 1024.0)
        );
        Targets {
            width,
            height,
            samples,
            pass,
            history,
            trans_samples: trans_n,
            trans,
            accum,
            glow,
            depth_tex,
            depth,
            groups,
        }
    }

    /// Frees the targets while the setting is off (as `Bloom::release`).
    pub fn release(&mut self) {
        if self.targets.take().is_some() {
            log::info!("oit stochastic: targets freed");
        }
        self.history_ready = false;
        self.refused = None;
        self.idle = 0;
    }

    /// A frame with the method on and no blended face: the targets go after
    /// a while.
    pub fn unused_frame(&mut self) {
        if self.targets.is_some() {
            self.idle += 1;
            if self.idle >= IDLE_FRAMES {
                self.release();
            }
        }
    }

    fn parity(&self) -> usize {
        (self.frame & 1) as usize
    }

    /// Group 3 of the alpha and depth passes.
    pub fn face_group(&self) -> &wgpu::BindGroup {
        &self.face_group
    }

    /// The total alpha runs against the stochastic depth (the scene without
    /// MSAA): `begin_iteration` comes before it, to spread the opaque depth
    /// over the passes' samples.
    pub fn alpha_on_pass_depth(&self) -> bool {
        self.targets.as_ref().is_some_and(|t| t.trans_samples != t.samples)
    }

    /// The pass of the total alpha: the target cleared to full
    /// transmittance, the scene depth (or, `alpha_on_pass_depth`, the
    /// stochastic depth holding the opaque one) read only.
    pub fn begin_alpha<'a>(
        &'a self,
        encoder: &'a mut wgpu::CommandEncoder,
        scene_depth: &'a wgpu::TextureView,
    ) -> Option<wgpu::RenderPass<'a>> {
        let t = self.targets.as_ref()?;
        let depth = if t.trans_samples != t.samples { &t.depth } else { scene_depth };
        Some(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("oit stochastic total alpha"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &t.trans,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: None,
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        }))
    }

    /// Before the passes: the opaque depth copied into the stochastic depth
    /// (spread over its samples when it has more), and the accumulation
    /// cleared.
    pub fn begin_iteration(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        scene_depth: (&wgpu::Texture, &wgpu::TextureView),
        r: &Resolve,
        clear_accum: bool,
    ) {
        let Some(t) = &self.targets else { return };
        match (&r.expand, t.pass != t.samples) {
            (Some(expand), true) => {
                // (binding 0 the multisampled scene depth, 2 the single one)
                let (layout, binding) = if t.samples > 1 {
                    (&self.shared.expand_layout, 0)
                } else {
                    (&self.shared.expand_layout_ss, 2)
                };
                let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("oit stochastic expand"),
                    layout,
                    entries: &[
                        bind(binding, wgpu::BindingResource::TextureView(scene_depth.1)),
                        bind(1, self.expand_params.as_entire_binding()),
                    ],
                });
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("oit stochastic expand"),
                    color_attachments: &[],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &t.depth,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(0.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(expand);
                pass.set_bind_group(0, &group, &[]);
                pass.draw(0..3, 0..1);
            }
            _ => encoder.copy_texture_to_texture(scene_depth.0.as_image_copy(), t.depth_tex.as_image_copy(), scene_depth.0.size()),
        }
        if clear_accum {
            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("oit stochastic accumulation clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &t.accum,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }
    }

    /// The stochastic depth pass: depth only, written.
    pub fn begin_depth<'a>(&'a self, encoder: &'a mut wgpu::CommandEncoder) -> Option<wgpu::RenderPass<'a>> {
        let t = self.targets.as_ref()?;
        Some(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("oit stochastic depth"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &t.depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        }))
    }

    /// The accumulation pass: additive into the accumulation, the
    /// stochastic depth read only.
    pub fn begin_accum<'a>(&'a self, encoder: &'a mut wgpu::CommandEncoder) -> Option<wgpu::RenderPass<'a>> {
        let t = self.targets.as_ref()?;
        Some(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("oit stochastic accumulation"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &t.accum,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &t.depth,
                depth_ops: None,
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        }))
    }

    /// The glow accumulation: additive into the glow target (cleared), the
    /// stochastic depth read only.
    pub fn begin_glow<'a>(&'a self, encoder: &'a mut wgpu::CommandEncoder) -> Option<wgpu::RenderPass<'a>> {
        let t = self.targets.as_ref()?;
        Some(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("oit stochastic glow"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &t.glow,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &t.depth,
                depth_ops: None,
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        }))
    }

    /// The resolve.
    pub fn resolve(&self, encoder: &mut wgpu::CommandEncoder, r: &Resolve) {
        let Some(t) = &self.targets else { return };
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("oit stochastic resolve"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&r.resolve);
        pass.set_bind_group(0, &t.groups[self.parity()].0, &[]);
        pass.dispatch_workgroups(t.width.div_ceil(8), t.height.div_ceil(8), 1);
    }

    /// The composite over the scene, in a scene pass (per sample).
    pub fn composite(&self, pass: &mut wgpu::RenderPass, r: &Resolve) {
        let Some(t) = &self.targets else { return };
        pass.set_pipeline(&r.composite);
        pass.set_bind_group(0, &t.groups[self.parity()].1, &[]);
        pass.draw(0..3, 0..1);
        if self.glow {
            pass.set_pipeline(&r.composite_glow);
            pass.draw(0..3, 0..1);
        }
    }
}

impl Shared {
    /// The samples per stochastic pass for a scene of `scene` samples.
    pub fn pass_samples(&self, scene: u32) -> u32 {
        pass_samples(scene, &self.pass_supported)
    }

    /// The scene module for the sample mask route: `src` (the scene's WGSL
    /// with the method's passes) with its overrides turned into constants
    /// (`values` for the ones the stochastic depth pipeline sets, the
    /// declared defaults for the rest: a passthrough module cannot be
    /// specialised), created through spirv_fix against the scene's layouts
    /// and the faces' group. `None` when it cannot be made right (Vulkan
    /// without the fix): the index route then.
    pub fn mask_module(&self, device: &wgpu::Device, src: &str, values: &[(&str, &str)]) -> Option<wgpu::ShaderModule> {
        let wgsl = constants_for_overrides(src, values);
        let (shaders, scene) = &*self.mask_shaders;
        let map = mask_binding_map(scene, &wgsl);
        shaders.try_create_shader_module_mapped(device, "scene stochastic mask", &wgsl, map)
    }
}

/// The binding map of the sample mask route's module: the scene's groups
/// then the faces' (group 3), and for every other resource the source
/// declares (the lists' group 3, the impostors'...: naga wants a mapping
/// for each) a binding past the real ones, which the entry points never use.
pub fn mask_binding_map(scene_groups: &[Vec<wgpu::BindGroupLayoutEntry>], wgsl: &str) -> naga::back::spv::BindingMap {
    let face = face_entries();
    let mut groups: Vec<&[wgpu::BindGroupLayoutEntry]> = scene_groups.iter().map(|g| g.as_slice()).collect();
    groups.push(&face);
    let mut map = crate::spirv_fix::binding_map(&groups);
    let mut next = 64u32;
    for (group, binding) in declared_bindings(wgsl) {
        map.entry(naga::ResourceBinding { group, binding }).or_insert_with(|| {
            next += 1;
            naga::back::spv::BindingInfo {
                descriptor_set: group,
                binding: next,
                binding_array_size: None,
            }
        });
    }
    map
}

/// Every `@group(g) @binding(b)` a WGSL source declares.
pub fn declared_bindings(src: &str) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    for (i, _) in src.match_indices("@group(") {
        let rest = &src[i + 7..];
        let Some(g_end) = rest.find(')') else { continue };
        let Ok(group) = rest[..g_end].trim().parse::<u32>() else { continue };
        let after = &rest[g_end + 1..];
        let Some(b_at) = after.find("@binding(") else { continue };
        if !after[..b_at].trim().is_empty() {
            continue;
        }
        let b_rest = &after[b_at + 9..];
        let Some(b_end) = b_rest.find(')') else { continue };
        if let Ok(binding) = b_rest[..b_end].trim().parse::<u32>() {
            out.push((group, binding));
        }
    }
    out
}

/// `src` with every `override NAME: T = v;` turned into `const NAME: T =
/// v;`, `values` replacing the declared values of the names it lists.
pub fn constants_for_overrides(src: &str, values: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(src.len());
    for line in src.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("override ")
            && let Some((name, _)) = rest.split_once(':')
            && let Some(ty_end) = rest.find('=')
            && let Some(semi) = rest.find(';')
        {
            let name = name.trim();
            let ty = rest[name.len()..ty_end].trim_start_matches([':', ' ']).trim();
            let value = values
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, v)| *v)
                .unwrap_or_else(|| rest[ty_end + 1..semi].trim());
            out.push_str(&format!("const {name}: {ty} = {value};{}", &rest[semi + 1..]));
        } else {
            out.push_str(line);
        }
    }
    out
}

/// The resolve's pipelines for `samples` (with the scene's pipelines).
pub(crate) fn make_resolve(device: &wgpu::Device, shared: &Shared, samples: u32) -> Resolve {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("oit stochastic resolve"),
        bind_group_layouts: &[Some(&shared.resolve_layout)],
        immediate_size: 0,
    });
    let resolve = device.create_compute_pipeline_cached(&wgpu::ComputePipelineDescriptor {
        label: Some("oit stochastic resolve"),
        layout: Some(&layout),
        module: &shared.module,
        entry_point: Some("cs_resolve"),
        compilation_options: Default::default(),
        cache: None,
    });
    let composite_pipeline = |label, fs: &str, blend: wgpu::BlendState, write_mask| {
        device.create_render_pipeline_cached(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shared.module,
                entry_point: Some("vs_full"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shared.module,
                entry_point: Some(fs),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: shared.hdr_format,
                    blend: Some(blend),
                    write_mask,
                })],
            }),
            primitive: Default::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: shared.depth_format,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: samples,
                ..Default::default()
            },
            multiview_mask: None,
            cache: None,
        })
    };
    // the scene sample dimmed by its transparency, the colour added; the
    // glow (scene alpha) dimmed the same way
    let composite = composite_pipeline(
        "oit stochastic composite",
        "fs_composite",
        wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
        },
        wgpu::ColorWrites::ALL,
    );
    // added to the glow amount (the scene alpha) alone
    let composite_glow = composite_pipeline(
        "oit stochastic composite glow",
        "fs_composite_glow",
        wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        },
        wgpu::ColorWrites::ALPHA,
    );
    let pass = shared.pass_samples(samples);
    let expand = (pass != samples).then(|| {
        let (expand_layout, fs) = if samples > 1 {
            (&shared.expand_layout, "fs_expand")
        } else {
            (&shared.expand_layout_ss, "fs_expand_ss")
        };
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("oit stochastic expand"),
            bind_group_layouts: &[Some(expand_layout)],
            immediate_size: 0,
        });
        device.create_render_pipeline_cached(&wgpu::RenderPipelineDescriptor {
            label: Some("oit stochastic expand"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shared.expand_module,
                entry_point: Some("vs_full"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shared.expand_module,
                entry_point: Some(fs),
                compilation_options: Default::default(),
                targets: &[],
            }),
            primitive: Default::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: shared.depth_format,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: pass,
                ..Default::default()
            },
            multiview_mask: None,
            cache: None,
        })
    });
    Resolve {
        resolve,
        composite,
        composite_glow,
        expand,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mask_table_holds_random_r_subsets() {
        for s in [2, 4, 8] {
            let t = mask_table(s);
            assert_eq!(t.len(), ((s + 1) * SEEDS) as usize);
            for r in 0..=s {
                for seed in 0..SEEDS {
                    let m = t[(r * SEEDS + seed) as usize];
                    assert_eq!(m.count_ones(), r, "S {s} R {r} seed {seed}");
                    assert_eq!(m >> s, 0);
                }
            }
        }
        // not the same subset for every seed
        let t = mask_table(8);
        let row = &t[(4 * SEEDS) as usize..(5 * SEEDS) as usize];
        assert!(row.iter().any(|&m| m != row[0]));
        // the same every call
        assert_eq!(mask_table(8), mask_table(8));
    }

    /// The shader's `st_hash` (PCG) and its per-pixel, per-layer subset index.
    fn st_hash(v: u32) -> u32 {
        let mut x = v.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
        x = ((x >> ((x >> 28) + 4)) ^ x).wrapping_mul(277_803_737);
        (x >> 22) ^ x
    }

    fn subset_index(layer_seed: u32, px: u32, py: u32) -> u32 {
        let h = st_hash(layer_seed ^ st_hash(px | (py << 16)) ^ 0x5bd1_e995) >> 8;
        ((h as f32 / 16_777_216.0) * SEEDS as f32) as u32
    }

    #[test]
    fn two_layers_never_share_a_subset_over_their_overlap() {
        // The index is hashed per pixel and layer: any two seeds, however
        // close, differ at most pixels, and each layer's indices stay inside
        // the table.
        let (a, b) = (st_hash(7), st_hash(7) + 1);
        let mut same = 0;
        let mut seen = std::collections::HashSet::new();
        for y in 0..64 {
            for x in 0..64 {
                let (ia, ib) = (subset_index(a, x, y), subset_index(b, x, y));
                assert!(ia < SEEDS && ib < SEEDS);
                same += u32::from(ia == ib);
                seen.insert(ia);
            }
        }
        // chance level is 4096 / 2048 = 2 pixels
        assert!(same < 40, "{same} of 4096 pixels share a subset");
        assert!(seen.len() > 1200, "one layer covers {} of {SEEDS} subsets", seen.len());
        // every index picks a valid R-subset of the table
        let t = mask_table(8);
        for i in 0..64 {
            assert_eq!(t[(5 * SEEDS + subset_index(a, i, 3)) as usize].count_ones(), 5);
        }
    }

    #[test]
    fn the_pass_takes_the_most_samples_up_to_8() {
        let all = [2, 4, 8];
        assert_eq!(pass_samples(4, &all), 8);
        assert_eq!(pass_samples(1, &all), 8);
        assert_eq!(pass_samples(8, &all), 8);
        // never fewer than the scene's, nor a count the device lacks
        assert_eq!(pass_samples(4, &[2, 4]), 4);
        assert_eq!(pass_samples(1, &[4]), 4);
        // a scene count the formats do not all take, or none at all: declined
        assert_eq!(pass_samples(8, &[2, 4]), 1);
        assert_eq!(pass_samples(2, &[4, 8]), 1);
        assert_eq!(pass_samples(1, &[]), 1);
    }

    #[test]
    fn each_pass_sample_takes_the_nearest_scene_sample() {
        // 8 over 4: two pass samples to each scene sample
        let m = expand_map(4, 8);
        assert_eq!(&m[..8], &[0, 3, 1, 0, 2, 2, 3, 1]);
        // the same count: each its own
        assert_eq!(&expand_map(4, 4)[..4], &[0, 1, 2, 3]);
        for n in [2, 4] {
            let m = expand_map(n, 8);
            assert!(m.iter().all(|&k| k < n));
            for k in 0..n {
                assert!(m.contains(&k), "{n}: scene sample {k} unused");
            }
        }
    }

    #[test]
    fn the_targets_hold_only_what_is_read() {
        let mib = |w, h, s, history| target_bytes(w, h, s, 8, history) as f64 / (1024.0 * 1024.0);
        // MSAA 4x: 136 bytes a pixel without the history, 8 more with it
        assert_eq!((mib(1600, 900, 4, false) + 0.5) as u32, 187);
        assert_eq!((mib(2560, 1440, 4, false) + 0.5) as u32, 478);
        assert_eq!((mib(1600, 900, 4, true) + 0.5) as u32, 198);
        assert_eq!((mib(2560, 1440, 4, true) + 0.5) as u32, 506);
        // without MSAA the total alpha is at the passes' samples
        assert_eq!((mib(1600, 900, 1, false) + 0.5) as u32, 198);
    }

    #[test]
    fn the_declared_bindings_are_listed() {
        let src = "@group(0) @binding(2) var<uniform> a: u32;
@group(3)  @binding(20) var<storage> b: array<u32>;
";
        assert_eq!(declared_bindings(src), vec![(0, 2), (3, 20)]);
    }

    #[test]
    fn overrides_become_constants() {
        let src = "override A: bool = true;\noverride WATER_CLIP: u32 = 0u;\n  override B: f32 = 0.5; // c\nfn f() {}\n";
        let out = constants_for_overrides(src, &[("WATER_CLIP", "1u")]);
        assert_eq!(
            out,
            "const A: bool = true;\nconst WATER_CLIP: u32 = 1u;\nconst B: f32 = 0.5; // c\nfn f() {}\n"
        );
    }

    #[test]
    fn the_method_follows_the_environment_then_the_setting() {
        assert_eq!(method_with(None, true), Method::Stochastic);
        assert_eq!(method_with(None, false), Method::Off);
        // the environment decides either way
        assert_eq!(method_with(Some("0"), true), Method::Off);
        assert_eq!(method_with(Some("1"), false), Method::Stochastic);
        assert_eq!(method_with(Some(" stochastic "), false), Method::Stochastic);
        // anything else leaves it to the setting
        assert_eq!(method_with(Some("on"), false), Method::Off);
        assert_eq!(method_with(Some("on"), true), Method::Stochastic);
    }

    #[test]
    fn a_frame_tries_the_method_only_when_it_is_on_with_blended_faces_in_the_plain_view() {
        let on = Method::Stochastic;
        assert!(frame_wants_method(on, 3, true));
        // off, nothing blended, or a photo or a snapshot tile: the sorted
        // order
        assert!(!frame_wants_method(Method::Off, 3, true));
        assert!(!frame_wants_method(on, 0, true));
        assert!(!frame_wants_method(on, 3, false));
    }

    #[test]
    fn the_temporal_blend_is_off_under_taa_only() {
        assert_eq!(temporal_weight(TEMPORAL, SceneHistory::None), 0.125);
        assert_eq!(temporal_weight(TEMPORAL, SceneHistory::Taa), 0.0);
        assert_eq!(temporal_weight(TEMPORAL, SceneHistory::Fsr), 0.125);
        assert_eq!(temporal_weight(0.0, SceneHistory::Fsr), 0.0);
    }

    #[test]
    fn the_scene_module_has_none_of_the_passes() {
        let object = include_str!("shaders/object.wgsl").replace("\r\n", "\n");
        let scene = without_st(&object);
        for name in [
            "fs_st_alpha",
            "fs_st_depth_mask",
            "fs_st_depth_index",
            "fs_st_glow",
            "st_masks",
            "water_clipped",
        ] {
            assert!(!scene.contains(name), "{name} is in the scene module");
            assert!(object.contains(name), "{name} is not in object.wgsl");
        }
        // what the scene and the passes share stays
        assert!(scene.contains("fn glow_amount"));
        assert!(scene.contains("fn fs_blend"));
        assert!(!scene.contains("ST_BEGIN") && !scene.contains("ST_END"));
    }

    #[test]
    fn the_shaders_are_valid() {
        fn validate(label: &str, src: &str) {
            let module = naga::front::wgsl::parse_str(src).unwrap_or_else(|e| panic!("{label}: {}", e.emit_to_string(src)));
            let mut v = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all());
            if let Err(e) = v.validate(&module) {
                panic!("{label}: {}", e.emit_to_string(src));
            }
        }
        validate("resolve", include_str!("shaders/oit_stochastic.wgsl"));
        validate("expand", include_str!("shaders/oit_stochastic_expand.wgsl"));
        let object = include_str!("shaders/object.wgsl").replace("\r\n", "\n");
        let env = include_str!("shaders/env.wgsl");
        // the scene module, without the passes
        validate("scene", &format!("{}\n{}\n{}", crate::COMMON_SHADER, without_st(&object), env));
        // the method's module, with and without the triangle index
        for prim in [false, true] {
            let generated = st_object_source_with(&object, prim);
            // (the parameter marker always goes; the value marker stays as a
            // comment when the triangle index is not read)
            assert!(!generated.contains("/*ST_PRIM_PARAM*/"));
            assert_eq!(generated.contains("prim_id"), prim);
            validate(
                &format!("st prim {prim}"),
                &format!("{}{}\n{}\n{}", enable_directive_with(prim), crate::COMMON_SHADER, generated, env),
            );
        }
    }

    /// The sample mask route off Vulkan (spirv_fix gives the plain module
    /// there): naga's HLSL (DX12) and MSL (Metal) writers take the scene
    /// module's `fs_st_depth_mask` as `mask_module` makes it (the overrides
    /// turned into constants), the coverage a scalar `SV_Coverage` /
    /// `[[sample_mask]]` output, as those languages want it.
    #[test]
    fn the_mask_route_writes_hlsl_and_msl() {
        let object = include_str!("shaders/object.wgsl").replace("\r\n", "\n");
        let src = format!(
            "{}{}\n{}\n{}",
            enable_directive_with(true),
            crate::COMMON_SHADER,
            st_object_source_with(&object, true),
            include_str!("shaders/env.wgsl")
        );
        let src = constants_for_overrides(&src, &[("WATER_CLIP", "1u"), ("BLEND_EARLY_DISCARD", "true")]);
        let module = naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&src)));
        let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
            .validate(&module)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&src)));
        let entry = Some((naga::ShaderStage::Fragment, "fs_st_depth_mask".to_owned()));
        // (wgpu sizes the bindless texture pages from the layout's count; the
        // other resources take made-up registers)
        let mut binding_map = naga::back::hlsl::BindingMap::default();
        binding_map.insert(
            naga::ResourceBinding { group: 2, binding: 0 },
            naga::back::hlsl::BindTarget {
                space: 2,
                register: 0,
                binding_array_size: Some(64),
                ..Default::default()
            },
        );
        let mut hlsl = String::new();
        naga::back::hlsl::Writer::new(
            &mut hlsl,
            &naga::back::hlsl::Options {
                // wgpu's DX12 backend compiles with DXC at 6.x (bindless)
                shader_model: naga::back::hlsl::ShaderModel::V6_6,
                binding_map,
                fake_missing_bindings: true,
                ..Default::default()
            },
            &naga::back::hlsl::PipelineOptions {
                entry_point: entry.clone(),
            },
        )
        .write(&module, &info, None)
        .expect("HLSL");
        let coverage: Vec<&str> = hlsl.lines().filter(|l| l.contains("SV_Coverage")).map(str::trim).collect();
        assert_eq!(coverage.len(), 1, "{coverage:?}");
        assert!(coverage[0].starts_with("uint "), "{coverage:?}");
        let (msl, _) = naga::back::msl::write_string(
            &module,
            &info,
            &naga::back::msl::Options {
                lang_version: (2, 4),
                ..Default::default()
            },
            &naga::back::msl::PipelineOptions {
                entry_point: entry,
                ..Default::default()
            },
        )
        .expect("MSL");
        assert_eq!(msl.matches("[[sample_mask]]").count(), 1);
        assert!(msl.contains("uint mask [[sample_mask]]"));
    }
}
