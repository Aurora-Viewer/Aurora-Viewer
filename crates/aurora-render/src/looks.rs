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
//!
//! Looks stack: each layer runs on the image the one before left, then is
//! laid over it at its own intensity, like the effects of a post-processing
//! chain. The comparison line and the photo's coverage come from the scene
//! before the first.
//!
//! Aurora adds operations of its own (`LookOp::Grain` and those after it),
//! for the looks of `looks/aurora/`: film grain, lens effects, colour
//! toning, creative effects, three that read the scene's depth (from the
//! focus distance of the frame: the photo's depth of field, or the scene at
//! the middle of the photo) and a 3D look-up table read from a .cube file
//! (`cube.rs`).

use bytemuck::{Pod, Zeroable};
use std::sync::Arc;

mod cube;
mod parse;
pub use cube::{CubeError, Lut, parse_cube};
pub use parse::{AuroraLook, AuroraSection, LookError, aurora_look, aurora_looks, legacy_look, legacy_looks, parse_look};

/// Operations of one look at most (Miniature has 27).
pub const MAX_OPS: usize = 64;
/// Layers of a frame at most.
pub const MAX_LAYERS: usize = 8;
/// Passes of a frame at most (one per convolution, and the first of each
/// layer).
const MAX_PASSES: usize = 128;
/// Histogram curves of one look at most.
const MAX_CURVES: usize = 8;
/// Histogram curves of a frame at most.
const MAX_SLOTS: usize = 16;
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
    // ---- Aurora's own; sizes are shares of the photo's height
    /// Film grain of `amount` (of the full range) in cells of `size` photo
    /// pixels, the same on the three channels when `mono`.
    Grain {
        amount: f32,
        size: f32,
        mono: bool,
    },
    /// Chromatic aberration: red and blue scaled apart from the centre by
    /// this share.
    Chroma(f32),
    /// Barrel (positive) or pincushion distortion, the corners kept.
    Distort(f32),
    /// Bars that leave a frame `ratio` wide (2.39: the cinema's scope).
    Bars {
        ratio: f32,
        color: [f32; 3],
    },
    /// A border, `side` wide and `bottom` at the bottom (shares of the
    /// shorter side).
    Frame {
        side: f32,
        bottom: f32,
        color: [f32; 3],
    },
    /// One hue kept (degrees, within `width`, fading over `soft`), the rest
    /// in grey.
    Selective {
        hue: f32,
        width: f32,
        soft: f32,
    },
    /// The brightness through three colours: shadows, middle, lights.
    GradientMap([[f32; 3]; 3]),
    /// Split toning: a colour in the shadows and one in the lights.
    Split {
        amount: f32,
        shadows: [f32; 3],
        lights: [f32; 3],
    },
    /// Blocks of this share of the height.
    Pixelate(f32),
    /// Bands of `band` torn by up to `amount`, the channels apart.
    Glitch {
        amount: f32,
        band: f32,
        seed: f32,
    },
    /// Oil paint (Kuwahara) of this radius.
    Paint(f32),
    /// Local contrast over `radius`.
    Clarity {
        amount: f32,
        radius: f32,
    },
    /// The light above `threshold` spread over `radius` and tinted (bloom,
    /// halation).
    Glow {
        amount: f32,
        radius: f32,
        threshold: f32,
        tint: [f32; 3],
    },
    /// Haze from `start` to `end` metres, up to `max`.
    Fog {
        start: f32,
        end: f32,
        max: f32,
        color: [f32; 3],
    },
    /// A blur of up to `radius`, full at `range` times the focus distance
    /// away from it; only behind it when `behind`.
    DepthBlur {
        radius: f32,
        range: f32,
        behind: bool,
    },
    /// What lies further than `tolerance` times the focus distance from it
    /// greyed and darkened: the subject stands out.
    Subject {
        desaturate: f32,
        darken: f32,
        tolerance: f32,
    },
    /// The look's 3D look-up table (`PhotoLook::lut`).
    Lut,
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

    /// The operation's numbers in the order of its fields, colours as
    /// three: what the interface's settings change (`param`, `set_param`).
    fn slots(&mut self) -> Vec<&mut f32> {
        fn three(v: &mut [f32; 3]) -> impl Iterator<Item = &mut f32> {
            v.iter_mut()
        }
        match self {
            LookOp::Stencil { min, max, params, .. } => [min, max].into_iter().chain(params.iter_mut()).collect(),
            LookOp::Saturate(v) | LookOp::Rotate(v) | LookOp::Chroma(v) | LookOp::Distort(v) | LookOp::Pixelate(v) | LookOp::Paint(v) => {
                vec![v]
            }
            LookOp::Gamma { gamma: v, alpha }
            | LookOp::Contrast { slope: v, alpha }
            | LookOp::Brightness { step: v, alpha }
            | LookOp::Linearize { tail: v, alpha }
            | LookOp::Posterize { classes: v, alpha } => std::iter::once(v).chain(three(alpha)).collect(),
            LookOp::Colorize { color, alpha } => three(color).chain(three(alpha)).collect(),
            LookOp::Screen { wavelength, angle, .. } => vec![wavelength, angle],
            LookOp::Convolve { kernel, .. } => kernel.iter_mut().flatten().collect(),
            LookOp::ColorTransform(m) => m.iter_mut().flatten().collect(),
            LookOp::Grain { amount, size, .. } => vec![amount, size],
            LookOp::Bars { ratio, color } => std::iter::once(ratio).chain(three(color)).collect(),
            LookOp::Frame { side, bottom, color } => [side, bottom].into_iter().chain(three(color)).collect(),
            LookOp::Selective { hue, width, soft } => vec![hue, width, soft],
            LookOp::GradientMap(colors) => colors.iter_mut().flatten().collect(),
            LookOp::Split { amount, shadows, lights } => std::iter::once(amount).chain(three(shadows)).chain(three(lights)).collect(),
            LookOp::Glitch { amount, band, seed } => vec![amount, band, seed],
            LookOp::Clarity { amount, radius } => vec![amount, radius],
            LookOp::Glow {
                amount,
                radius,
                threshold,
                tint,
            } => [amount, radius, threshold].into_iter().chain(three(tint)).collect(),
            LookOp::Fog { start, end, max, color } => [start, end, max].into_iter().chain(three(color)).collect(),
            LookOp::DepthBlur { radius, range, .. } => vec![radius, range],
            LookOp::Subject {
                desaturate,
                darken,
                tolerance,
            } => vec![desaturate, darken, tolerance],
            LookOp::Sepia | LookOp::GrayScale | LookOp::Lut => Vec::new(),
        }
    }

    /// The `index`-th number of the operation (`slots`).
    pub fn param(&self, index: usize) -> Option<f32> {
        self.clone().slots().get(index).map(|v| **v)
    }

    /// Changes the `index`-th number of the operation (`slots`).
    pub fn set_param(&mut self, index: usize, value: f32) {
        if let Some(v) = self.slots().into_iter().nth(index) {
            *v = value;
        }
    }

    /// Reads the image around the pixel: the first operation of a pass.
    fn reads_around(&self) -> bool {
        matches!(
            self,
            LookOp::Convolve { .. }
                | LookOp::Chroma(_)
                | LookOp::Distort(_)
                | LookOp::Pixelate(_)
                | LookOp::Glitch { .. }
                | LookOp::Paint(_)
                | LookOp::Clarity { .. }
                | LookOp::Glow { .. }
                | LookOp::DepthBlur { .. }
        )
    }

    /// Reads the focus distance.
    fn focus(&self) -> bool {
        matches!(self, LookOp::DepthBlur { .. } | LookOp::Subject { .. })
    }
}

/// One look of the stack.
#[derive(Debug, Clone, PartialEq)]
pub struct LookLayer {
    pub ops: Arc<[LookOp]>,
    /// Share of the look over the image the layers below left, 0..=1.
    pub intensity: f32,
    /// The table of `LookOp::Lut`.
    pub lut: Option<Arc<Lut>>,
}

/// The looks of the frame (`FrameParams::look`).
#[derive(Debug, Clone, PartialEq)]
pub struct PhotoLook {
    /// The layers, the first one drawn first.
    pub layers: Vec<LookLayer>,
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
    /// Keep the histogram curves and the focus distance of the last frame
    /// (the tiles of a photo drawn in parts, which must share them).
    pub hold: bool,
    /// The focus distance of the depth operations (m); None: the scene's at
    /// the middle of the photo.
    pub focus: Option<f32>,
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
    /// An operation reads the focus distance.
    focus: bool,
}

fn plan(ops: &[LookOp]) -> Plan {
    let ops = &ops[..ops.len().min(MAX_OPS)];
    let mut plan = Plan::default();
    let mut start = 0usize;
    // the pass already changes the image (a convolution must read its
    // source as the pass found it, and so must the histogram)
    let mut dirty = false;
    for (k, op) in ops.iter().enumerate() {
        let around = op.reads_around();
        let first_curve = op.histogram() && plan.histogram_pass.is_none();
        plan.focus |= op.focus();
        if (around || first_curve) && dirty && plan.passes.len() + 1 < MAX_PASSES {
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
            LookOp::Grain { amount, size, mono } => kind(6.0, [amount, size, f32::from(u8::from(mono))]),
            LookOp::Chroma(k) => kind(7.0, [k, 0.0, 0.0]),
            LookOp::Distort(k) => kind(8.0, [k, 0.0, 0.0]),
            LookOp::Bars { ratio, color } => GpuOp {
                b: v4(color),
                ..kind(9.0, [ratio.max(0.1), 0.0, 0.0])
            },
            LookOp::Frame { side, bottom, color } => GpuOp {
                b: v4(color),
                ..kind(10.0, [side, bottom, 0.0])
            },
            LookOp::Selective { hue, width, soft } => kind(11.0, [hue, width, soft.max(0.1)]),
            LookOp::GradientMap(colors) => GpuOp {
                a: [12.0, 0.0, 0.0, 0.0],
                b: v4(colors[0]),
                c: v4(colors[1]),
                d: v4(colors[2]),
            },
            LookOp::Split { amount, shadows, lights } => GpuOp {
                a: [22.0, amount, 0.0, 0.0],
                b: v4(shadows),
                c: v4(lights),
                d: [0.0; 4],
            },
            LookOp::Pixelate(block) => kind(13.0, [block, 0.0, 0.0]),
            LookOp::Glitch { amount, band, seed } => kind(14.0, [amount, band, seed]),
            LookOp::Paint(radius) => kind(15.0, [radius, 0.0, 0.0]),
            LookOp::Clarity { amount, radius } => kind(16.0, [amount, radius, 0.0]),
            LookOp::Glow {
                amount,
                radius,
                threshold,
                tint,
            } => GpuOp {
                b: v4(tint),
                ..kind(17.0, [amount, radius, threshold.clamp(0.0, 0.95)])
            },
            LookOp::Fog { start, end, max, color } => GpuOp {
                b: v4(color),
                ..kind(18.0, [start, end, max])
            },
            LookOp::DepthBlur { radius, range, behind } => kind(19.0, [radius, range, f32::from(u8::from(behind))]),
            LookOp::Subject {
                desaturate,
                darken,
                tolerance,
            } => kind(20.0, [desaturate, darken, tolerance.max(0.01)]),
            LookOp::Lut => kind(21.0, [0.0; 3]),
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

/// An Aurora operation: its kind and three parameters.
fn kind(kind: f32, params: [f32; 3]) -> GpuOp {
    GpuOp {
        a: [kind, params[0], params[1], params[2]],
        ..Default::default()
    }
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
    /// The layer's first operation: its stencil starts there.
    layer_start: u32,
    _pad: f32,
    inv_proj: [[f32; 4]; 4],
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
    inv_proj: [[f32; 4]; 4],
    /// x: the focus distance given (0: none), yz: the point of the frame
    /// whose depth gives it.
    focus: [f32; 4],
}

const SRC_SRGB: u32 = 1;
const FINAL: u32 = 2;
const DST_SRGB: u32 = 4;
const DEPTH_OK: u32 = 8;
/// The layer's source is the scene (its first layer).
const LAYER_SRGB: u32 = 16;
/// The last pass of the last layer: comparison and coverage.
const LAST: u32 = 32;
const SCENE_SRGB: u32 = 64;
const HIST_STRIDE: u64 = 256;
/// The images of the passes, for one size and format: the input the post
/// pass draws into, two images the passes of a layer go back and forth
/// between, and two the layers leave in turn for the next.
struct Targets {
    size: (u32, u32),
    format: wgpu::TextureFormat,
    input: wgpu::TextureView,
    inter: [wgpu::TextureView; 2],
    layers: [wgpu::TextureView; 2],
}

/// The frame's depth and projection, for the depth operations.
pub(crate) struct LookDepth<'a> {
    pub(crate) view: &'a wgpu::TextureView,
    pub(crate) proj: glam::Mat4,
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
    focus_pipeline: wgpu::ComputePipeline,
    /// The look whose histogram curves and focus distance the buffers hold:
    /// a tile keeps them only when they are its own (the frame before may
    /// have tried another filter pointed at in a list).
    counted: parking_lot::Mutex<Option<Vec<Arc<[LookOp]>>>>,
    /// The 3D tables uploaded, the latest last, and their textures.
    tables: parking_lot::Mutex<Vec<(Arc<Lut>, wgpu::TextureView)>>,
    /// Bound when the frame has no depth or no table.
    blank_depth: wgpu::TextureView,
    blank_table: wgpu::TextureView,
    focus: wgpu::Buffer,
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
                depth_entry(6, fragment),
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: fragment,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D3,
                        multisampled: false,
                    },
                    count: None,
                },
                buffer_entry(8, fragment, read, false),
                texture_entry(9, fragment, true),
            ],
        });
        let hist_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("photo look histogram"),
            entries: &[
                texture_entry(16, compute, false),
                buffer_entry(17, compute, write, false),
                buffer_entry(18, compute, wgpu::BufferBindingType::Uniform, true),
                buffer_entry(19, compute, read, false),
                buffer_entry(20, compute, write, false),
                depth_entry(21, compute),
                buffer_entry(22, compute, write, false),
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
        let focus_pipeline = compute_pipeline("cs_focus");
        let blank = |label: &str, format, dimension, usage| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension,
                    format,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | usage,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let blank_depth = blank(
            "photo look blank depth",
            wgpu::TextureFormat::Depth32Float,
            wgpu::TextureDimension::D2,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let blank_table = blank(
            "photo look blank table",
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureDimension::D3,
            wgpu::TextureUsages::COPY_DST,
        );
        let inter_pipeline = render_pipeline(device, &pipeline_layout, &module, INTER);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("photo look"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let storage = wgpu::BufferUsages::STORAGE;
        Looks {
            ops: buffer(
                device,
                "photo look ops",
                (MAX_LAYERS * MAX_OPS * size_of::<GpuOp>()) as u64,
                storage,
            ),
            luts: buffer(device, "photo look curves", (MAX_SLOTS * 256 * 4) as u64, storage),
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
                MAX_LAYERS as u64 * HIST_STRIDE,
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
            focus_pipeline,
            counted: Default::default(),
            tables: Default::default(),
            blank_depth,
            blank_table,
            focus: buffer(device, "photo look focus", 16, storage),
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
        let layers = [make("photo look layer a", INTER), make("photo look layer b", INTER)];
        self.targets = Some(Targets {
            size,
            format,
            input,
            inter,
            layers,
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

    /// Draws the looks from the input into `target`, of the format given to
    /// `prepare`, with the frame's depth for the depth operations. Returns
    /// the passes drawn.
    pub(crate) fn encode(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        look: &PhotoLook,
        target: &wgpu::TextureView,
        depth: Option<LookDepth>,
    ) -> u32 {
        let (Some(t), Some((_, final_pipeline))) = (&self.targets, &self.final_pipeline) else {
            return 0;
        };
        let layers = &look.layers[..look.layers.len().min(MAX_LAYERS)];
        if layers.is_empty() {
            return 0;
        }
        // every layer's operations one after the other, its passes and
        // curves moved to their place in the frame's buffers
        let plans: Vec<Plan> = layers.iter().map(|l| plan(&l.ops)).collect();
        let mut ops = Vec::new();
        let mut op_base = Vec::with_capacity(plans.len());
        let mut slot_base = Vec::with_capacity(plans.len());
        let mut slots = 0usize;
        for plan in &plans {
            op_base.push(ops.len() as u32);
            slot_base.push(slots);
            for op in &plan.ops {
                let mut op = *op;
                if op.a[0] == 3.0 {
                    op.a[1] = ((slots + op.a[1] as usize).min(MAX_SLOTS - 1)) as f32;
                }
                ops.push(op);
            }
            slots += plan.curves.len();
        }
        queue.write_buffer(&self.ops, 0, bytemuck::cast_slice(&ops));
        let srgb = t.format.is_srgb();
        let inv_proj = depth.as_ref().map_or(glam::Mat4::IDENTITY, |d| d.proj.inverse()).to_cols_array_2d();
        let depth_flag = if depth.is_some() { DEPTH_OK } else { 0 };
        let scale = look.scale.max(1e-3);

        // the passes: their layer, place in it, flags
        struct PassAt {
            layer: usize,
            local: usize,
            last_of_layer: bool,
        }
        let mut passes = Vec::new();
        let mut uniforms = Vec::new();
        for (l, plan) in plans.iter().enumerate() {
            let count = plan.passes.len();
            for (j, &(start, end)) in plan.passes.iter().enumerate() {
                if passes.len() >= MAX_PASSES {
                    break;
                }
                let last_of_layer = j + 1 == count;
                let last = last_of_layer && l + 1 == plans.len();
                let mut flags = depth_flag;
                if srgb {
                    flags |= SCENE_SRGB;
                    if l == 0 && j == 0 {
                        flags |= SRC_SRGB;
                    }
                    if l == 0 {
                        flags |= LAYER_SRGB;
                    }
                    if last {
                        flags |= DST_SRGB;
                    }
                }
                if last_of_layer {
                    flags |= FINAL;
                }
                if last {
                    flags |= LAST;
                }
                let u = PassU {
                    image: look.image,
                    scale,
                    start: op_base[l] + start,
                    end: op_base[l] + end,
                    flags,
                    intensity: layers[l].intensity,
                    compare: look.compare.unwrap_or(-1.0),
                    layer_start: op_base[l],
                    _pad: 0.0,
                    inv_proj,
                };
                let mut bytes = [0u8; PASS_STRIDE as usize];
                bytes[..size_of::<PassU>()].copy_from_slice(bytemuck::bytes_of(&u));
                uniforms.extend_from_slice(&bytes);
                passes.push(PassAt {
                    layer: l,
                    local: j,
                    last_of_layer,
                });
            }
        }
        queue.write_buffer(&self.passes, 0, &uniforms);

        // the histogram of each layer that has curves, from its own place
        let key: Vec<Arc<[LookOp]>> = layers.iter().map(|l| l.ops.clone()).collect();
        let held = look.hold && self.counted.lock().as_ref().is_some_and(|counted| *counted == key);
        let find_focus = plans.iter().any(|p| p.focus) && !held;
        let middle = [
            look.image[0] + look.image[2] * scale * 0.5,
            look.image[1] + look.image[3] * scale * 0.5,
        ];
        let mut hist = vec![0u8; layers.len() * HIST_STRIDE as usize];
        for (l, plan) in plans.iter().enumerate() {
            let mut curves = [[0u32; 4]; MAX_CURVES];
            for (k, (c, &[op, kind])) in curves.iter_mut().zip(&plan.curves).enumerate() {
                *c = [op_base[l] + op, kind, (slot_base[l] + k).min(MAX_SLOTS - 1) as u32, 0];
            }
            let first_pass = plan.histogram_pass == Some(0);
            let u = HistU {
                image: look.image,
                scale,
                flags: depth_flag | if l == 0 && first_pass && srgb { SRC_SRGB } else { 0 },
                count: plan.curves.len() as u32,
                _pad: 0,
                curves,
                inv_proj,
                focus: [look.focus.unwrap_or(0.0).max(0.0), middle[0], middle[1], 0.0],
            };
            let at = l * HIST_STRIDE as usize;
            hist[at..at + size_of::<HistU>()].copy_from_slice(bytemuck::bytes_of(&u));
        }
        queue.write_buffer(&self.hist_u, 0, &hist);
        if !held {
            *self.counted.lock() = Some(key);
        }

        let depth_view = depth.as_ref().map_or(&self.blank_depth, |d| d.view);
        let tables: Vec<Option<wgpu::TextureView>> = layers.iter().map(|l| self.table(device, queue, l.lut.as_ref())).collect();
        if find_focus {
            let bg = self.hist_bind_group(device, &t.input, depth_view);
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("photo look focus"),
                timestamp_writes: None,
            });
            pass.set_bind_group(0, &bg, &[0]);
            pass.set_pipeline(&self.focus_pipeline);
            pass.dispatch_workgroups(1, 1, 1);
        }

        let last_layer = layers.len() - 1;
        for (k, at) in passes.iter().enumerate() {
            let l = at.layer;
            let layer_input = if l == 0 { &t.input } else { &t.layers[(l - 1) % 2] };
            let src = if at.local == 0 { layer_input } else { &t.inter[(at.local - 1) % 2] };
            let to_target = at.last_of_layer && l == last_layer;
            let view = if to_target {
                target
            } else if at.last_of_layer {
                &t.layers[l % 2]
            } else {
                &t.inter[at.local % 2]
            };
            if !held && plans[l].histogram_pass == Some(at.local) && !plans[l].curves.is_empty() {
                // the curves from the image as this pass finds it
                encoder.clear_buffer(&self.bins, 0, None);
                let bg = self.hist_bind_group(device, src, depth_view);
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("photo look histogram"),
                    timestamp_writes: None,
                });
                pass.set_bind_group(0, &bg, &[(l as u64 * HIST_STRIDE) as u32]);
                pass.set_pipeline(&self.histogram);
                pass.dispatch_workgroups(t.size.0.div_ceil(16), t.size.1.div_ceil(16), 1);
                pass.set_pipeline(&self.lut);
                pass.dispatch_workgroups(1, 1, 1);
            }
            let table = tables[l].as_ref().unwrap_or(&self.blank_table);
            let bg = self.draw_bind_group(device, src, layer_input, &t.input, depth_view, table);
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
            pass.set_pipeline(if to_target { final_pipeline } else { &self.inter_pipeline });
            pass.set_bind_group(0, &bg, &[(k as u64 * PASS_STRIDE) as u32]);
            pass.draw(0..3, 0..1);
        }
        passes.len() as u32
    }

    /// A 3D table's texture, uploaded the first time it is drawn; the four
    /// latest are kept.
    fn table(&self, device: &wgpu::Device, queue: &wgpu::Queue, lut: Option<&Arc<Lut>>) -> Option<wgpu::TextureView> {
        let lut = lut?;
        let mut tables = self.tables.lock();
        if let Some(i) = tables.iter().position(|(uploaded, _)| Arc::ptr_eq(uploaded, lut)) {
            let entry = tables.remove(i);
            let view = entry.1.clone();
            tables.push(entry);
            return Some(view);
        }
        let n = lut.size;
        let extent = wgpu::Extent3d {
            width: n,
            height: n,
            depth_or_array_layers: n,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("photo look table"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            texture.as_image_copy(),
            &lut.rgba8(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(n * 4),
                rows_per_image: Some(n),
            },
            extent,
        );
        let view = texture.create_view(&Default::default());
        if tables.len() >= 4 {
            tables.remove(0);
        }
        tables.push((lut.clone(), view.clone()));
        Some(view)
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_bind_group(
        &self,
        device: &wgpu::Device,
        src: &wgpu::TextureView,
        original: &wgpu::TextureView,
        scene: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        table: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
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
                    resource: wgpu::BindingResource::TextureView(original),
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
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(depth),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::TextureView(table),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: self.focus.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: wgpu::BindingResource::TextureView(scene),
                },
            ],
        })
    }

    fn hist_bind_group(&self, device: &wgpu::Device, src: &wgpu::TextureView, depth: &wgpu::TextureView) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("photo look histogram"),
            layout: &self.hist_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 16,
                    resource: wgpu::BindingResource::TextureView(src),
                },
                wgpu::BindGroupEntry {
                    binding: 17,
                    resource: self.bins.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 18,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &self.hist_u,
                        offset: 0,
                        size: wgpu::BufferSize::new(size_of::<HistU>() as u64),
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 19,
                    resource: self.ops.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 20,
                    resource: self.luts.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 21,
                    resource: wgpu::BindingResource::TextureView(depth),
                },
                wgpu::BindGroupEntry {
                    binding: 22,
                    resource: self.focus.as_entire_binding(),
                },
            ],
        })
    }
}

fn depth_entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Depth,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

#[cfg(test)]
mod tests;
