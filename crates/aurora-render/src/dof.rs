//! Depth of field (LLPipeline::renderDoF, newview/pipeline.cpp, and its
//! shaders cofF.glsl / postDeferredF.glsl / dofCombineF.glsl, originally
//! LGPL 2.1): the circle of confusion of Firestorm's thin lens, the blur
//! Aurora's (`shaders/dof.wgsl`): half-resolution far and near fields, the
//! near one spread from a dilated tile map, laid over the sharp image.
//! Its targets exist only while the effect is on: off, it costs nothing.

use crate::DepthOfField;
use bytemuck::{Pod, Zeroable};
use glam::Mat4;

const HALF_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const TILE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R16Float;
/// Half pixels per tile of the near blur map (TILE in dof.wgsl).
const TILE: u32 = 16;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct DofU {
    inv_proj: [[f32; 4]; 4],
    lens: [f32; 4],
    sizes: [f32; 4],
    tiles: [f32; 4],
    /// Focus peaking: colour, 1 when on.
    peak: [f32; 4],
    /// The far end of the sharp zone (m), then padding.
    band: [f32; 4],
}

struct Targets {
    size: (u32, u32),
    half: wgpu::TextureView,
    tiles: wgpu::TextureView,
    dilated: wgpu::TextureView,
    far: wgpu::TextureView,
    near: wgpu::TextureView,
    out: wgpu::TextureView,
    /// The post pass's bind group reading `out`.
    post_bg: wgpu::BindGroup,
}

pub(crate) struct Dof {
    layout: wgpu::BindGroupLayout,
    buffer: wgpu::Buffer,
    sampler: wgpu::Sampler,
    setup: wgpu::RenderPipeline,
    tiles: wgpu::RenderPipeline,
    dilate: wgpu::RenderPipeline,
    blur: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,
    targets: Option<Targets>,
}

/// What the post pass reads, given to the targets' bind group.
pub(crate) struct PostInputs<'a> {
    pub layout: &'a wgpu::BindGroupLayout,
    pub sampler: &'a wgpu::Sampler,
    pub buffer: &'a wgpu::Buffer,
    pub glow: &'a wgpu::TextureView,
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

impl Dof {
    pub(crate) fn new(device: &wgpu::Device, hdr: wgpu::TextureFormat) -> Dof {
        let float = wgpu::TextureSampleType::Float { filterable: true };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("dof"),
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
            label: Some("dof params"),
            size: std::mem::size_of::<DofU>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("dof"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("dof"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/dof.wgsl").into()),
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("dof"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipe = |label: &str, fs: &str, formats: &[wgpu::TextureFormat]| {
            let targets: Vec<Option<wgpu::ColorTargetState>> = formats
                .iter()
                .map(|f| {
                    Some(wgpu::ColorTargetState {
                        format: *f,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })
                })
                .collect();
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pl),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_dof"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &targets,
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        Dof {
            setup: pipe("dof setup", "fs_setup", &[HALF_FORMAT]),
            tiles: pipe("dof tiles", "fs_tiles", &[TILE_FORMAT]),
            dilate: pipe("dof dilate", "fs_dilate", &[TILE_FORMAT]),
            blur: pipe("dof blur", "fs_blur", &[HALF_FORMAT, HALF_FORMAT]),
            composite: pipe("dof composite", "fs_composite", &[hdr]),
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

    fn ensure_targets(&mut self, device: &wgpu::Device, size: (u32, u32), hdr: wgpu::TextureFormat, post: &PostInputs) {
        if self.targets.as_ref().is_none_or(|t| t.size != size) {
            let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
            let view = |label: &str, w: u32, h: u32, f: wgpu::TextureFormat| crate::renderer::make_view(device, label, w, h, f, 1, usage).1;
            let (hw, hh) = (size.0.div_ceil(2), size.1.div_ceil(2));
            let (tw, th) = (hw.div_ceil(TILE), hh.div_ceil(TILE));
            let out = view("dof out", size.0, size.1, hdr);
            let post_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("post (dof)"),
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
                half: view("dof half", hw, hh, HALF_FORMAT),
                tiles: view("dof tiles", tw, th, TILE_FORMAT),
                dilated: view("dof tiles dilated", tw, th, TILE_FORMAT),
                far: view("dof far", hw, hh, HALF_FORMAT),
                near: view("dof near", hw, hh, HALF_FORMAT),
                out,
                post_bg,
            });
        }
    }

    /// The effect over `input` (the scene, after TAA) with the depth of the
    /// frame; returns the post pass's bind group reading the result.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        input: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        proj: Mat4,
        size: (u32, u32),
        params: &DepthOfField,
        hdr: wgpu::TextureFormat,
        post: &PostInputs,
        cutout: bool,
    ) -> &wgpu::BindGroup {
        let (hw, hh) = (size.0.div_ceil(2), size.1.div_ceil(2));
        let u = DofU {
            inv_proj: proj.inverse().to_cols_array_2d(),
            lens: [
                params.coc_scale,
                params.focus,
                params.max_coc.max(0.5),
                params.samples.clamp(8, 128) as f32,
            ],
            sizes: [size.0 as f32, size.1 as f32, hw as f32, hh as f32],
            tiles: [
                hw.div_ceil(TILE) as f32,
                hh.div_ceil(TILE) as f32,
                if params.debug { 1.0 } else { 0.0 },
                if cutout { 1.0 } else { 0.0 },
            ],
            peak: match params.peaking {
                Some([r, g, b]) => [r, g, b, 1.0],
                None => [0.0; 4],
            },
            band: [params.focus_far.max(params.focus), 0.0, 0.0, 0.0],
        };
        queue.write_buffer(&self.buffer, 0, bytemuck::bytes_of(&u));
        self.ensure_targets(device, size, hdr, post);
        let (layout, sampler, buffer) = (&self.layout, &self.sampler, &self.buffer);
        let (setup, tiles_pipe, dilate, blur, composite) = (&self.setup, &self.tiles, &self.dilate, &self.blur, &self.composite);
        let t = self.targets.as_ref().expect("targets made above");
        let bg = |a: &wgpu::TextureView, b: &wgpu::TextureView, c: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("dof"),
                layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(a),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(depth),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(b),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(c),
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
                   targets: &[&wgpu::TextureView],
                   pipe: &wgpu::RenderPipeline,
                   group: &wgpu::BindGroup| {
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
            pass.set_pipeline(pipe);
            pass.set_bind_group(0, group, &[]);
            pass.draw(0..3, 0..1);
        };
        run(encoder, "dof setup", &[&t.half], setup, &bg(input, &t.tiles, &t.tiles));
        run(encoder, "dof tiles", &[&t.tiles], tiles_pipe, &bg(&t.half, &t.half, &t.half));
        run(encoder, "dof dilate", &[&t.dilated], dilate, &bg(&t.tiles, &t.half, &t.half));
        run(encoder, "dof blur", &[&t.far, &t.near], blur, &bg(&t.half, &t.dilated, &t.tiles));
        run(encoder, "dof composite", &[&t.out], composite, &bg(input, &t.far, &t.near));
        &t.post_bg
    }
}
