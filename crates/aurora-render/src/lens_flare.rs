//! Screen-space optics from the resolved scene, before the display transform.
//! See John Chapman's "Screen Space Lens Flare" for the image-space ghost and halo technique.

use std::time::Instant;

use bytemuck::{Pod, Zeroable};

use crate::pipeline_cache::CachedPipelines;

/// The brightest input accepted by the flare extraction pass in post.wgsl.
pub const MAX_THRESHOLD: f32 = 32.0;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Quality {
    Performance,
    #[default]
    Balanced,
    High,
}

impl Quality {
    fn divisor(self) -> u32 {
        match self {
            Self::Performance => 8,
            Self::Balanced => 4,
            Self::High => 2,
        }
    }

    pub fn size(self, (width, height): (u32, u32)) -> (u32, u32) {
        let divisor = self.divisor();
        (width.div_ceil(divisor).max(1), height.div_ceil(divisor).max(1))
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LensFlare {
    pub intensity: f32,
    pub threshold: f32,
    pub knee: f32,
    pub blur: f32,
    pub quality: Quality,
    /// Display-space RGB, converted to a linear multiplier for the shader.
    pub tint: [f32; 3],
    pub ghosts: f32,
    pub ghost_count: u32,
    pub ghost_spread: f32,
    pub ghost_softness: f32,
    pub halo: f32,
    pub halo_radius: f32,
    pub halo_softness: f32,
    pub streaks: f32,
    pub streak_length: f32,
    pub streak_angle: f32,
    pub starburst: f32,
    pub star_length: f32,
    pub star_angle: f32,
    pub star_rays: u32,
    pub chromatic: f32,
}

impl Default for LensFlare {
    fn default() -> Self {
        Self {
            intensity: 0.45,
            threshold: 1.4,
            knee: 0.25,
            blur: 0.0,
            quality: Quality::Balanced,
            tint: [1.0; 3],
            ghosts: 0.10,
            ghost_count: 3,
            ghost_spread: 0.65,
            ghost_softness: 0.25,
            halo: 0.12,
            halo_radius: 0.21,
            halo_softness: 0.4,
            streaks: 0.0,
            streak_length: 0.55,
            streak_angle: 0.0,
            starburst: 0.0,
            star_length: 0.37,
            star_angle: 0.0,
            star_rays: 6,
            chromatic: 0.10,
        }
    }
}

impl LensFlare {
    pub fn sanitized(mut self) -> Self {
        let defaults = Self::default();
        fn bounded(value: f32, fallback: f32, min: f32, max: f32) -> f32 {
            if value.is_finite() { value.clamp(min, max) } else { fallback }
        }
        self.intensity = bounded(self.intensity, defaults.intensity, 0.0, 2.0);
        self.threshold = bounded(self.threshold, defaults.threshold, 0.0, MAX_THRESHOLD);
        self.knee = bounded(self.knee, defaults.knee, 0.0, 1.0);
        self.tint = self.tint.map(|c| bounded(c, 1.0, 0.0, 1.0));
        macro_rules! unit {
            ($($field:ident),+ $(,)?) => {
                $(self.$field = bounded(self.$field, defaults.$field, 0.0, 1.0);)+
            };
        }
        unit!(
            blur,
            ghosts,
            ghost_spread,
            ghost_softness,
            halo,
            halo_radius,
            halo_softness,
            streaks,
            streak_length,
            starburst,
            star_length,
            chromatic
        );
        self.streak_angle = bounded(self.streak_angle, defaults.streak_angle, 0.0, 180.0);
        self.star_angle = bounded(self.star_angle, defaults.star_angle, 0.0, 180.0);
        self.ghost_count = self.ghost_count.clamp(1, 6);
        if !matches!(self.star_rays, 4 | 6 | 8) {
            self.star_rays = defaults.star_rays;
        }
        self
    }

    pub fn active(self) -> bool {
        self.intensity > 0.0
            && self.threshold < MAX_THRESHOLD
            && self.tint.iter().any(|&channel| channel > 0.0)
            && (self.ghosts > 0.0
                || self.halo > 0.0
                || self.streaks > 0.0 && self.streak_length > 0.0
                || self.starburst > 0.0 && self.star_length > 0.0)
    }

    pub(crate) fn threshold_knee(self) -> f32 {
        self.knee * self.threshold.min(MAX_THRESHOLD - self.threshold)
    }

    fn ray_extents(self) -> [f32; 2] {
        [self.streak_length * 0.48, self.star_length * 0.30]
    }

    fn rays(self) -> impl Iterator<Item = (u32, f32)> {
        let [streak_extent, star_extent] = self.ray_extents();
        let streak = (self.streaks > 0.0 && streak_extent > 0.0).then_some((0, streak_extent));
        let star = if self.starburst > 0.0 && self.star_length > 0.0 {
            self.star_rays / 2
        } else {
            0
        };
        streak.into_iter().chain((1..=star).map(move |axis| (axis, star_extent)))
    }

    fn scratch_count(self) -> usize {
        if self.rays().next().is_some() {
            2
        } else {
            usize::from(self.blur > 0.0)
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Frame {
    pub settings: LensFlare,
    /// The finished 2D photo extent, before supersampling and tile padding.
    pub photo_size: Option<(u32, u32)>,
    pub last_context_tile: bool,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Optics {
    tint: [f32; 4],
    ghost: [f32; 4],
    halo: [f32; 4],
    streak: [f32; 4],
    star: [f32; 4],
    geometry: [f32; 4],
}

impl Optics {
    fn new(flare: LensFlare, extent: (u32, u32)) -> Self {
        let (width, height) = (extent.0 as f32, extent.1 as f32);
        let short = width.min(height).max(1.0);
        let [streak_extent, star_extent] = flare.ray_extents();
        let [r, g, b] = flare
            .tint
            .map(|c| if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) });
        Self {
            tint: [r, g, b, flare.intensity],
            ghost: [flare.ghosts, flare.ghost_count as f32, flare.ghost_spread, flare.ghost_softness],
            halo: [flare.halo, flare.halo_radius, flare.halo_softness, flare.chromatic],
            streak: [flare.streaks, streak_extent, flare.streak_angle.to_radians(), 0.0],
            star: [flare.starburst, star_extent, flare.star_angle.to_radians(), flare.star_rays as f32],
            geometry: [short / width.max(1.0), short / height.max(1.0), flare.blur, 0.0],
        }
    }
}

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

pub(crate) struct Pass {
    pub post_layout: wgpu::BindGroupLayout,
    off: wgpu::BindGroup,
    pipelines: Option<Pipelines>,
    live: Cache,
    photo: Cache,
}

#[derive(Default)]
struct Cache {
    targets: Option<Targets>,
    retry: crate::gpu_alloc::Backoff,
}

struct Targets {
    size: (u32, u32),
    highlights: wgpu::TextureView,
    generate: wgpu::BindGroup,
    result: wgpu::TextureView,
    post: wgpu::BindGroup,
    blur_input: wgpu::BindGroup,
    scratch: Vec<Scratch>,
}

struct Scratch {
    view: wgpu::TextureView,
    input: wgpu::BindGroup,
}

struct Pipelines {
    extract: wgpu::RenderPipeline,
    generate: wgpu::RenderPipeline,
    ray: wgpu::RenderPipeline,
    ray_add: wgpu::RenderPipeline,
    blur: wgpu::RenderPipeline,
    optics: wgpu::Buffer,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

fn texture() -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn post_group(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, view: &wgpu::TextureView) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("lens flare post"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::TextureView(view),
        }],
    })
}

fn pipeline(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    module: &wgpu::ShaderModule,
    vertex: &str,
    fragment: &str,
    additive: bool,
) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(fragment),
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline_cached(&wgpu::RenderPipelineDescriptor {
        label: Some(fragment),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some(vertex),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(fragment),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: FORMAT,
                blend: additive.then_some(wgpu::BlendState {
                    color: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::One,
                        dst_factor: wgpu::BlendFactor::One,
                        operation: wgpu::BlendOperation::Add,
                    },
                    alpha: wgpu::BlendComponent::REPLACE,
                }),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        multiview_mask: None,
        cache: None,
    })
}

impl Pipelines {
    fn new(device: &wgpu::Device, scene: &wgpu::BindGroupLayout) -> Self {
        let post_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("lens flare extraction"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/post.wgsl").into()),
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("lens flare optics"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/lens_flare.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lens flare optics"),
            entries: &[
                texture(),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<Optics>() as u64),
                    },
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("lens flare optics"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let optics = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("lens flare optics"),
            size: std::mem::size_of::<Optics>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            extract: pipeline(device, scene, &post_module, "vs_post", "fs_flare_extract", false),
            generate: pipeline(device, &layout, &module, "vs_flare", "fs_flare", false),
            ray: pipeline(device, &layout, &module, "vs_flare", "fs_flare_ray", false),
            ray_add: pipeline(device, &layout, &module, "vs_flare", "fs_flare_ray", true),
            blur: pipeline(device, &layout, &module, "vs_flare", "fs_flare_blur", false),
            optics,
            layout,
            sampler,
        }
    }

    fn targets(&self, device: &wgpu::Device, post_layout: &wgpu::BindGroupLayout, size: (u32, u32), scratch_count: usize) -> Targets {
        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let view = |label| crate::renderer::make_view(device, label, size.0, size.1, FORMAT, 1, usage).1;
        let highlights = view("lens flare highlights");
        let result = view("lens flare result");
        let bind = |input: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("lens flare optics"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(input),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: self.optics.as_entire_binding(),
                    },
                ],
            })
        };
        let generate = bind(&highlights);
        let post = post_group(device, post_layout, &result);
        let blur_input = bind(&result);
        let scratch = (0..scratch_count)
            .map(|_| {
                let view = view("lens flare scratch");
                let input = bind(&view);
                Scratch { view, input }
            })
            .collect();
        Targets {
            size,
            highlights,
            generate,
            result,
            post,
            blur_input,
            scratch,
        }
    }
}

impl Pass {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let post_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lens flare post"),
            entries: &[texture()],
        });
        let black = crate::renderer::make_view(device, "lens flare off", 1, 1, FORMAT, 1, wgpu::TextureUsages::TEXTURE_BINDING).1;
        Self {
            off: post_group(device, &post_layout, &black),
            post_layout,
            pipelines: None,
            live: Cache::default(),
            photo: Cache::default(),
        }
    }

    pub(crate) fn prepare(
        &mut self,
        device: &wgpu::Device,
        scene: &wgpu::BindGroupLayout,
        size: (u32, u32),
        photo: bool,
        flare: LensFlare,
    ) -> bool {
        let cache = if photo { &mut self.photo } else { &mut self.live };
        let scratch_count = flare.scratch_count();
        if cache
            .targets
            .as_ref()
            .is_some_and(|t| t.size == size && t.scratch.len() >= scratch_count)
        {
            return true;
        }
        let now = Instant::now();
        if !cache.retry.ready(now) {
            return false;
        }
        cache.targets = None;
        let first = self.pipelines.is_none();
        let (targets, refused) = crate::gpu_alloc::scoped(device, || {
            self.pipelines
                .get_or_insert_with(|| Pipelines::new(device, scene))
                .targets(device, &self.post_layout, size, scratch_count)
        });
        if refused && first {
            self.pipelines = None;
        }
        let refused = refused || crate::gpu_alloc::first_use_refused();
        if refused {
            cache.retry.failed(now);
            log::warn!("lens flare: could not allocate {} by {} targets", size.0, size.1);
        } else {
            cache.targets = Some(targets);
            cache.retry.succeeded();
        }
        !refused
    }

    pub(crate) fn release_live(&mut self) {
        self.live = Cache::default();
    }

    pub(crate) fn release_photo(&mut self) {
        self.photo = Cache::default();
    }

    pub(crate) fn photo_ready(&self) -> bool {
        self.photo.targets.is_some()
    }

    pub(crate) fn post_group(&self, active: bool, photo: bool) -> &wgpu::BindGroup {
        let cache = if photo { &self.photo } else { &self.live };
        cache.targets.as_ref().filter(|_| active).map_or(&self.off, |t| &t.post)
    }

    pub(crate) fn encode(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        scene: &wgpu::BindGroup,
        frame: Frame,
        extent: (u32, u32),
        photo: bool,
    ) {
        let cache = if photo { &self.photo } else { &self.live };
        let (Some(p), Some(t)) = (&self.pipelines, &cache.targets) else {
            return;
        };
        queue.write_buffer(&p.optics, 0, bytemuck::bytes_of(&Optics::new(frame.settings, extent)));
        draw(encoder, &t.highlights, &p.extract, scene, 0, false);
        draw(encoder, &t.result, &p.generate, &t.generate, 0, false);
        if let [a, b] = t.scratch.as_slice() {
            let rays = [a, b];
            for (axis, extent) in frame.settings.rays() {
                let radius = extent * t.size.0.min(t.size.1) as f32;
                let steps = ray_steps(radius);
                for stage in 0..steps {
                    let last = stage + 1 == steps;
                    let src = if stage == 0 {
                        &t.generate
                    } else {
                        &rays[(stage as usize - 1) % 2].input
                    };
                    let dst = if last { &t.result } else { &rays[stage as usize % 2].view };
                    // The fullscreen triangle carries the axis and filter
                    // strides in its instance index.
                    let instance = (axis << 8) | (steps << 4) | stage;
                    draw(encoder, dst, if last { &p.ray_add } else { &p.ray }, src, instance, last);
                }
            }
        }
        if frame.settings.blur > 0.0
            && let Some(scratch) = t.scratch.first()
        {
            draw(encoder, &scratch.view, &p.blur, &t.blur_input, 0, false);
            draw(encoder, &t.result, &p.blur, &scratch.input, 1, false);
        }
    }
}

/// Seven taps per stage at four times the previous stride. Overlapping
/// footprints keep rays continuous from highlights as narrow as one texel.
fn ray_steps(radius: f32) -> u32 {
    (1..=7).find(|steps| ((1u32 << (steps * 2)) - 1) as f32 >= radius).unwrap_or(7)
}

fn draw(
    encoder: &mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    pipeline: &wgpu::RenderPipeline,
    input: &wgpu::BindGroup,
    instance: u32,
    add: bool,
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("lens flare"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: if add {
                    wgpu::LoadOp::Load
                } else {
                    wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                },
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, input, &[]);
    pass.draw(0..3, instance..instance + 1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lens_flare_quality_rounds_odd_sizes_up_and_cannot_make_an_empty_map() {
        assert_eq!(Quality::Performance.size((1921, 1081)), (241, 136));
        assert_eq!(Quality::Balanced.size((1921, 1081)), (481, 271));
        assert_eq!(Quality::High.size((1921, 1081)), (961, 541));
        assert_eq!(Quality::Performance.size((1, 1)), (1, 1));
    }

    #[test]
    fn lens_flare_threshold_has_hard_endpoints_even_at_full_smoothness() {
        let flare = LensFlare {
            knee: 1.0,
            ..LensFlare::default()
        };
        let low = LensFlare { threshold: 0.0, ..flare };
        assert!(low.active());
        assert_eq!(low.threshold_knee(), 0.0);
        let high = LensFlare {
            threshold: MAX_THRESHOLD - 1.0,
            ..flare
        };
        assert!(high.active());
        assert_eq!(high.threshold_knee(), 1.0);
        let off = LensFlare {
            threshold: MAX_THRESHOLD,
            ..flare
        };
        assert!(!off.active());
        assert_eq!(off.threshold_knee(), 0.0);
    }

    #[test]
    fn lens_flare_ray_strides_start_no_further_apart_than_one_texel() {
        for radius in [0.25, 3.0, 3.1, 15.0, 65.0, 255.0, 520.0, 2048.0] {
            let steps = ray_steps(radius);
            let coverage = ((1u32 << (steps * 2)) - 1) as f32;
            assert!(radius / coverage <= 1.0, "radius {radius}, steps {steps}");
            assert!(steps == 1 || (((1u32 << ((steps - 1) * 2)) - 1) as f32) < radius);
        }
    }

    #[test]
    fn lens_flare_skips_work_for_black_or_zero_intensity_and_empty_components() {
        let flare = LensFlare::default();
        assert!(flare.active());
        assert!(!LensFlare { tint: [0.0; 3], ..flare }.active());
        assert!(!LensFlare { intensity: 0.0, ..flare }.active());
        assert!(
            !LensFlare {
                ghosts: 0.0,
                halo: 0.0,
                streaks: 1.0,
                streak_length: 0.0,
                ..flare
            }
            .active()
        );
    }

    #[test]
    fn lens_flare_shaders_translate_for_vulkan_and_metal() {
        for (source, entries) in [
            (include_str!("shaders/post.wgsl"), &["vs_post", "fs_flare_extract", "fs_post"][..]),
            (
                include_str!("shaders/lens_flare.wgsl"),
                &["vs_flare", "fs_flare", "fs_flare_ray", "fs_flare_blur"][..],
            ),
        ] {
            let module = naga::front::wgsl::parse_str(source).unwrap_or_else(|e| panic!("{}", e.emit_to_string(source)));
            let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::empty())
                .validate(&module)
                .unwrap_or_else(|e| panic!("{}", e.emit_to_string(source)));
            for ep in module.entry_points.iter().filter(|ep| entries.contains(&ep.name.as_str())) {
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
}
