//! Soft sun shadows (experimental): the sun's shadow of every pixel of the
//! depth prepass, computed once per pixel in compute (shaders/soft_shadows.wgsl)
//! with a penumbra that widens with the distance between the shadow and what
//! casts it, as the sun's does in real life: percentage-closer soft shadows
//! (Fernando, "Percentage-Closer Soft Shadows", SIGGRAPH 2005). A search of
//! the shadow map around the point finds the casters and their mean
//! distance, the size of the light gives the penumbra from it, and a filter of
//! rotated taps of that radius takes the light; a bilateral blur then removes
//! the noise of the rotation. The shading reads the result with one load
//! (`soft_shadow` in common.wgsl) where its surface is the one of the
//! prepass, and keeps the filter of before elsewhere (blended faces in front
//! of it, water, reflections and probes).
//!
//! Beyond the Legacy Viewer, which softens its shadows in screen space by a
//! fixed pixel radius (`RenderShadowBlurSize`, with SSAO on only): here the
//! penumbra is in metres, grows with the distance of the caster, and is
//! never narrower than 1.5 cm, sharp where a shadow touches what casts it.
//! The light is 2.3 degrees across at a softness of 1 (the look of the first
//! soft shadows PR, #2288); the real sun, 0.53 degrees, is a softness of 0.23.
//! The search for casters reads the shadow atlas's minimum depths over blocks
//! of 4, 8 and 16 texels, built each frame by a small pass (`mm_build0`,
//! `mm_build`), so that no thin caster slips between the texels of a block.
//!
//! Off, nothing is created or drawn, and the shadows are those of before to
//! the pixel: the pipelines are made with the first frame that wants them,
//! the mask with the targets, and the casters' culling planes are those of
//! before ([`reach`] is 0).
//!
//! `AURORA_SOFT_SHADOWS=0|1|alt` overrides the setting for a comparison;
//! `alt` switches every second.

use crate::pipeline_cache::CachedPipelines;
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};

/// Tangent of the light's half angle per unit of softness: a softness of 1
/// is a light 2.3 degrees across, the real sun (0.53 degrees) about 0.23.
const LIGHT_TAN: f32 = 0.02;

/// The least radius of the filter, in metres (`FILTER_FLOOR` in
/// soft_shadows.wgsl): a shadow is sharp where it touches what casts it,
/// sharper than the 3 cm filter of before.
pub(crate) const FILTER_FLOOR: f32 = 0.015;

/// Radius of the light of a projector at a softness of 1, in metres: a
/// lamp has no size of its own, a small bulb gives the projectors' soft
/// shadows (`projector_soft` in common.wgsl), scaled by the softness.
pub(crate) const PROJECTOR_LIGHT_RADIUS: f32 = 0.02;

/// `AURORA_SOFT_PROJECTORS=0`: the projectors keep their 5-tap shadows
/// while the sun's are soft (to measure their cost apart).
pub(crate) fn projectors_soft() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("AURORA_SOFT_PROJECTORS").map_or(true, |v| v.trim() != "0"))
}

/// Taps of the filter in the window, and in a photo (more, for the same
/// noise under a larger enlargement of the blur).
const TAPS: f32 = 12.0;
const PHOTO_TAPS: f32 = 32.0;
/// Gathers (of 2 x 2 blocks) of the search for casters, besides the one of
/// the texels at the point itself.
const GATHERS: f32 = 8.0;

/// Pixels the mask of a pixel reads around it: the blur's 4 on each side and
/// the one more of the neighbours its normal is made from. A photo drawn in
/// tiles draws this much around each one (`tile_reach`; the viewer's guard
/// band of 128 pixels is far wider, it is the limit to keep).
pub(crate) const BLUR_RADIUS: u32 = 5;

/// Tangent of the light's half angle at `softness` (1: 2.3 degrees across).
pub(crate) fn light_tan(softness: f32) -> f32 {
    LIGHT_TAN * softness.max(0.0)
}

/// The widest filter and the radius of the search around a point, in metres:
/// half a metre per unit of softness, between 25 cm and 1 m.
pub(crate) fn search_radius(softness: f32) -> f32 {
    (0.5 * softness).clamp(0.25, 1.0)
}

/// Radius of the filter at a point whose casters are `gap` metres nearer the
/// light, at `softness`: the light's half width seen from there, never under
/// [`FILTER_FLOOR`] (contact hardening) nor over [`search_radius`]. Mirrored
/// by `cascade_soft` in soft_shadows.wgsl (the tests state the model).
#[cfg(test)]
pub(crate) fn filter_radius(gap: f32, softness: f32) -> f32 {
    (gap.max(0.0) * light_tan(softness)).clamp(FILTER_FLOOR, search_radius(softness).max(FILTER_FLOOR))
}

/// How far, in metres, the casters of a cascade are read around a point
/// (the shadow casters' culling, `shadow_cull::lookup_margin`): the search
/// when the light has a size, else the filter alone.
pub(crate) fn reach(on: bool, softness: f32) -> f32 {
    match (on, softness > 0.0) {
        (false, _) => 0.0,
        (true, true) => search_radius(softness),
        (true, false) => FILTER_FLOOR,
    }
}

/// How far, in pixels of the image, the mask of a pixel reads the pixels
/// around it. A photo drawn in tiles draws this much around each one; 0
/// when the soft shadows are off (`on`, the setting; `AURORA_SOFT_SHADOWS`
/// included).
pub fn tile_reach(on: bool) -> u32 {
    if Override::from_env().needed(on) { BLUR_RADIUS } else { 0 }
}

/// `AURORA_SOFT_SHADOWS`: the setting forced for a comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Override {
    None,
    Off,
    On,
    /// Off and on by turns, one second each (on in odd seconds).
    Alternate,
}

impl Override {
    pub(crate) fn parse(v: Option<&str>) -> Self {
        match v.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
            Some("0" | "off") => Override::Off,
            Some("1" | "on") => Override::On,
            Some("alt") => Override::Alternate,
            _ => Override::None,
        }
    }

    pub(crate) fn from_env() -> Self {
        static MODE: std::sync::OnceLock<Override> = std::sync::OnceLock::new();
        *MODE.get_or_init(|| Override::parse(std::env::var("AURORA_SOFT_SHADOWS").ok().as_deref()))
    }

    /// Whether the targets are kept for the setting `on`.
    pub(crate) fn needed(self, on: bool) -> bool {
        match self {
            Override::None => on,
            Override::Off => false,
            Override::On | Override::Alternate => true,
        }
    }

    /// Whether the frame drawn `secs` seconds into the session has them.
    pub(crate) fn on_now(self, on: bool, secs: u64) -> bool {
        match self {
            Override::Alternate => secs % 2 == 1,
            _ => self.needed(on),
        }
    }
}

/// Must match `Params` in soft_shadows.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ParamsU {
    cascade_vp: [[[f32; 4]; 4]; 3],
    inv_view_proj: [[f32; 4]; 4],
    camera: [f32; 4],
    splits: [f32; 4],
    light: [f32; 4],
    size: [u32; 4],
}

/// What the passes of a frame need that the frame knows (`SoftShadows::write`).
pub(crate) struct Frame {
    /// World to the clip space of each cascade.
    pub cascades: [Mat4; 3],
    /// The far distance of each cascade from the camera.
    pub splits: [f32; 3],
    /// The depth prepass's clip space to the world (jittered as it was drawn).
    pub inv_view_proj: Mat4,
    pub camera: Vec3,
    pub softness: f32,
    /// The frame's step of the noise (0: the same every frame).
    pub phase: u32,
    /// A photo: more taps.
    pub photo: bool,
    /// The part of the whole photo this frame draws (`photo_part`; the
    /// whole image otherwise): the noise is counted from the photo's corner,
    /// so that the tiles of a photo share one pattern without seams.
    pub tile: [f32; 4],
}

/// The pixel of the whole photo a tile's first pixel is (the noise's origin),
/// as GTAO's `pixel_noise` counts it (gtao.wgsl).
pub(crate) fn tile_origin(tile: [f32; 4], size: (u32, u32)) -> [u32; 2] {
    let at = |part: f32, whole: f32, size: u32| (part / whole.max(1e-6) * size as f32).round().max(0.0) as u32;
    [at(tile[0], tile[2], size.0), at(tile[1], tile[3], size.1)]
}

/// The pipelines, made with the first frame that wants the soft shadows.
pub(crate) struct SoftShadows {
    soft: wgpu::ComputePipeline,
    blur_h: wgpu::ComputePipeline,
    blur_v: wgpu::ComputePipeline,
    mm_build0: wgpu::ComputePipeline,
    mm_build: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    buffer: wgpu::Buffer,
    compare: wgpu::Sampler,
    point: wgpu::Sampler,
}

/// The mask of one window size and its bind groups.
pub(crate) struct Targets {
    size: (u32, u32),
    /// The depth prepass, then the pass writing the mask.
    soft: wgpu::BindGroup,
    /// The same reading the occlusion depth, which holds the blended faces
    /// opaque enough (`blend_in_ao_depth` in renderer.rs).
    soft_more: wgpu::BindGroup,
    blur_h: wgpu::BindGroup,
    blur_v: wgpu::BindGroup,
    /// The three levels of the atlas's minimum depths, each built from the
    /// one before, and the size of each.
    mm: [wgpu::BindGroup; 3],
    mm_size: [(u32, u32); 3],
    atlas_width: u32,
    /// The distance of the surface the mask was made for (an f16, high 16
    /// bits), the radius of the filter (6 bits) and the light let through (low
    /// 10) in an R32Uint; the shading reads the distance and the light.
    result: wgpu::TextureView,
    /// Copied out by the tests.
    #[cfg(test)]
    texture: wgpu::Texture,
}

const MASK_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Uint;
const MIN_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;
/// Texels of the atlas a block of the first level of minimum depths spans.
const MIN_BLOCK: u32 = 4;

impl Targets {
    pub(crate) fn result(&self) -> &wgpu::TextureView {
        &self.result
    }
}

impl SoftShadows {
    /// The masks are copied out by the tests only.
    #[cfg(test)]
    const COPY_OUT: wgpu::TextureUsages = wgpu::TextureUsages::COPY_SRC;
    #[cfg(not(test))]
    const COPY_OUT: wgpu::TextureUsages = wgpu::TextureUsages::empty();

    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("soft shadows"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/soft_shadows.wgsl").into()),
        });
        let depth = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Depth,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("soft shadows"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<ParamsU>() as u64),
                    },
                    count: None,
                },
                depth(1),
                depth(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: MASK_FORMAT,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 8,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: MIN_FORMAT,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
            ],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("soft shadows"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let make = |entry: &'static str| {
            device.create_compute_pipeline_cached(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&pl),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let sampler = |label: &str, linear: bool| {
            let filter = if linear {
                wgpu::FilterMode::Linear
            } else {
                wgpu::FilterMode::Nearest
            };
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some(label),
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                address_mode_w: wgpu::AddressMode::ClampToEdge,
                mag_filter: filter,
                min_filter: filter,
                mipmap_filter: wgpu::MipmapFilterMode::Nearest,
                compare: linear.then_some(wgpu::CompareFunction::LessEqual),
                ..Default::default()
            })
        };
        SoftShadows {
            soft: make("soft"),
            blur_h: make("blur_h"),
            blur_v: make("blur_v"),
            mm_build0: make("mm_build0"),
            mm_build: make("mm_build"),
            layout,
            buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("soft shadows params"),
                size: std::mem::size_of::<ParamsU>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            compare: sampler("soft shadows compare", true),
            point: sampler("soft shadows point", false),
        }
    }

    /// The mask for a window of `width` x `height`, reading the depth prepass
    /// `depth` (or `depth_more`, which holds the blended faces too) and the
    /// shadow atlas `shadow`. Two targets of 4 bytes a pixel: the pass writes
    /// the first, the blur goes through the second and back.
    pub(crate) fn targets(
        &self,
        device: &wgpu::Device,
        depth: &wgpu::TextureView,
        depth_more: &wgpu::TextureView,
        shadow: &wgpu::TextureView,
        atlas: (u32, u32),
        width: u32,
        height: u32,
    ) -> Targets {
        let size = (width.max(1), height.max(1));
        // minimum depths of the atlas: 4, 8 and 16 texels a block
        let mm_size: [(u32, u32); 3] = std::array::from_fn(|k| ((atlas.0 / (MIN_BLOCK << k)).max(1), (atlas.1 / (MIN_BLOCK << k)).max(1)));
        let mm_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("soft shadows minimum depths"),
            size: wgpu::Extent3d {
                width: mm_size[0].0,
                height: mm_size[0].1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 3,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: MIN_FORMAT,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let level = |k: u32| {
            mm_texture.create_view(&wgpu::TextureViewDescriptor {
                base_mip_level: k,
                mip_level_count: Some(1),
                ..Default::default()
            })
        };
        let mm_all = mm_texture.create_view(&Default::default());
        // stand-ins for the bindings a pass does not use
        let tiny = |usage| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("soft shadows stand-in"),
                    size: wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: MIN_FORMAT,
                    usage,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let (stand_read, stand_write) = (
            tiny(wgpu::TextureUsages::TEXTURE_BINDING),
            tiny(wgpu::TextureUsages::STORAGE_BINDING),
        );
        let mask = |label: &str| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: size.0,
                    height: size.1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: MASK_FORMAT,
                usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING | Self::COPY_OUT,
                view_formats: &[],
            })
        };
        let (a_texture, b_texture) = (mask("soft shadows"), mask("soft shadows blur"));
        let (a, b) = (
            a_texture.create_view(&Default::default()),
            b_texture.create_view(&Default::default()),
        );
        let group = |label: &str,
                     depth: &wgpu::TextureView,
                     read: &wgpu::TextureView,
                     write: &wgpu::TextureView,
                     mm_read: &wgpu::TextureView,
                     mm_write: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(depth),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(shadow),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Sampler(&self.compare),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::Sampler(&self.point),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: wgpu::BindingResource::TextureView(read),
                    },
                    wgpu::BindGroupEntry {
                        binding: 6,
                        resource: wgpu::BindingResource::TextureView(write),
                    },
                    wgpu::BindGroupEntry {
                        binding: 7,
                        resource: wgpu::BindingResource::TextureView(mm_read),
                    },
                    wgpu::BindGroupEntry {
                        binding: 8,
                        resource: wgpu::BindingResource::TextureView(mm_write),
                    },
                ],
            })
        };
        Targets {
            size,
            // the pass writes `a` (reading `b`, which it does not use)
            soft: group("soft shadows", depth, &b, &a, &mm_all, &stand_write),
            soft_more: group("soft shadows (more)", depth_more, &b, &a, &mm_all, &stand_write),
            blur_h: group("soft shadows blur h", depth, &a, &b, &mm_all, &stand_write),
            blur_v: group("soft shadows blur v", depth, &b, &a, &mm_all, &stand_write),
            mm: [
                group("soft shadows min 0", depth, &b, &a, &stand_read, &level(0)),
                group("soft shadows min 1", depth, &b, &a, &level(0), &level(1)),
                group("soft shadows min 2", depth, &b, &a, &level(1), &level(2)),
            ],
            mm_size,
            atlas_width: atlas.0,
            result: a,
            #[cfg(test)]
            texture: a_texture,
        }
    }

    /// The constants of the frame, before the encoder runs.
    pub(crate) fn write(&self, queue: &wgpu::Queue, t: &Targets, f: &Frame) {
        let origin = tile_origin(f.tile, t.size);
        let u = ParamsU {
            cascade_vp: f.cascades.map(|m| m.to_cols_array_2d()),
            inv_view_proj: f.inv_view_proj.to_cols_array_2d(),
            camera: f.camera.extend(pixel_angle(f.inv_view_proj, t.size.1)).to_array(),
            splits: [f.splits[0], f.splits[1], f.splits[2], (f.phase % 64) as f32],
            light: [
                light_tan(f.softness),
                search_radius(f.softness),
                if f.photo { PHOTO_TAPS } else { TAPS },
                GATHERS,
            ],
            size: [t.size.0, t.size.1, origin[0], origin[1]],
        };
        queue.write_buffer(&self.buffer, 0, bytemuck::bytes_of(&u));
    }

    /// The dispatches (the atlas's minimum depths, the mask, its two blurs),
    /// after the depth prepass and the shadow cascades (`more`: after the
    /// blended faces were drawn into the occlusion depth). No barrier is
    /// needed between them: each reads the target the one before wrote,
    /// which the pass tracks.
    pub(crate) fn encode(&self, encoder: &mut wgpu::CommandEncoder, t: &Targets, more: bool) {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("soft shadows"),
            timestamp_writes: None,
        });
        let (x, y) = (t.size.0.div_ceil(8), t.size.1.div_ceil(8));
        for (k, group) in t.mm.iter().enumerate() {
            pass.set_pipeline(if k == 0 { &self.mm_build0 } else { &self.mm_build });
            pass.set_bind_group(0, group, &[]);
            // (the cascades' rows only: the atlas's own width of them)
            let rows = (t.mm_size[k].1).min(t.atlas_width / (MIN_BLOCK << k));
            pass.dispatch_workgroups(t.mm_size[k].0.div_ceil(8), rows.div_ceil(8), 1);
        }
        pass.set_pipeline(&self.soft);
        pass.set_bind_group(0, if more { &t.soft_more } else { &t.soft }, &[]);
        pass.dispatch_workgroups(x, y, 1);
        pass.set_pipeline(&self.blur_h);
        pass.set_bind_group(0, &t.blur_h, &[]);
        pass.dispatch_workgroups(x, y, 1);
        pass.set_pipeline(&self.blur_v);
        pass.set_bind_group(0, &t.blur_v, &[]);
        pass.dispatch_workgroups(x, y, 1);
    }
}

/// The angle between the rays of two neighbouring pixels at the centre of
/// the view, which the blur turns into metres.
fn pixel_angle(inv_view_proj: Mat4, height: u32) -> f32 {
    // in double precision: far from the origin (a skybox) the camera's
    // translation would swamp the two points' difference in single
    let inv = inv_view_proj.as_dmat4();
    let ray = |y: f64| {
        let a = inv.project_point3(glam::DVec3::new(0.0, y, 1.0));
        let b = inv.project_point3(glam::DVec3::new(0.0, y, 0.5));
        (a - b).normalize()
    };
    ray(0.0).angle_between(ray(2.0 / f64::from(height.max(1)))) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_light_is_2_3_degrees_across_at_a_softness_of_one() {
        // tan of the half angle 0.02: 2.3 degrees across, the first soft
        // shadows PR's look; the real sun (0.53 degrees) is about 0.23
        assert_eq!(light_tan(1.0), 0.02);
        let degrees = |t: f32| 2.0 * t.atan().to_degrees();
        assert!((degrees(light_tan(1.0)) - 2.29).abs() < 0.01);
        assert!((degrees(light_tan(0.23)) - 0.53).abs() < 0.01);
        // proportional to the softness, none at 0 or below
        assert!((light_tan(2.0) - 2.0 * light_tan(1.0)).abs() < 1e-7);
        assert_eq!(light_tan(0.0), 0.0);
        assert_eq!(light_tan(-1.0), 0.0);
    }

    #[test]
    fn the_penumbra_is_sharp_at_the_contact_and_grows_with_the_gap_up_to_the_search() {
        for softness in [0.0, 0.5, 1.0, 2.0] {
            assert_eq!(filter_radius(0.0, softness), FILTER_FLOOR);
            let mut before = 0.0;
            for gap in [0.0, 0.1, 0.5, 1.0, 3.0, 10.0, 30.0, 100.0, 1.0e4] {
                let r = filter_radius(gap, softness);
                assert!(r >= before, "monotonic at {gap} m, softness {softness}");
                assert!((FILTER_FLOOR..=search_radius(softness)).contains(&r));
                before = r;
            }
            let widest = if softness > 0.0 { search_radius(softness) } else { FILTER_FLOOR };
            assert_eq!(filter_radius(1.0e4, softness), widest);
        }
        // the search: half a metre a unit of softness, from 25 cm to 1 m
        assert_eq!(search_radius(0.2), 0.25);
        assert_eq!(search_radius(1.0), 0.5);
        assert_eq!(search_radius(2.0), 1.0);
        assert_eq!(search_radius(3.0), 1.0);
        // no light size: always the floor
        assert_eq!(filter_radius(50.0, 0.0), FILTER_FLOOR);
        // a softer light widens it, at the same distance
        assert!(filter_radius(10.0, 2.0) > filter_radius(10.0, 1.0));
        // a caster 5 m along the light at a softness of 1: 10 cm (the floor
        // by max, not in quadrature); under 75 cm the floor holds
        assert!((filter_radius(5.0, 1.0) - 0.10).abs() < 1e-6);
        assert_eq!(filter_radius(0.5, 1.0), FILTER_FLOOR);
        // the real sun (0.23), a caster 10 m up: 4.6 cm
        assert!((filter_radius(10.0, 0.23) - 0.046).abs() < 1e-4);
    }

    #[test]
    fn the_casters_reach_is_zero_without_soft_shadows() {
        assert_eq!(reach(false, 1.0), 0.0);
        assert_eq!(reach(false, 4.0), 0.0);
        assert_eq!(reach(true, 1.0), 0.5);
        assert_eq!(reach(true, 2.0), 1.0);
        assert_eq!(reach(true, 0.0), FILTER_FLOOR);
    }

    /// Without soft shadows the casters' culling planes are those of before,
    /// to the bit: the margin of a lookup gains `reach` and nothing else.
    #[test]
    fn the_culling_planes_are_unchanged_without_soft_shadows() {
        use crate::shadow_cull::{ReceiverView, cascade_caster_planes, lookup_margin};
        let view = ReceiverView {
            inv_view: Mat4::from_translation(Vec3::new(10.0, -4.0, 25.0)) * Mat4::from_rotation_x(1.2),
            tan_x: 1.2,
            tan_y: 0.7,
            near: 0.1,
        };
        let light = Vec3::new(0.3, -0.5, -0.8).normalize();
        let (near, far, texel) = ([0.1, 12.0, 45.0], [14.0, 50.0, 160.0], [0.007, 0.024, 0.08]);
        let before = cascade_caster_planes(&view, light, near, far, texel, reach(false, 1.0));
        let zero = cascade_caster_planes(&view, light, near, far, texel, 0.0);
        assert_eq!(before, zero);
        for (k, &t) in texel.iter().enumerate() {
            assert_eq!(lookup_margin(k, t, 0.0), 6.0 * t + 0.02 + 0.04 * k as f32 + 0.05);
        }
        // and the planes move out by the search when it runs
        let wide = cascade_caster_planes(&view, light, near, far, texel, reach(true, 1.0));
        assert_ne!(wide, zero);
    }

    #[test]
    fn the_shader_keeps_the_constants_of_the_shading() {
        let shader = include_str!("shaders/soft_shadows.wgsl");
        let common = include_str!("shaders/common.wgsl");
        let line = format!("const FILTER_FLOOR: f32 = {};", FILTER_FLOOR);
        assert!(shader.contains(&line), "soft_shadows.wgsl: {line}");
        // the normal offset and the receiver bias of `cascade_shadow`
        assert!(shader.contains("n * (0.02 + 0.04 * f32(cascade))"));
        assert!(common.contains("n * (0.02 + 0.04 * f32(cascade))"));
        assert!(shader.contains("2.0 * texel * radius * 2.0 * depth_per_metre"));
        assert!(common.contains("2.0 * texel * radius * 2.0 * depth_per_metre"));
        // the atlas addressing of the non-square atlas (`tile_scale`)
        assert!(shader.contains("vec2<f32>(0.5, dims.x / (2.0 * dims.y))"));
        // the cascade blend band and the far fade
        assert!(shader.contains("(end - start) * 0.15") && common.contains("(end - start) * 0.15"));
        assert!(shader.contains("(splits.z * 0.1)") && common.contains("(splits.z * 0.1)"));
    }

    /// `classic_shadowed` and `shadow_lift` of common.wgsl, `lift` being
    /// `frame.soft.y` (1 - the shadow intensity).
    fn classic_shadowed(nl: f32, shadow: f32, lift: f32) -> f32 {
        let lit = nl.min(shadow);
        lit + (nl - lit) * lift
    }

    fn shadow_lift(s: f32, lift: f32) -> f32 {
        s + (1.0 - s) * lift
    }

    #[test]
    fn the_shadow_intensity_lifts_the_shadow_and_leaves_full_intensity_untouched() {
        for nl in [0.0_f32, 0.2, 0.57, 1.0] {
            for shadow in [0.0_f32, 0.1, 0.5, 0.9, 1.0] {
                // full intensity: the Legacy min(N.L, shadow), to the bit
                assert_eq!(classic_shadowed(nl, shadow, 0.0).to_bits(), nl.min(shadow).to_bits());
                assert_eq!(shadow_lift(shadow, 0.0).to_bits(), shadow.to_bits());
                // no intensity: no shadow at all
                assert_eq!(classic_shadowed(nl, shadow, 1.0), nl);
                assert_eq!(shadow_lift(shadow, 1.0), 1.0);
                // linear between
                let (a, b) = (classic_shadowed(nl, shadow, 0.25), classic_shadowed(nl, shadow, 0.75));
                let (lit, lifted) = (nl.min(shadow), nl);
                assert!((a - (lit + 0.25 * (lifted - lit))).abs() < 1e-6 && (b - (lit + 0.75 * (lifted - lit))).abs() < 1e-6);
                assert!((shadow_lift(shadow, 0.5) - (shadow + 0.5 * (1.0 - shadow))).abs() < 1e-6);
            }
        }
        // the shader holds the same formulas, and every consumer of a sun
        // shadow takes the lift: classic PBR, classic highlights, water
        let common = include_str!("shaders/common.wgsl");
        assert!(common.contains("return lit + (nl - lit) * frame.soft.y;"));
        assert!(common.contains("return s + (1.0 - s) * frame.soft.y;"));
        assert!(common.contains("* shadow_lift(shadow), vec3<f32>(0.0), vec3<f32>(10.0)"));
        assert!(common.contains("shadow_lift(shadow_factor(pos,normal"));
        assert!(include_str!("shaders/env.wgsl").contains("shadow_lift(shadow_factor(in.world_pos"));
        assert!(include_str!("shaders/object.wgsl").contains("shadow_lift(sh), ll_sun_linear(a)"));
    }

    #[test]
    fn the_pixel_angle_holds_far_from_the_origin() {
        // a projection of 60 degrees, the camera at the origin and at skybox
        // altitude: the angle between neighbouring pixels' rays is the same
        let fov = 60.0_f64.to_radians();
        let height = 1080_u32;
        let expected = 2.0 * (fov / 2.0).tan() / f64::from(height);
        for z in [0.0_f64, 4096.0] {
            let proj = glam::DMat4::perspective_infinite_reverse_rh(fov, 16.0 / 9.0, 0.1);
            let view = glam::DMat4::look_at_rh(
                glam::DVec3::new(10.0, -20.0, z),
                glam::DVec3::new(10.0, 50.0, z - 5.0),
                glam::DVec3::Z,
            );
            let inverse = (proj * view).inverse().as_mat4();
            let angle = f64::from(pixel_angle(inverse, height));
            assert!((angle / expected - 1.0).abs() < 0.01, "z = {z}: {angle} against {expected}");
        }
    }

    #[test]
    fn a_tiles_noise_starts_at_its_pixel_of_the_whole_photo() {
        // the whole image, a window: the origin
        assert_eq!(tile_origin([0.0, 0.0, 1.0, 1.0], (1920, 1080)), [0, 0]);
        // the middle third of a 3 x 3 split, drawn at 640 x 360 a tile
        assert_eq!(tile_origin([1.0 / 3.0, 1.0 / 3.0, 1.0 / 3.0, 1.0 / 3.0], (640, 360)), [640, 360]);
        assert_eq!(tile_origin([2.0 / 3.0, 0.0, 1.0 / 3.0, 1.0 / 3.0], (640, 360)), [1280, 0]);
    }

    #[test]
    fn the_shader_texts_hold_the_surface_and_lift_rules() {
        let common = include_str!("shaders/common.wgsl");
        // a blended face outside the occlusion depth keeps the filter of before
        assert!(common.contains("if (frame.soft.x < 0.5 || !screen_surface) {"));
        // the terrain's wet glint takes the lift on a classic sky only (the
        // others have it from `sun_shadow`)
        assert!(include_str!("shaders/env.wgsl").contains("*select(sh, shadow_lift(sh), is_classic_sky()))*wet;"));
        // the noise is counted from the photo's corner
        assert!(include_str!("shaders/soft_shadows.wgsl").contains("id.xy + p.size.zw"));
    }

    #[test]
    fn the_params_match_the_shader_layout() {
        // Params in soft_shadows.wgsl: 3 x 64 + 64 + 4 x 16 bytes
        assert_eq!(std::mem::size_of::<ParamsU>(), 3 * 64 + 64 + 4 * 16);
        let shader = include_str!("shaders/soft_shadows.wgsl");
        assert!(shader.contains("cascade_vp: array<mat4x4<f32>, 3>,"));
    }

    #[test]
    fn the_override_follows_the_variable() {
        assert_eq!(Override::parse(None), Override::None);
        assert_eq!(Override::parse(Some("1")), Override::On);
        assert_eq!(Override::parse(Some("OFF")), Override::Off);
        assert_eq!(Override::parse(Some("alt")), Override::Alternate);
        assert!(Override::None.needed(true) && !Override::None.needed(false));
        assert!(!Override::Off.needed(true));
        assert!(Override::On.needed(false));
        assert!(Override::Alternate.on_now(false, 1) && !Override::Alternate.on_now(true, 2));
    }

    /// What the drivers get: Vulkan's SPIR-V and Metal's MSL, one entry
    /// point at a time (as wgpu writes them). The Metal compile is not run
    /// here: `textureGather` of a depth texture with a plain sampler and an
    /// R32Uint storage write are what the translation has to accept.
    #[test]
    fn the_shader_translates_to_spirv_and_msl() {
        let source = include_str!("shaders/soft_shadows.wgsl");
        let module = naga::front::wgsl::parse_str(source).unwrap_or_else(|e| panic!("{}", e.emit_to_string(source)));
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::SHADER_FLOAT16_IN_FLOAT32,
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(source)));
        assert_eq!(module.entry_points.len(), 5);
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

    /// The mask on a real device: a ground plane seen from above under a
    /// slab, in a shadow atlas made on the CPU. Lit where the sun reaches,
    /// dark under the slab, and the edge's width grows with the height of the
    /// slab and with the softness. Skipped without an adapter.
    #[test]
    fn gpu_penumbra_grows_with_the_height_of_the_caster_and_the_softness() {
        let Some(gpu) = GpuScene::new() else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let low = gpu.edge(0.5, 2.0);
        let high = gpu.edge(3.0, 2.0);
        let pinpoint = gpu.edge(3.0, 0.0);
        let mid = gpu.edge(3.0, 1.0);
        eprintln!("edge widths (m): low {low:?} high {high:?} pinpoint {pinpoint:?} mid {mid:?}");
        for (lit, umbra, _) in [low, high, pinpoint, mid] {
            assert!(lit > 0.98, "far from the slab the ground is lit: {lit}");
            assert!(umbra < 0.05, "under the slab it is dark: {umbra}");
        }
        // contact hardening: the caster 0.5 m up has a narrower penumbra than
        // the one 3 m up, whatever the light
        assert!(high.2 > low.2 * 1.3, "higher casters give wider penumbrae: {} vs {}", high.2, low.2);
        // a larger light gives a wider one, a pinpoint light the 1.5 cm floor
        assert!(high.2 > mid.2 * 1.3, "a larger light gives a wider one: {} vs {}", high.2, mid.2);
        assert!(
            mid.2 > pinpoint.2 * 1.3,
            "a light with a size beats none: {} vs {}",
            mid.2,
            pinpoint.2
        );
    }

    /// A ground plane seen from 10 m above, a 2 m square slab at some
    /// height above it, a sun 45 degrees up, on a 1024 x 512 shadow atlas
    /// (the first tile holds the cascade; a tile is as high as the atlas).
    struct GpuScene {
        device: wgpu::Device,
        queue: wgpu::Queue,
        soft: SoftShadows,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    const VIEW: u32 = 512;
    const TILE: u32 = 512;

    impl GpuScene {
        fn new() -> Option<Self> {
            let lock = crate::test_gpu::lock();
            let instance = wgpu::Instance::default();
            let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
            let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).ok()?;
            let soft = SoftShadows::new(&device);
            Some(GpuScene {
                device,
                queue,
                soft,
                _lock: lock,
            })
        }

        /// Fill a depth texture with `values` (a depth texture cannot be a copy
        /// destination): through an R32Float texture and a full-screen pass
        /// that writes the depth.
        fn upload_depth(&self, target: &wgpu::Texture, values: &[f32], width: u32, height: u32) {
            let (device, queue) = (&self.device, &self.queue);
            let extent = wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            };
            let source = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("test depth values"),
                size: extent,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R32Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            queue.write_texture(
                source.as_image_copy(),
                bytemuck::cast_slice(values),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(width * 4),
                    rows_per_image: Some(height),
                },
                extent,
            );
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("test depth fill"),
                source: wgpu::ShaderSource::Wgsl(
                    "@group(0) @binding(0) var values: texture_2d<f32>;
                     @vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
                         let q = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
                         return vec4<f32>(q * 2.0 - 1.0, 0.0, 1.0);
                     }
                     @fragment fn fs(@builtin(position) p: vec4<f32>) -> @builtin(frag_depth) f32 {
                         return textureLoad(values, vec2<i32>(p.xy), 0).x;
                     }"
                    .into(),
                ),
            });
            let pipeline = device.create_render_pipeline_cached(&wgpu::RenderPipelineDescriptor {
                label: Some("test depth fill"),
                layout: None,
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::Always),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[],
                }),
                multiview_mask: None,
                cache: None,
            });
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &pipeline.get_bind_group_layout(0),
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&source.create_view(&Default::default())),
                }],
            });
            let view = target.create_view(&Default::default());
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &group, &[]);
                pass.draw(0..3, 0..1);
            }
            queue.submit([encoder.finish()]);
        }

        /// The ground's light along a row through the shadow's edge for a slab
        /// `height` metres up: (light far past the edge, light under the slab,
        /// width in metres between 90 % and 10 % of the light).
        fn edge(&self, height: f32, softness: f32) -> (f32, f32, f32) {
            let (device, queue) = (&self.device, &self.queue);
            // the sun travels towards +x and down, 45 degrees
            let dir = Vec3::new(1.0, 0.0, -1.0).normalize();
            let radius = 6.0;
            let eye = -dir * (radius + 50.0);
            let view = glam::camera::rh::view::look_at_mat4(eye, Vec3::ZERO, Vec3::Z);
            let proj = glam::camera::rh::proj::directx::orthographic(-radius, radius, -radius, radius, 0.1, 2.0 * radius + 100.0);
            let cascade = proj * view;
            let inverse = cascade.inverse();
            // the atlas: two tiles wide, one tall; the slab's depth in the first
            let (aw, ah) = (2 * TILE, TILE);
            let mut atlas = vec![1.0f32; (aw * ah) as usize];
            for ty in 0..TILE {
                for tx in 0..TILE {
                    let ndc = glam::Vec2::new(
                        (tx as f32 + 0.5) / TILE as f32 * 2.0 - 1.0,
                        1.0 - (ty as f32 + 0.5) / TILE as f32 * 2.0,
                    );
                    let near = inverse * glam::Vec4::new(ndc.x, ndc.y, 0.0, 1.0);
                    let far = inverse * glam::Vec4::new(ndc.x, ndc.y, 1.0, 1.0);
                    let (near, far) = (near.truncate() / near.w, far.truncate() / far.w);
                    let t = (height - near.z) / (far.z - near.z);
                    let p = near + (far - near) * t;
                    if p.x.abs() <= 1.0 && p.y.abs() <= 1.0 {
                        let clip = cascade * p.extend(1.0);
                        atlas[(ty * aw + tx) as usize] = clip.z / clip.w;
                    }
                }
            }
            let shadow = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("test atlas"),
                size: wgpu::Extent3d {
                    width: aw,
                    height: ah,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth32Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            self.upload_depth(&shadow, &atlas, aw, ah);
            // the camera 10 m over the origin looking down, reversed infinite depth
            let cam = glam::camera::rh::view::look_at_mat4(Vec3::new(0.0, 0.0, 10.0), Vec3::ZERO, Vec3::Y);
            let cam_proj = glam::camera::rh::proj::directx::perspective_infinite_reverse(50.0_f32.to_radians(), 1.0, 0.1);
            let vp = cam_proj * cam;
            let inv_vp = vp.inverse();
            // the ground (z = 0) seen from above, and the slab's top where it hides it
            let mut depth = vec![0.0f32; (VIEW * VIEW) as usize];
            for y in 0..VIEW {
                for x in 0..VIEW {
                    let ndc = glam::Vec2::new(
                        (x as f32 + 0.5) / VIEW as f32 * 2.0 - 1.0,
                        1.0 - (y as f32 + 0.5) / VIEW as f32 * 2.0,
                    );
                    let a = inv_vp * glam::Vec4::new(ndc.x, ndc.y, 1.0, 1.0);
                    let b = inv_vp * glam::Vec4::new(ndc.x, ndc.y, 0.001, 1.0);
                    let (a, b) = (a.truncate() / a.w, b.truncate() / b.w);
                    let point_at = |z: f32| a + (b - a) * ((z - a.z) / (b.z - a.z));
                    let slab = point_at(height);
                    let p = if slab.x.abs() <= 1.0 && slab.y.abs() <= 1.0 {
                        slab
                    } else {
                        point_at(0.0)
                    };
                    let clip = vp * p.extend(1.0);
                    depth[(y * VIEW + x) as usize] = clip.z / clip.w;
                }
            }
            let depth_tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("test depth"),
                size: wgpu::Extent3d {
                    width: VIEW,
                    height: VIEW,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth32Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            self.upload_depth(&depth_tex, &depth, VIEW, VIEW);
            let (shadow_view, depth_view) = (shadow.create_view(&Default::default()), depth_tex.create_view(&Default::default()));
            let targets = self
                .soft
                .targets(device, &depth_view, &depth_view, &shadow_view, (aw, ah), VIEW, VIEW);
            let mut splits = [60.0; 3];
            splits[0] = 20.0;
            splits[1] = 40.0;
            self.soft.write(
                queue,
                &targets,
                &Frame {
                    cascades: [cascade; 3],
                    splits,
                    inv_view_proj: inv_vp,
                    camera: Vec3::new(0.0, 0.0, 10.0),
                    softness,
                    phase: 0,
                    photo: false,
                    tile: [0.0, 0.0, 1.0, 1.0],
                },
            );
            let mut encoder = device.create_command_encoder(&Default::default());
            self.soft.encode(&mut encoder, &targets, false);
            let bytes_per_row = (VIEW * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
            let readback = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("soft readback"),
                size: crate::readback::readback_size(u64::from(bytes_per_row * VIEW)),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &targets.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(bytes_per_row),
                        rows_per_image: Some(VIEW),
                    },
                },
                wgpu::Extent3d {
                    width: VIEW,
                    height: VIEW,
                    depth_or_array_layers: 1,
                },
            );
            queue.submit([encoder.finish()]);
            let slice = readback.slice(..);
            slice.map_async(wgpu::MapMode::Read, |r| r.expect("map"));
            device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
            let data = slice.get_mapped_range().expect("mapped");
            // the middle row: the ground's light along x (the image's x), with
            // the ground seen from 10 m over 50 degrees, 9.3 m wide
            let row = VIEW / 2;
            let visible = |x: u32| {
                let at = (row * bytes_per_row + x * 4) as usize;
                let word = u32::from_le_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]]);
                (word & 1023) as f32 / 1023.0
            };
            let metres = 2.0 * 10.0 * (25.0_f32.to_radians()).tan() / VIEW as f32;
            // the slab spans x in -1..1 (its top hides the ground), the shadow
            // of a 45 degree sun ends at 1 + height: look just past the slab
            let edge_x = 1.0 + height;
            let pixel_of = |x: f32| (VIEW as f32 / 2.0 + x / metres) as u32;
            let lit = visible(pixel_of(edge_x + 0.5));
            // (past the slab's own top, which hides the ground for x < 1)
            let umbra = visible(pixel_of((edge_x - 0.3).max(1.1)));
            let (mut hi, mut lo) = (None, None);
            for x in pixel_of((edge_x - 0.5).max(1.1))..pixel_of(edge_x + 0.5) {
                let v = visible(x);
                if v >= 0.1 && lo.is_none() {
                    lo = Some(x);
                }
                if v >= 0.9 && lo.is_some() && hi.is_none() {
                    hi = Some(x);
                }
            }
            let width = match (lo, hi) {
                (Some(lo), Some(hi)) => (hi - lo) as f32 * metres,
                _ => 0.0,
            };
            (lit, umbra, width)
        }
    }
}
