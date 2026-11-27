//! Motion blur, Aurora's own (the Legacy Viewer draws none): the image
//! smeared along each pixel's motion since the last frame, as a camera's
//! shutter open for a share of the frame (`shaders/motion_blur.wgsl`). It
//! reads the motion target of the depth prepass (the avatars, animesh and
//! moving objects) and the camera's own motion through the depth, after
//! the anti-aliasing and the depth of field, before the glow, the bloom
//! and the tone mapping, so that a bright light streaks as bright as it is.
//! Its targets exist only while the effect is on: off, it costs nothing.

use crate::MotionBlur;
use crate::dof::PostInputs;
use crate::pipeline_cache::CachedPipelines;
use bytemuck::{Pod, Zeroable};
use glam::Mat4;

/// The longest motion of a tile and of its neighbours.
const TILE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg16Float;
/// Pixels per tile of the motion map (TILE in motion_blur.wgsl).
const TILE: u32 = 16;
/// The gather's samples, at least and at most.
const SAMPLES: std::ops::RangeInclusive<u32> = 4..=64;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MbU {
    inv_vp: [[f32; 4]; 4],
    prev_vp: [[f32; 4]; 4],
    inv_proj: [[f32; 4]; 4],
    params: [f32; 4],
    sizes: [f32; 4],
}

/// The cameras of the frame and of the last one, without the jitter.
pub(crate) struct Cameras {
    pub view_proj: Mat4,
    pub prev_view_proj: Mat4,
    pub proj: Mat4,
}

/// The uniforms of a frame: `size` the image's, `motion` whether the
/// prepass's motion target is read.
fn uniforms(params: &MotionBlur, cameras: &Cameras, size: (u32, u32), motion: bool) -> MbU {
    MbU {
        inv_vp: cameras.view_proj.inverse().to_cols_array_2d(),
        prev_vp: cameras.prev_view_proj.to_cols_array_2d(),
        inv_proj: cameras.proj.inverse().to_cols_array_2d(),
        params: [
            params.scale.max(0.0),
            longest_px(params.max_share, size.1),
            params.samples.clamp(*SAMPLES.start(), *SAMPLES.end()) as f32,
            if motion { 1.0 } else { 0.0 },
        ],
        sizes: [
            size.0 as f32,
            size.1 as f32,
            size.0.div_ceil(TILE) as f32,
            size.1.div_ceil(TILE) as f32,
        ],
    }
}

/// The longest streak in pixels of an image `height` high: at least one,
/// at most half the image.
fn longest_px(share: f32, height: u32) -> f32 {
    (share.clamp(0.0, 0.5) * height as f32).max(1.0)
}

struct Targets {
    size: (u32, u32),
    tiles: wgpu::TextureView,
    neighbors: wgpu::TextureView,
    out: wgpu::TextureView,
    /// The post pass's bind group reading `out`.
    post_bg: wgpu::BindGroup,
}

pub(crate) struct MotionBlurPass {
    layout: wgpu::BindGroupLayout,
    buffer: wgpu::Buffer,
    sampler: wgpu::Sampler,
    tiles: wgpu::RenderPipeline,
    neighbors: wgpu::RenderPipeline,
    gather: wgpu::RenderPipeline,
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

impl MotionBlurPass {
    pub(crate) fn new(device: &wgpu::Device, hdr: wgpu::TextureFormat) -> MotionBlurPass {
        let float = wgpu::TextureSampleType::Float { filterable: true };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("motion blur"),
            entries: &[
                tex(0, float),
                tex(1, wgpu::TextureSampleType::Depth),
                tex(2, float),
                tex(3, float),
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
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
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("motion blur params"),
            size: std::mem::size_of::<MbU>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("motion blur"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("motion blur"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/motion_blur.wgsl").into()),
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("motion blur"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipe = |label: &str, fs: &str, format: wgpu::TextureFormat| {
            device.create_render_pipeline_cached(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pl),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_mb"),
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
        MotionBlurPass {
            tiles: pipe("motion blur tiles", "fs_tiles", TILE_FORMAT),
            neighbors: pipe("motion blur neighbors", "fs_neighbors", TILE_FORMAT),
            gather: pipe("motion blur gather", "fs_gather", hdr),
            layout,
            buffer,
            sampler,
            targets: None,
        }
    }

    /// The targets are dropped with the effect.
    pub(crate) fn release(&mut self) {
        self.targets = None;
    }

    /// Whether the targets for `size` are made (`ensure_targets` would
    /// make nothing).
    pub(crate) fn has_targets(&self, size: (u32, u32)) -> bool {
        self.targets.as_ref().is_some_and(|t| t.size == size)
    }

    /// The targets for `size`, made if needed (the renderer makes them
    /// first, checked: renderer.rs `first_use`).
    pub(crate) fn ensure_targets(&mut self, device: &wgpu::Device, size: (u32, u32), hdr: wgpu::TextureFormat, post: &PostInputs) {
        if self.targets.as_ref().is_none_or(|t| t.size != size) {
            let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
            let view = |label: &str, w: u32, h: u32, f: wgpu::TextureFormat| crate::renderer::make_view(device, label, w, h, f, 1, usage).1;
            let (tw, th) = (size.0.div_ceil(TILE), size.1.div_ceil(TILE));
            let out = view("motion blur out", size.0, size.1, hdr);
            let post_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("post (motion blur)"),
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
                tiles: view("motion blur tiles", tw, th, TILE_FORMAT),
                neighbors: view("motion blur neighbors", tw, th, TILE_FORMAT),
                out,
                post_bg,
            });
        }
    }

    /// The effect over `input` (the scene after the anti-aliasing and the
    /// depth of field), with the depth of the frame and the prepass's
    /// motion target (`None`: the camera's motion alone); returns the post
    /// pass's bind group reading the result.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        input: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        motion: Option<&wgpu::TextureView>,
        cameras: &Cameras,
        size: (u32, u32),
        params: &MotionBlur,
        hdr: wgpu::TextureFormat,
        post: &PostInputs,
    ) -> &wgpu::BindGroup {
        let u = uniforms(params, cameras, size, motion.is_some());
        queue.write_buffer(&self.buffer, 0, bytemuck::bytes_of(&u));
        self.ensure_targets(device, size, hdr, post);
        let (layout, sampler, buffer) = (&self.layout, &self.sampler, &self.buffer);
        let t = self.targets.as_ref().expect("targets made above");
        // (without the motion target the shader reads none: any float
        // texture no pass writes binds the slot)
        let motion = motion.unwrap_or(input);
        let bg = |color: &wgpu::TextureView, tiles: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("motion blur"),
                layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(color),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(depth),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(motion),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(tiles),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: buffer.as_entire_binding(),
                    },
                ],
            })
        };
        let run = |encoder: &mut wgpu::CommandEncoder,
                   label: &str,
                   target: &wgpu::TextureView,
                   pipe: &wgpu::RenderPipeline,
                   group: &wgpu::BindGroup| {
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
            pass.set_bind_group(0, group, &[]);
            pass.draw(0..3, 0..1);
        };
        // (each pass binds as `tiles` what it does not write)
        run(encoder, "motion blur tiles", &t.tiles, &self.tiles, &bg(input, &t.neighbors));
        run(
            encoder,
            "motion blur neighbors",
            &t.neighbors,
            &self.neighbors,
            &bg(input, &t.tiles),
        );
        run(encoder, "motion blur gather", &t.out, &self.gather, &bg(input, &t.neighbors));
        &t.post_bg
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(scale: f32, max_share: f32, samples: u32) -> MotionBlur {
        MotionBlur { scale, max_share, samples }
    }

    fn cameras() -> Cameras {
        Cameras {
            view_proj: Mat4::IDENTITY,
            prev_view_proj: Mat4::IDENTITY,
            proj: Mat4::IDENTITY,
        }
    }

    /// The shader is valid for Vulkan's SPIR-V and Metal's MSL, one entry
    /// point at a time as wgpu writes them, and its uniforms are laid out
    /// as `MbU`.
    #[test]
    fn the_shader_is_valid_for_vulkan_and_metal_and_reads_the_uniforms_as_written() {
        let source = include_str!("shaders/motion_blur.wgsl");
        let module = naga::front::wgsl::parse_str(source).unwrap_or_else(|e| panic!("{}", e.emit_to_string(source)));
        let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::empty())
            .validate(&module)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(source)));
        assert_eq!(module.entry_points.len(), 4);
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
        let span = module
            .types
            .iter()
            .find(|(_, t)| t.name.as_deref() == Some("Mb"))
            .map(|(_, t)| match &t.inner {
                naga::TypeInner::Struct { span, .. } => *span,
                other => panic!("{other:?}"),
            })
            .expect("Mb");
        assert_eq!(span as usize, std::mem::size_of::<MbU>());
    }

    #[test]
    fn the_longest_streak_is_a_share_of_the_image_height_held_to_half_of_it() {
        assert_eq!(longest_px(0.05, 1080), 54.0);
        assert_eq!(longest_px(2.0, 1000), 500.0);
        assert_eq!(longest_px(0.0, 1080), 1.0);
        assert_eq!(longest_px(-1.0, 1080), 1.0);
    }

    #[test]
    fn the_samples_stay_in_the_gathers_range_and_tiles_cover_the_image() {
        let u = uniforms(&params(0.5, 0.05, 1), &cameras(), (1921, 1080), true);
        assert_eq!(u.params[2], 4.0);
        assert_eq!(u.params[3], 1.0);
        assert_eq!(u.sizes, [1921.0, 1080.0, 121.0, 68.0]);
        let u = uniforms(&params(-1.0, 0.05, 1000), &cameras(), (64, 64), false);
        assert_eq!(u.params[0], 0.0);
        assert_eq!(u.params[2], 64.0);
        assert_eq!(u.params[3], 0.0);
    }
}
