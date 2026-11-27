//! Aurora's sun and moon shafts (the Legacy Viewer has none): the light of
//! the sky around the sun, or the moon by night, streaked from it across
//! the image and cut by what stands in front of it, as a post process
//! (Mitchell, "Volumetric Light Scattering as a Post-Process", GPU Gems 3,
//! chapter 13). The sky's pixels near the light are kept at half resolution
//! (post.wgsl `fs_shafts_mask`, the depth's empty pixels), blurred toward
//! the light's place on the image in [`PASSES`] passes of [`TAPS`] taps
//! (`fs_shafts_blur`), and added to the image before the bloom and the tone
//! mapping (`fs_shafts_composite`; their GPU time is the bloom's). The taps
//! scale the pixel's offset from the light: evenly in its logarithm, each
//! pass's steps a [`TAPS`]th of the one before, so that 24 taps give 512
//! even steps. Even steps along the way instead left a ghost of each bright
//! gap at every tap of the first pass, further apart away from the light
//! (seen in the demo's forest, 2026-11-27). No medium in the air: the
//! shafts show where the image shows the light, without the veil a fog
//! thick enough for its own rays would lay over the scene (the volumetric
//! rays tried first, through the fog's froxels, made a veil or blots
//! around the objects, 2026-11-27). Its targets exist only while the
//! effect is on: off, it costs nothing.

use crate::pipeline_cache::CachedPipelines;
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3, Vec4Swizzles};

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// Blur passes, and the taps of each (SHAFT_TAPS in post.wgsl).
pub(crate) const PASSES: usize = 3;
pub(crate) const TAPS: u32 = 8;
/// How far from the light the sky's pixels feed the shafts (in heights of
/// the image): the glow around the sun, not the whole of it (at 0.45, the
/// bright sky above and below the sun laid a band of light down a pillar
/// beside it, which read as a vertical ray: human report, 2026-11-27).
const MASK_RADIUS: f32 = 0.25;
/// The share of the shafts laid over an object in front of the sky: the
/// light that spills over its edge (the mean along the way to the light)
/// stays a faint glare instead of a bright band down it. Shortening the
/// shafts for it instead (a decay along the way) kept the shape of every
/// gap between the trunks of a forest, streaks running straight up the
/// image (demo forest at sunrise, 2026-11-27).
const ON_OBJECTS: f32 = 0.4;
/// The brightest a sky pixel counts, exposed (1 = white): the sun's disc,
/// far brighter, would otherwise turn every shaft into its own colour.
const MASK_CLAMP: f32 = 4.0;
/// The sky's light under this (exposed) feeds no shaft: only the glow
/// around the light does, not the whole sunset sky (which laid an orange
/// veil over the scene in front, in the demo's sunset).
const MASK_THRESHOLD: f32 = 0.6;
/// The same by night, when the light is the moon's: its disc and its glow
/// are far dimmer than the sun's, and under the day's threshold they fed
/// no shaft (human report, 2026-11-27).
const MOON_THRESHOLD: f32 = 0.08;
/// How far from the light the shafts reach on the image (in heights of
/// the image), fading on the way.
const REACH: f32 = 1.1;
/// How much of the blurred light comes back at full strength: the shafts'
/// light is an average along the way to the sun, mostly of dark.
const GAIN: f32 = 1.0;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug, PartialEq)]
struct ShaftsU {
    /// xy the light's place on the image (uv, may lie off it), z the gain
    /// of the composite (strength x fade), w this blur pass's span of the
    /// logarithm of the offset's scale (0 outside the blur).
    sun: [f32; 4],
    /// x width / height, y [`MASK_RADIUS`], z [`MASK_CLAMP`], w the
    /// threshold ([`mask_threshold`]).
    shape: [f32; 4],
    /// x [`REACH`], y [`ON_OBJECTS`], zw unused.
    reach: [f32; 4],
}

/// Where the light is on the image and how much of its shafts show: none
/// behind the camera, fading as it leaves the view's axis (its streaks run
/// parallel as it nears the side) and as it sets.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SunOnImage {
    pub uv: [f32; 2],
    pub fade: f32,
    /// The sky's threshold for its light ([`mask_threshold`]).
    pub threshold: f32,
}

/// The light (`sun_dir`, toward it: the moon's by night, `day` 0) through
/// the camera (`view_proj`, its axis `forward`), None when it shows no
/// shafts.
pub(crate) fn sun_on_image(view_proj: Mat4, forward: Vec3, sun_dir: Vec3, day: f32) -> Option<SunOnImage> {
    let dir = sun_dir.normalize_or_zero();
    let facing = forward.normalize_or_zero().dot(dir);
    // up to 60 degrees off the axis at full strength, none past 84
    let fade = smoothstep(0.1, 0.5, facing) * smoothstep(-0.02, 0.05, dir.z);
    let clip = view_proj * dir.extend(0.0);
    if fade <= 0.0 || clip.w <= 1e-6 {
        return None;
    }
    let ndc = clip.xy() / clip.w;
    Some(SunOnImage {
        uv: [ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5],
        fade,
        threshold: mask_threshold(day),
    })
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The sky's threshold for the light of `sun_dir` (`day`: `sun_visible`,
/// 1 the sun's by day, 0 the moon's by night, between them at dusk).
pub(crate) fn mask_threshold(day: f32) -> f32 {
    MOON_THRESHOLD + (MASK_THRESHOLD - MOON_THRESHOLD) * day.clamp(0.0, 1.0)
}

/// The nearest the taps come to the light: this share of the pixel's
/// offset from it (1/16; a pixel 16 times further than the sky's glow
/// still reaches it).
const NEAREST: f32 = 1.0 / 16.0;

/// The span of each blur pass in the logarithm of the offset's scale: the
/// whole of it (down to [`NEAREST`]), then each a [`TAPS`]th of the one
/// before, so that the last fills the gaps between the first's taps.
pub(crate) fn reaches() -> [f32; PASSES] {
    let whole = -NEAREST.ln();
    std::array::from_fn(|i| whole * (TAPS as f32).powi(-(i as i32)))
}

/// The strength this frame from Preferences' (`setting`, 0 = off) and
/// `AURORA_SUN_RAYS` (`0..1` or `off`), which overrides it.
pub fn strength(setting: f32) -> f32 {
    static ENV: std::sync::OnceLock<Option<f32>> = std::sync::OnceLock::new();
    let env = *ENV.get_or_init(|| std::env::var("AURORA_SUN_RAYS").ok().and_then(|v| parse(&v)));
    env.unwrap_or(setting).clamp(0.0, 1.0)
}

/// `AURORA_SUN_RAYS`: a strength (0 or `off`: none), None when unreadable.
fn parse(v: &str) -> Option<f32> {
    let v = v.trim();
    if v.eq_ignore_ascii_case("off") {
        return Some(0.0);
    }
    v.parse::<f32>().ok().filter(|s| s.is_finite()).map(|s| s.clamp(0.0, 1.0))
}

struct Targets {
    size: (u32, u32),
    /// The half-resolution mask and blur, ping-ponged.
    half: [wgpu::TextureView; 2],
    out: wgpu::TextureView,
    /// The post pass's group 0 reading `out`.
    post_bg: wgpu::BindGroup,
}

pub(crate) struct SunShafts {
    layout: wgpu::BindGroupLayout,
    /// One per pass: the mask, the blur passes, the composite.
    buffers: Vec<wgpu::Buffer>,
    mask: wgpu::RenderPipeline,
    blur: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,
    /// A 1 x 1 half target, read by the mask pass (which reads none).
    blank: wgpu::TextureView,
    targets: Option<Targets>,
}

fn tex(binding: u32, sample_type: wgpu::TextureSampleType) -> wgpu::BindGroupLayoutEntry {
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

impl SunShafts {
    /// `post_layout`: group 0 of the post pass (the scene image, its
    /// sampler and parameters), which the passes read the image through.
    pub(crate) fn new(device: &wgpu::Device, post_layout: &wgpu::BindGroupLayout, hdr: wgpu::TextureFormat) -> SunShafts {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sun shafts"),
            entries: &[
                tex(0, wgpu::TextureSampleType::Depth),
                tex(1, wgpu::TextureSampleType::Float { filterable: true }),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let buffers = (0..PASSES + 2)
            .map(|_| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("sun shafts"),
                    size: std::mem::size_of::<ShaftsU>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            })
            .collect();
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sun shafts"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/post.wgsl").into()),
        });
        // (group 1, the bloom's, is not read)
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sun shafts"),
            bind_group_layouts: &[Some(post_layout), None, Some(&layout)],
            immediate_size: 0,
        });
        let pipe = |label: &str, fs: &str, format: wgpu::TextureFormat| {
            device.create_render_pipeline_cached(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pl),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_post"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let blank = crate::renderer::make_view(device, "sun shafts blank", 1, 1, FORMAT, 1, wgpu::TextureUsages::TEXTURE_BINDING).1;
        SunShafts {
            mask: pipe("sun shafts mask", "fs_shafts_mask", FORMAT),
            blur: pipe("sun shafts blur", "fs_shafts_blur", FORMAT),
            composite: pipe("sun shafts composite", "fs_shafts_composite", hdr),
            layout,
            buffers,
            blank,
            targets: None,
        }
    }

    /// The targets are dropped with the effect.
    pub(crate) fn release(&mut self) {
        self.targets = None;
    }

    /// Whether the targets for `size` are made.
    pub(crate) fn has_targets(&self, size: (u32, u32)) -> bool {
        self.targets.as_ref().is_some_and(|t| t.size == size)
    }

    /// The targets for `size`, made if needed (the renderer makes them
    /// first, checked: renderer.rs `first_use`).
    pub(crate) fn ensure_targets(
        &mut self,
        device: &wgpu::Device,
        size: (u32, u32),
        hdr: wgpu::TextureFormat,
        post: &crate::dof::PostInputs,
    ) {
        if self.has_targets(size) {
            return;
        }
        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let (hw, hh) = (size.0.div_ceil(2), size.1.div_ceil(2));
        let half = |i: usize| crate::renderer::make_view(device, &format!("sun shafts {i}"), hw, hh, FORMAT, 1, usage).1;
        let out = crate::renderer::make_view(device, "sun shafts out", size.0, size.1, hdr, 1, usage).1;
        let post_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("post (sun shafts)"),
            layout: post.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&out),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(post.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: post.buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(post.glow),
                },
            ],
        });
        self.targets = Some(Targets {
            size,
            half: [half(0), half(1)],
            out,
            post_bg,
        });
    }

    /// The shafts over the image the post pass reads (`scene_bg`, its group
    /// 0), with the frame's `depth` (reverse Z, the sky at 0) and the light
    /// on it, at `gain` (strength x fade); returns the post pass's group 0
    /// reading the result. After [`SunShafts::ensure_targets`].
    pub(crate) fn encode(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        scene_bg: &wgpu::BindGroup,
        depth: &wgpu::TextureView,
        sun: SunOnImage,
        gain: f32,
    ) -> Option<&wgpu::BindGroup> {
        let t = self.targets.as_ref()?;
        let shape = [t.size.0 as f32 / t.size.1.max(1) as f32, MASK_RADIUS, MASK_CLAMP, sun.threshold];
        let u = |reach: f32| ShaftsU {
            sun: [sun.uv[0], sun.uv[1], gain * GAIN, reach],
            shape,
            reach: [REACH, ON_OBJECTS, 0.0, 0.0],
        };
        // the mask, the blur passes, the composite: each its own buffer (the
        // writes all land before the encoder runs)
        let reach = reaches();
        for (i, b) in self.buffers.iter().enumerate() {
            let r = if i == 0 || i > PASSES { 0.0 } else { reach[i - 1] };
            queue.write_buffer(b, 0, bytemuck::bytes_of(&u(r)));
        }
        let group = |src: &wgpu::TextureView, buffer: &wgpu::Buffer| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("sun shafts"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(depth),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(src),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: buffer.as_entire_binding(),
                    },
                ],
            })
        };
        let run = |encoder: &mut wgpu::CommandEncoder,
                   label: &str,
                   target: &wgpu::TextureView,
                   pipe: &wgpu::RenderPipeline,
                   g: &wgpu::BindGroup| {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(label),
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
            pass.set_bind_group(0, scene_bg, &[]);
            pass.set_bind_group(2, g, &[]);
            pass.draw(0..3, 0..1);
        };
        run(
            encoder,
            "sun shafts mask",
            &t.half[0],
            &self.mask,
            &group(&self.blank, &self.buffers[0]),
        );
        for i in 0..PASSES {
            let (src, dst) = (&t.half[i % 2], &t.half[(i + 1) % 2]);
            run(encoder, "sun shafts blur", dst, &self.blur, &group(src, &self.buffers[i + 1]));
        }
        run(
            encoder,
            "sun shafts composite",
            &t.out,
            &self.composite,
            &group(&t.half[PASSES % 2], &self.buffers[PASSES + 1]),
        );
        Some(&t.post_bg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec4;

    #[test]
    fn the_light_ahead_lands_on_the_image_and_behind_shows_none() {
        // looking along +x, z up, right toward -y, 90 degrees wide, an
        // infinite reverse depth: clip = (-y, z, 0.1, x)
        let vp = Mat4::from_cols(Vec4::W, -Vec4::X, Vec4::Y, Vec4::Z * 0.1);
        // straight ahead, a little up: at the centre, above it
        let ahead = sun_on_image(vp, Vec3::X, Vec3::new(1.0, 0.0, 0.2), 1.0).expect("ahead");
        assert!((ahead.uv[0] - 0.5).abs() < 1e-4 && ahead.uv[1] < 0.5);
        assert!(ahead.fade > 0.9);
        // to the left: left of the centre
        let left = sun_on_image(vp, Vec3::X, Vec3::new(1.0, 0.3, 0.2), 1.0).expect("left");
        assert!(left.uv[0] < 0.5);
        // behind, or set: none
        assert_eq!(sun_on_image(vp, Vec3::X, Vec3::new(-1.0, 0.0, 0.2), 1.0), None);
        assert_eq!(sun_on_image(vp, Vec3::X, Vec3::new(1.0, 0.0, -0.1), 1.0), None);
        // at the side, nearly none
        let side = sun_on_image(vp, Vec3::X, Vec3::new(0.15, 1.0, 0.1), 1.0).map_or(0.0, |s| s.fade);
        assert!(side < 0.05);
    }

    #[test]
    fn the_moon_feeds_shafts_from_a_dimmer_sky() {
        assert_eq!(mask_threshold(1.0), MASK_THRESHOLD);
        assert_eq!(mask_threshold(0.0), MOON_THRESHOLD);
        let dusk = mask_threshold(0.5);
        assert!(dusk > MOON_THRESHOLD && dusk < MASK_THRESHOLD);
        // out of range: clamped
        assert_eq!(mask_threshold(-1.0), MOON_THRESHOLD);
        assert_eq!(mask_threshold(3.0), MASK_THRESHOLD);
    }

    #[test]
    fn each_pass_spans_a_tap_of_the_one_before() {
        let r = reaches();
        // the first pass's last tap is a step short of the nearest
        let last = (-r[0] * (TAPS - 1) as f32 / TAPS as f32).exp();
        assert!(last > NEAREST && last < NEAREST * 1.5);
        for i in 1..PASSES {
            assert!((r[i] * TAPS as f32 - r[i - 1]).abs() < 1e-5);
        }
        // the last pass's taps fill the first's gaps: 512 even steps
        assert_eq!((TAPS as usize).pow(PASSES as u32), 512);
        // each under 1 % of the offset: no ghost of a bright gap shows
        assert!(1.0 - (-r[0] / 512.0).exp() < 0.01);
    }

    #[test]
    fn the_variable_reads_a_strength_or_off() {
        assert_eq!(parse(" off "), Some(0.0));
        assert_eq!(parse("0.7"), Some(0.7));
        assert_eq!(parse("3"), Some(1.0));
        assert_eq!(parse("0"), Some(0.0));
        assert_eq!(parse("bright"), None);
    }
}
