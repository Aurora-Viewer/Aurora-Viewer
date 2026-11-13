//! AMD FidelityFX Super Resolution 3.1, the upscaler, ported to WGSL
//! (`shaders/fsr/`) from the FidelityFX SDK 1.1.4 (MIT licence, see
//! NOTICE.md). The host side here follows ffx_fsr3upscaler.cpp
//! (FfxFsr3UpscalerContext's resources, constants and dispatches).
//!
//! FSR is one of the anti-aliasing choices, in the place of Aurora's TAA:
//! the world is drawn at the size of its quality mode (`FsrQuality`, the
//! display size for "native AA") and FSR draws it at the display size, the
//! passes after it (glow, bloom, tone mapping, the selection outline, the
//! depth of field...) working at that size with the depth it gives
//! (`display_depth`). It takes TAA's inputs: the HDR colour before the
//! tone mapping, the reverse-Z depth of the prepass, the prepass's motion
//! vectors (the camera's own motion where none was drawn, as taa.wgsl), and
//! TAA's jitter, over FSR's 8 x (display / render)^2 phases. Each pass is a full-screen
//! fragment pass (see `shaders/fsr/common.wgsl` for the differences from
//! the SDK). The reactive mask is generated from the scene before and after
//! the blended faces, the particles and the water (FSR's autogen reactive
//! pass, `AURORA_FSR_REACTIVE=0` turns it off). The transparency and
//! composition mask marks the faces with a running texture animation, read
//! from the prepass's normals (`NORMAL_COMPOSITION` in common.wgsl,
//! `AURORA_FSR_COMPOSITION=0` turns it off). The exposure is 1 (FSR's tone
//! mapping of the history works on the HDR values as they are).

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec2};

use crate::pipeline_cache::CachedPipelines;

const LUMA_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R16Float;
const MOTION_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg16Float;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;
const MASKS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const ACCUMULATION_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;
const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// The shading change pyramid's levels read (ffx_fsr3upscaler_shading_change.h
/// s_MipLevelsToUse).
const SHADING_CHANGE_LEVELS: u32 = 3;

/// ffxFsr3UpscalerGetJitterPhaseCount: 8 x (display / render)^2 phases.
pub(crate) fn jitter_phase_count(render_width: u32, display_width: u32) -> u32 {
    let ratio = display_width as f32 / render_width.max(1) as f32;
    ((8.0 * ratio * ratio) as u32).max(1)
}

/// RCAS sharpness from FSR's 0..1 setting (ffxFsr3UpscalerDispatch:
/// `-2 x sharpness + 2` stops, FsrRcasCon: `exp2(-stops)`).
pub(crate) fn rcas_scale(sharpness: f32) -> f32 {
    (-(-2.0 * sharpness.clamp(0.0, 1.0) + 2.0)).exp2()
}

/// Device depth to view depth for FSR's shaders (`z = y / (depth - x)`,
/// setupDeviceDepthToViewSpaceDepthParams): from the projection's own
/// terms, `depth = -m22 + m23 / z`; the epsilon keeps the infinite
/// reverse-Z's far plane (depth 0) finite, as the SDK's.
pub(crate) fn device_to_view(proj: Mat4) -> [f32; 2] {
    [-proj.z_axis.z - f32::EPSILON, proj.w_axis.z]
}

/// The material textures' mip bias while FSR draws the world smaller
/// than the window: `log2(render / display) - 1`, as AMD's integration
/// guide asks, so that the textures keep the window's detail (Quality
/// -1.58, Performance -2); 0 at the window's size, where the textures stay
/// those of TAA and the Legacy Viewer. `AURORA_FSR_MIP_BIAS=0` keeps 0
/// (comparison).
pub(crate) fn mip_bias(render_width: u32, display_width: u32) -> f32 {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let on = *ON.get_or_init(|| std::env::var("AURORA_FSR_MIP_BIAS").map_or(true, |v| v.trim() != "0"));
    if on { mip_bias_for(render_width, display_width) } else { 0.0 }
}

fn mip_bias_for(render_width: u32, display_width: u32) -> f32 {
    if display_width <= render_width {
        return 0.0;
    }
    (render_width.max(1) as f32 / display_width as f32).log2() - 1.0
}

/// ffxFsr3UpscalerGetRenderResolutionFromQualityMode: the display size
/// divided by the mode's ratio.
pub(crate) fn render_size(display: (u32, u32), ratio: f32) -> (u32, u32) {
    let ratio = ratio.max(1.0);
    (
        ((display.0 as f32 / ratio) as u32).max(1),
        ((display.1 as f32 / ratio) as u32).max(1),
    )
}

/// `AURORA_FSR_REACTIVE=0`: no generated reactive mask (read once).
fn reactive_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("AURORA_FSR_REACTIVE").map_or(true, |v| v.trim() != "0"))
}

/// `AURORA_FSR_COMPOSITION=0`: no transparency and composition mask (read
/// once).
fn composition_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("AURORA_FSR_COMPOSITION").map_or(true, |v| v.trim() != "0"))
}

/// `AURORA_FSR_SHARPNESS=<0..1>|off`: RCAS's strength, or no RCAS, over
/// the setting (read once).
fn sharpness_override() -> Option<Option<f32>> {
    static S: std::sync::OnceLock<Option<Option<f32>>> = std::sync::OnceLock::new();
    *S.get_or_init(|| std::env::var("AURORA_FSR_SHARPNESS").ok().map(|v| parse_sharpness(Some(&v))))
}

fn parse_sharpness(v: Option<&str>) -> Option<f32> {
    match v.map(str::trim) {
        Some("off") => None,
        Some(s) => Some(s.parse::<f32>().map_or(0.8, |x| x.clamp(0.0, 1.0))),
        None => Some(0.8),
    }
}

/// FSR's constant buffer (cbFSR3Upscaler, with the camera matrices of the
/// motion where the prepass drew none); `shaders/fsr/common.wgsl` FsrParams.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FsrU {
    inv_view_proj: [[f32; 4]; 4],
    prev_view_proj: [[f32; 4]; 4],
    render_size: [i32; 2],
    prev_render_size: [i32; 2],
    upscale_size: [i32; 2],
    prev_upscale_size: [i32; 2],
    max_render_size: [i32; 2],
    max_upscale_size: [i32; 2],
    device_to_view: [f32; 4],
    jitter: [f32; 2],
    prev_jitter: [f32; 2],
    downscale: [f32; 2],
    jitter_sequence_length: f32,
    delta_pre_exposure: f32,
    frame_index: f32,
    velocity_factor: f32,
    accumulation_added_per_frame: f32,
    min_disocclusion_accumulation: f32,
    exposure: f32,
    rcas: f32,
    view_to_meters: f32,
    motion_on: f32,
    reactive_on: f32,
    composition_on: f32,
    pad: [f32; 2],
}

/// What the frame gives FSR.
pub(crate) struct FrameInputs {
    /// This frame's jittered view projection.
    pub view_proj: Mat4,
    /// Last frame's unjittered one.
    pub prev_view_proj: Mat4,
    pub proj: Mat4,
    /// In pixels, y down: where the jitter moved the image.
    pub jitter: Vec2,
    /// The history starts again (first frame, camera cut).
    pub reset: bool,
    pub motion_on: bool,
    /// RCAS's strength (0..1), none without RCAS.
    pub sharpness: Option<f32>,
}

/// The shared inputs, made by the renderer.
pub(crate) struct Inputs<'a> {
    pub color: &'a wgpu::TextureView,
    pub depth: &'a wgpu::TextureView,
    pub motion: &'a wgpu::TextureView,
    /// The scene after the opaque faces only (the refraction's copy):
    /// with `color`, what the generated reactive mask compares.
    pub opaque: &'a wgpu::TextureView,
}

struct Pipelines {
    autogen_reactive: wgpu::RenderPipeline,
    prepare: wgpu::RenderPipeline,
    farthest_mip1: wgpu::RenderPipeline,
    shading_change_mip0: wgpu::RenderPipeline,
    downsample: wgpu::RenderPipeline,
    shading_change: wgpu::RenderPipeline,
    reactivity: wgpu::RenderPipeline,
    luma_instability: wgpu::RenderPipeline,
    accumulate: wgpu::RenderPipeline,
    rcas: wgpu::RenderPipeline,
    display_depth: wgpu::RenderPipeline,
}

/// One frame parity's bind groups (`[w]` writes the textures of parity w
/// and reads those of 1 - w).
struct Groups {
    autogen_reactive: wgpu::BindGroup,
    prepare: wgpu::BindGroup,
    farthest_mip1: wgpu::BindGroup,
    shading_change_mip0: wgpu::BindGroup,
    downsample: [wgpu::BindGroup; SHADING_CHANGE_LEVELS as usize - 1],
    shading_change: wgpu::BindGroup,
    reactivity: wgpu::BindGroup,
    luma_instability: wgpu::BindGroup,
    accumulate: wgpu::BindGroup,
    rcas: wgpu::BindGroup,
}

pub(crate) struct Fsr {
    pipelines: Pipelines,
    buffer: wgpu::Buffer,
    reconstructed: wgpu::Buffer,
    /// The world's size (FSR's render size).
    size: (u32, u32),
    /// FSR's output size, the display's.
    display: (u32, u32),
    /// The scene's depth at the display size, when it differs from the
    /// render size.
    display_depth: Option<wgpu::TextureView>,
    reactive: wgpu::TextureView,
    dilated_motion: wgpu::TextureView,
    dilated_depth: wgpu::TextureView,
    farthest: wgpu::TextureView,
    luma: [wgpu::TextureView; 2],
    farthest_mip1: wgpu::TextureView,
    /// One view a level of the shading change pyramid.
    spd_levels: Vec<wgpu::TextureView>,
    shading_change: wgpu::TextureView,
    masks: wgpu::TextureView,
    accumulation: [wgpu::TextureView; 2],
    luma_history: [wgpu::TextureView; 2],
    instability: wgpu::TextureView,
    internal: [wgpu::TextureView; 2],
    output: wgpu::TextureView,
    groups: [Groups; 2],
    parity: usize,
    frame_index: f32,
    prev_jitter: Vec2,
}

fn tex(binding: u32, filterable: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn storage(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn layout(device: &wgpu::Device, label: &str, extra: &[wgpu::BindGroupLayoutEntry]) -> wgpu::BindGroupLayout {
    let mut entries = vec![
        wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        },
        wgpu::BindGroupLayoutEntry {
            binding: 1,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        },
    ];
    entries.extend_from_slice(extra);
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(label),
        entries: &entries,
    })
}

fn module(device: &wgpu::Device, label: &str, pass: &str) -> wgpu::ShaderModule {
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(format!("{}\n{pass}", include_str!("shaders/fsr/common.wgsl")).into()),
    })
}

fn pipeline(
    device: &wgpu::Device,
    module: &wgpu::ShaderModule,
    layout: &wgpu::BindGroupLayout,
    label: &str,
    entry: &str,
    formats: &[wgpu::TextureFormat],
) -> wgpu::RenderPipeline {
    pipeline_to(device, module, layout, label, entry, formats, None)
}

/// `pipeline`, also writing `depth` (fs_display_depth).
fn pipeline_to(
    device: &wgpu::Device,
    module: &wgpu::ShaderModule,
    layout: &wgpu::BindGroupLayout,
    label: &str,
    entry: &str,
    formats: &[wgpu::TextureFormat],
    depth: Option<wgpu::TextureFormat>,
) -> wgpu::RenderPipeline {
    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    });
    let targets: Vec<Option<wgpu::ColorTargetState>> = formats
        .iter()
        .map(|&format| {
            Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })
        })
        .collect();
    device.create_render_pipeline_cached(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(&pl),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("vs_fsr"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(entry),
            compilation_options: Default::default(),
            targets: &targets,
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: depth.map(|format| wgpu::DepthStencilState {
            format,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Always),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

fn view(device: &wgpu::Device, label: &str, (w, h): (u32, u32), format: wgpu::TextureFormat) -> wgpu::TextureView {
    crate::renderer::make_view(
        device,
        label,
        w,
        h,
        format,
        1,
        wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | if format.is_depth_stencil_format() {
                wgpu::TextureUsages::COPY_SRC
            } else {
                wgpu::TextureUsages::empty()
            },
    )
    .1
}

fn entry_tex(binding: u32, v: &wgpu::TextureView) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: wgpu::BindingResource::TextureView(v),
    }
}

fn entry_buf(binding: u32, b: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: b.as_entire_binding(),
    }
}

impl Fsr {
    /// FSR's resources from the render `size` to the `display` size,
    /// reading `inputs`; `depth_format` is the scene depth's, which
    /// `display_depth` takes.
    pub(crate) fn new(
        device: &wgpu::Device,
        size: (u32, u32),
        display: (u32, u32),
        depth_format: wgpu::TextureFormat,
        inputs: &Inputs,
    ) -> Fsr {
        let size = (size.0.max(1), size.1.max(1));
        let display = (display.0.max(size.0), display.1.max(size.1));
        let half = ((size.0 / 2).max(1), (size.1 / 2).max(1));
        let l_prepare = layout(
            device,
            "fsr prepare",
            &[
                tex(2, true),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                tex(4, true),
                storage(5, false),
            ],
        );
        let l_tex3 = layout(device, "fsr pyramids", &[tex(2, true), tex(3, true), tex(4, true)]);
        let l_reactivity = layout(
            device,
            "fsr reactivity",
            &[
                tex(2, true),
                tex(3, false),
                tex(4, true),
                tex(5, true),
                tex(6, true),
                storage(7, true),
                storage(8, false),
                tex(9, true),
                tex(10, true),
            ],
        );
        let l_accumulate = layout(
            device,
            "fsr accumulate",
            &[
                tex(2, true),
                tex(3, true),
                tex(4, true),
                tex(5, true),
                tex(6, true),
                tex(7, true),
                storage(8, false),
            ],
        );
        let m_prepare = module(device, "fsr prepare", include_str!("shaders/fsr/prepare_inputs.wgsl"));
        let m_pyramids = module(device, "fsr pyramids", include_str!("shaders/fsr/pyramids.wgsl"));
        let m_reactivity = module(device, "fsr reactivity", include_str!("shaders/fsr/reactivity.wgsl"));
        let m_accumulate = module(device, "fsr accumulate", include_str!("shaders/fsr/accumulate.wgsl"));
        let pipelines = Pipelines {
            autogen_reactive: pipeline(
                device,
                &m_pyramids,
                &l_tex3,
                "fsr autogen reactive",
                "fs_autogen_reactive",
                &[ACCUMULATION_FORMAT],
            ),
            prepare: pipeline(
                device,
                &m_prepare,
                &l_prepare,
                "fsr prepare inputs",
                "fs_prepare_inputs",
                &[MOTION_FORMAT, DEPTH_FORMAT, LUMA_FORMAT, LUMA_FORMAT],
            ),
            farthest_mip1: pipeline(
                device,
                &m_pyramids,
                &l_tex3,
                "fsr farthest depth",
                "fs_farthest_depth_mip1",
                &[LUMA_FORMAT],
            ),
            shading_change_mip0: pipeline(
                device,
                &m_pyramids,
                &l_tex3,
                "fsr shading change pyramid",
                "fs_shading_change_mip0",
                &[MOTION_FORMAT],
            ),
            downsample: pipeline(device, &m_pyramids, &l_tex3, "fsr downsample", "fs_downsample", &[MOTION_FORMAT]),
            shading_change: pipeline(
                device,
                &m_pyramids,
                &l_tex3,
                "fsr shading change",
                "fs_shading_change",
                &[ACCUMULATION_FORMAT],
            ),
            reactivity: pipeline(
                device,
                &m_reactivity,
                &l_reactivity,
                "fsr prepare reactivity",
                "fs_prepare_reactivity",
                &[MASKS_FORMAT, ACCUMULATION_FORMAT],
            ),
            luma_instability: pipeline(
                device,
                &m_accumulate,
                &l_accumulate,
                "fsr luma instability",
                "fs_luma_instability",
                &[COLOR_FORMAT, LUMA_FORMAT],
            ),
            accumulate: pipeline(
                device,
                &m_accumulate,
                &l_accumulate,
                "fsr accumulate",
                "fs_accumulate",
                &[COLOR_FORMAT, COLOR_FORMAT],
            ),
            rcas: pipeline(device, &m_accumulate, &l_accumulate, "fsr rcas", "fs_rcas", &[COLOR_FORMAT]),
            display_depth: pipeline_to(
                device,
                &m_prepare,
                &l_prepare,
                "fsr display depth",
                "fs_display_depth",
                &[],
                Some(depth_format),
            ),
        };
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fsr params"),
            size: std::mem::size_of::<FsrU>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let pixels = u64::from(size.0) * u64::from(size.1) * 4;
        let display_pixels = u64::from(display.0) * u64::from(display.1) * 4;
        let reconstructed = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fsr reconstructed previous depth"),
            size: pixels,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // zeroed at creation; the accumulation zeroes what it read
        let new_locks = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fsr new locks"),
            size: display_pixels,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("fsr linear clamp"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let reactive = view(device, "fsr reactive mask", size, ACCUMULATION_FORMAT);
        let dilated_motion = view(device, "fsr dilated motion", size, MOTION_FORMAT);
        let dilated_depth = view(device, "fsr dilated depth", size, DEPTH_FORMAT);
        let farthest = view(device, "fsr farthest depth", size, LUMA_FORMAT);
        let luma = [
            view(device, "fsr luma", size, LUMA_FORMAT),
            view(device, "fsr luma", size, LUMA_FORMAT),
        ];
        let farthest_mip1 = view(device, "fsr farthest depth mip 1", half, LUMA_FORMAT);
        let levels = SHADING_CHANGE_LEVELS.min(32 - half.0.max(half.1).leading_zeros());
        let spd = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("fsr shading change pyramid"),
            size: wgpu::Extent3d {
                width: half.0,
                height: half.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: MOTION_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let spd_all = spd.create_view(&wgpu::TextureViewDescriptor::default());
        let spd_levels: Vec<wgpu::TextureView> = (0..levels)
            .map(|level| {
                spd.create_view(&wgpu::TextureViewDescriptor {
                    base_mip_level: level,
                    mip_level_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();
        let shading_change = view(device, "fsr shading change", half, ACCUMULATION_FORMAT);
        let masks = view(device, "fsr dilated reactive masks", size, MASKS_FORMAT);
        let accumulation = [
            view(device, "fsr accumulation", size, ACCUMULATION_FORMAT),
            view(device, "fsr accumulation", size, ACCUMULATION_FORMAT),
        ];
        let luma_history = [
            view(device, "fsr luma history", size, COLOR_FORMAT),
            view(device, "fsr luma history", size, COLOR_FORMAT),
        ];
        let instability = view(device, "fsr luma instability", size, LUMA_FORMAT);
        let internal = [
            view(device, "fsr internal colour", display, COLOR_FORMAT),
            view(device, "fsr internal colour", display, COLOR_FORMAT),
        ];
        let output = view(device, "fsr output", display, COLOR_FORMAT);
        let display_depth = (display != size).then(|| view(device, "fsr display depth", display, depth_format));
        let common = |l: &wgpu::BindGroupLayout, label: &str, extra: &[wgpu::BindGroupEntry]| {
            let mut entries = vec![
                entry_buf(0, &buffer),
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ];
            entries.extend_from_slice(extra);
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: l,
                entries: &entries,
            })
        };
        let groups = [0usize, 1].map(|w| {
            let r = 1 - w;
            Groups {
                autogen_reactive: common(
                    &l_tex3,
                    "fsr autogen reactive",
                    &[entry_tex(2, inputs.opaque), entry_tex(3, inputs.color), entry_tex(4, inputs.color)],
                ),
                prepare: common(
                    &l_prepare,
                    "fsr prepare inputs",
                    &[
                        entry_tex(2, inputs.color),
                        entry_tex(3, inputs.depth),
                        entry_tex(4, inputs.motion),
                        entry_buf(5, &reconstructed),
                    ],
                ),
                farthest_mip1: common(
                    &l_tex3,
                    "fsr farthest depth",
                    &[entry_tex(2, &farthest), entry_tex(3, &farthest), entry_tex(4, &farthest)],
                ),
                shading_change_mip0: common(
                    &l_tex3,
                    "fsr shading change pyramid",
                    &[entry_tex(2, &luma[w]), entry_tex(3, &luma[r]), entry_tex(4, &dilated_motion)],
                ),
                downsample: std::array::from_fn(|i| {
                    let above = &spd_levels[i.min(spd_levels.len() - 1)];
                    common(
                        &l_tex3,
                        "fsr downsample",
                        &[entry_tex(2, above), entry_tex(3, above), entry_tex(4, above)],
                    )
                }),
                shading_change: common(
                    &l_tex3,
                    "fsr shading change",
                    &[entry_tex(2, &spd_all), entry_tex(3, &spd_all), entry_tex(4, &spd_all)],
                ),
                reactivity: common(
                    &l_reactivity,
                    "fsr prepare reactivity",
                    &[
                        entry_tex(2, &dilated_motion),
                        entry_tex(3, &dilated_depth),
                        entry_tex(4, &shading_change),
                        entry_tex(5, &accumulation[r]),
                        entry_tex(6, &luma[w]),
                        entry_buf(7, &reconstructed),
                        entry_buf(8, &new_locks),
                        entry_tex(9, &reactive),
                        entry_tex(10, inputs.motion),
                    ],
                ),
                luma_instability: common(
                    &l_accumulate,
                    "fsr luma instability",
                    &[
                        entry_tex(2, &dilated_motion),
                        entry_tex(3, &masks),
                        entry_tex(4, &luma[w]),
                        entry_tex(5, &luma_history[r]),
                        entry_tex(6, &farthest_mip1),
                        entry_tex(7, &farthest_mip1),
                        entry_buf(8, &new_locks),
                    ],
                ),
                accumulate: common(
                    &l_accumulate,
                    "fsr accumulate",
                    &[
                        entry_tex(2, &dilated_motion),
                        entry_tex(3, &masks),
                        entry_tex(4, inputs.color),
                        entry_tex(5, &internal[r]),
                        entry_tex(6, &farthest_mip1),
                        entry_tex(7, &instability),
                        entry_buf(8, &new_locks),
                    ],
                ),
                rcas: common(
                    &l_accumulate,
                    "fsr rcas",
                    &[
                        entry_tex(2, &dilated_motion),
                        entry_tex(3, &masks),
                        entry_tex(4, inputs.color),
                        entry_tex(5, &internal[w]),
                        entry_tex(6, &farthest_mip1),
                        entry_tex(7, &instability),
                        entry_buf(8, &new_locks),
                    ],
                ),
            }
        });
        Fsr {
            pipelines,
            buffer,
            reconstructed,
            size,
            display,
            display_depth,
            reactive,
            dilated_motion,
            dilated_depth,
            farthest,
            luma,
            farthest_mip1,
            spd_levels,
            shading_change,
            masks,
            accumulation,
            luma_history,
            instability,
            internal,
            output,
            groups,
            parity: 0,
            frame_index: 0.0,
            prev_jitter: Vec2::ZERO,
        }
    }

    /// FSR's output size, the display's.
    pub(crate) fn display_size(&self) -> (u32, u32) {
        self.display
    }

    /// The scene's depth at the display size, for the passes after FSR
    /// (the render pixel under each display pixel); none at native size,
    /// where the scene's own depth is that.
    pub(crate) fn display_depth(&self) -> Option<&wgpu::TextureView> {
        self.display_depth.as_ref()
    }

    /// The upscaled (here anti-aliased) HDR image, the glow amount of the
    /// frame in alpha, as TAA's history: what the post passes read.
    pub(crate) fn output(&self) -> &wgpu::TextureView {
        &self.output
    }

    /// Records FSR's passes for this frame.
    pub(crate) fn encode(&mut self, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, f: &FrameInputs) {
        let w = self.parity;
        if f.reset {
            self.frame_index = 0.0;
            self.prev_jitter = f.jitter;
        }
        let size = [self.size.0 as i32, self.size.1 as i32];
        let display = [self.display.0 as i32, self.display.1 as i32];
        let sharpness = sharpness_override().unwrap_or(f.sharpness);
        let [dx, dy] = device_to_view(f.proj);
        let u = FsrU {
            inv_view_proj: f.view_proj.inverse().to_cols_array_2d(),
            prev_view_proj: f.prev_view_proj.to_cols_array_2d(),
            render_size: size,
            prev_render_size: size,
            upscale_size: display,
            prev_upscale_size: display,
            max_render_size: size,
            max_upscale_size: display,
            device_to_view: [dx, dy, 0.0, 0.0],
            jitter: f.jitter.to_array(),
            prev_jitter: self.prev_jitter.to_array(),
            downscale: [
                self.size.0 as f32 / self.display.0 as f32,
                self.size.1 as f32 / self.display.1 as f32,
            ],
            jitter_sequence_length: jitter_phase_count(self.size.0, self.display.0) as f32,
            delta_pre_exposure: 1.0,
            frame_index: self.frame_index,
            velocity_factor: 1.0,
            accumulation_added_per_frame: 1.0 / 3.0,
            min_disocclusion_accumulation: -1.0 / 3.0,
            exposure: 1.0,
            rcas: rcas_scale(sharpness.unwrap_or(0.0)),
            view_to_meters: 1.0,
            motion_on: if f.motion_on { 1.0 } else { 0.0 },
            reactive_on: if reactive_on() { 1.0 } else { 0.0 },
            composition_on: if composition_on() { 1.0 } else { 0.0 },
            pad: [0.0; 2],
        };
        queue.write_buffer(&self.buffer, 0, bytemuck::bytes_of(&u));
        // nothing pushed yet: 0 is the farthest in reverse-Z
        encoder.clear_buffer(&self.reconstructed, 0, None);
        if f.reset {
            // the last frame's counts and lumas mean nothing
            for v in [&self.accumulation[1 - w], &self.luma_history[1 - w], &self.luma[1 - w]] {
                clear(encoder, v);
            }
        }
        let g = &self.groups[w];
        let p = &self.pipelines;
        run(
            encoder,
            "fsr autogen reactive",
            &[&self.reactive],
            &p.autogen_reactive,
            &g.autogen_reactive,
        );
        run(
            encoder,
            "fsr prepare inputs",
            &[&self.dilated_motion, &self.dilated_depth, &self.farthest, &self.luma[w]],
            &p.prepare,
            &g.prepare,
        );
        run(
            encoder,
            "fsr farthest depth",
            &[&self.farthest_mip1],
            &p.farthest_mip1,
            &g.farthest_mip1,
        );
        run(
            encoder,
            "fsr shading change pyramid",
            &[&self.spd_levels[0]],
            &p.shading_change_mip0,
            &g.shading_change_mip0,
        );
        for level in 1..self.spd_levels.len() {
            run(
                encoder,
                "fsr downsample",
                &[&self.spd_levels[level]],
                &p.downsample,
                &g.downsample[level - 1],
            );
        }
        run(
            encoder,
            "fsr shading change",
            &[&self.shading_change],
            &p.shading_change,
            &g.shading_change,
        );
        run(
            encoder,
            "fsr prepare reactivity",
            &[&self.masks, &self.accumulation[w]],
            &p.reactivity,
            &g.reactivity,
        );
        run(
            encoder,
            "fsr luma instability",
            &[&self.luma_history[w], &self.instability],
            &p.luma_instability,
            &g.luma_instability,
        );
        run(
            encoder,
            "fsr accumulate",
            &[&self.internal[w], &self.output],
            &p.accumulate,
            &g.accumulate,
        );
        if sharpness.is_some() {
            run(encoder, "fsr rcas", &[&self.output], &p.rcas, &g.rcas);
        }
        if let Some(depth) = &self.display_depth {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("fsr display depth"),
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
            pass.set_pipeline(&p.display_depth);
            pass.set_bind_group(0, &g.prepare, &[]);
            pass.draw(0..3, 0..1);
        }
        self.prev_jitter = f.jitter;
        self.frame_index += 1.0;
        self.parity = 1 - w;
    }
}

fn clear(encoder: &mut wgpu::CommandEncoder, v: &wgpu::TextureView) {
    let _ = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("fsr clear"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: v,
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

/// One full-screen pass into `targets`.
fn run(
    encoder: &mut wgpu::CommandEncoder,
    label: &str,
    targets: &[&wgpu::TextureView],
    pipeline: &wgpu::RenderPipeline,
    group: &wgpu::BindGroup,
) {
    let attachments: Vec<Option<wgpu::RenderPassColorAttachment>> = targets
        .iter()
        .map(|v| {
            Some(wgpu::RenderPassColorAttachment {
                view: v,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })
        })
        .collect();
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &attachments,
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, group, &[]);
    pass.draw(0..3, 0..1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jitter_phases_follow_the_sdk() {
        // ffxFsr3UpscalerGetJitterPhaseCount: 8 at native, 18 at 1.5x
        assert_eq!(jitter_phase_count(1920, 1920), 8);
        assert_eq!(jitter_phase_count(1280, 1920), 18);
        assert_eq!(jitter_phase_count(960, 1920), 32);
    }

    #[test]
    fn mip_bias_follows_the_integration_guide() {
        assert_eq!(mip_bias_for(1920, 1920), 0.0);
        assert!((mip_bias_for(1280, 1920) - -1.585).abs() < 1e-3);
        assert!((mip_bias_for(960, 1920) - -2.0).abs() < 1e-6);
        assert!((mip_bias_for(640, 1920) - -2.585).abs() < 1e-3);
    }

    #[test]
    fn rcas_strength_from_the_setting() {
        // FSR's sharpness 1 is RCAS at full strength, 0 two stops below
        assert!((rcas_scale(1.0) - 1.0).abs() < 1e-6);
        assert!((rcas_scale(0.0) - 0.25).abs() < 1e-6);
        assert!((rcas_scale(0.8) - 2f32.powf(-0.4)).abs() < 1e-6);
        assert_eq!(parse_sharpness(None), Some(0.8));
        assert_eq!(parse_sharpness(Some(" off ")), None);
        assert_eq!(parse_sharpness(Some("0.5")), Some(0.5));
        assert_eq!(parse_sharpness(Some("3")), Some(1.0));
    }

    #[test]
    fn view_depth_of_the_infinite_reverse_z() {
        let near = 0.1;
        let proj = glam::camera::rh::proj::directx::perspective_infinite_reverse(1.0, 1.6, near);
        let [x, y] = device_to_view(proj);
        // a point 25 m away: its device depth back to 25 m
        let clip = proj * glam::Vec4::new(0.0, 0.0, -25.0, 1.0);
        let depth = clip.z / clip.w;
        assert!((y / (depth - x) - 25.0).abs() < 1e-3);
        // the sky (depth 0) stays finite
        assert!((y / (0.0 - x)).is_finite());
    }

    #[test]
    fn fsr_shaders_are_valid() {
        let common = include_str!("shaders/fsr/common.wgsl");
        for pass in [
            include_str!("shaders/fsr/prepare_inputs.wgsl"),
            include_str!("shaders/fsr/pyramids.wgsl"),
            include_str!("shaders/fsr/reactivity.wgsl"),
            include_str!("shaders/fsr/accumulate.wgsl"),
        ] {
            let source = format!("{common}\n{pass}");
            let module = naga::front::wgsl::parse_str(&source).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
            let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::empty())
                .validate(&module)
                .unwrap_or_else(|e| panic!("{e:?}"));
            // what the drivers get: Vulkan's SPIR-V and Metal's MSL, one
            // entry point at a time (as wgpu writes them)
            for ep in &module.entry_points {
                let spv = naga::back::spv::PipelineOptions {
                    shader_stage: ep.stage,
                    entry_point: ep.name.clone(),
                };
                naga::back::spv::write_vec(&module, &info, &naga::back::spv::Options::default(), Some(&spv))
                    .unwrap_or_else(|e| panic!("{}: {e:?}", ep.name));
                let msl = naga::back::msl::PipelineOptions {
                    entry_point: Some((ep.stage, ep.name.clone())),
                    ..Default::default()
                };
                naga::back::msl::write_string(
                    &module,
                    &info,
                    &naga::back::msl::Options {
                        lang_version: (2, 4),
                        ..Default::default()
                    },
                    &msl,
                )
                .unwrap_or_else(|e| panic!("{}: {e:?}", ep.name));
            }
        }
    }

    #[test]
    fn params_match_the_shader_layout() {
        // FsrParams in shaders/fsr/common.wgsl: 272 bytes
        assert_eq!(std::mem::size_of::<FsrU>(), 272);
    }
}
