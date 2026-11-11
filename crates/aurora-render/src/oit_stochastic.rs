//! Order-independent transparency by depth-based stochastic transparency
//! (the OIT setting; `AURORA_OIT`, `AURORA_OIT_SAMPLES` and
//! `AURORA_OIT_STOCH` in docs/TESTING.md). An Aurora addition: the Legacy
//! Viewer sorts its alpha (LLDrawPoolAlpha).
//!
//! Enderton, Sintorn, Shirley and Luebke, "Stochastic Transparency", I3D
//! 2010, section 3.4, the final anti-aliased depth-based algorithm:
//!
//! 1. the opaque scene in its multisampled depth (the renderer's);
//! 2. the total alpha of every sample, `1 - prod(1 - alpha)`, into a
//!    multisampled target (`fs_st_alpha`, one pass, order independent);
//! 3. the blended faces into a copy of the opaque depth, each fragment
//!    covering a random subset of its samples of size `alpha * S`, depth
//!    only (`fs_st_depth_index` or `fs_st_depth_mask`);
//! 4. the accumulation: `alpha * colour` (and alpha) added into the samples
//!    where the fragment is at or in front of the stochastic depth
//!    (`fs_blend`, additive, GreaterEqual);
//! 5. the composite: every scene sample dimmed by its transparency, plus
//!    the accumulated colour averaged over the pixel's samples, times the
//!    alpha correction (total alpha over accumulated alpha).
//!
//! The glow of the blended faces that have some is accumulated like their
//! colour (`fs_st_glow`, after each accumulation), corrected the same way and
//! added to the scene alpha after the composite: on average each face's
//! glow dimmed by the layers in front of it, as the Legacy Viewer's glow
//! drawn after each face in order and suppressed by the ones after it.
//!
//! The passes 3 and 4 run at their own sample count, the most the device
//! has up to 8 (the paper's, section 4: 8 per pass, 1 + 2 S / 8 passes),
//! whatever the scene's: their depth starts as the opaque depth spread over
//! those samples (`oit_stochastic_expand.wgsl`). With MSAA the total alpha
//! and the composite stay at the scene's samples (each scene sample dimmed by
//! its own transparency). Without (SMAA, FXAA, TAA, FSR or none: one sample) the
//! total alpha runs at the passes' samples against that spread depth and the
//! composite dims each pixel by its resolved transparency; the scene's
//! anti-aliasing then runs over the result as over the sorted order.
//! Under FSR that is the render size, before the upscale. The temporal
//! blend is off under TAA, on under FSR (`temporal_weight`).
//! More samples than a pass
//! holds (`AURORA_OIT_SAMPLES`): the passes 3 and 4 run again with the next
//! bits of one stratified mask (paper footnote 1). The masks come from a table of random S-choose-R
//! subsets (`masks`), fixed for every launch; the seed is the pixel, the
//! face's tie-break and the triangle. Needs multisampled targets of at least
//! 2 samples (every device has 4), and `PRIMITIVE_INDEX` for the triangle in
//! the seed (without it the seed is per face: coplanar fragments of one face
//! then occlude instead of blending).

use crate::pipeline_cache::CachedPipelines;
use bytemuck::{Pod, Zeroable};

/// Seeds per alpha level in the mask table (the paper's width, 2 047).
pub const SEEDS: u32 = 2048;
pub const MAX_SAMPLES: u32 = 32;
/// The most samples a stochastic pass takes (the expand map's size).
pub const MAX_PASS_SAMPLES: u32 = 16;

/// How the coverage mask reaches the samples of the stochastic depth.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Route {
    /// one invocation per sample (`@builtin(sample_index)`), the samples
    /// off the mask discarded: the fallback, and `AURORA_OIT_STOCH=index`
    SampleIndex,
    /// the mask written once per fragment (`@builtin(sample_mask)`): the
    /// default, a quarter of the invocations at 4x. On Vulkan only through
    /// spirv_fix (naga 30's SPIR-V declares the built-in wrong); when that
    /// cannot make the module, the index route is taken
    SampleMask,
}

/// `AURORA_OIT=stochastic`, `AURORA_OIT_SAMPLES`, `AURORA_OIT_STOCH`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Config {
    /// samples per pixel in all: 8, 16 or 32 (8 by default, one run of the
    /// passes, with the temporal blend averaging the noise over the frames:
    /// the paper finds 8 pleasant on hair and foliage, 16 adequate on broad
    /// smooth surfaces)
    pub samples: u32,
    pub stratified: bool,
    pub correction: bool,
    /// the cross-bilateral filter's radius (0: off) and alpha sigma
    pub filter: u32,
    pub sigma_alpha: f32,
    /// the temporal blend's weight of the current frame (0: off; 1/8 by
    /// default, and off under TAA, which accumulates already)
    pub temporal: f32,
    pub route: Route,
    /// the triangle in the seed
    pub prim_seed: bool,
    /// the per-pixel part of the seed
    pub seed: Seed,
    /// samples per stochastic pass: 0 the most the device has up to 8,
    /// else this count when the device has it (`pass=<n>`)
    pub pass_samples: u32,
}

/// The per-pixel value the masks are drawn from: white noise (a hash), the
/// R2 low-discrepancy sequence, or a 64 x 64 blue-noise tile
/// (oit_stochastic_blue.rs).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Seed {
    White = 0,
    R2 = 1,
    Blue = 2,
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
/// then stay the same from frame to frame, as with `notemporal`, and TAA
/// keeps that grain steady. `wanted` otherwise, FSR included: its reactive
/// mask (the scene copy against the composited colour) marks the blended
/// faces, where FSR takes this frame's colour rather than its history, so
/// a static grain would stay, enlarged by the upscale and sharpened by
/// RCAS, while the blend averages it as without a scene history (8 panes
/// under FSR Quality: the grain gone, as smooth as OIT off; no trail seen
/// on the hair fly-through).
pub(crate) fn temporal_weight(wanted: f32, history: SceneHistory) -> f32 {
    match history {
        SceneHistory::None | SceneHistory::Fsr => wanted,
        SceneHistory::Taa => 0.0,
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            samples: 8,
            stratified: true,
            correction: true,
            filter: 0,
            sigma_alpha: 0.1,
            temporal: 1.0 / 8.0,
            route: Route::SampleMask,
            prim_seed: true,
            seed: Seed::White,
            pass_samples: 0,
        }
    }
}

/// How the blended faces are drawn: the OIT setting on means stochastic
/// transparency, off the Legacy Viewer's sorted order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Method {
    /// the Legacy Viewer's sorted order
    Off,
    Stochastic,
}

/// `AURORA_OIT`: `0` off, `1` or `stochastic` on, whatever the setting;
/// `None` without the variable (the setting decides).
pub(crate) fn env_method() -> Option<Method> {
    static METHOD: std::sync::OnceLock<Option<Method>> = std::sync::OnceLock::new();
    *METHOD.get_or_init(|| match std::env::var("AURORA_OIT").ok()?.trim() {
        "0" => Some(Method::Off),
        "1" | "stochastic" => Some(Method::Stochastic),
        _ => None,
    })
}

/// The method this frame, from the environment or the OIT setting.
pub(crate) fn method(setting_oit: bool) -> Method {
    env_method().unwrap_or(if setting_oit { Method::Stochastic } else { Method::Off })
}

/// The stochastic method can be the frame's (not ruled out by the
/// environment): the GPU timer logs its passes then.
pub(crate) fn wanted() -> bool {
    env_method() != Some(Method::Off)
}

pub(crate) fn config() -> Config {
    static CONFIG: std::sync::OnceLock<Config> = std::sync::OnceLock::new();
    *CONFIG.get_or_init(|| {
        parse_config(
            std::env::var("AURORA_OIT_SAMPLES").ok().as_deref(),
            std::env::var("AURORA_OIT_STOCH").ok().as_deref(),
        )
    })
}

/// `samples`: 8 (the default), 16 or 32. `flags`, comma separated: `naive` (one coin per
/// sample instead of a stratified mask), `nocorr` (no alpha correction),
/// `filter=<radius>[:<sigma>]`, `temporal[=<frames>]` (on by default with
/// 8 frames; `notemporal` turns it off: the resolved colour
/// blended with the last frame's reprojected through the nearest
/// transparent depth, weight 1/frames, the seeds changing every frame),
/// `index` (the sample index route instead of the sample mask's), `mask`
/// (the sample mask route, the default), `noprim` (the triangle left out of
/// the seed), `seed=white|r2|blue` (the per-pixel part of the seed),
/// `pass=<n>` (samples per stochastic pass, 2 to 16; the scene's count at
/// least).
pub fn parse_config(samples: Option<&str>, flags: Option<&str>) -> Config {
    let mut c = Config::default();
    if let Some(s) = samples.and_then(|s| s.trim().parse::<u32>().ok()) {
        c.samples = match s {
            0..=8 => 8,
            9..=16 => 16,
            _ => 32,
        };
    }
    for flag in flags.unwrap_or("").split(',').map(str::trim).filter(|f| !f.is_empty()) {
        let (key, value) = flag.split_once('=').unwrap_or((flag, ""));
        match key {
            "naive" => c.stratified = false,
            "nocorr" => c.correction = false,
            "filter" => {
                let (r, sigma) = value.split_once(':').unwrap_or((value, ""));
                c.filter = r.parse().unwrap_or(2).clamp(1, 4);
                if let Ok(s) = sigma.parse::<f32>() {
                    c.sigma_alpha = s.clamp(0.01, 1.0);
                }
            }
            "notemporal" => c.temporal = 0.0,
            "temporal" => c.temporal = 1.0 / value.parse::<f32>().unwrap_or(8.0).clamp(1.0, 64.0),
            "mask" => c.route = Route::SampleMask,
            "index" => c.route = Route::SampleIndex,
            "noprim" => c.prim_seed = false,
            "pass" => c.pass_samples = value.parse::<u32>().unwrap_or(0).min(MAX_PASS_SAMPLES),
            "seed" => {
                c.seed = match value {
                    "r2" => Seed::R2,
                    "blue" => Seed::Blue,
                    _ => Seed::White,
                }
            }
            _ => log::warn!("oit stochastic: unknown flag {flag:?}"),
        }
    }
    c
}

/// object.wgsl with the triangle index in the fragment seeds when the
/// device has `PRIMITIVE_INDEX` (the `ST_PRIM` markers).
pub(crate) fn object_source(device: &wgpu::Device, src: &str) -> String {
    object_source_with(src, device.features().contains(wgpu::Features::PRIMITIVE_INDEX))
}

pub(crate) fn object_source_with(src: &str, prim: bool) -> String {
    if prim {
        src.replace("/*ST_PRIM_PARAM*/", "@builtin(primitive_index) prim_id: u32,")
            .replace("/*ST_PRIM*/0u", "prim_id")
    } else {
        src.replace("/*ST_PRIM_PARAM*/", "")
    }
}

/// The directive the scene module starts with when the triangle index is
/// read (`enable primitive_index;`), empty otherwise.
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

/// The table of masks for `samples` bits: `[r * SEEDS + seed]` a random
/// r-subset of the bits, from a fixed generator (the same every launch).
pub fn mask_table(samples: u32) -> Vec<u32> {
    let s = samples.clamp(1, MAX_SAMPLES);
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

/// The samples per stochastic pass for a scene of `scene` samples:
/// `wanted` when the device has it (and it is at least the scene's), else
/// the most the device has up to 8, never fewer than the scene's (whose
/// targets the passes can always use). `supported`: the counts both the
/// colour and the depth formats take.
pub fn pass_samples(scene: u32, wanted: u32, supported: &[u32]) -> u32 {
    let ok = |n: u32| n >= scene && n <= MAX_PASS_SAMPLES && supported.contains(&n);
    if wanted > 0 && ok(wanted) {
        return wanted;
    }
    supported
        .iter()
        .copied()
        .filter(|&n| ok(n) && n <= 8)
        .max()
        .unwrap_or(scene)
        .max(scene)
}

/// The standard sample positions of Vulkan and Direct3D (in sixteenths of
/// a pixel from its centre), for the counts a pass or a scene can have.
fn standard_positions(n: u32) -> &'static [(i32, i32)] {
    match n {
        2 => &[(4, 4), (-4, -4)],
        4 => &[(-2, -6), (6, -2), (-6, 2), (2, 6)],
        8 => &[(1, -3), (-1, 3), (5, 1), (-3, -5), (-5, 5), (-7, -1), (3, 7), (7, -7)],
        16 => &[
            (1, 1),
            (-1, -3),
            (-3, 2),
            (4, -1),
            (-5, -2),
            (2, 5),
            (5, 3),
            (3, -5),
            (-2, 6),
            (0, -7),
            (-4, -6),
            (-6, 4),
            (-8, 0),
            (7, -4),
            (6, 7),
            (-7, -8),
        ],
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

/// Which of the resolve's optional full-size targets a configuration needs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Plan {
    /// the last frame's resolved colour (the temporal blend)
    pub history: bool,
    /// the filtered colour (the cross-bilateral filter)
    pub filtered: bool,
}

pub fn plan(c: &Config) -> Plan {
    Plan {
        history: c.temporal > 0.0,
        filtered: c.filter > 0,
    }
}

/// The bytes of the targets `make_targets` allocates, from their formats
/// (Rgba16Float colour, Depth32Float depth): the total alpha, the
/// accumulation, the glow and the stochastic depth multisampled, the
/// resolved colour (two with the history), the total alpha and glow, the
/// filtered colour with the filter, and the six 1 x 1 placeholders.
pub fn target_bytes(width: u32, height: u32, samples: u32, pass: u32, plan: Plan) -> u64 {
    let px = u64::from(width.max(1)) * u64::from(height.max(1));
    let trans = 2 * u64::from(trans_samples(samples, pass));
    let per_pass = (8 + 2 + 4) * u64::from(pass);
    let resolved = 8 * (1 + u64::from(plan.history));
    let filtered = 8 * u64::from(plan.filtered);
    px * (trans + per_pass + resolved + 8 + filtered) + 6 * 8
}

/// object.wgsl `StParams`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct StParams {
    samples_hw: u32,
    samples: u32,
    iteration: u32,
    flags: u32,
    frame_seed: u32,
    seed_mode: u32,
    _pad: [u32; 2],
}

/// oit_stochastic.wgsl `ResolveParams`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ResolveParams {
    width: u32,
    height: u32,
    /// samples per stochastic pass, and in all
    samples_hw: u32,
    samples: u32,
    flags: u32,
    radius: u32,
    sigma_alpha: f32,
    temporal_weight: f32,
    /// samples of the scene (the total alpha's target)
    trans_samples: u32,
    _pad: [u32; 3],
    /// this frame's clip space to the last frame's (frame.motion_reproj)
    reproj: [[f32; 4]; 4],
}

/// The scene module's pipelines of the three passes over the faces (made
/// with the scene's, renderer.rs `make_pipelines`).
pub(crate) struct ScenePipes {
    /// total alpha, by sidedness
    pub alpha: [wgpu::RenderPipeline; 2],
    /// stochastic depth, by sidedness
    pub depth: [wgpu::RenderPipeline; 2],
    /// accumulation, by sidedness
    pub accum: [wgpu::RenderPipeline; 2],
    /// the glowing faces' glow accumulated (no culling: fs_glow's own test
    /// of the backs, as the Legacy glow pass)
    pub glow: wgpu::RenderPipeline,
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
    mask_shaders: Option<std::sync::Arc<(crate::spirv_fix::SampleMaskShaders, Vec<Vec<wgpu::BindGroupLayoutEntry>>)>>,
    /// the sample counts the device takes for the stochastic passes'
    /// targets (the colour and the depth formats both)
    pass_supported: Vec<u32>,
    /// the opaque depth spread over a pass's samples, from a multisampled
    /// scene depth and from a single-sampled one
    expand_layout: wgpu::BindGroupLayout,
    expand_layout_ss: wgpu::BindGroupLayout,
    expand_module: wgpu::ShaderModule,
}

/// The resolve's pipelines for one sample count of the scene.
pub(crate) struct Resolve {
    samples: u32,
    resolve: wgpu::ComputePipeline,
    filter: wgpu::ComputePipeline,
    composite: wgpu::RenderPipeline,
    /// the blended faces' glow added to the scene alpha
    composite_glow: wgpu::RenderPipeline,
    /// the opaque depth spread over the pass's samples (more than the
    /// scene's only)
    expand: Option<wgpu::RenderPipeline>,
}

impl Resolve {
    pub fn samples(&self) -> u32 {
        self.samples
    }
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
    /// per parity of the frame: the resolve's, the filter's and the
    /// composite's groups
    groups: [(wgpu::BindGroup, wgpu::BindGroup, wgpu::BindGroup); 2],
}

pub(crate) struct Stochastic {
    shared: Shared,
    /// the configuration in force (its temporal weight 0 under TAA or FSR)
    config: Config,
    /// the scene history the last `prepare` saw (its log line on a change)
    scene_history: SceneHistory,
    /// the temporal weight the environment and the defaults ask for
    wanted_temporal: f32,
    /// the mask table for `config.samples` and the blue-noise tile (bound
    /// by `face_groups`)
    _masks: wgpu::Buffer,
    _blue: wgpu::Buffer,
    /// one StParams per iteration
    params: Vec<wgpu::Buffer>,
    face_groups: Vec<wgpu::BindGroup>,
    resolve_params: wgpu::Buffer,
    /// oit_stochastic_expand.wgsl `ExpandParams`
    expand_params: wgpu::Buffer,
    targets: Option<Targets>,
    frame: u32,
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

impl Stochastic {
    /// `pass_supported`: the sample counts the device takes for the passes'
    /// formats (the stochastic passes' choice, `pass_samples`; `prepare`
    /// declines a frame when it holds no multisampled count).
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        hdr_format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
        pass_supported: Vec<u32>,
    ) -> Self {
        use wgpu::ShaderStages as S;
        let config = config();
        let face_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("oit stochastic faces"),
            entries: &[
                entry(30, S::FRAGMENT, buffer(wgpu::BufferBindingType::Storage { read_only: true })),
                entry(31, S::FRAGMENT, buffer(wgpu::BufferBindingType::Uniform)),
                entry(32, S::FRAGMENT, buffer(wgpu::BufferBindingType::Storage { read_only: true })),
            ],
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
                entry(8, S::COMPUTE, storage(wgpu::TextureFormat::Rgba16Float)),
                entry(9, all, texture(false)),
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
        let expand_params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("oit stochastic expand parameters"),
            size: u64::from(MAX_PASS_SAMPLES * 4),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let table = mask_table(config.samples);
        let masks = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("oit stochastic masks"),
            size: (table.len() * 4) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&masks, 0, bytemuck::cast_slice(&table));
        let blue_table: Vec<u32> = crate::oit_stochastic_blue::BLUE_NOISE_64.iter().map(|&v| u32::from(v)).collect();
        let blue = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("oit stochastic blue noise"),
            size: (blue_table.len() * 4) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&blue, 0, bytemuck::cast_slice(&blue_table));
        let params: Vec<wgpu::Buffer> = (0..MAX_SAMPLES)
            .map(|_| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("oit stochastic parameters"),
                    size: std::mem::size_of::<StParams>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            })
            .collect();
        let face_groups = params
            .iter()
            .map(|p| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("oit stochastic faces"),
                    layout: &face_layout,
                    entries: &[
                        bind(30, masks.as_entire_binding()),
                        bind(31, p.as_entire_binding()),
                        bind(32, blue.as_entire_binding()),
                    ],
                })
            })
            .collect();
        let resolve_params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("oit stochastic resolve parameters"),
            size: std::mem::size_of::<ResolveParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        log::info!("oit stochastic: {config:?}; pass sample counts the device takes: {pass_supported:?}");
        Stochastic {
            shared: Shared {
                face_layout,
                resolve_layout,
                module,
                hdr_format,
                depth_format,
                mask_shaders: None,
                pass_supported,
                expand_layout,
                expand_layout_ss,
                expand_module,
            },
            wanted_temporal: config.temporal,
            scene_history: SceneHistory::None,
            config,
            _masks: masks,
            _blue: blue,
            params,
            face_groups,
            resolve_params,
            expand_params,
            targets: None,
            frame: 0,
            glow: false,
        }
    }

    pub fn shared(&self) -> &Shared {
        &self.shared
    }

    /// The sample mask route's shader maker and the scene's group layouts
    /// (groups 0 to 2; group 3 is `face_layout`).
    pub fn set_mask_shaders(&mut self, shaders: crate::spirv_fix::SampleMaskShaders, scene_groups: Vec<Vec<wgpu::BindGroupLayoutEntry>>) {
        self.shared.mask_shaders = Some(std::sync::Arc::new((shaders, scene_groups)));
    }

    /// Runs of the passes 3 and 4 for `pass` samples per pass.
    fn iterations_for(&self, pass: u32) -> u32 {
        self.config.samples.div_ceil(pass.max(1)).clamp(1, MAX_SAMPLES)
    }

    /// Runs of the passes 3 and 4 this frame (after `prepare`).
    pub fn iterations(&self) -> u32 {
        self.targets.as_ref().map_or(1, |t| self.iterations_for(t.pass))
    }

    /// The frame's targets and parameters, for a scene of `samples` (1
    /// without MSAA). `false` when the device has no multisampled pass
    /// target (fewer than 2 samples). `glow`:
    /// glowing blended faces will be accumulated (`begin_glow`).
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        samples: u32,
        reproj: [[f32; 4]; 4],
        glow: bool,
        scene_history: SceneHistory,
    ) -> bool {
        let pass = self.shared.pass_samples(samples);
        if pass < 2 {
            return false;
        }
        // the scene's TAA accumulates already: a second history would ghost
        // twice, so the blend (and its seeds changing every frame) is off
        // there; on under FSR (`temporal_weight`)
        let temporal = temporal_weight(self.wanted_temporal, scene_history);
        if temporal != self.config.temporal || scene_history != self.scene_history {
            let why = match scene_history {
                SceneHistory::Taa => "off, the scene's TAA accumulates",
                _ if temporal == 0.0 => "off",
                SceneHistory::Fsr => "on, before FSR (its reactive mask leaves the blended faces to this frame)",
                SceneHistory::None => "on",
            };
            log::info!("oit stochastic: temporal blend {why}");
            self.config.temporal = temporal;
        }
        self.scene_history = scene_history;
        let history = plan(&self.config).history;
        let stale = self
            .targets
            .as_ref()
            .is_none_or(|t| t.width != width || t.height != height || t.samples != samples || t.pass != pass || t.history != history);
        if stale {
            self.targets = Some(self.make_targets(device, width, height, samples, pass));
            queue.write_buffer(&self.expand_params, 0, bytemuck::cast_slice(&expand_map(samples, pass)));
        }
        self.frame = self.frame.wrapping_add(1);
        let c = self.config;
        let iterations = self.iterations_for(pass);
        let flags = u32::from(c.stratified) | (u32::from(c.temporal > 0.0) << 1) | (u32::from(!c.prim_seed) << 2);
        for (i, p) in self.params.iter().enumerate().take(iterations as usize) {
            let sp = StParams {
                samples_hw: pass,
                samples: c.samples,
                iteration: i as u32,
                flags,
                frame_seed: self.frame.wrapping_mul(0x9e37_79b9),
                seed_mode: c.seed as u32,
                _pad: [0; 2],
            };
            queue.write_buffer(p, 0, bytemuck::bytes_of(&sp));
        }
        let rp = ResolveParams {
            width,
            height,
            samples_hw: pass,
            samples: pass * iterations,
            flags: u32::from(c.correction)
                | (u32::from(c.filter > 0) << 1)
                | (u32::from(c.temporal > 0.0) << 2)
                | (u32::from(glow) << 3)
                | (u32::from(samples == 1) << 4),
            radius: c.filter,
            sigma_alpha: c.sigma_alpha,
            temporal_weight: c.temporal,
            trans_samples: trans_samples(samples, pass),
            _pad: [0; 3],
            reproj,
        };
        queue.write_buffer(&self.resolve_params, 0, bytemuck::bytes_of(&rp));
        self.glow = glow;
        true
    }

    fn make_targets(&self, device: &wgpu::Device, width: u32, height: u32, samples: u32, pass: u32) -> Targets {
        let make = |label, format, samples, usage| {
            let (tex, view) = crate::renderer::make_view(device, label, width, height, format, samples, usage);
            (tex, view)
        };
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
        // A texture written as storage and read as sampled in one group
        // conflicts: each step's group binds placeholders where it does not
        // write (two of each format, one read, one written). No entry point
        // touches them: 1 x 1 (`plan`)
        let small = |label, format| crate::renderer::make_view(device, label, 1, 1, format, 1, single).1;
        let (dr_r, dr_w) = (
            small("oit stochastic dummy rgba r", wgpu::TextureFormat::Rgba16Float),
            small("oit stochastic dummy rgba w", wgpu::TextureFormat::Rgba16Float),
        );
        let (df_r, df_w) = (
            small("oit stochastic dummy rg32 r", wgpu::TextureFormat::Rg32Float),
            small("oit stochastic dummy rg32 w", wgpu::TextureFormat::Rg32Float),
        );
        let plan = plan(&self.config);
        // the resolved colour: the last frame's is read by the temporal
        // blend alone (otherwise one, the placeholder as its history)
        let resolved0 = make("oit stochastic resolved 0", wgpu::TextureFormat::Rgba16Float, 1, single).1;
        let resolved1 = plan
            .history
            .then(|| make("oit stochastic resolved 1", wgpu::TextureFormat::Rgba16Float, 1, single).1);
        // the total alpha (red) and the corrected glow (green)
        let (_, total) = make("oit stochastic total alpha", wgpu::TextureFormat::Rg32Float, 1, single);
        // the filtered colour: written and read with the filter alone
        let filtered = if plan.filtered {
            make("oit stochastic filtered", wgpu::TextureFormat::Rgba16Float, 1, single).1
        } else {
            small("oit stochastic filtered (off)", wgpu::TextureFormat::Rgba16Float)
        };
        let groups = std::array::from_fn(|parity| {
            let (cur, old) = match &resolved1 {
                Some(r1) if parity == 1 => (r1, &resolved0),
                Some(r1) => (&resolved0, r1),
                None => (&resolved0, &dr_r),
            };
            let group = |label, entries: [(u32, &wgpu::TextureView); 7]| {
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
                [(3, cur), (4, &total), (5, old), (6, &dr_r), (7, &df_r), (8, &dr_w), (9, &dr_r)],
            );
            // writes filtered, reads cur and total
            let filter = group(
                "oit stochastic filter",
                [
                    (3, &dr_w),
                    (4, &df_w),
                    (5, &dr_r),
                    (6, cur),
                    (7, &total),
                    (8, &filtered),
                    (9, &dr_r),
                ],
            );
            // reads cur, total and filtered
            let composite = group(
                "oit stochastic composite",
                [
                    (3, &dr_w),
                    (4, &df_w),
                    (5, &dr_r),
                    (6, cur),
                    (7, &total),
                    (8, &dr_w),
                    (9, &filtered),
                ],
            );
            (resolve, filter, composite)
        });
        log::info!(
            "oit stochastic: targets {width}x{height}, the scene at {samples} samples, the passes at {pass}: {} iterations for {} samples, {:.0} MB",
            self.iterations_for(pass),
            self.config.samples,
            target_bytes(width, height, samples, pass, plan) as f64 / (1024.0 * 1024.0)
        );
        Targets {
            width,
            height,
            samples,
            pass,
            history: plan.history,
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
            log::info!("oit stochastic: off, targets freed");
        }
    }

    fn parity(&self) -> usize {
        (self.frame & 1) as usize
    }

    /// Group 3 of the alpha and depth passes for iteration `i`.
    pub fn face_group(&self, i: u32) -> &wgpu::BindGroup {
        &self.face_groups[(i as usize).min(self.face_groups.len() - 1)]
    }

    /// The total alpha runs against the stochastic depth (the scene without
    /// MSAA): the first iteration's `begin_iteration` comes before it, to
    /// spread the opaque depth over the passes' samples.
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

    /// Before each iteration: the opaque depth copied into the stochastic
    /// depth (spread over its samples when it has more); before the first,
    /// the accumulation cleared.
    pub fn begin_iteration(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        scene_depth: (&wgpu::Texture, &wgpu::TextureView),
        r: &Resolve,
        first: bool,
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
        if first {
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

    /// The glow accumulation of an iteration: additive into the glow
    /// target (cleared on the first), the stochastic depth read only.
    pub fn begin_glow<'a>(&'a self, encoder: &'a mut wgpu::CommandEncoder, first: bool) -> Option<wgpu::RenderPass<'a>> {
        let t = self.targets.as_ref()?;
        Some(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("oit stochastic glow"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &t.glow,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: if first {
                        wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                    } else {
                        wgpu::LoadOp::Load
                    },
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

    /// The resolve (and the filter when on).
    pub fn resolve(&self, encoder: &mut wgpu::CommandEncoder, r: &Resolve) {
        let Some(t) = &self.targets else { return };
        let (resolve, filter, _) = &t.groups[self.parity()];
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("oit stochastic resolve"),
            timestamp_writes: None,
        });
        let (x, y) = (t.width.div_ceil(8), t.height.div_ceil(8));
        pass.set_pipeline(&r.resolve);
        pass.set_bind_group(0, resolve, &[]);
        pass.dispatch_workgroups(x, y, 1);
        if self.config.filter > 0 {
            pass.set_pipeline(&r.filter);
            pass.set_bind_group(0, filter, &[]);
            pass.dispatch_workgroups(x, y, 1);
        }
    }

    /// The composite over the scene, in a scene pass (per sample).
    pub fn composite(&self, pass: &mut wgpu::RenderPass, r: &Resolve) {
        let Some(t) = &self.targets else { return };
        pass.set_pipeline(&r.composite);
        pass.set_bind_group(0, &t.groups[self.parity()].2, &[]);
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
        pass_samples(scene, config().pass_samples, &self.pass_supported)
    }

    /// The scene module for the sample mask route: `src` (the scene's WGSL)
    /// with its overrides turned into constants (`values` for the ones the
    /// stochastic depth pipeline sets, the declared defaults for the rest:
    /// a passthrough module cannot be specialised), created through
    /// spirv_fix against the scene's layouts and `face_layout`. `None` when
    /// it cannot be made right (Vulkan without the fix): the index route
    /// then.
    pub fn mask_module(&self, device: &wgpu::Device, src: &str, values: &[(&str, &str)]) -> Option<wgpu::ShaderModule> {
        let wgsl = constants_for_overrides(src, values);
        let face = [
            layout_entry(30, wgpu::BufferBindingType::Storage { read_only: true }),
            layout_entry(31, wgpu::BufferBindingType::Uniform),
            layout_entry(32, wgpu::BufferBindingType::Storage { read_only: true }),
        ];
        match &self.mask_shaders {
            Some(ms) => {
                let (shaders, scene) = &**ms;
                let mut groups: Vec<&[wgpu::BindGroupLayoutEntry]> = scene.iter().map(|g| g.as_slice()).collect();
                groups.push(&face);
                // the scene source declares more resources than this
                // pipeline binds (the lists' group 3, the impostors'...):
                // naga wants a mapping for each, they get bindings past the
                // real ones (the entry points never use them)
                let mut map = crate::spirv_fix::binding_map(&groups);
                let mut next = 64u32;
                for (group, binding) in declared_bindings(&wgsl) {
                    map.entry(naga::ResourceBinding { group, binding }).or_insert_with(|| {
                        next += 1;
                        naga::back::spv::BindingInfo {
                            descriptor_set: group,
                            binding: next,
                            binding_array_size: None,
                        }
                    });
                }
                shaders.try_create_shader_module_mapped(device, "scene stochastic mask", &wgsl, map)
            }
            None => None,
        }
    }
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
        if after[..b_at].trim() != "" {
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

fn layout_entry(binding: u32, ty: wgpu::BufferBindingType) -> wgpu::BindGroupLayoutEntry {
    entry(binding, wgpu::ShaderStages::FRAGMENT, buffer(ty))
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
    let compute = |label, entry_point| {
        device.create_compute_pipeline_cached(&wgpu::ComputePipelineDescriptor {
            label: Some(label),
            layout: Some(&layout),
            module: &shared.module,
            entry_point: Some(entry_point),
            compilation_options: Default::default(),
            cache: None,
        })
    };
    let composite = device.create_render_pipeline_cached(&wgpu::RenderPipelineDescriptor {
        label: Some("oit stochastic composite"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shared.module,
            entry_point: Some("vs_full"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shared.module,
            entry_point: Some("fs_composite"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: shared.hdr_format,
                // the scene sample dimmed by its transparency, the colour
                // added; the glow (scene alpha) dimmed the same way
                blend: Some(wgpu::BlendState {
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
                }),
                write_mask: wgpu::ColorWrites::ALL,
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
    });
    let composite_glow = device.create_render_pipeline_cached(&wgpu::RenderPipelineDescriptor {
        label: Some("oit stochastic composite glow"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shared.module,
            entry_point: Some("vs_full"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shared.module,
            entry_point: Some("fs_composite_glow"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: shared.hdr_format,
                // added to the glow amount (the scene alpha) alone
                blend: Some(wgpu::BlendState {
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
                }),
                write_mask: wgpu::ColorWrites::ALPHA,
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
    });
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
        samples,
        resolve: compute("oit stochastic resolve", "cs_resolve"),
        filter: compute("oit stochastic filter", "cs_filter"),
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
        for s in [8, 16, 32] {
            let t = mask_table(s);
            assert_eq!(t.len(), ((s + 1) * SEEDS) as usize);
            for r in 0..=s {
                for seed in 0..SEEDS {
                    let m = t[(r * SEEDS + seed) as usize];
                    assert_eq!(m.count_ones(), r, "S {s} R {r} seed {seed}");
                    if s < 32 {
                        assert_eq!(m >> s, 0);
                    }
                }
            }
            // not the same subset for every seed
            let row = &t[(s / 2 * SEEDS) as usize..((s / 2 + 1) * SEEDS) as usize];
            assert!(row.iter().any(|&m| m != row[0]));
        }
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
        // Before, the index was (pixel value + layer offset) mod 1: two layers
        // whose offsets met had the same subset at every pixel. Now the index
        // is hashed per pixel and layer: any two seeds, however close, differ
        // at most pixels, and each layer's indices stay inside the table.
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
        let t = mask_table(16);
        for i in 0..64 {
            assert_eq!(t[(5 * SEEDS + subset_index(a, i, 3)) as usize].count_ones(), 5);
        }
    }

    #[test]
    fn the_pass_takes_the_most_samples_up_to_8() {
        let all = [2, 4, 8, 16];
        assert_eq!(pass_samples(4, 0, &all), 8);
        assert_eq!(pass_samples(4, 16, &all), 16);
        assert_eq!(pass_samples(4, 4, &all), 4);
        // never fewer than the scene's, nor a count the device lacks
        assert_eq!(pass_samples(8, 2, &all), 8);
        assert_eq!(pass_samples(4, 0, &[4]), 4);
        assert_eq!(pass_samples(2, 16, &[2, 4]), 4);
    }

    #[test]
    fn each_pass_sample_takes_the_nearest_scene_sample() {
        // 8 over 4: two pass samples to each scene sample
        let m = expand_map(4, 8);
        assert_eq!(&m[..8], &[0, 3, 1, 0, 2, 2, 3, 1]);
        // the same count: each its own
        assert_eq!(&expand_map(4, 4)[..4], &[0, 1, 2, 3]);
        for n in [2, 4, 8] {
            let m = expand_map(n, 16);
            assert!(m.iter().all(|&k| k < n));
            for k in 0..n {
                assert!(m.contains(&k), "{n}: scene sample {k} unused");
            }
        }
    }

    #[test]
    fn the_targets_hold_only_what_the_configuration_reads() {
        let mib = |w, h, s, plan| target_bytes(w, h, s, 8, plan) as f64 / (1024.0 * 1024.0);
        // the default keeps the last frame's colour for the temporal blend
        let default = plan(&Config::default());
        assert_eq!(
            default,
            Plan {
                history: true,
                filtered: false
            }
        );
        let off = plan(&parse_config(None, Some("notemporal")));
        assert_eq!(
            off,
            Plan {
                history: false,
                filtered: false
            }
        );
        // without the history, MSAA 4x: 136 bytes a pixel (the earlier layout,
        // with the second resolved colour, the filtered one and the full-size
        // placeholders, took 184: 253 and 647 MB); the history adds 8
        assert_eq!((mib(1600, 900, 4, off) + 0.5) as u32, 187);
        assert_eq!((mib(2560, 1440, 4, off) + 0.5) as u32, 478);
        assert_eq!((mib(1600, 900, 4, default) + 0.5) as u32, 198);
        assert_eq!((mib(2560, 1440, 4, default) + 0.5) as u32, 506);
        // without MSAA the total alpha is at the passes' samples
        assert_eq!((mib(1600, 900, 1, off) + 0.5) as u32, 198);
        let all = Plan {
            history: true,
            filtered: true,
        };
        assert_eq!(plan(&parse_config(None, Some("temporal,filter"))), all);
        assert_eq!((mib(2560, 1440, 4, all) + 0.5) as u32, 534);
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
    fn the_flags_parse() {
        let c = parse_config(None, None);
        assert_eq!(c, Config::default());
        let c = parse_config(Some("16"), Some("naive,nocorr,filter=3:0.2,temporal=4,index,noprim"));
        assert_eq!(c.samples, 16);
        assert!(!c.stratified && !c.correction && !c.prim_seed);
        assert_eq!((c.filter, c.sigma_alpha, c.temporal, c.route), (3, 0.2, 0.25, Route::SampleIndex));
        assert_eq!(parse_config(None, None).route, Route::SampleMask);
        assert_eq!(parse_config(None, Some("seed=blue")).seed, Seed::Blue);
        assert_eq!(method(true), Method::Stochastic);
        assert_eq!(method(false), Method::Off);
        assert_eq!(parse_config(None, Some("seed=r2")).seed, Seed::R2);
        assert_eq!(parse_config(Some("32"), Some("filter")).filter, 2);
        assert_eq!(parse_config(Some("12"), None).samples, 16);
        assert_eq!(parse_config(None, None).samples, 8);
        assert_eq!(parse_config(Some("16"), None).samples, 16);
        // the temporal blend is the default; `notemporal` turns it off
        assert_eq!(parse_config(None, None).temporal, 0.125);
        assert_eq!(parse_config(None, Some("notemporal")).temporal, 0.0);
    }

    #[test]
    fn the_temporal_blend_is_off_under_taa_only() {
        let wanted = parse_config(None, None).temporal;
        assert_eq!(temporal_weight(wanted, SceneHistory::None), 0.125);
        assert_eq!(temporal_weight(wanted, SceneHistory::Taa), 0.0);
        assert_eq!(temporal_weight(wanted, SceneHistory::Fsr), 0.125);
        assert_eq!(temporal_weight(0.0, SceneHistory::Fsr), 0.0);
        assert_eq!(temporal_weight(0.0, SceneHistory::None), 0.0);
    }

    #[test]
    fn the_shaders_are_valid() {
        for src in [
            include_str!("shaders/oit_stochastic.wgsl"),
            include_str!("shaders/oit_stochastic_expand.wgsl"),
        ] {
            let module = naga::front::wgsl::parse_str(src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(src)));
            let mut v = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all());
            if let Err(e) = v.validate(&module) {
                panic!("{}", e.emit_to_string(src));
            }
        }
        let object = include_str!("shaders/object.wgsl").replace("\r\n", "\n");
        for prim in [false, true] {
            let generated = object_source_with(&object, prim);
            // (the parameter marker always goes; the value marker stays as a
            // comment when the triangle index is not read)
            assert!(!generated.contains("/*ST_PRIM_PARAM*/"));
            assert_eq!(generated.contains("prim_id"), prim);
            let src = format!(
                "{}{}\n{}\n{}",
                enable_directive_with(prim),
                crate::COMMON_SHADER,
                generated,
                include_str!("shaders/env.wgsl")
            );
            let module = naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&src)));
            let mut v = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all());
            if let Err(e) = v.validate(&module) {
                panic!("prim {prim}: {}", e.emit_to_string(&src));
            }
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
            object_source_with(&object, true),
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
