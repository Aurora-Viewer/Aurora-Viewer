//! Frame orchestration through wgpu's Vulkan backend:
//!
//! 1. cascaded sun shadows
//! 2. single-sample depth prepass (shared by SSAO, water, SSR, particles, TAA)
//! 3. ambient occlusion (optional): the Legacy SSAO at half resolution, or
//!    GTAO in compute (gtao.rs); then the sun's contact shadows in compute
//!    (contact_shadows.rs, optional)
//! 4. planar reflections: water and the nearest mirror (optional, reduced resolution)
//! 5. scene pass A: sky, terrain, opaque and masked geometry (MSAA, HDR);
//!    with MSAA, first a depth prepass of its multisampled depth
//! 6. copy of the resolved scene (refraction / SSR source)
//! 7. scene pass B: SSR, water, alpha-blended faces, particles
//! 8. TAA (optional), tonemapping, egui overlay
//!
//! This is the render thread's half of the renderer (render_thread.rs): the
//! [`Backend`] owns the window surface, the pipelines, the render targets,
//! the per-frame buffers and the readbacks, and turns a frame packet
//! (packet.rs) into GPU work. The scene's data (draw records, culling
//! tables, geometry, texture pages) belongs to the main thread's `Renderer`
//! (main_thread.rs): the backend only sees the handles a packet carries,
//! and it alone uses the queue.

use crate::gpu_cull::{
    BIN_PROBE, BIN_REFL_MIRROR, BIN_REFL_WATER, BIN_SHADOW, BIN_SHADOW_ALPHA, BIN_SPOT, BIN_WATER_EXCLUSION, COUNT_PRE1, COUNT_PRE2,
    CullFrame, GpuCull, MAIN_BINS, REGION_PRE1, REGION_PRE2, SPOT_SHADOWS, frame_flags,
};
use crate::helpers::Helpers;
use crate::light_tiles::{self, LightCulling, LightSets};
use crate::packet::{Capture, FramePacket, FrameResult, SceneGpu, Spent, SurfaceState};
use crate::types::*;
use crate::ui_frame::EguiFrame;
use crate::upload::FrameUploads;
use crate::writes::QueueSink;
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

/// Projector of a light (types.rs `Projector`), kept apart from `LightU`:
/// every lit fragment reads the lights, few of them are projectors.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ProjectorU {
    matrix: [[f32; 4]; 4],
    /// Apex of the frustum, focus.
    origin: [f32; 4],
    /// Direction, depth of the frustum.
    dir: [f32; 4],
    /// Near distance, ambiance.
    params: [f32; 4],
    /// Texture slot.
    tex: [u32; 4],
}

/// Must match `Frame` in common.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FrameU {
    view_proj: [[f32; 4]; 4],
    inv_view_proj: [[f32; 4]; 4],
    eye_view_proj: [[f32; 4]; 4],
    inv_view_proj_rot: [[f32; 4]; 4],
    eye: [f32; 4],
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
    water_air_vp: [[f32; 4]; 4],
    water_air_inv_vp: [[f32; 4]; 4],
    water_air: [f32; 4],
    mirror_plane: [f32; 4],
    mirror_box: [[f32; 4]; 4],
    mirror_view_proj: [[f32; 4]; 4],
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
    sky_optics: [f32; 4],
    sky_optics_tex: [u32; 4],
    anim_clock: [u32; 4],
    selection_color: [f32; 4],
    rez_gradient: [[f32; 4]; 3],
    cascade_vp: [[[f32; 4]; 4]; CASCADES],
    lights: [LightU; MAX_LIGHTS],
    /// Which of `lights` a fragment walks (light_tiles.rs).
    light_sets: LightSets,
    /// Projector of light `i`, for the lights of `light_projectors`.
    projectors: [ProjectorU; MAX_LIGHTS],
    /// Bit `i` (low word, then high word): light `i` is a projector.
    light_projectors: [u32; 4],
    /// World to the clip space of each projector shadow map.
    spot_vp: [[[f32; 4]; 4]; SPOT_SHADOWS],
    /// Light of each projector shadow map (-1: none).
    spot_lights: [i32; 4],
    /// What each map leaves of its shadows unlit: 1 - fade (spotLightF's
    /// shadow_fade).
    spot_fade: [f32; 4],
    /// x: 1 when the shading takes the contact shadows (main view only).
    contact: [f32; 4],
    /// x: common.wgsl FEATURE_* bits, the same in every pass.
    features: [u32; 4],
}

/// Must match `ShadowParams` in object.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ShadowU {
    vp: [[f32; 4]; 4],
    anim_clock: [u32; 4],
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
    bloom: f32,
    bloom_threshold: f32,
    bloom_knee: f32,
    bloom_norm: f32,
    tonemap_mix: f32,
    /// Written on the GPU by `Exposure::encode`; 1 until then.
    auto_exposure: f32,
    /// A cut-out photo (cutout.rs): 1 the snapshot, 2 the live view.
    cutout: f32,
    _pad: f32,
    /// The part of the photo the frame draws ([`photo_part`]).
    photo_part: [f32; 4],
}

/// Where `PostU::auto_exposure` sits in the post uniforms.
const POST_AUTO_EXPOSURE_OFFSET: u64 = std::mem::offset_of!(PostU, auto_exposure) as u64;

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

    /// `post_layouts`: the post pass's groups (scene image, bloom).
    fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        post_layouts: [&wgpu::BindGroupLayout; 2],
        surface_format: wgpu::TextureFormat,
    ) -> Smaa {
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
            unclipped_depth: false,
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
            bind_group_layouts: &[Some(post_layouts[0]), Some(post_layouts[1])],
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

/// The projector shadow maps of a frame: each projector of the frame's list
/// that casts shadows, with the view-projection of its map. None without
/// shadows (reflections, probe faces and impostor pictures draw none).
fn spot_shadow_views(f: &FrameParams) -> [Option<(SpotShadow, Mat4)>; SPOT_SHADOWS] {
    f.spot_shadows.map(|s| {
        let s = s.filter(|_| f.shadows)?;
        let light = f.lights.get(s.light as usize).filter(|_| (s.light as usize) < MAX_LIGHTS)?;
        Some((s, light.projector?.shadow_matrix))
    })
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
            unclipped_depth: false,
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

/// The replay of a frame's write journal is logged over this (ms).
const SLOW_WRITES_MS: f32 = 4.0;

/// Work of the helper threads (`Backend::finishers`).
fn finish_encoder(encoder: wgpu::CommandEncoder) -> wgpu::CommandBuffer {
    encoder.finish()
}

/// View-projection matrix for positions given relative to `eye`, a point
/// at or near the camera of `view`.
///
/// The object shaders subtract `eye` from the translation of the model
/// matrix before anything else, so every product works on distances from
/// the camera, not on region coordinates. High above the ground those are
/// too coarse in single precision (steps of 0.24 mm between 2048 and
/// 4096 m): each vertex then landed up to a pixel away from its place on
/// close objects, somewhere else whenever the view changed (edges trembling
/// while the camera moves). LL viewers have the same steps per object (the
/// model-view matrix is multiplied on the CPU), not per vertex.
///
/// `exact`: `eye` is the very position `view` looks from (the main camera).
/// The relative view is then its rotation alone; the translation of `view`
/// (minus the rotated eye, rounded to those same steps) is left out.
/// Otherwise the translation left is computed in double precision.
fn eye_view_proj(proj: Mat4, view: Mat4, eye: Vec3, exact: bool) -> Mat4 {
    let relative = if exact {
        Mat4::from_mat3(glam::Mat3::from_mat4(view))
    } else {
        (view.as_dmat4() * glam::DMat4::from_translation(eye.as_dvec3())).as_mat4()
    };
    proj * relative
}

/// Inverse of the projection by the view's rotation alone: it turns a screen
/// point into a direction from the camera. The sky took its directions from
/// the full inverse minus the camera position, both rounded to the f32 of
/// region coordinates: at skybox heights they shook by about a pixel from one
/// frame to the next as the camera moved (#1059).
fn inv_view_proj_rot(proj: Mat4, view: Mat4) -> Mat4 {
    let rotation = glam::DMat4::from_mat3(glam::Mat3::from_mat4(view).as_dmat3());
    (proj.as_dmat4() * rotation).inverse().as_mat4()
}

/// AURORA_EYE_RELATIVE=0: object vertices transformed in region coordinates,
/// as before (comparison).
fn eye_relative_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("AURORA_EYE_RELATIVE").is_none_or(|v| v != "0"))
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
    depth_msaa_tex: Option<wgpu::Texture>,
    /// A copy of the scene depth (same size and samples) the rigged blended
    /// faces draw into, writing their depth as they go
    /// (`RiggedBlendDepth::Layered` and `Layers`, made on its first use).
    blend_layers: Option<(wgpu::Texture, wgpu::TextureView)>,
    /// Single-sample depth from the prepass.
    depth_ss_tex: wgpu::Texture,
    depth_ss: wgpu::TextureView,
    /// Nearest water/exclusion surface, ordered by view distance even inside near.
    water_interface: wgpu::TextureView,
    water_mask: wgpu::TextureView,
    scene_copy_tex: wgpu::Texture,
    scene_copy: wgpu::TextureView,
    /// G-buffer (normal, roughness, metallic) for SSR.
    gbuf_msaa: Option<wgpu::TextureView>,
    gbuf: Option<wgpu::TextureView>,
    /// SSAO raw and blurred (half resolution).
    ao: Option<(wgpu::TextureView, wgpu::TextureView, u32, u32)>,
    /// GTAO's targets (gtao.rs), when its method is chosen or `AURORA_AO`
    /// keeps both ready.
    gtao: Option<crate::gtao::Targets>,
    /// The frame groups bind GTAO's occlusion rather than the Legacy one.
    gtao_shown: bool,
    /// The depth both occlusion methods read: a copy of the prepass with the
    /// blended faces at least half covered drawn over it (#882), when one
    /// of them is ready.
    ao_depth: Option<(wgpu::Texture, wgpu::TextureView)>,
    /// The contact shadows' mask (contact_shadows.rs), when they are on or
    /// `AURORA_CONTACT_SHADOWS` keeps it ready.
    contact: Option<crate::contact_shadows::Targets>,
    water_refl: Option<ReflTarget>,
    water_air: Option<ReflTarget>,
    mirror: Option<ReflTarget>,
    post_bind_group: wgpu::BindGroup,
}

struct Pipelines {
    hud_opaque: wgpu::RenderPipeline,
    hud_blend: wgpu::RenderPipeline,
    hud_debug_red: wgpu::RenderPipeline,
    hud_debug_blue: wgpu::RenderPipeline,
    sky: wgpu::RenderPipeline,
    terrain: wgpu::RenderPipeline,
    /// PBR terrain (vs_terrain_pbr / fs_terrain_pbr), masked or not.
    terrain_pbr: wgpu::RenderPipeline,
    pre_terrain: wgpu::RenderPipeline,
    pre_terrain_normal: wgpu::RenderPipeline,
    pre_terrain_msaa: Option<wgpu::RenderPipeline>,
    opaque: wgpu::RenderPipeline,
    opaque_2s: wgpu::RenderPipeline,
    mask: wgpu::RenderPipeline,
    mask_2s: wgpu::RenderPipeline,
    water: wgpu::RenderPipeline,
    water_unclipped: Option<wgpu::RenderPipeline>,
    water_mask: wgpu::RenderPipeline,
    water_exclusion: wgpu::RenderPipeline,
    blend: wgpu::RenderPipeline,
    /// Blended faces lit before their invisible fragments are dropped, for
    /// frames that ask for it ([`BlendDiscard::Alternate`] only).
    blend_late: Option<wgpu::RenderPipeline>,
    /// Blended faces of rigged meshes, drawn before the others and writing
    /// depth from 1/255 (`DrawLists::blend_depth`; into the scene depth with
    /// `RiggedBlendDepth::Write`, into `Targets::blend_layers` with
    /// `Layered`).
    blend_depth: wgpu::RenderPipeline,
    /// The rigged blended faces' depth where at least half covered
    /// (`HalfCovered`, then their colour without depth; `Layered`, after
    /// their colour).
    blend_depth_mask: wgpu::RenderPipeline,
    blend_depth_color: wgpu::RenderPipeline,
    /// Their depth where nearly opaque, before their colour (`Runs`).
    blend_depth_opaque: wgpu::RenderPipeline,
    /// `Split`: their texels at least half covered, writing depth as they
    /// are drawn, then the fainter ones, depth tested only.
    blend_depth_solid: wgpu::RenderPipeline,
    blend_depth_fringe: wgpu::RenderPipeline,
    /// `Layers`: after each object's fringes, their depth from 1/255.
    blend_depth_fringe_depth: wgpu::RenderPipeline,
    particles: wgpu::RenderPipeline,
    water_particles: wgpu::RenderPipeline,
    water_particles_under: wgpu::RenderPipeline,
    glow: wgpu::RenderPipeline,
    glow_max: wgpu::RenderPipeline,
    glow_suppress: wgpu::RenderPipeline,
    /// Debug "Show transparency": red and blue translucent faces.
    debug_red: wgpu::RenderPipeline,
    land_overlay: wgpu::RenderPipeline,
    debug_blue: wgpu::RenderPipeline,
    debug_green: wgpu::RenderPipeline,
    debug_yellow: wgpu::RenderPipeline,
    /// Debug wireframe (None without POLYGON_MODE_LINE).
    debug_wire: Option<wgpu::RenderPipeline>,
    /// Selection wireframes (SilhouetteParentColor / SilhouetteChildColor).
    select_root_wire: Option<wgpu::RenderPipeline>,
    creation_preview: wgpu::RenderPipeline,
    physics_fill: wgpu::RenderPipeline,
    physics_wire: Option<wgpu::RenderPipeline>,
    select_child_wire: Option<wgpu::RenderPipeline>,
    selection_mask: wgpu::RenderPipeline,
    ssr: Option<wgpu::RenderPipeline>,
    pre_opaque: wgpu::RenderPipeline,
    pre_opaque_2s: wgpu::RenderPipeline,
    pre_portal: wgpu::RenderPipeline,
    pre_portal_2s: wgpu::RenderPipeline,
    pre_mask: wgpu::RenderPipeline,
    pre_mask_2s: wgpu::RenderPipeline,
    /// The same four writing GTAO's mesh normals as well
    /// (`gtao::NORMAL_FORMAT`), in the order of the main bins.
    pre_normal: [wgpu::RenderPipeline; MAIN_BINS],
    /// Opaque normal passes with the finite portal cut; ordinary normals
    /// retain their lean vertex path while no avatar intersects a portal.
    pre_normal_portal: [wgpu::RenderPipeline; 2],
    /// Blended faces opaque enough to cast contact shadows, into their
    /// depth (contact_shadows.rs).
    pre_contact: wgpu::RenderPipeline,
    /// `pre_contact` with GTAO's mesh normals, into the occlusion's depth.
    pre_contact_normal: wgpu::RenderPipeline,
    /// Depth prepass into the scene pass's own multisampled depth (MSAA
    /// only): ordinary / portal-clipped, each one-sided and two-sided.
    pre_msaa: Option<[[wgpu::RenderPipeline; 2]; 2]>,
    refl_sky: wgpu::RenderPipeline,
    refl_terrain: wgpu::RenderPipeline,
    refl_terrain_pbr: wgpu::RenderPipeline,
    refl_obj: wgpu::RenderPipeline,
    /// Shadow casters with the alpha test of `fs_shadow`.
    shadow: wgpu::RenderPipeline,
    /// Opaque shadow casters: the same vertices, no fragment shader.
    shadow_depth: wgpu::RenderPipeline,
    /// Terrain casters with the terrain's alpha test (`fs_shadow_terrain`):
    /// the other casters keep `fs_shadow` without the terrain's inputs.
    shadow_terrain: wgpu::RenderPipeline,
    post: wgpu::RenderPipeline,
    /// Faces drawn under a pixel (drawn_pick.rs): culled like the scene's
    /// one-sided and two-sided faces.
    pick: wgpu::RenderPipeline,
    pick_terrain: wgpu::RenderPipeline,
    pick_terrain_mask: wgpu::RenderPipeline,
    pick_2s: wgpu::RenderPipeline,
    pick_visible: wgpu::RenderPipeline,
    pick_overlay: wgpu::RenderPipeline,
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
    /// Last breakdown of `GpuElement::ShadowsDepth` (ms), in
    /// [`GPU_SHADOW_PARTS`] order.
    last_shadow_parts: Option<[f32; 5]>,
    /// Last times of the depth prepasses (ms), in [`GPU_DEPTH`] order.
    last_depth: Option<[f32; 2]>,
    /// Last time of the ambient occlusion (ms).
    last_ssao: Option<f32>,
    /// Last time of the contact shadows (ms).
    last_contact: Option<f32>,
    /// Last time of the bloom levels (ms).
    last_bloom: Option<f32>,
    /// Last times of the rest of the effects (ms), in [`GPU_EFFECT_PARTS`]
    /// order.
    last_effects: Option<[f32; 4]>,
    /// AURORA_MSAA_PREPASS=alt: the variant each readback slot's frame was
    /// drawn with, and the frames measured since the variant last changed.
    slot_prepass: [std::cell::Cell<bool>; 3],
    compare: PrepassCompare,
}

/// AURORA_MSAA_PREPASS=alt: GPU times of the frames drawn with one variant,
/// from one switch to the next.
#[derive(Default)]
struct PrepassCompare {
    prepass: bool,
    /// Frames still to leave out after a switch.
    skip: u32,
    /// Per frame: the whole frame, each [`GpuElement`], each [`GPU_DEPTH`].
    frames: Vec<[f32; COMPARE_KEYS.len()]>,
}

const COMPARE_KEYS: [&str; 3 + GpuElement::ALL.len()] = [
    "gpu",
    "shadows_depth",
    "effects",
    "terrain_sky",
    "objects",
    "water",
    "transparent",
    "post",
    "depth",
    "depth_msaa",
];

impl PrepassCompare {
    /// A measured frame drawn with (or without) the prepass. Logs the
    /// median of each time over the frames of a variant when it ends:
    /// another process's turn on the GPU lands in one step of one frame,
    /// which a median leaves out and a mean does not.
    fn add(&mut self, prepass: bool, times: [f32; COMPARE_KEYS.len()]) {
        if prepass != self.prepass {
            let n = self.frames.len();
            if n >= 20 {
                let mut line = format!("prepass={} frames={n}", if self.prepass { "on" } else { "off" });
                for (k, key) in COMPARE_KEYS.iter().enumerate() {
                    let mut v: Vec<f32> = self.frames.iter().map(|f| f[k]).collect();
                    v.sort_by(f32::total_cmp);
                    line.push_str(&format!(" {key}={:.3}", v[n / 2]));
                }
                log::info!("msaa prepass alt: {line}");
            }
            self.frames.clear();
            self.prepass = prepass;
            // the first frames after a switch are not at their steady cost
            self.skip = 8;
        }
        if self.skip > 0 {
            self.skip -= 1;
        } else {
            self.frames.push(times);
        }
    }
}

/// Timestamps of a frame: 0 first pass, 1 end of post / HUDs, then the marks.
const GPU_TIMESTAMPS: u32 = 2 + GPU_MARKS;
const GPU_MARKS: u32 = 25;
/// Mark indices, in frame order. Those inside the scene passes split them
/// by draw group; a timestamp there waits for the draws before it, so the
/// split is close but not exact (the GPU overlaps neighbouring draws).
/// Measured on the blended faces: when they are heavy, part of their shading
/// is still running at the marks that follow them and is counted there (the
/// particles, then the post element); transparent + post is what stays put.
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
/// Before the shadow pass (after the culling), then inside it after the
/// first and the second cascade.
const MARK_SHADOW_BEGIN: u32 = 17;
const MARK_SHADOW_C0_END: u32 = 18;
const MARK_SHADOW_C1_END: u32 = 19;
/// What `GpuElement::ShadowsDepth` is made of (`ShadowStats::gpu_ms`): the
/// GPU culling, the three cascades, then the depth prepass with the
/// occlusion.
const GPU_SHADOW_PARTS: [(u32, u32); 5] = [
    (0, MARK_SHADOW_BEGIN),
    (MARK_SHADOW_BEGIN, MARK_SHADOW_C0_END),
    (MARK_SHADOW_C0_END, MARK_SHADOW_C1_END),
    (MARK_SHADOW_C1_END, MARK_SHADOWS_END),
    (MARK_SHADOWS_END, MARK_DEPTH_END),
];
/// After the prepass of the multisampled depth, which opens the scene
/// encoder (written right after `MARK_SCENE_BEGIN` without MSAA).
const MARK_DEPTH_MSAA_END: u32 = 20;
/// Around the bloom levels (bloom.rs), after the glow; part of the post.
const MARK_BLOOM_BEGIN: u32 = 21;
const MARK_BLOOM_END: u32 = 22;
/// The depth prepasses on their own, both counted in the first element:
/// single-sample prepass with the occlusion culling, multisampled prepass.
const GPU_DEPTH: [(u32, u32); 2] = [(MARK_SHADOWS_END, MARK_DEPTH_END), (MARK_WATER_INTERFACE_END, MARK_DEPTH_MSAA_END)];
/// After the contact shadows, which follow the occlusion (written there
/// when they are off): inside the effects' range from `MARK_SSAO_END` to
/// `MARK_REFL_END`.
const MARK_CONTACT_END: u32 = 23;
const MARK_WATER_INTERFACE_END: u32 = 24;
const MARK_WATER_AIR_BEGIN: u32 = 25;
const MARK_WATER_AIR_END: u32 = 26;
/// The effects beyond the occlusion and the contact shadows
/// (`RenderStats::gpu_effects`): the planar reflections (water, mirror), with
/// the refracted water light before them; the reflection probe face with its
/// filtering; the scene copy, with the cut-out pass and the submerged water
/// particles before it; the SSR.
const GPU_EFFECT_PARTS: [(u32, u32); 4] = [
    (MARK_CONTACT_END, MARK_WATER_AIR_BEGIN),
    (MARK_WATER_AIR_END, MARK_REFL_END),
    (MARK_SCENE_A_END, MARK_COPY_END),
    (MARK_COPY_END, MARK_SSR_END),
];
/// GPU time ranges `(from, to)` of each kind of element, in
/// [`GpuElement::ALL`] order; together they cover the whole frame.
const GPU_ELEMENTS: [&[(u32, u32)]; GpuElement::ALL.len()] = [
    // shadows and depth
    &[(0, MARK_SHADOWS_END), GPU_DEPTH[0], GPU_DEPTH[1]],
    // effects: SSAO, planar reflections, probes, scene copy, SSR
    &[
        (MARK_DEPTH_END, MARK_SSAO_END),
        (MARK_SSAO_END, MARK_WATER_AIR_BEGIN),
        (MARK_WATER_AIR_END, MARK_REFL_END),
        (MARK_SCENE_A_END, MARK_COPY_END),
        (MARK_COPY_END, MARK_SSR_END),
    ],
    // terrain and sky
    &[(MARK_DEPTH_MSAA_END, MARK_SKY_END), (MARK_SKY_END, MARK_TERRAIN_END)],
    // objects and avatars (opaque, masked), impostor pictures and cards
    &[
        (MARK_TERRAIN_END, MARK_OBJECTS_END),
        (MARK_REFL_END, MARK_SCENE_BEGIN),
        (MARK_OBJECTS_END, MARK_SCENE_A_END),
    ],
    // water
    &[
        (MARK_WATER_AIR_BEGIN, MARK_WATER_AIR_END),
        (MARK_SCENE_BEGIN, MARK_WATER_INTERFACE_END),
        (MARK_SSR_END, MARK_WATER_END),
    ],
    // blended faces and particles
    &[(MARK_GLOW_END, MARK_BLEND_END), (MARK_BLEND_END, MARK_SCENE_END)],
    // post: glow, TAA, tonemap, SMAA and screen-space HUDs
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
            last_shadow_parts: None,
            last_depth: None,
            last_ssao: None,
            last_contact: None,
            last_bloom: None,
            last_effects: None,
            slot_prepass: Default::default(),
            compare: PrepassCompare::default(),
        }
    }

    /// Collect finished readbacks.
    fn harvest(&mut self) {
        for (slot, (buf, ready, pending)) in self.readback.iter_mut().enumerate() {
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
                            self.last_shadow_parts = Some(GPU_SHADOW_PARTS.map(|(a, b)| ms(a, b)));
                            let d = GPU_DEPTH.map(|(a, b)| ms(a, b));
                            self.last_depth = Some(d);
                            self.last_ssao = Some(ms(MARK_DEPTH_END, MARK_SSAO_END));
                            self.last_contact = Some(ms(MARK_SSAO_END, MARK_CONTACT_END));
                            self.last_bloom = Some(ms(MARK_BLOOM_BEGIN, MARK_BLOOM_END));
                            self.last_effects = Some(GPU_EFFECT_PARTS.map(|(a, b)| ms(a, b)));
                            if msaa_prepass() == MsaaPrepass::Alternate {
                                let mut times = [0.0; COMPARE_KEYS.len()];
                                times[0] = ms(0, 1);
                                times[1..1 + e.len()].copy_from_slice(&e);
                                times[1 + e.len()..].copy_from_slice(&d);
                                self.compare.add(self.slot_prepass[slot].get(), times);
                            }
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

/// AURORA_PROFILE_FRAMES: one `render profile` / `gpu profile` log line per
/// frame (~2 lines and 2 file writes per frame). AURORA_PROFILE alone keeps
/// to the viewer's once-a-second summary, which reads the same values from
/// `RenderStats`. Read once: the per-frame variable lookup is not free.
fn profile_frames() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("AURORA_PROFILE_FRAMES").is_some())
}

/// AURORA_MSAA_PREPASS: what the scene pass starts from with MSAA.
#[derive(Clone, Copy, PartialEq)]
enum MsaaPrepass {
    /// The prepass of the multisampled depth (default).
    On,
    /// `0`: an empty depth, as before that prepass.
    Off,
    /// `alt`: one second without, two with, and the GPU times of each
    /// variant in the log (`msaa prepass alt`), to compare the two on a
    /// GPU other processes use too. Three seconds, so that work done
    /// every other second does not always fall on the same variant.
    Alternate,
}

fn msaa_prepass() -> MsaaPrepass {
    static MODE: std::sync::OnceLock<MsaaPrepass> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("AURORA_MSAA_PREPASS").as_deref().map(str::trim) {
        Ok("0") => MsaaPrepass::Off,
        Ok("alt") => MsaaPrepass::Alternate,
        _ => MsaaPrepass::On,
    })
}

/// AURORA_PBR_OCCLUSION: whether a PBR face's reflection and ambient take
/// the occlusion of its material (the red of the packed map, as pbrIbl).
#[derive(Clone, Copy, PartialEq, Debug)]
enum PbrOcclusion {
    On,
    /// `0`: left out, as before.
    Off,
    /// `alt`: left out one second in three, to compare the two on screen.
    Alternate,
}

impl PbrOcclusion {
    fn parse(v: Option<&str>) -> PbrOcclusion {
        match v.map(str::trim) {
            Some("0") => PbrOcclusion::Off,
            Some("alt") => PbrOcclusion::Alternate,
            _ => PbrOcclusion::On,
        }
    }

    fn from_env() -> PbrOcclusion {
        static MODE: std::sync::OnceLock<PbrOcclusion> = std::sync::OnceLock::new();
        *MODE.get_or_init(|| PbrOcclusion::parse(std::env::var("AURORA_PBR_OCCLUSION").ok().as_deref()))
    }

    /// Whether the frame drawn `secs` whole seconds after the start applies it.
    fn applied(self, secs: u64) -> bool {
        match self {
            PbrOcclusion::On => true,
            PbrOcclusion::Off => false,
            PbrOcclusion::Alternate => !secs.is_multiple_of(3),
        }
    }
}

/// AURORA_SPECULAR_AA: the geometric specular anti-aliasing of the objects
/// (common.wgsl `specular_aa_roughness`), forced on or off for a launch.
#[derive(Clone, Copy, PartialEq, Debug)]
enum SpecularAa {
    /// Unset: Preferences › Graphics decides.
    Setting,
    On,
    Off,
    /// `alt`: on one second, off the next, to compare on screen.
    Alternate,
}

impl SpecularAa {
    fn parse(v: Option<&str>) -> SpecularAa {
        match v.map(str::trim) {
            Some("0") => SpecularAa::Off,
            Some("1") => SpecularAa::On,
            Some("alt") => SpecularAa::Alternate,
            _ => SpecularAa::Setting,
        }
    }

    fn from_env() -> SpecularAa {
        static MODE: std::sync::OnceLock<SpecularAa> = std::sync::OnceLock::new();
        *MODE.get_or_init(|| SpecularAa::parse(std::env::var("AURORA_SPECULAR_AA").ok().as_deref()))
    }

    /// Whether the frame drawn `secs` whole seconds after the start applies
    /// it, the setting being `on`.
    fn applied(self, on: bool, secs: u64) -> bool {
        match self {
            SpecularAa::Setting => on,
            SpecularAa::On => true,
            SpecularAa::Off => false,
            SpecularAa::Alternate => secs.is_multiple_of(2),
        }
    }
}

fn specular_aa_now(on: bool) -> bool {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    let secs = START.get_or_init(Instant::now).elapsed().as_secs();
    SpecularAa::from_env().applied(on, secs)
}

fn pbr_occlusion_now() -> bool {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    let secs = START.get_or_init(Instant::now).elapsed().as_secs();
    PbrOcclusion::from_env().applied(secs)
}

/// Whether this frame takes GTAO's indirect light, the setting being `on`
/// (AURORA_SSIL).
fn gtao_indirect_now(on: bool) -> bool {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    let secs = START.get_or_init(Instant::now).elapsed().as_secs();
    crate::gtao::Indirect::from_env().applied(on, secs)
}

/// Whether this frame takes the specular occlusion of GTAO's bent normal
/// (with GTAO shown; AURORA_GTAO_SPECULAR).
fn gtao_specular_now() -> bool {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    let secs = START.get_or_init(Instant::now).elapsed().as_secs();
    crate::gtao::SpecularOcclusion::from_env().applied(secs)
}

/// Whether GTAO accumulates over the frames this frame
/// (AURORA_GTAO_TEMPORAL).
fn gtao_temporal_now() -> bool {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    let secs = START.get_or_init(Instant::now).elapsed().as_secs();
    crate::gtao::Temporal::from_env().applied(secs)
}

/// Whether GTAO reads the mesh normals this frame (AURORA_GTAO_NORMALS).
fn gtao_mesh_normals_now() -> bool {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    let secs = START.get_or_init(Instant::now).elapsed().as_secs();
    crate::gtao::Normals::from_env().mesh(secs)
}

/// AURORA_PROFILE: the summary also wants the time each step waited.
fn profile() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("AURORA_PROFILE").is_some())
}

/// Ends of the steps of a frame ([`RENDER_PHASES`]), in order: the time,
/// and with AURORA_PROFILE what the thread had run by then
/// (`thread_cpu::cycles`, a system call per mark).
struct Marks {
    cpu: bool,
    ends: Vec<(&'static str, Instant, u64)>,
}

impl Marks {
    /// `first` ended at `at`.
    fn start(first: &'static str, at: Instant) -> Self {
        let cpu = profile();
        Marks {
            cpu,
            ends: vec![(first, at, if cpu { crate::thread_cpu::cycles() } else { 0 })],
        }
    }

    /// `step` ends now.
    fn mark(&mut self, step: &'static str) {
        let cycles = if self.cpu { crate::thread_cpu::cycles() } else { 0 };
        self.ends.push((step, Instant::now(), cycles));
    }

    /// Each step after the first: its name, its time and the part of it
    /// spent off the processor (ms; 0 without the profile).
    fn steps(&self) -> impl Iterator<Item = (&'static str, f32, f32)> + '_ {
        self.ends.windows(2).map(|w| {
            let ms = (w[1].1 - w[0].1).as_secs_f32() * 1000.0;
            let waited = if self.cpu {
                crate::thread_cpu::waited_ms(ms, w[1].2.wrapping_sub(w[0].2))
            } else {
                0.0
            };
            (w[1].0, ms, waited)
        })
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
    /// Water mask generation: no reads from the targets being written.
    mask: wgpu::BindGroup,
    /// Prepass and scene pass A: no scene copy / depth reads.
    a: wgpu::BindGroup,
    /// Scene pass B: everything.
    b: wgpu::BindGroup,
    /// Own uniforms: [0] water reflection, [1] mirror, [2] probe,
    /// [3]/[4] impostor pictures, [5] HUD, [6] drawn pick, [7] water exterior,
    /// [8] probe irradiance capture.
    refl: [wgpu::BindGroup; 9],
}

/// What a frame draws into: the swapchain image, presented, or a photo's
/// own texture (`Capture::Snapshot`), read back and dropped.
enum FrameTarget {
    Surface(wgpu::SurfaceTexture),
    Offscreen(wgpu::Texture),
}

impl FrameTarget {
    fn texture(&self) -> &wgpu::Texture {
        match self {
            FrameTarget::Surface(t) => &t.texture,
            FrameTarget::Offscreen(t) => t,
        }
    }
}

/// The passes of a frame and all they own. Lives on the render thread, or
/// in the `Renderer` itself with AURORA_RENDER_THREAD=0.
pub(crate) struct Backend {
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    /// The window's size while a photo frame draws at its own size in
    /// `config` (`Capture::Snapshot`); put back by `result`.
    snapshot_window: Option<(u32, u32)>,
    surface_srgb: bool,
    egui: egui_wgpu::Renderer,
    ui_blur: Option<crate::ui_blur::Blur>,
    ui_glass: Option<crate::ui_glass::Glass>,
    previews: crate::preview::Cache,
    selection: Option<crate::selection::Overlay>,
    hud_selection: Option<crate::selection::Overlay>,
    /// The coverage pass of cut-out photos (cutout.rs), made on first use.
    cutout: Option<crate::cutout::Cutout>,
    frame_layout: wgpu::BindGroupLayout,
    records_layout: wgpu::BindGroupLayout,
    /// Layout of the scene's texture pages (textures.rs), for the pipelines.
    tex_layout: wgpu::BindGroupLayout,
    shadow_layout: wgpu::BindGroupLayout,
    post_layout: wgpu::BindGroupLayout,
    gbuf_layout: wgpu::BindGroupLayout,
    pipelines: Pipelines,
    ssao: Ssao,
    gtao: crate::gtao::Gtao,
    water_effects: crate::water_effects::WaterGpu,
    contact: crate::contact_shadows::ContactShadows,
    /// This frame draws the contact shadows (`AURORA_CONTACT_SHADOWS=alt`
    /// switches them each second).
    contact_now: bool,
    /// This frame's GTAO reads the mesh normals the depth prepass draws
    /// (`AURORA_GTAO_NORMALS=alt` switches them each second).
    gtao_normals_now: bool,
    /// This frame draws the blended faces at least half covered into the
    /// occlusion's depth (`AURORA_BLEND_AO`).
    blend_ao_depth: bool,
    bloom: crate::bloom::Bloom,
    /// The automatic exposure of skies that are not classic (exposure.rs).
    exposure: crate::exposure::Exposure,
    /// The photo viewfinder's luminance histogram (histogram.rs).
    histogram: crate::histogram::Histogram,
    /// The glow was drawn last frame: its image is there for the exposure.
    glow_drawn: bool,
    frame_buffer: wgpu::Buffer,
    refl_buffers: [wgpu::Buffer; 9],
    hud_depth: Option<(u32, u32, wgpu::TextureView)>,
    probes: crate::probes::Probes,
    /// Faces (bit per face) of the probe being captured that have their
    /// irradiance capture: all six, its irradiance comes from it.
    probe_irr_faces: u8,
    groups: FrameGroups,
    gbuf_group: Option<wgpu::BindGroup>,
    ssao_groups: Option<(wgpu::BindGroup, wgpu::BindGroup)>,
    dummies: Dummies,
    lin_clamp: wgpu::Sampler,
    /// The scene's records, palettes and skin bindings; rebuilt when the
    /// frame's `SceneGpu::records_key` differs from `records_key`.
    records_bind_group: wgpu::BindGroup,
    records_key: (u64, u64),
    /// Shadow atlas: one tile per cascade (see `make_shadow_atlas`).
    shadow_view: wgpu::TextureView,
    shadow_buffers: Vec<wgpu::Buffer>,
    shadow_bind_groups: Vec<wgpu::BindGroup>,
    shadow_size: u32,
    /// Opaque casters shared between the cascades (shadow_merge.rs).
    shadow_merge: crate::shadow_merge::ShadowMerge,
    post_buffer: wgpu::Buffer,
    post_sampler: wgpu::Sampler,
    shadow_sampler: wgpu::Sampler,
    glow: Glow,
    /// Depth of field, made the first time it is on.
    dof: Option<crate::dof::Dof>,
    /// The photo's look, made the first time one is on.
    looks: Option<crate::looks::Looks>,
    rlva_spheres: Option<crate::rlva::Spheres>,
    rlva_overlays: Option<crate::rlva::overlay::Overlays>,
    rlva_resolution_divisor: u32,
    rlva_scaled: Option<crate::rlva::resolution::Scaled>,
    smaa: Smaa,
    smaa_targets: Option<SmaaTargets>,
    targets: Targets,
    indirect: wgpu::Buffer,
    indirect_cpu: Vec<DrawIndexedIndirect>,
    occlusion: crate::occlusion::Occlusion,
    /// GPU-driven draw lists: the compute culling of the scene's face and
    /// object tables, and its bins (gpu_cull.rs).
    cull: GpuCull,
    /// The device can draw the GPU lists (and AURORA_CPU_CULL is not set).
    gpu_cull: bool,
    /// Depth under the cursor (hover without waiting, clicks).
    depth_pick: crate::pick::DepthPick,
    /// Faces drawn under a clicked pixel (drawn_pick.rs).
    drawn_pick: crate::drawn_pick::DrawnPick,
    /// The world's labels, drawn behind what the scene has in front of
    /// them (label_depth.rs).
    label_depth: crate::label_depth::LabelDepth,
    impostors: Impostors,
    cull_cpu: Vec<crate::occlusion::CullDraw>,
    particle_buffer: wgpu::Buffer,
    timer: Option<GpuTimer>,
    /// Two threads that finish command encoders beside this one.
    finishers: Helpers<wgpu::CommandEncoder, wgpu::CommandBuffer>,
    msaa_samples: u32,
    vsync: bool,
    /// Presentation modes of the surface, read when it was created.
    present_modes: Vec<wgpu::PresentMode>,
    settings: RenderSettings,
    taa: Option<Taa>,
    prev_view_proj: Option<Mat4>,
    frame_index: u64,
    /// Texture animation clock (tex_anim.rs) and its value for this frame.
    anim_clock: crate::tex_anim::AnimClock,
    /// (x, y the effects' clock, z, w the animated textures')
    anim_now: [u32; 4],
    /// `shadow_cull::path` bits of the frame being drawn.
    shadow_path: u32,
    /// Inverse view-projection of the last rendered frame (picking).
    last_inv_vp: Mat4,
    msaa_supported: Vec<u32>,
    /// Set by the device-lost callback: the GPU is never waited on again
    /// (device_lost.rs).
    lost: crate::device_lost::DeviceLost,
}

/// What the main thread's `Renderer` creates the backend from.
pub(crate) struct BackendInit {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub surface: wgpu::Surface<'static>,
    pub adapter: wgpu::Adapter,
    pub width: u32,
    pub height: u32,
    pub vsync: bool,
    /// The device has TIMESTAMP_QUERY (GPU timers).
    pub timestamps: bool,
    /// MSAA sample counts other than 4 can be used
    /// (TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES).
    pub adapter_specific: bool,
    pub gpu_cull: bool,
    pub tex_layout: wgpu::BindGroupLayout,
    /// The scene's buffers at the start (first records bind group).
    pub scene: SceneGpu,
    pub anim_clock: crate::tex_anim::AnimClock,
    pub lost: crate::device_lost::DeviceLost,
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
    /// Preserve water geometry in front of the near plane.
    unclipped_depth: bool,
}

/// Draws of `list` whose bounding sphere touches the view of `view_proj`
/// (side and near planes; reverse-Z infinite projections have no far plane).
fn cull_to_view(view_proj: Mat4, list: &[ShadowCaster]) -> Vec<DrawCmd> {
    let planes = crate::gpu_cull::view_planes(view_proj).map(|p| p.to_array());
    list.iter()
        .filter(|c| crate::gpu_cull::sphere_in(&planes, c.center, c.radius))
        .map(|c| c.cmd)
        .collect()
}

/// Draws of `list` whose `bounds` touch the view (radius 0: always kept).
fn cull_cmds_to_view(view_proj: Mat4, list: &[DrawCmd]) -> Vec<DrawCmd> {
    let planes = crate::gpu_cull::view_planes(view_proj).map(|p| p.to_array());
    list.iter()
        .filter(|c| {
            c.bounds[3] <= 0.0 || crate::gpu_cull::sphere_in(&planes, Vec3::new(c.bounds[0], c.bounds[1], c.bounds[2]), c.bounds[3])
        })
        .copied()
        .collect()
}

/// A part of scene pass B, drawing over the color of pass A. `last`: the
/// part that resolves the multisampled color (the others keep it for the
/// next part); `write_depth`: the part of the rigged blended faces, the
/// others only read the depth (also sampled through the frame group).
fn begin_scene_b<'a>(
    encoder: &'a mut wgpu::CommandEncoder,
    t: &Targets,
    label: &'static str,
    last: bool,
    write_depth: bool,
) -> wgpu::RenderPass<'a> {
    begin_scene_b_on(encoder, t, label, last, write_depth, t.depth_msaa.as_ref().unwrap_or(&t.depth_ss))
}

/// [`begin_scene_b`] on another depth attachment of the same size and
/// samples (`Targets::blend_layers`).
fn begin_scene_b_on<'a>(
    encoder: &'a mut wgpu::CommandEncoder,
    t: &Targets,
    label: &'static str,
    last: bool,
    write_depth: bool,
    depth: &wgpu::TextureView,
) -> wgpu::RenderPass<'a> {
    let (view, resolve) = match &t.color_msaa {
        Some(msaa) => (msaa, Some(&t.color)),
        None => (&t.color, None),
    };
    let single = resolve.is_none();
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: resolve.filter(|_| last),
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Load,
                store: if single || !last {
                    wgpu::StoreOp::Store
                } else {
                    wgpu::StoreOp::Discard
                },
            },
        })],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: depth,
            depth_ops: write_depth.then_some(wgpu::Operations {
                load: wgpu::LoadOp::Load,
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: None,
        }),
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    })
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
            unclipped_depth: d.unclipped_depth,
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

/// Shadows do not bind the scene frame group. Their paintmap sampler lives
/// with the light's parameters, so alpha terrain can use the same mixer.
pub(crate) fn shadow_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("shadow params"),
        entries: &[
            uniform_entry(0, wgpu::ShaderStages::VERTEX),
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    })
}

pub(crate) fn shadow_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    buffer: &wgpu::Buffer,
    sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("shadow vp"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
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
    /// The cards' depth alone, single sample (contact shadows).
    sprite_depth: wgpu::RenderPipeline,
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
        let (capture_opaque, capture_blend, sprite, sprite_depth) =
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
            sprite_depth,
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
        (self.capture_opaque, self.capture_blend, self.sprite, self.sprite_depth) =
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
    ) -> (
        wgpu::RenderPipeline,
        wgpu::RenderPipeline,
        wgpu::RenderPipeline,
        wgpu::RenderPipeline,
    ) {
        let src = format!(
            "{}\n{}\n{}",
            crate::common_shader_source(),
            crate::water_shader_source(&["object.wgsl"], include_str!("shaders/object.wgsl")),
            crate::water_shader_source(&["env.wgsl"], include_str!("shaders/env.wgsl"))
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
            unclipped_depth: false,
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
        // their depth alone for the contact shadows (single sample, no color)
        let sprite_depth = make_pipeline(
            device,
            &module,
            &pl,
            PipeDesc {
                format: None,
                depth_compare: wgpu::CompareFunction::Greater,
                ..desc("impostor cards depth", "vs_impostor", "fs_impostor_depth", Vb::Sprite, 1, false)
            },
        );
        (capture_opaque, capture_blend, sprite, sprite_depth)
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

/// The projection of the whole image a frame draws with `proj`: the
/// photo's when the frame draws one of its tiles (`FrameParams::photo_tile`),
/// `proj` itself otherwise.
fn whole_proj(proj: Mat4, photo_tile: Mat4) -> Mat4 {
    if photo_tile == Mat4::IDENTITY {
        proj
    } else {
        photo_tile.inverse() * proj
    }
}

/// The part of the whole image a frame draws with `photo_tile` (see
/// [`whole_proj`]), in fractions of it: left, top, width, height (0, 0, 1,
/// 1 outside a tiled photo). The glow keeps the whole photo's 512 x 512
/// texels with it: drawn on a tile's own, its halo shrank with the tile and
/// stopped at its border, a line along every seam.
fn photo_part(photo_tile: Mat4) -> [f32; 4] {
    let (sx, sy) = (photo_tile.x_axis.x, photo_tile.y_axis.y);
    if photo_tile == Mat4::IDENTITY || sx <= 0.0 || sy <= 0.0 {
        return [0.0, 0.0, 1.0, 1.0];
    }
    // the tile's centre in normalised device coordinates (y up)
    let (cx, cy) = (-photo_tile.w_axis.x / sx, -photo_tile.w_axis.y / sy);
    let (w, h) = (1.0 / sx, 1.0 / sy);
    [(cx + 1.0) * 0.5 - w * 0.5, (1.0 - cy) * 0.5 - h * 0.5, w, h]
}

/// Whether this frame uses the receiver bias of before
/// (`AURORA_SHADOW_BIAS`: 0 the new one, 1 the old one, 2 one second in
/// two each).
fn shadow_bias_legacy_now(mode: u8, now: std::time::SystemTime) -> bool {
    match mode {
        1 => true,
        2 => now
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs())
            .is_multiple_of(2),
        _ => false,
    }
}

/// How blended faces take the screen-space occlusion and indirect light
/// (`AURORA_BLEND_AO`, see the frame uniforms).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlendAo {
    /// Those at least half covered are in the occlusion's depth and take
    /// it (the default, #882).
    New,
    /// None takes it (#556).
    None,
    /// All take that of what lies behind them (before #556).
    Old,
    /// `None` and `New` one second in two each.
    Alt,
}

impl BlendAo {
    fn from_env(value: Option<&str>) -> Self {
        match value.map(str::trim) {
            Some("none") => Self::None,
            Some("old") => Self::Old,
            Some("alt") => Self::Alt,
            _ => Self::New,
        }
    }

    /// The mode of the frame drawn at `now`.
    fn now(self, now: std::time::SystemTime) -> Self {
        match self {
            Self::Alt if shadow_bias_legacy_now(2, now) => Self::None,
            Self::Alt => Self::New,
            mode => mode,
        }
    }
}

/// The receiver bias `cascade_shadow` (common.wgsl) subtracts from a
/// cascade's stored depth, in that depth's units: two texels of the
/// cascade, in metres along the light, times the depth per metre of the
/// cascade's matrix. `texel` is a texel of the tile in tile coordinates.
#[cfg(test)]
fn shadow_receiver_bias(m: Mat4, texel: f32, radius: f32) -> f32 {
    let depth_per_metre = Vec3::new(m.x_axis.z, m.y_axis.z, m.z_axis.z).length();
    2.0 * texel * radius * 2.0 * depth_per_metre
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
    terrain_pbr: (u64, u32),
    terrain_mask: (u64, u32),
    opaque: (u64, u32),
    opaque_2s: (u64, u32),
    mask: (u64, u32),
    mask_2s: (u64, u32),
    water: (u64, u32),
    water_exclusion: (u64, u32),
    blend: (u64, u32),
    glow: (u64, u32),
    glow_alpha: (u64, u32),
    debug_red: (u64, u32),
    land_overlay: (u64, u32),
    debug_blue: (u64, u32),
    debug_green: (u64, u32),
    debug_yellow: (u64, u32),
    select_root: (u64, u32),
    creation_preview: (u64, u32),
    physics_shapes: (u64, u32),
    select_child: (u64, u32),
    select_faces: (u64, u32),
    refl_water: (u64, u32),
    water_air: (u64, u32),
    water_air_terrain: (u64, u32),
    water_air_terrain_pbr: (u64, u32),
    refl_mirror: (u64, u32),
    refl_terrain_water: (u64, u32),
    refl_terrain_mirror: (u64, u32),
    refl_terrain_probe: (u64, u32),
    refl_terrain_pbr_water: (u64, u32),
    refl_terrain_pbr_mirror: (u64, u32),
    refl_terrain_pbr_probe: (u64, u32),
    probe: (u64, u32),
    /// CPU casters of each cascade drawn depth only, then those drawn with
    /// the alpha test.
    shadow: [(u64, u32); CASCADES],
    shadow_alpha: [(u64, u32); CASCADES],
    /// Terrain casters of each cascade drawn with the terrain's alpha test.
    shadow_terrain: [(u64, u32); CASCADES],
    /// Casters of the CPU lists in each projector shadow map.
    spot: [(u64, u32); SPOT_SHADOWS],
    spot_terrain: [(u64, u32); SPOT_SHADOWS],
}

impl Backend {
    /// Surface configuration, pipelines, targets and per-frame buffers.
    pub fn new(init: BackendInit) -> Result<Backend, RenderError> {
        let BackendInit {
            device,
            queue,
            surface,
            adapter,
            width,
            height,
            vsync,
            timestamps,
            adapter_specific,
            gpu_cull,
            tex_layout,
            scene,
            anim_clock,
            lost,
        } = init;
        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| matches!(f, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm))
            .or_else(|| caps.formats.first().copied())
            .ok_or_else(|| RenderError::Surface("no surface formats".into()))?;
        let surface_srgb = format.is_srgb();
        // one rule for the creation and for later vsync switches (present.rs)
        let present_modes = caps.present_modes.clone();
        let present_mode = crate::present::choose(&present_modes, vsync, crate::present::forced());
        log::info!(
            "presentation: {} (vsync {}, surface supports {:?})",
            crate::present::name(present_mode),
            if vsync { "on" } else { "off" },
            present_modes
        );
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
                wgpu::BindGroupLayoutEntry {
                    binding: 15,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 16,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                uniform_entry(17, wgpu::ShaderStages::VERTEX_FRAGMENT),
                wgpu::BindGroupLayoutEntry {
                    binding: 18,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                float_tex(19),
                float_tex(21),
                float_tex(23),
                wgpu::BindGroupLayoutEntry {
                    binding: 22,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 20,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                float_tex(3),
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
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
                // the contact shadows' mask (contact_shadows.rs)
                float_tex(13),
                // GTAO's screen-space indirect light (gtao.rs)
                float_tex(14),
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
        let shadow_layout = shadow_layout(&device);
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
            mk_frame_buffer("HUD uniforms"),
            mk_frame_buffer("drawn pick uniforms"),
            mk_frame_buffer("water exterior uniforms"),
            mk_frame_buffer("probe irradiance capture uniforms"),
        ];
        let probes = crate::probes::Probes::new(&device, RenderSettings::default().probe_slots);

        let shadow_size = 2048u32;
        let shadow_view = Self::make_shadow_atlas(&device, shadow_size);
        let shadow_merge = crate::shadow_merge::ShadowMerge::new(&device, &shadow_view, shadow_size, SHADOW_FORMAT);
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
        // one per cascade, then one per projector shadow map
        let shadow_buffers: Vec<wgpu::Buffer> = (0..CASCADES + SPOT_SHADOWS)
            .map(|_| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("shadow vp"),
                    size: std::mem::size_of::<ShadowU>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            })
            .collect();
        let shadow_bind_groups = shadow_buffers
            .iter()
            .map(|b| shadow_bind_group(&device, &shadow_layout, b, &lin_clamp))
            .collect();
        let records_bind_group = Self::make_records_bg(&device, &records_layout, &scene);

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
        let bloom = crate::bloom::Bloom::new(&device, &post_layout);
        let pipelines = Self::make_pipelines(
            &device,
            &frame_layout,
            &records_layout,
            &tex_layout,
            &shadow_layout,
            [&post_layout, &bloom.post_layout],
            &gbuf_layout,
            msaa_samples,
            format,
            settings.ssr,
        );
        let ssao = Self::make_ssao(&device);
        let gtao = crate::gtao::Gtao::new(&device);
        let water_effects = crate::water_effects::WaterGpu::new(&device, 1);
        let contact = crate::contact_shadows::ContactShadows::new(&device);
        let glow = Glow::new(&device, &post_layout);
        let smaa = Smaa::new(&device, &queue, [&post_layout, &bloom.post_layout], format);
        let targets = Self::make_targets(
            &device,
            &post_layout,
            &post_sampler,
            &post_buffer,
            &glow.views[1],
            &gtao,
            &contact,
            config.width,
            config.height,
            msaa_samples,
            &settings,
        );
        let mut exposure = crate::exposure::Exposure::new(&device);
        exposure.bind(&device, &targets.color, &targets.depth_ss, &glow.views[1]);
        let mut histogram = crate::histogram::Histogram::new(&device);
        histogram.bind(&device, &targets.color);
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
            &water_effects,
        );
        let indirect = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("indirect"),
            size: 64 * 1024 * std::mem::size_of::<DrawIndexedIndirect>() as u64,
            usage: wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let depth_pick = crate::pick::DepthPick::new(&device);
        let drawn_pick = crate::drawn_pick::DrawnPick::new(&device, DEPTH_FORMAT);
        let label_depth = crate::label_depth::LabelDepth::new(&device, format);
        let occlusion = crate::occlusion::Occlusion::new(&device);
        let cull = GpuCull::new(&device);
        let impostors = Impostors::new(&device, &frame_layout, &records_layout, &tex_layout, msaa_samples, settings.ssr);
        let particle_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("particles"),
            size: 4096 * std::mem::size_of::<ParticleInstance>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let egui = egui_wgpu::Renderer::new(&device, format, egui_wgpu::RendererOptions::default());
        let timer = timestamps.then(|| GpuTimer::new(&device, &queue));

        let mut r = Backend {
            device,
            queue,
            surface,
            config,
            snapshot_window: None,
            surface_srgb,
            egui,
            ui_blur: None,
            ui_glass: None,
            previews: Default::default(),
            selection: None,
            cutout: None,
            hud_selection: None,
            frame_layout,
            records_layout,
            tex_layout,
            shadow_layout,
            post_layout,
            gbuf_layout,
            pipelines,
            ssao,
            gtao,
            water_effects,
            contact,
            contact_now: false,
            gtao_normals_now: false,
            blend_ao_depth: true,
            bloom,
            exposure,
            histogram,
            glow_drawn: false,
            frame_buffer,
            refl_buffers,
            hud_depth: None,
            probes,
            probe_irr_faces: 0,
            groups,
            gbuf_group: None,
            ssao_groups: None,
            dummies,
            lin_clamp,
            records_bind_group,
            records_key: scene.records_key,
            shadow_view,
            shadow_buffers,
            shadow_bind_groups,
            shadow_size,
            shadow_merge,
            post_buffer,
            post_sampler,
            shadow_sampler,
            glow,
            dof: None,
            looks: None,
            rlva_spheres: None,
            rlva_overlays: None,
            rlva_resolution_divisor: 1,
            rlva_scaled: None,
            smaa,
            smaa_targets: None,
            targets,
            indirect,
            indirect_cpu: Vec::new(),
            occlusion,
            cull,
            gpu_cull,
            depth_pick,
            drawn_pick,
            label_depth,
            impostors,
            cull_cpu: Vec::new(),
            particle_buffer,
            timer,
            finishers: Helpers::new(2, "aurora-render-finish", finish_encoder),
            msaa_samples,
            vsync,
            present_modes,
            settings,
            taa: None,
            prev_view_proj: None,
            frame_index: 0,
            anim_clock,
            anim_now: [0; 4],
            shadow_path: crate::shadow_cull::path::NEW,
            last_inv_vp: Mat4::IDENTITY,
            msaa_supported,
            lost,
        };
        r.rebuild_bind_groups();
        Ok(r)
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

    fn make_records_bg(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, scene: &SceneGpu) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("records"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: scene.records.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: scene.palettes.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: scene.skin_binds.as_entire_binding(),
                },
            ],
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn make_pipelines(
        device: &wgpu::Device,
        frame_layout: &wgpu::BindGroupLayout,
        records_layout: &wgpu::BindGroupLayout,
        tex_layout: &wgpu::BindGroupLayout,
        shadow_layout: &wgpu::BindGroupLayout,
        post_layouts: [&wgpu::BindGroupLayout; 2],
        gbuf_layout: &wgpu::BindGroupLayout,
        samples: u32,
        surface_format: wgpu::TextureFormat,
        ssr: bool,
    ) -> Pipelines {
        let src = format!(
            "{}\n{}\n{}",
            crate::common_shader_source(),
            crate::water_shader_source(&["object.wgsl"], include_str!("shaders/object.wgsl")),
            crate::water_shader_source(&["env.wgsl"], include_str!("shaders/env.wgsl"))
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
            unclipped_depth: false,
        };
        let g = |fs: &'static str, fs_g: &'static str| if ssr { fs_g } else { fs };
        let pipe = |d: PipeDesc| make_pipeline(device, &module, &scene_layout, d);
        let hud_constants = [("HUD_SRGB_TARGET", if surface_format.is_srgb() { 1.0 } else { 0.0 })];
        let hud_desc = |label, blend, depth_write| PipeDesc {
            format: Some(surface_format),
            samples: 1,
            gbuf: false,
            cull: None,
            blend,
            depth_write,
            write_mask: wgpu::ColorWrites::ALL,
            constants: &hud_constants,
            ..base(label, "vs_main", "fs_hud")
        };
        let hud_opaque = pipe(hud_desc("HUD opaque", None, true));
        let hud_blend = pipe(hud_desc("HUD blended", Some(wgpu::BlendState::ALPHA_BLENDING), false));
        let hud_debug = |label, constants| {
            pipe(PipeDesc {
                fs: Some("fs_hud_debug"),
                constants,
                ..hud_desc(label, Some(wgpu::BlendState::ALPHA_BLENDING), false)
            })
        };
        let hud_debug_red = hud_debug(
            "HUD transparency red",
            &[("debug_r", 1.0), ("debug_g", 0.0), ("debug_b", 0.0), ("debug_a", 0.35)],
        );
        let hud_debug_blue = hud_debug(
            "HUD transparency blue",
            &[("debug_r", 0.0), ("debug_g", 0.2), ("debug_b", 1.0), ("debug_a", 0.35)],
        );
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
        let terrain_pbr = pipe(PipeDesc {
            gbuf: ssr,
            ..base("terrain pbr", "vs_terrain_pbr", g("fs_terrain_pbr", "fs_terrain_pbr_g"))
        });
        let opaque = pipe(PipeDesc {
            gbuf: ssr,
            ..base("opaque", "vs_main", g("fs_opaque", "fs_opaque_g"))
        });
        let creation_preview = pipe(PipeDesc {
            depth_write: false,
            ..base("creation hologram", "vs_main", "fs_opaque")
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
        let hardware_water_depth = device.features().contains(wgpu::Features::DEPTH_CLIP_CONTROL);
        log::info!(
            "water interface depth clamp: {}",
            if hardware_water_depth { "hardware" } else { "portable" }
        );
        let water = pipe(PipeDesc {
            cull: None,
            blend: Some(premultiplied()),
            depth_write: false,
            depth_compare: wgpu::CompareFunction::Greater,
            unclipped_depth: hardware_water_depth,
            ..base("water", "vs_water", "fs_water")
        });
        let water_unclipped = (!hardware_water_depth).then(|| {
            pipe(PipeDesc {
                cull: None,
                blend: Some(premultiplied()),
                depth_write: false,
                depth_compare: wgpu::CompareFunction::Greater,
                ..base("portable water depth clamp", "vs_water_unclipped", "fs_water_unclipped")
            })
        });
        // LLDrawPoolWaterExclusion::render: water wins until a nearer,
        // front-facing exclusion surface replaces it. Independent of scene depth.
        let water_mask = pipe(PipeDesc {
            format: Some(wgpu::TextureFormat::R8Unorm),
            samples: 1,
            cull: None,
            depth_compare: wgpu::CompareFunction::Greater,
            ..base("water mask", "vs_water_unclipped", "fs_water_mask")
        });
        let water_exclusion = pipe(PipeDesc {
            format: Some(wgpu::TextureFormat::R8Unorm),
            samples: 1,
            depth_compare: wgpu::CompareFunction::Greater,
            ..base("water exclusion", "vs_main", "fs_water_exclusion")
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
        let blend_discard = BlendDiscard::from_env();
        let blend_pipe = |label: &'static str, early: bool, depth_write: bool| {
            pipe(PipeDesc {
                blend: Some(suppress),
                write_mask: wgpu::ColorWrites::ALL,
                depth_write,
                depth_compare: wgpu::CompareFunction::Greater,
                constants: if early {
                    &[("BLEND_EARLY_DISCARD", 1.0)]
                } else {
                    &[("BLEND_EARLY_DISCARD", 0.0)]
                },
                ..base(label, "vs_main", "fs_blend")
            })
        };
        let blend = blend_pipe("blend", blend_discard != BlendDiscard::Late, false);
        let blend_late = (blend_discard == BlendDiscard::Alternate).then(|| blend_pipe("blend late discard", false, false));
        // LLDrawPoolAlpha::forwardRender(true): rigged alpha writes depth
        // (fragments under 1/255 of alpha are dropped, as minimum_alpha)
        let blend_depth = blend_pipe("blend rigged", true, true);
        // The rigged faces' depth only where at least half covered
        // (fs_depth_contact): a soft fringe no longer hides the glass behind
        // it (#556). Before their colour with `RiggedBlendDepth::HalfCovered`
        // (blend_depth_color, depth tested, nothing written), after it with
        // `Layered` (#935)
        let blend_depth_mask = pipe(PipeDesc {
            fs: Some("fs_depth_contact"),
            blend: None,
            write_mask: wgpu::ColorWrites::empty(),
            depth_write: true,
            depth_compare: wgpu::CompareFunction::Greater,
            ..base("blend rigged depth", "vs_main", "fs_blend")
        });
        // `RiggedBlendDepth::Runs`: their depth where nearly opaque
        // (fs_depth_opaque), before any of their colour (#935)
        let blend_depth_opaque = pipe(PipeDesc {
            fs: Some("fs_depth_opaque"),
            blend: None,
            write_mask: wgpu::ColorWrites::empty(),
            depth_write: true,
            depth_compare: wgpu::CompareFunction::Greater,
            ..base("blend rigged opaque depth", "vs_main", "fs_blend")
        });
        let blend_depth_color = pipe(PipeDesc {
            blend: Some(suppress),
            write_mask: wgpu::ColorWrites::ALL,
            depth_write: false,
            depth_compare: wgpu::CompareFunction::GreaterEqual,
            constants: &[("BLEND_EARLY_DISCARD", 1.0)],
            ..base("blend rigged colour", "vs_main", "fs_blend")
        });
        // `RiggedBlendDepth::Split` (#935): first the texels at least half
        // covered, each writing its depth as it is drawn, as the Legacy
        // Viewer's rigged alpha pass does from 1/255 (LLDrawPoolAlpha::
        // forwardRender(true), LEQUAL); then the fainter ones over them,
        // depth tested, writing nothing
        let blend_depth_solid = pipe(PipeDesc {
            blend: Some(suppress),
            write_mask: wgpu::ColorWrites::ALL,
            depth_write: true,
            depth_compare: wgpu::CompareFunction::GreaterEqual,
            constants: &[("BLEND_EARLY_DISCARD", 1.0), ("BLEND_SPLIT", 1.0)],
            ..base("blend rigged solid", "vs_main", "fs_blend")
        });
        let blend_depth_fringe = pipe(PipeDesc {
            blend: Some(suppress),
            write_mask: wgpu::ColorWrites::ALL,
            depth_write: false,
            depth_compare: wgpu::CompareFunction::GreaterEqual,
            constants: &[("BLEND_EARLY_DISCARD", 1.0), ("BLEND_SPLIT", 2.0)],
            ..base("blend rigged fringe", "vs_main", "fs_blend")
        });
        // `RiggedBlendDepth::Layers` (#1300): after an object's fringes,
        // their depth from 1/255 (MINIMUM_ALPHA), so that what is drawn
        // after them and behind them is hidden, as in the Legacy Viewer
        let blend_depth_fringe_depth = pipe(PipeDesc {
            fs: Some("fs_depth_contact"),
            blend: None,
            write_mask: wgpu::ColorWrites::empty(),
            depth_write: true,
            depth_compare: wgpu::CompareFunction::Greater,
            constants: &[("DEPTH_CONTACT_ALPHA", 0.004)],
            ..base("blend rigged fringe depth", "vs_main", "fs_blend")
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
        let water_particles = pipe(PipeDesc {
            vb: Vb::None,
            cull: None,
            blend: Some(suppress),
            write_mask: wgpu::ColorWrites::ALL,
            depth_write: false,
            depth_compare: wgpu::CompareFunction::Greater,
            ..base("water droplets and bubbles", "vs_water_particle", "fs_water_particle")
        });
        let water_particles_under = pipe(PipeDesc {
            vb: Vb::None,
            cull: None,
            blend: Some(suppress),
            write_mask: wgpu::ColorWrites::ALL,
            depth_write: false,
            depth_compare: wgpu::CompareFunction::Greater,
            ..base("submerged water particles", "vs_water_particle", "fs_water_particle_under")
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
                unclipped_depth: false,
                ..base(label, "vs_main", "fs_debug_flat")
            })
        };
        let land_overlay = pipe(PipeDesc {
            cull: None,
            blend: Some(alpha_blend),
            write_mask: wgpu::ColorWrites::COLOR,
            depth_write: false,
            depth_compare: wgpu::CompareFunction::GreaterEqual,
            ..base("parcel overlays", "vs_land_overlay", "fs_land_overlay")
        });
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
        // Dev Toolbox R3 (pass per face): opaque faces and hidden ones
        let debug_green = debug(
            "debug green",
            &[("debug_r", 0.1), ("debug_g", 0.85), ("debug_b", 0.3), ("debug_a", 0.3)],
            false,
        );
        let debug_yellow = debug(
            "debug yellow",
            &[("debug_r", 1.0), ("debug_g", 0.85), ("debug_b", 0.1), ("debug_a", 0.45)],
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
        let physics_fill = pipe(PipeDesc {
            cull: Some(wgpu::Face::Back),
            blend: Some(alpha_blend),
            write_mask: wgpu::ColorWrites::COLOR,
            depth_write: false,
            depth_compare: wgpu::CompareFunction::GreaterEqual,
            depth_bias: wgpu::DepthBiasState {
                constant: 2,
                slope_scale: 1.0,
                clamp: 0.0,
            },
            constants: &[("debug_r", 0.15), ("debug_g", 0.8), ("debug_b", 0.35), ("debug_a", 0.5)],
            ..base("selected physics shape", "vs_main", "fs_debug_flat")
        });
        let physics_wire = line_mode.then(|| {
            debug(
                "selected physics triangles",
                &[("debug_r", 0.05), ("debug_g", 0.3), ("debug_b", 0.1), ("debug_a", 0.8)],
                true,
            )
        });
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
        // Coverage of the selected material faces, including closed and
        // disconnected surfaces, drawn with the same posed vertices as depth.
        let selection_mask = pipe(PipeDesc {
            cull: None,
            samples: 1,
            format: Some(wgpu::TextureFormat::R8Unorm),
            depth_write: false,
            write_mask: wgpu::ColorWrites::RED,
            depth_bias: wgpu::DepthBiasState {
                constant: 1,
                slope_scale: 0.0,
                clamp: 0.0,
            },
            ..base("selected face coverage", "vs_main", "fs_selection_mask")
        });
        let ssr_pipe = ssr.then(|| {
            let src = format!("{}\n{}", crate::common_shader_source(), include_str!("shaders/ssr.wgsl"));
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
        let pre_portal = pre("prepass portal", "vs_depth", Some("fs_depth_portal"), Some(wgpu::Face::Back));
        let pre_portal_2s = pre("prepass portal 2s", "vs_depth", Some("fs_depth_portal"), None);
        let pre_mask = pre("prepass mask", "vs_main", Some("fs_depth_mask"), Some(wgpu::Face::Back));
        let pre_mask_2s = pre("prepass mask 2s", "vs_main", Some("fs_depth_mask"), None);
        let pre_terrain = pre("terrain depth", "vs_terrain_pbr", Some("fs_terrain_depth"), Some(wgpu::Face::Back));
        let pre_terrain_msaa = (samples > 1).then(|| {
            pipe(PipeDesc {
                format: None,
                depth_compare: wgpu::CompareFunction::Greater,
                ..base("terrain msaa depth", "vs_terrain_pbr", "fs_terrain_depth")
            })
        });
        // both sides: a double-sided blended face casts either way, the
        // nearer one keeps the depth
        let pre_contact = pre("prepass contact", "vs_main", Some("fs_depth_contact"), None);
        // the same with GTAO's mesh normals (gtao.rs)
        let pre_n = |label: &'static str, vs: &'static str, fs: &'static str, cull: Option<wgpu::Face>| {
            pipe(PipeDesc {
                cull,
                samples: 1,
                format: Some(crate::gtao::NORMAL_FORMAT),
                write_mask: wgpu::ColorWrites::ALL,
                depth_compare: wgpu::CompareFunction::Greater,
                ..base(label, vs, fs)
            })
        };
        let pre_normal = [
            pre_n("prepass normals", "vs_depth_normal", "fs_depth_normal", Some(wgpu::Face::Back)),
            pre_n("prepass normals 2s", "vs_depth_normal", "fs_depth_normal", None),
            pre_n("prepass mask normals", "vs_main", "fs_depth_mask_normal", Some(wgpu::Face::Back)),
            pre_n("prepass mask normals 2s", "vs_main", "fs_depth_mask_normal", None),
        ];
        let pre_terrain_normal = pre_n(
            "terrain depth normals",
            "vs_terrain_pbr",
            "fs_terrain_depth_normal",
            Some(wgpu::Face::Back),
        );
        let pre_normal_portal = [
            pre_n(
                "prepass portal normals",
                "vs_main",
                "fs_depth_portal_normal",
                Some(wgpu::Face::Back),
            ),
            pre_n("prepass portal normals 2s", "vs_main", "fs_depth_portal_normal", None),
        ];
        let pre_contact_normal = pre_n("prepass contact normals", "vs_main", "fs_depth_contact_normal", None);
        let pre_msaa = (samples > 1).then(|| {
            let msaa = |label: &'static str, cull: Option<wgpu::Face>, clipped: bool| {
                pipe(PipeDesc {
                    fs: clipped.then_some("fs_depth_portal"),
                    cull,
                    format: None,
                    depth_compare: wgpu::CompareFunction::Greater,
                    ..base(label, "vs_depth", "fs_opaque")
                })
            };
            [
                [
                    msaa("prepass msaa", Some(wgpu::Face::Back), false),
                    msaa("prepass msaa 2s", None, false),
                ],
                [
                    msaa("prepass msaa portal", Some(wgpu::Face::Back), true),
                    msaa("prepass msaa portal 2s", None, true),
                ],
            ]
        });
        // planar reflections (single sample; mirrored winding: no culling
        // here, fs_reflect keeps the side facing the mirrored camera)
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
        let refl_terrain_pbr = refl("reflection terrain pbr", "vs_terrain_pbr", "fs_terrain_pbr", Vb::Mesh);
        // faces drawn under a pixel: record, depth and surface coordinates
        let pick_desc = |label: &'static str, cull: Option<wgpu::Face>| PipeDesc {
            cull,
            samples: 1,
            format: Some(crate::drawn_pick::FORMAT),
            write_mask: wgpu::ColorWrites::ALL,
            depth_compare: wgpu::CompareFunction::Greater,
            ..base(label, "vs_pick", "fs_pick")
        };
        let pick = pipe(pick_desc("drawn pick", Some(wgpu::Face::Back)));
        let pick_terrain = pipe(PipeDesc {
            vs: "vs_terrain",
            fs: Some("fs_terrain_pick"),
            ..pick_desc("terrain pick", Some(wgpu::Face::Back))
        });
        let pick_terrain_mask = pipe(PipeDesc {
            vs: "vs_terrain_pbr",
            fs: Some("fs_terrain_pick_mask"),
            ..pick_desc("terrain mask pick", Some(wgpu::Face::Back))
        });
        let pick_2s = pipe(pick_desc("drawn pick 2s", None));
        let pick_visible = pipe(PipeDesc {
            fs: Some("fs_pick_visible"),
            ..pick_desc("visible pick", Some(wgpu::Face::Back))
        });
        let pick_overlay = pipe(PipeDesc {
            fs: Some("fs_pick_overlay"),
            ..pick_desc("transparency overlay pick", None)
        });
        let refl_obj = refl("reflection objects", "vs_main", "fs_reflect", Vb::Mesh);
        let shadow_desc = |label, fs| PipeDesc {
            label,
            vs: "vs_shadow",
            fs,
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
                // (the merge of two cascades makes up for it: shadow_merge.rs)
                slope_scale: crate::shadow_cull::SLOPE_BIAS,
                clamp: 0.0,
            },
            constants: &[],
            polygon_line: false,
            unclipped_depth: false,
        };
        let shadow = make_pipeline(device, &module, &shadow_pl, shadow_desc("shadow", Some("fs_shadow")));
        // opaque casters: `fs_shadow` discards nothing of them, so the same
        // vertex shader alone writes the same depth, without a texture read
        // per fragment
        let shadow_depth = make_pipeline(device, &module, &shadow_pl, shadow_desc("shadow depth", None));
        let shadow_terrain = make_pipeline(
            device,
            &module,
            &shadow_pl,
            PipeDesc {
                vs: "vs_shadow_terrain",
                ..shadow_desc("shadow terrain", Some("fs_shadow_terrain"))
            },
        );
        let post_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("post"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/post.wgsl").into()),
        });
        let post_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("post"),
            bind_group_layouts: &[Some(post_layouts[0]), Some(post_layouts[1])],
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
                unclipped_depth: false,
            },
        );
        Pipelines {
            creation_preview,
            physics_fill,
            physics_wire,
            hud_opaque,
            hud_blend,
            hud_debug_red,
            hud_debug_blue,
            sky,
            terrain,
            terrain_pbr,
            pre_terrain,
            pre_terrain_normal,
            pre_terrain_msaa,
            opaque,
            opaque_2s,
            mask,
            mask_2s,
            water,
            water_unclipped,
            water_mask,
            water_exclusion,
            blend,
            blend_late,
            blend_depth,
            blend_depth_mask,
            blend_depth_color,
            blend_depth_opaque,
            blend_depth_solid,
            blend_depth_fringe,
            blend_depth_fringe_depth,
            particles,
            water_particles,
            water_particles_under,
            glow,
            glow_max,
            glow_suppress,
            debug_red,
            land_overlay,
            debug_blue,
            debug_green,
            debug_yellow,
            debug_wire,
            select_root_wire,
            select_child_wire,
            selection_mask,
            ssr: ssr_pipe,
            pre_opaque,
            pre_opaque_2s,
            pre_portal,
            pre_portal_2s,
            pre_mask,
            pre_mask_2s,
            pre_normal,
            pre_normal_portal,
            pre_contact,
            pre_contact_normal,
            pre_msaa,
            refl_sky,
            refl_terrain,
            refl_terrain_pbr,
            refl_obj,
            shadow,
            shadow_depth,
            shadow_terrain,
            post,
            pick,
            pick_terrain,
            pick_terrain_mask,
            pick_2s,
            pick_visible,
            pick_overlay,
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
                    unclipped_depth: false,
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
        gtao: &crate::gtao::Gtao,
        contact: &crate::contact_shadows::ContactShadows,
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
        let (depth_msaa_tex, depth_msaa) = match (samples > 1).then(|| {
            make_view(
                device,
                "depth msaa",
                w,
                h,
                DEPTH_FORMAT,
                samples,
                // (read by the coverage of a cut-out photo, cutout.rs)
                sampled | wgpu::TextureUsages::COPY_SRC,
            )
        }) {
            Some((tex, view)) => (Some(tex), Some(view)),
            None => (None, None),
        };

        // read by the cursor picks through a compute pass (pick.rs)
        let (depth_ss_tex, depth_ss) = make_view(
            device,
            "depth prepass",
            w,
            h,
            DEPTH_FORMAT,
            1,
            sampled | wgpu::TextureUsages::COPY_SRC,
        );
        let (scene_copy_tex, scene_copy) = make_view(
            device,
            "scene copy",
            w,
            h,
            HDR_FORMAT,
            1,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        let water_interface = make_view(device, "nearest water interface", w, h, DEPTH_FORMAT, 1, sampled).1;
        let water_mask = make_view(device, "water exclusion mask", w, h, wgpu::TextureFormat::R8Unorm, 1, sampled).1;
        let (gbuf_msaa, gbuf) = if s.ssr {
            (
                (samples > 1).then(|| make_view(device, "gbuffer msaa", w, h, GBUF_FORMAT, samples, att).1),
                Some(make_view(device, "gbuffer", w, h, GBUF_FORMAT, 1, sampled).1),
            )
        } else {
            (None, None)
        };
        let ao_override = crate::gtao::Override::from_env();
        let (legacy_ao, gtao_ao) = ao_override.needed(s.ao_method);
        let ao_depth = (s.ssao > 0 && (legacy_ao || gtao_ao)).then(|| {
            // copied in turn for the contact shadows
            let copies = wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC;
            make_view(device, "occlusion depth", w, h, DEPTH_FORMAT, 1, sampled | copies)
        });
        let ao_depth_view = ao_depth.as_ref().map_or(&depth_ss, |(_, view)| view);
        let ao = (s.ssao > 0 && legacy_ao).then(|| {
            let (hw, hh) = (w.div_ceil(2), h.div_ceil(2));
            (
                make_view(device, "ssao", hw, hh, AO_FORMAT, 1, sampled).1,
                make_view(device, "ssao blur", hw, hh, AO_FORMAT, 1, sampled).1,
                hw,
                hh,
            )
        });
        let refl = |label: &str, scale: f32, sample_depth: bool| {
            (scale > 0.0).then(|| {
                let rw = ((w as f32 * scale.min(1.0)) as u32).max(16);
                let rh = ((h as f32 * scale.min(1.0)) as u32).max(16);
                ReflTarget {
                    color: make_view(device, label, rw, rh, HDR_FORMAT, 1, sampled).1,
                    depth: make_view(device, label, rw, rh, DEPTH_FORMAT, 1, if sample_depth { sampled } else { att }).1,
                    width: rw,
                    height: rh,
                }
            })
        };
        // with the screen-space indirect light, it reads the previous frame
        // from the scene copy
        let indirect = crate::gtao::Indirect::from_env().needed(s.screen_indirect);
        let gtao = (s.ssao > 0 && gtao_ao).then(|| {
            let scene = indirect.then_some(&scene_copy);
            gtao.targets(device, ao_depth_view, scene, w, h, ao_override.level(s.ao_method))
        });
        // the method shown first; `AURORA_AO=alt` switches it each second
        let gtao_shown = gtao.is_some() && !legacy_ao;
        let contact = crate::contact_shadows::Override::from_env()
            .needed(s.contact_shadows)
            .then(|| contact.targets(device, &depth_ss, DEPTH_FORMAT, w, h));
        let water_refl = refl("water reflection", s.water_reflection_scale, false);
        let water_air = refl("water exterior", crate::water_optics::scale(s.water_effects), true);
        let mirror = refl("mirror", s.mirror_scale, false);
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
            depth_msaa_tex,
            blend_layers: None,
            depth_ss,
            water_interface,
            water_mask,
            scene_copy_tex,
            scene_copy,
            gbuf_msaa,
            gbuf,
            ao,
            gtao,
            gtao_shown,
            ao_depth,
            contact,
            depth_ss_tex,
            water_refl,
            water_air,
            mirror,
            post_bind_group,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn make_frame_groups(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        frame_buffer: &wgpu::Buffer,
        refl_buffers: &[wgpu::Buffer; 9],
        shadow_view: &wgpu::TextureView,
        shadow_sampler: &wgpu::Sampler,
        lin_clamp: &wgpu::Sampler,
        dummies: &Dummies,
        t: &Targets,
        probes: &crate::probes::Probes,
        water_effects: &crate::water_effects::WaterGpu,
    ) -> FrameGroups {
        let mk = |label: &str,
                  buffer: &wgpu::Buffer,
                  ao: &wgpu::TextureView,
                  water: &wgpu::TextureView,
                  mirror: &wgpu::TextureView,
                  scene: &wgpu::TextureView,
                  depth: &wgpu::TextureView,
                  contact: &wgpu::TextureView,
                  il: &wgpu::TextureView,
                  sample_water: bool,
                  sample_mask: bool| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 23,
                        resource: wgpu::BindingResource::TextureView(if sample_mask { &t.water_mask } else { &dummies.white }),
                    },
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
                    wgpu::BindGroupEntry {
                        binding: 13,
                        resource: wgpu::BindingResource::TextureView(contact),
                    },
                    wgpu::BindGroupEntry {
                        binding: 14,
                        resource: wgpu::BindingResource::TextureView(il),
                    },
                    wgpu::BindGroupEntry {
                        binding: 15,
                        resource: wgpu::BindingResource::TextureView(&water_effects.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 16,
                        resource: water_effects.events.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 17,
                        resource: water_effects.uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 18,
                        resource: wgpu::BindingResource::TextureView(&water_effects.ground_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 19,
                        resource: wgpu::BindingResource::TextureView(&water_effects.caustics_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 20,
                        resource: wgpu::BindingResource::TextureView(if sample_water { &t.water_interface } else { &dummies.depth }),
                    },
                    wgpu::BindGroupEntry {
                        binding: 21,
                        resource: wgpu::BindingResource::TextureView(if sample_water {
                            t.water_air.as_ref().map_or(&dummies.black, |t| &t.color)
                        } else {
                            &dummies.black
                        }),
                    },
                    wgpu::BindGroupEntry {
                        binding: 22,
                        resource: wgpu::BindingResource::TextureView(if sample_water {
                            t.water_air.as_ref().map_or(&dummies.depth, |t| &t.depth)
                        } else {
                            &dummies.depth
                        }),
                    },
                ],
            })
        };
        let ao = if t.gtao_shown {
            t.gtao.as_ref().map(|g| &g.result)
        } else {
            t.ao.as_ref().map(|a| &a.1)
        }
        .unwrap_or(&dummies.white);
        let water = t.water_refl.as_ref().map(|r| &r.color).unwrap_or(&dummies.black);
        let mirror = t.mirror.as_ref().map(|r| &r.color).unwrap_or(&dummies.black);
        let contact = t.contact.as_ref().map(|c| &c.result).unwrap_or(&dummies.black);
        let il = t
            .gtao
            .as_ref()
            .and_then(|g| g.indirect.as_ref())
            .map(|i| &i.result)
            .unwrap_or(&dummies.black);
        FrameGroups {
            mask: mk(
                "frame mask",
                frame_buffer,
                &dummies.white,
                &dummies.black,
                &dummies.black,
                &dummies.black,
                &dummies.depth,
                &dummies.black,
                &dummies.black,
                false,
                false,
            ),
            a: mk(
                "frame a",
                frame_buffer,
                ao,
                water,
                mirror,
                &dummies.black,
                &dummies.depth,
                contact,
                il,
                false,
                true,
            ),
            b: mk(
                "frame b",
                frame_buffer,
                ao,
                water,
                mirror,
                &t.scene_copy,
                &t.depth_ss,
                contact,
                il,
                true,
                true,
            ),
            refl: [
                mk(
                    "frame water reflection",
                    &refl_buffers[0],
                    &dummies.white,
                    &dummies.black,
                    &dummies.black,
                    &dummies.black,
                    &dummies.depth,
                    &dummies.black,
                    &dummies.black,
                    false,
                    false,
                ),
                mk(
                    "frame mirror",
                    &refl_buffers[1],
                    &dummies.white,
                    &dummies.black,
                    &dummies.black,
                    &dummies.black,
                    &dummies.depth,
                    &dummies.black,
                    &dummies.black,
                    false,
                    false,
                ),
                mk(
                    "frame probe capture",
                    &refl_buffers[2],
                    &dummies.white,
                    &dummies.black,
                    &dummies.black,
                    &dummies.black,
                    &dummies.depth,
                    &dummies.black,
                    &dummies.black,
                    false,
                    false,
                ),
                mk(
                    "frame impostor 0",
                    &refl_buffers[3],
                    &dummies.white,
                    &dummies.black,
                    &dummies.black,
                    &dummies.black,
                    &dummies.depth,
                    &dummies.black,
                    &dummies.black,
                    false,
                    false,
                ),
                mk(
                    "frame impostor 1",
                    &refl_buffers[4],
                    &dummies.white,
                    &dummies.black,
                    &dummies.black,
                    &dummies.black,
                    &dummies.depth,
                    &dummies.black,
                    &dummies.black,
                    false,
                    false,
                ),
                mk(
                    "frame HUD",
                    &refl_buffers[5],
                    &dummies.white,
                    &dummies.black,
                    &dummies.black,
                    &dummies.black,
                    &dummies.depth,
                    &dummies.black,
                    &dummies.black,
                    false,
                    false,
                ),
                mk(
                    "frame drawn pick",
                    &refl_buffers[6],
                    &dummies.white,
                    &dummies.black,
                    &dummies.black,
                    &dummies.black,
                    &dummies.depth,
                    &dummies.black,
                    &dummies.black,
                    false,
                    false,
                ),
                mk(
                    "frame water exterior",
                    &refl_buffers[7],
                    &dummies.white,
                    &dummies.black,
                    &dummies.black,
                    &dummies.black,
                    &dummies.depth,
                    &dummies.black,
                    &dummies.black,
                    false,
                    false,
                ),
                mk(
                    "frame probe irradiance capture",
                    &refl_buffers[8],
                    &dummies.white,
                    &dummies.black,
                    &dummies.black,
                    &dummies.black,
                    &dummies.depth,
                    &dummies.black,
                    &dummies.black,
                    false,
                    false,
                ),
            ],
        }
    }

    /// Whether this frame draws the contact shadows: with the sun shadows
    /// and the mask ready, `AURORA_CONTACT_SHADOWS=alt` every other second.
    fn contact_on(&self, shadows: bool) -> bool {
        static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        let secs = START.get_or_init(Instant::now).elapsed().as_secs();
        shadows
            && self.targets.contact.is_some()
            && crate::contact_shadows::Override::from_env().on_now(self.settings.contact_shadows, secs)
    }

    /// The occlusion method of this frame: `AURORA_AO=alt` switches it each
    /// second, which binds the other result to the frame groups.
    fn show_ao_method(&mut self) {
        static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        let secs = START.get_or_init(Instant::now).elapsed().as_secs();
        let gtao = crate::gtao::Override::from_env().gtao_now(self.settings.ao_method, secs);
        let t = &self.targets;
        let show = t.gtao.is_some() && (gtao || t.ao.is_none());
        if show != t.gtao_shown {
            self.targets.gtao_shown = show;
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
                &self.water_effects,
            );
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
            &self.water_effects,
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
                            resource: wgpu::BindingResource::TextureView(
                                self.targets.ao_depth.as_ref().map_or(&self.targets.depth_ss, |(_, view)| view),
                            ),
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

    fn scene_size(&self) -> (u32, u32) {
        (self.targets.color_tex.width(), self.targets.color_tex.height())
    }

    fn rebuild_targets(&mut self) {
        self.occlusion.invalidate();
        self.depth_pick.scene_changed();
        let scale = if self.snapshot_window.is_some() {
            100
        } else {
            self.settings.render_scale_percent
        };
        let size = crate::rlva::resolution::render_size((self.config.width, self.config.height), self.rlva_resolution_divisor, scale);
        log::info!(
            "world resolution: {}x{} (output {}x{}, scale {}%)",
            size.0,
            size.1,
            self.config.width,
            self.config.height,
            scale
        );
        self.targets = Self::make_targets(
            &self.device,
            &self.post_layout,
            &self.post_sampler,
            &self.post_buffer,
            &self.glow.views[1],
            &self.gtao,
            &self.contact,
            size.0,
            size.1,
            self.msaa_samples,
            &self.settings,
        );
        self.exposure
            .bind(&self.device, &self.targets.color, &self.targets.depth_ss, &self.glow.views[1]);
        self.histogram.bind(&self.device, &self.targets.color);
        self.smaa_targets = (self.settings.aa == AntiAliasing::Smaa).then(|| {
            self.smaa
                .make_targets(&self.device, &self.queue, &self.dummies.white, size.0, size.1, self.surface_srgb)
        });
        self.rebuild_bind_groups();
    }

    /// The surface can be copied from (captures).
    pub fn can_capture(&self) -> bool {
        self.config.usage.contains(wgpu::TextureUsages::COPY_SRC)
    }

    /// Follow the window: size and vsync wanted for the frame being drawn.
    fn apply_surface(&mut self, s: SurfaceState) {
        self.set_vsync(s.vsync);
        if (s.width, s.height) != (self.config.width, self.config.height) {
            self.resize(s.width, s.height);
        }
    }

    fn resize(&mut self, width: u32, height: u32) {
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

    /// Apply quality settings (rebuilds pipelines / targets only when
    /// needed). The anisotropy belongs to the scene's texture table.
    pub fn apply_settings(&mut self, s: RenderSettings) {
        let samples = match s.aa {
            AntiAliasing::Msaa(n) => self.msaa_supported.iter().copied().filter(|&m| m <= n).max().unwrap_or(1),
            _ => 1,
        };
        let pipelines_dirty = samples != self.msaa_samples || s.ssr != self.settings.ssr;
        let targets_dirty = pipelines_dirty
            || s.render_scale_percent != self.settings.render_scale_percent
            || (s.aa == AntiAliasing::Smaa) != (self.settings.aa == AntiAliasing::Smaa)
            || s.ssao.min(1) != self.settings.ssao.min(1)
            || s.ao_method != self.settings.ao_method
            || s.screen_indirect != self.settings.screen_indirect
            || s.contact_shadows != self.settings.contact_shadows
            || s.water_reflection_scale != self.settings.water_reflection_scale
            || crate::water_optics::scale(s.water_effects) != crate::water_optics::scale(self.settings.water_effects)
            || s.mirror_scale != self.settings.mirror_scale;
        let taa_was = self.taa.is_some();
        self.msaa_samples = samples;
        if pipelines_dirty {
            self.pipelines = Self::make_pipelines(
                &self.device,
                &self.frame_layout,
                &self.records_layout,
                &self.tex_layout,
                &self.shadow_layout,
                [&self.post_layout, &self.bloom.post_layout],
                &self.gbuf_layout,
                samples,
                self.config.format,
                s.ssr,
            );
            self.impostors.rebuild_pipelines(
                &self.device,
                &self.frame_layout,
                &self.records_layout,
                &self.tex_layout,
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
            self.shadow_merge.atlas_changed(&self.device, &self.shadow_view, res, SHADOW_FORMAT);
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
                unclipped_depth: false,
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
                self.scene_size().0,
                self.scene_size().1,
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

    fn set_vsync(&mut self, vsync: bool) {
        if vsync == self.vsync {
            return;
        }
        self.vsync = vsync;
        // The rule of the creation (present.rs): switching the setting in a
        // session ends on the mode a launch with that setting presents with.
        let mode = crate::present::choose(&self.present_modes, vsync, crate::present::forced());
        log::info!(
            "presentation: {} (vsync {})",
            crate::present::name(mode),
            if vsync { "on" } else { "off" }
        );
        if mode == self.config.present_mode {
            return;
        }
        self.config.present_mode = mode;
        self.surface.configure(&self.device, &self.config);
    }

    // ---------------------------------------------------------------- frame

    fn cascade_matrices(&self, f: &FrameParams) -> ([Mat4; CASCADES], [f32; CASCADES]) {
        let far = f.far.min(f.shadow_distance).max(16.0);
        let splits = [far * 0.08, far * 0.3, far];
        // the whole photo's proportions when a frame draws one of its tiles
        // (the render size is the tile's): cascades fitted to a tile, centred
        // on the view axis and as narrow as the tile, left the receivers of
        // the tiles off that axis outside them
        let aspect = (self.config.width as f32 * f.photo_tile.x_axis.x) / (self.config.height.max(1) as f32 * f.photo_tile.y_axis.y);
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
            let eye = center - light_dir * (radius + crate::shadow_cull::CASCADE_DEPTH_PAD);
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
            let depth = radius * 2.0 + 2.0 * crate::shadow_cull::CASCADE_DEPTH_PAD;
            let proj = glam::camera::rh::proj::directx::orthographic(-radius, radius, -radius, radius, 0.1, depth);
            out[i] = proj * view;
            near = split;
        }
        (out, splits)
    }

    /// Fill the uniform block for a camera (main view or a reflection).
    ///
    /// `eye`: the position `view` looks from when it is known exactly (the
    /// main camera), for the eye-relative transform of the objects.
    fn frame_uniforms(
        &self,
        f: &FrameParams,
        view: Mat4,
        eye: Option<Vec3>,
        cascades: &[Mat4; CASCADES],
        splits: &[f32; CASCADES],
    ) -> FrameU {
        let vp = f.proj * view;
        let inv_view = view.inverse();
        let camera_pos = inv_view.transform_point3(Vec3::ZERO);
        let mut u = FrameU::zeroed();
        u.selection_color = f.selection_color;
        u.rez_gradient = f.rez_gradient;
        u.view_proj = vp.to_cols_array_2d();
        u.inv_view_proj = vp.inverse().to_cols_array_2d();
        u.inv_view_proj_rot = inv_view_proj_rot(f.proj, view).to_cols_array_2d();
        if eye_relative_enabled() {
            let origin = eye.unwrap_or(camera_pos);
            u.eye_view_proj = eye_view_proj(f.proj, view, origin, eye.is_some()).to_cols_array_2d();
            u.eye = origin.extend(0.0).to_array();
        } else {
            u.eye_view_proj = u.view_proj;
        }
        u.camera_pos = camera_pos.extend(f.time).to_array();
        u.anim_clock = self.anim_now;
        u.sun_dir = f.sun_dir.normalize_or(Vec3::Z).extend(f.sun_visible).to_array();
        u.sun_color = f.sun_color.extend(1.0).to_array();
        u.sky_zenith = f.sky_zenith.extend(1.0).to_array();
        u.sky_horizon = f.sky_horizon.extend(1.0).to_array();
        u.ground_color = f.ground_color.extend(1.0).to_array();
        u.fog = f.fog_color.extend(f.fog_density).to_array();
        let n_lights = f.lights.len().min(MAX_LIGHTS);
        u.params = [f.exposure, f.water_height, if f.shadows { 1.0 } else { 0.0 }, n_lights as f32];
        let pixel_splits = self.shadow_path & crate::shadow_cull::path::PIXEL_SPLITS != 0;
        // the whole list for every fragment; the main view narrows it (render)
        u.light_sets = LightSets::whole();
        u.cascade_splits = [splits[0], splits[1], splits[2], if pixel_splits { 1.0 } else { 0.0 }];
        u.screen = [
            self.scene_size().0 as f32,
            self.scene_size().1 as f32,
            1.0 / self.scene_size().0 as f32,
            1.0 / self.scene_size().1 as f32,
        ];
        let right = inv_view.transform_vector3(Vec3::X).normalize_or(Vec3::X);
        let up = inv_view.transform_vector3(Vec3::Y).normalize_or(Vec3::Z);
        u.cam_right = right.extend(0.5).to_array();
        u.cam_up = up.extend(f.water_height).to_array();
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
        // z: the PBR materials' occlusion applied (eval_material)
        u.sky_ll = [ll_mode, s.distance_multiplier, if pbr_occlusion_now() { 1.0 } else { 0.0 }, 0.0];
        u.sky_obj_light = s.object_sunlight.extend(0.0).to_array();
        u.sky_optics = [s.moisture_level, s.droplet_radius, s.ice_level, 0.0];
        u.sky_optics_tex = [s.rainbow_texture, s.halo_texture, 0, 0];
        // (common.wgsl FEATURE_NO_EMISSIVE)
        u.features[0] = if self.settings.hdr { 0 } else { 1 };
        for (i, m) in cascades.iter().enumerate() {
            u.cascade_vp[i] = m.to_cols_array_2d();
        }
        for (i, l) in f.lights.iter().take(MAX_LIGHTS).enumerate() {
            u.lights[i] = LightU {
                pos_radius: l.position.extend(l.radius).to_array(),
                color_falloff: l.color.extend(l.falloff).to_array(),
            };
            if let Some(p) = &l.projector {
                u.projectors[i] = ProjectorU {
                    matrix: p.matrix.to_cols_array_2d(),
                    origin: p.origin.extend(p.focus).to_array(),
                    dir: p.direction.extend(p.range).to_array(),
                    params: [p.near, p.ambiance, 0.0, 0.0],
                    tex: [p.texture, 0, 0, 0],
                };
                u.light_projectors[i / 32] |= 1 << (i % 32);
            }
        }
        u.spot_lights = [-1; 4];
        for (k, (spot, vp)) in spot_shadow_views(f).into_iter().enumerate().filter_map(|(k, v)| Some((k, v?))) {
            u.spot_vp[k] = vp.to_cols_array_2d();
            u.spot_lights[k] = spot.light as i32;
            u.spot_fade[k] = 1.0 - spot.fade.clamp(0.0, 1.0);
        }
        u
    }

    /// What a frame gives back: its statistics, the readbacks that have
    /// arrived, the packet's buffers.
    fn result(&mut self, index: u64, stats: RenderStats, captured: Option<(u32, u32, Vec<u8>)>, spent: Spent) -> FrameResult {
        let snapshot = self.end_snapshot();
        FrameResult {
            index,
            stats,
            hover: self.depth_pick.latest(),
            picks: self.depth_pick.take_click_answers(),
            drawn: self.drawn_pick.take_answers(),
            cull: self.cull.last,
            captured,
            snapshot,
            histogram: self.histogram.take(),
            spent,
        }
    }

    /// A photo frame draws every pass at the photo's size: the render size
    /// (`config`, which every pass reads) takes it for this frame, the
    /// swapchain keeps the window's (it is not reconfigured).
    fn begin_snapshot(&mut self, s: SnapshotSpec) {
        self.snapshot_window = Some((self.config.width, self.config.height));
        self.config.width = s.width;
        self.config.height = s.height;
        self.rebuild_targets();
    }

    /// Back to the window's size after a photo frame, however it ended.
    /// True when the frame was one.
    fn end_snapshot(&mut self) -> bool {
        let Some((w, h)) = self.snapshot_window.take() else {
            return false;
        };
        // the video memory at the photo's peak (its targets still alive)
        let live = self.device.get_internal_counters().hal;
        log::info!(
            "snapshot {} x {}: {} MB of textures, {} MB of buffers at its peak, {} sample(s) per pixel",
            self.config.width,
            self.config.height,
            live.texture_memory.read().max(0) >> 20,
            live.buffer_memory.read().max(0) >> 20,
            self.msaa_samples
        );
        self.config.width = w;
        self.config.height = h;
        self.rebuild_targets();
        true
    }

    /// End of a frame that cannot be drawn (no swapchain image, or its
    /// encoding failed). Its writes were replayed and its copies are still
    /// submitted: the scene's mirrors count them as sent.
    fn skip(
        &mut self,
        index: u64,
        stats: RenderStats,
        upload_cmds: Option<wgpu::CommandBuffer>,
        uploads: FrameUploads,
        spent: Spent,
    ) -> FrameResult {
        if upload_cmds.is_some() {
            self.queue.submit(upload_cmds);
        }
        uploads.after_submit();
        // (nothing was drawn for the pixel this frame began to answer)
        self.drawn_pick.abandon();
        self.result(index, stats, None, spent)
    }

    /// Render one frame from its packet.
    pub fn render(&mut self, packet: FramePacket) -> FrameResult {
        let FramePacket {
            index,
            mut writes,
            uploads: mut frame_uploads,
            scene,
            params,
            lists: draw_lists,
            ui: mut egui_frame,
            surface,
            settings,
            hover,
            picks,
            drawn,
            capture: wanted,
            histogram,
        } = packet;
        let mut stats = RenderStats::default();
        if let Some(t) = &mut self.timer {
            t.harvest();
        }
        self.occlusion.harvest();
        self.cull.harvest();
        self.depth_pick.harvest();
        self.depth_pick.harvest_clicks();
        self.drawn_pick.harvest();
        self.drawn_pick.ask(drawn);
        let previous_scene = self.scene_size();
        let picks = picks
            .into_iter()
            .map(|(ticket, x, y)| {
                let (x, y) = crate::rlva::resolution::pixel((x, y), (self.config.width, self.config.height), previous_scene);
                (ticket, x, y)
            })
            .collect();
        // the depth under the clicks made on the last frame drawn, before
        // anything of this frame replaces it (targets, depth, camera)
        self.depth_pick.submit_clicks(
            &self.device,
            &self.queue,
            &self.targets.depth_ss,
            previous_scene,
            self.last_inv_vp,
            picks,
        );
        // the window and the settings as the main thread had them for this frame
        self.apply_surface(surface);
        if self.rlva_resolution_divisor != params.rlva_resolution_divisor {
            self.rlva_resolution_divisor = params.rlva_resolution_divisor;
            self.rebuild_targets();
        }
        stats.present_mode = Some(self.config.present_mode);
        if settings != self.settings {
            self.apply_settings(settings);
        }
        let snapshot = match wanted {
            Some(Capture::Snapshot(s)) if s.width > 0 && s.height > 0 => Some(s),
            _ => None,
        };
        if let Some(s) = snapshot {
            self.begin_snapshot(s);
        } else if let Some((x, y)) = hover {
            // (window pixels: a photo frame has none of them)
            let (x, y) = crate::rlva::resolution::pixel((x, y), (self.config.width, self.config.height), self.scene_size());
            self.depth_pick.hover(x, y);
        }
        let world_size = self.scene_size();

        // egui textures first, so they are never lost on a skipped frame
        if let Some(ui) = &mut egui_frame {
            for (id, deltas) in &ui.textures_delta.set {
                for delta in deltas.iter() {
                    self.egui.update_texture(&self.device, &self.queue, *id, delta);
                }
            }
            for id in &ui.textures_delta.free {
                self.egui.free_texture(id);
            }
            // applied: egui's debug builds refuse to drop a delta that is not
            ui.textures_delta.clear();
        }
        // a photo has the interface or not; with it, the window's interface
        // scaled to the photo's height (laid out in points, it keeps its
        // place when the photo has the window's proportions)
        if let Some(s) = snapshot {
            if !s.interface {
                egui_frame = None;
            } else if let (Some(ui), Some((_, window_h))) = (&mut egui_frame, self.snapshot_window) {
                ui.pixels_per_point *= s.height as f32 / window_h.max(1) as f32;
            }
        }
        // what the scene wrote for this frame, in the order it was recorded,
        // before the frame's own uniforms; then its streamed copies, first
        // command buffer of the frame's submit. Done even when the frame
        // cannot be drawn.
        let t_writes = Instant::now();
        let (recorded, counts) = (writes.bytes(), writes.counts());
        writes.replay(&mut QueueSink(&self.queue));
        let replay_ms = t_writes.elapsed().as_secs_f32() * 1000.0;
        let upload_cmds = frame_uploads.take_commands();
        let writes_ms = t_writes.elapsed().as_secs_f32() * 1000.0;
        if writes_ms > SLOW_WRITES_MS {
            // a hitch worth explaining (an arrival: thousands of placeholder
            // textures, megabytes of records, an arena that grows)
            let [buffers, textures, submits] = counts;
            log::info!(
                "frame writes took {writes_ms:.1} ms: {} KB in {buffers} buffer and {textures} texture writes, {submits} submits \
                 ({replay_ms:.1} ms), staged copies finished in {:.1} ms",
                recorded >> 10,
                writes_ms - replay_ms
            );
        }
        let t_acquire = Instant::now();
        let surface_tex = if snapshot.is_some() {
            // a photo is drawn offscreen at its size, never presented
            Some(FrameTarget::Offscreen(self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("snapshot"),
                size: wgpu::Extent3d {
                    width: self.config.width,
                    height: self.config.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.config.format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })))
        } else {
            match self.surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => Some(FrameTarget::Surface(t)),
                wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                    self.surface.configure(&self.device, &self.config);
                    None
                }
                _ => None,
            }
        };
        let Some(surface_tex) = surface_tex else {
            let spent = Spent { writes, lists: draw_lists };
            return self.skip(index, stats, upload_cmds, frame_uploads, spent);
        };
        // CPU encode time excludes waiting for the swapchain (vsync).
        let t0 = Instant::now();
        let mut prof = Marks::start("acquire", t0);
        // ---- the interface's shapes to triangles, off the main thread
        let mut ui = egui_frame.map(EguiFrame::tessellate);
        if let Some(ui) = &ui {
            stats.ui_shapes = ui.shapes as u32;
            stats.ui_meshes = ui.primitives.len() as u32;
            stats.ui_vertices = ui.vertices as u32;
        }
        prof.mark("tessellate");
        let (f, lists) = (&params, &draw_lists);
        let present_view = surface_tex.texture().create_view(&wgpu::TextureViewDescriptor::default());
        // what the interface painted for the world comes first: it is drawn
        // before the HUD attachments, the rest after them
        let labels_end = ui.as_mut().map_or(0, |ui| crate::ui_world::take_end(&mut ui.primitives));
        let blur_boundary = ui.as_mut().and_then(|ui| crate::ui_blur::take_boundary(&mut ui.primitives));
        if let Some(ui) = ui.as_ref().filter(|_| blur_boundary.is_some()) {
            let size = [self.config.width, self.config.height];
            if !self.ui_blur.as_ref().is_some_and(|blur| blur.matches(size, ui.pixels_per_point)) {
                self.ui_blur = Some(crate::ui_blur::Blur::new(
                    &self.device,
                    self.config.format,
                    size,
                    ui.pixels_per_point,
                ));
            }
        } else {
            self.ui_blur = None;
        }
        // Offscreen only while a modal is visible. This works even when the
        // swapchain cannot be sampled or copied, and preserves scene captures.
        let surface_view = self.ui_blur.as_ref().map_or_else(|| present_view.clone(), |blur| blur.view.clone());

        if !lists.select_faces.is_empty() {
            let size = [world_size.0, world_size.1];
            if self.selection.as_ref().is_none_or(|selection| selection.size != size) {
                self.selection = Some(crate::selection::Overlay::new(&self.device, self.config.format, size));
            }
            if let Some(selection) = &self.selection {
                selection.update(&self.queue, f.selection_color, f.selection_pulse, f.selection_scale);
            }
        }
        if !lists.hud_select_faces.is_empty() {
            let size = [self.config.width, self.config.height];
            if self.hud_selection.as_ref().is_none_or(|selection| selection.size != size) {
                self.hud_selection = Some(crate::selection::Overlay::new(&self.device, self.config.format, size));
            }
            if let Some(selection) = &self.hud_selection {
                selection.update(&self.queue, f.selection_color, f.selection_pulse, f.selection_scale);
            }
        }

        // ---- resources: the scene's buffers as they are for this frame
        if scene.records_key != self.records_key {
            self.records_bind_group = Self::make_records_bg(&self.device, &self.records_layout, &scene);
            self.records_key = scene.records_key;
        }
        prof.mark("resources");

        // one animation time for every pass of the frame
        let effects = self.anim_clock.now(f.anim_at.unwrap_or_else(Instant::now));
        let textures = f.tex_anim_at.map_or(effects, |at| self.anim_clock.now(at));
        self.anim_now = [effects[0], effects[1], textures[0], textures[1]];
        self.shadow_path = crate::shadow_cull::current_path();
        let (cascades, splits) = self.cascade_matrices(f);
        let unjittered_vp = f.proj * f.view;
        let jittered;
        // where the scene's depth has a pixel of the image, in pixels (y down)
        let mut depth_jitter = glam::Vec2::ZERO;
        let fu: &FrameParams = if self.taa.is_some() {
            let i = (self.frame_index % 16) + 1;
            let jx = halton(i, 2) * 2.0 / world_size.0 as f32;
            let jy = halton(i, 3) * 2.0 / world_size.1 as f32;
            depth_jitter = glam::Vec2::new(
                halton(i, 2) * self.config.width as f32 / world_size.0 as f32,
                -halton(i, 3) * self.config.height as f32 / world_size.1 as f32,
            );
            let mut j = f.clone();
            j.proj = Mat4::from_translation(Vec3::new(jx, jy, 0.0)) * f.proj;
            jittered = j;
            &jittered
        } else {
            f
        };
        self.frame_index = self.frame_index.wrapping_add(1);

        // ---- planar reflections this frame
        // LLViewerCamera::cameraUnderWater uses a flat region plane. Our
        // displaced surface needs the camera's actual wave height instead.
        let camera_water_level =
            lists
                .water_ground
                .camera_level(f.camera_pos.truncate(), f.time, self.settings.water_effects, f.water_height);
        // Fine interaction height stays on the GPU. Keep both eligible paths
        // within its bounded displacement; their fragments select the side.
        let wake_margin = if self.settings.water_effects.enabled && self.settings.water_effects.interactions {
            0.16
        } else {
            0.0
        };
        let above_water_possible = f.camera_pos.z >= camera_water_level - wake_margin;
        // Only the portable path needs late fragment depth at the waterline.
        // Hardware clamp keeps ordinary early depth testing for the same mesh.
        let aspect = self.config.width as f32 / self.config.height.max(1) as f32;
        let near_radius = f.near * (1.0 + (f.fov_y * 0.5).tan().powi(2) * (1.0 + aspect * aspect)).sqrt();
        let water_near = (f.camera_pos.z - camera_water_level).abs() <= near_radius * 1.3 + wake_margin;
        let do_water_refl = self.targets.water_refl.is_some() && !lists.water.is_empty() && above_water_possible;
        let air_view =
            self.targets.water_air.as_ref().filter(|_| !lists.water.is_empty()).and_then(|t| {
                crate::water_optics::view(f, camera_water_level, self.settings.water_effects, t.width as f32 / t.height as f32)
            });
        let do_water_air = air_view.is_some();
        let air_vp = air_view.map_or(Mat4::IDENTITY, |(v, p)| p * v);
        let mirror = f.mirror.filter(|_| self.targets.mirror.is_some());

        self.show_ao_method();
        self.contact_now = self.contact_on(f.shadows);
        self.gtao_normals_now = self.targets.gtao_shown && gtao_mesh_normals_now();
        let mut main = self.frame_uniforms(fu, fu.view, Some(fu.camera_pos), &cascades, &splits);
        main.cam_up[3] = camera_water_level;
        if let Some((view, proj)) = air_view {
            main.water_air_vp = air_vp.to_cols_array_2d();
            main.water_air_inv_vp = air_vp.inverse().to_cols_array_2d();
            main.water_air[0] = 1.0;
            let mut rf = f.clone();
            rf.proj = proj;
            rf.shadows = false;
            let mut air = self.frame_uniforms(&rf, view, None, &cascades, &splits);
            air.misc = [0.0, 0.0, 0.0, 1.0];
            air.clip_plane = [0.0, 0.0, 1.0, -f.water_height];
            self.queue.write_buffer(&self.refl_buffers[7], 0, bytemuck::bytes_of(&air));
        }
        if self.contact_now {
            main.contact[0] = 1.0;
            // the blended faces opaque enough are in the depth it read
            if crate::contact_shadows::Override::from_env().all_casters() {
                main.contact[1] = 1.0;
            }
        }
        main.misc = [
            // 2: GTAO's occlusion, without the normal map part the Legacy
            // pass misses (normal_map_occlusion, common.wgsl)
            if self.targets.gtao_shown {
                2.0
            } else if self.targets.ao.is_some() {
                1.0
            } else {
                0.0
            },
            if self.settings.ssr { 1.0 } else { 0.0 },
            if do_water_refl { 1.0 } else { 0.0 },
            0.0,
        ];
        // the reflections occluded along GTAO's bent normal
        // (specular_occlusion, common.wgsl)
        main.sky_ll[3] = if self.targets.gtao_shown && gtao_specular_now() { 1.0 } else { 0.0 };
        // GTAO's indirect light this frame (screen_indirect, common.wgsl)
        let indirect_now = self.targets.gtao_shown
            && self.targets.gtao.as_ref().is_some_and(|g| g.indirect.is_some())
            && gtao_indirect_now(self.settings.screen_indirect);
        if let Some(m) = &mirror {
            let n = m.normal.normalize_or(Vec3::Z);
            main.mirror_plane = n.extend(-n.dot(m.point)).to_array();
            main.mirror_box = m.world_to_box.to_cols_array_2d();
            // the camera of the mirror pass (refl_frame below), which
            // mirror_tap projects the reflected rays through
            main.mirror_view_proj = (f.proj * f.view * reflection_matrix(n, -n.dot(m.point))).to_cols_array_2d();
        }
        // local lights by screen tile and depth, for the main view (light_tiles.rs)
        let light_culling = LightCulling::from_env();
        let light_tiles_on = match light_culling {
            LightCulling::Tiles | LightCulling::Check => true,
            LightCulling::All => false,
            LightCulling::Alternate => !f.all_lights,
        };
        if light_tiles_on {
            let size = [world_size.0, world_size.1];
            let lights = &f.lights[..f.lights.len().min(MAX_LIGHTS)];
            // (the eye the shaders measure depths from: the uniform's camera)
            let eye = Vec3::from_slice(&main.camera_pos[..3]);
            main.light_sets = light_tiles::bin(fu.view, fu.proj, eye, size, lights);
            main.light_sets.mode[1] = u32::from(light_culling == LightCulling::Check || f.debug.check_light_sets);
        }
        stats.light_tiles = light_tiles_on;
        // reflection probes by screen tile and depth, for the main view
        // (probes::tiles), and how its fragments walk them (sample_probes_pair)
        let probe_walk = crate::probes::ProbeWalk::from_env();
        let probe_sets = if f.probes.enabled && probe_walk.tiles(f.probe_walk_list) {
            let size = [world_size.0, world_size.1];
            let eye = Vec3::from_slice(&main.camera_pos[..3]);
            crate::probes::tiles(fu.view, fu.proj, eye, size, &f.probes.probes)
        } else {
            LightSets::whole()
        };
        main.light_sets.mode[3] = probe_walk.shader_mode(f.probe_walk_list, f.debug.check_probe_sets);
        // the view shown instead of the image (debug_view in common.wgsl):
        // the cascade each pixel reads and its shadow (AURORA_DEBUG_SHADOWS
        // too), or the ambient occlusion
        static SHADOW_VIEW: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let cascades_asked = *SHADOW_VIEW.get_or_init(|| std::env::var_os("AURORA_DEBUG_SHADOWS").is_some());
        main.light_sets.mode[2] = match (cascades_asked || f.debug.shadows, f.debug.ambient_occlusion) {
            (true, _) => 1,
            (false, true) => 2,
            (false, false) => 0,
        };
        // AURORA_SHADOW_FILTER, to compare with the shadow lookup of before:
        // `gaps` its filter (4 x 4 taps up to two texels apart: lines along
        // the edges), `mapped` its offset along the normal-mapped normal,
        // `before` both (common.wgsl SHADOW_FILTER_GAPS, SHADOW_OFFSET_MAPPED)
        static SHADOW_BEFORE: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
        main.light_sets.mode[2] |= *SHADOW_BEFORE.get_or_init(|| match std::env::var("AURORA_SHADOW_FILTER").as_deref().map(str::trim) {
            Ok("gaps") => 0x100,
            Ok("mapped") => 0x200,
            Ok("before") => 0x300,
            _ => 0,
        });
        // AURORA_SHADOW_BIAS=legacy|new|alt: the receiver bias of before (a
        // fixed 0.0005 of the cascade's depth range), the one in texels, or
        // one second in two each (common.wgsl SHADOW_BIAS_LEGACY)
        static SHADOW_BIAS: std::sync::OnceLock<u8> = std::sync::OnceLock::new();
        let bias = *SHADOW_BIAS.get_or_init(|| match std::env::var("AURORA_SHADOW_BIAS").as_deref().map(str::trim) {
            Ok("legacy") => 1,
            Ok("alt") => 2,
            _ => 0,
        });
        if shadow_bias_legacy_now(bias, std::time::SystemTime::now()) {
            main.light_sets.mode[2] |= 0x2000;
        }
        // AURORA_BLEND_AO=new|none|old|alt: the blended faces at least half
        // covered drawn into the occlusion's depth and taking the
        // screen-space occlusion and indirect light (#882), no blended face
        // taking them (#556), all of them taking those of what lies behind
        // them, or `none` and `new` one second in two each (common.wgsl
        // BLEND_SCREEN_DEPTH, BLEND_SCREEN_TERMS)
        static BLEND_AO: std::sync::OnceLock<BlendAo> = std::sync::OnceLock::new();
        let blend_ao = BLEND_AO
            .get_or_init(|| BlendAo::from_env(std::env::var("AURORA_BLEND_AO").ok().as_deref()))
            .now(std::time::SystemTime::now());
        self.blend_ao_depth = blend_ao == BlendAo::New;
        main.light_sets.mode[2] |= match blend_ao {
            BlendAo::New => 0x10000,
            BlendAo::Old => 0x4000,
            _ => 0,
        };
        // AURORA_TANGENTS=vertex|pixel|derivative|alt (common.wgsl
        // TANGENT_VERTEX, TANGENT_UNIT): the vertices' tangents, the
        // per-pixel frame where a face has none; the per-pixel frame alone;
        // the cotangent frame of before; vertex and pixel a second each
        static TANGENTS: std::sync::OnceLock<u8> = std::sync::OnceLock::new();
        let tangents = *TANGENTS.get_or_init(|| match std::env::var("AURORA_TANGENTS").as_deref().map(str::trim) {
            Ok("derivative") => 0,
            Ok("pixel") => 1,
            Ok("alt") => 3,
            _ => 2,
        });
        let tangents_now = match tangents {
            3 => {
                let second = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs());
                if second.is_multiple_of(2) { 2 } else { 1 }
            }
            t => t,
        };
        main.light_sets.mode[2] |= match tangents_now {
            2 => 0x1800,
            1 => 0x800,
            _ => 0,
        };
        // (common.wgsl SCREEN_INDIRECT)
        if indirect_now {
            main.light_sets.mode[2] |= 0x400;
        }
        // (common.wgsl SPECULAR_AA)
        if specular_aa_now(self.settings.specular_aa) {
            main.light_sets.mode[2] |= 0x8000;
        }
        self.queue.write_buffer(&self.frame_buffer, 0, bytemuck::bytes_of(&main));
        // a pixel whose drawn faces are asked: the same camera, narrowed to it
        let main_vp = Mat4::from_cols_array_2d(&main.view_proj);
        // (asked in window pixels: a photo frame answers none of them)
        let drawn_px = if snapshot.is_none() {
            self.drawn_pick.begin((self.config.width, self.config.height), main_vp.inverse())
        } else {
            None
        };
        if let Some((x, y, _)) = drawn_px {
            let narrow = crate::drawn_pick::narrow(x, y, self.config.width, self.config.height);
            let mut u = main;
            u.view_proj = (narrow * main_vp).to_cols_array_2d();
            u.eye_view_proj = (narrow * Mat4::from_cols_array_2d(&main.eye_view_proj)).to_cols_array_2d();
            self.queue.write_buffer(&self.refl_buffers[6], 0, bytemuck::bytes_of(&u));
        }
        if let Some(view) = lists.hud_view {
            let mut hud = main;
            hud.view_proj = view.matrix().to_cols_array_2d();
            // (HUD records are in screen space: nothing to take away)
            hud.eye_view_proj = hud.view_proj;
            hud.eye = [0.0; 4];
            hud.screen = [
                self.config.width as f32,
                self.config.height as f32,
                1.0 / self.config.width as f32,
                1.0 / self.config.height as f32,
            ];
            self.queue.write_buffer(&self.refl_buffers[5], 0, bytemuck::bytes_of(&hud));
            if self
                .hud_depth
                .as_ref()
                .is_none_or(|(w, h, _)| *w != self.config.width || *h != self.config.height)
            {
                let depth = self.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("HUD depth"),
                    size: wgpu::Extent3d {
                        width: self.config.width,
                        height: self.config.height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: DEPTH_FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                });
                self.hud_depth = Some((self.config.width, self.config.height, depth.create_view(&Default::default())));
            }
        }
        let refl_frame = |this: &Self, n: Vec3, d: f32, keep_offset: f32| {
            let m = reflection_matrix(n, d);
            let mut rf = f.clone();
            rf.shadows = false;
            let mut u = this.frame_uniforms(&rf, f.view * m, None, &cascades, &splits);
            // 2: a reflection pass whose view is mirrored (mirrored_pass, common.wgsl)
            u.misc = [0.0, 0.0, 0.0, 2.0];
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
        self.probes.upload(&self.queue, f.probes.enabled, &f.probes.probes, &probe_sets);
        let capture = f.probes.capture;
        // skies with a probe ambiance light with the probes' irradiance
        // (shade_pbr), which gets a capture of its own
        let irr_capture = capture.is_some() && !f.sky.classic && !f.sky.no_post;
        let mut probe_vp = Mat4::IDENTITY;
        if let Some(c) = &capture {
            let mut cf = f.clone();
            cf.shadows = false;
            cf.proj = glam::camera::rh::proj::directx::perspective_infinite_reverse(std::f32::consts::FRAC_PI_2, 1.0, c.near.max(0.1));
            let mut u = self.frame_uniforms(&cf, crate::probes::face_view(c.origin, c.face), None, &cascades, &splits);
            probe_vp = Mat4::from_cols_array_2d(&u.view_proj);
            // reflection pass: no SSAO / SSR / mirrors / probes, no clip plane
            u.misc = [0.0, 0.0, 0.0, 1.0];
            u.clip_plane = [0.0; 4];
            let r = crate::probes::CAPTURE_RES as f32;
            u.screen = [r, r, 1.0 / r, 1.0 / r];
            self.queue.write_buffer(&self.refl_buffers[2], 0, bytemuck::bytes_of(&u));
            if c.face == 0 {
                self.probe_irr_faces = 0;
            }
            if irr_capture {
                // the irradiance pass of LLReflectionMapManager: no ambient
                // (LLSettingsVOSky::applySpecial), no clouds
                // (LLDrawPoolWLSky::renderDeferred) and, RenderDesaturateIrradiance,
                // the sky's blue_horizon and blue_density at their HSL
                // lightness (draw_color), so that the sky does not tint the
                // world's ambient blue
                let lightness = |c: [f32; 4]| {
                    let l = (c[0].max(c[1]).max(c[2]) + c[0].min(c[1]).min(c[2])) * 0.5;
                    [l, l, l, c[3]]
                };
                u.sky_blue_horizon = lightness(u.sky_blue_horizon);
                u.sky_blue_density = lightness(u.sky_blue_density);
                u.sky_ambient = [0.0, 0.0, 0.0, u.sky_ambient[3]];
                u.sky_cloud_color[3] = 0.0;
                let r = crate::probes::IRR_CAPTURE_RES as f32;
                u.screen = [r, r, 1.0 / r, 1.0 / r];
                self.queue.write_buffer(&self.refl_buffers[8], 0, bytemuck::bytes_of(&u));
                self.probe_irr_faces |= 1 << c.face;
            }
        }
        // impostor pictures: lit like the scene, seen from the camera
        let imp_caps: Vec<&ImpostorCapture> = lists.impostor_captures.iter().take(IMPOSTOR_CAPTURES).collect();
        for (i, c) in imp_caps.iter().enumerate() {
            let mut cf = f.clone();
            cf.shadows = false;
            cf.proj = ortho_reverse_z(c.half.max(0.1), 0.05, c.depth.max(0.5));
            let mut u = self.frame_uniforms(&cf, c.view, None, &cascades, &splits);
            u.misc = [0.0, 0.0, 0.0, 1.0];
            u.clip_plane = [0.0; 4];
            // neither haze (a classic sky's grows with its distance
            // multiplier) nor fog over the few metres to the picture's
            // eye: the card takes those of its real distance
            u.sky_ll[1] = 0.0;
            u.fog[3] = 0.0;
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
        // projector shadow maps of the frame (none without shadows)
        let spot_vp: [Option<Mat4>; SPOT_SHADOWS] = spot_shadow_views(f).map(|v| v.map(|(_, m)| m));
        for (i, m) in cascades.iter().copied().map(Some).chain(spot_vp).enumerate() {
            let Some(m) = m else { continue };
            let u = ShadowU {
                vp: m.to_cols_array_2d(),
                anim_clock: self.anim_now,
            };
            self.queue.write_buffer(&self.shadow_buffers[i], 0, bytemuck::bytes_of(&u));
        }
        // glow passes only when some visible face glows (RenderGlow)
        // (a cut-out photo's alpha holds its coverage instead, cutout.rs)
        let glow_on = f.glow && !f.cutout && (!lists.glow.is_empty() || !lists.glow_alpha.is_empty() || lists.blend_glow.contains(&true));
        // diagnostic: AURORA_GLOW_SKIP bits leave out a family of glowing faces
        // (2 opaque, 4 alpha pool, 8 blended)
        static GLOW_SKIP: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
        let glow_skip = *GLOW_SKIP.get_or_init(|| std::env::var("AURORA_GLOW_SKIP").ok().and_then(|v| v.parse().ok()).unwrap_or(0));
        // Aurora's bloom: its levels kept while on (or alternated), none
        // under a debug view, which replaces the image
        let bloom = {
            static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
            let secs = START.get_or_init(Instant::now).elapsed().as_secs();
            let debug_view = f.debug_glow || f.debug.shadows || f.debug.ambient_occlusion;
            let (strength, keep) = crate::bloom::Override::from_env().strength(if debug_view { 0.0 } else { f.bloom }, secs);
            if keep && !debug_view {
                self.bloom.ensure(&self.device, world_size);
            } else {
                self.bloom.release();
            }
            if self.bloom.norm() > 0.0 { strength.clamp(0.0, 1.0) } else { 0.0 }
        };
        // "HDR and Emissive" off: LLPipeline::renderFinalize skips the
        // luminance, the exposure, the tone mapping and the CAS for
        // gammaCorrect, which only encodes and clamps (legacyGamma kept on
        // skies without a probe ambiance, as no_post below)
        let hdr = self.settings.hdr;
        let post = PostU {
            exposure: if hdr { f.exposure } else { 1.0 },
            // SMAA works on a gamma-encoded 8-bit image
            output_srgb: if self.surface_srgb && self.smaa_targets.is_none() {
                1.0
            } else {
                0.0
            },
            // classic skies are not tone mapped (Firestorm: no_post + legacy gamma)
            tonemapper: if f.sky.no_post || !hdr {
                2.0
            } else {
                match self.settings.tonemapper {
                    Tonemapper::KhronosNeutral => 0.0,
                    Tonemapper::Aces => 1.0,
                }
            },
            fxaa: if self.settings.aa == AntiAliasing::Fxaa { 1.0 } else { 0.0 },
            sharpen: if hdr { f.sharpen.clamp(0.0, 1.0) } else { 0.0 },
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
            bloom,
            bloom_threshold: crate::bloom::THRESHOLD,
            bloom_knee: crate::bloom::KNEE,
            bloom_norm: self.bloom.norm() * crate::bloom::GAIN,
            // RenderTonemapMix on skies that are not classic; a classic sky
            // tone mapped all the same (probe ambiance above 0) is not mixed
            // (LLSettingsSky::getTonemapMix, LLSettingsVOSky::applySpecial)
            tonemap_mix: if f.sky.classic || !hdr {
                0.0
            } else {
                f.tonemap_mix.clamp(0.0, 1.0)
            },
            auto_exposure: 1.0,
            cutout: match (f.cutout, snapshot.is_some()) {
                (false, _) => 0.0,
                (true, true) => 1.0,
                (true, false) => 2.0,
            },
            _pad: 0.0,
            photo_part: photo_part(f.photo_tile),
        };
        self.queue.write_buffer(&self.post_buffer, 0, bytemuck::bytes_of(&post));
        if let (true, Some(t)) = (self.contact_now, &self.targets.contact) {
            // the light through the projection the depth prepass was drawn
            // with (jittered under TAA)
            let light = crate::contact_shadows::light_projection(fu.proj * fu.view, f.sun_dir);
            // an infinite reversed projection keeps its near plane there
            // (centred or not: tiled photos)
            self.contact.write(&self.queue, t, light, fu.proj.w_axis.z);
        }
        if let (true, Some(t)) = (self.targets.gtao_shown, &mut self.targets.gtao) {
            let prev = self.prev_view_proj.unwrap_or(unjittered_vp);
            let intensity = crate::gtao::intensity_override().unwrap_or(self.settings.screen_indirect_intensity);
            let frame = crate::gtao::Frame {
                index: self.frame_index,
                temporal: gtao_temporal_now(),
                previous_camera: self.prev_view_proj.is_some(),
            };
            let normals = self.gtao_normals_now.then_some(f.view);
            let reproj = crate::gtao::reprojection(f.view, prev);
            self.gtao.write(&self.queue, t, f.proj, reproj, intensity, frame, normals);
        } else if self.targets.ao.is_some() {
            let samples = if self.settings.ssao >= 2 { 16.0 } else { 8.0 };
            let u = SsaoU {
                view_proj: unjittered_vp.to_cols_array_2d(),
                inv_view_proj: unjittered_vp.inverse().to_cols_array_2d(),
                camera_pos: f.camera_pos.extend(0.0).to_array(),
                // Firestorm defaults: RenderSSAOScale 500, RenderSSAOMaxScale
                // 200, RenderSSAOFactor 0.3 (LLPipeline::bindDeferredShader);
                // normal_map_occlusion (common.wgsl) carries the same three
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

        // ---- shadow caster culling per cascade (CPU lists: the terrain,
        // and every caster without the GPU lists). Each cascade keeps the
        // casters that can shadow what the view samples in it
        // (shadow_cull.rs); opaque ones are drawn depth only.
        let shadow_path = self.shadow_path;
        let caster_planes = if f.shadows && shadow_path & crate::shadow_cull::path::TIGHT != 0 {
            let (near, far) = crate::shadow_cull::cascade_reach(splits, shadow_path & crate::shadow_cull::path::PIXEL_SPLITS != 0);
            // the receivers of the whole photo, which every tile's cascades
            // cover: the view's volume is centred on its axis, a tile's
            // frustum is not
            let proj = whole_proj(f.proj, f.photo_tile);
            crate::shadow_cull::cascade_caster_planes(
                &crate::shadow_cull::ReceiverView {
                    inv_view: f.view.inverse(),
                    tan_x: 1.0 / proj.x_axis.x.abs().max(1e-6),
                    tan_y: 1.0 / proj.y_axis.y.abs().max(1e-6),
                    near: f.near,
                },
                -f.sun_dir.normalize_or(Vec3::Z),
                near,
                far,
                // (orthographic: the first row of a cascade's matrix is the
                // light's right axis over its half extent)
                cascades.map(|m| 2.0 / (m.row(0).truncate().length().max(1e-6) * self.shadow_size as f32)),
            )
        } else {
            [crate::shadow_cull::CASTER_PLANES_ALL; CASCADES]
        };
        let shadow_depth_only = shadow_path & crate::shadow_cull::path::DEPTH_ONLY != 0 && !draw_lists.portal_clipping;
        // Opaque casters that the cascade before holds are left out of a
        // cascade, which takes their depth from that finer map
        // (shadow_merge.rs): both pairs of cascades, or none.
        let merges: [Option<crate::shadow_cull::Merge>; crate::shadow_merge::STEPS] =
            if f.shadows && shadow_depth_only && shadow_path & crate::shadow_cull::path::MERGE != 0 {
                std::array::from_fn(|i| crate::shadow_cull::merge_of(&cascades, i + 1, self.shadow_size, self.shadow_merge.scratch_size()))
            } else {
                [None; crate::shadow_merge::STEPS]
            };
        let merges: Option<[crate::shadow_cull::Merge; crate::shadow_merge::STEPS]> = match merges {
            [Some(a), Some(b)] => Some([a, b]),
            _ => None,
        };
        let merge_size = if merges.is_some() { self.shadow_size as f32 } else { 0.0 };
        if let Some(merges) = &merges {
            for (step, m) in merges.iter().enumerate() {
                self.shadow_merge.write(&self.queue, step, m, self.shadow_size);
            }
        }
        let mut shadow_lists: [Vec<DrawCmd>; CASCADES] = Default::default();
        let mut shadow_alpha_lists: [Vec<DrawCmd>; CASCADES] = Default::default();
        let mut shadow_terrain_lists: [Vec<DrawCmd>; CASCADES] = Default::default();
        if f.shadows {
            for (ci, m) in cascades.iter().enumerate() {
                let casters = lists.shadow_casters.iter().map(|c| (false, c));
                for (terrain, c) in casters.chain(lists.terrain_shadow_casters.iter().map(|c| (true, c))) {
                    if !crate::gpu_cull::in_cascade(m, c.center, c.radius)
                        || !crate::shadow_cull::casts_into(&caster_planes[ci], c.center, c.radius)
                        || (!c.alpha
                            && shadow_depth_only
                            && crate::shadow_cull::merged_from_finer(&cascades, &caster_planes, merge_size, ci, c.center, c.radius))
                    {
                        continue;
                    }
                    if terrain && (c.alpha || !shadow_depth_only) {
                        shadow_terrain_lists[ci].push(c.cmd);
                    } else if c.alpha || !shadow_depth_only {
                        shadow_alpha_lists[ci].push(c.cmd);
                    } else {
                        shadow_lists[ci].push(c.cmd);
                    }
                }
            }
        }
        // casters of the CPU lists in each projector's shadow map
        let mut spot_lists: [Vec<DrawCmd>; SPOT_SHADOWS] = Default::default();
        let mut spot_terrain_lists: [Vec<DrawCmd>; SPOT_SHADOWS] = Default::default();
        for (k, vp) in spot_vp.iter().enumerate() {
            let Some(vp) = vp else { continue };
            let planes = crate::gpu_cull::view_planes(*vp).map(|p| p.to_array());
            let inside = |c: &&ShadowCaster| crate::gpu_cull::sphere_in(&planes, c.center, c.radius);
            spot_lists[k].extend(lists.shadow_casters.iter().filter(inside).map(|c| c.cmd));
            spot_terrain_lists[k].extend(lists.terrain_shadow_casters.iter().filter(inside).map(|c| c.cmd));
        }

        // ---- indirect args
        self.indirect_cpu.clear();
        let push = |cpu: &mut Vec<DrawIndexedIndirect>, l: &[DrawCmd]| -> (u64, u32) {
            let start = cpu.len() as u64 * std::mem::size_of::<DrawIndexedIndirect>() as u64;
            cpu.extend(l.iter().map(DrawIndexedIndirect::from));
            (start, l.len() as u32)
        };
        let cpu = &mut self.indirect_cpu;
        // a cut-out photo leaves the ground and the water out of the view
        // (cutout.rs); the reflections and the probes keep them, for the
        // light they give
        let (terrain, water): (&[DrawCmd], &[DrawCmd]) = if f.cutout { (&[], &[]) } else { (&lists.terrain, &lists.water) };
        let mut rg = Ranges {
            terrain: push(cpu, terrain),
            terrain_pbr: push(cpu, if f.cutout { &[] } else { &lists.terrain_pbr }),
            terrain_mask: push(cpu, if f.cutout { &[] } else { &lists.terrain_mask }),
            opaque: push(cpu, &lists.opaque),
            opaque_2s: push(cpu, &lists.opaque_two_sided),
            mask: push(cpu, &lists.mask),
            mask_2s: push(cpu, &lists.mask_two_sided),
            water: push(cpu, water),
            water_exclusion: push(cpu, &lists.water_exclusion),
            blend: push(cpu, &lists.blend),
            glow: push(cpu, &lists.glow),
            glow_alpha: push(cpu, &lists.glow_alpha),
            ..Default::default()
        };
        let main_count = cpu.len();
        // GPU draw lists this frame (gpu_cull.rs)
        let gpu = lists.gpu.filter(|_| self.gpu_cull);
        // GPU occlusion of the main draws (AURORA_NO_OCCLUSION turns it off)
        static NO_OCCLUSION: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let occl = self.settings.occlusion
            && (main_count > 0 || gpu.is_some())
            && !*NO_OCCLUSION.get_or_init(|| std::env::var_os("AURORA_NO_OCCLUSION").is_some());
        if occl {
            use crate::occlusion::{CullDraw, KIND_NONE, KIND_PREPASS, KIND_TESTED};
            let mut cull = std::mem::take(&mut self.cull_cpu);
            cull.clear();
            let lists_kinds: [(&[DrawCmd], u32); 12] = [
                (&lists.terrain, KIND_NONE),
                (&lists.terrain_pbr, KIND_NONE),
                (&lists.terrain_mask, KIND_NONE),
                (&lists.opaque, KIND_PREPASS),
                (&lists.opaque_two_sided, KIND_PREPASS),
                (&lists.mask, KIND_PREPASS),
                (&lists.mask_two_sided, KIND_PREPASS),
                (&lists.water, KIND_NONE),
                (&lists.water_exclusion, KIND_NONE),
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
            let records = scene.record_slots;
            self.occlusion.prepare(
                &self.device,
                &self.queue,
                &self.targets.depth_ss,
                world_size.0,
                world_size.1,
                vp,
                &cull,
                records,
            );
            self.cull_cpu = cull;
        }
        if let Some(g) = &gpu {
            let probe = capture.filter(|c| !c.sky_only);
            let mut flags = 0;
            for (on, bit) in [
                (g.shadows && f.shadows, frame_flags::SHADOWS),
                (g.reflections || do_water_air, frame_flags::REFLECTIONS),
                (do_water_refl, frame_flags::WATER),
                (do_water_air, frame_flags::WATER_AIR),
                (mirror.is_some(), frame_flags::MIRROR),
                (probe.is_some(), frame_flags::PROBE),
                (occl, frame_flags::OCCLUSION),
                (profile(), frame_flags::SHADOW_STATS),
                (!shadow_depth_only, frame_flags::SHADOW_ALPHA_ALL),
                (crate::shadow_cull::blend_casts(), frame_flags::BLEND_CASTS),
            ] {
                if on {
                    flags |= bit;
                }
            }
            let mut frame = CullFrame::new(
                g,
                &cascades,
                &caster_planes,
                merge_size,
                [water_vp, mirror_vp, probe_vp],
                spot_vp,
                flags,
            );
            if do_water_air {
                frame.water_air = crate::gpu_cull::view_planes(air_vp).map(|p| p.to_array());
                frame.dist[1] = frame.dist[1].max(g.draw_distance.min(200.0));
            }
            self.cull
                .prepare(&self.device, &self.queue, scene.face_count, scene.object_count, frame);
        }
        let cpu = &mut self.indirect_cpu;
        let hud_opaque_range = push(cpu, &lists.hud_opaque);
        let hud_blend_range = push(cpu, &lists.hud_blend);
        let hud_debug_red_range = push(cpu, &lists.hud_debug_red);
        let hud_debug_blue_range = push(cpu, &lists.hud_debug_blue);
        let hud_select_faces_range = push(cpu, &lists.hud_select_faces);
        rg.debug_red = push(cpu, &lists.debug_red);
        rg.land_overlay = push(cpu, &lists.land_overlay);
        rg.debug_blue = push(cpu, &lists.debug_blue);
        rg.debug_green = push(cpu, &lists.debug_green);
        rg.debug_yellow = push(cpu, &lists.debug_yellow);
        rg.select_root = push(cpu, &lists.select_root);
        rg.creation_preview = push(cpu, &lists.creation_preview);
        rg.physics_shapes = push(cpu, &lists.physics_shapes);
        rg.select_child = push(cpu, &lists.select_child);
        rg.select_faces = push(cpu, &lists.select_faces);
        let imp_ranges: Vec<((u64, u32), (u64, u32))> = imp_caps.iter().map(|c| (push(cpu, &c.opaque), push(cpu, &c.blend))).collect();
        // reflection passes: each culled to its own frustum
        if capture.is_some() {
            rg.probe = push(cpu, &cull_to_view(probe_vp, &lists.probe));
        }
        if do_water_refl {
            rg.refl_water = push(cpu, &cull_to_view(water_vp, &lists.reflection));
        }
        if do_water_air {
            rg.water_air = push(cpu, &cull_to_view(air_vp, &lists.reflection));
            rg.water_air_terrain = push(cpu, &cull_cmds_to_view(air_vp, &lists.reflection_terrain));
            rg.water_air_terrain_pbr = push(cpu, &cull_cmds_to_view(air_vp, &lists.reflection_terrain_pbr));
        }
        if mirror.is_some() {
            rg.refl_mirror = push(cpu, &cull_to_view(mirror_vp, &lists.reflection));
        }
        // terrain of the reflection passes, each culled to its own frustum
        if capture.is_some() {
            rg.refl_terrain_probe = push(cpu, &cull_cmds_to_view(probe_vp, &lists.reflection_terrain));
            rg.refl_terrain_pbr_probe = push(cpu, &cull_cmds_to_view(probe_vp, &lists.reflection_terrain_pbr));
        }
        if do_water_refl {
            rg.refl_terrain_water = push(cpu, &cull_cmds_to_view(water_vp, &lists.reflection_terrain));
            rg.refl_terrain_pbr_water = push(cpu, &cull_cmds_to_view(water_vp, &lists.reflection_terrain_pbr));
        }
        if mirror.is_some() {
            rg.refl_terrain_mirror = push(cpu, &cull_cmds_to_view(mirror_vp, &lists.reflection_terrain));
            rg.refl_terrain_pbr_mirror = push(cpu, &cull_cmds_to_view(mirror_vp, &lists.reflection_terrain_pbr));
        }
        for i in 0..CASCADES {
            rg.shadow[i] = push(cpu, &shadow_lists[i]);
            rg.shadow_alpha[i] = push(cpu, &shadow_alpha_lists[i]);
            rg.shadow_terrain[i] = push(cpu, &shadow_terrain_lists[i]);
        }
        for (range, list) in rg.spot.iter_mut().zip(&spot_lists) {
            *range = push(cpu, list);
        }
        for (range, list) in rg.spot_terrain.iter_mut().zip(&spot_terrain_lists) {
            *range = push(cpu, list);
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
        if !lists.water.is_empty() {
            stats.draws += lists.water.len() as u32;
            stats.triangles += lists.water.iter().map(|d| d.index_count as u64 / 3).sum::<u64>();
        }
        let cpu_casters: [u32; CASCADES] =
            std::array::from_fn(|i| (shadow_lists[i].len() + shadow_alpha_lists[i].len() + shadow_terrain_lists[i].len()) as u32);
        stats.shadow_draws = cpu_casters.iter().sum();
        stats.shadow.draws = cpu_casters;
        stats.shadow.cpu_draws = cpu_casters;
        stats.blend_triangles = lists.blend.iter().map(|c| c.index_count as u64 / 3).sum();
        stats.lights = f.lights.len().min(MAX_LIGHTS) as u32;
        stats.probes = if f.probes.enabled { f.probes.probes.len() as u32 } else { 0 };
        stats.blend_early_discard = match BlendDiscard::from_env() {
            BlendDiscard::Early => true,
            BlendDiscard::Late => false,
            BlendDiscard::Alternate => !f.blend_late_discard,
        };
        stats.particles = n_particles;
        stats.occluded = occl.then_some(self.occlusion.last_hidden);
        if gpu.is_some() {
            // the GPU bins, read back a frame or two late
            let c = self.cull.last;
            stats.draws += c.main_draws() + c.bins[BIN_WATER_EXCLUSION];
            stats.shadow_draws += c.shadow_draws();
            for k in 0..CASCADES {
                stats.shadow.draws[k] += c.cascade_draws(k);
            }
            stats.shadow.triangles = c.shadow_triangles;
            stats.shadow.rigged_draws = c.shadow_rigged;
            stats.shadow.rigged_triangles = c.shadow_rigged_triangles;
            stats.triangles += c.triangles as u64;
            stats.occluded = occl.then_some(self.occlusion.last_hidden + c.hidden);
            stats.visible_objects = Some(c.visible_objects);
            stats.gpu_cull = true;
        }

        let mut water_settings = self.settings.water_effects;
        water_settings.particle_budget = water_settings.particle_budget.saturating_sub(n_particles);
        if self.water_effects.prepare(
            &self.device,
            &self.queue,
            water_settings,
            f.time,
            f.camera_pos.truncate(),
            &lists.water_ground,
            &lists.water_events,
            &lists.water_contacts,
        ) {
            self.rebuild_bind_groups();
        }
        stats.particles += self.water_effects.particles;
        prof.mark("lists");
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
        // ---- GPU draw lists: every face against every view of the frame
        let gpu_on = gpu.is_some();
        if gpu_on {
            let ts = if take_first() {
                self.timer.as_ref().map(|t| wgpu::ComputePassTimestampWrites {
                    query_set: &t.query_set,
                    beginning_of_pass_write_index: Some(0),
                    end_of_pass_write_index: None,
                })
            } else {
                None
            };
            self.cull.encode(
                &self.device,
                &mut encoder,
                &crate::gpu_cull::CullInputs {
                    faces: &scene.faces,
                    objects: &scene.objects,
                    palettes: &scene.palettes,
                    skin_binds: &scene.skin_binds,
                    boxes: &scene.joint_boxes,
                },
                self.occlusion.visibility(),
                ts,
            );
        }

        let bind_mesh = |pass: &mut wgpu::RenderPass, frame_bg: &wgpu::BindGroup| {
            pass.set_bind_group(0, frame_bg, &[]);
            pass.set_bind_group(1, &self.records_bind_group, &[]);
            pass.set_bind_group(2, &scene.textures, &[]);
            pass.set_vertex_buffer(0, scene.vertices.slice(..));
            pass.set_vertex_buffer(1, scene.skin.slice(..));
            pass.set_index_buffer(scene.indices.slice(..), wgpu::IndexFormat::Uint16);
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
        // a GPU bin (argument `region`, count of `bin`); nothing on the CPU lists
        let cull = &self.cull;
        let draw_bin = |pass: &mut wgpu::RenderPass, pipe: Option<&wgpu::RenderPipeline>, region: usize, bin: usize| {
            if gpu_on {
                calls.set(calls.get() + 1);
                if let Some(pipe) = pipe {
                    pass.set_pipeline(pipe);
                }
                pass.multi_draw_indexed_indirect_count(
                    cull.args(),
                    cull.region_offset(region),
                    cull.counts(),
                    GpuCull::count_offset(bin),
                    cull.cap(),
                );
            }
        };

        prof.mark("setup");
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
        mark(&mut encoder, MARK_SHADOW_BEGIN);
        if !f.shadows {
            mark(&mut encoder, MARK_SHADOW_C0_END);
            mark(&mut encoder, MARK_SHADOW_C1_END);
        }
        if f.shadows {
            // What the atlas passes draw, in order: each cascade in its
            // tile, then the projectors' maps. One pass, unless the
            // cascades are merged: the pass then ends after the opaque
            // casters of the first and of the second cascade, whose tile is
            // copied to the scratch map and laid over the next cascade
            // (shadow_merge.rs). The alpha-tested casters come after the
            // copy: every cascade draws its own.
            #[derive(Clone, Copy)]
            enum Step {
                Opaque(usize),
                Alpha(usize),
                Merge(usize),
                Spots,
            }
            use Step::*;
            const ONE_PASS: &[&[Step]] = &[&[Opaque(0), Alpha(0), Opaque(1), Alpha(1), Opaque(2), Alpha(2), Spots]];
            const MERGED: &[&[Step]] = &[
                &[Opaque(0)],
                &[Alpha(0), Opaque(1), Merge(0)],
                &[Alpha(1), Opaque(2), Merge(1), Alpha(2), Spots],
            ];
            let size = self.shadow_size;
            let half = size / 2;
            let passes = if merges.is_some() { MERGED } else { ONE_PASS };
            for (i, steps) in passes.iter().enumerate() {
                let ts = if i == 0 && take_first() { ts_begin(&self.timer) } else { None };
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("shadow"),
                    color_attachments: &[],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &self.shadow_view,
                        depth_ops: Some(wgpu::Operations {
                            load: if i == 0 { wgpu::LoadOp::Clear(1.0) } else { wgpu::LoadOp::Load },
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: ts,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                let cpu_ranges = rg.shadow.iter().chain(&rg.shadow_alpha).chain(&rg.shadow_terrain);
                if gpu_on || cpu_ranges.chain(&rg.spot).chain(&rg.spot_terrain).any(|r| r.1 > 0) {
                    pass.set_bind_group(1, &self.records_bind_group, &[]);
                    pass.set_bind_group(2, &scene.textures, &[]);
                    pass.set_vertex_buffer(0, scene.vertices.slice(..));
                    pass.set_vertex_buffer(1, scene.skin.slice(..));
                    pass.set_index_buffer(scene.indices.slice(..), wgpu::IndexFormat::Uint16);
                }
                for step in steps.iter().copied() {
                    match step {
                        Opaque(ci) | Alpha(ci) => {
                            let (x, y) = ((ci as u32 % 2) * size, (ci as u32 / 2) * size);
                            pass.set_viewport(x as f32, y as f32, size as f32, size as f32, 0.0, 1.0);
                            pass.set_scissor_rect(x, y, size, size);
                            pass.set_bind_group(3, &self.shadow_bind_groups[ci], &[]);
                            // depth only, then alpha tested: the depth kept
                            // is the nearest whatever the order
                            let (pipe, (off, count), bin) = match step {
                                Opaque(_) => (&self.pipelines.shadow_depth, rg.shadow[ci], BIN_SHADOW + ci),
                                _ => (&self.pipelines.shadow, rg.shadow_alpha[ci], BIN_SHADOW_ALPHA + ci),
                            };
                            if count > 0 || gpu_on {
                                pass.set_pipeline(pipe);
                                if count > 0 {
                                    pass.multi_draw_indexed_indirect(&self.indirect, off, count);
                                    calls.set(calls.get() + 1);
                                }
                                draw_bin(&mut pass, None, bin, bin);
                            }
                            if let Alpha(_) = step {
                                draw(&mut pass, &self.pipelines.shadow_terrain, rg.shadow_terrain[ci]);
                            }
                            match step {
                                Alpha(0) => pass_mark(&mut pass, MARK_SHADOW_C0_END),
                                Alpha(1) => pass_mark(&mut pass, MARK_SHADOW_C1_END),
                                _ => {}
                            }
                        }
                        Merge(k) => {
                            if let Some(merges) = &merges {
                                self.shadow_merge.merge(&mut pass, k, &merges[k], size);
                            }
                        }
                        Spots => {
                            // projector shadow maps: the halves of the top
                            // row of the atlas's fourth tile, every caster
                            // with the alpha test
                            for (k, vp) in spot_vp.iter().enumerate() {
                                if vp.is_none() {
                                    continue;
                                }
                                let (x, y) = (size + k as u32 * half, size);
                                pass.set_viewport(x as f32, y as f32, half as f32, half as f32, 0.0, 1.0);
                                pass.set_scissor_rect(x, y, half, half);
                                pass.set_bind_group(3, &self.shadow_bind_groups[CASCADES + k], &[]);
                                draw(&mut pass, &self.pipelines.shadow_terrain, rg.spot_terrain[k]);
                                let (off, count) = rg.spot[k];
                                if count == 0 && !gpu_on {
                                    continue;
                                }
                                pass.set_pipeline(&self.pipelines.shadow);
                                if count > 0 {
                                    pass.multi_draw_indexed_indirect(&self.indirect, off, count);
                                    calls.set(calls.get() + 1);
                                }
                                draw_bin(&mut pass, None, BIN_SPOT + k, BIN_SPOT + k);
                            }
                        }
                    }
                }
                drop(pass);
                // the tile whose opaque casters were just drawn, for the next cascade
                if let Some(merges) = &merges
                    && i + 1 < passes.len()
                {
                    self.shadow_merge.extract(&mut encoder, i, &merges[i]);
                }
            }
        }

        prof.mark("shadows");
        mark(&mut encoder, MARK_SHADOWS_END);
        let (pre_opaque, pre_opaque_2s) = if draw_lists.portal_clipping {
            (&self.pipelines.pre_portal, &self.pipelines.pre_portal_2s)
        } else {
            (&self.pipelines.pre_opaque, &self.pipelines.pre_opaque_2s)
        };
        // GTAO's mesh normals, drawn beside the depth when it reads them;
        // without them the passes keep no colour target at all
        let normals = self.targets.gtao.as_ref().filter(|_| self.gtao_normals_now).map(|g| &g.normals);
        let normal_target = |load| {
            normals.map(|view| {
                Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load,
                        store: wgpu::StoreOp::Store,
                    },
                })
            })
        };
        let pre_pipes: [&wgpu::RenderPipeline; MAIN_BINS] = if normals.is_some() {
            let mut pipes = self.pipelines.pre_normal.each_ref();
            if draw_lists.portal_clipping {
                pipes[..2].copy_from_slice(&self.pipelines.pre_normal_portal.each_ref());
            }
            pipes
        } else {
            [pre_opaque, pre_opaque_2s, &self.pipelines.pre_mask, &self.pipelines.pre_mask_2s]
        };
        // ---- depth prepass (single sample); with occlusion, phase 1 draws
        // what was visible last frame
        if occl {
            self.occlusion.encode_prepare(&self.device, &mut encoder, &self.indirect);
        }
        {
            let ts = if take_first() { ts_begin(&self.timer) } else { None };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("depth prepass"),
                color_attachments: normal_target(wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)).as_slice(),
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
            let terrain_pipe = if normals.is_some() {
                &self.pipelines.pre_terrain_normal
            } else {
                &self.pipelines.pre_terrain
            };
            draw(&mut pass, pre_pipes[0], rg.terrain);
            draw(&mut pass, pre_pipes[0], rg.terrain_pbr);
            draw(&mut pass, terrain_pipe, rg.terrain_mask);
            let pre = if occl { &self.occlusion.pre1 } else { &self.indirect };
            draw_from(&mut pass, pre_pipes[0], pre, rg.opaque);
            draw_from(&mut pass, pre_pipes[1], pre, rg.opaque_2s);
            draw_from(&mut pass, pre_pipes[2], pre, rg.mask);
            draw_from(&mut pass, pre_pipes[3], pre, rg.mask_2s);
            for (b, pipe) in pre_pipes.iter().enumerate() {
                if occl {
                    draw_bin(&mut pass, Some(pipe), REGION_PRE1 + b, COUNT_PRE1 + b);
                } else {
                    draw_bin(&mut pass, Some(pipe), b, b);
                }
            }
        }
        // ---- occlusion: Hi-Z of phase 1, test of every draw, then phase 2
        // adds the draws visible now that phase 1 did not draw
        if occl {
            self.occlusion.encode_cull(&self.device, &mut encoder, &self.indirect);
            if let (true, Some(hiz)) = (gpu_on, self.occlusion.hiz_view()) {
                cull.encode_occlusion(
                    &self.device,
                    &mut encoder,
                    &scene.faces,
                    &scene.objects,
                    self.occlusion.visibility(),
                    self.occlusion.params(),
                    hiz,
                );
            }
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("depth prepass 2"),
                color_attachments: normal_target(wgpu::LoadOp::Load).as_slice(),
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
            draw_from(&mut pass, pre_pipes[0], pre, rg.opaque);
            draw_from(&mut pass, pre_pipes[1], pre, rg.opaque_2s);
            draw_from(&mut pass, pre_pipes[2], pre, rg.mask);
            draw_from(&mut pass, pre_pipes[3], pre, rg.mask_2s);
            for (b, pipe) in pre_pipes.iter().enumerate() {
                draw_bin(&mut pass, Some(pipe), REGION_PRE2 + b, COUNT_PRE2 + b);
            }
        }
        if gpu_on {
            cull.copy_counts(&mut encoder);
        }

        prof.mark("prepass");
        mark(&mut encoder, MARK_DEPTH_END);
        // ---- occlusion depth: the prepass and the blended faces at least
        // half covered (fs_depth_contact). The many heads, bodies and hair
        // whose faces are blended though they look opaque then take the
        // occlusion like the opaque faces beside them, instead of what lies
        // behind them (#556) or none (#882: one half of a head darker)
        let (_, n_blend) = rg.blend;
        let blend_in_ao_depth = self.blend_ao_depth && n_blend > 0 && self.targets.ao_depth.is_some();
        if let Some((tex, view)) = &self.targets.ao_depth {
            encoder.copy_texture_to_texture(
                self.targets.depth_ss_tex.as_image_copy(),
                tex.as_image_copy(),
                self.targets.depth_ss_tex.size(),
            );
            if blend_in_ao_depth {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("occlusion depth"),
                    color_attachments: normal_target(wgpu::LoadOp::Load).as_slice(),
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view,
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
                let pipe = if normals.is_some() {
                    &self.pipelines.pre_contact_normal
                } else {
                    &self.pipelines.pre_contact
                };
                draw(&mut pass, pipe, rg.blend);
            }
        }
        // ---- SSAO
        if let (true, Some(t)) = (self.targets.gtao_shown, &self.targets.gtao) {
            self.gtao.encode(&mut encoder, t, indirect_now);
        } else if let (Some((raw, blurred, _, _)), Some((bg_ssao, bg_blur))) = (&self.targets.ao, &self.ssao_groups) {
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

        prof.mark("ssao");
        mark(&mut encoder, MARK_SSAO_END);
        // ---- contact shadows: what casts them beyond the prepass (the
        // blended faces opaque enough, the impostor cards) is drawn into a
        // copy of its depth, which the pass then reads; the occlusion's
        // depth already holds those blended faces
        if let (true, Some(t)) = (self.contact_now, &self.targets.contact) {
            let more = crate::contact_shadows::Override::from_env().all_casters() && (n_blend > 0 || n_sprites > 0);
            if more {
                let source = match &self.targets.ao_depth {
                    Some((tex, _)) if blend_in_ao_depth => tex,
                    _ => &self.targets.depth_ss_tex,
                };
                encoder.copy_texture_to_texture(
                    source.as_image_copy(),
                    t.depth_texture().as_image_copy(),
                    self.targets.depth_ss_tex.size(),
                );
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("contact shadows depth"),
                    color_attachments: &[],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: t.depth_view(),
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
                if !blend_in_ao_depth {
                    draw(&mut pass, &self.pipelines.pre_contact, rg.blend);
                }
                if n_sprites > 0 {
                    pass.set_pipeline(&self.impostors.sprite_depth);
                    pass.set_bind_group(3, &self.impostors.bind_group, &[]);
                    pass.set_vertex_buffer(0, self.impostors.sprites.slice(..));
                    pass.draw(0..6, 0..n_sprites as u32);
                }
            }
            self.contact.encode(&mut encoder, t, more);
        }
        mark(&mut encoder, MARK_CONTACT_END);
        self.water_effects.encode(&mut encoder);
        // ---- planar reflections
        let refl_passes = [
            (
                do_water_refl,
                self.targets.water_refl.as_ref(),
                &self.groups.refl[0],
                "water reflection",
                rg.refl_water,
                (rg.refl_terrain_water, rg.refl_terrain_pbr_water),
                BIN_REFL_WATER,
            ),
            (
                mirror.is_some(),
                self.targets.mirror.as_ref(),
                &self.groups.refl[1],
                "mirror",
                rg.refl_mirror,
                (rg.refl_terrain_mirror, rg.refl_terrain_pbr_mirror),
                BIN_REFL_MIRROR,
            ),
        ];
        for (enabled, target, bg, label, objects, terrain, bin) in refl_passes {
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
            draw(&mut pass, &self.pipelines.refl_terrain, terrain.0);
            draw(&mut pass, &self.pipelines.refl_terrain_pbr, terrain.1);
            draw(&mut pass, &self.pipelines.refl_obj, objects);
            draw_bin(&mut pass, Some(&self.pipelines.refl_obj), bin, bin);
        }

        mark(&mut encoder, MARK_WATER_AIR_BEGIN);
        if let (true, Some(t)) = (do_water_air, &self.targets.water_air) {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("water exterior"),
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
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_viewport(0.0, 0.0, t.width as f32, t.height as f32, 0.0, 1.0);
            bind_mesh(&mut pass, &self.groups.refl[7]);
            pass.set_pipeline(&self.pipelines.refl_sky);
            pass.draw(0..3, 0..1);
            draw(&mut pass, &self.pipelines.refl_terrain, rg.water_air_terrain);
            draw(&mut pass, &self.pipelines.refl_terrain_pbr, rg.water_air_terrain_pbr);
            draw(&mut pass, &self.pipelines.refl_obj, rg.water_air);
            draw_bin(&mut pass, Some(&self.pipelines.refl_obj), BIN_REFL_WATER, BIN_REFL_WATER);
        }
        mark(&mut encoder, MARK_WATER_AIR_END);

        // ---- reflection probe: one cube face (LLReflectionMap::update ->
        // cubeSnapshot), then the radiance filtering after the sixth
        if let Some(c) = &capture {
            // the radiance face, then (a sky with a probe ambiance) the same
            // face of the irradiance capture
            let radiance = (
                self.probes.capture_target(c.face),
                &self.probes.depth,
                &self.groups.refl[2],
                "probe capture",
            );
            let irradiance = (
                self.probes.irr_capture_target(c.face),
                &self.probes.irr_depth,
                &self.groups.refl[8],
                "probe irradiance capture",
            );
            for (target, depth, group, label) in std::iter::once(radiance).chain(irr_capture.then_some(irradiance)) {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some(label),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: target,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: depth,
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
                bind_mesh(&mut pass, group);
                pass.set_pipeline(&self.pipelines.refl_sky);
                pass.draw(0..3, 0..1);
                // the default probe sees sky, terrain and water only
                draw(&mut pass, &self.pipelines.refl_terrain, rg.refl_terrain_probe);
                draw(&mut pass, &self.pipelines.refl_terrain_pbr, rg.refl_terrain_pbr_probe);
                if !c.sky_only {
                    draw(&mut pass, &self.pipelines.refl_obj, rg.probe);
                    draw_bin(&mut pass, Some(&self.pipelines.refl_obj), BIN_PROBE, BIN_PROBE);
                }
            }
            if c.finish {
                self.probes
                    .filter(&mut encoder, c.slot, self.probe_irr_faces == 0x3f, self.settings.hdr);
            }
        }

        prof.mark("reflections");
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

        prof.mark("impostors");
        // the frame is recorded into three encoders finished in parallel
        // below (wgpu replays and tracks every command in finish())
        let enc_pre = std::mem::replace(
            &mut encoder,
            self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame scene"),
            }),
        );
        mark(&mut encoder, MARK_SCENE_BEGIN);
        // Keep water and exclusion depths separate from opaque objects. The
        // same mask removes surface shading and above-water haze through a face.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("water exclusion mask"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.targets.water_mask,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.targets.water_interface,
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
            bind_mesh(&mut pass, &self.groups.mask);
            draw(&mut pass, &self.pipelines.water_mask, rg.water);
            draw(&mut pass, &self.pipelines.water_exclusion, rg.water_exclusion);
            draw_bin(
                &mut pass,
                Some(&self.pipelines.water_exclusion),
                BIN_WATER_EXCLUSION,
                BIN_WATER_EXCLUSION,
            );
        }
        mark(&mut encoder, MARK_WATER_INTERFACE_END);
        let prepass_wanted = match msaa_prepass() {
            MsaaPrepass::On => true,
            MsaaPrepass::Off => false,
            MsaaPrepass::Alternate => {
                static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
                !START.get_or_init(Instant::now).elapsed().as_secs().is_multiple_of(3)
            }
        };
        // ---- MSAA: depth prepass of the scene pass's own depth. The scene
        // pass cannot test against the single-sample prepass, and without
        // this one it shades every opaque fragment that reaches it, hidden
        // or not. Terrain and opaque faces only, exactly the draws of the
        // scene pass (same lists, so the depth of each sample is the one
        // the scene pass would have ended with and the image is the same).
        // Alpha-masked faces stay out: their coverage comes from
        // alpha-to-coverage in the scene pass, which a depth-only pass
        // cannot reproduce sample for sample; they are drawn after the
        // opaque faces and were already tested against them.
        let msaa_pre = match (&self.pipelines.pre_msaa, &self.targets.depth_msaa) {
            (Some(variants), Some(depth)) if prepass_wanted => {
                let pipes = &variants[usize::from(draw_lists.portal_clipping)];
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("depth prepass msaa"),
                    color_attachments: &[],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: depth,
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
                bind_mesh(&mut pass, &self.groups.a);
                if let Some(terrain) = &self.pipelines.pre_terrain_msaa {
                    draw(&mut pass, terrain, rg.terrain_mask);
                }
                draw(&mut pass, &pipes[0], rg.terrain);
                draw(&mut pass, &pipes[0], rg.terrain_pbr);
                for (b, (pipe, r)) in [(&pipes[0], rg.opaque), (&pipes[1], rg.opaque_2s)].into_iter().enumerate() {
                    draw(&mut pass, pipe, r);
                    draw_bin(&mut pass, Some(pipe), b, b);
                }
                true
            }
            _ => false,
        };
        mark(&mut encoder, MARK_DEPTH_MSAA_END);
        // (the variant asked for: without MSAA the two are the same frame,
        // and their times tell how far apart two equal measures fall)
        if let (Some(t), Some(slot)) = (&self.timer, timer_slot) {
            t.slot_prepass[slot].set(prepass_wanted);
        }
        // ---- scene pass A: sky, terrain, opaque, masked
        {
            let (view, resolve) = match &self.targets.color_msaa {
                Some(msaa) => (msaa, Some(&self.targets.color)),
                None => (&self.targets.color, None),
            };
            // (a cut-out photo starts from black: what is drawn over it is
            // then premultiplied by its coverage)
            let cc = if f.cutout { glam::Vec4::ZERO } else { f.clear_color };
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
            // start from a prepass depth (early-z, no overdraw): the
            // single-sample one, or with MSAA the multisampled one above
            let (depth, load) = match &self.targets.depth_msaa {
                Some(d) if msaa_pre => (d, wgpu::LoadOp::Load),
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
            if !f.cutout {
                pass.set_pipeline(&self.pipelines.sky);
                pass.draw(0..3, 0..1);
            }
            pass_mark(&mut pass, MARK_SKY_END);
            draw(&mut pass, &self.pipelines.terrain, rg.terrain);
            draw(&mut pass, &self.pipelines.terrain_pbr, rg.terrain_pbr);
            draw(&mut pass, &self.pipelines.terrain_pbr, rg.terrain_mask);
            pass_mark(&mut pass, MARK_TERRAIN_END);
            let scene_pipes = [
                (&self.pipelines.opaque, rg.opaque),
                (&self.pipelines.opaque_2s, rg.opaque_2s),
                (&self.pipelines.mask, rg.mask),
                (&self.pipelines.mask_2s, rg.mask_2s),
            ];
            for (b, (pipe, r)) in scene_pipes.into_iter().enumerate() {
                draw(&mut pass, pipe, r);
                draw_bin(&mut pass, Some(pipe), b, b);
            }
            pass_mark(&mut pass, MARK_OBJECTS_END);
            // impostor cards (last: they rebind vertex buffer 0)
            if n_sprites > 0 {
                pass.set_pipeline(&self.impostors.sprite);
                pass.set_bind_group(3, &self.impostors.bind_group, &[]);
                pass.set_vertex_buffer(0, self.impostors.sprites.slice(..));
                pass.draw(0..6, 0..n_sprites as u32);
            }
        }

        prof.mark("scene_a");
        mark(&mut encoder, MARK_SCENE_A_END);
        // a cut-out photo: what shows through, from the depth the scene
        // pass ended with (cutout.rs)
        if f.cutout {
            let t = &self.targets;
            let (color, depth, samples) = match (&t.color_msaa, &t.depth_msaa, &t.depth_msaa_tex) {
                (Some(c), Some(d), Some(tex)) => (c, d, tex.sample_count()),
                _ => (&t.color, &t.depth_ss, 1),
            };
            self.cutout
                .get_or_insert_with(|| crate::cutout::Cutout::new(&self.device, HDR_FORMAT))
                .encode(&self.device, &mut encoder, color, depth, HDR_FORMAT, samples);
        }
        let water_particles = if f.cutout { 0 } else { self.water_effects.particles };
        if water_particles > 0 && above_water_possible {
            let mut pass = begin_scene_b(&mut encoder, &self.targets, "submerged water particles", false, false);
            bind_mesh(&mut pass, &self.groups.a);
            pass.set_pipeline(&self.pipelines.water_particles_under);
            pass.draw(0..crate::water_effects::WATER_PARTICLE_VERTICES, 0..water_particles);
        }
        if water_particles > 0 && self.targets.color_msaa.is_some() && above_water_possible {
            let _resolve = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("resolve submerged particles"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: self.targets.color_msaa.as_ref().unwrap_or(&self.targets.color),
                    depth_slice: None,
                    resolve_target: Some(&self.targets.color),
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
        }
        // ---- scene copy (refraction / SSR source)
        let size = wgpu::Extent3d {
            width: world_size.0,
            height: world_size.1,
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

        // the copy of the depth `RiggedBlendDepth::Layered` and `Layers`
        // draw into, made on its first use (the size and samples of the
        // scene depth)
        if matches!(lists.blend_depth_mode, RiggedBlendDepth::Layered | RiggedBlendDepth::Layers)
            && lists.blend_depth > 0
            && self.targets.blend_layers.is_none()
        {
            let size = self.targets.depth_ss_tex.size();
            let samples = self.targets.depth_msaa_tex.as_ref().map_or(1, |t| t.sample_count());
            self.targets.blend_layers = Some(make_view(
                &self.device,
                "rigged blend layers depth",
                size.width,
                size.height,
                DEPTH_FORMAT,
                samples,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_DST,
            ));
        }
        // ---- scene pass B: SSR, water, blended faces, particles. The
        // blended faces of rigged meshes come first and write depth
        // (`DrawLists::blend_depth`, LLDrawPoolAlpha::forwardRender(true)):
        // the pass is cut around them, the rest of it only reads the depth.
        let stride = std::mem::size_of::<DrawIndexedIndirect>() as u64;
        let (b0, bn) = rg.blend;
        let n_depth = (lists.blend_depth as u32).min(bn);
        let split = n_depth > 0;
        let blend_pipe = self
            .pipelines
            .blend_late
            .as_ref()
            .filter(|_| f.blend_late_discard)
            .unwrap_or(&self.pipelines.blend);
        // blended faces `from..to`, back to front; a glowing one adds its glow
        // right after itself, so later layers in front dim it (LLDrawPoolAlpha)
        let blended = |pass: &mut wgpu::RenderPass, pipe: &wgpu::RenderPipeline, from: u32, to: u32| {
            let mut start = from;
            for i in from..to {
                if !(glow_on && glow_skip & 8 == 0 && lists.blend_glow.get(i as usize).copied().unwrap_or(false)) {
                    continue;
                }
                draw(pass, pipe, (b0 + start as u64 * stride, i + 1 - start));
                draw(pass, &self.pipelines.glow, (b0 + i as u64 * stride, 1));
                start = i + 1;
            }
            draw(pass, pipe, (b0 + start as u64 * stride, to - start));
        };
        // the other blended faces, the overlays and the particles
        let rest = |pass: &mut wgpu::RenderPass| {
            // The local hologram leaves world depth and the SSR G-buffer
            // untouched; transparent world faces still cover it normally.
            draw(pass, &self.pipelines.creation_preview, rg.creation_preview);
            blended(pass, blend_pipe, n_depth, bn);
            // debug overlays (before the particles, which rebind vertex buffer 0)
            draw(pass, &self.pipelines.land_overlay, rg.land_overlay);
            draw(pass, &self.pipelines.debug_red, rg.debug_red);
            draw(pass, &self.pipelines.debug_blue, rg.debug_blue);
            draw(pass, &self.pipelines.debug_green, rg.debug_green);
            draw(pass, &self.pipelines.debug_yellow, rg.debug_yellow);
            draw(pass, &self.pipelines.physics_fill, rg.physics_shapes);
            if let Some(wire) = &self.pipelines.physics_wire {
                draw(pass, wire, rg.physics_shapes);
            }
            if let Some(wire) = &self.pipelines.select_child_wire {
                draw(pass, wire, rg.select_child);
            }
            if let Some(wire) = &self.pipelines.select_root_wire {
                draw(pass, wire, rg.select_root);
            }
            if let (true, Some(wire)) = (f.wireframe, &self.pipelines.debug_wire) {
                for r in [
                    rg.terrain,
                    rg.terrain_pbr,
                    rg.terrain_mask,
                    rg.opaque,
                    rg.opaque_2s,
                    rg.mask,
                    rg.mask_2s,
                    rg.blend,
                ] {
                    draw(pass, wire, r);
                }
                for b in 0..MAIN_BINS {
                    draw_bin(pass, Some(wire), b, b);
                }
            }
            pass_mark(pass, MARK_BLEND_END);
            if water_particles > 0 {
                pass.set_pipeline(&self.pipelines.water_particles);
                pass.draw(0..crate::water_effects::WATER_PARTICLE_VERTICES, 0..water_particles);
            }
            // particles last: they rebind vertex buffer 0
            if n_particles > 0 {
                pass.set_pipeline(&self.pipelines.particles);
                pass.set_vertex_buffer(0, self.particle_buffer.slice(..));
                pass.draw(0..6, 0..n_particles);
            }
        };
        {
            let mut pass = begin_scene_b(&mut encoder, &self.targets, "scene b", !split, false);
            bind_mesh(&mut pass, &self.groups.b);
            if let (Some(ssr), Some(gbg)) = (&self.pipelines.ssr, &self.gbuf_group) {
                pass.set_pipeline(ssr);
                pass.set_bind_group(3, gbg, &[]);
                pass.draw(0..3, 0..1);
            }
            pass_mark(&mut pass, MARK_SSR_END);
            let water_pipe = self
                .pipelines
                .water_unclipped
                .as_ref()
                .filter(|_| water_near)
                .unwrap_or(&self.pipelines.water);
            draw(&mut pass, water_pipe, rg.water);
            pass_mark(&mut pass, MARK_WATER_END);
            if glow_on {
                draw(&mut pass, &self.pipelines.glow_max, rg.creation_preview);
                if glow_skip & 2 == 0 {
                    draw(&mut pass, &self.pipelines.glow_max, rg.glow);
                }
                let (g0, gn) = rg.glow_alpha;
                for i in 0..(if glow_skip & 4 == 0 { gn as u64 } else { 0 }) {
                    draw(&mut pass, &self.pipelines.glow_suppress, (g0 + i * stride, 1));
                    draw(&mut pass, &self.pipelines.glow, (g0 + i * stride, 1));
                }
            }
            pass_mark(&mut pass, MARK_GLOW_END);
            if !split {
                rest(&mut pass);
            }
        }
        if split {
            // (the frame group without the scene copy and the depth: the
            // depth is written here, and fs_blend reads neither)
            match lists.blend_depth_mode {
                RiggedBlendDepth::Write => {
                    let mut pass = begin_scene_b(&mut encoder, &self.targets, "scene b rigged blend", false, true);
                    bind_mesh(&mut pass, &self.groups.a);
                    blended(&mut pass, &self.pipelines.blend_depth, 0, n_depth);
                }
                RiggedBlendDepth::HalfCovered => {
                    let mut pass = begin_scene_b(&mut encoder, &self.targets, "scene b rigged blend", false, true);
                    bind_mesh(&mut pass, &self.groups.a);
                    draw(&mut pass, &self.pipelines.blend_depth_mask, (b0, n_depth));
                    blended(&mut pass, &self.pipelines.blend_depth_color, 0, n_depth);
                }
                RiggedBlendDepth::Layered => {
                    // their colour as the Legacy Viewer draws it, each
                    // fragment from 1/255 hiding the layers drawn after it
                    // behind it, on a copy of the depth...
                    let t = &self.targets;
                    if let Some((layers_tex, layers)) = &t.blend_layers {
                        let scene_depth = t.depth_msaa_tex.as_ref().unwrap_or(&t.depth_ss_tex);
                        encoder.copy_texture_to_texture(scene_depth.as_image_copy(), layers_tex.as_image_copy(), scene_depth.size());
                        let mut pass = begin_scene_b_on(&mut encoder, t, "scene b rigged blend", false, true, layers);
                        bind_mesh(&mut pass, &self.groups.a);
                        blended(&mut pass, &self.pipelines.blend_depth, 0, n_depth);
                    }
                    // ...and in the scene depth only where at least half
                    // covered, for what is drawn after them (#556)
                    let mut pass = begin_scene_b(&mut encoder, t, "scene b rigged blend depth", false, true);
                    bind_mesh(&mut pass, &self.groups.a);
                    draw(&mut pass, &self.pipelines.blend_depth_mask, (b0, n_depth));
                }
                RiggedBlendDepth::Split => {
                    // object by object in their drawing order, what is at
                    // least half covered, hiding what is drawn after it
                    // behind it, then the object's fringes (#935); the glow
                    // once, with the first
                    let mut pass = begin_scene_b(&mut encoder, &self.targets, "scene b rigged blend", false, true);
                    bind_mesh(&mut pass, &self.groups.a);
                    let mut from = 0;
                    for end in lists.blend_depth_runs.iter().map(|&e| e.min(n_depth)).chain([n_depth]) {
                        if end <= from {
                            continue;
                        }
                        blended(&mut pass, &self.pipelines.blend_depth_solid, from, end);
                        draw(
                            &mut pass,
                            &self.pipelines.blend_depth_fringe,
                            (b0 + from as u64 * stride, end - from),
                        );
                        from = end;
                    }
                }
                RiggedBlendDepth::Layers => {
                    // object by object as `Split`, on a copy of the depth,
                    // each object's fringes then hiding behind them what is
                    // drawn after them (#1300)...
                    let t = &self.targets;
                    if let Some((layers_tex, layers)) = &t.blend_layers {
                        let scene_depth = t.depth_msaa_tex.as_ref().unwrap_or(&t.depth_ss_tex);
                        encoder.copy_texture_to_texture(scene_depth.as_image_copy(), layers_tex.as_image_copy(), scene_depth.size());
                        let mut pass = begin_scene_b_on(&mut encoder, t, "scene b rigged blend", false, true, layers);
                        bind_mesh(&mut pass, &self.groups.a);
                        let mut from = 0;
                        for end in lists.blend_depth_runs.iter().map(|&e| e.min(n_depth)).chain([n_depth]) {
                            if end <= from {
                                continue;
                            }
                            let run = (b0 + from as u64 * stride, end - from);
                            blended(&mut pass, &self.pipelines.blend_depth_solid, from, end);
                            draw(&mut pass, &self.pipelines.blend_depth_fringe, run);
                            draw(&mut pass, &self.pipelines.blend_depth_fringe_depth, run);
                            from = end;
                        }
                    }
                    // ...and in the scene depth only where at least half
                    // covered, for what is drawn after them (#556)
                    let mut pass = begin_scene_b(&mut encoder, t, "scene b rigged blend depth", false, true);
                    bind_mesh(&mut pass, &self.groups.a);
                    draw(&mut pass, &self.pipelines.blend_depth_mask, (b0, n_depth));
                }
                RiggedBlendDepth::Runs => {
                    // their depth where nearly opaque, then run by run
                    // their colour and their depth where half covered (#935)
                    let mut pass = begin_scene_b(&mut encoder, &self.targets, "scene b rigged blend", false, true);
                    bind_mesh(&mut pass, &self.groups.a);
                    draw(&mut pass, &self.pipelines.blend_depth_opaque, (b0, n_depth));
                    let mut from = 0;
                    for end in lists.blend_depth_runs.iter().map(|&e| e.min(n_depth)).chain([n_depth]) {
                        if end <= from {
                            continue;
                        }
                        blended(&mut pass, &self.pipelines.blend_depth_color, from, end);
                        draw(&mut pass, &self.pipelines.blend_depth_mask, (b0 + from as u64 * stride, end - from));
                        from = end;
                    }
                }
            }
            let mut pass = begin_scene_b(&mut encoder, &self.targets, "scene b blend", true, false);
            bind_mesh(&mut pass, &self.groups.b);
            rest(&mut pass);
        }

        prof.mark("scene_b");
        mark(&mut encoder, MARK_SCENE_END);
        // ---- faces drawn under a clicked pixel: opaque and masked, then
        // blended ones in front of them (only in the frame of a click)
        if drawn_px.is_some() {
            let blend_pick = if drawn_px.is_some_and(|(_, _, mode)| mode == crate::DrawnMode::Visible) {
                &self.pipelines.pick_visible
            } else {
                &self.pipelines.pick
            };
            let layers: [&[(&wgpu::RenderPipeline, (u64, u32), Option<usize>)]; 2] = [
                &[
                    (&self.pipelines.pick_terrain, rg.terrain, None),
                    (&self.pipelines.pick_terrain, rg.terrain_pbr, None),
                    (&self.pipelines.pick_terrain_mask, rg.terrain_mask, None),
                    (&self.pipelines.pick, rg.opaque, Some(0)),
                    (&self.pipelines.pick_2s, rg.opaque_2s, Some(1)),
                    (&self.pipelines.pick, rg.mask, Some(2)),
                    (&self.pipelines.pick_2s, rg.mask_2s, Some(3)),
                ],
                &[
                    (blend_pick, rg.blend, None),
                    // LLViewerWindow::pickAsync / pickImmediate: debug alpha
                    // makes transparent surfaces pickable. Reuse the actual
                    // overlay lists, including hidden faces and only the
                    // rigged geometry enabled for display, with their full
                    // two-sided triangles instead of their texture cutouts.
                    (&self.pipelines.pick_overlay, rg.debug_red, None),
                    (&self.pipelines.pick_overlay, rg.debug_blue, None),
                ],
            ];
            for (k, draws) in layers.iter().enumerate() {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("drawn pick"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &self.drawn_pick.views[k],
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &self.drawn_pick.depth,
                        depth_ops: Some(wgpu::Operations {
                            // the blended faces behind the opaque ones stay hidden
                            load: if k == 0 { wgpu::LoadOp::Clear(0.0) } else { wgpu::LoadOp::Load },
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                bind_mesh(&mut pass, &self.groups.refl[6]);
                for (pipe, range, bin) in draws.iter() {
                    draw(&mut pass, pipe, *range);
                    if let Some(b) = bin {
                        draw_bin(&mut pass, Some(pipe), *b, *b);
                    }
                }
            }
            self.drawn_pick.encode_copy(&mut encoder);
        }
        // depth under the cursor, read back in a later frame
        self.depth_pick.encode_hover(
            &self.device,
            &self.queue,
            &mut encoder,
            &self.targets.depth_ss,
            world_size,
            unjittered_vp.inverse(),
        );
        let enc_scene = std::mem::replace(
            &mut encoder,
            self.device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame post") }),
        );
        // ---- temporal anti-aliasing
        let mut post_bg = &self.targets.post_bind_group;
        let mut post_src = &self.targets.color;
        if let Some(taa) = &mut self.taa {
            let w = taa.parity;
            let prev = self.prev_view_proj.unwrap_or(unjittered_vp);
            let u = TaaU {
                inv_view_proj: (fu.proj * fu.view).inverse().to_cols_array_2d(),
                prev_view_proj: prev.to_cols_array_2d(),
                params: [
                    if taa.reset || self.prev_view_proj.is_none() { 1.0 } else { 0.0 },
                    0.1,
                    world_size.0 as f32,
                    world_size.1 as f32,
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
            post_src = &taa.history[w];
        }
        // ---- depth of field, on the resolved (and TAA-steady) image,
        // before the glow and the tone mapping (not in probe captures)
        match &f.dof {
            Some(params) => {
                let dof = self.dof.get_or_insert_with(|| crate::dof::Dof::new(&self.device, HDR_FORMAT));
                post_bg = dof.encode(
                    &self.device,
                    &self.queue,
                    &mut encoder,
                    post_src,
                    &self.targets.depth_ss,
                    f.proj,
                    world_size,
                    params,
                    HDR_FORMAT,
                    &crate::dof::PostInputs {
                        layout: &self.post_layout,
                        sampler: &self.post_sampler,
                        buffer: &self.post_buffer,
                        glow: &self.glow.views[1],
                    },
                    f.cutout,
                );
            }
            None => {
                if let Some(dof) = &mut self.dof {
                    dof.release();
                }
            }
        }
        self.prev_view_proj = Some(unjittered_vp);
        self.last_inv_vp = unjittered_vp.inverse();

        // ---- automatic exposure, from the resolved scene and the glow of
        // the last frame, into the post uniforms the glow, the bloom and the
        // tone mapping read
        self.exposure.encode(
            &self.queue,
            &mut encoder,
            &crate::exposure::FrameExposure {
                // (no generateExposure without HDR: the exposure stays 1)
                tone_mapped: !f.sky.no_post && self.settings.hdr,
                hdr: self.settings.hdr,
                hdr_scale: f.sky.hdr_scale,
                // (a cut-out photo's empty background would raise it: the
                // subject keeps the exposure the whole scene gave it;
                // without HDR there is none to keep)
                hold: (snapshot.is_some() || f.exposure_lock || f.cutout) && self.settings.hdr,
                glow: self.glow_drawn,
                inv_view_proj: unjittered_vp.inverse(),
            },
            &self.post_buffer,
            POST_AUTO_EXPOSURE_OFFSET,
        );
        // the viewfinder's histogram, of the live view only
        if let Some(exposure) = histogram
            && snapshot.is_none()
        {
            self.histogram.encode(&self.queue, &mut encoder, exposure);
        }
        self.glow_drawn = glow_on;
        if glow_on {
            self.glow.encode(&mut encoder, post_bg);
        }
        // ---- bloom, from the image the post pass reads (after the glow,
        // which it leaves as it is)
        mark(&mut encoder, MARK_BLOOM_BEGIN);
        if bloom > 0.0 {
            self.bloom.encode(&mut encoder, post_bg);
        }
        mark(&mut encoder, MARK_BLOOM_END);

        let draw_huds = lists.hud_view.is_some() && self.hud_depth.is_some() && snapshot.is_none_or(|s| s.huds);
        if world_size != (self.config.width, self.config.height) {
            let smooth = self.settings.render_scale_percent < 100 && self.rlva_resolution_divisor <= 1 && snapshot.is_none();
            if self
                .rlva_scaled
                .as_ref()
                .is_none_or(|scaled| scaled.size() != world_size || scaled.smooth != smooth)
            {
                self.rlva_scaled = Some(crate::rlva::resolution::Scaled::new(
                    &self.device,
                    self.config.format,
                    world_size,
                    smooth,
                ));
            }
        } else {
            self.rlva_scaled = None;
        }
        let composed_view = self.rlva_scaled.as_ref().map_or(&surface_view, |scaled| &scaled.view);
        let scene_target = if !f.rlva_spheres.is_empty() {
            self.rlva_spheres
                .get_or_insert_with(|| crate::rlva::Spheres::new(&self.device, self.config.format))
                .prepare(&self.device, world_size, self.config.format)
        } else {
            self.rlva_spheres = None;
            composed_view
        };
        // the photo's look: the post pass and SMAA draw into its input,
        // and its passes into the frame
        let look = f.look.as_ref().filter(|look| look.layers.iter().any(|l| !l.ops.is_empty()));
        if look.is_some() {
            self.looks
                .get_or_insert_with(|| crate::looks::Looks::new(&self.device))
                .prepare(&self.device, self.config.format, world_size);
        } else if let Some(looks) = &mut self.looks {
            looks.idle();
        }
        let looks = look.and(self.looks.as_ref()).and_then(|looks| Some((looks, looks.input()?)));
        let post_target = looks.map_or(scene_target, |(_, input)| input);
        // ---- post (tonemap) to the swapchain
        {
            let ts = match (&self.timer, timer_slot) {
                (Some(t), Some(_)) if !draw_huds && rg.select_faces.1 == 0 => Some(wgpu::RenderPassTimestampWrites {
                    query_set: &t.query_set,
                    beginning_of_pass_write_index: None,
                    end_of_pass_write_index: Some(1),
                }),
                _ => None,
            };
            // with SMAA the post pass fills its gamma-encoded input image
            let (post_view, post_pipe, post_ts, smaa_ts) = match &self.smaa_targets {
                Some(st) => (&st.ldr, &self.smaa.post_ldr, None, ts),
                None => (post_target, &self.pipelines.post, ts, None),
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
                pass.set_bind_group(1, self.bloom.post_bg(), &[]);
                pass.draw(0..3, 0..1);
            }
            if let Some(st) = &self.smaa_targets {
                self.smaa.encode(&mut encoder, st, post_target, smaa_ts);
            }
            if let (Some((looks, _)), Some(look)) = (looks, look) {
                let depth = crate::looks::LookDepth {
                    view: &self.targets.depth_ss,
                    proj: f.proj,
                };
                let passes = looks.encode(&self.device, &self.queue, &mut encoder, look, scene_target, Some(depth));
                calls.set(calls.get() + passes);
            }
        }

        if rg.select_faces.1 > 0
            && let Some(selection) = &self.selection
        {
            let mut pass = selection.begin_mask(&mut encoder, &self.targets.depth_ss);
            bind_mesh(&mut pass, &self.groups.b);
            draw(&mut pass, &self.pipelines.selection_mask, rg.select_faces);
            drop(pass);
            let timestamps = match (&self.timer, timer_slot) {
                (Some(t), Some(_)) if !draw_huds => Some(wgpu::RenderPassTimestampWrites {
                    query_set: &t.query_set,
                    beginning_of_pass_write_index: None,
                    end_of_pass_write_index: Some(1),
                }),
                _ => None,
            };
            selection.encode(&mut encoder, scene_target, timestamps);
            calls.set(calls.get() + 2);
        }

        if let Some(effects) = &mut self.rlva_spheres {
            let passes = effects.encode(
                &self.device,
                &self.queue,
                &mut encoder,
                &self.targets.depth_ss,
                unjittered_vp.inverse(),
                world_size,
                &f.rlva_spheres,
                composed_view,
            );
            calls.set(calls.get() + passes as u32);
        }
        if let Some(scaled) = &self.rlva_scaled {
            scaled.encode(&mut encoder, &surface_view);
            calls.set(calls.get() + 1);
        }

        let mut capture = None;

        prof.mark("post");
        // ---- egui: its meshes, and what it painted for the world (name
        // tags, floating texts), which the HUD attachments cover
        let mut extra_cmds = Vec::new();
        let mut window_glass = false;
        let mut meshes_uploaded = false;
        let sd = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [self.config.width, self.config.height],
            pixels_per_point: ui.as_ref().map_or(1.0, |ui| ui.pixels_per_point),
        };
        if let Some(ui) = &mut ui {
            let size = sd.size_in_pixels;
            let scale = ui.pixels_per_point;
            let glass_supported = self.ui_blur.is_some() || self.config.usage.contains(wgpu::TextureUsages::COPY_SRC);
            let (labels, rest) = ui.primitives.split_at_mut(labels_end);
            // the labels show the scene alone, blurred where they are; the
            // windows show it with its labels and the HUD attachments
            let label_glass = crate::ui_glass::bounds(labels, scale, size).filter(|_| glass_supported);
            window_glass = glass_supported && crate::ui_glass::requested(rest);
            if label_glass.is_some() || window_glass {
                if !self.ui_glass.as_ref().is_some_and(|glass| glass.matches(size, scale)) {
                    if let Some(old) = self.ui_glass.take() {
                        old.free(&mut self.egui);
                    }
                    self.ui_glass = Some(crate::ui_glass::Glass::new(
                        &self.device,
                        &mut self.egui,
                        self.config.format,
                        size,
                        scale,
                    ));
                }
            } else if let Some(old) = self.ui_glass.take() {
                old.free(&mut self.egui);
            }
            match (label_glass, &mut self.ui_glass) {
                (Some(area), Some(glass)) => {
                    let scene = self.ui_blur.as_ref().map_or(surface_tex.texture(), |blur| &blur.source);
                    let texture = glass.encode_under(&self.device, &mut self.egui, &mut encoder, scene, area);
                    glass.resolve(labels, scale, texture);
                }
                _ => crate::ui_glass::skip(labels),
            }
            match &self.ui_glass {
                Some(glass) if window_glass => glass.resolve(rest, scale, glass.texture),
                _ => crate::ui_glass::skip(rest),
            }
            if self.label_depth.enabled() {
                // the labels leave egui-wgpu's hands: drawn below behind
                // what the scene has in front of them
                self.label_depth.prepare(
                    &self.device,
                    &self.queue,
                    &self.targets.depth_ss,
                    labels,
                    &crate::label_depth::LabelView {
                        view_proj: unjittered_vp,
                        pixels_per_point: scale,
                        size,
                        jitter: depth_jitter,
                    },
                );
            } else {
                crate::label_depth::skip(labels);
            }
            self.previews
                .render(&self.device, &self.queue, &mut encoder, &mut self.egui, &mut ui.primitives, scale);
            extra_cmds = self
                .egui
                .update_buffers(&self.device, &self.queue, &mut encoder, &ui.primitives, &sd);
            meshes_uploaded = crate::ui_frame::meshes_uploaded(&ui.primitives);
        }
        let draw_ui = |encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, primitives: &[egui::ClippedPrimitive]| {
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("egui"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
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
            let jobs = crate::ui_frame::paint_jobs(primitives, meshes_uploaded);
            self.egui.render(&mut pass, &jobs, &sd);
        };
        if !self.label_depth.enabled() {
            // AURORA_LABEL_OCCLUSION=0: over the scene, by egui-wgpu
            if labels_end > 0
                && let Some(ui) = &mut ui
            {
                draw_ui(&mut encoder, &surface_view, &ui.primitives[..labels_end]);
                // egui starts its mesh-buffer iterators at zero on each
                // render. Empty clips advance them without drawing the
                // labels a second time with the rest of the interface.
                for primitive in &mut ui.primitives[..labels_end] {
                    primitive.clip_rect = egui::Rect::NOTHING;
                }
            }
        } else if labels_end > 0 && ui.is_some() && self.label_depth.any() {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("world labels"),
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
            self.label_depth.render(&mut pass, &self.egui);
        }
        if !f.rlva_overlays.is_empty() {
            self.rlva_overlays
                .get_or_insert_with(|| crate::rlva::overlay::Overlays::new(&self.device, self.config.format, &self.tex_layout))
                .encode(&self.queue, &mut encoder, &surface_view, &scene.textures, &f.rlva_overlays);
            calls.set(calls.get() + 1);
        } else {
            self.rlva_overlays = None;
        }
        // Vision restrictions also apply to photos without the viewer UI.
        if wanted == Some(Capture::Scene) {
            let source = self.ui_blur.as_ref().map_or(surface_tex.texture(), |blur| &blur.source);
            capture = self.encode_capture(&mut encoder, source, false);
        }
        // HUDs retain their original texture colors and never inherit world
        // depth, fog, shadows, exposure or post effects (render_hud_attachments).
        if draw_huds && let Some((_, _, depth)) = &self.hud_depth {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("HUD attachments"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &surface_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: match (&self.timer, timer_slot) {
                    (Some(t), Some(_)) if hud_select_faces_range.1 == 0 => Some(wgpu::RenderPassTimestampWrites {
                        query_set: &t.query_set,
                        beginning_of_pass_write_index: None,
                        end_of_pass_write_index: Some(1),
                    }),
                    _ => None,
                },
                occlusion_query_set: None,
                multiview_mask: None,
            });
            bind_mesh(&mut pass, &self.groups.refl[5]);
            draw(&mut pass, &self.pipelines.hud_opaque, hud_opaque_range);
            draw(&mut pass, &self.pipelines.hud_blend, hud_blend_range);
            draw(&mut pass, &self.pipelines.hud_debug_red, hud_debug_red_range);
            draw(&mut pass, &self.pipelines.hud_debug_blue, hud_debug_blue_range);
        }
        if hud_select_faces_range.1 > 0
            && let (Some(selection), Some((_, _, depth))) = (&self.hud_selection, &self.hud_depth)
        {
            let mut pass = selection.begin_mask(&mut encoder, depth);
            bind_mesh(&mut pass, &self.groups.refl[5]);
            draw(&mut pass, &self.pipelines.selection_mask, hud_select_faces_range);
            drop(pass);
            let timestamps = match (&self.timer, timer_slot) {
                (Some(t), Some(_)) => Some(wgpu::RenderPassTimestampWrites {
                    query_set: &t.query_set,
                    beginning_of_pass_write_index: None,
                    end_of_pass_write_index: Some(1),
                }),
                _ => None,
            };
            selection.encode(&mut encoder, &surface_view, timestamps);
            calls.set(calls.get() + 2);
        }
        if let (Some(t), Some(slot)) = (&self.timer, timer_slot) {
            encoder.resolve_query_set(&t.query_set, 0..GPU_TIMESTAMPS, &t.resolve, 0);
            encoder.copy_buffer_to_buffer(&t.resolve, 0, &t.readback[slot].0, 0, GPU_TIMESTAMPS as u64 * 8);
        }
        // ---- egui: the rest of the interface, over the HUD attachments
        if let Some(ui) = &mut ui {
            if window_glass && let Some(glass) = &self.ui_glass {
                let scene = self.ui_blur.as_ref().map_or(surface_tex.texture(), |blur| &blur.source);
                glass.encode(&mut encoder, scene);
            }
            if let (Some(boundary), Some(blur)) = (blur_boundary, &self.ui_blur) {
                draw_ui(&mut encoder, &surface_view, &ui.primitives[..boundary]);
                blur.encode(&mut encoder, &present_view);
                // egui starts its mesh-buffer iterators at zero on each render.
                // Empty clips advance them without drawing the lower layers a
                // second time. Buffers and texture uploads remain shared.
                for primitive in &mut ui.primitives[..boundary] {
                    primitive.clip_rect = egui::Rect::NOTHING;
                }
                draw_ui(&mut encoder, &present_view, &ui.primitives);
            } else {
                draw_ui(&mut encoder, &surface_view, &ui.primitives);
            }
        }

        prof.mark("egui");
        if wanted == Some(Capture::Full) {
            capture = self.encode_capture(&mut encoder, surface_tex.texture(), false);
        } else if snapshot.is_some() {
            capture = self.encode_capture(&mut encoder, surface_tex.texture(), true);
        }
        // finish() validates, tracks and encodes every command of its
        // encoder: the three run at once (wgpu-core only takes a shared
        // lock there), two on the helper threads and one here, then are
        // submitted in frame order. The scene's encoder, the longest, is
        // the one finished here: handed to a helper, the frame also waited
        // for that helper to wake up, for the few commands of the last
        // encoder (finish 0.56 -> 0.51 ms, 1.9 -> 1.6 ms in the longest
        // frame of a second)
        let (pre, post, scene) = self.finishers.pair(enc_pre, encoder, move || enc_scene.finish());
        match (pre, post) {
            (Some(pre), Some(post)) => extra_cmds.extend([pre, scene, post]),
            _ => {
                log::error!("render: a command encoder thread panicked");
                let spent = Spent { writes, lists: draw_lists };
                return self.skip(index, stats, upload_cmds, frame_uploads, spent);
            }
        }
        prof.mark("finish");
        self.queue.submit(upload_cmds.into_iter().chain(extra_cmds));
        // staging chunks whose copies are now submitted are mapped again
        frame_uploads.after_submit();
        let mut captured = None;
        if let Some((buf, w, h, row)) = capture {
            buf.map_async(wgpu::MapMode::Read, .., |_| {});
            // (a device lost before: no image, the photo fails)
            if self.lost.wait(&self.device)
                && let Ok(view) = buf.get_mapped_range(..)
            {
                let bgra = matches!(
                    self.config.format,
                    wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
                );
                // (a cut-out photo keeps its coverage, cutout.rs)
                let opaque = if f.cutout { 0 } else { 255 };
                let mut out = Vec::with_capacity((w * h * 4) as usize);
                for y in 0..h {
                    let r = &view[(y * row) as usize..(y * row + w * 4) as usize];
                    for px in r.as_chunks::<4>().0 {
                        let a = px[3] | opaque;
                        if bgra {
                            out.extend_from_slice(&[px[2], px[1], px[0], a]);
                        } else {
                            out.extend_from_slice(&[px[0], px[1], px[2], a]);
                        }
                    }
                }
                captured = Some((w, h, out));
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
        self.cull.after_submit();
        self.depth_pick.after_submit();
        self.exposure.after_submit();
        self.histogram.after_submit();
        self.drawn_pick.after_submit();
        prof.mark("submit");
        let encode_ms = t0.elapsed().as_secs_f32() * 1000.0;
        if let FrameTarget::Surface(t) = surface_tex {
            self.queue.present(t);
            stats.presented = true;
        }
        self.lost.poll(&self.device);
        // a click answered by now goes back with this frame's result
        self.depth_pick.harvest_clicks();
        self.drawn_pick.harvest();
        prof.mark("present");
        let phase = |step: &str| RENDER_PHASES.iter().position(|p| *p == step);
        stats.cpu_phases[0] = (t0 - t_acquire).as_secs_f32() * 1000.0;
        // the journal's replay is part of "resources"
        if let Some(i) = phase("resources") {
            stats.cpu_phases[i] = writes_ms;
        }
        for (step, ms, waited) in prof.steps() {
            if let Some(i) = phase(step) {
                stats.cpu_phases[i] += ms;
                stats.cpu_waits[i] += waited;
            }
        }
        stats.draw_calls = calls.get();
        let profile_frames = profile_frames();
        if profile_frames {
            let mut out = format!("acquire={:.2} writes={writes_ms:.2} ", stats.cpu_phases[0]);
            for (step, ms, _) in prof.steps() {
                out.push_str(&format!("{step}={ms:.2} "));
            }
            log::info!("render profile: {out}");
        }

        stats.gpu_ms = self.timer.as_ref().and_then(|t| t.last_ms);
        stats.gpu_elements = self.timer.as_ref().and_then(|t| t.last_elements);
        stats.shadow.gpu_ms = self.timer.as_ref().and_then(|t| t.last_shadow_parts);
        stats.gpu_depth = self.timer.as_ref().and_then(|t| t.last_depth);
        stats.gpu_ssao = self.timer.as_ref().and_then(|t| t.last_ssao);
        stats.gpu_contact = self.timer.as_ref().and_then(|t| t.last_contact);
        stats.gpu_bloom = self.timer.as_ref().and_then(|t| t.last_bloom);
        stats.gpu_effects = self.timer.as_ref().and_then(|t| t.last_effects);
        if profile_frames && let Some(e) = stats.gpu_elements {
            let mut out = String::new();
            for (el, ms) in GpuElement::ALL.iter().zip(e) {
                out.push_str(&format!("{}={ms:.3} ", el.key()));
            }
            log::info!("gpu profile: {out}");
        }
        stats.cpu_encode_ms = encode_ms;
        stats.water_reflection = do_water_refl;
        stats.mirror = mirror.is_some();
        let spent = Spent { writes, lists: draw_lists };
        self.result(index, stats, captured, spent)
    }

    /// Copy the swapchain image to a readback buffer (frame captures).
    /// `offscreen`: a texture of the renderer's own (a photo), always
    /// copyable, unlike the swapchain.
    fn encode_capture(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        texture: &wgpu::Texture,
        offscreen: bool,
    ) -> Option<(wgpu::Buffer, u32, u32, u32)> {
        if !offscreen && !self.config.usage.contains(wgpu::TextureUsages::COPY_SRC) {
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

    /// World position of the opaque surface under a pixel of the last frame
    /// drawn (depth prepass read-back; None for the sky). Blocks on the GPU
    /// for one tiny copy: clicks only.
    pub fn pick_world(&mut self, x: f32, y: f32) -> Option<Vec3> {
        let (w, h) = (self.config.width, self.config.height);
        if x < 0.0 || y < 0.0 || x >= w as f32 || y >= h as f32 {
            return None;
        }
        let px = crate::pick::PickPixel {
            x: crate::rlva::resolution::pixel((x as u32, y as u32), (w, h), self.scene_size()).0,
            y: crate::rlva::resolution::pixel((x as u32, y as u32), (w, h), self.scene_size()).1,
            width: self.scene_size().0,
            height: self.scene_size().1,
            inv_view_proj: self.last_inv_vp,
        };
        self.depth_pick
            .blocking(&self.device, &self.queue, &self.lost, &self.targets.depth_ss, px)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn specular_aa_follows_the_setting_unless_forced_and_alternates_each_second() {
        assert_eq!(SpecularAa::parse(None), SpecularAa::Setting);
        assert_eq!(SpecularAa::parse(Some(" 0 ")), SpecularAa::Off);
        assert_eq!(SpecularAa::parse(Some("1")), SpecularAa::On);
        assert_eq!(SpecularAa::parse(Some("alt")), SpecularAa::Alternate);
        assert!((0..4).all(|s| SpecularAa::Setting.applied(true, s)));
        assert!((0..4).all(|s| !SpecularAa::Setting.applied(false, s)));
        assert!((0..4).all(|s| SpecularAa::On.applied(false, s)));
        assert!((0..4).all(|s| !SpecularAa::Off.applied(true, s)));
        let alt: Vec<bool> = (0..4).map(|s| SpecularAa::Alternate.applied(false, s)).collect();
        assert_eq!(alt, [true, false, true, false]);
    }

    #[test]
    fn pbr_occlusion_is_on_unless_turned_off_and_alternates_one_second_in_three() {
        assert_eq!(PbrOcclusion::parse(None), PbrOcclusion::On);
        assert_eq!(PbrOcclusion::parse(Some(" 0 ")), PbrOcclusion::Off);
        assert_eq!(PbrOcclusion::parse(Some("alt")), PbrOcclusion::Alternate);
        assert_eq!(PbrOcclusion::parse(Some("1")), PbrOcclusion::On);
        assert!((0..6).all(|s| PbrOcclusion::On.applied(s)));
        assert!((0..6).all(|s| !PbrOcclusion::Off.applied(s)));
        let alt: Vec<bool> = (0..6).map(|s| PbrOcclusion::Alternate.applied(s)).collect();
        assert_eq!(alt, [false, true, true, false, true, true]);
    }

    /// The object vertex shader's arithmetic, in single precision like the
    /// GPU, against the exact position: a 20 cm object 40 cm in front of a
    /// camera turning slowly around it, 2 106 m above the ground, in a
    /// 1359-pixel-high window (the report). Largest error in pixels, in
    /// region coordinates (before) and relative to the eye.
    fn vertex_errors(exact_eye: bool) -> (f64, f64) {
        use glam::{DMat4, DVec3, DVec4};
        // (look_at_mat4, in double precision)
        let exact_view = |eye: DVec3, center: DVec3| {
            let f = (center - eye).normalize();
            let s = f.cross(DVec3::Z).normalize();
            let u = s.cross(f);
            DMat4::from_cols(
                DVec4::new(s.x, u.x, -f.x, 0.0),
                DVec4::new(s.y, u.y, -f.y, 0.0),
                DVec4::new(s.z, u.z, -f.z, 0.0),
                DVec4::new(-s.dot(eye), -u.dot(eye), f.dot(eye), 1.0),
            )
        };
        let proj = glam::camera::rh::proj::directx::perspective_infinite_reverse(60f32.to_radians(), 1278.0 / 1359.0, 0.1);
        let half = glam::DVec2::new(1278.0, 1359.0) * 0.5;
        let target = Vec3::new(120.3, 186.9, 2106.25);
        let translation = target + Vec3::new(0.03, 0.02, -0.01);
        let (mut world, mut eye_relative) = (0f64, 0f64);
        for step in 0..400 {
            let a = step as f32 * 1e-4;
            let eye = target + Vec3::new(-0.4 * a.cos(), -0.4 * a.sin(), 0.07);
            let view = glam::camera::rh::view::look_at_mat4(eye, target, Vec3::Z);
            let exact_vp = proj.as_dmat4() * exact_view(eye.as_dvec3(), target.as_dvec3());
            let relative_vp = eye_view_proj(proj, view, eye, exact_eye);
            let world_vp = proj * view;
            for corner in 0..8 {
                let local = Vec3::new(
                    if corner & 1 == 0 { -0.1 } else { 0.1 },
                    if corner & 2 == 0 { -0.1 } else { 0.1 },
                    if corner & 4 == 0 { -0.1 } else { 0.1 },
                );
                let pixels = |clip: glam::DVec4| glam::DVec2::new(clip.x, clip.y) / clip.w * half;
                let exact = pixels(exact_vp * (translation.as_dvec3() + local.as_dvec3()).extend(1.0));
                let wp = Mat4::from_translation(translation) * local.extend(1.0);
                world = world.max((pixels((world_vp * wp).as_dvec4()) - exact).abs().max_element());
                let ep = Mat4::from_translation(translation - eye) * local.extend(1.0);
                eye_relative = eye_relative.max((pixels((relative_vp * ep).as_dvec4()) - exact).abs().max_element());
            }
        }
        (world, eye_relative)
    }

    #[test]
    fn eye_relative_vertices_land_on_their_pixel_high_above_the_ground() {
        let (world, eye_relative) = vertex_errors(true);
        // in region coordinates the vertices are off by a good part of a pixel
        assert!(world > 0.2, "{world}");
        assert!(eye_relative < 0.01, "{eye_relative}");
    }

    #[test]
    fn eye_relative_view_of_an_approximate_eye_keeps_the_view_it_was_given() {
        // (reflections, probes: the eye is derived from the view, whose
        // translation carries the rounding of its construction, 0.7 pixel in
        // the case above; the vertices themselves add nothing to it)
        let (_, eye_relative) = vertex_errors(false);
        assert!(eye_relative < 1.0, "{eye_relative}");
        let proj = Mat4::IDENTITY;
        let view = glam::camera::rh::view::look_at_mat4(Vec3::new(5.0, 6.0, 7.0), Vec3::ZERO, Vec3::Z);
        let eye = Vec3::new(5.0, 6.0, 7.0);
        let p = Vec3::new(1.0, -2.0, 3.0);
        let relative = eye_view_proj(proj, view, eye, false) * (p - eye).extend(1.0);
        assert!(relative.abs_diff_eq(view * p.extend(1.0), 1e-5));
    }

    #[test]
    fn sky_directions_hold_still_high_above_the_ground() {
        // A camera walking on a skybox 2 447 m up (the report), in a 720-pixel
        // high window. The sky shader's arithmetic in single precision against
        // the exact direction of the same view: largest error in pixels.
        let fov = 60f32.to_radians();
        let proj = glam::camera::rh::proj::directx::perspective_infinite_reverse(fov, 16.0 / 9.0, 0.1);
        let pixel = f64::from(fov) / 720.0;
        let (mut world, mut rotation) = (0f64, 0f64);
        for step in 0..200 {
            let s = step as f32;
            let eye = Vec3::new(206.0 + s * 0.031, 189.0 - s * 0.017, 2448.7);
            let target = eye + Vec3::new((s * 0.05).cos() * 3.0, (s * 0.05).sin() * 3.0, -0.5);
            let view = glam::camera::rh::view::look_at_mat4(eye, target, Vec3::Z);
            let exact_inv = (proj.as_dmat4() * view.as_dmat4()).inverse();
            let exact_eye = view.as_dmat4().inverse().w_axis.truncate();
            let inv_world = (proj * view).inverse();
            let camera = view.inverse().transform_point3(Vec3::ZERO);
            let inv_rot = inv_view_proj_rot(proj, view);
            for corner in 0..9 {
                let ndc = glam::Vec2::new((corner % 3) as f32 - 1.0, (corner / 3) as f32 - 1.0) * 0.9;
                let p = ndc.extend(0.5).extend(1.0);
                let e = exact_inv * p.as_dvec4();
                let exact = (e.truncate() / e.w - exact_eye).normalize();
                let angle = |d: Vec3| d.as_dvec3().normalize().dot(exact).clamp(-1.0, 1.0).acos() / pixel;
                let w = inv_world * p;
                world = world.max(angle(w.truncate() / w.w - camera));
                let r = inv_rot * p;
                rotation = rotation.max(angle(r.truncate() / r.w));
            }
        }
        // from region positions the sky shook by about a pixel (#1059)
        assert!(world > 0.3, "{world}");
        assert!(rotation < 0.02, "{rotation}");
    }

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
    fn shadow_receiver_bias_is_two_texels_along_the_light() {
        // a cascade as cascade_matrices builds it: a 6 m square, the depth
        // padded by CASCADE_DEPTH_PAD on each side
        let radius = 6.0f32;
        let light = Vec3::new(0.3, -0.2, -0.93).normalize();
        let center = Vec3::new(128.0, 128.0, 25.0);
        let eye = center - light * (radius + crate::shadow_cull::CASCADE_DEPTH_PAD);
        let view = glam::camera::rh::view::look_at_mat4(eye, center, Vec3::Z);
        let depth = radius * 2.0 + 2.0 * crate::shadow_cull::CASCADE_DEPTH_PAD;
        let m = glam::camera::rh::proj::directx::orthographic(-radius, radius, -radius, radius, 0.1, depth) * view;
        let texel = 1.0 / 2048.0;
        let bias = shadow_receiver_bias(m, texel, radius);
        // the same bias in metres: two texels of 2 x 6 m / 2048
        let metres = |b: f32| {
            let a = m.project_point3(center);
            let p = m.project_point3(center - light * 1.0);
            b / (a.z - p.z).abs()
        };
        assert!((metres(bias) - 2.0 * 2.0 * radius / 2048.0).abs() < 1e-4, "{}", metres(bias));
        // the bias of before was about 0.4 m in the finest cascade
        assert!((metres(0.0005) - 0.406).abs() < 0.01, "{}", metres(0.0005));
    }

    /// A tile of a photo gets back the projection of the whole photo, the
    /// one its sun shadows are fitted to; a frame in one pass keeps its own.
    #[test]
    fn a_tile_shadows_the_whole_photo() {
        let photo = glam::camera::rh::proj::directx::perspective_infinite_reverse(1.0, 16.0 / 9.0, 0.1);
        assert_eq!(whole_proj(photo, Mat4::IDENTITY), photo);
        // the right-hand tile of the middle row of a 3 x 3 grid (snapshot::tile_matrix)
        let tile = Mat4::from_cols(Vec4::X * 3.0, Vec4::Y * 3.0, Vec4::Z, Vec4::new(-2.0, 0.0, 0.0, 1.0));
        assert!(whole_proj(tile * photo, tile).abs_diff_eq(photo, 1e-5));
        // the photo's centre, around which the cascades are laid, is
        // outside the tile
        assert!((tile * photo).project_point3(Vec3::new(0.0, 0.0, -10.0)).x < -1.0);
    }

    /// The glow of a tile reads the whole photo's texels: the tile knows
    /// where it lies in the photo, from the top left.
    #[test]
    fn a_tile_knows_its_part_of_the_photo() {
        assert_eq!(photo_part(Mat4::IDENTITY), [0.0, 0.0, 1.0, 1.0]);
        let near = |a: [f32; 4], b: [f32; 4]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6);
        // the right-hand tile of the middle row of a 3 x 3 grid
        let tile = Mat4::from_cols(Vec4::X * 3.0, Vec4::Y * 3.0, Vec4::Z, Vec4::new(-2.0, 0.0, 0.0, 1.0));
        assert!(
            near(photo_part(tile), [2.0 / 3.0, 1.0 / 3.0, 1.0 / 3.0, 1.0 / 3.0]),
            "{:?}",
            photo_part(tile)
        );
        // the top left quarter (its centre at -0.5, 0.5)
        let tile = Mat4::from_cols(Vec4::X * 2.0, Vec4::Y * 2.0, Vec4::Z, Vec4::new(1.0, -1.0, 0.0, 1.0));
        assert!(near(photo_part(tile), [0.0, 0.0, 0.5, 0.5]), "{:?}", photo_part(tile));
    }

    #[test]
    fn shadow_bias_switch_alternates_by_the_second() {
        let at = |s: u64| std::time::UNIX_EPOCH + std::time::Duration::from_secs(s);
        assert!(!shadow_bias_legacy_now(0, at(10)));
        assert!(shadow_bias_legacy_now(1, at(11)));
        assert!(shadow_bias_legacy_now(2, at(10)));
        assert!(!shadow_bias_legacy_now(2, at(11)));
    }

    #[test]
    fn blend_ao_switch_reads_its_modes() {
        let at = |s: u64| std::time::UNIX_EPOCH + std::time::Duration::from_secs(s);
        assert_eq!(BlendAo::from_env(None), BlendAo::New);
        assert_eq!(BlendAo::from_env(Some("new")), BlendAo::New);
        assert_eq!(BlendAo::from_env(Some(" none ")), BlendAo::None);
        assert_eq!(BlendAo::from_env(Some("old")), BlendAo::Old);
        assert_eq!(BlendAo::from_env(Some("alt")).now(at(10)), BlendAo::None);
        assert_eq!(BlendAo::from_env(Some("alt")).now(at(11)), BlendAo::New);
        assert_eq!(BlendAo::Old.now(at(10)), BlendAo::Old);
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

    /// The scene pass tests its vertices against the depth the prepasses
    /// wrote (`GreaterEqual`): a fragment whose depth is one step under
    /// the prepass's is rejected and shows what is behind (the sky seen
    /// through the ground). Draws a plain face, a rigged one placed by its
    /// record, a rigged one placed by its skeleton's anchor, a worn prim
    /// placed by its attachment point's anchor and a terrain patch, 3 000 m
    /// up, through the prepass's vertex shader and through the scene
    /// pass's (`vs_main`, `vs_terrain`) with 4 samples, and compares the
    /// two depths sample for sample. Needs a GPU:
    /// `cargo test -p aurora-render prepass_depth -- --ignored`.
    #[test]
    #[ignore]
    fn gpu_prepass_depth_is_the_scene_depth() {
        use glam::Quat;
        use wgpu::util::DeviceExt;
        const SIZE: u32 = 256;
        const SAMPLES: u32 = 4;
        let _gpu = crate::test_gpu::lock();
        let instance = wgpu::Instance::default();
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        // (the scene shader declares the texture pages as a binding array)
        let bindless =
            wgpu::Features::TEXTURE_BINDING_ARRAY | wgpu::Features::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING;
        if !adapter.features().contains(bindless) {
            eprintln!("no texture binding arrays: skipped");
            return;
        }
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_features: bindless,
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .expect("GPU device");
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("scene"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "{}\n{}\n{}",
                    crate::common_shader_source(),
                    crate::water_shader_source(&["object.wgsl"], include_str!("shaders/object.wgsl")),
                    crate::water_shader_source(&["env.wgsl"], include_str!("shaders/env.wgsl"))
                )
                .into(),
            ),
        });
        // depth only, like the prepass of the multisampled depth
        let pipeline = |vs: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(vs),
                layout: None,
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some(vs),
                    compilation_options: Default::default(),
                    buffers: &[Some(vertex_layout()), Some(skin_layout())],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::Greater),
                    stencil: wgpu::StencilState::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState {
                    count: SAMPLES,
                    mask: !0,
                    alpha_to_coverage_enabled: false,
                },
                fragment: None,
                multiview_mask: None,
                cache: None,
            })
        };

        // a bumpy grid, every vertex weighted on four joints
        const N: u32 = 24;
        let mut vertices = Vec::new();
        let mut skin = Vec::new();
        for j in 0..=N {
            for i in 0..=N {
                let (u, v) = (i as f32 / N as f32, j as f32 / N as f32);
                let bump = 0.07 * ((u * 9.0).sin() + (v * 7.0).cos());
                vertices.push(Vertex::new([u - 0.5, v - 0.5, bump], [0.0, 0.0, 1.0], [u, v]));
                let a = (u * 255.0) as u8;
                let b = ((1.0 - u) * v * 255.0) as u8;
                let rest = 255 - a - b;
                skin.push(SkinVertex {
                    joints: [0, 1, 2, 3],
                    weights: [a, b, rest / 2, rest - rest / 2],
                });
            }
        }
        let mut indices: Vec<u16> = Vec::new();
        for j in 0..N {
            for i in 0..N {
                let k = (j * (N + 1) + i) as u16;
                let below = k + N as u16 + 1;
                indices.extend([k, k + 1, below, k + 1, below + 1, below]);
            }
        }
        let buffer = |label: &str, contents: &[u8], usage: wgpu::BufferUsages| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents,
                usage,
            })
        };
        let vertex_buf = buffer("vertices", bytemuck::cast_slice(&vertices), wgpu::BufferUsages::VERTEX);
        let skin_buf = buffer("skin", bytemuck::cast_slice(&skin), wgpu::BufferUsages::VERTEX);
        let index_buf = buffer("indices", bytemuck::cast_slice(&indices), wgpu::BufferUsages::INDEX);

        // high above the ground, where region coordinates only hold 0.24 mm
        let center = Vec3::new(131.37, 97.81, 3000.43);
        let eye = center + Vec3::new(-2.3, -3.1, 1.2);
        let proj = glam::camera::rh::proj::directx::perspective_infinite_reverse(60f32.to_radians(), 1.0, 0.1);
        let view = glam::camera::rh::view::look_at_mat4(eye, center, Vec3::Z);
        let mut frame = FrameU::zeroed();
        frame.view_proj = (proj * view).to_cols_array_2d();
        frame.eye_view_proj = eye_view_proj(proj, view, eye, true).to_cols_array_2d();
        frame.eye = eye.extend(0.0).to_array();
        frame.camera_pos = eye.extend(0.0).to_array();
        let frame_buf = buffer("frame", bytemuck::bytes_of(&frame), wgpu::BufferUsages::UNIFORM);

        // one skeleton at palette 208: four posed joints, its anchor, and
        // the anchor of an attachment point after it
        const SKELETON: u32 = 208;
        let place = |offset: Vec3, turn: f32| {
            Mat4::from_scale_rotation_translation(
                Vec3::new(1.9, 2.3, 1.4),
                Quat::from_euler(glam::EulerRot::ZXY, turn, 0.4 * turn, -0.7),
                center + offset,
            )
        };
        const LIP_BANK: u32 = SKELETON + 212;
        // the sparse live morph bank: per vertex (first entry, count), then
        // two entries per vertex, weighed by columns 4 and 5; column 15 says
        // the part moves
        let mut bank: Vec<[f32; 4]> = Vec::new();
        let entries = 4 * LIP_BANK as usize + vertices.len();
        for i in 0..vertices.len() {
            bank.push([(entries + 4 * i) as f32, 2.0, 0.0, 0.0]);
        }
        for vertex in &vertices {
            bank.extend([[0.02, 0.0, vertex.pos[0] * 0.09, 4.0], [0.0, 0.01, 0.0, 0.0]]);
            bank.extend([[0.0, -0.03, 0.02, 5.0], [0.0; 4]]);
        }
        let mut palettes = vec![Mat4::IDENTITY.to_cols_array_2d(); LIP_BANK as usize + bank.len().div_ceil(4)];
        palettes[(SKELETON + LIVE_MORPH_WEIGHTS) as usize] = [[0.0; 4], [0.7, 0.3, 0.0, 0.0], [0.0; 4], [0.0, 0.0, 0.0, 1.0]];
        for (i, v) in bank.iter().enumerate() {
            palettes[LIP_BANK as usize + i / 4][i % 4] = *v;
        }
        for j in 0..4 {
            let m = Mat4::from_rotation_translation(
                Quat::from_rotation_x(0.21 * j as f32 - 0.3),
                Vec3::new(0.013 * j as f32, -0.02, 0.031 * j as f32),
            );
            palettes[(SKELETON + j) as usize] = m.to_cols_array_2d();
        }
        let rigid = |offset: Vec3, turn: f32| Mat4::from_rotation_translation(Quat::from_rotation_z(turn), center + offset);
        palettes[(SKELETON + SKELETON_ANCHOR) as usize] = rigid(Vec3::new(0.9, 0.3, -0.2), 0.8).to_cols_array_2d();
        palettes[(SKELETON + SKELETON_ANCHOR + 1) as usize] = rigid(Vec3::new(-0.8, 0.7, 0.4), -1.1).to_cols_array_2d();
        let binds: Vec<SkinBind> = (0..4)
            .map(|j| SkinBind {
                inverse_bind: Mat4::from_translation(Vec3::new(0.004 * j as f32, 0.0, -0.002)).to_cols_array_2d(),
                joint: [j, 0, 0, 0],
            })
            .collect();
        let record = |model: Mat4, flags: [u32; 4]| DrawRecord {
            model: model.to_cols_array_2d(),
            flags,
            ..Default::default()
        };
        let local = Mat4::from_scale_rotation_translation(Vec3::splat(1.6), Quat::from_rotation_y(0.5), Vec3::new(0.1, 0.2, 0.05));
        let records = [
            // plain face
            record(place(Vec3::new(-0.6, -0.5, 0.0), 0.3), [0; 4]),
            // rigged mesh placed by its record
            record(place(Vec3::new(0.7, -0.6, 0.3), -0.9), [flags::SKINNED, SKELETON, 0, 0]),
            // rigged mesh placed by its skeleton's anchor
            record(Mat4::IDENTITY, [flags::SKINNED | flags::ANCHORED, SKELETON, 0, 0]),
            // worn prim placed by its attachment point's anchor
            record(local, [flags::ANCHORED, SKELETON + SKELETON_ANCHOR + 1, 0, 0]),
            // terrain patch (drawn with vs_terrain by the scene pass)
            record(place(Vec3::new(0.1, 0.4, -0.9), 0.1) * Mat4::from_scale(Vec3::splat(3.0)), [0; 4]),
            // Lip morphs must have exactly the same depth in both vertex passes.
            DrawRecord {
                anim: [0, 4 * LIP_BANK, vertices.len() as u32, 15],
                ..record(
                    place(Vec3::new(0.0, 0.2, 0.8), 0.4),
                    [flags::SKINNED | flags::LIP_SYNC, SKELETON, 0, 0],
                )
            },
        ];
        const TERRAIN: u32 = 4;
        let storage = wgpu::BufferUsages::STORAGE;
        let records_buf = buffer("records", bytemuck::cast_slice(&records), storage);
        let palettes_buf = buffer("palettes", bytemuck::cast_slice(&palettes), storage);
        let binds_buf = buffer("skin binds", bytemuck::cast_slice(&binds), storage);

        let depth = |label: &str| {
            make_view(
                &device,
                label,
                SIZE,
                SIZE,
                DEPTH_FORMAT,
                SAMPLES,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            )
            .1
        };
        let (prepass_depth, scene_depth) = (depth("prepass"), depth("scene"));
        let (pre, main, terrain) = (pipeline("vs_depth"), pipeline("vs_main"), pipeline("vs_terrain"));
        let groups = |p: &wgpu::RenderPipeline| {
            fn entry(binding: u32, b: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
                wgpu::BindGroupEntry {
                    binding,
                    resource: b.as_entire_binding(),
                }
            }
            [
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: None,
                    layout: &p.get_bind_group_layout(0),
                    entries: &[entry(0, &frame_buf)],
                }),
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: None,
                    layout: &p.get_bind_group_layout(1),
                    entries: &[entry(0, &records_buf), entry(1, &palettes_buf), entry(2, &binds_buf)],
                }),
            ]
        };
        let mut encoder = device.create_command_encoder(&Default::default());
        let draws: [(&wgpu::TextureView, [&wgpu::RenderPipeline; 2]); 2] =
            [(&prepass_depth, [&pre, &pre]), (&scene_depth, [&main, &terrain])];
        for (target, [objects, ground]) in draws {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: target,
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
            pass.set_vertex_buffer(0, vertex_buf.slice(..));
            pass.set_vertex_buffer(1, skin_buf.slice(..));
            pass.set_index_buffer(index_buf.slice(..), wgpu::IndexFormat::Uint16);
            for r in 0..records.len() as u32 {
                let p = if r == TERRAIN { ground } else { objects };
                let [g0, g1] = groups(p);
                pass.set_pipeline(p);
                pass.set_bind_group(0, &g0, &[]);
                pass.set_bind_group(1, &g1, &[]);
                pass.draw_indexed(0..indices.len() as u32, 0, r..r + 1);
            }
        }

        // per pixel: samples whose two depths differ, samples drawn
        let compare = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("compare"),
            source: wgpu::ShaderSource::Wgsl(
                "@group(0) @binding(0) var a: texture_depth_multisampled_2d;
                 @group(0) @binding(1) var b: texture_depth_multisampled_2d;
                 @vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
                     return vec4<f32>(f32((i << 1u) & 2u) * 2.0 - 1.0, f32(i & 2u) * 2.0 - 1.0, 0.0, 1.0);
                 }
                 @fragment fn fs(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
                     let c = vec2<i32>(p.xy);
                     var differ = 0.0;
                     var drawn = 0.0;
                     for (var s = 0; s < 4; s++) {
                         let da = textureLoad(a, c, s);
                         let db = textureLoad(b, c, s);
                         if (da != db) { differ += 1.0; }
                         if (da > 0.0) { drawn += 1.0; }
                     }
                     return vec4<f32>(differ / 255.0, drawn / 255.0, 0.0, 1.0);
                 }"
                .into(),
            ),
        });
        let compare_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("compare"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &compare,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &compare,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::TextureFormat::Rgba8Unorm.into())],
            }),
            multiview_mask: None,
            cache: None,
        });
        let result = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("result"),
            size: wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let compare_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &compare_pipeline.get_bind_group_layout(0),
            entries: &[tex(0, &prepass_depth), tex(1, &scene_depth)],
        });
        {
            let view = result.create_view(&Default::default());
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
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
            pass.set_pipeline(&compare_pipeline);
            pass.set_bind_group(0, &compare_group, &[]);
            pass.draw(0..3, 0..1);
        }
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: (SIZE * SIZE * 4) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            result.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(SIZE * 4),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
        );
        queue.submit([encoder.finish()]);
        readback.map_async(wgpu::MapMode::Read, .., |_| {});
        device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
        let view = readback.get_mapped_range(..).expect("mapped");
        let (mut differ, mut drawn) = (0u32, 0u32);
        for px in view.as_chunks::<4>().0 {
            differ += px[0] as u32;
            drawn += px[1] as u32;
        }
        // (every kind of face is on screen: well over a tenth of the samples)
        assert!(drawn > SIZE * SIZE * SAMPLES / 10, "{drawn} samples drawn");
        assert_eq!(differ, 0, "samples whose scene depth is not the prepass depth, of {drawn} drawn");
    }
}
