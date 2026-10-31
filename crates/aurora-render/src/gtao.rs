//! Ground-truth ambient occlusion (GTAO), the option beside the Legacy
//! Viewer's screen-space occlusion (shaders/ssao.wgsl): Intel's XeGTAO
//! (MIT licence, see NOTICE.md) in compute steps from the depth prepass —
//! depth levels, the occlusion with the depth edges, its accumulation over
//! the frames, then XeGTAO's edge-aware blur (shaders/gtao.wgsl). Its result goes where the
//! Legacy occlusion goes (`screen_ao` in common.wgsl), so the method only
//! changes the occlusion term.
//!
//! `AURORA_AO=legacy|gtao|alt` overrides the method of the settings for a
//! comparison; `alt` switches every second, both methods kept ready;
//! `gtao:<0-3>` and `alt:<0-3>` also choose GTAO's quality.
//!
//! The result also carries XeGTAO's bent normal, from which the shading
//! takes a specular occlusion (`specular_occlusion` in common.wgsl): the
//! reflections fade in the creases and recesses GTAO finds. It comes with
//! GTAO; `AURORA_GTAO_SPECULAR=0|1|alt` turns it off, on (the default) or
//! on every other second, to compare.
//!
//! XeGTAO is meant to run under TAA, which spreads its noise over frames;
//! Aurora's default anti-aliasing is SMAA, so the occlusion gathers its own
//! frames (`temporal` in gtao.wgsl): the noise moves every frame and each
//! pixel blends with its reprojected history, which takes the fine grain
//! out of the penumbra that the 3 x 3 blur alone left. The history lives
//! with the targets, so new ones (a resize, a quality change, a photo
//! frame or tile) start again from the current frame, as does a frame
//! after one without GTAO or without the previous camera.
//! `AURORA_GTAO_TEMPORAL=0|1|alt` turns it off (the noise fixed again, as
//! before), on (the default) or on every other second.
//!
//! With the screen-space indirect light of Preferences on, `main_bitmask`
//! runs after the main pass: one bounce of the previous frame's lit colour
//! (`scene_copy`, reprojected) gathered on a visibility bitmask with a
//! thickness per sample (Therrien et al. 2023), blurred like the occlusion
//! into `Targets::indirect`, which the shading adds to the diffuse light
//! (`screen_indirect` in common.wgsl). The occlusion stays the main pass's:
//! the same image with the light on or off. `AURORA_SSIL=0|1|alt`
//! overrides the setting (`alt`: on every other second).

use crate::types::AoMethod;
use bytemuck::{Pod, Zeroable};
use glam::Mat4;

/// XE_GTAO_DEPTH_MIP_LEVELS.
const MIP_LEVELS: u32 = 5;
/// XeGTAO's default constants (XeGTAO.h, GTAOSettings): effect radius 0.5
/// times the radius multiplier 1.457 (metres here), falloff range 0.615,
/// sample distribution power 2, thin occluder compensation 0, final value
/// power 2.2, depth MIP sampling offset 3.30, denoise blur beta 1.2.
const EFFECT_RADIUS: f32 = 0.5 * 1.457;
const FALLOFF_RANGE: f32 = 0.615;
const SAMPLE_DISTRIBUTION_POWER: f32 = 2.0;
const THIN_OCCLUDER_COMPENSATION: f32 = 0.0;
const FINAL_VALUE_POWER: f32 = 2.2;
const DEPTH_MIP_SAMPLING_OFFSET: f32 = 3.30;
const DENOISE_BLUR_BETA: f32 = 1.2;
/// The indirect light's samples: how thick a surface is taken to be behind
/// its front (metres) and how far the light comes from (metres). Its
/// strength is a setting.
const INDIRECT_THICKNESS: f32 = 0.35;
const INDIRECT_RADIUS: f32 = 1.5;
/// Blur passes of the bent normal alone, before the occlusion's last one.
const BENT_BLUR: u32 = 3;
/// Blur passes of the indirect light beyond the occlusion's.
const INDIRECT_EXTRA_BLUR: u32 = 4;
/// The accumulation: the current frame's least weight (the history then
/// holds about thirty frames), the same where the point moves on screen by
/// a pixel a frame or more (about ten frames: the long history lagged
/// behind the camera, and the motion hides the grain it quiets), and the
/// half width of the neighbourhood the history is clipped to, in standard
/// deviations.
const TEMPORAL_MIN_WEIGHT: f32 = 0.03;
const TEMPORAL_MOVING_WEIGHT: f32 = 0.1;
const TEMPORAL_CLIP: f32 = 1.25;
/// Times more slices on a frame that accumulates without a history.
const NO_HISTORY_SLICES: u32 = 4;
/// Frames the noise goes through before it repeats (XeGTAO's 64).
const NOISE_FRAMES: u64 = 64;

/// How much work one quality level does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Quality {
    /// Slices of the hemisphere per pixel.
    pub slices: u32,
    /// Horizon samples on each side of a slice.
    pub steps: u32,
    /// Computed at half the window's resolution.
    pub half_res: bool,
    /// Blur passes (at least one: the last one writes the result).
    pub denoise_passes: u32,
}

/// The four levels of Preferences (0 low … 3 ultra). XeGTAO's own presets
/// are slices 1 / 2 / 3 / 9 with 2 / 2 / 3 / 3 steps; low takes two slices
/// at half resolution and the levels blur more where they sample less, as
/// they did before the accumulation: a frame without history (a photo)
/// looks as it did.
pub(crate) fn quality(level: u8) -> Quality {
    match level {
        0 => Quality {
            slices: 2,
            steps: 2,
            half_res: true,
            denoise_passes: 2,
        },
        1 => Quality {
            slices: 2,
            steps: 2,
            half_res: false,
            denoise_passes: 2,
        },
        2 => Quality {
            slices: 3,
            steps: 3,
            half_res: false,
            denoise_passes: 2,
        },
        _ => Quality {
            slices: 9,
            steps: 3,
            half_res: false,
            denoise_passes: 1,
        },
    }
}

/// `AURORA_AO`: the method forced for a comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Override {
    None,
    Legacy,
    /// With its quality, else the settings' (high under the Legacy method).
    Gtao(Option<u8>),
    /// Legacy and GTAO by turns, one second each.
    Alternate(Option<u8>),
}

impl Override {
    pub(crate) fn parse(v: Option<&str>) -> Self {
        let Some(v) = v.map(|v| v.trim().to_ascii_lowercase()) else {
            return Override::None;
        };
        let (mode, level) = match v.split_once(':') {
            Some((m, l)) => (m.to_owned(), l.trim().parse::<u8>().ok().map(|l| l.min(3))),
            None => (v, None),
        };
        match mode.as_str() {
            "legacy" => Override::Legacy,
            "gtao" => Override::Gtao(level),
            "alt" => Override::Alternate(level),
            _ => Override::None,
        }
    }

    pub(crate) fn from_env() -> Self {
        static MODE: std::sync::OnceLock<Override> = std::sync::OnceLock::new();
        *MODE.get_or_init(|| Override::parse(std::env::var("AURORA_AO").ok().as_deref()))
    }

    /// The methods to keep ready for `method` of the settings: (Legacy, GTAO).
    pub(crate) fn needed(self, method: AoMethod) -> (bool, bool) {
        match self {
            Override::Legacy => (true, false),
            Override::Gtao(_) => (false, true),
            Override::Alternate(_) => (true, true),
            Override::None => match method {
                AoMethod::Legacy => (true, false),
                AoMethod::Gtao { .. } => (false, true),
            },
        }
    }

    /// Whether GTAO draws the frame `secs` seconds into the session.
    pub(crate) fn gtao_now(self, method: AoMethod, secs: u64) -> bool {
        match self {
            Override::Alternate(_) => secs % 2 == 1,
            _ => self.needed(method).1,
        }
    }

    /// GTAO's quality for `method`: the one forced, the settings', or high
    /// when GTAO is forced over the Legacy method.
    pub(crate) fn level(self, method: AoMethod) -> u8 {
        match (self, method) {
            (Override::Gtao(Some(q)) | Override::Alternate(Some(q)), _) => q.min(3),
            (_, AoMethod::Gtao { quality }) => quality.min(3),
            (_, AoMethod::Legacy) => 2,
        }
    }
}

/// `AURORA_GTAO_SPECULAR`: the specular occlusion from GTAO's bent normal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SpecularOcclusion {
    On,
    Off,
    /// On one second, off the next.
    Alternate,
}

impl SpecularOcclusion {
    pub(crate) fn parse(v: Option<&str>) -> Self {
        match v.map(str::trim) {
            Some("0") | Some("off") => SpecularOcclusion::Off,
            Some("alt") => SpecularOcclusion::Alternate,
            _ => SpecularOcclusion::On,
        }
    }

    pub(crate) fn from_env() -> Self {
        static MODE: std::sync::OnceLock<SpecularOcclusion> = std::sync::OnceLock::new();
        *MODE.get_or_init(|| SpecularOcclusion::parse(std::env::var("AURORA_GTAO_SPECULAR").ok().as_deref()))
    }

    /// Whether the frame drawn `secs` whole seconds into the session takes
    /// it (with GTAO shown).
    pub(crate) fn applied(self, secs: u64) -> bool {
        match self {
            SpecularOcclusion::On => true,
            SpecularOcclusion::Off => false,
            SpecularOcclusion::Alternate => secs.is_multiple_of(2),
        }
    }
}

/// `AURORA_GTAO_TEMPORAL`: the occlusion's accumulation over the frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Temporal {
    On,
    Off,
    /// On one second, off the next.
    Alternate,
}

impl Temporal {
    pub(crate) fn parse(v: Option<&str>) -> Self {
        match v.map(str::trim) {
            Some("0") | Some("off") => Temporal::Off,
            Some("alt") => Temporal::Alternate,
            _ => Temporal::On,
        }
    }

    pub(crate) fn from_env() -> Self {
        static MODE: std::sync::OnceLock<Temporal> = std::sync::OnceLock::new();
        *MODE.get_or_init(|| Temporal::parse(std::env::var("AURORA_GTAO_TEMPORAL").ok().as_deref()))
    }

    /// Whether the frame drawn `secs` whole seconds into the session
    /// accumulates.
    pub(crate) fn applied(self, secs: u64) -> bool {
        match self {
            Temporal::On => true,
            Temporal::Off => false,
            Temporal::Alternate => secs.is_multiple_of(2),
        }
    }
}

/// `AURORA_GTAO_NORMALS`: the surface normal the occlusion starts from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Normals {
    /// The mesh's smooth normal, which the depth prepass draws beside the
    /// depth (the default).
    Mesh,
    /// XeGTAO's from the depth, as before: the flat triangle's, which shows
    /// the facets of low-poly meshes (#993).
    Depth,
    /// The mesh's one second, the depth's the next.
    Alternate,
}

impl Normals {
    pub(crate) fn parse(v: Option<&str>) -> Self {
        match v.map(str::trim) {
            Some("depth") | Some("0") => Normals::Depth,
            Some("alt") => Normals::Alternate,
            _ => Normals::Mesh,
        }
    }

    pub(crate) fn from_env() -> Self {
        static MODE: std::sync::OnceLock<Normals> = std::sync::OnceLock::new();
        *MODE.get_or_init(|| Normals::parse(std::env::var("AURORA_GTAO_NORMALS").ok().as_deref()))
    }

    /// Whether the frame drawn `secs` whole seconds into the session reads
    /// the mesh normals.
    pub(crate) fn mesh(self, secs: u64) -> bool {
        match self {
            Normals::Mesh => true,
            Normals::Depth => false,
            Normals::Alternate => secs.is_multiple_of(2),
        }
    }
}

/// What a frame's occlusion knows of its frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Frame {
    /// The renderer's frame counter.
    pub index: u64,
    /// It accumulates (`Temporal::applied`).
    pub temporal: bool,
    /// The previous frame's camera is known (else the reprojection is the
    /// current one's and the history is not read).
    pub previous_camera: bool,
}

/// The accumulation's state between frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct History {
    /// The frame that last wrote the history, and into which of the two.
    written: Option<(u64, usize)>,
}

impl History {
    /// This frame's history target and whether the other one holds the
    /// previous frame's (None: the frame does not accumulate).
    fn next(&mut self, f: Frame) -> Option<(usize, bool)> {
        if !f.temporal {
            // the blur uses the history targets: nothing is left to read
            self.written = None;
            return None;
        }
        let (target, valid) = match self.written {
            Some((frame, last)) if frame.wrapping_add(1) == f.index => (1 - last, f.previous_camera),
            _ => (0, false),
        };
        self.written = Some((f.index, target));
        Some((target, valid))
    }
}

/// `AURORA_SSIL`: the screen-space indirect light forced on or off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Indirect {
    /// As the settings say.
    Settings,
    On,
    Off,
    /// On one second, off the next (its targets kept ready).
    Alternate,
}

/// `AURORA_SSIL_INTENSITY`: the indirect light's strength in place of the
/// setting's (0 to 8).
pub(crate) fn intensity_override() -> Option<f32> {
    static VALUE: std::sync::OnceLock<Option<f32>> = std::sync::OnceLock::new();
    *VALUE.get_or_init(|| parse_intensity(std::env::var("AURORA_SSIL_INTENSITY").ok().as_deref()))
}

fn parse_intensity(v: Option<&str>) -> Option<f32> {
    v.and_then(|v| v.trim().parse::<f32>().ok())
        .filter(|v| v.is_finite())
        .map(|v| v.clamp(0.0, 8.0))
}

impl Indirect {
    pub(crate) fn parse(v: Option<&str>) -> Self {
        match v.map(str::trim) {
            Some("1") | Some("on") => Indirect::On,
            Some("0") | Some("off") => Indirect::Off,
            Some("alt") => Indirect::Alternate,
            _ => Indirect::Settings,
        }
    }

    pub(crate) fn from_env() -> Self {
        static MODE: std::sync::OnceLock<Indirect> = std::sync::OnceLock::new();
        *MODE.get_or_init(|| Indirect::parse(std::env::var("AURORA_SSIL").ok().as_deref()))
    }

    /// Whether its targets are needed with the setting `on`.
    pub(crate) fn needed(self, on: bool) -> bool {
        match self {
            Indirect::Settings => on,
            Indirect::On | Indirect::Alternate => true,
            Indirect::Off => false,
        }
    }

    /// Whether the frame drawn `secs` whole seconds into the session has it.
    pub(crate) fn applied(self, on: bool, secs: u64) -> bool {
        match self {
            Indirect::Alternate => secs.is_multiple_of(2),
            _ => self.needed(on),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GtaoU {
    proj: [f32; 4],
    depth: [f32; 4],
    radius: [f32; 4],
    quality: [f32; 4],
    misc: [f32; 4],
    reproj: [[f32; 4]; 4],
    il: [f32; 4],
    temporal: [f32; 4],
    /// A mat3x3 (columns padded to four).
    normal_view: [[f32; 4]; 3],
}

/// The constants of a frame drawn with `proj` into targets of `size`;
/// `reproj` takes its view space (z in front) to the clip space of the
/// previous frame; `intensity` scales the indirect light; `temporal` is the
/// accumulation's frame, if it accumulates, and whether a history is there;
/// `normals` the frame's view when it reads the mesh normals.
fn uniforms(
    proj: Mat4,
    reproj: Mat4,
    intensity: f32,
    size: (u32, u32),
    q: Quality,
    temporal: Option<(u64, bool)>,
    normals: Option<Mat4>,
) -> GtaoU {
    let (noise, history) = match temporal {
        Some((index, history)) => ((index % NOISE_FRAMES) as f32, history),
        None => (0.0, false),
    };
    // a frame that accumulates without a history (a photo, its tiles, new
    // targets) would show the grain the history takes out: it samples more
    let slices = if temporal.is_some() && !history { NO_HISTORY_SLICES } else { 1 };
    GtaoU {
        // scales and off-centre terms (a photo tile's projection has them)
        proj: [proj.x_axis.x, proj.y_axis.y, proj.z_axis.x, proj.z_axis.y],
        // reversed infinite depth: the buffer holds P32 / (view z) - P22
        depth: [proj.w_axis.z, proj.z_axis.z, size.0 as f32, size.1 as f32],
        radius: [EFFECT_RADIUS, FALLOFF_RANGE, SAMPLE_DISTRIBUTION_POWER, THIN_OCCLUDER_COMPENSATION],
        quality: [FINAL_VALUE_POWER, DEPTH_MIP_SAMPLING_OFFSET, q.slices as f32, q.steps as f32],
        misc: [
            DENOISE_BLUR_BETA,
            if q.half_res { 2.0 } else { 1.0 },
            (q.slices * slices) as f32,
            if normals.is_some() { 1.0 } else { 0.0 },
        ],
        reproj: reproj.to_cols_array_2d(),
        il: [INDIRECT_THICKNESS, INDIRECT_RADIUS, intensity, TEMPORAL_MOVING_WEIGHT],
        temporal: [noise, if history { 0.0 } else { 1.0 }, TEMPORAL_MIN_WEIGHT, TEMPORAL_CLIP],
        normal_view: normals.map_or([[0.0; 4]; 3], |view| {
            let m = normal_view(view);
            [m.x_axis.extend(0.0), m.y_axis.extend(0.0), m.z_axis.extend(0.0)].map(|c| c.to_array())
        }),
    }
}

/// The rotation from world directions to the view space of the GTAO pass
/// for a frame seen through `view` (its z flipped, as in [`reprojection`]).
pub(crate) fn normal_view(view: Mat4) -> glam::Mat3 {
    glam::Mat3::from_diagonal(glam::Vec3::new(1.0, 1.0, -1.0)) * glam::Mat3::from_mat4(view)
}

/// The matrix from the view space of the GTAO pass (x right, y up, z the
/// distance in front: glam's view space with z flipped) of a frame seen
/// through `view` to the clip space of the previous frame (`prev_view_proj`).
pub(crate) fn reprojection(view: Mat4, prev_view_proj: Mat4) -> Mat4 {
    prev_view_proj * view.inverse() * Mat4::from_scale(glam::Vec3::new(1.0, 1.0, -1.0))
}

/// Size of the targets for a window of `w` x `h`.
pub(crate) fn target_size(w: u32, h: u32, q: Quality) -> (u32, u32) {
    if q.half_res {
        (w.div_ceil(2).max(1), h.div_ceil(2).max(1))
    } else {
        (w.max(1), h.max(1))
    }
}

/// Depth levels a target of `w` x `h` can hold, up to XeGTAO's five.
pub(crate) fn mip_count(w: u32, h: u32) -> u32 {
    (32 - w.max(h).max(1).leading_zeros()).min(MIP_LEVELS)
}

/// Pipelines and the constants buffer, made once.
pub(crate) struct Gtao {
    prefilter0: (wgpu::ComputePipeline, wgpu::BindGroupLayout),
    prefilter: (wgpu::ComputePipeline, wgpu::BindGroupLayout),
    main: (wgpu::ComputePipeline, wgpu::BindGroupLayout),
    temporal: (wgpu::ComputePipeline, wgpu::BindGroupLayout),
    denoise: (wgpu::ComputePipeline, wgpu::BindGroupLayout),
    denoise_final: (wgpu::ComputePipeline, wgpu::BindGroupLayout),
    denoise_bent: (wgpu::ComputePipeline, wgpu::BindGroupLayout),
    main_bitmask: (wgpu::ComputePipeline, wgpu::BindGroupLayout),
    denoise_il: (wgpu::ComputePipeline, wgpu::BindGroupLayout),
    denoise_il_final: (wgpu::ComputePipeline, wgpu::BindGroupLayout),
    buffer: wgpu::Buffer,
}

/// One turn of the occlusion's blur.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Blur {
    /// The visibility and the bent normal.
    Both,
    /// The bent normal alone (`denoise_bent`).
    Bent,
    /// Both, into the result (`denoise_final`).
    Final,
}

/// The screen-space indirect light's targets and bind groups.
pub(crate) struct IndirectTargets {
    main: wgpu::BindGroup,
    /// In order; the last one writes `result`.
    denoise: Vec<(wgpu::BindGroup, bool)>,
    /// The light (linear rgb, the units of the lit colour), sampled by the
    /// shading.
    pub result: wgpu::TextureView,
}

/// The targets of one window size and quality, with their bind groups.
pub(crate) struct Targets {
    quality: Quality,
    size: (u32, u32),
    mips: Vec<(u32, u32)>,
    prefilter0: wgpu::BindGroup,
    prefilter: Vec<wgpu::BindGroup>,
    main: wgpu::BindGroup,
    /// The accumulation writing history target `i` (reading the other).
    temporal: [wgpu::BindGroup; 2],
    /// The blur, in order (the last one writes `result`): from `main`'s
    /// result, or after the accumulation into history target `i`.
    denoise: Vec<(wgpu::BindGroup, Blur)>,
    denoise_temporal: [Vec<(wgpu::BindGroup, Blur)>; 2],
    history: History,
    /// This frame's history target, when it accumulates (set by `write`).
    accumulate: Option<usize>,
    /// The visibility in `.r` and the view-space bent normal in `.gba`
    /// (0..1), sampled by the shading.
    pub result: wgpu::TextureView,
    /// The mesh normals (`NORMAL_FORMAT`, the window's size), the depth
    /// prepass's colour target when the frame reads them.
    pub normals: wgpu::TextureView,
    /// With the screen-space indirect light.
    pub indirect: Option<IndirectTargets>,
}

fn uniform_entry() -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn texture_entry(binding: u32, sample_type: wgpu::TextureSampleType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn storage_entry(binding: u32, format: wgpu::TextureFormat) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format,
            view_dimension: wgpu::TextureViewDimension::D2,
        },
        count: None,
    }
}

const UNFILTERED: wgpu::TextureSampleType = wgpu::TextureSampleType::Float { filterable: false };
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;
/// The visibility and the bent normal while they are blurred.
const AO_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const EDGES_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const RESULT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
/// The history's view depth and frame count, two halves (pack2x16float).
const META_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Uint;
/// The indirect light while it is blurred, and its result.
const LIGHT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// The mesh normals of the depth prepass (xy, octahedral, halves: the
/// G-buffer's eight bits per channel would show steps in a smooth
/// occlusion), and the motion of each pixel (zw, `screen_motion` in
/// common.wgsl).
pub(crate) const NORMAL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

impl Gtao {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gtao"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/gtao.wgsl").into()),
        });
        let make = |entry: &'static str, entries: &[wgpu::BindGroupLayoutEntry]| {
            let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some(entry),
                entries,
            });
            let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(entry),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
            let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&pl),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            });
            (pipeline, layout)
        };
        Gtao {
            prefilter0: make(
                "prefilter_depth0",
                &[
                    uniform_entry(),
                    texture_entry(1, wgpu::TextureSampleType::Depth),
                    storage_entry(3, DEPTH_FORMAT),
                ],
            ),
            prefilter: make(
                "prefilter_depth",
                &[uniform_entry(), texture_entry(2, UNFILTERED), storage_entry(3, DEPTH_FORMAT)],
            ),
            main: make(
                "main",
                &[
                    uniform_entry(),
                    texture_entry(4, UNFILTERED),
                    storage_entry(5, AO_FORMAT),
                    storage_entry(6, EDGES_FORMAT),
                    texture_entry(11, UNFILTERED),
                ],
            ),
            temporal: make(
                "temporal",
                &[
                    uniform_entry(),
                    texture_entry(4, UNFILTERED),
                    texture_entry(7, UNFILTERED),
                    texture_entry(11, UNFILTERED),
                    texture_entry(15, UNFILTERED),
                    texture_entry(16, wgpu::TextureSampleType::Uint),
                    storage_entry(17, AO_FORMAT),
                    storage_entry(18, META_FORMAT),
                ],
            ),
            denoise: make(
                "denoise",
                &[
                    uniform_entry(),
                    storage_entry(5, AO_FORMAT),
                    texture_entry(7, UNFILTERED),
                    texture_entry(8, UNFILTERED),
                ],
            ),
            denoise_final: make(
                "denoise_final",
                &[
                    uniform_entry(),
                    texture_entry(7, UNFILTERED),
                    texture_entry(8, UNFILTERED),
                    storage_entry(9, RESULT_FORMAT),
                ],
            ),
            denoise_bent: make(
                "denoise_bent",
                &[
                    uniform_entry(),
                    storage_entry(5, AO_FORMAT),
                    texture_entry(7, UNFILTERED),
                    texture_entry(8, UNFILTERED),
                ],
            ),
            main_bitmask: make(
                "main_bitmask",
                &[
                    uniform_entry(),
                    texture_entry(4, UNFILTERED),
                    texture_entry(11, UNFILTERED),
                    texture_entry(13, UNFILTERED),
                    storage_entry(14, LIGHT_FORMAT),
                ],
            ),
            denoise_il: make(
                "denoise_il",
                &[
                    uniform_entry(),
                    texture_entry(8, UNFILTERED),
                    texture_entry(10, UNFILTERED),
                    storage_entry(14, LIGHT_FORMAT),
                ],
            ),
            denoise_il_final: make(
                "denoise_il_final",
                &[
                    uniform_entry(),
                    texture_entry(8, UNFILTERED),
                    texture_entry(10, UNFILTERED),
                    storage_entry(12, LIGHT_FORMAT),
                ],
            ),
            buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("gtao params"),
                size: std::mem::size_of::<GtaoU>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
        }
    }

    /// Targets for a window of `width` x `height` reading `depth` (the
    /// single-sample depth prepass), and with `scene` (the previous frame's
    /// lit colour) those of the indirect light.
    pub(crate) fn targets(
        &self,
        device: &wgpu::Device,
        depth: &wgpu::TextureView,
        scene: Option<&wgpu::TextureView>,
        width: u32,
        height: u32,
        level: u8,
    ) -> Targets {
        let q = quality(level);
        let (w, h) = target_size(width, height, q);
        let storage = wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING;
        let texture = |label: &str, format, mips| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: mips,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: storage,
                view_formats: &[],
            })
        };
        let mip_count = mip_count(w, h);
        let depth_mips = texture("gtao depth", DEPTH_FORMAT, mip_count);
        let mip_view = |level: u32| {
            depth_mips.create_view(&wgpu::TextureViewDescriptor {
                label: Some("gtao depth level"),
                base_mip_level: level,
                mip_level_count: Some(1),
                ..Default::default()
            })
        };
        let all_mips = depth_mips.create_view(&wgpu::TextureViewDescriptor::default());
        let view = |t: wgpu::Texture| t.create_view(&wgpu::TextureViewDescriptor::default());
        // `main` writes `ao`; the two history targets are the accumulation's
        // (this frame's and the previous one's), and the blur's second
        // working target: the other history once the accumulation has read
        // it, the first one without the accumulation
        let ao = view(texture("gtao a", AO_FORMAT, 1));
        let hist = [
            view(texture("gtao history a", AO_FORMAT, 1)),
            view(texture("gtao history b", AO_FORMAT, 1)),
        ];
        let meta = [
            view(texture("gtao history depth a", META_FORMAT, 1)),
            view(texture("gtao history depth b", META_FORMAT, 1)),
        ];
        let edges = view(texture("gtao edges", EDGES_FORMAT, 1));
        let result = view(texture("gtao", RESULT_FORMAT, 1));
        // drawn by the depth prepass, at the window's size
        let normals = view(device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gtao normals"),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: NORMAL_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        }));
        let group = |layout: &wgpu::BindGroupLayout, entries: &[(u32, &wgpu::TextureView)]| {
            let mut all = vec![wgpu::BindGroupEntry {
                binding: 0,
                resource: self.buffer.as_entire_binding(),
            }];
            all.extend(entries.iter().map(|&(binding, v)| wgpu::BindGroupEntry {
                binding,
                resource: wgpu::BindingResource::TextureView(v),
            }));
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("gtao"),
                layout,
                entries: &all,
            })
        };
        let prefilter0 = group(&self.prefilter0.1, &[(1, depth), (3, &mip_view(0))]);
        let prefilter = (1..mip_count)
            .map(|l| group(&self.prefilter.1, &[(2, &mip_view(l - 1)), (3, &mip_view(l))]))
            .collect();
        let main = group(&self.main.1, &[(4, &all_mips), (5, &ao), (6, &edges), (11, &normals)]);
        let temporal = [0, 1].map(|i| {
            group(
                &self.temporal.1,
                &[
                    (4, &all_mips),
                    (7, &ao),
                    (11, &normals),
                    (15, &hist[1 - i]),
                    (16, &meta[1 - i]),
                    (17, &hist[i]),
                    (18, &meta[i]),
                ],
            )
        });
        // the passes read `first` then the two working targets by turns; the
        // bent normal's own turns come before the last, which writes the
        // result
        let passes = q.denoise_passes.max(1);
        let kinds: Vec<Blur> = std::iter::repeat_n(Blur::Both, passes as usize - 1)
            .chain(std::iter::repeat_n(Blur::Bent, BENT_BLUR as usize))
            .chain(std::iter::once(Blur::Final))
            .collect();
        let chain = |first: &wgpu::TextureView, work: [&wgpu::TextureView; 2]| -> Vec<(wgpu::BindGroup, Blur)> {
            kinds
                .iter()
                .enumerate()
                .map(|(i, &kind)| {
                    let src = if i == 0 { first } else { work[(i + 1) % 2] };
                    let dst = work[i % 2];
                    let bg = match kind {
                        Blur::Final => group(&self.denoise_final.1, &[(7, src), (8, &edges), (9, &result)]),
                        Blur::Both => group(&self.denoise.1, &[(5, dst), (7, src), (8, &edges)]),
                        Blur::Bent => group(&self.denoise_bent.1, &[(5, dst), (7, src), (8, &edges)]),
                    };
                    (bg, kind)
                })
                .collect()
        };
        let denoise = chain(&ao, [&hist[0], &ao]);
        let denoise_temporal = [0, 1].map(|i| chain(&hist[i], [&ao, &hist[1 - i]]));
        let indirect = scene.map(|scene| {
            let light = [
                view(texture("gtao light a", LIGHT_FORMAT, 1)),
                view(texture("gtao light b", LIGHT_FORMAT, 1)),
            ];
            let result = view(texture("gtao light", LIGHT_FORMAT, 1));
            let main = group(
                &self.main_bitmask.1,
                &[(4, &all_mips), (11, &normals), (13, scene), (14, &light[0])],
            );
            // four turns more than the occlusion's blur: one bounce from a few
            // samples is grainy, and the light has no sharp detail to keep
            let il_passes = passes + INDIRECT_EXTRA_BLUR;
            let denoise = (0..il_passes)
                .map(|i| {
                    let (src, dst) = (&light[i as usize % 2], &light[(i as usize + 1) % 2]);
                    if i + 1 == il_passes {
                        (group(&self.denoise_il_final.1, &[(8, &edges), (10, src), (12, &result)]), true)
                    } else {
                        (group(&self.denoise_il.1, &[(8, &edges), (10, src), (14, dst)]), false)
                    }
                })
                .collect();
            IndirectTargets { main, denoise, result }
        });
        let mips = (0..mip_count).map(|l| ((w >> l).max(1), (h >> l).max(1))).collect();
        Targets {
            quality: q,
            size: (w, h),
            mips,
            prefilter0,
            prefilter,
            main,
            temporal,
            denoise,
            denoise_temporal,
            history: History::default(),
            accumulate: None,
            result,
            normals,
            indirect,
        }
    }

    /// The constants of this frame (before the encoder runs): `proj` its
    /// projection, `reproj` from [`reprojection`], `intensity` the indirect
    /// light's strength, `frame` what it knows of the frames before,
    /// `normals` the view when the depth prepass draws the mesh normals.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn write(
        &self,
        queue: &wgpu::Queue,
        t: &mut Targets,
        proj: Mat4,
        reproj: Mat4,
        intensity: f32,
        frame: Frame,
        normals: Option<Mat4>,
    ) {
        let next = t.history.next(frame);
        t.accumulate = next.map(|(target, _)| target);
        let temporal = next.map(|(_, history)| (frame.index, history));
        queue.write_buffer(
            &self.buffer,
            0,
            bytemuck::bytes_of(&uniforms(proj, reproj, intensity, t.size, t.quality, temporal, normals)),
        );
    }

    /// The passes, after the depth prepass; `indirect` with the indirect
    /// light (when its targets exist).
    pub(crate) fn encode(&self, encoder: &mut wgpu::CommandEncoder, t: &Targets, indirect: bool) {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("gtao"),
            timestamp_writes: None,
        });
        let groups = |(w, h): (u32, u32)| (w.div_ceil(8), h.div_ceil(8), 1);
        let dispatch = |pass: &mut wgpu::ComputePass, (x, y, z): (u32, u32, u32)| pass.dispatch_workgroups(x, y, z);
        pass.set_pipeline(&self.prefilter0.0);
        pass.set_bind_group(0, &t.prefilter0, &[]);
        dispatch(&mut pass, groups(t.mips[0]));
        pass.set_pipeline(&self.prefilter.0);
        for (bg, size) in t.prefilter.iter().zip(&t.mips[1..]) {
            pass.set_bind_group(0, bg, &[]);
            dispatch(&mut pass, groups(*size));
        }
        // the occlusion (and the edges the blurs follow), with or without the
        // light, then the light alone
        pass.set_pipeline(&self.main.0);
        pass.set_bind_group(0, &t.main, &[]);
        dispatch(&mut pass, groups(t.size));
        let il = t.indirect.as_ref().filter(|_| indirect);
        if let Some(il) = il {
            pass.set_pipeline(&self.main_bitmask.0);
            pass.set_bind_group(0, &il.main, &[]);
            dispatch(&mut pass, groups(t.size));
        }
        let denoise = match t.accumulate {
            Some(i) => {
                pass.set_pipeline(&self.temporal.0);
                pass.set_bind_group(0, &t.temporal[i], &[]);
                dispatch(&mut pass, groups(t.size));
                &t.denoise_temporal[i]
            }
            None => &t.denoise,
        };
        for (bg, kind) in denoise {
            pass.set_pipeline(match kind {
                Blur::Both => &self.denoise.0,
                Blur::Bent => &self.denoise_bent.0,
                Blur::Final => &self.denoise_final.0,
            });
            pass.set_bind_group(0, bg, &[]);
            dispatch(&mut pass, groups(t.size));
        }
        for (bg, last) in il.iter().flat_map(|il| &il.denoise) {
            pass.set_pipeline(if *last { &self.denoise_il_final.0 } else { &self.denoise_il.0 });
            pass.set_bind_group(0, bg, &[]);
            dispatch(&mut pass, groups(t.size));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_sample_more_as_they_go_up_and_low_is_half_resolution() {
        let q: Vec<Quality> = (0..4).map(quality).collect();
        assert!(q[0].half_res && q[1..].iter().all(|q| !q.half_res));
        let samples: Vec<u32> = q.iter().map(|q| q.slices * q.steps).collect();
        assert!(samples.windows(2).all(|w| w[0] <= w[1]), "{samples:?}");
        assert!(q.iter().all(|q| q.denoise_passes >= 1));
        // XeGTAO's high and ultra presets
        assert_eq!((q[2].slices, q[2].steps), (3, 3));
        assert_eq!((q[3].slices, q[3].steps), (9, 3));
        // anything above ultra is ultra
        assert_eq!(quality(7), q[3]);
    }

    #[test]
    fn target_size_and_levels_follow_the_window() {
        assert_eq!(target_size(2560, 1369, quality(0)), (1280, 685));
        assert_eq!(target_size(2560, 1369, quality(2)), (2560, 1369));
        assert_eq!(mip_count(2560, 1369), 5);
        assert_eq!(mip_count(8, 3), 4);
        assert_eq!(mip_count(1, 1), 1);
    }

    #[test]
    fn override_picks_the_methods_kept_ready_and_alternates_every_second() {
        let gtao = AoMethod::Gtao { quality: 1 };
        assert_eq!(Override::parse(None), Override::None);
        assert_eq!(Override::parse(Some(" GTAO ")), Override::Gtao(None));
        assert_eq!(Override::parse(Some("gtao:1")), Override::Gtao(Some(1)));
        assert_eq!(Override::parse(Some("alt:9")), Override::Alternate(Some(3)));
        assert_eq!(Override::parse(Some("legacy")), Override::Legacy);
        assert_eq!(Override::parse(Some("alt")), Override::Alternate(None));
        assert_eq!(Override::parse(Some("other")), Override::None);
        assert_eq!(Override::None.needed(AoMethod::Legacy), (true, false));
        assert_eq!(Override::None.needed(gtao), (false, true));
        assert_eq!(Override::Legacy.needed(gtao), (true, false));
        assert_eq!(Override::Alternate(None).needed(AoMethod::Legacy), (true, true));
        let alt: Vec<bool> = (0..4).map(|s| Override::Alternate(None).gtao_now(AoMethod::Legacy, s)).collect();
        assert_eq!(alt, [false, true, false, true]);
        assert!((0..4).all(|s| Override::None.gtao_now(gtao, s)));
        assert!((0..4).all(|s| !Override::None.gtao_now(AoMethod::Legacy, s)));
        assert_eq!(Override::None.level(gtao), 1);
        assert_eq!(Override::Gtao(None).level(AoMethod::Legacy), 2);
        assert_eq!(Override::Gtao(Some(0)).level(gtao), 0);
    }

    #[test]
    fn specular_switch_is_on_by_default_and_alternates_every_second() {
        assert_eq!(SpecularOcclusion::parse(None), SpecularOcclusion::On);
        assert_eq!(SpecularOcclusion::parse(Some("1")), SpecularOcclusion::On);
        assert_eq!(SpecularOcclusion::parse(Some(" 0 ")), SpecularOcclusion::Off);
        assert_eq!(SpecularOcclusion::parse(Some("alt")), SpecularOcclusion::Alternate);
        let alt: Vec<bool> = (0..4).map(|s| SpecularOcclusion::Alternate.applied(s)).collect();
        assert_eq!(alt, [true, false, true, false]);
        assert!((0..4).all(|s| SpecularOcclusion::On.applied(s) && !SpecularOcclusion::Off.applied(s)));
    }

    #[test]
    fn temporal_switch_is_on_by_default_and_alternates_every_second() {
        assert_eq!(Temporal::parse(None), Temporal::On);
        assert_eq!(Temporal::parse(Some("1")), Temporal::On);
        assert_eq!(Temporal::parse(Some(" off ")), Temporal::Off);
        assert_eq!(Temporal::parse(Some("0")), Temporal::Off);
        let alt: Vec<bool> = (0..4).map(|s| Temporal::Alternate.applied(s)).collect();
        assert_eq!(alt, [true, false, true, false]);
        assert!((0..4).all(|s| Temporal::On.applied(s) && !Temporal::Off.applied(s)));
    }

    #[test]
    fn normals_switch_is_the_mesh_by_default_and_alternates_every_second() {
        assert_eq!(Normals::parse(None), Normals::Mesh);
        assert_eq!(Normals::parse(Some("mesh")), Normals::Mesh);
        assert_eq!(Normals::parse(Some(" depth ")), Normals::Depth);
        assert_eq!(Normals::parse(Some("0")), Normals::Depth);
        assert_eq!(Normals::parse(Some("alt")), Normals::Alternate);
        let alt: Vec<bool> = (0..4).map(|s| Normals::Alternate.mesh(s)).collect();
        assert_eq!(alt, [true, false, true, false]);
        assert!((0..4).all(|s| Normals::Mesh.mesh(s) && !Normals::Depth.mesh(s)));
    }

    /// The mesh normals reach the pass's view space (x right, y up, z in
    /// front): a face turned to the camera points back along z, the ground
    /// up, and the flag tells the shader to read them.
    #[test]
    fn mesh_normals_turn_into_the_pass_view_space() {
        let eye = glam::Vec3::new(10.0, 5.0, 2.0);
        let view = glam::camera::rh::view::look_at_mat4(eye, glam::Vec3::new(14.0, 5.0, 2.0), glam::Vec3::Z);
        let m = normal_view(view);
        // a wall facing the camera (it looks along +x)
        assert!((m * -glam::Vec3::X - glam::Vec3::new(0.0, 0.0, -1.0)).length() < 1e-5);
        assert!((m * glam::Vec3::Z - glam::Vec3::Y).length() < 1e-5);
        // the camera's right (+x on screen) is -y in the world here
        assert!((m * -glam::Vec3::Y - glam::Vec3::X).length() < 1e-5);
        let proj = glam::camera::rh::proj::directx::perspective_infinite_reverse(1.0, 1.6, 0.1);
        let on = uniforms(proj, Mat4::IDENTITY, 1.0, (64, 64), quality(2), None, Some(view));
        assert_eq!(on.misc[3], 1.0);
        let col = |i: usize| glam::Vec3::from_slice(&on.normal_view[i][..3]);
        assert!(glam::Mat3::from_cols(col(0), col(1), col(2)).abs_diff_eq(m, 1e-6));
        let off = uniforms(proj, Mat4::IDENTITY, 1.0, (64, 64), quality(2), None, None);
        assert_eq!(off.misc[3], 0.0);
    }

    /// The history targets take turns while frames follow each other; a
    /// frame skipped, one without the accumulation or without the previous
    /// camera, and new targets start again from the current frame.
    #[test]
    fn history_takes_turns_and_starts_again_after_a_gap() {
        let frame = |index, temporal, previous_camera| Frame {
            index,
            temporal,
            previous_camera,
        };
        let mut h = History::default();
        assert_eq!(h.next(frame(10, true, true)), Some((0, false)), "new targets");
        assert_eq!(h.next(frame(11, true, true)), Some((1, true)));
        assert_eq!(h.next(frame(12, true, true)), Some((0, true)));
        assert_eq!(h.next(frame(14, true, true)), Some((0, false)), "a frame without GTAO");
        assert_eq!(h.next(frame(15, true, false)), Some((1, false)), "no previous camera");
        assert_eq!(h.next(frame(16, true, true)), Some((0, true)));
        assert_eq!(h.next(frame(17, false, true)), None);
        assert_eq!(h.next(frame(18, true, true)), Some((0, false)), "after a frame without it");
        // the frame counter wraps
        let mut h = History::default();
        h.next(frame(u64::MAX, true, true));
        assert_eq!(h.next(frame(0, true, true)), Some((1, true)));
    }

    #[test]
    fn noise_moves_only_while_accumulating_and_history_is_read_only_when_there() {
        let proj = glam::camera::rh::proj::directx::perspective_infinite_reverse(1.0, 1.6, 0.1);
        let u = |t| uniforms(proj, Mat4::IDENTITY, 1.0, (64, 64), quality(2), t, None).temporal;
        assert_eq!(u(None)[..2], [0.0, 1.0]);
        assert_eq!(u(Some((3, true)))[..2], [3.0, 0.0]);
        assert_eq!(u(Some((NOISE_FRAMES + 5, false)))[..2], [5.0, 1.0]);
        // without a history the frame samples more slices; without the
        // accumulation, as before
        let slices = |t| uniforms(proj, Mat4::IDENTITY, 1.0, (64, 64), quality(2), t, None).misc[2];
        assert_eq!(slices(None), 3.0);
        assert_eq!(slices(Some((3, true))), 3.0);
        assert_eq!(slices(Some((3, false))), 3.0 * NO_HISTORY_SLICES as f32);
    }

    #[test]
    fn intensity_override_is_a_clamped_number() {
        assert_eq!(parse_intensity(None), None);
        assert_eq!(parse_intensity(Some(" 2.5 ")), Some(2.5));
        assert_eq!(parse_intensity(Some("40")), Some(8.0));
        assert_eq!(parse_intensity(Some("-1")), Some(0.0));
        assert_eq!(parse_intensity(Some("NaN")), None);
        assert_eq!(parse_intensity(Some("much")), None);
    }

    #[test]
    fn indirect_switch_follows_the_settings_unless_forced() {
        assert_eq!(Indirect::parse(None), Indirect::Settings);
        assert_eq!(Indirect::parse(Some(" 1")), Indirect::On);
        assert_eq!(Indirect::parse(Some("0")), Indirect::Off);
        assert_eq!(Indirect::parse(Some("alt")), Indirect::Alternate);
        assert!(Indirect::Settings.needed(true) && !Indirect::Settings.needed(false));
        assert!(Indirect::On.needed(false) && !Indirect::Off.needed(true));
        // alternate keeps the targets and lights every other second
        assert!(Indirect::Alternate.needed(false));
        let alt: Vec<bool> = (0..4).map(|s| Indirect::Alternate.applied(false, s)).collect();
        assert_eq!(alt, [true, false, true, false]);
        assert!((0..4).all(|s| Indirect::Settings.applied(true, s) && !Indirect::Off.applied(true, s)));
    }

    /// The shader's `sector` and `sectors_between` (gtao.wgsl), line for line.
    fn sector(angle: f32, nn: f32) -> f32 {
        ((angle - nn).clamp(-std::f32::consts::FRAC_PI_2, std::f32::consts::FRAC_PI_2).sin() + 1.0) * 0.5
    }

    fn sectors_between(t0: f32, t1: f32) -> u32 {
        let lo = (t0.min(t1) * 32.0).floor().clamp(0.0, 32.0) as u32;
        let hi = (t0.max(t1) * 32.0).ceil().clamp(0.0, 32.0) as u32;
        if hi <= lo {
            return 0;
        }
        let count = hi - lo;
        let ones = if count >= 32 { u32::MAX } else { (1 << count) - 1 };
        ones << lo
    }

    /// The sectors are cosine-weighted: a wall standing beside the pixel
    /// from its horizon up to 45° above it hides 1 - sin 45° of its side's
    /// half, an object straight above hides the middle, and the order of
    /// the front and the back does not matter.
    #[test]
    fn sectors_cover_the_cosine_weighted_half_disc() {
        use std::f32::consts::{FRAC_PI_2, FRAC_PI_4};
        let all = sectors_between(sector(-FRAC_PI_2, 0.0), sector(FRAC_PI_2, 0.0));
        assert_eq!(all, u32::MAX);
        assert_eq!(sectors_between(0.5, 0.5), 0);
        // from 45° to the horizon on the positive side: (1 - sin 45°) / 2 of
        // the 32 sectors, rounded outwards
        let wall = sectors_between(sector(FRAC_PI_4, 0.0), sector(FRAC_PI_2, 0.0));
        let share = wall.count_ones() as f32 / 32.0;
        assert!((share - (1.0 - FRAC_PI_4.sin()) / 2.0).abs() <= 1.0 / 32.0, "{share}");
        assert_eq!(wall.leading_zeros(), 0, "the wall's sectors end at the horizon");
        // a thin object right above: a few sectors in the middle
        let above = sectors_between(sector(0.05, 0.0), sector(-0.05, 0.0));
        assert!(above.count_ones() <= 3 && above & (1 << 16) != 0, "{above:032b}");
        assert_eq!(sectors_between(0.2, 0.6), sectors_between(0.6, 0.2));
        // the table of the sectors' centres is their angle
        let centre = |k: f32| ((k + 0.5) / 16.0 - 1.0).asin();
        assert!((sector(centre(7.0), 0.0) * 32.0 - 7.5).abs() < 1e-4);
    }

    /// A point of the GTAO pass's view space lands where the previous
    /// frame's camera saw it: the same camera gives the same clip position,
    /// a camera moved sideways shifts it.
    #[test]
    fn reprojection_finds_the_point_in_the_previous_frame() {
        let proj = glam::camera::rh::proj::directx::perspective_infinite_reverse(1.0, 1.6, 0.1);
        let view = glam::camera::rh::view::look_at_mat4(glam::Vec3::new(10.0, 5.0, 2.0), glam::Vec3::new(14.0, 7.0, 2.5), glam::Vec3::Z);
        let world = glam::Vec3::new(13.0, 8.0, 2.2);
        let v = view.transform_point3(world);
        // the pass's view space: z in front
        let p = glam::Vec4::new(v.x, v.y, -v.z, 1.0);
        let same = reprojection(view, proj * view) * p;
        let expect = proj * view * world.extend(1.0);
        assert!((same / same.w - expect / expect.w).length() < 1e-4, "{same} {expect}");
        let moved = glam::camera::rh::view::look_at_mat4(glam::Vec3::new(10.5, 5.0, 2.0), glam::Vec3::new(14.5, 7.0, 2.5), glam::Vec3::Z);
        let before = reprojection(view, proj * moved) * p;
        let expect = proj * moved * world.extend(1.0);
        assert!((before / before.w - expect / expect.w).length() < 1e-4);
        assert!((before.x / before.w - same.x / same.w).abs() > 0.01);
    }

    /// The shader's `cone_specular_occlusion` (common.wgsl), line for line.
    fn cone_specular_occlusion(bent: glam::Vec3, visibility: f32, r: glam::Vec3, roughness: f32) -> f32 {
        fn caps(cos_a: f32, cos_b: f32, cos_d: f32) -> f32 {
            let (ra, rb, d) = (
                cos_a.clamp(-1.0, 1.0).acos(),
                cos_b.clamp(-1.0, 1.0).acos(),
                cos_d.clamp(-1.0, 1.0).acos(),
            );
            if ra.min(rb) <= ra.max(rb) - d {
                return 1.0 - cos_a.max(cos_b);
            }
            if ra + rb <= d {
                return 0.0;
            }
            let delta = (ra - rb).abs();
            let x = 1.0 - ((d - delta) / (ra + rb - delta).max(1e-4)).clamp(0.0, 1.0);
            x * x * (3.0 - 2.0 * x) * (1.0 - cos_a.max(cos_b))
        }
        let smooth = |e0: f32, e1: f32, x: f32| {
            let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        };
        let a = roughness * roughness;
        let cos_v = (1.0 - visibility).max(0.0).sqrt();
        let cos_s = (-std::f32::consts::LOG2_10 * a * a).exp2().min(0.9);
        let cos_b = bent.dot(r);
        let open = caps(0.0, cos_s, cos_b);
        if open <= 1e-6 {
            return 1.0;
        }
        let so = caps(cos_v, cos_s, cos_b) / open;
        1.0 + (so.clamp(0.0, 1.0).max(visibility) - 1.0) * smooth(0.01, 0.09, a)
    }

    /// Open surroundings let a reflection through, a crease hides the rays
    /// that leave away from its opening, and a mirror keeps its reflection.
    #[test]
    fn specular_occlusion_follows_the_opening_and_spares_mirrors() {
        let up = glam::Vec3::Z;
        // nothing around: the visibility cone is the whole hemisphere, and
        // a grazing ray (the rim of a sphere) keeps its reflection too
        assert!((cone_specular_occlusion(up, 1.0, up, 0.6) - 1.0).abs() < 1e-5);
        let rim = glam::Vec3::new(1.0, 0.0, 0.05).normalize();
        assert!((cone_specular_occlusion(up, 1.0, rim, 0.6) - 1.0).abs() < 1e-5);
        // a crease opening upwards: a ray up passes, a grazing one is hidden
        // down to the crease's ambient light, no further
        let grazing = glam::Vec3::new(1.0, 0.0, 0.1).normalize();
        let along = cone_specular_occlusion(up, 0.35, up, 0.6);
        let across = cone_specular_occlusion(up, 0.35, grazing, 0.6);
        assert!(along > 0.7 && (across - 0.35).abs() < 1e-5, "{along} {across}");
        // the less visible, the less reflection (same ray, rough enough)
        let r = glam::Vec3::new(0.5, 0.0, 1.0).normalize();
        let v: Vec<f32> = [1.0, 0.8, 0.6, 0.4, 0.2]
            .iter()
            .map(|&vis| cone_specular_occlusion(up, vis, r, 0.7))
            .collect();
        assert!(v.windows(2).all(|w| w[0] >= w[1] - 1e-5), "{v:?}");
        // a mirror (roughness below 0.3) is left alone, in any crease
        assert!((cone_specular_occlusion(up, 0.2, grazing, 0.1) - 1.0).abs() < 1e-6);
        // and the result stays between 0 and 1
        for vis in [0.0, 0.03, 0.5, 1.0] {
            for rough in [0.0, 0.3, 0.5, 1.0] {
                let so = cone_specular_occlusion(up, vis, grazing, rough);
                assert!((0.0..=1.0).contains(&so), "{vis} {rough} {so}");
            }
        }
    }

    /// The view depth the shader rebuilds from the projection's terms is the
    /// distance in front of the camera, and the position matches the point.
    #[test]
    fn uniforms_rebuild_view_positions() {
        let proj = glam::camera::rh::proj::directx::perspective_infinite_reverse(1.0, 1.6, 0.1);
        let u = uniforms(proj, Mat4::IDENTITY, 1.0, (1600, 1000), quality(2), None, None);
        // a point 7 m in front, 1 m right and 0.5 m up (right-handed view: -z)
        let p = glam::Vec4::new(1.0, 0.5, -7.0, 1.0);
        let clip = proj * p;
        let (ndc, d) = (glam::Vec2::new(clip.x / clip.w, clip.y / clip.w), clip.z / clip.w);
        let z = u.depth[0] / (d + u.depth[1]);
        assert!((z - 7.0).abs() < 1e-4, "{z}");
        let xy = (ndc + glam::Vec2::new(u.proj[2], u.proj[3])) / glam::Vec2::new(u.proj[0], u.proj[1]) * z;
        assert!((xy.x - 1.0).abs() < 1e-4 && (xy.y - 0.5).abs() < 1e-4, "{xy}");
    }

    /// The Hilbert index (shader's `hilbert_index`) visits every pixel of a
    /// 64 x 64 tile once, each step to a neighbour.
    #[test]
    fn hilbert_index_walks_the_tile() {
        fn hilbert(mut x: u32, mut y: u32) -> u32 {
            let mut index = 0;
            let mut level = 32;
            while level > 0 {
                let rx = u32::from(x & level > 0);
                let ry = u32::from(y & level > 0);
                index += level * level * ((3 * rx) ^ ry);
                if ry == 0 {
                    if rx == 1 {
                        x = 63 - x;
                        y = 63 - y;
                    }
                    std::mem::swap(&mut x, &mut y);
                }
                level /= 2;
            }
            index
        }
        let mut at = vec![(0u32, 0u32); 64 * 64];
        let mut seen = vec![false; 64 * 64];
        for y in 0..64 {
            for x in 0..64 {
                let i = hilbert(x, y) as usize;
                assert!(!seen[i], "index {i} twice");
                seen[i] = true;
                at[i] = (x, y);
            }
        }
        for w in at.windows(2) {
            let d = w[0].0.abs_diff(w[1].0) + w[0].1.abs_diff(w[1].1);
            assert_eq!(d, 1, "{:?} -> {:?}", w[0], w[1]);
        }
    }
}
