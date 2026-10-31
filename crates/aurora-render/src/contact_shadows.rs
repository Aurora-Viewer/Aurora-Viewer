//! Screen-space contact shadows of the sun or the moon: Bend Studio's
//! screen space shadows (Apache 2.0, see NOTICE.md) in one compute shader
//! run over a few dispatches from the single-sample depth prepass
//! (shaders/contact_shadows.wgsl). The sun's shadow map lacks the
//! resolution for what touches: feet on the ground, objects resting on a
//! surface, folds, jewellery; a short march towards the light on screen
//! finds those occluders and the shading takes the darker of the two
//! (`contact_shadow` in common.wgsl). Opaque faces of the main view only.
//!
//! The dispatches are laid out around the light's point on screen
//! ([`build_dispatch_list`], a port of Bend's `BuildDispatchList`,
//! bend_sss_cpu.h): each workgroup is a wavefront of [`WAVE_SIZE`] pixels
//! along a ray towards that point, in up to eight dispatches.
//!
//! What casts: the depth prepass (opaque and masked faces, terrain), and,
//! drawn into a copy of it each frame, the blended faces opaque enough
//! (half covered: the many mesh bodies, shoes and hair whose faces are
//! blended though they look opaque) and the impostor cards; those blended
//! faces then also receive them.
//!
//! `AURORA_CONTACT_SHADOWS=0|1|alt|prepass|shell` overrides the setting for
//! a comparison; `alt` switches every second, `prepass`: the prepass alone
//! casts (before the blended faces and the cards were added), `shell`: the
//! occluders are Bend's thin shell instead of [`OCCLUDER_DEPTH`] deep.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};

/// Pixels per wavefront (one workgroup). Must match the shader's.
pub(crate) const WAVE_SIZE: i32 = 64;

/// Bend's samples per pixel (SampleCount): the shadow's length in pixels
/// of the window, and HardShadowSamples, the first of them that alone can
/// shadow the pixel (the last twice as many fade out).
const SAMPLE_COUNT: f32 = 60.0;
const HARD_SHADOW_SAMPLES: f32 = 4.0;

/// The most samples per pixel (8 times Bend's), for a photo up to 8 times
/// taller than the viewfinder's view of it. Must match the shader's
/// MAX_SAMPLE_COUNT, which sizes its workgroup memory.
pub(crate) const MAX_SAMPLE_COUNT: i32 = 480;

/// The samples per pixel and the hard ones for an image `scale` times
/// taller than the viewfinder's view of the same field (1 in the window).
/// At a fixed 60 pixels a 4K or 8K photo, 3 to 7 times the window's
/// height, had shadows 3 to 7 times shorter than the viewfinder showed
/// (issue #1662); in proportion they keep their length in the image.
pub(crate) fn samples(scale: f32) -> (i32, i32) {
    let scale = if scale.is_finite() { scale.max(1.0) } else { 1.0 };
    let count = ((SAMPLE_COUNT * scale).round() as i32).min(MAX_SAMPLE_COUNT);
    let hard = ((HARD_SHADOW_SAMPLES * count as f32 / SAMPLE_COUNT).round() as i32).max(1);
    (count, hard)
}

/// How far, in pixels of the image, the contact shadow of a pixel reads the
/// depth around it at `scale` ([`samples`]): its samples, the bilinear
/// read beside the ray, and the filter's radius (contact_shadows_filter.wgsl).
/// A photo drawn in tiles draws this much around each one; 0 when the
/// contact shadows are off (`on`, the setting; `AURORA_CONTACT_SHADOWS`
/// included).
pub fn tile_reach(on: bool, scale: f32) -> u32 {
    if Override::from_env().needed(on) {
        samples(scale).0 as u32 + 1 + 2
    } else {
        0
    }
}

/// Bend's starting values (DispatchParameters::SetDefaults): the surface
/// thickness is a share of the depth left to the far plane (0.5 %: about
/// 5 cm at 10 m with the reversed depth of an infinite projection), the
/// edge threshold of the bilinear reads, the contrast of the shadow's
/// border.
const SURFACE_THICKNESS: f32 = 0.005;
const BILINEAR_THRESHOLD: f32 = 0.02;
const SHADOW_CONTRAST: f32 = 4.0;

/// The deepest an occluder is taken to be behind its visible face, in
/// metres. Bend's window is a shell of one surface thickness (1.5 cm at
/// 3 m): a ray that went under a shoe or a hoof, through its body behind the
/// visible faces, was not stopped, and only the outline where the edges
/// cross the ray darkened the ground (issue 988). The shader takes the body
/// as deep as the ray went under it, back to the pixel's own depth, up to
/// this (`depth_delta`, contact_shadows.wgsl); one shell from 10 m on. With
/// one layer of depth a ray that passes beside or behind a caster looks the
/// same, so each caster also darkens a little of the ground beside it: at
/// 15 cm that copy of its outline showed (demo `AURORA_DEMO_FEET`, sun
/// ahead), at 5 cm it stays within the shadow's own edge.
const OCCLUDER_DEPTH: f32 = 0.05;

/// The shader's `tuning.w`: [`OCCLUDER_DEPTH`] in units of surface
/// thickness per unit of reversed depth. With an infinite reversed
/// projection a pixel at `z` metres has the depth `near / z`, so `depth`
/// metres are `depth / (thickness × z)` units, `depth × d / (thickness ×
/// near)`; 0 (Bend's shell) without a usable near plane.
pub(crate) fn occluder_units(depth: f32, near: f32) -> f32 {
    if near > 0.0 && depth > 0.0 {
        depth / (SURFACE_THICKNESS * near)
    } else {
        0.0
    }
}

/// One compute dispatch: workgroup counts and its first wave relative to
/// the light (`DispatchData`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Dispatch {
    pub wave_count: [i32; 3],
    pub wave_offset: [i32; 2],
}

/// The dispatches of one light (`DispatchList`): at most eight.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct DispatchList {
    /// The light on screen: x, y in pixels (y down), z its depth, w 1 in
    /// front of the camera, -1 behind.
    pub light: [f32; 4],
    pub dispatch: [Dispatch; 8],
    pub count: usize,
}

impl DispatchList {
    pub(crate) fn dispatches(&self) -> &[Dispatch] {
        &self.dispatch[..self.count]
    }
}

/// Port of Bend's `BuildDispatchList` (bend_sss_cpu.h): the dispatches
/// that cover `min_bounds..=max_bounds` of a viewport of `viewport` pixels
/// for a light whose homogeneous clip position is `light` (a direction
/// with w = 0 for the sun). Depth in [0, 1] (no expanded range).
pub(crate) fn build_dispatch_list(
    light: [f32; 4],
    viewport: [i32; 2],
    min_bounds: [i32; 2],
    max_bounds: [i32; 2],
    wave: i32,
) -> DispatchList {
    let mut out = DispatchList::default();
    // the division loses precision when the light is very far off screen
    let mut xy_light_w = light[3];
    let fp_limit = 0.000002 * wave as f32;
    if xy_light_w >= 0.0 && xy_light_w < fp_limit {
        xy_light_w = fp_limit;
    } else if xy_light_w < 0.0 && xy_light_w > -fp_limit {
        xy_light_w = -fp_limit;
    }
    out.light[0] = ((light[0] / xy_light_w) * 0.5 + 0.5) * viewport[0] as f32;
    out.light[1] = ((light[1] / xy_light_w) * -0.5 + 0.5) * viewport[1] as f32;
    out.light[2] = if light[3] == 0.0 { 0.0 } else { light[2] / light[3] };
    out.light[3] = if light[3] > 0.0 { 1.0 } else { -1.0 };

    let light_xy = [(out.light[0] + 0.5) as i32, (out.light[1] + 0.5) as i32];
    // the bounds, inclusive, relative to the light
    let biased = [
        min_bounds[0] - light_xy[0],
        -(max_bounds[1] - light_xy[1]),
        max_bounds[0] - light_xy[0],
        -(min_bounds[1] - light_xy[1]),
    ];
    // four quadrants around the light, each split in two on its longer axis
    // when it is not square
    for q in 0..4 {
        let vertical = q == 0 || q == 3;
        let bounds = [
            (if q & 1 != 0 { biased[0] } else { -biased[2] }).max(0) / wave,
            (if q & 2 != 0 { biased[1] } else { -biased[3] }).max(0) / wave,
            ((if q & 1 != 0 { biased[2] } else { -biased[0] }) + wave * if vertical { 1 } else { 2 } - 1).max(0) / wave,
            ((if q & 2 != 0 { biased[3] } else { -biased[1] }) + wave * if vertical { 2 } else { 1 } - 1).max(0) / wave,
        ];
        if bounds[2] - bounds[0] <= 0 || bounds[3] - bounds[1] <= 0 {
            continue;
        }
        let bias_x = i32::from(q == 2 || q == 3);
        let bias_y = i32::from(q == 1 || q == 3);
        let first = out.count;
        out.dispatch[first] = Dispatch {
            wave_count: [wave, bounds[2] - bounds[0], bounds[3] - bounds[1]],
            wave_offset: [
                (if q & 1 != 0 { bounds[0] } else { -bounds[2] }) + bias_x,
                (if q & 2 != 0 { -bounds[3] } else { bounds[1] }) + bias_y,
            ],
        };
        out.count += 1;
        // where the diagonal from the light meets the far corner of the
        // quadrant
        let mut axis_delta = biased[0] - biased[1];
        if q == 1 {
            axis_delta = biased[2] + biased[1];
        }
        if q == 2 {
            axis_delta = -biased[0] - biased[3];
        }
        if q == 3 {
            axis_delta = -biased[2] + biased[3];
        }
        axis_delta = (axis_delta + wave - 1) / wave;
        if axis_delta <= 0 {
            continue;
        }
        let second = out.count;
        let mut disp = out.dispatch[first];
        let mut disp2 = disp;
        match q {
            0 => {
                // split on y, the split one larger on x
                disp2.wave_count[2] = disp.wave_count[2].min(axis_delta);
                disp.wave_count[2] -= disp2.wave_count[2];
                disp2.wave_offset[1] = disp.wave_offset[1] + disp.wave_count[2];
                disp2.wave_offset[0] -= 1;
                disp2.wave_count[1] += 1;
            }
            1 => {
                // split on x, the split one larger on y
                disp2.wave_count[1] = disp.wave_count[1].min(axis_delta);
                disp.wave_count[1] -= disp2.wave_count[1];
                disp2.wave_offset[0] = disp.wave_offset[0] + disp.wave_count[1];
                disp2.wave_count[2] += 1;
            }
            2 => {
                // split on x, the split one larger on y
                disp2.wave_count[1] = disp.wave_count[1].min(axis_delta);
                disp.wave_count[1] -= disp2.wave_count[1];
                disp.wave_offset[0] += disp2.wave_count[1];
                disp2.wave_count[2] += 1;
                disp2.wave_offset[1] -= 1;
            }
            _ => {
                // split on y, the split one larger on x
                disp2.wave_count[2] = disp.wave_count[2].min(axis_delta);
                disp.wave_count[2] -= disp2.wave_count[2];
                disp.wave_offset[1] += disp2.wave_count[2];
                disp2.wave_count[1] += 1;
            }
        }
        out.dispatch[first] = disp;
        out.dispatch[second] = disp2;
        out.count += 1;
        // drop the empty ones (the original's swap with the last entry)
        if disp2.wave_count[1] <= 0 || disp2.wave_count[2] <= 0 {
            out.count -= 1;
            out.dispatch[second] = out.dispatch[out.count];
        }
        if disp.wave_count[1] <= 0 || disp.wave_count[2] <= 0 {
            out.count -= 1;
            out.dispatch[first] = out.dispatch[out.count];
        }
    }
    // the shader takes the offsets in pixels
    for d in &mut out.dispatch[..out.count] {
        d.wave_offset[0] *= wave;
        d.wave_offset[1] *= wave;
    }
    out
}

/// `AURORA_CONTACT_SHADOWS`: the setting forced for a comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Override {
    None,
    Off,
    On,
    /// Off and on by turns, one second each (on in odd seconds).
    Alternate,
    /// On, with the depth prepass alone as caster.
    Prepass,
    /// On, the occluders taken as Bend's shell of one surface thickness
    /// (before issue 988).
    Shell,
}

impl Override {
    pub(crate) fn parse(v: Option<&str>) -> Self {
        match v.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
            Some("0" | "off") => Override::Off,
            Some("1" | "on") => Override::On,
            Some("alt") => Override::Alternate,
            Some("prepass") => Override::Prepass,
            Some("shell") => Override::Shell,
            _ => Override::None,
        }
    }

    pub(crate) fn from_env() -> Self {
        static MODE: std::sync::OnceLock<Override> = std::sync::OnceLock::new();
        *MODE.get_or_init(|| Override::parse(std::env::var("AURORA_CONTACT_SHADOWS").ok().as_deref()))
    }

    /// Whether the targets are kept for the setting `on`.
    pub(crate) fn needed(self, on: bool) -> bool {
        match self {
            Override::None => on,
            Override::Off => false,
            Override::On | Override::Alternate | Override::Prepass | Override::Shell => true,
        }
    }

    /// The deepest an occluder is taken to be behind its visible face,
    /// metres ([`OCCLUDER_DEPTH`], none with `shell`).
    pub(crate) fn occluder_depth(self) -> f32 {
        if self == Override::Shell { 0.0 } else { OCCLUDER_DEPTH }
    }

    /// Whether the blended faces and the impostor cards cast too.
    pub(crate) fn all_casters(self) -> bool {
        self != Override::Prepass
    }

    /// Whether the frame drawn `secs` seconds into the session has them.
    pub(crate) fn on_now(self, on: bool, secs: u64) -> bool {
        match self {
            Override::Alternate => secs % 2 == 1,
            _ => self.needed(on),
        }
    }
}

/// Must match `Params` in contact_shadows.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ParamsU {
    light: [f32; 4],
    wave_offset: [i32; 4],
    tuning: [f32; 4],
}

/// The light of the frame as Bend takes it: the direction towards it (w =
/// 0) through the view-projection the depth prepass was drawn with.
pub(crate) fn light_projection(view_proj: Mat4, towards_light: Vec3) -> [f32; 4] {
    (view_proj * towards_light.normalize_or(Vec3::Z).extend(0.0)).to_array()
}

/// The pipeline and the constants of the dispatches, made once.
pub(crate) struct ContactShadows {
    pipeline: wgpu::ComputePipeline,
    clear: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    buffer: wgpu::Buffer,
    /// Bytes between the constants of two dispatches.
    stride: u64,
    /// This frame's dispatches (`write`, then `encode`).
    list: DispatchList,
    filter: Filter,
}

/// The softening of the mask before the shading reads it
/// (shaders/contact_shadows_filter.wgsl).
struct Filter {
    soft: wgpu::ComputePipeline,
    copy: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
}

impl Filter {
    fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("contact shadows filter"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/contact_shadows_filter.wgsl").into()),
        });
        let texture = |binding: u32, sample_type: wgpu::TextureSampleType| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Texture {
                sample_type,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("contact shadows filter"),
            entries: &[
                texture(0, wgpu::TextureSampleType::Depth),
                texture(1, wgpu::TextureSampleType::Float { filterable: false }),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: RESULT_FORMAT,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
            ],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("contact shadows filter"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let make = |entry: &'static str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("contact shadows filter"),
                layout: Some(&pl),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        Filter {
            soft: make("main"),
            copy: make("copy"),
            layout,
        }
    }
}

/// `AURORA_CONTACT_SOFT=0`: the mask reaches the shading as the pass wrote
/// it, hard pixel steps, for a comparison.
fn soft_from_env() -> bool {
    static SOFT: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SOFT.get_or_init(|| std::env::var("AURORA_CONTACT_SOFT").map_or(true, |v| v.trim() != "0"))
}

/// The mask of one window size, the depth it may read instead of the
/// prepass, and their bind groups.
pub(crate) struct Targets {
    size: (u32, u32),
    /// Reading the depth prepass.
    group: wgpu::BindGroup,
    /// Reading `depth`: the prepass with the blended faces and the cards.
    group_more: wgpu::BindGroup,
    /// The filter's, reading the same depths and the pass's mask.
    filter_group: wgpu::BindGroup,
    filter_group_more: wgpu::BindGroup,
    depth_texture: wgpu::Texture,
    depth: wgpu::TextureView,
    /// The shadow in `.r`, 0 lit, 1 in contact shadow, softened; read by the
    /// shading.
    pub result: wgpu::TextureView,
}

const RESULT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

impl Targets {
    /// The copy of the prepass the other casters are drawn into.
    pub(crate) fn depth_texture(&self) -> &wgpu::Texture {
        &self.depth_texture
    }

    pub(crate) fn depth_view(&self) -> &wgpu::TextureView {
        &self.depth
    }
}

impl ContactShadows {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("contact shadows"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/contact_shadows.wgsl").into()),
        });
        let stride = std::mem::size_of::<ParamsU>().next_multiple_of(device.limits().min_uniform_buffer_offset_alignment as usize) as u64;
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("contact shadows"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<ParamsU>() as u64),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: RESULT_FORMAT,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
            ],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("contact shadows"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let make = |entry: &'static str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("contact shadows"),
                layout: Some(&pl),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let (pipeline, clear) = (make("main"), make("clear"));
        let filter = Filter::new(device);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("contact shadows params"),
            size: stride * 8,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        ContactShadows {
            pipeline,
            clear,
            layout,
            buffer,
            stride,
            list: DispatchList::default(),
            filter,
        }
    }

    /// The mask for a window of `width` x `height` reading `depth` (the
    /// single-sample depth prepass).
    pub(crate) fn targets(
        &self,
        device: &wgpu::Device,
        depth: &wgpu::TextureView,
        depth_format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> Targets {
        let size = (width.max(1), height.max(1));
        let mask = |label: &str| {
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
                    format: RESULT_FORMAT,
                    usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        // the pass writes `raw`, the filter `result`
        let (raw, result) = (mask("contact shadows raw"), mask("contact shadows"));
        let depth_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("contact shadows depth"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: depth_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let own_depth = depth_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let group = |depth: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("contact shadows"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &self.buffer,
                            offset: 0,
                            size: wgpu::BufferSize::new(std::mem::size_of::<ParamsU>() as u64),
                        }),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(depth),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&raw),
                    },
                ],
            })
        };
        let filter_group = |depth: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("contact shadows filter"),
                layout: &self.filter.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(depth),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&raw),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&result),
                    },
                ],
            })
        };
        Targets {
            size,
            group: group(depth),
            group_more: group(&own_depth),
            filter_group: filter_group(depth),
            filter_group_more: filter_group(&own_depth),
            depth_texture,
            depth: own_depth,
            result,
        }
    }

    /// The dispatches and their constants for a light at `light` (clip
    /// position, [`light_projection`]) seen through a projection whose near
    /// plane is `near` metres away, in an image `scale` times taller than
    /// the viewfinder's view of it ([`samples`]), before the encoder runs.
    pub(crate) fn write(&mut self, queue: &wgpu::Queue, t: &Targets, light: [f32; 4], near: f32, scale: f32) {
        let (w, h) = (t.size.0 as i32, t.size.1 as i32);
        self.list = build_dispatch_list(light, [w, h], [0, 0], [w, h], WAVE_SIZE);
        let occluder = occluder_units(Override::from_env().occluder_depth(), near);
        let (count, hard) = samples(scale);
        let mut bytes = vec![0u8; (self.stride as usize) * self.list.count];
        for (i, d) in self.list.dispatches().iter().enumerate() {
            let u = ParamsU {
                light: self.list.light,
                wave_offset: [d.wave_offset[0], d.wave_offset[1], count, hard],
                tuning: [SURFACE_THICKNESS, BILINEAR_THRESHOLD, SHADOW_CONTRAST, occluder],
            };
            let at = i * self.stride as usize;
            bytes[at..at + std::mem::size_of::<ParamsU>()].copy_from_slice(bytemuck::bytes_of(&u));
        }
        if !bytes.is_empty() {
            queue.write_buffer(&self.buffer, 0, &bytes);
        }
    }

    /// The dispatches of `write`, after the depth prepass (`more`: after
    /// the blended faces and the cards were drawn into `depth`). No barrier is
    /// needed between them: each writes its own pixels. The target is
    /// emptied first (0, lit): the pixel under the light's point, which no
    /// wavefront writes, and the sky stay lit. Then the filter softens the
    /// mask into `result`, over the same depth.
    pub(crate) fn encode(&self, encoder: &mut wgpu::CommandEncoder, t: &Targets, more: bool) {
        let (group, filter_group) = if more {
            (&t.group_more, &t.filter_group_more)
        } else {
            (&t.group, &t.filter_group)
        };
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("contact shadows"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.clear);
        pass.set_bind_group(0, group, &[0]);
        pass.dispatch_workgroups(t.size.0.div_ceil(8), t.size.1.div_ceil(8), 1);
        pass.set_pipeline(&self.pipeline);
        for (i, d) in self.list.dispatches().iter().enumerate() {
            pass.set_bind_group(0, group, &[(i as u64 * self.stride) as u32]);
            pass.dispatch_workgroups(d.wave_count[0] as u32, d.wave_count[1] as u32, d.wave_count[2] as u32);
        }
        pass.set_pipeline(if soft_from_env() { &self.filter.soft } else { &self.filter.copy });
        pass.set_bind_group(0, filter_group, &[]);
        pass.dispatch_workgroups(t.size.0.div_ceil(8), t.size.1.div_ceil(8), 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pixel each thread of each wavefront writes, as the shader finds
    /// it (`ComputeWavefrontExtents`, contact_shadows.wgsl): a CPU copy of
    /// its first lines.
    fn written_pixels(list: &DispatchList, wave: i32, mut each: impl FnMut(i32, i32)) {
        let light = list.light;
        let light_xy = [light[0].floor() + 0.5, light[1].floor() + 0.5];
        let frac = [light[0] - light_xy[0], light[1] - light_xy[1]];
        let reverse = light[3] > 0.0;
        for d in list.dispatches() {
            for gz in 0..d.wave_count[2] {
                for gy in 0..d.wave_count[1] {
                    for gx in 0..d.wave_count[0] {
                        let mut xy = [gy * wave + d.wave_offset[0], gz * wave + d.wave_offset[1]];
                        let sign = [xy[0].signum(), xy[1].signum()];
                        let horizontal = (xy[0] + sign[1]).abs() < (xy[1] - sign[0]).abs();
                        let axis = if horizontal { [sign[1], 0] } else { [0, -sign[0]] };
                        xy = [axis[0] * gx + xy[0], axis[1] * gx + xy[1]];
                        let f = [xy[0] as f32, xy[1] as f32];
                        let x_major = f[0].abs() > f[1].abs();
                        let major = if x_major { f[0] } else { f[1] };
                        let (start_m, end_m) = (major.abs(), major.abs() - wave as f32);
                        let mut ma = if x_major { frac[0] } else { frac[1] };
                        if major > 0.0 {
                            ma = -ma;
                        }
                        let start = [f[0] + light_xy[0], f[1] + light_xy[1]];
                        let t = (end_m + ma) / (start_m + ma);
                        let end = [light[0] + (start[0] - light[0]) * t, light[1] + (start[1] - light[1]) * t];
                        for tid in 0..wave {
                            let step = (tid ^ if reverse { 0 } else { wave - 1 }) as f32;
                            let p = [
                                start[0] + (end[0] - start[0]) * step / wave as f32,
                                start[1] + (end[1] - start[1]) * step / wave as f32,
                            ];
                            each(p[0].floor() as i32, p[1].floor() as i32);
                        }
                    }
                }
            }
        }
    }

    fn coverage(light: [f32; 4], w: i32, h: i32) -> (usize, DispatchList) {
        let list = build_dispatch_list(light, [w, h], [0, 0], [w, h], WAVE_SIZE);
        let mut seen = vec![false; (w * h) as usize];
        written_pixels(&list, WAVE_SIZE, |x, y| {
            if x >= 0 && y >= 0 && x < w && y < h {
                seen[(y * w + x) as usize] = true;
            }
        });
        // the pixel under the light's point is left out (it stays lit)
        let at = (list.light[0].floor() as i32, list.light[1].floor() as i32);
        let missed = (0..w * h).filter(|&i| !seen[i as usize] && (i % w, i / w) != at).count();
        (missed, list)
    }

    /// Every pixel of the screen gets a value, wherever the light falls:
    /// off screen on each side, on screen, behind the camera (but the one
    /// under the light's point, cleared to lit).
    #[test]
    fn the_dispatches_cover_every_pixel() {
        let (w, h) = (640, 360);
        for light in [
            // directional lights well off screen (w = 0 as for the sun)
            [0.3, 0.8, 0.1, 0.0],
            [-2.0, 0.1, 0.1, 0.0],
            [0.0, -3.0, 0.1, 0.0],
            // on screen, in front of the camera and behind it
            [0.2, -0.3, 0.5, 1.0],
            [-0.6, 0.4, 0.5, -1.0],
            [0.0, 0.0, 0.5, 1.0],
        ] {
            let (missed, list) = coverage(light, w, h);
            assert_eq!(missed, 0, "light {light:?}: {} dispatches", list.count);
            assert!((1..=8).contains(&list.count), "{}", list.count);
        }
    }

    /// The light's point on screen and its side of the camera, as Bend
    /// computes them.
    #[test]
    fn the_light_lands_on_its_pixel() {
        let list = build_dispatch_list([0.5, 0.5, 0.25, 1.0], [800, 600], [0, 0], [800, 600], WAVE_SIZE);
        assert_eq!(list.light, [600.0, 150.0, 0.25, 1.0]);
        let behind = build_dispatch_list([0.5, 0.5, 0.25, -1.0], [800, 600], [0, 0], [800, 600], WAVE_SIZE);
        assert_eq!(behind.light[3], -1.0);
        // a direction (w = 0): far off along its axis, depth 0
        let sun = build_dispatch_list([1.0, 0.0, 0.0, 0.0], [800, 600], [0, 0], [800, 600], WAVE_SIZE);
        assert!(sun.light[0] > 1e6 && sun.light[2] == 0.0 && sun.light[3] == -1.0);
    }

    /// The sun in front of the camera lands where a far point along it
    /// projects; one behind the camera gives a negative w.
    #[test]
    fn light_projection_follows_the_view() {
        let proj = glam::camera::rh::proj::directx::perspective_infinite_reverse(1.0, 1.6, 0.1);
        let view = glam::camera::rh::view::look_at_mat4(Vec3::ZERO, Vec3::X, Vec3::Z);
        let vp = proj * view;
        let dir = Vec3::new(1.0, 0.2, 0.3);
        let ahead = light_projection(vp, dir);
        assert!(ahead[3] > 0.0);
        // a point far along the direction lands on the same spot of the screen
        let far = vp * (dir.normalize() * 1e5).extend(1.0);
        assert!((ahead[0] / ahead[3] - far.x / far.w).abs() < 1e-3);
        assert!((ahead[1] / ahead[3] - far.y / far.w).abs() < 1e-3);
        let behind = light_projection(vp, Vec3::new(-1.0, 0.0, 0.5));
        assert!(behind[3] < 0.0);
    }

    /// The occluders' depth in the shader's units: `tuning.w` times the
    /// pixel's reversed depth is that many metres over one surface
    /// thickness at the pixel's distance (issue 988).
    #[test]
    fn occluder_depth_is_metres_over_the_surface_thickness() {
        let near = 0.1;
        let proj = glam::camera::rh::proj::directx::perspective_infinite_reverse(1.0, 1.6, near);
        let w = occluder_units(OCCLUDER_DEPTH, proj.w_axis.z);
        for z in [1.0f32, 3.0, 10.0, 40.0] {
            let clip = proj * glam::Vec4::new(0.0, 0.0, -z, 1.0);
            let units = w * clip.z / clip.w;
            let expected = OCCLUDER_DEPTH / (SURFACE_THICKNESS * z);
            assert!(
                (units - expected).abs() < expected * 1e-3,
                "{z} m: {units} units, {expected} expected"
            );
        }
        // 5 cm is ten shells at 1 m, less than one past 10 m (the shader
        // keeps one there)
        assert!((w * near / 1.0 - 10.0).abs() < 1e-3);
        assert!(w * near / 12.0 < 1.0);
        assert_eq!(occluder_units(Override::Shell.occluder_depth(), near), 0.0);
        assert_eq!(occluder_units(OCCLUDER_DEPTH, 0.0), 0.0);
        assert_eq!(Override::None.occluder_depth(), OCCLUDER_DEPTH);
    }

    #[test]
    fn override_keeps_the_mask_and_alternates_every_second() {
        assert_eq!(Override::parse(None), Override::None);
        assert_eq!(Override::parse(Some(" ALT ")), Override::Alternate);
        assert_eq!(Override::parse(Some("0")), Override::Off);
        assert_eq!(Override::parse(Some("1")), Override::On);
        assert_eq!(Override::parse(Some("maybe")), Override::None);
        assert_eq!(Override::parse(Some("prepass")), Override::Prepass);
        assert_eq!(Override::parse(Some("shell")), Override::Shell);
        assert!(Override::Shell.needed(false) && Override::Shell.all_casters());
        assert!(Override::Prepass.needed(false) && !Override::Prepass.all_casters());
        assert!(Override::None.all_casters() && Override::Alternate.all_casters());
        assert!(Override::None.needed(true) && !Override::None.needed(false));
        assert!(!Override::Off.needed(true) && Override::On.needed(false));
        assert!(Override::Alternate.needed(false));
        let alt: Vec<bool> = (0..4).map(|s| Override::Alternate.on_now(false, s)).collect();
        assert_eq!(alt, [false, true, false, true]);
        assert!((0..4).all(|s| Override::None.on_now(true, s)));
    }

    #[test]
    fn a_photo_keeps_the_shadows_length_in_the_image() {
        // the window: Bend's 60 samples, 4 hard
        assert_eq!(samples(1.0), (60, 4));
        // never shorter than the window's, whatever comes in
        assert_eq!(samples(0.5), (60, 4));
        assert_eq!(samples(f32::NAN), (60, 4));
        // an 8K photo of a 1080p viewfinder, 4 times taller
        assert_eq!(samples(4.0), (240, 16));
        assert_eq!(samples(3.2), (192, 13));
        // up to the workgroup memory the shader has
        assert_eq!(samples(8.0), (480, 32));
        assert_eq!(samples(20.0), (MAX_SAMPLE_COUNT, 32));
        // the tiles' guard band covers the samples and the filter
        assert_eq!(tile_reach(true, 4.0), 243);
    }

    #[test]
    fn the_shader_has_room_for_the_most_samples() {
        let shader = include_str!("shaders/contact_shadows.wgsl");
        assert!(shader.contains(&format!("const MAX_SAMPLE_COUNT: i32 = {MAX_SAMPLE_COUNT};")));
        assert!(shader.contains(&format!("const WAVE_SIZE: i32 = {WAVE_SIZE};")));
    }
}
