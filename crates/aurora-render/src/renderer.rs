//! Frame orchestration through wgpu's Vulkan backend:
//!
//! 1. cascaded sun shadows
//! 2. single-sample depth prepass (shared by SSAO, water, SSR, particles, TAA)
//! 3. SSAO at half resolution (optional)
//! 4. planar reflections: water and the nearest mirror (optional, reduced resolution)
//! 5. scene pass A: sky, terrain, opaque and masked geometry (MSAA, HDR)
//! 6. copy of the resolved scene (refraction / SSR source)
//! 7. scene pass B: SSR, water, alpha-blended faces, particles
//! 8. TAA (optional), tonemapping, egui overlay

use crate::arena::{GeometryArena, MeshAlloc, RecordStore};
use crate::textures::{MipLevel, TextureTable};
use crate::types::*;
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3, Vec4};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

pub const CASCADES: usize = 3;
const MAX_LIGHTS: usize = 64;
const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const GBUF_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const AO_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const SHADOW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct LightU {
    pos_radius: [f32; 4],
    color_falloff: [f32; 4],
}

/// Must match `Frame` in common.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FrameU {
    view_proj: [[f32; 4]; 4],
    inv_view_proj: [[f32; 4]; 4],
    camera_pos: [f32; 4],
    sun_dir: [f32; 4],
    sun_color: [f32; 4],
    sky_zenith: [f32; 4],
    sky_horizon: [f32; 4],
    ground_color: [f32; 4],
    fog: [f32; 4],
    params: [f32; 4],
    cascade_splits: [f32; 4],
    screen: [f32; 4],
    misc: [f32; 4],
    clip_plane: [f32; 4],
    cam_right: [f32; 4],
    cam_up: [f32; 4],
    water_fog: [f32; 4],
    water_params: [f32; 4],
    water_waves: [f32; 4],
    water_normal: [f32; 4],
    mirror_plane: [f32; 4],
    mirror_box: [[f32; 4]; 4],
    sky_light: [f32; 4],
    sky_sunlight: [f32; 4],
    sky_ambient: [f32; 4],
    sky_blue_horizon: [f32; 4],
    sky_blue_density: [f32; 4],
    sky_glow: [f32; 4],
    sky_cloud_color: [f32; 4],
    sky_cloud_pd1: [f32; 4],
    sky_cloud_pd2: [f32; 4],
    sky_sun: [f32; 4],
    sky_moon: [f32; 4],
    sky_misc: [f32; 4],
    tex_slots: [u32; 4],
    sky_ll: [f32; 4],
    sky_obj_light: [f32; 4],
    cascade_vp: [[[f32; 4]; 4]; CASCADES],
    lights: [LightU; MAX_LIGHTS],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PostU {
    exposure: f32,
    output_srgb: f32,
    tonemapper: f32,
    fxaa: f32,
    sharpen: f32,
    glow: f32,
    debug_glow: f32,
    legacy_gamma: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GlowU {
    delta: [f32; 2],
    strength: f32,
    _pad: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SmaaU {
    rt: [f32; 4],
    output_srgb: f32,
    _pad: [f32; 3],
}

/// SMAA 1x (preset high) as Firestorm's RenderFSAAType 2: edge detection,
/// blending weights and neighborhood blending after the post pass, which
/// then renders into an 8-bit gamma-encoded image.
struct Smaa {
    layout: wgpu::BindGroupLayout,
    post_ldr: wgpu::RenderPipeline,
    edge: wgpu::RenderPipeline,
    weights: wgpu::RenderPipeline,
    blend: wgpu::RenderPipeline,
    area: wgpu::TextureView,
    search: wgpu::TextureView,
    sampler: wgpu::Sampler,
    buffer: wgpu::Buffer,
}

/// Screen-sized SMAA images and the bind group of each pass (the texture a
/// pass writes is replaced by `dummy` in its own bind group).
struct SmaaTargets {
    ldr: wgpu::TextureView,
    edges: wgpu::TextureView,
    weights: wgpu::TextureView,
    bgs: [wgpu::BindGroup; 3],
}

impl Smaa {
    const LDR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

    fn new(device: &wgpu::Device, queue: &wgpu::Queue, post_layout: &wgpu::BindGroupLayout, surface_format: wgpu::TextureFormat) -> Smaa {
        let usage = wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST;
        let (area_t, area) = make_view(device, "smaa area", 160, 560, wgpu::TextureFormat::Rg8Unorm, 1, usage);
        let (search_t, search) = make_view(device, "smaa search", 64, 16, wgpu::TextureFormat::R8Unorm, 1, usage);
        let upload = |t: &wgpu::Texture, bytes: &[u8], w: u32, h: u32, bpp: u32| {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: t,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(w * bpp),
                    rows_per_image: Some(h),
                },
                wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
            );
        };
        upload(&area_t, include_bytes!("smaa/area.bin"), 160, 560, 2);
        upload(&search_t, include_bytes!("smaa/search.bin"), 64, 16, 1);
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("smaa"),
            entries: &[
                float_tex(0),
                float_tex(1),
                float_tex(2),
                float_tex(3),
                float_tex(4),
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                uniform_entry(6, wgpu::ShaderStages::FRAGMENT),
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("smaa"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("smaa"),
            size: std::mem::size_of::<SmaaU>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let desc = |label: &'static str, vs: &'static str, fs: &'static str, format: wgpu::TextureFormat| PipeDesc {
            label,
            vs,
            fs: Some(fs),
            vb: Vb::None,
            cull: None,
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
            depth_write: false,
            depth_compare: wgpu::CompareFunction::Always,
            a2c: false,
            samples: 1,
            format: Some(format),
            gbuf: false,
            depth_format: None,
            depth_bias: Default::default(),
            constants: &[],
            polygon_line: false,
        };
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("smaa"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/smaa.wgsl").into()),
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("smaa"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let edge = make_pipeline(
            device,
            &module,
            &pl,
            desc("smaa edges", "vs_smaa", "fs_smaa_edge", wgpu::TextureFormat::Rg8Unorm),
        );
        let weights = make_pipeline(device, &module, &pl, desc("smaa weights", "vs_smaa", "fs_smaa_weights", Self::LDR));
        let blend = make_pipeline(device, &module, &pl, desc("smaa blend", "vs_smaa", "fs_smaa_blend", surface_format));
        let post_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("post ldr"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/post.wgsl").into()),
        });
        let post_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("post ldr"),
            bind_group_layouts: &[Some(post_layout)],
            immediate_size: 0,
        });
        let post_ldr = make_pipeline(device, &post_module, &post_pl, desc("post ldr", "vs_post", "fs_post", Self::LDR));
        Smaa {
            layout,
            post_ldr,
            edge,
            weights,
            blend,
            area,
            search,
            sampler,
            buffer,
        }
    }

    fn make_targets(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        dummy: &wgpu::TextureView,
        width: u32,
        height: u32,
        surface_srgb: bool,
    ) -> SmaaTargets {
        let (w, h) = (width.max(1), height.max(1));
        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let ldr = make_view(device, "smaa color", w, h, Self::LDR, 1, usage).1;
        let edges = make_view(device, "smaa edges", w, h, wgpu::TextureFormat::Rg8Unorm, 1, usage).1;
        let weights = make_view(device, "smaa weights", w, h, Self::LDR, 1, usage).1;
        let u = SmaaU {
            rt: [1.0 / w as f32, 1.0 / h as f32, w as f32, h as f32],
            output_srgb: if surface_srgb { 1.0 } else { 0.0 },
            _pad: [0.0; 3],
        };
        queue.write_buffer(&self.buffer, 0, bytemuck::bytes_of(&u));
        let bg = |color: &wgpu::TextureView, e: &wgpu::TextureView, b: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("smaa"),
                layout: &self.layout,
                entries: &[
                    tex(0, color),
                    tex(1, e),
                    tex(2, b),
                    tex(3, &self.area),
                    tex(4, &self.search),
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 6,
                        resource: self.buffer.as_entire_binding(),
                    },
                ],
            })
        };
        let bgs = [bg(&ldr, dummy, dummy), bg(&ldr, &edges, dummy), bg(&ldr, dummy, &weights)];
        SmaaTargets { ldr, edges, weights, bgs }
    }

    fn encode(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        t: &SmaaTargets,
        surface: &wgpu::TextureView,
        ts: Option<wgpu::RenderPassTimestampWrites>,
    ) {
        let passes: [(&wgpu::TextureView, &wgpu::RenderPipeline, &wgpu::BindGroup); 3] = [
            (&t.edges, &self.edge, &t.bgs[0]),
            (&t.weights, &self.weights, &t.bgs[1]),
            (surface, &self.blend, &t.bgs[2]),
        ];
        let mut ts = ts;
        for (i, (target, pipe, bg)) in passes.into_iter().enumerate() {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("smaa"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: if i == 2 { ts.take() } else { None },
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipe);
            pass.set_bind_group(0, bg, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

/// Screen-space glow as LLPipeline::generateGlow / combineGlow (originally LGPL 2.1,
/// Linden Research / Firestorm): the glow amount accumulated in the scene
/// alpha is extracted from the gamma-corrected image into 512x512 8-bit
/// targets, blurred RenderGlowIterations (2) x 2 times, then added.
struct Glow {
    /// [0] / [1] blur ping-pong (the result ends in [1]), [2] extraction.
    views: [wgpu::TextureView; 3],
    extract: wgpu::RenderPipeline,
    blur: wgpu::RenderPipeline,
    /// Blur sources: ([2], h), ([0], v), ([1], h); passes use 0, 1, 2, 1.
    blur_bgs: [wgpu::BindGroup; 3],
}

impl Glow {
    const SIZE: u32 = 512;
    /// RenderGlowWidth / glow resolution (RenderGlowResolutionPow 9).
    const DELTA: f32 = 1.3 / 512.0;
    /// RenderGlowStrength.
    const STRENGTH: f32 = 0.325;

    fn new(device: &wgpu::Device, post_layout: &wgpu::BindGroupLayout) -> Glow {
        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let fmt = wgpu::TextureFormat::Rgba8Unorm;
        let views = [0, 1, 2].map(|i| make_view(device, &format!("glow {i}"), Self::SIZE, Self::SIZE, fmt, 1, usage).1);
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("glow"),
            entries: &[
                float_tex(0),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                uniform_entry(2, wgpu::ShaderStages::FRAGMENT),
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("glow"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let ubuf = |delta: [f32; 2]| {
            let u = GlowU {
                delta,
                strength: Self::STRENGTH,
                _pad: 0.0,
            };
            wgpu::util::DeviceExt::create_buffer_init(
                device,
                &wgpu::util::BufferInitDescriptor {
                    label: Some("glow params"),
                    contents: bytemuck::bytes_of(&u),
                    usage: wgpu::BufferUsages::UNIFORM,
                },
            )
        };
        let h = ubuf([Self::DELTA, 0.0]);
        let v = ubuf([0.0, Self::DELTA]);
        let bg = |src: &wgpu::TextureView, u: &wgpu::Buffer| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("glow"),
                layout: &layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(src),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: u.as_entire_binding(),
                    },
                ],
            })
        };
        let blur_bgs = [bg(&views[2], &h), bg(&views[0], &v), bg(&views[1], &h)];
        let desc = |label: &'static str, vs: &'static str, fs: &'static str| PipeDesc {
            label,
            vs,
            fs: Some(fs),
            vb: Vb::None,
            cull: None,
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
            depth_write: false,
            depth_compare: wgpu::CompareFunction::Always,
            a2c: false,
            samples: 1,
            format: Some(fmt),
            gbuf: false,
            depth_format: None,
            depth_bias: Default::default(),
            constants: &[],
            polygon_line: false,
        };
        let post_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("glow extract"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/post.wgsl").into()),
        });
        let post_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("glow extract"),
            bind_group_layouts: &[Some(post_layout)],
            immediate_size: 0,
        });
        let extract = make_pipeline(device, &post_module, &post_pl, desc("glow extract", "vs_post", "fs_glow_extract"));
        let blur_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("glow blur"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/glow.wgsl").into()),
        });
        let blur_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("glow blur"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let blur = make_pipeline(device, &blur_module, &blur_pl, desc("glow blur", "vs_glow", "fs_glow_blur"));
        Glow {
            views,
            extract,
            blur,
            blur_bgs,
        }
    }

    /// Extraction from the scene image (`post_bg`), then the blur passes.
    fn encode(&self, encoder: &mut wgpu::CommandEncoder, post_bg: &wgpu::BindGroup) {
        let run = |encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, pipe: &wgpu::RenderPipeline, bg: &wgpu::BindGroup| {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("glow"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
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
            pass.set_pipeline(pipe);
            pass.set_bind_group(0, bg, &[]);
            pass.draw(0..3, 0..1);
        };
        run(encoder, &self.views[2], &self.extract, post_bg);
        for (i, b) in [0usize, 1, 2, 1].into_iter().enumerate() {
            run(encoder, &self.views[i % 2], &self.blur, &self.blur_bgs[b]);
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct TaaU {
    inv_view_proj: [[f32; 4]; 4],
    prev_view_proj: [[f32; 4]; 4],
    params: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SsaoU {
    view_proj: [[f32; 4]; 4],
    inv_view_proj: [[f32; 4]; 4],
    camera_pos: [f32; 4],
    params: [f32; 4],
}

struct Taa {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    buffer: wgpu::Buffer,
    history: [wgpu::TextureView; 2],
    /// bind group `i` reads history[1 - i] and is used when writing history[i]
    bind_groups: [wgpu::BindGroup; 2],
    /// post bind group reading history[i]
    post_bgs: [wgpu::BindGroup; 2],
    parity: usize,
    reset: bool,
}

/// Halton(2,3) jitter sequence in [-0.5, 0.5].
fn halton(i: u64, base: u64) -> f32 {
    let mut f = 1.0f32;
    let mut r = 0.0f32;
    let mut i = i;
    while i > 0 {
        f /= base as f32;
        r += f * (i % base) as f32;
        i /= base;
    }
    r - 0.5
}

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("no compatible Vulkan adapter found: {0}")]
    NoAdapter(String),
    #[error("device request failed: {0}")]
    Device(String),
    #[error("surface error: {0}")]
    Surface(String),
}

#[derive(Debug, Clone)]
pub struct GpuInfo {
    pub name: String,
    pub backend: String,
    pub driver: String,
    pub bindless: bool,
    pub timestamps: bool,
    pub max_textures: u32,
    /// Dedicated video memory (MB) when it can be detected.
    pub vram_mb: Option<u64>,
    /// Integrated GPU (shares the system memory).
    pub integrated: bool,
}

pub struct EguiFrame<'a> {
    pub primitives: &'a [egui::ClippedPrimitive],
    pub textures_delta: &'a egui::TexturesDelta,
    pub pixels_per_point: f32,
}

/// A planar reflection render target.
struct ReflTarget {
    color: wgpu::TextureView,
    depth: wgpu::TextureView,
    width: u32,
    height: u32,
}

struct Targets {
    color_msaa: Option<wgpu::TextureView>,
    color_tex: wgpu::Texture,
    color: wgpu::TextureView,
    /// Multisampled depth (MSAA only; otherwise `depth_ss` is the scene depth).
    depth_msaa: Option<wgpu::TextureView>,
    /// Single-sample depth from the prepass.
    depth_ss: wgpu::TextureView,
    scene_copy_tex: wgpu::Texture,
    scene_copy: wgpu::TextureView,
    /// G-buffer (normal, roughness, metallic) for SSR.
    gbuf_msaa: Option<wgpu::TextureView>,
    gbuf: Option<wgpu::TextureView>,
    /// SSAO raw and blurred (half resolution).
    ao: Option<(wgpu::TextureView, wgpu::TextureView, u32, u32)>,
    water_refl: Option<ReflTarget>,
    mirror: Option<ReflTarget>,
    post_bind_group: wgpu::BindGroup,
}

struct Pipelines {
    sky: wgpu::RenderPipeline,
    terrain: wgpu::RenderPipeline,
    opaque: wgpu::RenderPipeline,
    opaque_2s: wgpu::RenderPipeline,
    mask: wgpu::RenderPipeline,
    mask_2s: wgpu::RenderPipeline,
    water: wgpu::RenderPipeline,
    blend: wgpu::RenderPipeline,
    particles: wgpu::RenderPipeline,
    glow: wgpu::RenderPipeline,
    glow_max: wgpu::RenderPipeline,
    glow_suppress: wgpu::RenderPipeline,
    /// Debug « Afficher la transparence »: red and blue translucent faces.
    debug_red: wgpu::RenderPipeline,
    debug_blue: wgpu::RenderPipeline,
    /// Debug wireframe (None without POLYGON_MODE_LINE).
    debug_wire: Option<wgpu::RenderPipeline>,
    /// Selection wireframes (SilhouetteParentColor / SilhouetteChildColor).
    select_root_wire: Option<wgpu::RenderPipeline>,
    select_child_wire: Option<wgpu::RenderPipeline>,
    ssr: Option<wgpu::RenderPipeline>,
    pre_opaque: wgpu::RenderPipeline,
    pre_opaque_2s: wgpu::RenderPipeline,
    pre_mask: wgpu::RenderPipeline,
    pre_mask_2s: wgpu::RenderPipeline,
    refl_sky: wgpu::RenderPipeline,
    refl_terrain: wgpu::RenderPipeline,
    refl_obj: wgpu::RenderPipeline,
    shadow: wgpu::RenderPipeline,
    post: wgpu::RenderPipeline,
}

struct Ssao {
    layout: wgpu::BindGroupLayout,
    ssao: wgpu::RenderPipeline,
    blur: wgpu::RenderPipeline,
    buffer: wgpu::Buffer,
}

struct GpuTimer {
    query_set: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    readback: Vec<(wgpu::Buffer, Arc<AtomicBool>, bool)>,
    next: usize,
    period_ns: f32,
    last_ms: Option<f32>,
    /// Marks between and inside passes are written
    /// (TIMESTAMP_QUERY_INSIDE_ENCODERS + TIMESTAMP_QUERY_INSIDE_PASSES).
    marks: bool,
    /// Last breakdown by kind of element (ms), in [`GpuElement::ALL`] order.
    last_elements: Option<[f32; GpuElement::ALL.len()]>,
}

/// Timestamps of a frame: 0 first pass, 1 end of post, then the marks.
const GPU_TIMESTAMPS: u32 = 2 + GPU_MARKS;
const GPU_MARKS: u32 = 15;
/// Mark indices, in frame order. Those inside the scene passes split them
/// by draw group; a timestamp there waits for the draws before it, so the
/// split is close but not exact (the GPU overlaps neighbouring draws).
const MARK_SHADOWS_END: u32 = 2;
/// Depth prepass, occlusion culling, second prepass.
const MARK_DEPTH_END: u32 = 3;
const MARK_SSAO_END: u32 = 4;
/// Planar reflections, then the reflection probe face and its filtering.
const MARK_REFL_END: u32 = 5;
/// After the impostor pictures.
const MARK_SCENE_BEGIN: u32 = 6;
/// Inside scene pass A.
const MARK_SKY_END: u32 = 7;
const MARK_TERRAIN_END: u32 = 8;
const MARK_OBJECTS_END: u32 = 9;
/// After scene pass A (impostor cards).
const MARK_SCENE_A_END: u32 = 10;
/// After the scene copy (refraction / SSR source).
const MARK_COPY_END: u32 = 11;
/// Inside scene pass B.
const MARK_SSR_END: u32 = 12;
const MARK_WATER_END: u32 = 13;
const MARK_GLOW_END: u32 = 14;
/// Blended faces and the debug / selection overlays.
const MARK_BLEND_END: u32 = 15;
/// After scene pass B (particles).
const MARK_SCENE_END: u32 = 16;
/// GPU time ranges `(from, to)` of each kind of element, in
/// [`GpuElement::ALL`] order; together they cover the whole frame.
const GPU_ELEMENTS: [&[(u32, u32)]; GpuElement::ALL.len()] = [
    // shadows and depth
    &[(0, MARK_SHADOWS_END), (MARK_SHADOWS_END, MARK_DEPTH_END)],
    // effects: SSAO, planar reflections, probes, scene copy, SSR
    &[
        (MARK_DEPTH_END, MARK_SSAO_END),
        (MARK_SSAO_END, MARK_REFL_END),
        (MARK_SCENE_A_END, MARK_COPY_END),
        (MARK_COPY_END, MARK_SSR_END),
    ],
    // terrain and sky
    &[(MARK_SCENE_BEGIN, MARK_SKY_END), (MARK_SKY_END, MARK_TERRAIN_END)],
    // objects and avatars (opaque, masked), impostor pictures and cards
    &[
        (MARK_TERRAIN_END, MARK_OBJECTS_END),
        (MARK_REFL_END, MARK_SCENE_BEGIN),
        (MARK_OBJECTS_END, MARK_SCENE_A_END),
    ],
    // water
    &[(MARK_SSR_END, MARK_WATER_END)],
    // blended faces and particles
    &[(MARK_GLOW_END, MARK_BLEND_END), (MARK_BLEND_END, MARK_SCENE_END)],
    // post: glow, TAA, tonemap, SMAA
    &[(MARK_WATER_END, MARK_GLOW_END), (MARK_SCENE_END, 1)],
];

impl GpuTimer {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> GpuTimer {
        let query_set = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("frame timer"),
            ty: wgpu::QueryType::Timestamp,
            count: GPU_TIMESTAMPS,
        });
        let resolve = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("timestamp resolve"),
            size: GPU_TIMESTAMPS as u64 * 8,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = (0..3)
            .map(|_| {
                (
                    device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("timestamp readback"),
                        size: GPU_TIMESTAMPS as u64 * 8,
                        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    }),
                    Arc::new(AtomicBool::new(false)),
                    false,
                )
            })
            .collect();
        GpuTimer {
            query_set,
            resolve,
            readback,
            next: 0,
            period_ns: queue.get_timestamp_period(),
            last_ms: None,
            marks: device
                .features()
                .contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS | wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES),
            last_elements: None,
        }
    }

    /// Collect finished readbacks.
    fn harvest(&mut self) {
        for (buf, ready, pending) in self.readback.iter_mut() {
            if *pending && ready.load(Ordering::Acquire) {
                if let Ok(view) = buf.get_mapped_range(..) {
                    let ts: &[u64] = bytemuck::cast_slice(&view[..GPU_TIMESTAMPS as usize * 8]);
                    let ms = |a: u32, b: u32| {
                        let (a, b) = (ts[a as usize], ts[b as usize]);
                        (b.saturating_sub(a) as f64 * self.period_ns as f64 / 1e6) as f32
                    };
                    if ts[1] > ts[0] {
                        self.last_ms = Some(ms(0, 1));
                        if self.marks {
                            let mut e = [0.0; GpuElement::ALL.len()];
                            for (t, ranges) in e.iter_mut().zip(GPU_ELEMENTS) {
                                *t = ranges.iter().map(|&(a, b)| ms(a, b)).sum();
                            }
                            self.last_elements = Some(e);
                        }
                    }
                }
                buf.unmap();
                *pending = false;
                ready.store(false, Ordering::Release);
            }
        }
    }

    /// Index of a free readback buffer for this frame, if any.
    fn slot(&mut self) -> Option<usize> {
        for i in 0..self.readback.len() {
            let idx = (self.next + i) % self.readback.len();
            if !self.readback[idx].2 {
                self.next = (idx + 1) % self.readback.len();
                return Some(idx);
            }
        }
        None
    }
}

fn ts_begin(t: &Option<GpuTimer>) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
    t.as_ref().map(|t| wgpu::RenderPassTimestampWrites {
        query_set: &t.query_set,
        beginning_of_pass_write_index: Some(0),
        end_of_pass_write_index: None,
    })
}

/// Dummy resources bound where a pass must not (or cannot) read the real ones.
struct Dummies {
    white: wgpu::TextureView,
    black: wgpu::TextureView,
    depth: wgpu::TextureView,
}

/// Frame bind groups (group 0) for the different passes.
struct FrameGroups {
    /// Prepass and scene pass A: no scene copy / depth reads.
    a: wgpu::BindGroup,
    /// Scene pass B: everything.
    b: wgpu::BindGroup,
    /// Water and mirror reflection passes (own uniform buffers).
    /// Water reflection, mirror and reflection probe capture passes.
    /// [3], [4]: impostor pictures.
    refl: [wgpu::BindGroup; 5],
}

pub struct Renderer {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    surface_srgb: bool,
    pub info: GpuInfo,
    pub geometry: GeometryArena,
    pub records: RecordStore,
    pub textures: TextureTable,
    pub egui: egui_wgpu::Renderer,
    frame_layout: wgpu::BindGroupLayout,
    records_layout: wgpu::BindGroupLayout,
    shadow_layout: wgpu::BindGroupLayout,
    post_layout: wgpu::BindGroupLayout,
    gbuf_layout: wgpu::BindGroupLayout,
    pipelines: Pipelines,
    ssao: Ssao,
    frame_buffer: wgpu::Buffer,
    refl_buffers: [wgpu::Buffer; 5],
    probes: crate::probes::Probes,
    groups: FrameGroups,
    gbuf_group: Option<wgpu::BindGroup>,
    ssao_groups: Option<(wgpu::BindGroup, wgpu::BindGroup)>,
    dummies: Dummies,
    lin_clamp: wgpu::Sampler,
    records_bind_group: wgpu::BindGroup,
    records_generation: u64,
    palette_buffer: wgpu::Buffer,
    /// Palette bytes written since the last frame (AURORA_PROFILE).
    palettes_uploaded: u64,
    skin_bind_buffer: wgpu::Buffer,
    /// Shadow atlas: one tile per cascade (see `make_shadow_atlas`).
    shadow_view: wgpu::TextureView,
    shadow_buffers: Vec<wgpu::Buffer>,
    shadow_bind_groups: Vec<wgpu::BindGroup>,
    shadow_size: u32,
    post_buffer: wgpu::Buffer,
    post_sampler: wgpu::Sampler,
    shadow_sampler: wgpu::Sampler,
    glow: Glow,
    smaa: Smaa,
    smaa_targets: Option<SmaaTargets>,
    targets: Targets,
    indirect: wgpu::Buffer,
    indirect_cpu: Vec<DrawIndexedIndirect>,
    occlusion: crate::occlusion::Occlusion,
    /// Depth under the cursor (hover without waiting, clicks).
    depth_pick: crate::pick::DepthPick,
    impostors: Impostors,
    cull_cpu: Vec<crate::occlusion::CullDraw>,
    particle_buffer: wgpu::Buffer,
    timer: Option<GpuTimer>,
    pub msaa_samples: u32,
    vsync: bool,
    pub settings: RenderSettings,
    taa: Option<Taa>,
    prev_view_proj: Option<Mat4>,
    frame_index: u64,
    /// Inverse view-projection of the last rendered frame (picking).
    last_inv_vp: Mat4,
    msaa_supported: Vec<u32>,
    /// Set to request a copy of the next presented frame.
    pub capture_request: bool,
    /// Capture the 3D view without the interface (next frame) into `captured`.
    pub capture_scene: bool,
    /// Last captured frame (width, height, RGBA8).
    pub captured: Option<(u32, u32, Vec<u8>)>,
}

fn skin_layout() -> wgpu::VertexBufferLayout<'static> {
    const ATTRS: [wgpu::VertexAttribute; 2] = [
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Uint8x4,
            offset: 0,
            shader_location: 3,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Unorm8x4,
            offset: 4,
            shader_location: 4,
        },
    ];
    wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<SkinVertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &ATTRS,
    }
}

fn vertex_layout() -> wgpu::VertexBufferLayout<'static> {
    const ATTRS: [wgpu::VertexAttribute; 3] = [
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x3,
            offset: 0,
            shader_location: 0,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Snorm8x4,
            offset: 12,
            shader_location: 1,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x2,
            offset: 16,
            shader_location: 2,
        },
    ];
    wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<Vertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &ATTRS,
    }
}

fn particle_layout() -> wgpu::VertexBufferLayout<'static> {
    const ATTRS: [wgpu::VertexAttribute; 7] = [
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x3,
            offset: 0,
            shader_location: 0,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Uint32,
            offset: 12,
            shader_location: 1,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x2,
            offset: 16,
            shader_location: 2,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Uint32,
            offset: 24,
            shader_location: 3,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Unorm8x4,
            offset: 28,
            shader_location: 4,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x3,
            offset: 32,
            shader_location: 5,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32,
            offset: 44,
            shader_location: 6,
        },
    ];
    wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<ParticleInstance>() as u64,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &ATTRS,
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Vb {
    None,
    Mesh,
    Particle,
    Sprite,
}

fn sprite_layout() -> wgpu::VertexBufferLayout<'static> {
    const ATTRS: [wgpu::VertexAttribute; 4] = [
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x3,
            offset: 0,
            shader_location: 0,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Uint32,
            offset: 12,
            shader_location: 1,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x3,
            offset: 16,
            shader_location: 2,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x3,
            offset: 28,
            shader_location: 3,
        },
    ];
    wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<ImpostorSprite>() as u64,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &ATTRS,
    }
}

struct PipeDesc<'a> {
    label: &'a str,
    vs: &'a str,
    fs: Option<&'a str>,
    vb: Vb,
    cull: Option<wgpu::Face>,
    blend: Option<wgpu::BlendState>,
    /// Channels written to the first color target.
    write_mask: wgpu::ColorWrites,
    depth_write: bool,
    depth_compare: wgpu::CompareFunction,
    a2c: bool,
    samples: u32,
    format: Option<wgpu::TextureFormat>,
    /// Second color target (G-buffer for SSR).
    gbuf: bool,
    depth_format: Option<wgpu::TextureFormat>,
    depth_bias: wgpu::DepthBiasState,
    constants: &'a [(&'a str, f64)],
    /// Draw triangle edges only (POLYGON_MODE_LINE).
    polygon_line: bool,
}

/// Draws of `list` whose bounding sphere touches the view of `view_proj`
/// (side and near planes; reverse-Z infinite projections have no far plane).
fn cull_to_view(view_proj: Mat4, list: &[ShadowCaster]) -> Vec<DrawCmd> {
    let m = view_proj.transpose();
    let norm = |p: Vec4| p / p.truncate().length().max(1e-6);
    let planes = [
        norm(m.w_axis + m.x_axis),
        norm(m.w_axis - m.x_axis),
        norm(m.w_axis + m.y_axis),
        norm(m.w_axis - m.y_axis),
        // reverse-Z: z_clip <= w_clip in front of the near plane
        norm(m.w_axis - m.z_axis),
    ];
    list.iter()
        .filter(|c| planes.iter().all(|p| p.truncate().dot(c.center) + p.w >= -c.radius))
        .map(|c| c.cmd)
        .collect()
}

/// Fullscreen-triangle pipeline with one bind group (filters, blits).
pub(crate) fn make_pipeline_simple(
    device: &wgpu::Device,
    module: &wgpu::ShaderModule,
    layout: &wgpu::BindGroupLayout,
    label: &'static str,
    vs: &'static str,
    fs: &'static str,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    });
    make_pipeline(
        device,
        module,
        &pl,
        PipeDesc {
            label,
            vs,
            fs: Some(fs),
            vb: Vb::None,
            cull: None,
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
            depth_write: false,
            depth_compare: wgpu::CompareFunction::Always,
            a2c: false,
            samples: 1,
            format: Some(format),
            gbuf: false,
            depth_format: None,
            depth_bias: Default::default(),
            constants: &[],
            polygon_line: false,
        },
    )
}

fn make_pipeline(device: &wgpu::Device, module: &wgpu::ShaderModule, layout: &wgpu::PipelineLayout, d: PipeDesc) -> wgpu::RenderPipeline {
    let mesh_bufs = [Some(vertex_layout()), Some(skin_layout())];
    let part_bufs = [Some(particle_layout())];
    let sprite_bufs = [Some(sprite_layout())];
    let mut targets = Vec::new();
    if let Some(f) = d.format {
        targets.push(Some(wgpu::ColorTargetState {
            format: f,
            blend: d.blend,
            write_mask: d.write_mask,
        }));
    }
    if d.gbuf {
        targets.push(Some(wgpu::ColorTargetState {
            format: GBUF_FORMAT,
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
        }));
    }
    let options = wgpu::PipelineCompilationOptions {
        constants: d.constants,
        ..Default::default()
    };
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(d.label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some(d.vs),
            compilation_options: options.clone(),
            buffers: match d.vb {
                Vb::None => &[],
                Vb::Mesh => &mesh_bufs,
                Vb::Particle => &part_bufs,
                Vb::Sprite => &sprite_bufs,
            },
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: d.cull,
            unclipped_depth: false,
            polygon_mode: if d.polygon_line {
                wgpu::PolygonMode::Line
            } else {
                wgpu::PolygonMode::Fill
            },
            conservative: false,
        },
        depth_stencil: d.depth_format.map(|f| wgpu::DepthStencilState {
            format: f,
            depth_write_enabled: Some(d.depth_write),
            depth_compare: Some(d.depth_compare),
            stencil: wgpu::StencilState::default(),
            bias: d.depth_bias,
        }),
        multisample: wgpu::MultisampleState {
            count: d.samples,
            mask: !0,
            alpha_to_coverage_enabled: d.a2c,
        },
        fragment: d.fs.map(|fs| wgpu::FragmentState {
            module,
            entry_point: Some(fs),
            compilation_options: options,
            targets: &targets,
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn premultiplied() -> wgpu::BlendState {
    wgpu::BlendState {
        color: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        },
        alpha: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        },
    }
}

fn tex_entry(binding: u32, sample_type: wgpu::TextureSampleType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn tex(binding: u32, v: &wgpu::TextureView) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: wgpu::BindingResource::TextureView(v),
    }
}

fn float_tex(binding: u32) -> wgpu::BindGroupLayoutEntry {
    tex_entry(binding, wgpu::TextureSampleType::Float { filterable: true })
}

fn uniform_entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

pub(crate) fn make_view(
    device: &wgpu::Device,
    label: &str,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
    samples: u32,
    usage: wgpu::TextureUsages,
) -> (wgpu::Texture, wgpu::TextureView) {
    let t = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: samples,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    });
    let v = t.create_view(&wgpu::TextureViewDescriptor::default());
    (t, v)
}

/// Reflection of points about the plane dot(n, p) + d = 0.
fn reflection_matrix(n: Vec3, d: f32) -> Mat4 {
    let n = n.normalize_or(Vec3::Z);
    Mat4::from_cols(
        Vec4::new(1.0 - 2.0 * n.x * n.x, -2.0 * n.x * n.y, -2.0 * n.x * n.z, 0.0),
        Vec4::new(-2.0 * n.x * n.y, 1.0 - 2.0 * n.y * n.y, -2.0 * n.y * n.z, 0.0),
        Vec4::new(-2.0 * n.x * n.z, -2.0 * n.y * n.z, 1.0 - 2.0 * n.z * n.z, 0.0),
        Vec4::new(-2.0 * d * n.x, -2.0 * d * n.y, -2.0 * d * n.z, 1.0),
    )
}

/// Impostors (LLVOAvatar impostor rendering): pictures of far avatars in an
/// atlas, drawn as cards in the scene.
struct Impostors {
    layout: wgpu::BindGroupLayout,
    atlas: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    scratch: wgpu::Texture,
    scratch_view: wgpu::TextureView,
    depth: wgpu::TextureView,
    sprites: wgpu::Buffer,
    capture_opaque: wgpu::RenderPipeline,
    capture_blend: wgpu::RenderPipeline,
    sprite: wgpu::RenderPipeline,
}

impl Impostors {
    fn new(
        device: &wgpu::Device,
        frame_layout: &wgpu::BindGroupLayout,
        records_layout: &wgpu::BindGroupLayout,
        tex_layout: &wgpu::BindGroupLayout,
        samples: u32,
        ssr: bool,
    ) -> Impostors {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("impostor atlas"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let size = IMPOSTOR_TILE * IMPOSTOR_TILES_PER_ROW;
        let atlas = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("impostor atlas"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HDR_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let atlas_view = atlas.create_view(&Default::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("impostor"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("impostor atlas"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&atlas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        let tile = wgpu::Extent3d {
            width: IMPOSTOR_TILE,
            height: IMPOSTOR_TILE,
            depth_or_array_layers: 1,
        };
        let scratch = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("impostor capture"),
            size: tile,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HDR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let scratch_view = scratch.create_view(&Default::default());
        let depth = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("impostor capture depth"),
                size: tile,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: DEPTH_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&Default::default());
        let sprites = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("impostor cards"),
            size: IMPOSTOR_TILES as u64 * std::mem::size_of::<ImpostorSprite>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let (capture_opaque, capture_blend, sprite) =
            Self::make_pipelines(device, &layout, frame_layout, records_layout, tex_layout, samples, ssr);
        Impostors {
            layout,
            atlas,
            bind_group,
            scratch,
            scratch_view,
            depth,
            sprites,
            capture_opaque,
            capture_blend,
            sprite,
        }
    }

    fn rebuild_pipelines(
        &mut self,
        device: &wgpu::Device,
        frame_layout: &wgpu::BindGroupLayout,
        records_layout: &wgpu::BindGroupLayout,
        tex_layout: &wgpu::BindGroupLayout,
        samples: u32,
        ssr: bool,
    ) {
        (self.capture_opaque, self.capture_blend, self.sprite) =
            Self::make_pipelines(device, &self.layout, frame_layout, records_layout, tex_layout, samples, ssr);
    }

    fn make_pipelines(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        frame_layout: &wgpu::BindGroupLayout,
        records_layout: &wgpu::BindGroupLayout,
        tex_layout: &wgpu::BindGroupLayout,
        samples: u32,
        ssr: bool,
    ) -> (wgpu::RenderPipeline, wgpu::RenderPipeline, wgpu::RenderPipeline) {
        let src = format!(
            "{}\n{}\n{}",
            include_str!("shaders/common.wgsl"),
            include_str!("shaders/object.wgsl"),
            include_str!("shaders/env.wgsl")
        );
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("impostors"),
            source: wgpu::ShaderSource::Wgsl(src.into()),
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("impostors"),
            bind_group_layouts: &[Some(frame_layout), Some(records_layout), Some(tex_layout), Some(layout)],
            immediate_size: 0,
        });
        // the pictures do not read the atlas they are copied into
        let capture_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("impostor capture"),
            bind_group_layouts: &[Some(frame_layout), Some(records_layout), Some(tex_layout)],
            immediate_size: 0,
        });
        let desc = |label: &'static str, vs: &'static str, fs: &'static str, vb: Vb, samples: u32, gbuf: bool| PipeDesc {
            label,
            vs,
            fs: Some(fs),
            vb,
            cull: None,
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
            depth_write: true,
            depth_compare: wgpu::CompareFunction::GreaterEqual,
            a2c: false,
            samples,
            format: Some(HDR_FORMAT),
            gbuf,
            depth_format: Some(DEPTH_FORMAT),
            depth_bias: Default::default(),
            constants: &[],
            polygon_line: false,
        };
        // pictures: alpha 1 where the avatar covers the tile
        let capture_opaque = make_pipeline(
            device,
            &module,
            &capture_pl,
            desc("impostor capture", "vs_main", "fs_impostor_capture", Vb::Mesh, 1, false),
        );
        let capture_blend = make_pipeline(
            device,
            &module,
            &capture_pl,
            desc("impostor capture blend", "vs_main", "fs_impostor_capture_blend", Vb::Mesh, 1, false),
        );
        // cards in scene pass A (its sample count and SSR G-buffer); the scene
        // alpha holds the glow: color only
        let sprite = make_pipeline(
            device,
            &module,
            &pl,
            PipeDesc {
                write_mask: wgpu::ColorWrites::COLOR,
                ..desc(
                    "impostor cards",
                    "vs_impostor",
                    if ssr { "fs_impostor_g" } else { "fs_impostor" },
                    Vb::Sprite,
                    samples,
                    ssr,
                )
            },
        );
        (capture_opaque, capture_blend, sprite)
    }
}

/// Orthographic projection with reverse Z (1 at `near`, 0 at `far`), framing
/// a square of half size `half` (impostor pictures).
/// World point the shadow texel grid of a cascade is anchored on: its
/// center rounded down to a lattice of the cascade radius (see
/// `cascade_matrices`).
fn shadow_anchor(center: Vec3, radius: f32) -> Vec3 {
    (center / radius).floor() * radius
}

fn ortho_reverse_z(half: f32, near: f32, far: f32) -> Mat4 {
    let d = (far - near).max(1e-3);
    Mat4::from_cols(
        Vec4::new(1.0 / half, 0.0, 0.0, 0.0),
        Vec4::new(0.0, 1.0 / half, 0.0, 0.0),
        Vec4::new(0.0, 0.0, 1.0 / d, 0.0),
        Vec4::new(0.0, 0.0, far / d, 1.0),
    )
}

/// Index ranges of the draw lists inside the indirect buffer.
#[derive(Default, Clone, Copy)]
struct Ranges {
    terrain: (u64, u32),
    opaque: (u64, u32),
    opaque_2s: (u64, u32),
    mask: (u64, u32),
    mask_2s: (u64, u32),
    water: (u64, u32),
    blend: (u64, u32),
    glow: (u64, u32),
    glow_alpha: (u64, u32),
    debug_red: (u64, u32),
    debug_blue: (u64, u32),
    select_root: (u64, u32),
    select_child: (u64, u32),
    refl_water: (u64, u32),
    refl_mirror: (u64, u32),
    refl_terrain: (u64, u32),
    probe: (u64, u32),
    shadow: [(u64, u32); CASCADES],
}

impl Renderer {
    /// Create the renderer on the Vulkan backend for the given window.
    pub fn new(target: impl Into<wgpu::SurfaceTarget<'static>>, width: u32, height: u32, vsync: bool) -> Result<Renderer, RenderError> {
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
        desc.backends = wgpu::Backends::VULKAN;
        desc.flags = if std::env::var_os("AURORA_GPU_VALIDATION").is_some() {
            wgpu::InstanceFlags::debugging()
        } else {
            wgpu::InstanceFlags::empty()
        };
        let instance = wgpu::Instance::new(desc);
        let surface = instance.create_surface(target).map_err(|e| RenderError::Surface(e.to_string()))?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: Some(&surface),
            apply_limit_buckets: false,
        }))
        .map_err(|e| RenderError::NoAdapter(e.to_string()))?;
        let ainfo = adapter.get_info();
        log::info!(
            "GPU: {} ({:?}, driver {} {})",
            ainfo.name,
            ainfo.backend,
            ainfo.driver,
            ainfo.driver_info
        );
        let vram_mb = detect_vram_mb(&adapter);
        log::info!(
            "GPU memory: {}",
            vram_mb.map(|m| format!("{m} MB")).unwrap_or_else(|| "unknown".into())
        );

        let afeat = adapter.features();
        let alim = adapter.limits();
        let bindless_feats =
            wgpu::Features::TEXTURE_BINDING_ARRAY | wgpu::Features::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING;
        let bindless = afeat.contains(bindless_feats) && alim.max_binding_array_elements_per_shader_stage >= 64;
        if !bindless {
            return Err(RenderError::Device(
                "the GPU driver lacks descriptor indexing (texture binding arrays)".into(),
            ));
        }
        let mut features = bindless_feats;
        if afeat.contains(wgpu::Features::INDIRECT_FIRST_INSTANCE) {
            features |= wgpu::Features::INDIRECT_FIRST_INSTANCE;
        } else {
            return Err(RenderError::Device("INDIRECT_FIRST_INSTANCE unsupported".into()));
        }
        let timestamps = afeat.contains(wgpu::Features::TIMESTAMP_QUERY);
        if timestamps {
            features |= wgpu::Features::TIMESTAMP_QUERY;
            // GPU time by kind of element (performance panel)
            for f in [
                wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS,
                wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES,
            ] {
                if afeat.contains(f) {
                    features |= f;
                }
            }
        }
        // needed for MSAA 8x on HDR targets (adapter-specific sample counts)
        let adapter_specific = afeat.contains(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES);
        if adapter_specific {
            features |= wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES;
        }
        // debug wireframe
        if afeat.contains(wgpu::Features::POLYGON_MODE_LINE) {
            features |= wgpu::Features::POLYGON_MODE_LINE;
        }
        if afeat.contains(wgpu::Features::PARTIALLY_BOUND_BINDING_ARRAY) {
            features |= wgpu::Features::PARTIALLY_BOUND_BINDING_ARRAY;
        }
        let max_textures = alim
            .max_binding_array_elements_per_shader_stage
            .min(alim.max_sampled_textures_per_shader_stage.saturating_sub(16))
            .clamp(64, 16384);
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("aurora"),
            required_features: features,
            required_limits: alim.clone(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
        }))
        .map_err(|e| RenderError::Device(e.to_string()))?;
        device.on_uncaptured_error(Arc::new(|e: wgpu::Error| {
            log::error!("wgpu error: {e}");
        }));

        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| matches!(f, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm))
            .or_else(|| caps.formats.first().copied())
            .ok_or_else(|| RenderError::Surface("no surface formats".into()))?;
        let surface_srgb = format.is_srgb();
        let present_mode = Self::pick_present_mode(&caps, vsync);
        let mut surface_usage = wgpu::TextureUsages::RENDER_ATTACHMENT;
        if caps.usages.contains(wgpu::TextureUsages::COPY_SRC) {
            surface_usage |= wgpu::TextureUsages::COPY_SRC;
        }
        let config = wgpu::SurfaceConfiguration {
            usage: surface_usage,
            format,
            color_space: Default::default(),
            width: width.max(1),
            height: height.max(1),
            present_mode,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes.first().copied().unwrap_or(wgpu::CompositeAlphaMode::Auto),
            view_formats: vec![],
        };
        surface.configure(&device, &config);

        // ---- layouts
        let frame_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("frame"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::VERTEX_FRAGMENT),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
                float_tex(3),
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                float_tex(5),
                float_tex(6),
                float_tex(7),
                tex_entry(8, wgpu::TextureSampleType::Depth),
                wgpu::BindGroupLayoutEntry {
                    binding: 9,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 10,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::CubeArray,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 11,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 12,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::CubeArray,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let records_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("records"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let shadow_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shadow params"),
            entries: &[uniform_entry(0, wgpu::ShaderStages::VERTEX)],
        });
        let post_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("post"),
            entries: &[
                float_tex(0),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                uniform_entry(2, wgpu::ShaderStages::FRAGMENT),
                float_tex(3),
            ],
        });
        let gbuf_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gbuffer"),
            entries: &[float_tex(0)],
        });

        let textures = TextureTable::new(&device, &queue, max_textures, 8);
        let geometry = GeometryArena::new(&device, alim.max_buffer_size.min(1 << 31));
        let records = RecordStore::new(&device);

        let mk_frame_buffer = |label| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: std::mem::size_of::<FrameU>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let frame_buffer = mk_frame_buffer("frame uniforms");
        let refl_buffers = [
            mk_frame_buffer("water reflection uniforms"),
            mk_frame_buffer("mirror uniforms"),
            mk_frame_buffer("probe capture uniforms"),
            mk_frame_buffer("impostor uniforms 0"),
            mk_frame_buffer("impostor uniforms 1"),
        ];
        let probes = crate::probes::Probes::new(&device, RenderSettings::default().probe_slots);

        let shadow_size = 2048u32;
        let shadow_view = Self::make_shadow_atlas(&device, shadow_size);
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("shadow"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            lod_min_clamp: 0.0,
            lod_max_clamp: 0.0,
            compare: Some(wgpu::CompareFunction::LessEqual),
            anisotropy_clamp: 1,
            border_color: None,
        });
        let lin_clamp = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("linear clamp"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let shadow_buffers: Vec<wgpu::Buffer> = (0..CASCADES)
            .map(|_| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("shadow vp"),
                    size: 64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            })
            .collect();
        let shadow_bind_groups = shadow_buffers
            .iter()
            .map(|b| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("shadow vp"),
                    layout: &shadow_layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: b.as_entire_binding(),
                    }],
                })
            })
            .collect();
        let palette_buffer = Self::make_palette_buffer(&device, 64 * 160);
        let skin_bind_buffer = Self::make_skin_bind_buffer(&device, 256);
        let records_bind_group = Self::make_records_bg(&device, &records_layout, &records, &palette_buffer, &skin_bind_buffer);

        let post_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("post"),
            size: std::mem::size_of::<PostU>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let post_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("post"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let dummies = {
            let usage = wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST;
            let (wt, white) = make_view(&device, "dummy white", 1, 1, wgpu::TextureFormat::Rgba8Unorm, 1, usage);
            let (_, black) = make_view(&device, "dummy black", 1, 1, HDR_FORMAT, 1, usage);
            let (_, depth) = make_view(
                &device,
                "dummy depth",
                1,
                1,
                DEPTH_FORMAT,
                1,
                wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
            );
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &wt,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &[255u8; 4],
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4),
                    rows_per_image: Some(1),
                },
                wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
            );
            Dummies { white, black, depth }
        };

        let fmt_features = adapter.get_texture_format_features(HDR_FORMAT);
        let depth_features = adapter.get_texture_format_features(DEPTH_FORMAT);
        let gbuf_features = adapter.get_texture_format_features(GBUF_FORMAT);
        let msaa_supported: Vec<u32> = [2u32, 4, 8]
            .into_iter()
            .filter(|n| *n == 4 || adapter_specific)
            .filter(|n| {
                fmt_features.flags.sample_count_supported(*n)
                    && depth_features.flags.sample_count_supported(*n)
                    && gbuf_features.flags.sample_count_supported(*n)
            })
            .collect();
        let msaa_samples = if msaa_supported.contains(&4) { 4 } else { 1 };
        let settings = RenderSettings::default();
        let pipelines = Self::make_pipelines(
            &device,
            &frame_layout,
            &records_layout,
            &textures.layout,
            &shadow_layout,
            &post_layout,
            &gbuf_layout,
            msaa_samples,
            format,
            settings.ssr,
        );
        let ssao = Self::make_ssao(&device);
        let glow = Glow::new(&device, &post_layout);
        let smaa = Smaa::new(&device, &queue, &post_layout, format);
        let targets = Self::make_targets(
            &device,
            &post_layout,
            &post_sampler,
            &post_buffer,
            &glow.views[1],
            config.width,
            config.height,
            msaa_samples,
            &settings,
        );
        let groups = Self::make_frame_groups(
            &device,
            &frame_layout,
            &frame_buffer,
            &refl_buffers,
            &shadow_view,
            &shadow_sampler,
            &lin_clamp,
            &dummies,
            &targets,
            &probes,
        );
        let indirect = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("indirect"),
            size: 64 * 1024 * std::mem::size_of::<DrawIndexedIndirect>() as u64,
            usage: wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let depth_pick = crate::pick::DepthPick::new(&device);
        let occlusion = crate::occlusion::Occlusion::new(&device);
        let impostors = Impostors::new(
            &device,
            &frame_layout,
            &records_layout,
            &textures.layout,
            msaa_samples,
            settings.ssr,
        );
        let particle_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("particles"),
            size: 4096 * std::mem::size_of::<ParticleInstance>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let egui = egui_wgpu::Renderer::new(&device, format, egui_wgpu::RendererOptions::default());
        let timer = timestamps.then(|| GpuTimer::new(&device, &queue));

        let info = GpuInfo {
            name: ainfo.name.clone(),
            backend: format!("{:?}", ainfo.backend),
            driver: format!("{} {}", ainfo.driver, ainfo.driver_info),
            bindless,
            timestamps,
            max_textures,
            vram_mb,
            integrated: ainfo.device_type == wgpu::DeviceType::IntegratedGpu,
        };
        let mut r = Renderer {
            device,
            queue,
            surface,
            config,
            surface_srgb,
            info,
            geometry,
            records,
            textures,
            egui,
            frame_layout,
            records_layout,
            shadow_layout,
            post_layout,
            gbuf_layout,
            pipelines,
            ssao,
            frame_buffer,
            refl_buffers,
            probes,
            groups,
            gbuf_group: None,
            ssao_groups: None,
            dummies,
            lin_clamp,
            records_bind_group,
            records_generation: 0,
            palette_buffer,
            palettes_uploaded: 0,
            skin_bind_buffer,
            shadow_view,
            shadow_buffers,
            shadow_bind_groups,
            shadow_size,
            post_buffer,
            post_sampler,
            shadow_sampler,
            glow,
            smaa,
            smaa_targets: None,
            targets,
            indirect,
            indirect_cpu: Vec::new(),
            occlusion,
            depth_pick,
            impostors,
            cull_cpu: Vec::new(),
            particle_buffer,
            timer,
            msaa_samples,
            vsync,
            settings,
            taa: None,
            prev_view_proj: None,
            frame_index: 0,
            last_inv_vp: Mat4::IDENTITY,
            msaa_supported,
            capture_request: false,
            capture_scene: false,
            captured: None,
        };
        r.rebuild_bind_groups();
        Ok(r)
    }

    fn pick_present_mode(caps: &wgpu::SurfaceCapabilities, vsync: bool) -> wgpu::PresentMode {
        let has = |m| caps.present_modes.contains(&m);
        if vsync {
            if has(wgpu::PresentMode::FifoRelaxed) {
                wgpu::PresentMode::FifoRelaxed
            } else {
                wgpu::PresentMode::Fifo
            }
        } else if has(wgpu::PresentMode::Mailbox) {
            wgpu::PresentMode::Mailbox
        } else if has(wgpu::PresentMode::Immediate) {
            wgpu::PresentMode::Immediate
        } else {
            wgpu::PresentMode::Fifo
        }
    }

    /// Shadow atlas of 2x2 tiles of `size` (cascade i in tile (i % 2, i / 2)),
    /// all rendered in one pass: the bindless texture group, which wgpu checks
    /// element by element for every pass binding it, is bound once instead of
    /// once per cascade. The fourth tile is unused.
    fn make_shadow_atlas(device: &wgpu::Device, size: u32) -> wgpu::TextureView {
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shadow atlas"),
            size: wgpu::Extent3d {
                width: size * 2,
                height: size * 2,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SHADOW_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        tex.create_view(&wgpu::TextureViewDescriptor::default())
    }

    fn make_skin_bind_buffer(device: &wgpu::Device, count: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("skin binds"),
            size: (count.max(1) * std::mem::size_of::<SkinBind>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Upload the skin bindings (inverse bind matrix + palette joint per
    /// mesh joint) referenced by `DrawRecord::flags[2]` of skinned records.
    pub fn set_skin_binds(&mut self, binds: &[SkinBind]) {
        if binds.is_empty() {
            return;
        }
        let size = std::mem::size_of::<SkinBind>();
        if std::mem::size_of_val(binds) as u64 > self.skin_bind_buffer.size() {
            let mut n = (self.skin_bind_buffer.size() as usize / size).max(1);
            while n < binds.len() {
                n *= 2;
            }
            self.skin_bind_buffer = Self::make_skin_bind_buffer(&self.device, n);
            self.records_bind_group = Self::make_records_bg(
                &self.device,
                &self.records_layout,
                &self.records,
                &self.palette_buffer,
                &self.skin_bind_buffer,
            );
        }
        self.queue.write_buffer(&self.skin_bind_buffer, 0, bytemuck::cast_slice(binds));
    }

    fn make_palette_buffer(device: &wgpu::Device, matrices: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("joint palettes"),
            size: (matrices.max(1) * 64) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    fn make_records_bg(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        records: &RecordStore,
        palettes: &wgpu::Buffer,
        skin_binds: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("records"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: records.buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: palettes.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: skin_binds.as_entire_binding(),
                },
            ],
        })
    }

    /// Upload joint palettes (column-major matrices). Indices referenced by
    /// `DrawRecord::flags[1]` of skinned records.
    pub fn set_palettes(&mut self, mats: &[[[f32; 4]; 4]]) {
        if mats.is_empty() {
            return;
        }
        let bytes = (mats.len() * 64) as u64;
        if bytes > self.palette_buffer.size() {
            let mut n = (self.palette_buffer.size() / 64) as usize;
            while n * 64 < bytes as usize {
                n *= 2;
            }
            self.palette_buffer = Self::make_palette_buffer(&self.device, n);
            self.records_bind_group = Self::make_records_bg(
                &self.device,
                &self.records_layout,
                &self.records,
                &self.palette_buffer,
                &self.skin_bind_buffer,
            );
        }
        self.queue.write_buffer(&self.palette_buffer, 0, bytemuck::cast_slice(mats));
        self.palettes_uploaded += bytes;
    }

    /// Upload only some palettes (`per` matrices each, by slot index): the
    /// skeletons posed this frame. Everything when the buffer must grow.
    pub fn set_palette_slots(&mut self, mats: &[[[f32; 4]; 4]], slots: &[usize], per: usize) {
        if (mats.len() * 64) as u64 > self.palette_buffer.size() {
            self.set_palettes(mats);
            return;
        }
        let mut sorted = slots.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        let mut i = 0;
        while i < sorted.len() {
            // neighbouring slots in one write
            let first = sorted[i];
            let mut last = first;
            while i + 1 < sorted.len() && sorted[i + 1] == last + 1 {
                i += 1;
                last = sorted[i];
            }
            i += 1;
            let (a, b) = (first * per, ((last + 1) * per).min(mats.len()));
            if a < b {
                self.queue
                    .write_buffer(&self.palette_buffer, (a * 64) as u64, bytemuck::cast_slice(&mats[a..b]));
                self.palettes_uploaded += ((b - a) * 64) as u64;
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn make_pipelines(
        device: &wgpu::Device,
        frame_layout: &wgpu::BindGroupLayout,
        records_layout: &wgpu::BindGroupLayout,
        tex_layout: &wgpu::BindGroupLayout,
        shadow_layout: &wgpu::BindGroupLayout,
        post_layout: &wgpu::BindGroupLayout,
        gbuf_layout: &wgpu::BindGroupLayout,
        samples: u32,
        surface_format: wgpu::TextureFormat,
        ssr: bool,
    ) -> Pipelines {
        let src = format!(
            "{}\n{}\n{}",
            include_str!("shaders/common.wgsl"),
            include_str!("shaders/object.wgsl"),
            include_str!("shaders/env.wgsl")
        );
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("scene"),
            source: wgpu::ShaderSource::Wgsl(src.into()),
        });
        let scene_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("scene"),
            bind_group_layouts: &[Some(frame_layout), Some(records_layout), Some(tex_layout)],
            immediate_size: 0,
        });
        let shadow_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shadow"),
            bind_group_layouts: &[None, Some(records_layout), Some(tex_layout), Some(shadow_layout)],
            immediate_size: 0,
        });
        let a2c = [("ALPHA_TO_COVERAGE", if samples > 1 { 1.0 } else { 0.0 })];
        // pass A (G-buffer variants when SSR is on)
        let base = |label: &'static str, vs: &'static str, fs: &'static str| PipeDesc {
            label,
            vs,
            fs: Some(fs),
            vb: Vb::Mesh,
            cull: Some(wgpu::Face::Back),
            blend: None,
            // the scene alpha holds the glow amount (LL: color mask off on
            // alpha, glow pass writes alpha only)
            write_mask: wgpu::ColorWrites::COLOR,
            depth_write: true,
            depth_compare: wgpu::CompareFunction::GreaterEqual,
            a2c: false,
            samples,
            format: Some(HDR_FORMAT),
            gbuf: false,
            depth_format: Some(DEPTH_FORMAT),
            depth_bias: Default::default(),
            constants: &[],
            polygon_line: false,
        };
        let g = |fs: &'static str, fs_g: &'static str| if ssr { fs_g } else { fs };
        let pipe = |d: PipeDesc| make_pipeline(device, &module, &scene_layout, d);
        let sky = pipe(PipeDesc {
            vb: Vb::None,
            cull: None,
            depth_write: false,
            depth_compare: wgpu::CompareFunction::Always,
            gbuf: ssr,
            ..base("sky", "vs_sky", g("fs_sky", "fs_sky_g"))
        });
        let terrain = pipe(PipeDesc {
            gbuf: ssr,
            ..base("terrain", "vs_terrain", g("fs_terrain", "fs_terrain_g"))
        });
        let opaque = pipe(PipeDesc {
            gbuf: ssr,
            ..base("opaque", "vs_main", g("fs_opaque", "fs_opaque_g"))
        });
        let opaque_2s = pipe(PipeDesc {
            cull: None,
            gbuf: ssr,
            ..base("opaque 2s", "vs_main", g("fs_opaque", "fs_opaque_g"))
        });
        let mask = pipe(PipeDesc {
            a2c: samples > 1,
            gbuf: ssr,
            constants: &a2c,
            ..base("mask", "vs_main", g("fs_mask", "fs_mask_g"))
        });
        let mask_2s = pipe(PipeDesc {
            a2c: samples > 1,
            cull: None,
            gbuf: ssr,
            constants: &a2c,
            ..base("mask 2s", "vs_main", g("fs_mask", "fs_mask_g"))
        });
        // pass B
        let water = pipe(PipeDesc {
            cull: None,
            blend: Some(premultiplied()),
            depth_write: false,
            depth_compare: wgpu::CompareFunction::Greater,
            ..base("water", "vs_water", "fs_water")
        });
        // LLDrawPoolAlpha "glow suppression": blended faces scale the glow
        // behind them by (1 - alpha)
        let suppress = wgpu::BlendState {
            color: premultiplied().color,
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
        };
        let blend = pipe(PipeDesc {
            blend: Some(suppress),
            write_mask: wgpu::ColorWrites::ALL,
            depth_write: false,
            depth_compare: wgpu::CompareFunction::Greater,
            ..base("blend", "vs_main", "fs_blend")
        });
        let particles = pipe(PipeDesc {
            vb: Vb::Particle,
            cull: None,
            blend: Some(suppress),
            write_mask: wgpu::ColorWrites::ALL,
            depth_write: false,
            depth_compare: wgpu::CompareFunction::Greater,
            ..base("particles", "vs_particle", "fs_particle")
        });
        // LLDrawPoolGlow: glow faces again, adding texture alpha x glow to
        // the scene alpha only (depth tested, pulled toward the camera)
        let glow = pipe(PipeDesc {
            cull: None,
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
            depth_write: false,
            // same vertex shader as the scene pass: equal depths pass without
            // the polygon offset LL needs, which let layered clothes 1-2 mm
            // under the visible one add their glow too
            depth_compare: wgpu::CompareFunction::GreaterEqual,
            ..base("glow", "vs_main", "fs_glow")
        });
        // opaque glowing faces: mesh bodies stack coplanar layers that all pass
        // the depth test; keep the strongest glow instead of their sum
        let glow_max = pipe(PipeDesc {
            cull: None,
            blend: Some(wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::Zero,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Max,
                },
            }),
            write_mask: wgpu::ColorWrites::ALPHA,
            depth_write: false,
            depth_compare: wgpu::CompareFunction::GreaterEqual,
            ..base("glow max", "vs_main", "fs_glow")
        });
        let glow_suppress = pipe(PipeDesc {
            cull: None,
            blend: Some(wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::Zero,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::Zero,
                    dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                    operation: wgpu::BlendOperation::Add,
                },
            }),
            write_mask: wgpu::ColorWrites::ALPHA,
            depth_write: false,
            depth_compare: wgpu::CompareFunction::GreaterEqual,
            ..base("glow suppress", "vs_main", "fs_glow_suppress")
        });
        // debug overlays: flat translucent colors over the scene, depth tested
        let alpha_blend = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::SrcAlpha,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent::REPLACE,
        };
        let debug = |label: &'static str, constants: &'static [(&'static str, f64)], line: bool| {
            pipe(PipeDesc {
                cull: None,
                blend: Some(alpha_blend),
                write_mask: wgpu::ColorWrites::COLOR,
                depth_write: false,
                depth_compare: wgpu::CompareFunction::GreaterEqual,
                constants,
                polygon_line: line,
                ..base(label, "vs_main", "fs_debug_flat")
            })
        };
        let debug_red = debug(
            "debug red",
            &[("debug_r", 1.0), ("debug_g", 0.0), ("debug_b", 0.0), ("debug_a", 0.35)],
            false,
        );
        let debug_blue = debug(
            "debug blue",
            &[("debug_r", 0.0), ("debug_g", 0.2), ("debug_b", 1.0), ("debug_a", 0.35)],
            false,
        );
        let debug_wire = device.features().contains(wgpu::Features::POLYGON_MODE_LINE).then(|| {
            debug(
                "debug wire",
                &[("debug_r", 0.37), ("debug_g", 0.92), ("debug_b", 0.83), ("debug_a", 0.55)],
                true,
            )
        });
        let line_mode = device.features().contains(wgpu::Features::POLYGON_MODE_LINE);
        let select_root_wire = line_mode.then(|| {
            debug(
                "select root wire",
                &[("debug_r", 1.0), ("debug_g", 1.0), ("debug_b", 0.0), ("debug_a", 0.4)],
                true,
            )
        });
        let select_child_wire = line_mode.then(|| {
            debug(
                "select child wire",
                &[("debug_r", 0.3), ("debug_g", 0.6), ("debug_b", 0.9), ("debug_a", 0.4)],
                true,
            )
        });
        let ssr_pipe = ssr.then(|| {
            let src = format!("{}\n{}", include_str!("shaders/common.wgsl"), include_str!("shaders/ssr.wgsl"));
            let m = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("ssr"),
                source: wgpu::ShaderSource::Wgsl(src.into()),
            });
            let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("ssr"),
                bind_group_layouts: &[Some(frame_layout), None, None, Some(gbuf_layout)],
                immediate_size: 0,
            });
            make_pipeline(
                device,
                &m,
                &pl,
                PipeDesc {
                    vb: Vb::None,
                    cull: None,
                    blend: Some(premultiplied()),
                    depth_write: false,
                    depth_compare: wgpu::CompareFunction::Always,
                    ..base("ssr", "vs_ssr", "fs_ssr")
                },
            )
        });
        // depth prepass (single sample)
        let pre = |label: &'static str, vs: &'static str, fs: Option<&'static str>, cull: Option<wgpu::Face>| {
            pipe(PipeDesc {
                label,
                vs,
                fs,
                cull,
                samples: 1,
                format: None,
                depth_compare: wgpu::CompareFunction::Greater,
                ..base(label, vs, "fs_opaque")
            })
        };
        let pre_opaque = pre("prepass", "vs_depth", None, Some(wgpu::Face::Back));
        let pre_opaque_2s = pre("prepass 2s", "vs_depth", None, None);
        let pre_mask = pre("prepass mask", "vs_main", Some("fs_depth_mask"), Some(wgpu::Face::Back));
        let pre_mask_2s = pre("prepass mask 2s", "vs_main", Some("fs_depth_mask"), None);
        // planar reflections (single sample, mirrored winding -> no culling)
        let refl = |label: &'static str, vs: &'static str, fs: &'static str, vb: Vb| {
            pipe(PipeDesc {
                vb,
                cull: None,
                samples: 1,
                depth_compare: wgpu::CompareFunction::Greater,
                ..base(label, vs, fs)
            })
        };
        let refl_sky = pipe(PipeDesc {
            vb: Vb::None,
            cull: None,
            samples: 1,
            depth_write: false,
            depth_compare: wgpu::CompareFunction::Always,
            ..base("reflection sky", "vs_sky", "fs_sky")
        });
        let refl_terrain = refl("reflection terrain", "vs_terrain", "fs_terrain", Vb::Mesh);
        let refl_obj = refl("reflection objects", "vs_main", "fs_reflect", Vb::Mesh);
        let shadow = make_pipeline(
            device,
            &module,
            &shadow_pl,
            PipeDesc {
                label: "shadow",
                vs: "vs_shadow",
                fs: Some("fs_shadow"),
                vb: Vb::Mesh,
                cull: None,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
                depth_write: true,
                depth_compare: wgpu::CompareFunction::LessEqual,
                a2c: false,
                samples: 1,
                format: None,
                gbuf: false,
                depth_format: Some(SHADOW_FORMAT),
                depth_bias: wgpu::DepthBiasState {
                    constant: 2,
                    slope_scale: 2.0,
                    clamp: 0.0,
                },
                constants: &[],
                polygon_line: false,
            },
        );
        let post_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("post"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/post.wgsl").into()),
        });
        let post_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("post"),
            bind_group_layouts: &[Some(post_layout)],
            immediate_size: 0,
        });
        let post = make_pipeline(
            device,
            &post_module,
            &post_pl,
            PipeDesc {
                label: "post",
                vs: "vs_post",
                fs: Some("fs_post"),
                vb: Vb::None,
                cull: None,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
                depth_write: false,
                depth_compare: wgpu::CompareFunction::Always,
                a2c: false,
                samples: 1,
                format: Some(surface_format),
                gbuf: false,
                depth_format: None,
                depth_bias: Default::default(),
                constants: &[],
                polygon_line: false,
            },
        );
        Pipelines {
            sky,
            terrain,
            opaque,
            opaque_2s,
            mask,
            mask_2s,
            water,
            blend,
            particles,
            glow,
            glow_max,
            glow_suppress,
            debug_red,
            debug_blue,
            debug_wire,
            select_root_wire,
            select_child_wire,
            ssr: ssr_pipe,
            pre_opaque,
            pre_opaque_2s,
            pre_mask,
            pre_mask_2s,
            refl_sky,
            refl_terrain,
            refl_obj,
            shadow,
            post,
        }
    }

    fn make_ssao(device: &wgpu::Device) -> Ssao {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ssao"),
            entries: &[
                tex_entry(0, wgpu::TextureSampleType::Depth),
                float_tex(1),
                uniform_entry(2, wgpu::ShaderStages::FRAGMENT),
            ],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ssao"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/ssao.wgsl").into()),
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ssao"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let mk = |label: &'static str, fs: &'static str| {
            make_pipeline(
                device,
                &module,
                &pl,
                PipeDesc {
                    label,
                    vs: "vs_fullscreen",
                    fs: Some(fs),
                    vb: Vb::None,
                    cull: None,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                    depth_write: false,
                    depth_compare: wgpu::CompareFunction::Always,
                    a2c: false,
                    samples: 1,
                    format: Some(AO_FORMAT),
                    gbuf: false,
                    depth_format: None,
                    depth_bias: Default::default(),
                    constants: &[],
                    polygon_line: false,
                },
            )
        };
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ssao params"),
            size: std::mem::size_of::<SsaoU>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Ssao {
            ssao: mk("ssao", "fs_ssao"),
            blur: mk("ssao blur", "fs_blur"),
            layout,
            buffer,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn make_targets(
        device: &wgpu::Device,
        post_layout: &wgpu::BindGroupLayout,
        post_sampler: &wgpu::Sampler,
        post_buffer: &wgpu::Buffer,
        glow_view: &wgpu::TextureView,
        width: u32,
        height: u32,
        samples: u32,
        s: &RenderSettings,
    ) -> Targets {
        let (w, h) = (width.max(1), height.max(1));
        let att = wgpu::TextureUsages::RENDER_ATTACHMENT;
        let sampled = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let (color_tex, color) = make_view(device, "hdr", w, h, HDR_FORMAT, 1, sampled | wgpu::TextureUsages::COPY_SRC);
        let color_msaa = (samples > 1).then(|| make_view(device, "hdr msaa", w, h, HDR_FORMAT, samples, att).1);
        let depth_msaa = (samples > 1).then(|| make_view(device, "depth msaa", w, h, DEPTH_FORMAT, samples, att).1);
        // read by the cursor picks through a compute pass (pick.rs)
        let depth_ss = make_view(device, "depth prepass", w, h, DEPTH_FORMAT, 1, sampled).1;
        let (scene_copy_tex, scene_copy) = make_view(
            device,
            "scene copy",
            w,
            h,
            HDR_FORMAT,
            1,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        let (gbuf_msaa, gbuf) = if s.ssr {
            (
                (samples > 1).then(|| make_view(device, "gbuffer msaa", w, h, GBUF_FORMAT, samples, att).1),
                Some(make_view(device, "gbuffer", w, h, GBUF_FORMAT, 1, sampled).1),
            )
        } else {
            (None, None)
        };
        let ao = (s.ssao > 0).then(|| {
            let (hw, hh) = (w.div_ceil(2), h.div_ceil(2));
            (
                make_view(device, "ssao", hw, hh, AO_FORMAT, 1, sampled).1,
                make_view(device, "ssao blur", hw, hh, AO_FORMAT, 1, sampled).1,
                hw,
                hh,
            )
        });
        let refl = |label: &str, scale: f32| {
            (scale > 0.0).then(|| {
                let rw = ((w as f32 * scale.min(1.0)) as u32).max(16);
                let rh = ((h as f32 * scale.min(1.0)) as u32).max(16);
                ReflTarget {
                    color: make_view(device, label, rw, rh, HDR_FORMAT, 1, sampled).1,
                    depth: make_view(device, label, rw, rh, DEPTH_FORMAT, 1, att).1,
                    width: rw,
                    height: rh,
                }
            })
        };
        let water_refl = refl("water reflection", s.water_reflection_scale);
        let mirror = refl("mirror", s.mirror_scale);
        let post_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("post"),
            layout: post_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&color),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(post_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: post_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(glow_view),
                },
            ],
        });
        Targets {
            color_msaa,
            color_tex,
            color,
            depth_msaa,
            depth_ss,
            scene_copy_tex,
            scene_copy,
            gbuf_msaa,
            gbuf,
            ao,
            water_refl,
            mirror,
            post_bind_group,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn make_frame_groups(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        frame_buffer: &wgpu::Buffer,
        refl_buffers: &[wgpu::Buffer; 5],
        shadow_view: &wgpu::TextureView,
        shadow_sampler: &wgpu::Sampler,
        lin_clamp: &wgpu::Sampler,
        dummies: &Dummies,
        t: &Targets,
        probes: &crate::probes::Probes,
    ) -> FrameGroups {
        let mk = |label: &str,
                  buffer: &wgpu::Buffer,
                  ao: &wgpu::TextureView,
                  water: &wgpu::TextureView,
                  mirror: &wgpu::TextureView,
                  scene: &wgpu::TextureView,
                  depth: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(shadow_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(shadow_sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(ao),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::Sampler(lin_clamp),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: wgpu::BindingResource::TextureView(water),
                    },
                    wgpu::BindGroupEntry {
                        binding: 6,
                        resource: wgpu::BindingResource::TextureView(mirror),
                    },
                    wgpu::BindGroupEntry {
                        binding: 7,
                        resource: wgpu::BindingResource::TextureView(scene),
                    },
                    wgpu::BindGroupEntry {
                        binding: 8,
                        resource: wgpu::BindingResource::TextureView(depth),
                    },
                    wgpu::BindGroupEntry {
                        binding: 9,
                        resource: probes.buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 10,
                        resource: wgpu::BindingResource::TextureView(&probes.array_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 11,
                        resource: wgpu::BindingResource::Sampler(&probes.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 12,
                        resource: wgpu::BindingResource::TextureView(&probes.irradiance_view),
                    },
                ],
            })
        };
        let ao = t.ao.as_ref().map(|a| &a.1).unwrap_or(&dummies.white);
        let water = t.water_refl.as_ref().map(|r| &r.color).unwrap_or(&dummies.black);
        let mirror = t.mirror.as_ref().map(|r| &r.color).unwrap_or(&dummies.black);
        FrameGroups {
            a: mk("frame a", frame_buffer, ao, water, mirror, &dummies.black, &dummies.depth),
            b: mk("frame b", frame_buffer, ao, water, mirror, &t.scene_copy, &t.depth_ss),
            refl: [
                mk(
                    "frame water reflection",
                    &refl_buffers[0],
                    &dummies.white,
                    &dummies.black,
                    &dummies.black,
                    &dummies.black,
                    &dummies.depth,
                ),
                mk(
                    "frame mirror",
                    &refl_buffers[1],
                    &dummies.white,
                    &dummies.black,
                    &dummies.black,
                    &dummies.black,
                    &dummies.depth,
                ),
                mk(
                    "frame probe capture",
                    &refl_buffers[2],
                    &dummies.white,
                    &dummies.black,
                    &dummies.black,
                    &dummies.black,
                    &dummies.depth,
                ),
                mk(
                    "frame impostor 0",
                    &refl_buffers[3],
                    &dummies.white,
                    &dummies.black,
                    &dummies.black,
                    &dummies.black,
                    &dummies.depth,
                ),
                mk(
                    "frame impostor 1",
                    &refl_buffers[4],
                    &dummies.white,
                    &dummies.black,
                    &dummies.black,
                    &dummies.black,
                    &dummies.depth,
                ),
            ],
        }
    }

    /// Recreate every bind group that references size- or setting-dependent targets.
    fn rebuild_bind_groups(&mut self) {
        self.groups = Self::make_frame_groups(
            &self.device,
            &self.frame_layout,
            &self.frame_buffer,
            &self.refl_buffers,
            &self.shadow_view,
            &self.shadow_sampler,
            &self.lin_clamp,
            &self.dummies,
            &self.targets,
            &self.probes,
        );
        self.gbuf_group = self.targets.gbuf.as_ref().map(|g| {
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("gbuffer"),
                layout: &self.gbuf_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(g),
                }],
            })
        });
        self.ssao_groups = self.targets.ao.as_ref().map(|(raw, _, _, _)| {
            let mk = |input: &wgpu::TextureView| {
                self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("ssao"),
                    layout: &self.ssao.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&self.targets.depth_ss),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(input),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: self.ssao.buffer.as_entire_binding(),
                        },
                    ],
                })
            };
            (mk(&self.dummies.white), mk(raw))
        });
        if self.taa.is_some() {
            self.taa = Some(self.make_taa());
        }
    }

    fn rebuild_targets(&mut self) {
        self.occlusion.invalidate();
        self.targets = Self::make_targets(
            &self.device,
            &self.post_layout,
            &self.post_sampler,
            &self.post_buffer,
            &self.glow.views[1],
            self.config.width,
            self.config.height,
            self.msaa_samples,
            &self.settings,
        );
        self.smaa_targets = (self.settings.aa == AntiAliasing::Smaa).then(|| {
            self.smaa.make_targets(
                &self.device,
                &self.queue,
                &self.dummies.white,
                self.config.width,
                self.config.height,
                self.surface_srgb,
            )
        });
        self.rebuild_bind_groups();
    }

    pub fn size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    pub fn surface_format(&self) -> wgpu::TextureFormat {
        self.config.format
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        self.rebuild_targets();
    }

    /// Supported MSAA sample counts (besides 1).
    pub fn msaa_supported(&self) -> &[u32] {
        &self.msaa_supported
    }

    /// Apply quality settings (rebuilds pipelines / targets only when needed).
    pub fn apply_settings(&mut self, s: RenderSettings) {
        let samples = match s.aa {
            AntiAliasing::Msaa(n) => self.msaa_supported.iter().copied().filter(|&m| m <= n).max().unwrap_or(1),
            _ => 1,
        };
        let pipelines_dirty = samples != self.msaa_samples || s.ssr != self.settings.ssr;
        let targets_dirty = pipelines_dirty
            || (s.aa == AntiAliasing::Smaa) != (self.settings.aa == AntiAliasing::Smaa)
            || s.ssao.min(1) != self.settings.ssao.min(1)
            || s.water_reflection_scale != self.settings.water_reflection_scale
            || s.mirror_scale != self.settings.mirror_scale;
        let taa_was = self.taa.is_some();
        if s.anisotropy != self.settings.anisotropy {
            self.textures.set_anisotropy(&self.device, s.anisotropy);
        }
        self.msaa_samples = samples;
        if pipelines_dirty {
            self.pipelines = Self::make_pipelines(
                &self.device,
                &self.frame_layout,
                &self.records_layout,
                &self.textures.layout,
                &self.shadow_layout,
                &self.post_layout,
                &self.gbuf_layout,
                samples,
                self.config.format,
                s.ssr,
            );
            self.impostors.rebuild_pipelines(
                &self.device,
                &self.frame_layout,
                &self.records_layout,
                &self.textures.layout,
                samples,
                s.ssr,
            );
        }
        let slots = s.probe_slots.clamp(1, crate::probes::MAX_PROBES as u32);
        let probes_dirty = slots != self.probes.slots;
        if probes_dirty {
            self.probes = crate::probes::Probes::new(&self.device, slots);
        }
        // the atlas is two tiles wide
        let res = s
            .shadow_resolution
            .clamp(512, 8192)
            .min(self.device.limits().max_texture_dimension_2d / 2);
        let shadows_dirty = s.shadow_resolution > 0 && res != self.shadow_size;
        if shadows_dirty {
            self.shadow_view = Self::make_shadow_atlas(&self.device, res);
            self.shadow_size = res;
        }
        self.settings = s;
        if s.aa == AntiAliasing::Taa {
            if !taa_was {
                self.taa = Some(self.make_taa());
            }
        } else {
            self.taa = None;
            self.prev_view_proj = None;
        }
        if targets_dirty {
            self.rebuild_targets();
        } else if shadows_dirty || probes_dirty {
            self.rebuild_bind_groups();
        }
    }

    fn make_taa(&self) -> Taa {
        let device = &self.device;
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("taa"),
            entries: &[
                float_tex(0),
                tex_entry(1, wgpu::TextureSampleType::Depth),
                float_tex(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                uniform_entry(4, wgpu::ShaderStages::FRAGMENT),
            ],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("taa"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/taa.wgsl").into()),
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("taa"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = make_pipeline(
            device,
            &module,
            &pl,
            PipeDesc {
                label: "taa",
                vs: "vs_taa",
                fs: Some("fs_taa"),
                vb: Vb::None,
                cull: None,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
                depth_write: false,
                depth_compare: wgpu::CompareFunction::Always,
                a2c: false,
                samples: 1,
                format: Some(HDR_FORMAT),
                gbuf: false,
                depth_format: None,
                depth_bias: Default::default(),
                constants: &[],
                polygon_line: false,
            },
        );
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("taa params"),
            size: std::mem::size_of::<TaaU>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mk = || {
            make_view(
                device,
                "taa history",
                self.config.width,
                self.config.height,
                HDR_FORMAT,
                1,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            )
            .1
        };
        let history = [mk(), mk()];
        let bg = |read: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("taa"),
                layout: &layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&self.targets.color),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&self.targets.depth_ss),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(read),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Sampler(&self.post_sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: buffer.as_entire_binding(),
                    },
                ],
            })
        };
        let bind_groups = [bg(&history[1]), bg(&history[0])];
        let post_bg = |v: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("post taa"),
                layout: &self.post_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(v),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.post_sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: self.post_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(&self.glow.views[1]),
                    },
                ],
            })
        };
        let post_bgs = [post_bg(&history[0]), post_bg(&history[1])];
        Taa {
            pipeline,
            layout,
            buffer,
            history,
            bind_groups,
            post_bgs,
            parity: 0,
            reset: true,
        }
    }

    pub fn set_vsync(&mut self, vsync: bool) {
        if vsync == self.vsync {
            return;
        }
        self.vsync = vsync;
        // Present modes are validated by wgpu; fall back to Fifo when unsupported.
        self.config.present_mode = if vsync {
            wgpu::PresentMode::Fifo
        } else {
            wgpu::PresentMode::AutoNoVsync
        };
        self.surface.configure(&self.device, &self.config);
    }

    pub fn vsync(&self) -> bool {
        self.vsync
    }

    // ------------------------------------------------------------ resources

    pub fn upload_mesh(&mut self, vertices: &[Vertex], indices: &[u16]) -> Option<MeshAlloc> {
        self.geometry.alloc(&self.device, &self.queue, vertices, None, indices)
    }

    pub fn upload_skinned_mesh(&mut self, vertices: &[Vertex], skin: &[SkinVertex], indices: &[u16]) -> Option<MeshAlloc> {
        self.geometry.alloc(&self.device, &self.queue, vertices, Some(skin), indices)
    }

    pub fn free_mesh(&mut self, m: MeshAlloc) {
        self.geometry.free(m);
    }

    pub fn create_texture(&mut self, mips: &[MipLevel]) -> Option<u32> {
        self.textures.create(&self.device, &self.queue, mips)
    }

    pub fn replace_texture(&mut self, slot: u32, mips: &[MipLevel]) -> bool {
        self.textures.replace(&self.device, &self.queue, slot, mips)
    }

    /// Overwrite part of a texture's level 0 in place (media frames).
    pub fn update_texture_region(&mut self, slot: u32, x: u32, y: u32, w: u32, h: u32, rgba: &[u8]) -> bool {
        self.textures.write_region(&self.queue, slot, x, y, w, h, rgba)
    }

    pub fn free_texture(&mut self, slot: u32) {
        self.textures.free(slot);
    }

    // ---------------------------------------------------------------- frame

    fn cascade_matrices(&self, f: &FrameParams) -> ([Mat4; CASCADES], [f32; CASCADES]) {
        let far = f.far.min(f.shadow_distance).max(16.0);
        let splits = [far * 0.08, far * 0.3, far];
        let aspect = self.config.width as f32 / self.config.height.max(1) as f32;
        let inv_view = f.view.inverse();
        let light_dir = -f.sun_dir.normalize_or(Vec3::Z);
        let mut out = [Mat4::IDENTITY; CASCADES];
        let mut near = f.near;
        for (i, &split) in splits.iter().enumerate() {
            // frustum slice corners in view space
            let th = (f.fov_y * 0.5).tan();
            let mut corners = Vec::with_capacity(8);
            for &d in &[near, split] {
                let h = d * th;
                let w = h * aspect;
                for &(sx, sy) in &[(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                    corners.push(inv_view.transform_point3(Vec3::new(sx * w, sy * h, -d)));
                }
            }
            let center = corners.iter().fold(Vec3::ZERO, |a, c| a + *c) / 8.0;
            let radius = corners.iter().map(|c| c.distance(center)).fold(0.0f32, f32::max).max(1.0);
            let radius = (radius * 16.0).ceil() / 16.0;
            let up = if light_dir.z.abs() > 0.99 { Vec3::Y } else { Vec3::Z };
            let eye = center - light_dir * (radius + 400.0);
            let mut view = glam::camera::rh::view::look_at_mat4(eye, center, up);
            // snap to texel grid to avoid shimmering when the camera moves.
            // The grid is anchored on a world point near the cascade (its
            // center rounded to a lattice of the cascade's radius), not on
            // the region origin: the grid turns with the sun around its
            // anchor, and from ~200 m away it swept the shadows by up to
            // ~12 texels a second near the camera (visible jumps as the EEP
            // sun moves); from the cell it is 20-80 times slower, a slow
            // glide like Firestorm (which does not snap). The anchor only
            // changes when the camera crosses a cell.
            let texel = 2.0 * radius / self.shadow_size as f32;
            let anchor = shadow_anchor(center, radius);
            let origin = view.transform_point3(anchor);
            let snapped = Vec3::new((origin.x / texel).round() * texel, (origin.y / texel).round() * texel, origin.z);
            view = Mat4::from_translation(snapped - origin) * view;
            let proj = glam::camera::rh::proj::directx::orthographic(-radius, radius, -radius, radius, 0.1, radius * 2.0 + 800.0);
            out[i] = proj * view;
            near = split;
        }
        (out, splits)
    }

    /// Fill the uniform block for a camera (main view or a reflection).
    fn frame_uniforms(&self, f: &FrameParams, view: Mat4, cascades: &[Mat4; CASCADES], splits: &[f32; CASCADES]) -> FrameU {
        let vp = f.proj * view;
        let inv_view = view.inverse();
        let camera_pos = inv_view.transform_point3(Vec3::ZERO);
        let mut u = FrameU::zeroed();
        u.view_proj = vp.to_cols_array_2d();
        u.inv_view_proj = vp.inverse().to_cols_array_2d();
        u.camera_pos = camera_pos.extend(f.time).to_array();
        u.sun_dir = f.sun_dir.normalize_or(Vec3::Z).extend(f.sun_visible).to_array();
        u.sun_color = f.sun_color.extend(1.0).to_array();
        u.sky_zenith = f.sky_zenith.extend(1.0).to_array();
        u.sky_horizon = f.sky_horizon.extend(1.0).to_array();
        u.ground_color = f.ground_color.extend(1.0).to_array();
        u.fog = f.fog_color.extend(f.fog_density).to_array();
        let n_lights = f.lights.len().min(MAX_LIGHTS);
        u.params = [f.exposure, f.water_height, if f.shadows { 1.0 } else { 0.0 }, n_lights as f32];
        u.cascade_splits = [splits[0], splits[1], splits[2], 0.0];
        u.screen = [
            self.config.width as f32,
            self.config.height as f32,
            1.0 / self.config.width.max(1) as f32,
            1.0 / self.config.height.max(1) as f32,
        ];
        let right = inv_view.transform_vector3(Vec3::X).normalize_or(Vec3::X);
        let up = inv_view.transform_vector3(Vec3::Y).normalize_or(Vec3::Z);
        u.cam_right = right.extend(0.5).to_array();
        u.cam_up = up.extend(0.0).to_array();
        // water
        let w = &f.water;
        let lin = |c: Vec3| {
            let f = |x: f32| {
                if x <= 0.04045 { x / 12.92 } else { ((x + 0.055) / 1.055).powf(2.4) }
            };
            Vec3::new(f(c.x), f(c.y), f(c.z))
        };
        let kd = w.fog_density;
        let mut kd_under = kd;
        let fog_mod = w.underwater_fog_mod.clamp(0.0, 10.0);
        if fog_mod > 0.0 {
            if kd_under < 0.0 && fog_mod != fog_mod.round() {
                kd_under = 1.0;
            }
            kd_under = kd_under.powf(fog_mod);
        }
        // SL uses the clamped light norm's GL z component (world x) here
        let ks = 1.0 / f.sky.light_norm.x.max(0.3);
        u.water_fog = lin(w.fog_color.max(Vec3::ZERO)).extend(kd.max(0.0)).to_array();
        u.water_params = [ks, w.fresnel_scale, w.fresnel_offset, w.blur_multiplier];
        u.water_waves = [w.wave1.x, w.wave1.y, w.wave2.x, w.wave2.y];
        u.water_normal = w
            .normal_scale
            .extend(if kd_under.is_finite() { kd_under.max(0.0) } else { 1.0 })
            .to_array();
        // sky
        let s = &f.sky;
        u.sky_light = s.light_norm.extend(s.sun_moon_glow_factor).to_array();
        u.sky_sunlight = s.sunlight.extend(s.cloud_shadow).to_array();
        u.sky_ambient = s.ambient.extend(s.density_multiplier).to_array();
        u.sky_blue_horizon = s.blue_horizon.extend(s.haze_horizon).to_array();
        u.sky_blue_density = s.blue_density.extend(s.haze_density).to_array();
        u.sky_glow = s.glow.extend(s.max_y.max(1.0)).to_array();
        u.sky_cloud_color = s.cloud_color.extend(s.cloud_scale).to_array();
        u.sky_cloud_pd1 = s.cloud_pos_density1.extend(s.cloud_variance).to_array();
        u.sky_cloud_pd2 = s.cloud_pos_density2.extend(s.hdr_scale).to_array();
        u.sky_sun = s.sun_dir.extend(s.sun_scale).to_array();
        u.sky_moon = s.moon_dir.extend(s.moon_scale).to_array();
        u.sky_misc = [s.dome_offset, s.dome_radius, s.moon_brightness, s.star_brightness];
        u.tex_slots = [s.cloud_texture, s.sun_texture, s.moon_texture, w.normal_texture];
        // lighting model: 1 classic, 2 linear SL lighting without tone mapping, 0 HDR
        let ll_mode = if s.classic {
            1.0
        } else if s.no_post {
            2.0
        } else {
            0.0
        };
        u.sky_ll = [ll_mode, s.distance_multiplier, 0.0, 0.0];
        u.sky_obj_light = s.object_sunlight.extend(0.0).to_array();
        for (i, m) in cascades.iter().enumerate() {
            u.cascade_vp[i] = m.to_cols_array_2d();
        }
        for (i, l) in f.lights.iter().take(MAX_LIGHTS).enumerate() {
            u.lights[i] = LightU {
                pos_radius: l.position.extend(l.radius).to_array(),
                color_falloff: l.color.extend(l.falloff).to_array(),
            };
        }
        u
    }

    /// Render one frame. Returns statistics.
    pub fn render(&mut self, f: &FrameParams, lists: &DrawLists, ui: Option<EguiFrame>) -> RenderStats {
        let mut stats = RenderStats::default();
        if let Some(t) = &mut self.timer {
            t.harvest();
        }
        {
            self.occlusion.harvest();
        }
        self.depth_pick.harvest();

        // egui textures first, so they are never lost on a skipped frame
        if let Some(ui) = &ui {
            for (id, deltas) in &ui.textures_delta.set {
                for delta in deltas.iter() {
                    self.egui.update_texture(&self.device, &self.queue, *id, delta);
                }
            }
            for id in &ui.textures_delta.free {
                self.egui.free_texture(id);
            }
        }
        let t_acquire = Instant::now();
        let surface_tex = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return stats;
            }
            _ => return stats,
        };
        let surface_view = surface_tex.texture.create_view(&wgpu::TextureViewDescriptor::default());
        // CPU encode time excludes waiting for the swapchain (vsync).
        let t0 = Instant::now();
        let mut prof: Vec<(&str, Instant)> = vec![("acquire", t0)];

        // ---- resources
        self.records.flush(&self.device, &self.queue);
        if self.records.generation != self.records_generation {
            self.records_bind_group = Self::make_records_bg(
                &self.device,
                &self.records_layout,
                &self.records,
                &self.palette_buffer,
                &self.skin_bind_buffer,
            );
            self.records_generation = self.records.generation;
        }
        self.textures.maintain(&self.device, false);
        prof.push(("resources", Instant::now()));

        let (cascades, splits) = self.cascade_matrices(f);
        let unjittered_vp = f.proj * f.view;
        let jittered;
        let fu: &FrameParams = if self.taa.is_some() {
            let i = (self.frame_index % 16) + 1;
            let jx = halton(i, 2) * 2.0 / self.config.width.max(1) as f32;
            let jy = halton(i, 3) * 2.0 / self.config.height.max(1) as f32;
            let mut j = f.clone();
            j.proj = Mat4::from_translation(Vec3::new(jx, jy, 0.0)) * f.proj;
            jittered = j;
            &jittered
        } else {
            f
        };
        self.frame_index = self.frame_index.wrapping_add(1);

        // ---- planar reflections this frame
        let underwater = f.camera_pos.z < f.water_height;
        let do_water_refl = self.targets.water_refl.is_some() && !lists.water.is_empty() && !underwater;
        let mirror = f.mirror.filter(|_| self.targets.mirror.is_some());

        let mut main = self.frame_uniforms(fu, fu.view, &cascades, &splits);
        main.misc = [
            if self.targets.ao.is_some() { 1.0 } else { 0.0 },
            if self.settings.ssr { 1.0 } else { 0.0 },
            if do_water_refl { 1.0 } else { 0.0 },
            0.0,
        ];
        if let Some(m) = &mirror {
            let n = m.normal.normalize_or(Vec3::Z);
            main.mirror_plane = n.extend(-n.dot(m.point)).to_array();
            main.mirror_box = m.world_to_box.to_cols_array_2d();
        }
        self.queue.write_buffer(&self.frame_buffer, 0, bytemuck::bytes_of(&main));
        let refl_frame = |this: &Self, n: Vec3, d: f32, keep_offset: f32| {
            let m = reflection_matrix(n, d);
            let mut rf = f.clone();
            rf.shadows = false;
            let mut u = this.frame_uniforms(&rf, f.view * m, &cascades, &splits);
            u.misc = [0.0, 0.0, 0.0, 1.0];
            u.clip_plane = n.extend(d + keep_offset).to_array();
            u
        };
        let mut water_vp = Mat4::IDENTITY;
        let mut mirror_vp = Mat4::IDENTITY;
        if do_water_refl {
            let u = refl_frame(self, Vec3::Z, -f.water_height, 0.05);
            water_vp = Mat4::from_cols_array_2d(&u.view_proj);
            self.queue.write_buffer(&self.refl_buffers[0], 0, bytemuck::bytes_of(&u));
        }
        if let Some(m) = &mirror {
            let n = m.normal.normalize_or(Vec3::Z);
            let u = refl_frame(self, n, -n.dot(m.point), 0.01);
            mirror_vp = Mat4::from_cols_array_2d(&u.view_proj);
            self.queue.write_buffer(&self.refl_buffers[1], 0, bytemuck::bytes_of(&u));
        }
        // ---- reflection probes: list for the shaders, face capture uniforms
        self.probes.upload(&self.queue, f.probes.enabled, &f.probes.probes);
        let capture = f.probes.capture;
        let mut probe_vp = Mat4::IDENTITY;
        if let Some(c) = &capture {
            let mut cf = f.clone();
            cf.shadows = false;
            cf.proj = glam::camera::rh::proj::directx::perspective_infinite_reverse(std::f32::consts::FRAC_PI_2, 1.0, c.near.max(0.1));
            let mut u = self.frame_uniforms(&cf, crate::probes::face_view(c.origin, c.face), &cascades, &splits);
            probe_vp = Mat4::from_cols_array_2d(&u.view_proj);
            // reflection pass: no SSAO / SSR / mirrors / probes, no clip plane
            u.misc = [0.0, 0.0, 0.0, 1.0];
            u.clip_plane = [0.0; 4];
            let r = crate::probes::CAPTURE_RES as f32;
            u.screen = [r, r, 1.0 / r, 1.0 / r];
            self.queue.write_buffer(&self.refl_buffers[2], 0, bytemuck::bytes_of(&u));
        }
        // impostor pictures: lit like the scene, seen from the camera
        let imp_caps: Vec<&ImpostorCapture> = lists.impostor_captures.iter().take(IMPOSTOR_CAPTURES).collect();
        for (i, c) in imp_caps.iter().enumerate() {
            let mut cf = f.clone();
            cf.shadows = false;
            cf.proj = ortho_reverse_z(c.half.max(0.1), 0.05, c.depth.max(0.5));
            let mut u = self.frame_uniforms(&cf, c.view, &cascades, &splits);
            u.misc = [0.0, 0.0, 0.0, 1.0];
            u.clip_plane = [0.0; 4];
            let r = IMPOSTOR_TILE as f32;
            u.screen = [r, r, 1.0 / r, 1.0 / r];
            self.queue.write_buffer(&self.refl_buffers[3 + i], 0, bytemuck::bytes_of(&u));
        }
        let n_sprites = lists.impostor_sprites.len().min(IMPOSTOR_TILES as usize);
        if n_sprites > 0 {
            self.queue.write_buffer(
                &self.impostors.sprites,
                0,
                bytemuck::cast_slice(&lists.impostor_sprites[..n_sprites]),
            );
        }
        for (i, m) in cascades.iter().enumerate() {
            self.queue
                .write_buffer(&self.shadow_buffers[i], 0, bytemuck::bytes_of(&m.to_cols_array_2d()));
        }
        // glow passes only when some visible face glows (RenderGlow)
        let glow_on = f.glow && (!lists.glow.is_empty() || !lists.glow_alpha.is_empty() || lists.blend_glow.contains(&true));
        // diagnostic: AURORA_GLOW_SKIP bits leave out a family of glowing faces
        // (2 opaque, 4 alpha pool, 8 blended)
        static GLOW_SKIP: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
        let glow_skip = *GLOW_SKIP.get_or_init(|| std::env::var("AURORA_GLOW_SKIP").ok().and_then(|v| v.parse().ok()).unwrap_or(0));
        let post = PostU {
            exposure: f.exposure,
            // SMAA works on a gamma-encoded 8-bit image
            output_srgb: if self.surface_srgb && self.smaa_targets.is_none() {
                1.0
            } else {
                0.0
            },
            // classic skies are not tone mapped (Firestorm: no_post + legacy gamma)
            tonemapper: if f.sky.no_post {
                2.0
            } else {
                match self.settings.tonemapper {
                    Tonemapper::KhronosNeutral => 0.0,
                    Tonemapper::Aces => 1.0,
                }
            },
            fxaa: if self.settings.aa == AntiAliasing::Fxaa { 1.0 } else { 0.0 },
            sharpen: f.sharpen.clamp(0.0, 1.0),
            glow: if glow_on { 1.0 } else { 0.0 },
            debug_glow: {
                static DEBUG_GLOW: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
                let env = *DEBUG_GLOW.get_or_init(|| std::env::var_os("AURORA_DEBUG_GLOW").is_some());
                if env || f.debug_glow { 1.0 } else { 0.0 }
            },
            // legacyGamma (postDeferredTonemap / CASF LEGACY_GAMMA): same
            // condition as no_post in LLPipeline::tonemap / applyCAS
            legacy_gamma: if f.sky.no_post && f.sky.gamma.is_finite() {
                f.sky.gamma.max(0.0)
            } else {
                1.0
            },
        };
        self.queue.write_buffer(&self.post_buffer, 0, bytemuck::bytes_of(&post));
        if self.targets.ao.is_some() {
            let samples = if self.settings.ssao >= 2 { 16.0 } else { 8.0 };
            let u = SsaoU {
                view_proj: unjittered_vp.to_cols_array_2d(),
                inv_view_proj: unjittered_vp.inverse().to_cols_array_2d(),
                camera_pos: f.camera_pos.extend(0.0).to_array(),
                // Firestorm defaults: RenderSSAOScale 500, RenderSSAOMaxScale
                // 200, RenderSSAOFactor 0.3 (LLPipeline::bindDeferredShader)
                params: [500.0, samples, 200.0, 0.3],
            };
            self.queue.write_buffer(&self.ssao.buffer, 0, bytemuck::bytes_of(&u));
        }

        // ---- particles
        let n_particles = lists.particles.len() as u32;
        if n_particles > 0 {
            let bytes = std::mem::size_of_val(lists.particles.as_slice()) as u64;
            if bytes > self.particle_buffer.size() {
                let mut size = self.particle_buffer.size();
                while size < bytes {
                    size *= 2;
                }
                self.particle_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("particles"),
                    size,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
            }
            self.queue
                .write_buffer(&self.particle_buffer, 0, bytemuck::cast_slice(&lists.particles));
        }

        // ---- shadow caster culling per cascade
        let mut shadow_lists: [Vec<DrawCmd>; CASCADES] = Default::default();
        if f.shadows {
            for (ci, m) in cascades.iter().enumerate() {
                for c in &lists.shadow_casters {
                    let p = *m * c.center.extend(1.0);
                    // ortho: w == 1, radius in clip units ~ r / half-extent
                    let r = c.radius * m.x_axis.truncate().length();
                    if p.x + r < -1.0 || p.x - r > 1.0 || p.y + r < -1.0 || p.y - r > 1.0 || p.z - r > 1.0 {
                        continue;
                    }
                    shadow_lists[ci].push(c.cmd);
                }
            }
        }

        // ---- indirect args
        self.indirect_cpu.clear();
        let push = |cpu: &mut Vec<DrawIndexedIndirect>, l: &[DrawCmd]| -> (u64, u32) {
            let start = cpu.len() as u64 * std::mem::size_of::<DrawIndexedIndirect>() as u64;
            cpu.extend(l.iter().map(DrawIndexedIndirect::from));
            (start, l.len() as u32)
        };
        let cpu = &mut self.indirect_cpu;
        let mut rg = Ranges {
            terrain: push(cpu, &lists.terrain),
            opaque: push(cpu, &lists.opaque),
            opaque_2s: push(cpu, &lists.opaque_two_sided),
            mask: push(cpu, &lists.mask),
            mask_2s: push(cpu, &lists.mask_two_sided),
            water: push(cpu, &lists.water),
            blend: push(cpu, &lists.blend),
            glow: push(cpu, &lists.glow),
            glow_alpha: push(cpu, &lists.glow_alpha),
            ..Default::default()
        };
        let main_count = cpu.len();
        // GPU occlusion of the main draws (AURORA_NO_OCCLUSION turns it off)
        static NO_OCCLUSION: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let occl =
            self.settings.occlusion && main_count > 0 && !*NO_OCCLUSION.get_or_init(|| std::env::var_os("AURORA_NO_OCCLUSION").is_some());
        if occl {
            use crate::occlusion::{CullDraw, KIND_NONE, KIND_PREPASS, KIND_TESTED};
            let mut cull = std::mem::take(&mut self.cull_cpu);
            cull.clear();
            let lists_kinds: [(&[DrawCmd], u32); 9] = [
                (&lists.terrain, KIND_NONE),
                (&lists.opaque, KIND_PREPASS),
                (&lists.opaque_two_sided, KIND_PREPASS),
                (&lists.mask, KIND_PREPASS),
                (&lists.mask_two_sided, KIND_PREPASS),
                (&lists.water, KIND_NONE),
                (&lists.blend, KIND_TESTED),
                (&lists.glow, KIND_TESTED),
                (&lists.glow_alpha, KIND_TESTED),
            ];
            for (l, kind) in lists_kinds {
                cull.extend(l.iter().map(|c| CullDraw {
                    sphere: c.bounds,
                    kind,
                    _pad: [0; 3],
                }));
            }
            debug_assert_eq!(cull.len(), main_count);
            let vp = fu.proj * fu.view;
            let records = self.records.slot_count();
            self.occlusion.prepare(
                &self.device,
                &self.queue,
                &self.targets.depth_ss,
                self.config.width,
                self.config.height,
                vp,
                &cull,
                records,
            );
            self.cull_cpu = cull;
        }
        let cpu = &mut self.indirect_cpu;
        rg.debug_red = push(cpu, &lists.debug_red);
        rg.debug_blue = push(cpu, &lists.debug_blue);
        rg.select_root = push(cpu, &lists.select_root);
        rg.select_child = push(cpu, &lists.select_child);
        let imp_ranges: Vec<((u64, u32), (u64, u32))> = imp_caps.iter().map(|c| (push(cpu, &c.opaque), push(cpu, &c.blend))).collect();
        // reflection passes: each culled to its own frustum
        if capture.is_some() {
            rg.probe = push(cpu, &cull_to_view(probe_vp, &lists.probe));
        }
        if do_water_refl {
            rg.refl_water = push(cpu, &cull_to_view(water_vp, &lists.reflection));
        }
        if mirror.is_some() {
            rg.refl_mirror = push(cpu, &cull_to_view(mirror_vp, &lists.reflection));
        }
        if do_water_refl || mirror.is_some() || capture.is_some() {
            rg.refl_terrain = push(cpu, &lists.reflection_terrain);
        }
        for (i, sl) in shadow_lists.iter().enumerate() {
            rg.shadow[i] = push(cpu, sl);
        }
        let needed = (self.indirect_cpu.len() * std::mem::size_of::<DrawIndexedIndirect>()) as u64;
        if needed > self.indirect.size() {
            let mut size = self.indirect.size();
            while size < needed {
                size *= 2;
            }
            self.indirect = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("indirect"),
                size,
                usage: wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            });
        }
        if needed > 0 {
            self.queue.write_buffer(&self.indirect, 0, bytemuck::cast_slice(&self.indirect_cpu));
        }
        for d in &self.indirect_cpu[..main_count] {
            stats.triangles += d.index_count as u64 / 3;
        }
        stats.draws = lists.total() as u32;
        stats.shadow_draws = shadow_lists.iter().map(|l| l.len() as u32).sum();
        stats.particles = n_particles;
        stats.occluded = occl.then_some(self.occlusion.last_hidden);

        prof.push(("lists", Instant::now()));
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });

        let timer_slot = self.timer.as_mut().and_then(|t| t.slot());
        let mut first_pass = true;
        let mut take_first = || {
            let first = first_pass && timer_slot.is_some();
            first_pass = false;
            first
        };

        let geo = &self.geometry;
        let bind_mesh = |pass: &mut wgpu::RenderPass, frame_bg: &wgpu::BindGroup| {
            pass.set_bind_group(0, frame_bg, &[]);
            pass.set_bind_group(1, &self.records_bind_group, &[]);
            pass.set_bind_group(2, &self.textures.bind_group, &[]);
            pass.set_vertex_buffer(0, geo.vertex_buffer.slice(..));
            pass.set_vertex_buffer(1, geo.skin_buffer.slice(..));
            pass.set_index_buffer(geo.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
        };
        // draw commands recorded (AURORA_PROFILE)
        let calls = std::cell::Cell::new(0u32);
        let draw = |pass: &mut wgpu::RenderPass, pipe: &wgpu::RenderPipeline, r: (u64, u32)| {
            if r.1 > 0 {
                calls.set(calls.get() + 1);
                pass.set_pipeline(pipe);
                pass.multi_draw_indexed_indirect(&self.indirect, r.0, r.1);
            }
        };
        let draw_from = |pass: &mut wgpu::RenderPass, pipe: &wgpu::RenderPipeline, buf: &wgpu::Buffer, r: (u64, u32)| {
            if r.1 > 0 {
                calls.set(calls.get() + 1);
                pass.set_pipeline(pipe);
                pass.multi_draw_indexed_indirect(buf, r.0, r.1);
            }
        };

        prof.push(("setup", Instant::now()));
        // GPU timestamps between and inside passes (time by kind of element)
        let marks = self
            .timer
            .as_ref()
            .filter(|t| t.marks && timer_slot.is_some())
            .map(|t| &t.query_set);
        let mark = |encoder: &mut wgpu::CommandEncoder, i: u32| {
            if let Some(q) = marks {
                encoder.write_timestamp(q, i);
            }
        };
        let pass_mark = |pass: &mut wgpu::RenderPass, i: u32| {
            if let Some(q) = marks {
                pass.write_timestamp(q, i);
            }
        };
        // ---- shadows
        if f.shadows {
            // one pass for all cascades, each in its atlas tile
            let ts = if take_first() { ts_begin(&self.timer) } else { None };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shadow"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.shadow_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: ts,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if rg.shadow.iter().any(|r| r.1 > 0) {
                pass.set_pipeline(&self.pipelines.shadow);
                pass.set_bind_group(1, &self.records_bind_group, &[]);
                pass.set_bind_group(2, &self.textures.bind_group, &[]);
                pass.set_vertex_buffer(0, geo.vertex_buffer.slice(..));
                pass.set_vertex_buffer(1, geo.skin_buffer.slice(..));
                pass.set_index_buffer(geo.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
            }
            let size = self.shadow_size;
            for ci in 0..CASCADES {
                let (off, count) = rg.shadow[ci];
                if count == 0 {
                    continue;
                }
                let (x, y) = ((ci as u32 % 2) * size, (ci as u32 / 2) * size);
                pass.set_viewport(x as f32, y as f32, size as f32, size as f32, 0.0, 1.0);
                pass.set_scissor_rect(x, y, size, size);
                pass.set_bind_group(3, &self.shadow_bind_groups[ci], &[]);
                pass.multi_draw_indexed_indirect(&self.indirect, off, count);
                calls.set(calls.get() + 1);
            }
        }

        prof.push(("shadows", Instant::now()));
        mark(&mut encoder, MARK_SHADOWS_END);
        // ---- depth prepass (single sample); with occlusion, phase 1 draws
        // what was visible last frame
        if occl {
            self.occlusion.encode_prepare(&self.device, &mut encoder, &self.indirect);
        }
        {
            let ts = if take_first() { ts_begin(&self.timer) } else { None };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("depth prepass"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.targets.depth_ss,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: ts,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            bind_mesh(&mut pass, &self.groups.a);
            draw(&mut pass, &self.pipelines.pre_opaque, rg.terrain);
            let pre = if occl { &self.occlusion.pre1 } else { &self.indirect };
            draw_from(&mut pass, &self.pipelines.pre_opaque, pre, rg.opaque);
            draw_from(&mut pass, &self.pipelines.pre_opaque_2s, pre, rg.opaque_2s);
            draw_from(&mut pass, &self.pipelines.pre_mask, pre, rg.mask);
            draw_from(&mut pass, &self.pipelines.pre_mask_2s, pre, rg.mask_2s);
        }
        // ---- occlusion: Hi-Z of phase 1, test of every draw, then phase 2
        // adds the draws visible now that phase 1 did not draw
        if occl {
            self.occlusion.encode_cull(&self.device, &mut encoder, &self.indirect);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("depth prepass 2"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.targets.depth_ss,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            bind_mesh(&mut pass, &self.groups.a);
            let pre = &self.occlusion.pre2;
            draw_from(&mut pass, &self.pipelines.pre_opaque, pre, rg.opaque);
            draw_from(&mut pass, &self.pipelines.pre_opaque_2s, pre, rg.opaque_2s);
            draw_from(&mut pass, &self.pipelines.pre_mask, pre, rg.mask);
            draw_from(&mut pass, &self.pipelines.pre_mask_2s, pre, rg.mask_2s);
        }

        prof.push(("prepass", Instant::now()));
        mark(&mut encoder, MARK_DEPTH_END);
        // ---- SSAO
        if let (Some((raw, blurred, _, _)), Some((bg_ssao, bg_blur))) = (&self.targets.ao, &self.ssao_groups) {
            for (view, pipe, bg, label) in [
                (raw, &self.ssao.ssao, bg_ssao, "ssao"),
                (blurred, &self.ssao.blur, bg_blur, "ssao blur"),
            ] {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some(label),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(pipe);
                pass.set_bind_group(0, bg, &[]);
                pass.draw(0..3, 0..1);
            }
        }

        prof.push(("ssao", Instant::now()));
        mark(&mut encoder, MARK_SSAO_END);
        // ---- planar reflections
        let refl_passes = [
            (
                do_water_refl,
                self.targets.water_refl.as_ref(),
                &self.groups.refl[0],
                "water reflection",
                rg.refl_water,
            ),
            (
                mirror.is_some(),
                self.targets.mirror.as_ref(),
                &self.groups.refl[1],
                "mirror",
                rg.refl_mirror,
            ),
        ];
        for (enabled, target, bg, label, objects) in refl_passes {
            let (true, Some(t)) = (enabled, target) else {
                continue;
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(label),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &t.color,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &t.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_viewport(0.0, 0.0, t.width as f32, t.height as f32, 0.0, 1.0);
            bind_mesh(&mut pass, bg);
            pass.set_pipeline(&self.pipelines.refl_sky);
            pass.draw(0..3, 0..1);
            draw(&mut pass, &self.pipelines.refl_terrain, rg.refl_terrain);
            draw(&mut pass, &self.pipelines.refl_obj, objects);
        }

        // ---- reflection probe: one cube face (LLReflectionMap::update ->
        // cubeSnapshot), then the radiance filtering after the sixth
        if let Some(c) = &capture {
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("probe capture"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: self.probes.capture_target(c.face),
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &self.probes.depth,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(0.0),
                            store: wgpu::StoreOp::Discard,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                bind_mesh(&mut pass, &self.groups.refl[2]);
                pass.set_pipeline(&self.pipelines.refl_sky);
                pass.draw(0..3, 0..1);
                // the default probe sees sky, terrain and water only
                draw(&mut pass, &self.pipelines.refl_terrain, rg.refl_terrain);
                if !c.sky_only {
                    draw(&mut pass, &self.pipelines.refl_obj, rg.probe);
                }
            }
            if c.finish {
                self.probes.filter(&self.device, &mut encoder, c.slot);
            }
        }

        prof.push(("reflections", Instant::now()));
        mark(&mut encoder, MARK_REFL_END);
        // ---- impostor pictures, copied into their atlas tile
        for (i, (c, (opaque, blend))) in imp_caps.iter().zip(&imp_ranges).enumerate() {
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("impostor picture"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &self.impostors.scratch_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &self.impostors.depth,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(0.0),
                            store: wgpu::StoreOp::Discard,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                bind_mesh(&mut pass, &self.groups.refl[3 + i]);
                draw(&mut pass, &self.impostors.capture_opaque, *opaque);
                draw(&mut pass, &self.impostors.capture_blend, *blend);
            }
            let tile = c.tile % IMPOSTOR_TILES;
            encoder.copy_texture_to_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.impostors.scratch,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyTextureInfo {
                    texture: &self.impostors.atlas,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: (tile % IMPOSTOR_TILES_PER_ROW) * IMPOSTOR_TILE,
                        y: (tile / IMPOSTOR_TILES_PER_ROW) * IMPOSTOR_TILE,
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::Extent3d {
                    width: IMPOSTOR_TILE,
                    height: IMPOSTOR_TILE,
                    depth_or_array_layers: 1,
                },
            );
        }

        prof.push(("impostors", Instant::now()));
        // the frame is recorded into three encoders finished in parallel
        // below (wgpu replays and tracks every command in finish())
        let enc_pre = std::mem::replace(
            &mut encoder,
            self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame scene"),
            }),
        );
        mark(&mut encoder, MARK_SCENE_BEGIN);
        // ---- scene pass A: sky, terrain, opaque, masked
        let single = self.targets.color_msaa.is_none();
        {
            let (view, resolve) = match &self.targets.color_msaa {
                Some(msaa) => (msaa, Some(&self.targets.color)),
                None => (&self.targets.color, None),
            };
            let cc = f.clear_color;
            let mut atts = vec![Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: resolve,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: cc.x as f64,
                        g: cc.y as f64,
                        b: cc.z as f64,
                        // the alpha accumulates the glow amount
                        a: 0.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })];
            if let Some(g) = &self.targets.gbuf {
                let (gv, gr) = match &self.targets.gbuf_msaa {
                    Some(m) => (m, Some(g)),
                    None => (g, None),
                };
                atts.push(Some(wgpu::RenderPassColorAttachment {
                    view: gv,
                    depth_slice: None,
                    resolve_target: gr,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.5,
                            g: 0.5,
                            b: 1.0,
                            a: 0.0,
                        }),
                        store: if gr.is_some() {
                            wgpu::StoreOp::Discard
                        } else {
                            wgpu::StoreOp::Store
                        },
                    },
                }));
            }
            // single sample: reuse the prepass depth (early-z, no overdraw)
            let (depth, load) = match &self.targets.depth_msaa {
                Some(d) => (d, wgpu::LoadOp::Clear(0.0)),
                None => (&self.targets.depth_ss, wgpu::LoadOp::Load),
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene a"),
                color_attachments: &atts,
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth,
                    depth_ops: Some(wgpu::Operations {
                        load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            bind_mesh(&mut pass, &self.groups.a);
            pass.set_pipeline(&self.pipelines.sky);
            pass.draw(0..3, 0..1);
            pass_mark(&mut pass, MARK_SKY_END);
            draw(&mut pass, &self.pipelines.terrain, rg.terrain);
            pass_mark(&mut pass, MARK_TERRAIN_END);
            draw(&mut pass, &self.pipelines.opaque, rg.opaque);
            draw(&mut pass, &self.pipelines.opaque_2s, rg.opaque_2s);
            draw(&mut pass, &self.pipelines.mask, rg.mask);
            draw(&mut pass, &self.pipelines.mask_2s, rg.mask_2s);
            pass_mark(&mut pass, MARK_OBJECTS_END);
            // impostor cards (last: they rebind vertex buffer 0)
            if n_sprites > 0 {
                pass.set_pipeline(&self.impostors.sprite);
                pass.set_bind_group(3, &self.impostors.bind_group, &[]);
                pass.set_vertex_buffer(0, self.impostors.sprites.slice(..));
                pass.draw(0..6, 0..n_sprites as u32);
            }
        }

        prof.push(("scene_a", Instant::now()));
        mark(&mut encoder, MARK_SCENE_A_END);
        // ---- scene copy (refraction / SSR source)
        let size = wgpu::Extent3d {
            width: self.config.width.max(1),
            height: self.config.height.max(1),
            depth_or_array_layers: 1,
        };
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.targets.color_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyTextureInfo {
                texture: &self.targets.scene_copy_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            size,
        );

        mark(&mut encoder, MARK_COPY_END);
        // ---- scene pass B: SSR, water, blended faces, particles
        {
            let (view, resolve) = match &self.targets.color_msaa {
                Some(msaa) => (msaa, Some(&self.targets.color)),
                None => (&self.targets.color, None),
            };
            let depth = self.targets.depth_msaa.as_ref().unwrap_or(&self.targets.depth_ss);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene b"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: resolve,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: if single { wgpu::StoreOp::Store } else { wgpu::StoreOp::Discard },
                    },
                })],
                // read-only depth: also sampled through the frame bind group
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth,
                    depth_ops: None,
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            bind_mesh(&mut pass, &self.groups.b);
            if let (Some(ssr), Some(gbg)) = (&self.pipelines.ssr, &self.gbuf_group) {
                pass.set_pipeline(ssr);
                pass.set_bind_group(3, gbg, &[]);
                pass.draw(0..3, 0..1);
            }
            pass_mark(&mut pass, MARK_SSR_END);
            draw(&mut pass, &self.pipelines.water, rg.water);
            pass_mark(&mut pass, MARK_WATER_END);
            if glow_on {
                if glow_skip & 2 == 0 {
                    draw(&mut pass, &self.pipelines.glow_max, rg.glow);
                }
                let stride = std::mem::size_of::<DrawIndexedIndirect>() as u64;
                let (g0, gn) = rg.glow_alpha;
                for i in 0..(if glow_skip & 4 == 0 { gn as u64 } else { 0 }) {
                    draw(&mut pass, &self.pipelines.glow_suppress, (g0 + i * stride, 1));
                    draw(&mut pass, &self.pipelines.glow, (g0 + i * stride, 1));
                }
            }
            pass_mark(&mut pass, MARK_GLOW_END);
            // blended faces back to front; a glowing one adds its glow right
            // after itself, so later layers in front dim it (LLDrawPoolAlpha)
            let stride = std::mem::size_of::<DrawIndexedIndirect>() as u64;
            let (b0, bn) = rg.blend;
            let mut start = 0u32;
            for i in 0..bn {
                if !(glow_on && glow_skip & 8 == 0 && lists.blend_glow.get(i as usize).copied().unwrap_or(false)) {
                    continue;
                }
                draw(&mut pass, &self.pipelines.blend, (b0 + start as u64 * stride, i + 1 - start));
                draw(&mut pass, &self.pipelines.glow, (b0 + i as u64 * stride, 1));
                start = i + 1;
            }
            draw(&mut pass, &self.pipelines.blend, (b0 + start as u64 * stride, bn - start));
            // debug overlays (before the particles, which rebind vertex buffer 0)
            draw(&mut pass, &self.pipelines.debug_red, rg.debug_red);
            draw(&mut pass, &self.pipelines.debug_blue, rg.debug_blue);
            if let Some(wire) = &self.pipelines.select_child_wire {
                draw(&mut pass, wire, rg.select_child);
            }
            if let Some(wire) = &self.pipelines.select_root_wire {
                draw(&mut pass, wire, rg.select_root);
            }
            if let (true, Some(wire)) = (f.wireframe, &self.pipelines.debug_wire) {
                for r in [rg.terrain, rg.opaque, rg.opaque_2s, rg.mask, rg.mask_2s, rg.blend] {
                    draw(&mut pass, wire, r);
                }
            }
            pass_mark(&mut pass, MARK_BLEND_END);
            // particles last: they rebind vertex buffer 0
            if n_particles > 0 {
                pass.set_pipeline(&self.pipelines.particles);
                pass.set_vertex_buffer(0, self.particle_buffer.slice(..));
                pass.draw(0..6, 0..n_particles);
            }
        }

        prof.push(("scene_b", Instant::now()));
        mark(&mut encoder, MARK_SCENE_END);
        // depth under the cursor, read back in a later frame
        self.depth_pick.encode_hover(
            &self.device,
            &self.queue,
            &mut encoder,
            &self.targets.depth_ss,
            (self.config.width, self.config.height),
            unjittered_vp.inverse(),
        );
        let enc_scene = std::mem::replace(
            &mut encoder,
            self.device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame post") }),
        );
        // ---- temporal anti-aliasing
        let mut post_bg = &self.targets.post_bind_group;
        if let Some(taa) = &mut self.taa {
            let w = taa.parity;
            let prev = self.prev_view_proj.unwrap_or(unjittered_vp);
            let u = TaaU {
                inv_view_proj: (fu.proj * fu.view).inverse().to_cols_array_2d(),
                prev_view_proj: prev.to_cols_array_2d(),
                params: [
                    if taa.reset || self.prev_view_proj.is_none() { 1.0 } else { 0.0 },
                    0.1,
                    self.config.width as f32,
                    self.config.height as f32,
                ],
            };
            self.queue.write_buffer(&taa.buffer, 0, bytemuck::bytes_of(&u));
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("taa"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &taa.history[w],
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&taa.pipeline);
                pass.set_bind_group(0, &taa.bind_groups[w], &[]);
                pass.draw(0..3, 0..1);
            }
            let _ = &taa.layout;
            taa.reset = false;
            taa.parity = 1 - w;
            post_bg = &taa.post_bgs[w];
        }
        self.prev_view_proj = Some(unjittered_vp);
        self.last_inv_vp = unjittered_vp.inverse();

        if glow_on {
            self.glow.encode(&mut encoder, post_bg);
        }

        // ---- post (tonemap) to the swapchain
        {
            let ts = match (&self.timer, timer_slot) {
                (Some(t), Some(_)) => Some(wgpu::RenderPassTimestampWrites {
                    query_set: &t.query_set,
                    beginning_of_pass_write_index: None,
                    end_of_pass_write_index: Some(1),
                }),
                _ => None,
            };
            // with SMAA the post pass fills its gamma-encoded input image
            let (post_view, post_pipe, post_ts, smaa_ts) = match &self.smaa_targets {
                Some(st) => (&st.ldr, &self.smaa.post_ldr, None, ts),
                None => (&surface_view, &self.pipelines.post, ts, None),
            };
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("post"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: post_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: post_ts,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(post_pipe);
                pass.set_bind_group(0, post_bg, &[]);
                pass.draw(0..3, 0..1);
            }
            if let Some(st) = &self.smaa_targets {
                self.smaa.encode(&mut encoder, st, &surface_view, smaa_ts);
            }
        }

        if let (Some(t), Some(slot)) = (&self.timer, timer_slot) {
            encoder.resolve_query_set(&t.query_set, 0..GPU_TIMESTAMPS, &t.resolve, 0);
            encoder.copy_buffer_to_buffer(&t.resolve, 0, &t.readback[slot].0, 0, GPU_TIMESTAMPS as u64 * 8);
        }

        // scene only (no interface): copied before egui draws on top
        let mut capture = None;
        if self.capture_scene {
            capture = self.encode_capture(&mut encoder, &surface_tex.texture);
        }

        prof.push(("post", Instant::now()));
        // ---- egui
        let mut extra_cmds = Vec::new();
        if let Some(ui) = &ui {
            let sd = egui_wgpu::ScreenDescriptor {
                size_in_pixels: [self.config.width, self.config.height],
                pixels_per_point: ui.pixels_per_point,
            };
            extra_cmds = self
                .egui
                .update_buffers(&self.device, &self.queue, &mut encoder, ui.primitives, &sd);
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("egui"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &surface_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            let mut pass = pass.forget_lifetime();
            self.egui.render(&mut pass, ui.primitives, &sd);
        }

        prof.push(("egui", Instant::now()));
        if capture.is_none() && self.capture_request {
            capture = self.encode_capture(&mut encoder, &surface_tex.texture);
        }
        // finish() validates, tracks and encodes every command of its
        // encoder: the three run at once (wgpu-core only takes a shared
        // lock there), then are submitted in frame order
        let (pre, scene) = std::thread::scope(|s| {
            let pre = s.spawn(move || enc_pre.finish());
            let scene = s.spawn(move || enc_scene.finish());
            (pre.join(), scene.join())
        });
        let post = encoder.finish();
        match (pre, scene) {
            (Ok(pre), Ok(scene)) => extra_cmds.extend([pre, scene, post]),
            _ => {
                log::error!("render: a command encoder thread panicked");
                return stats;
            }
        }
        prof.push(("finish", Instant::now()));
        self.queue.submit(extra_cmds);
        if let Some((buf, w, h, row)) = capture {
            self.capture_request = false;
            self.capture_scene = false;
            buf.map_async(wgpu::MapMode::Read, .., |_| {});
            let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
            if let Ok(view) = buf.get_mapped_range(..) {
                let bgra = matches!(
                    self.config.format,
                    wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
                );
                let mut out = Vec::with_capacity((w * h * 4) as usize);
                for y in 0..h {
                    let r = &view[(y * row) as usize..(y * row + w * 4) as usize];
                    for px in r.as_chunks::<4>().0 {
                        if bgra {
                            out.extend_from_slice(&[px[2], px[1], px[0], 255]);
                        } else {
                            out.extend_from_slice(&[px[0], px[1], px[2], 255]);
                        }
                    }
                }
                self.captured = Some((w, h, out));
            }
        }

        if let (Some(t), Some(slot)) = (&mut self.timer, timer_slot) {
            let ready = t.readback[slot].1.clone();
            t.readback[slot].2 = true;
            t.readback[slot].0.map_async(wgpu::MapMode::Read, .., move |r| {
                if r.is_ok() {
                    ready.store(true, Ordering::Release);
                }
            });
        }
        self.occlusion.after_submit();
        self.depth_pick.after_submit();
        prof.push(("submit", Instant::now()));
        let encode_ms = t0.elapsed().as_secs_f32() * 1000.0;
        self.queue.present(surface_tex);
        let _ = self.device.poll(wgpu::PollType::Poll);
        prof.push(("present", Instant::now()));
        stats.cpu_phases[0] = (t0 - t_acquire).as_secs_f32() * 1000.0;
        for w in prof.windows(2) {
            if let Some(i) = RENDER_PHASES.iter().position(|p| *p == w[1].0) {
                stats.cpu_phases[i] += (w[1].1 - w[0].1).as_secs_f32() * 1000.0;
            }
        }
        stats.draw_calls = calls.get();
        stats.records_uploaded = self.records.uploaded;
        stats.palettes_uploaded = std::mem::take(&mut self.palettes_uploaded);
        if std::env::var_os("AURORA_PROFILE").is_some() {
            let mut out = format!("acquire={:.2} ", stats.cpu_phases[0]);
            for w in prof.windows(2) {
                out.push_str(&format!("{}={:.2} ", w[1].0, (w[1].1 - w[0].1).as_secs_f32() * 1000.0));
            }
            log::info!("render profile: {out}");
        }

        stats.textures = self.textures.live();
        stats.texture_bytes = self.textures.bytes();
        stats.geometry_bytes = self.geometry.bytes();
        let (vu, iu) = self.geometry.used();
        stats.vertex_used = vu;
        stats.index_used = iu;
        stats.records = self.records.live();
        stats.gpu_ms = self.timer.as_ref().and_then(|t| t.last_ms);
        stats.gpu_elements = self.timer.as_ref().and_then(|t| t.last_elements);
        if std::env::var_os("AURORA_PROFILE").is_some()
            && let Some(e) = stats.gpu_elements
        {
            let mut out = String::new();
            for (el, ms) in GpuElement::ALL.iter().zip(e) {
                out.push_str(&format!("{}={ms:.3} ", el.key()));
            }
            log::info!("gpu profile: {out}");
        }
        stats.cpu_encode_ms = encode_ms;
        stats.water_reflection = do_water_refl;
        stats.mirror = mirror.is_some();
        stats
    }

    /// World position of the opaque surface under a pixel of the last frame
    /// (depth prepass read-back; None for the sky). Blocks on the GPU for one
    /// tiny copy, so call it on clicks only.
    /// Copy the swapchain image to a readback buffer (frame captures).
    fn encode_capture(&self, encoder: &mut wgpu::CommandEncoder, texture: &wgpu::Texture) -> Option<(wgpu::Buffer, u32, u32, u32)> {
        if !self.config.usage.contains(wgpu::TextureUsages::COPY_SRC) {
            return None;
        }
        let w = self.config.width;
        let h = self.config.height;
        let row = (w * 4).div_ceil(256) * 256;
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("capture"),
            size: (row * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        Some((buf, w, h, row))
    }

    pub fn pick_world(&mut self, x: f32, y: f32) -> Option<Vec3> {
        let (w, h) = (self.config.width, self.config.height);
        if x < 0.0 || y < 0.0 || x >= w as f32 || y >= h as f32 {
            return None;
        }
        let px = crate::pick::PickPixel {
            x: x as u32,
            y: y as u32,
            width: w,
            height: h,
            inv_view_proj: self.last_inv_vp,
        };
        self.depth_pick.blocking(&self.device, &self.queue, &self.targets.depth_ss, px)
    }

    /// Like `pick_world` for the hover cursor, without waiting for the GPU:
    /// asks for this pixel in the next frame and returns the last answer
    /// (a frame or two old).
    pub fn hover_pick(&mut self, x: f32, y: f32) -> Option<Vec3> {
        let (w, h) = (self.config.width, self.config.height);
        if x < 0.0 || y < 0.0 || x >= w as f32 || y >= h as f32 {
            return None;
        }
        self.depth_pick.hover(x as u32, y as u32)
    }

    /// World ray under a pixel of the last frame: origin on the near plane,
    /// unit direction.
    pub fn cursor_ray(&self, x: f32, y: f32) -> Option<(Vec3, Vec3)> {
        let (w, h) = (self.config.width as f32, self.config.height as f32);
        let ndc = |z: f32| Vec4::new(x / w * 2.0 - 1.0, 1.0 - y / h * 2.0, z, 1.0);
        let un = |v: Vec4| {
            let p = self.last_inv_vp * v;
            (p.w.abs() > 1e-9).then(|| p.truncate() / p.w).filter(|v| v.is_finite())
        };
        // reverse-Z: 1 is the near plane, small values far away
        let near = un(ndc(1.0))?;
        let far = un(ndc(0.001))?;
        let dir = (far - near).try_normalize()?;
        Some((near, dir))
    }

    /// Project a world position to screen pixels (None if behind camera).
    pub fn project(&self, f: &FrameParams, p: Vec3) -> Option<(f32, f32, f32)> {
        let c = f.proj * f.view * p.extend(1.0);
        if c.w <= 0.01 {
            return None;
        }
        let ndc = c.truncate() / c.w;
        let x = (ndc.x * 0.5 + 0.5) * self.config.width as f32;
        let y = (0.5 - ndc.y * 0.5) * self.config.height as f32;
        Some((x, y, c.w))
    }

    pub fn clear_color_default() -> Vec4 {
        Vec4::new(0.03, 0.04, 0.12, 1.0)
    }
}

/// Size of the largest device-local memory heap (the GPU's own VRAM on a
/// discrete card), read from Vulkan. None on other backends.
fn detect_vram_mb(adapter: &wgpu::Adapter) -> Option<u64> {
    use ash::vk;
    // SAFETY: the hal adapter is only borrowed for this call; the physical
    // device query does not change any state.
    let hal = unsafe { adapter.as_hal::<wgpu::hal::api::Vulkan>() }?;
    let instance = hal.shared_instance().raw_instance();
    let props = unsafe { instance.get_physical_device_memory_properties(hal.raw_physical_device()) };
    props
        .memory_heaps_as_slice()
        .iter()
        .filter(|h| h.flags.contains(vk::MemoryHeapFlags::DEVICE_LOCAL))
        .map(|h| h.size / (1024 * 1024))
        .max()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shadow_anchor_stays_near_and_still() {
        let c = Vec3::new(149.3, 123.8, 23.5);
        let a = shadow_anchor(c, 6.0);
        assert!(a.distance(c) <= 6.0 * 3f32.sqrt());
        // small camera moves inside the cell keep the same anchor
        assert_eq!(shadow_anchor(c + Vec3::new(0.5, -0.4, 0.2), 6.0), a);
        assert_eq!(shadow_anchor(Vec3::new(-0.1, 0.0, 0.0), 6.0), Vec3::new(-6.0, 0.0, 0.0));
    }

    #[test]
    fn reflection_matrix_mirrors_about_plane() {
        // water at z = 20
        let m = reflection_matrix(Vec3::Z, -20.0);
        let p = m.transform_point3(Vec3::new(3.0, 4.0, 25.0));
        assert!((p - Vec3::new(3.0, 4.0, 15.0)).length() < 1e-5);
        // tilted plane through (1, 0, 0) with normal +X
        let m = reflection_matrix(Vec3::X, -1.0);
        let p = m.transform_point3(Vec3::new(3.0, 2.0, 1.0));
        assert!((p - Vec3::new(-1.0, 2.0, 1.0)).length() < 1e-5);
    }

    #[test]
    fn frame_uniform_layout_is_16_byte_aligned() {
        assert_eq!(std::mem::size_of::<FrameU>() % 16, 0);
    }
}
