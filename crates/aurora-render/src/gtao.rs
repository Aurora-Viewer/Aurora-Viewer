//! Ground-truth ambient occlusion (GTAO), the option beside the Legacy
//! Viewer's screen-space occlusion (shaders/ssao.wgsl): Intel's XeGTAO
//! (MIT licence, see NOTICE.md) in four compute steps from the depth
//! prepass — depth levels, the occlusion with the depth edges, then
//! XeGTAO's edge-aware blur (shaders/gtao.wgsl). Its result goes where the
//! Legacy occlusion goes (`screen_ao` in common.wgsl), so the method only
//! changes the occlusion term.
//!
//! `AURORA_AO=legacy|gtao|alt` overrides the method of the settings for a
//! comparison; `alt` switches every second, both methods kept ready;
//! `gtao:<0-3>` and `alt:<0-3>` also choose GTAO's quality.

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
/// are slices 1 / 2 / 3 / 9 with 2 / 2 / 3 / 3 steps; with no temporal
/// accumulation to spread their noise over frames, low takes two slices at
/// half resolution and the levels blur more where they sample less.
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

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GtaoU {
    proj: [f32; 4],
    depth: [f32; 4],
    radius: [f32; 4],
    quality: [f32; 4],
    misc: [f32; 4],
}

/// The constants of a frame drawn with `proj` into targets of `size`.
fn uniforms(proj: Mat4, size: (u32, u32), q: Quality) -> GtaoU {
    GtaoU {
        // scales and off-centre terms (a photo tile's projection has them)
        proj: [proj.x_axis.x, proj.y_axis.y, proj.z_axis.x, proj.z_axis.y],
        // reversed infinite depth: the buffer holds P32 / (view z) - P22
        depth: [proj.w_axis.z, proj.z_axis.z, size.0 as f32, size.1 as f32],
        radius: [EFFECT_RADIUS, FALLOFF_RANGE, SAMPLE_DISTRIBUTION_POWER, THIN_OCCLUDER_COMPENSATION],
        quality: [FINAL_VALUE_POWER, DEPTH_MIP_SAMPLING_OFFSET, q.slices as f32, q.steps as f32],
        misc: [DENOISE_BLUR_BETA, if q.half_res { 2.0 } else { 1.0 }, 0.0, 0.0],
    }
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
    denoise: (wgpu::ComputePipeline, wgpu::BindGroupLayout),
    denoise_final: (wgpu::ComputePipeline, wgpu::BindGroupLayout),
    buffer: wgpu::Buffer,
}

/// The targets of one window size and quality, with their bind groups.
pub(crate) struct Targets {
    quality: Quality,
    size: (u32, u32),
    mips: Vec<(u32, u32)>,
    prefilter0: wgpu::BindGroup,
    prefilter: Vec<wgpu::BindGroup>,
    main: wgpu::BindGroup,
    /// In order; the last one writes `result`.
    denoise: Vec<(wgpu::BindGroup, bool)>,
    /// The visibility in `.r`, sampled by the shading.
    pub result: wgpu::TextureView,
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
const AO_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;
const EDGES_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const RESULT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

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
            buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("gtao params"),
                size: std::mem::size_of::<GtaoU>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
        }
    }

    /// Targets for a window of `width` x `height` reading `depth` (the
    /// single-sample depth prepass).
    pub(crate) fn targets(&self, device: &wgpu::Device, depth: &wgpu::TextureView, width: u32, height: u32, level: u8) -> Targets {
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
        let ao = [view(texture("gtao a", AO_FORMAT, 1)), view(texture("gtao b", AO_FORMAT, 1))];
        let edges = view(texture("gtao edges", EDGES_FORMAT, 1));
        let result = view(texture("gtao", RESULT_FORMAT, 1));
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
        let main = group(&self.main.1, &[(4, &all_mips), (5, &ao[0]), (6, &edges)]);
        // the passes read a and b by turns; the last one writes the result
        let passes = q.denoise_passes.max(1);
        let denoise = (0..passes)
            .map(|i| {
                let (src, dst) = (&ao[i as usize % 2], &ao[(i as usize + 1) % 2]);
                if i + 1 == passes {
                    (group(&self.denoise_final.1, &[(7, src), (8, &edges), (9, &result)]), true)
                } else {
                    (group(&self.denoise.1, &[(5, dst), (7, src), (8, &edges)]), false)
                }
            })
            .collect();
        let mips = (0..mip_count).map(|l| ((w >> l).max(1), (h >> l).max(1))).collect();
        Targets {
            quality: q,
            size: (w, h),
            mips,
            prefilter0,
            prefilter,
            main,
            denoise,
            result,
        }
    }

    /// The constants of this frame (before the encoder runs).
    pub(crate) fn write(&self, queue: &wgpu::Queue, t: &Targets, proj: Mat4) {
        queue.write_buffer(&self.buffer, 0, bytemuck::bytes_of(&uniforms(proj, t.size, t.quality)));
    }

    /// The passes, after the depth prepass.
    pub(crate) fn encode(&self, encoder: &mut wgpu::CommandEncoder, t: &Targets) {
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
        pass.set_pipeline(&self.main.0);
        pass.set_bind_group(0, &t.main, &[]);
        dispatch(&mut pass, groups(t.size));
        for (bg, last) in &t.denoise {
            pass.set_pipeline(if *last { &self.denoise_final.0 } else { &self.denoise.0 });
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

    /// The view depth the shader rebuilds from the projection's terms is the
    /// distance in front of the camera, and the position matches the point.
    #[test]
    fn uniforms_rebuild_view_positions() {
        let proj = glam::camera::rh::proj::directx::perspective_infinite_reverse(1.0, 1.6, 0.1);
        let u = uniforms(proj, (1600, 1000), quality(2));
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
