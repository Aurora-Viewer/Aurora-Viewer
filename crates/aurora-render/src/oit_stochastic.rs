//! Order-independent transparency by depth-based stochastic transparency
//! (the OIT setting; `AURORA_OIT`, `AURORA_OIT_SAMPLES` and
//! `AURORA_OIT_STOCH` in docs/TESTING.md). An Aurora addition: the Legacy
//! Viewer sorts its alpha by object (LLDrawPoolAlpha).
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
//! More samples than the scene's targets hold (`AURORA_OIT_SAMPLES`): the
//! passes 3 and 4 run again with the next bits of one stratified mask
//! (paper footnote 1). The masks come from a table of random S-choose-R
//! subsets (`masks`), fixed for every launch; the seed is the pixel, the
//! face's tie-break and the triangle. Needs MSAA (the targets are
//! multisampled), and `PRIMITIVE_INDEX` for the triangle in the
//! seed (without it the seed is per face: coplanar fragments of one face
//! then occlude instead of blending).

use crate::pipeline_cache::CachedPipelines;
use bytemuck::{Pod, Zeroable};

/// Seeds per alpha level in the mask table (the paper's width, 2 047).
pub const SEEDS: u32 = 2048;
pub const MAX_SAMPLES: u32 = 32;

/// How the coverage mask reaches the samples (the known blocker of the
/// `@builtin(sample_mask)` output on naga's SPIR-V).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Route {
    /// one invocation per sample (`@builtin(sample_index)`), the samples
    /// off the mask discarded
    SampleIndex,
    /// the mask written once per fragment (`@builtin(sample_mask)`)
    SampleMask,
}

/// `AURORA_OIT=stochastic`, `AURORA_OIT_SAMPLES`, `AURORA_OIT_STOCH`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Config {
    /// samples per pixel in all: 8, 16 or 32 (16 by default: the paper
    /// finds 8 pleasant on hair and foliage, 16 adequate on broad smooth
    /// surfaces)
    pub samples: u32,
    pub stratified: bool,
    pub correction: bool,
    /// the cross-bilateral filter's radius (0: off) and alpha sigma
    pub filter: u32,
    pub sigma_alpha: f32,
    /// the temporal blend's weight of the current frame (0: off)
    pub temporal: f32,
    pub route: Route,
    /// the triangle in the seed
    pub prim_seed: bool,
    /// the per-pixel part of the seed
    pub seed: Seed,
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

impl Default for Config {
    fn default() -> Self {
        Config {
            samples: 16,
            stratified: true,
            correction: true,
            filter: 0,
            sigma_alpha: 0.1,
            temporal: 0.0,
            route: Route::SampleIndex,
            prim_seed: true,
            seed: Seed::White,
        }
    }
}

/// How the blended faces are drawn: the OIT setting on means stochastic
/// transparency, off the Legacy Viewer's per-object order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Method {
    /// the Legacy Viewer's per-object order
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

/// `samples`: 8, 16 or 32. `flags`, comma separated: `naive` (one coin per
/// sample instead of a stratified mask), `nocorr` (no alpha correction),
/// `filter=<radius>[:<sigma>]`, `temporal[=<frames>]` (the resolved colour
/// blended with the last frame's reprojected through the nearest
/// transparent depth, weight 1/frames, the seeds changing every frame),
/// `mask` (the sample mask route), `noprim` (the triangle left out of the
/// seed), `seed=white|r2|blue` (the per-pixel part of the seed).
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
            "temporal" => c.temporal = 1.0 / value.parse::<f32>().unwrap_or(8.0).clamp(1.0, 64.0),
            "mask" => c.route = Route::SampleMask,
            "index" => c.route = Route::SampleIndex,
            "noprim" => c.prim_seed = false,
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
    samples_hw: u32,
    samples: u32,
    flags: u32,
    radius: u32,
    sigma_alpha: f32,
    temporal_weight: f32,
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
}

/// The resolve's pipelines for one sample count.
pub(crate) struct Resolve {
    samples: u32,
    resolve: wgpu::ComputePipeline,
    filter: wgpu::ComputePipeline,
    composite: wgpu::RenderPipeline,
}

impl Resolve {
    pub fn samples(&self) -> u32 {
        self.samples
    }
}

struct Targets {
    width: u32,
    height: u32,
    samples: u32,
    trans: wgpu::TextureView,
    accum: wgpu::TextureView,
    depth_tex: wgpu::Texture,
    depth: wgpu::TextureView,
    /// per parity of the frame: the resolve's, the filter's and the
    /// composite's groups
    groups: [(wgpu::BindGroup, wgpu::BindGroup, wgpu::BindGroup); 2],
}

pub(crate) struct Stochastic {
    shared: Shared,
    config: Config,
    /// the mask table for `config.samples` and the blue-noise tile (bound
    /// by `face_groups`)
    _masks: wgpu::Buffer,
    _blue: wgpu::Buffer,
    /// one StParams per iteration
    params: Vec<wgpu::Buffer>,
    face_groups: Vec<wgpu::BindGroup>,
    resolve_params: wgpu::Buffer,
    targets: Option<Targets>,
    frame: u32,
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
    /// `None` when the device cannot (no MSAA targets are checked here: the
    /// frame checks the scene's samples).
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, hdr_format: wgpu::TextureFormat, depth_format: wgpu::TextureFormat) -> Self {
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
                entry(4, S::COMPUTE, storage(wgpu::TextureFormat::R32Float)),
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
            ],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("oit stochastic"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/oit_stochastic.wgsl").into()),
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
        log::info!("oit stochastic: {config:?}");
        Stochastic {
            shared: Shared {
                face_layout,
                resolve_layout,
                module,
                hdr_format,
                depth_format,
                mask_shaders: None,
            },
            config,
            _masks: masks,
            _blue: blue,
            params,
            face_groups,
            resolve_params,
            targets: None,
            frame: 0,
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

    /// Runs of the passes 3 and 4 for `samples_hw` samples per target.
    pub fn iterations(&self, samples_hw: u32) -> u32 {
        self.config.samples.div_ceil(samples_hw.max(1)).clamp(1, MAX_SAMPLES)
    }

    /// The frame's targets and parameters. `false` without MSAA.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        samples: u32,
        reproj: [[f32; 4]; 4],
    ) -> bool {
        if samples <= 1 {
            return false;
        }
        let stale = self
            .targets
            .as_ref()
            .is_none_or(|t| t.width != width || t.height != height || t.samples != samples);
        if stale {
            self.targets = Some(self.make_targets(device, width, height, samples));
        }
        self.frame = self.frame.wrapping_add(1);
        let c = self.config;
        let flags = u32::from(c.stratified) | (u32::from(c.temporal > 0.0) << 1) | (u32::from(!c.prim_seed) << 2);
        for (i, p) in self.params.iter().enumerate().take(self.iterations(samples) as usize) {
            let sp = StParams {
                samples_hw: samples,
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
            samples_hw: samples,
            samples: samples * self.iterations(samples),
            flags: u32::from(c.correction) | (u32::from(c.filter > 0) << 1) | (u32::from(c.temporal > 0.0) << 2),
            radius: c.filter,
            sigma_alpha: c.sigma_alpha,
            temporal_weight: c.temporal,
            reproj,
        };
        queue.write_buffer(&self.resolve_params, 0, bytemuck::bytes_of(&rp));
        true
    }

    fn make_targets(&self, device: &wgpu::Device, width: u32, height: u32, samples: u32) -> Targets {
        let make = |label, format, samples, usage| {
            let (tex, view) = crate::renderer::make_view(device, label, width, height, format, samples, usage);
            (tex, view)
        };
        use wgpu::TextureUsages as U;
        let (_, trans) = make(
            "oit stochastic transmittance",
            wgpu::TextureFormat::R16Float,
            samples,
            U::RENDER_ATTACHMENT | U::TEXTURE_BINDING,
        );
        let (_, accum) = make(
            "oit stochastic accumulation",
            self.shared.hdr_format,
            samples,
            U::RENDER_ATTACHMENT | U::TEXTURE_BINDING,
        );
        let (depth_tex, depth) = make(
            "oit stochastic depth",
            self.shared.depth_format,
            samples,
            U::RENDER_ATTACHMENT | U::COPY_DST | U::TEXTURE_BINDING,
        );
        let single = U::STORAGE_BINDING | U::TEXTURE_BINDING;
        let resolved = [
            make("oit stochastic resolved 0", wgpu::TextureFormat::Rgba16Float, 1, single).1,
            make("oit stochastic resolved 1", wgpu::TextureFormat::Rgba16Float, 1, single).1,
        ];
        let (_, total) = make("oit stochastic total alpha", wgpu::TextureFormat::R32Float, 1, single);
        let (_, filtered) = make("oit stochastic filtered", wgpu::TextureFormat::Rgba16Float, 1, single);
        // A texture written as storage and read as sampled in one group
        // conflicts: each step's group binds dummies where it does not
        // write (two of each format, one read, one written)
        let dummy = |label, format| make(label, format, 1, single).1;
        let (dr_r, dr_w) = (
            dummy("oit stochastic dummy rgba r", wgpu::TextureFormat::Rgba16Float),
            dummy("oit stochastic dummy rgba w", wgpu::TextureFormat::Rgba16Float),
        );
        let (df_r, df_w) = (
            dummy("oit stochastic dummy r32 r", wgpu::TextureFormat::R32Float),
            dummy("oit stochastic dummy r32 w", wgpu::TextureFormat::R32Float),
        );
        let groups = std::array::from_fn(|parity| {
            let (cur, old) = (&resolved[parity], &resolved[1 - parity]);
            let group = |label, entries: [(u32, &wgpu::TextureView); 7]| {
                let mut all = vec![
                    bind(0, self.resolve_params.as_entire_binding()),
                    bind(1, wgpu::BindingResource::TextureView(&accum)),
                    bind(2, wgpu::BindingResource::TextureView(&trans)),
                    bind(10, wgpu::BindingResource::TextureView(&depth)),
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
            "oit stochastic: targets {width}x{height} at {samples} samples, {} iterations for {} samples",
            self.iterations(samples),
            self.config.samples
        );
        Targets {
            width,
            height,
            samples,
            trans,
            accum,
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

    /// The pass of the total alpha: the target cleared to full
    /// transmittance, the scene depth read only.
    pub fn begin_alpha<'a>(
        &'a self,
        encoder: &'a mut wgpu::CommandEncoder,
        scene_depth: &'a wgpu::TextureView,
    ) -> Option<wgpu::RenderPass<'a>> {
        let t = self.targets.as_ref()?;
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
                view: scene_depth,
                depth_ops: None,
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        }))
    }

    /// Before each iteration: the opaque depth copied into the stochastic
    /// depth; before the first, the accumulation cleared.
    pub fn begin_iteration(&self, encoder: &mut wgpu::CommandEncoder, scene_depth: &wgpu::Texture, first: bool) {
        let Some(t) = &self.targets else { return };
        encoder.copy_texture_to_texture(scene_depth.as_image_copy(), t.depth_tex.as_image_copy(), scene_depth.size());
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
    }
}

impl Shared {
    /// The scene module for the sample mask route: `src` (the scene's WGSL)
    /// with its overrides turned into constants (`values` for the ones the
    /// stochastic depth pipeline sets, the declared defaults for the rest:
    /// a passthrough module cannot be specialised), created through
    /// spirv_fix against the scene's layouts and `face_layout`.
    pub fn mask_module(&self, device: &wgpu::Device, src: &str, values: &[(&str, &str)]) -> wgpu::ShaderModule {
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
                shaders.create_shader_module_mapped(device, "scene stochastic mask", &wgsl, map)
            }
            None => device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("scene stochastic mask"),
                source: wgpu::ShaderSource::Wgsl(wgsl.into()),
            }),
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
            let ty = rest[name.len()..ty_end].trim_start_matches(|c: char| c == ':' || c == ' ').trim();
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
    Resolve {
        samples,
        resolve: compute("oit stochastic resolve", "cs_resolve"),
        filter: compute("oit stochastic filter", "cs_filter"),
        composite,
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
        let c = parse_config(Some("16"), Some("naive,nocorr,filter=3:0.2,temporal=4,mask,noprim"));
        assert_eq!(c.samples, 16);
        assert!(!c.stratified && !c.correction && !c.prim_seed);
        assert_eq!((c.filter, c.sigma_alpha, c.temporal, c.route), (3, 0.2, 0.25, Route::SampleMask));
        assert_eq!(parse_config(None, Some("seed=blue")).seed, Seed::Blue);
        assert_eq!(method(true), Method::Stochastic);
        assert_eq!(method(false), Method::Off);
        assert_eq!(parse_config(None, Some("seed=r2")).seed, Seed::R2);
        assert_eq!(parse_config(Some("32"), Some("filter")).filter, 2);
        assert_eq!(parse_config(Some("12"), None).samples, 16);
        assert_eq!(parse_config(None, None).samples, 16);
    }

    #[test]
    fn the_shaders_are_valid() {
        let src = include_str!("shaders/oit_stochastic.wgsl");
        let module = naga::front::wgsl::parse_str(src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(src)));
        let mut v = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all());
        if let Err(e) = v.validate(&module) {
            panic!("{}", e.emit_to_string(src));
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
}
