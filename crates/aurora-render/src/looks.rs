//! Photo looks: the Legacy Viewer's snapshot filters (the Filter list of
//! its photo window), drawn on the GPU in the frame, so that the live view
//! of the photo mode shows them and the photo is drawn with them. Port of
//! LLImageFilter::executeFilter and its primitives
//! (indra/llimage/llimagefilter.cpp, originally LGPL 2.1).
//!
//! There a filter runs on the CPU over the snapshot's 8-bit image, once
//! taken: a list of operations, each laid on the image through the current
//! procedural stencil. Here the same list runs after the tone mapping and
//! the anti-aliasing, on the same gamma-encoded levels truncated after each
//! operation (`shaders/looks.wgsl`). The operations that only change a
//! pixel run one after the other in one pass; a 3 x 3 convolution reads the
//! pixels around, so it starts a pass of its own, and the histogram curves
//! (filterLinearize, filterEqualize) take the histogram of the image as it
//! is before the first of them, counted and turned into look-up tables by
//! two compute dispatches, without a read back. As there, the histogram is
//! counted once and reused by the later curves of the filter.
//!
//! On the live view the photo is the viewfinder's frame: the operations are
//! worked in the photo's own pixels (`PhotoLook::scale`), so that a blur or
//! a vignette look on screen as on the photo, at its size.

use bytemuck::{Pod, Zeroable};
use std::sync::Arc;

mod parse;
pub use parse::{LookError, legacy_look, legacy_looks, parse_look};

/// Operations of one look at most (Miniature has 27).
pub const MAX_OPS: usize = 64;
/// Passes of one look at most (one per convolution, and the first).
const MAX_PASSES: usize = 48;
/// Histogram curves of one look at most.
const MAX_CURVES: usize = 8;
const PASS_STRIDE: u64 = 256;
const INTER: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// How a stencil's alpha is laid over the image (EStencilShape).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StencilShape {
    Uniform,
    Gradient,
    Vignette,
    ScanLines,
}

/// How an operation's result goes through the stencil (EStencilBlendMode).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StencilMode {
    Blend,
    Add,
    AddBack,
    Fade,
}

/// The pattern of filterScreen (EScreenMode).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenMode {
    Sine2d,
    Line,
}

/// One operation of a look, as the filter files list them. Colours and
/// alphas are per channel; a `Convolve` kernel is in rows, the first one
/// below the pixel (the image's rows go up).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LookOp {
    Stencil {
        shape: StencilShape,
        mode: StencilMode,
        min: f32,
        max: f32,
        params: [f32; 4],
    },
    Sepia,
    GrayScale,
    Saturate(f32),
    Rotate(f32),
    Gamma {
        gamma: f32,
        alpha: [f32; 3],
    },
    Colorize {
        color: [f32; 3],
        alpha: [f32; 3],
    },
    Contrast {
        slope: f32,
        alpha: [f32; 3],
    },
    /// filterBrightness: `darken` is a negative step.
    Brightness {
        step: f32,
        alpha: [f32; 3],
    },
    Linearize {
        tail: f32,
        alpha: [f32; 3],
    },
    /// filterEqualize.
    Posterize {
        classes: f32,
        alpha: [f32; 3],
    },
    Screen {
        mode: ScreenMode,
        wavelength: f32,
        angle: f32,
    },
    Convolve {
        kernel: [[f32; 3]; 3],
        normalize: bool,
        abs: bool,
    },
    /// colortransform: the rows give the output channels.
    ColorTransform([[f32; 3]; 3]),
}

impl LookOp {
    /// "blur": the 3 x 3 average.
    pub fn blur() -> LookOp {
        LookOp::Convolve {
            kernel: [[1.0; 3]; 3],
            normalize: true,
            abs: false,
        }
    }

    pub fn sharpen() -> LookOp {
        LookOp::Convolve {
            kernel: [[-1.0, -1.0, -1.0], [-1.0, 9.0, -1.0], [-1.0, -1.0, -1.0]],
            normalize: false,
            abs: false,
        }
    }

    /// "gradient": the edges, in absolute value.
    pub fn gradient() -> LookOp {
        LookOp::Convolve {
            kernel: [[-1.0, -1.0, -1.0], [-1.0, 8.0, -1.0], [-1.0, -1.0, -1.0]],
            normalize: false,
            abs: true,
        }
    }

    fn histogram(&self) -> bool {
        matches!(self, LookOp::Linearize { .. } | LookOp::Posterize { .. })
    }
}

/// The look of the frame (`FrameParams::look`).
#[derive(Debug, Clone, PartialEq)]
pub struct PhotoLook {
    pub ops: Arc<[LookOp]>,
    /// Share of the look over the image as it is, 0..=1.
    pub intensity: f32,
    /// The image as it is left of this share of the photo's width (the
    /// before / after comparison of the live view); None: the whole look.
    pub compare: Option<f32>,
    /// Where the photo is in the frame drawn: its top left corner in pixels
    /// of the frame (negative for a tile past the first), and its size in
    /// pixels of the photo.
    pub image: [f32; 4],
    /// Pixels of the frame per pixel of the photo (below 1 on the live
    /// view of a larger photo, the supersampling of a photo drawn larger).
    pub scale: f32,
    /// Keep the histogram curves of the last frame (the tiles of a photo
    /// drawn in parts, which must share them).
    pub hold_histogram: bool,
}

/// Frames the images of the passes are kept after the last look (the
/// pointer going from one filter of a list to the next, none in between).
const KEEP_FRAMES: u32 = 120;

/// 3 x 3 matrices as rows (LLMatrix3::mMatrix).
type M3 = [[f32; 3]; 3];

fn mul(a: &M3, b: &M3) -> M3 {
    let mut m = [[0.0; 3]; 3];
    for (j, row) in m.iter_mut().enumerate() {
        for (i, v) in row.iter_mut().enumerate() {
            *v = a[j][0] * b[0][i] + a[j][1] * b[1][i] + a[j][2] * b[2][i];
        }
    }
    m
}

fn transpose(a: &M3) -> M3 {
    let mut m = [[0.0; 3]; 3];
    for (j, row) in m.iter_mut().enumerate() {
        for (i, v) in row.iter_mut().enumerate() {
            *v = a[i][j];
        }
    }
    m
}

/// A transform `t` around the grey axis (filterSaturate, filterRotate:
/// Lij⁻¹ · t · Lij), as the rows of the output channels: LLVector3 *
/// LLMatrix3 multiplies the row vector from the right.
fn around_grey(t: &M3) -> M3 {
    let oo_sqrt2 = std::f32::consts::FRAC_1_SQRT_2;
    let r_a = [[oo_sqrt2, oo_sqrt2, 0.0], [-oo_sqrt2, oo_sqrt2, 0.0], [0.0, 0.0, 1.0]];
    let oo_sqrt3 = 1.0 / 3f32.sqrt();
    let sin_54 = 2f32.sqrt() * oo_sqrt3;
    let r_b = [[oo_sqrt3, 0.0, -sin_54], [0.0, 1.0, 0.0], [sin_54, 0.0, oo_sqrt3]];
    let lij = mul(&r_b, &r_a);
    transpose(&mul(&mul(&transpose(&lij), t), &lij))
}

/// The output rows of a colour operation, None for the others.
fn color_rows(op: &LookOp) -> Option<M3> {
    const LUMINOSITY: [f32; 3] = [0.2125, 0.7154, 0.0721];
    Some(match *op {
        LookOp::GrayScale => [LUMINOSITY; 3],
        LookOp::Sepia => [[0.3588, 0.7044, 0.1368], [0.2990, 0.5870, 0.1140], [0.2392, 0.4696, 0.0912]],
        LookOp::Saturate(s) => around_grey(&[[s, 0.0, 0.0], [0.0, s, 0.0], [0.0, 0.0, 1.0]]),
        LookOp::Rotate(degrees) => {
            let (s, c) = degrees.to_radians().sin_cos();
            around_grey(&[[c, s, 0.0], [-s, c, 0.0], [0.0, 0.0, 1.0]])
        }
        LookOp::ColorTransform(m) => m,
        _ => return None,
    })
}

/// An operation as the shader reads it (looks.wgsl `Op`).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Pod, Zeroable)]
struct GpuOp {
    a: [f32; 4],
    b: [f32; 4],
    c: [f32; 4],
    d: [f32; 4],
}

fn v4(v: [f32; 3]) -> [f32; 4] {
    [v[0], v[1], v[2], 0.0]
}

/// What the frame draws for a look: the operations, the passes (first and
/// end operation of each) and the histogram curves (their operation, and
/// 0 for a linearization, 1 for posterize).
#[derive(Debug, Default, PartialEq)]
struct Plan {
    ops: Vec<GpuOp>,
    passes: Vec<(u32, u32)>,
    /// The pass whose source the histogram is counted on.
    histogram_pass: Option<usize>,
    curves: Vec<[u32; 2]>,
}

fn plan(ops: &[LookOp]) -> Plan {
    let ops = &ops[..ops.len().min(MAX_OPS)];
    let mut plan = Plan::default();
    let mut start = 0usize;
    // the pass already changes the image (a convolution must read its
    // source as the pass found it, and so must the histogram)
    let mut dirty = false;
    for (k, op) in ops.iter().enumerate() {
        let convolve = matches!(op, LookOp::Convolve { .. });
        let first_curve = op.histogram() && plan.histogram_pass.is_none();
        if (convolve || first_curve) && dirty && plan.passes.len() + 1 < MAX_PASSES {
            plan.passes.push((start as u32, k as u32));
            start = k;
            dirty = false;
        }
        if first_curve {
            plan.histogram_pass = Some(plan.passes.len());
        }
        let gpu = match *op {
            LookOp::Stencil {
                shape,
                mode,
                min,
                max,
                params,
            } => {
                let shape = match shape {
                    StencilShape::Uniform => 0.0,
                    StencilShape::Gradient => 1.0,
                    StencilShape::Vignette => 2.0,
                    StencilShape::ScanLines => 3.0,
                };
                let mode = match mode {
                    StencilMode::Blend => 0.0,
                    StencilMode::Add => 1.0,
                    StencilMode::AddBack => 2.0,
                    StencilMode::Fade => 3.0,
                };
                GpuOp {
                    a: [0.0, shape, mode, min.clamp(-1.0, 1.0)],
                    b: [max.clamp(-1.0, 1.0), 0.0, 0.0, 0.0],
                    c: params,
                    d: [0.0; 4],
                }
            }
            LookOp::Gamma { gamma, alpha } => curve(0.0, gamma, [0.0; 3], alpha),
            LookOp::Colorize { color, alpha } => curve(1.0, 0.0, color, alpha),
            LookOp::Contrast { slope, alpha } => curve(2.0, slope, [0.0; 3], alpha),
            LookOp::Brightness { step, alpha } => curve(3.0, step, [0.0; 3], alpha),
            LookOp::Linearize { tail, alpha } | LookOp::Posterize { classes: tail, alpha } => {
                let lut = plan.curves.len().min(MAX_CURVES - 1);
                if plan.curves.len() < MAX_CURVES {
                    plan.curves.push([k as u32, u32::from(matches!(op, LookOp::Posterize { .. }))]);
                }
                GpuOp {
                    a: [3.0, lut as f32, tail, 0.0],
                    b: v4(alpha),
                    ..Default::default()
                }
            }
            LookOp::Screen { mode, wavelength, angle } => GpuOp {
                a: [4.0, if mode == ScreenMode::Sine2d { 0.0 } else { 1.0 }, wavelength, angle],
                ..Default::default()
            },
            LookOp::Convolve { kernel, normalize, abs } => {
                let (mut lo, mut hi) = (0.0f32, 0.0f32);
                for &v in kernel.iter().flatten() {
                    if v >= 0.0 {
                        hi += v;
                    } else {
                        lo += v;
                    }
                }
                if abs {
                    hi = hi.abs().max(lo.abs());
                    lo = 0.0;
                }
                let k = kernel.as_flattened();
                GpuOp {
                    a: [5.0, f32::from(u8::from(normalize)), f32::from(u8::from(abs)), lo],
                    b: [k[0], k[1], k[2], k[3]],
                    c: [k[4], k[5], k[6], k[7]],
                    d: [k[8], hi - lo, 0.0, 0.0],
                }
            }
            _ => {
                let rows = color_rows(op).unwrap_or([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);
                GpuOp {
                    a: [1.0, 0.0, 0.0, 0.0],
                    b: v4(rows[0]),
                    c: v4(rows[1]),
                    d: v4(rows[2]),
                }
            }
        };
        plan.ops.push(gpu);
        dirty |= !matches!(op, LookOp::Stencil { .. });
    }
    plan.passes.push((start as u32, ops.len() as u32));
    plan
}

fn curve(kind: f32, param: f32, color: [f32; 3], alpha: [f32; 3]) -> GpuOp {
    GpuOp {
        a: [2.0, kind, param, 0.0],
        b: v4(color),
        c: v4(alpha),
        d: [0.0; 4],
    }
}

/// One pass (looks.wgsl `Pass`).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PassU {
    image: [f32; 4],
    scale: f32,
    start: u32,
    end: u32,
    flags: u32,
    intensity: f32,
    compare: f32,
    _pad: [f32; 2],
}

/// The histogram's dispatches (looks.wgsl `HistU`).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct HistU {
    image: [f32; 4],
    scale: f32,
    flags: u32,
    count: u32,
    _pad: u32,
    curves: [[u32; 4]; MAX_CURVES],
}

const SRC_SRGB: u32 = 1;
const FINAL: u32 = 2;
const DST_SRGB: u32 = 4;

/// The images of the passes, for one size and format: the input the post
/// pass draws into, two images the passes go back and forth between, and
/// the bind groups that read each of the three.
struct Targets {
    size: (u32, u32),
    format: wgpu::TextureFormat,
    input: wgpu::TextureView,
    inter: [wgpu::TextureView; 2],
    draw_bgs: [wgpu::BindGroup; 3],
    hist_bgs: [wgpu::BindGroup; 3],
}

pub(crate) struct Looks {
    layout: wgpu::BindGroupLayout,
    hist_layout: wgpu::BindGroupLayout,
    module: wgpu::ShaderModule,
    pipeline_layout: wgpu::PipelineLayout,
    inter_pipeline: wgpu::RenderPipeline,
    /// The last pass's, for the format of the frame.
    final_pipeline: Option<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
    histogram: wgpu::ComputePipeline,
    lut: wgpu::ComputePipeline,
    /// The look whose histogram curves the buffers hold: a tile keeps them
    /// only when they are its own (the frame before may have tried another
    /// filter pointed at in a list).
    counted: parking_lot::Mutex<Option<Arc<[LookOp]>>>,
    /// Frames without a look.
    idle: u32,
    sampler: wgpu::Sampler,
    ops: wgpu::Buffer,
    luts: wgpu::Buffer,
    bins: wgpu::Buffer,
    passes: wgpu::Buffer,
    hist_u: wgpu::Buffer,
    targets: Option<Targets>,
}

fn texture_entry(binding: u32, visibility: wgpu::ShaderStages, filterable: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn buffer_entry(binding: u32, visibility: wgpu::ShaderStages, ty: wgpu::BufferBindingType, dynamic: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty,
            has_dynamic_offset: dynamic,
            min_binding_size: None,
        },
        count: None,
    }
}

fn buffer(device: &wgpu::Device, label: &str, size: u64, usage: wgpu::BufferUsages) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage: usage | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn render_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    module: &wgpu::ShaderModule,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("photo look"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("vs_look"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some("fs_look"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

impl Looks {
    pub(crate) fn new(device: &wgpu::Device) -> Looks {
        let fragment = wgpu::ShaderStages::FRAGMENT;
        let compute = wgpu::ShaderStages::COMPUTE;
        let read = wgpu::BufferBindingType::Storage { read_only: true };
        let write = wgpu::BufferBindingType::Storage { read_only: false };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("photo look"),
            entries: &[
                texture_entry(0, fragment, true),
                texture_entry(1, fragment, true),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: fragment,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                buffer_entry(3, fragment, read, false),
                buffer_entry(4, fragment, read, false),
                buffer_entry(5, fragment, wgpu::BufferBindingType::Uniform, true),
            ],
        });
        let hist_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("photo look histogram"),
            entries: &[
                texture_entry(8, compute, false),
                buffer_entry(9, compute, write, false),
                buffer_entry(10, compute, wgpu::BufferBindingType::Uniform, false),
                buffer_entry(11, compute, read, false),
                buffer_entry(12, compute, write, false),
            ],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("photo look"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/looks.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("photo look"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let hist_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("photo look histogram"),
            bind_group_layouts: &[Some(&hist_layout)],
            immediate_size: 0,
        });
        let compute_pipeline = |entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("photo look histogram"),
                layout: Some(&hist_pipeline_layout),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let histogram = compute_pipeline("cs_histogram");
        let lut = compute_pipeline("cs_lut");
        let inter_pipeline = render_pipeline(device, &pipeline_layout, &module, INTER);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("photo look"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let storage = wgpu::BufferUsages::STORAGE;
        Looks {
            ops: buffer(device, "photo look ops", (MAX_OPS * size_of::<GpuOp>()) as u64, storage),
            luts: buffer(device, "photo look curves", (MAX_CURVES * 256 * 4) as u64, storage),
            bins: buffer(device, "photo look histogram", 256 * 4, storage),
            passes: buffer(
                device,
                "photo look passes",
                MAX_PASSES as u64 * PASS_STRIDE,
                wgpu::BufferUsages::UNIFORM,
            ),
            hist_u: buffer(
                device,
                "photo look histogram params",
                size_of::<HistU>() as u64,
                wgpu::BufferUsages::UNIFORM,
            ),
            layout,
            hist_layout,
            module,
            pipeline_layout,
            inter_pipeline,
            final_pipeline: None,
            histogram,
            lut,
            counted: Default::default(),
            idle: 0,
            sampler,
            targets: None,
        }
    }

    /// The image the post pass draws into while a look is on, made for
    /// the frame's size and format.
    pub(crate) fn prepare(&mut self, device: &wgpu::Device, format: wgpu::TextureFormat, size: (u32, u32)) {
        let size = (size.0.max(1), size.1.max(1));
        self.idle = 0;
        if self.final_pipeline.as_ref().is_none_or(|(f, _)| *f != format) {
            self.final_pipeline = Some((format, render_pipeline(device, &self.pipeline_layout, &self.module, format)));
        }
        if self.targets.as_ref().is_some_and(|t| t.size == size && t.format == format) {
            return;
        }
        // COPY_DST: the tests write the input
        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST;
        let make = |label: &str, format: wgpu::TextureFormat| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: size.0,
                        height: size.1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let input = make("photo look input", format);
        let inter = [make("photo look a", INTER), make("photo look b", INTER)];
        let draw_bg = |src: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("photo look"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(src),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&input),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: self.ops.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: self.luts.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &self.passes,
                            offset: 0,
                            size: wgpu::BufferSize::new(size_of::<PassU>() as u64),
                        }),
                    },
                ],
            })
        };
        let hist_bg = |src: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("photo look histogram"),
                layout: &self.hist_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 8,
                        resource: wgpu::BindingResource::TextureView(src),
                    },
                    wgpu::BindGroupEntry {
                        binding: 9,
                        resource: self.bins.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 10,
                        resource: self.hist_u.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 11,
                        resource: self.ops.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 12,
                        resource: self.luts.as_entire_binding(),
                    },
                ],
            })
        };
        let sources = [&input, &inter[0], &inter[1]];
        let draw_bgs = sources.map(draw_bg);
        let hist_bgs = sources.map(hist_bg);
        self.targets = Some(Targets {
            size,
            format,
            input,
            inter,
            draw_bgs,
            hist_bgs,
        });
    }

    /// A frame without a look: the images are let go after a while.
    pub(crate) fn idle(&mut self) {
        self.idle = self.idle.saturating_add(1);
        if self.idle > KEEP_FRAMES {
            self.targets = None;
        }
    }

    /// The image the post pass draws into (after `prepare`).
    pub(crate) fn input(&self) -> Option<&wgpu::TextureView> {
        self.targets.as_ref().map(|t| &t.input)
    }

    /// Draws the look from the input into `target`, of the format given to
    /// `prepare`. Returns the passes drawn.
    pub(crate) fn encode(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        look: &PhotoLook,
        target: &wgpu::TextureView,
    ) -> u32 {
        let (Some(t), Some((_, final_pipeline))) = (&self.targets, &self.final_pipeline) else {
            return 0;
        };
        let plan = plan(&look.ops);
        if !plan.ops.is_empty() {
            queue.write_buffer(&self.ops, 0, bytemuck::cast_slice(&plan.ops));
        }
        let srgb = t.format.is_srgb();
        let count = plan.passes.len();
        let mut uniforms = vec![0u8; count * PASS_STRIDE as usize];
        for (k, &(start, end)) in plan.passes.iter().enumerate() {
            let last = k + 1 == count;
            let mut flags = 0;
            if k == 0 && srgb {
                flags |= SRC_SRGB;
            }
            if last {
                flags |= FINAL;
                if srgb {
                    flags |= DST_SRGB;
                }
            }
            let u = PassU {
                image: look.image,
                scale: look.scale.max(1e-3),
                start,
                end,
                flags,
                intensity: look.intensity,
                compare: look.compare.unwrap_or(-1.0),
                _pad: [0.0; 2],
            };
            let at = k * PASS_STRIDE as usize;
            uniforms[at..at + size_of::<PassU>()].copy_from_slice(bytemuck::bytes_of(&u));
        }
        queue.write_buffer(&self.passes, 0, &uniforms);

        // which image each pass reads: 0 the input, then the two in turn
        let source = |k: usize| if k == 0 { 0 } else { 1 + (k - 1) % 2 };
        let held = look.hold_histogram && self.counted.lock().as_deref().is_some_and(|ops| *ops == *look.ops);
        if let Some(h) = plan.histogram_pass.filter(|_| !held && !plan.curves.is_empty()) {
            let mut curves = [[0u32; 4]; MAX_CURVES];
            for (c, &[op, kind]) in curves.iter_mut().zip(&plan.curves) {
                *c = [op, kind, 0, 0];
            }
            let u = HistU {
                image: look.image,
                scale: look.scale.max(1e-3),
                flags: if h == 0 && srgb { SRC_SRGB } else { 0 },
                count: plan.curves.len() as u32,
                _pad: 0,
                curves,
            };
            *self.counted.lock() = Some(look.ops.clone());
            queue.write_buffer(&self.hist_u, 0, bytemuck::bytes_of(&u));
            // the passes before the one counted, then the counting
            self.draw_passes(encoder, t, final_pipeline, &plan, 0..h, target);
            encoder.clear_buffer(&self.bins, 0, None);
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("photo look histogram"),
                timestamp_writes: None,
            });
            pass.set_bind_group(0, &t.hist_bgs[source(h)], &[]);
            pass.set_pipeline(&self.histogram);
            pass.dispatch_workgroups(t.size.0.div_ceil(16), t.size.1.div_ceil(16), 1);
            pass.set_pipeline(&self.lut);
            pass.dispatch_workgroups(1, 1, 1);
            drop(pass);
            self.draw_passes(encoder, t, final_pipeline, &plan, h..count, target);
        } else {
            self.draw_passes(encoder, t, final_pipeline, &plan, 0..count, target);
        }
        count as u32
    }

    fn draw_passes(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        t: &Targets,
        final_pipeline: &wgpu::RenderPipeline,
        plan: &Plan,
        range: std::ops::Range<usize>,
        target: &wgpu::TextureView,
    ) {
        let count = plan.passes.len();
        for k in range {
            let last = k + 1 == count;
            let view = if last { target } else { &t.inter[k % 2] };
            let src = if k == 0 { 0 } else { 1 + (k - 1) % 2 };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("photo look"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
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
            pass.set_pipeline(if last { final_pipeline } else { &self.inter_pipeline });
            pass.set_bind_group(0, &t.draw_bgs[src], &[(k as u64 * PASS_STRIDE) as u32]);
            pass.draw(0..3, 0..1);
        }
    }
}

#[cfg(test)]
mod tests;
