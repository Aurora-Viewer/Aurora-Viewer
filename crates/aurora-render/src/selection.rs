//! Selected material faces: visible coverage, an exterior halo, and a brief
//! selection pulse. The mask uses the posed triangles, so closed surfaces,
//! holes and disconnected mesh islands work without topology reconstruction.
//! Screen-space passes run after post processing to avoid TAA trails and keep
//! the material untouched once the pulse ends. No work without selected faces.

use bytemuck::{Pod, Zeroable};
#[cfg(test)]
mod tests;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    color: [f32; 4],
    pulse: f32,
    output_srgb: f32,
    scale: f32,
    _pad: f32,
}

pub(crate) struct Overlay {
    pub mask: wgpu::TextureView,
    horizontal: wgpu::TextureView,
    groups: [wgpu::BindGroup; 2],
    buffer: wgpu::Buffer,
    spread: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,
    pub size: [u32; 2],
    output_srgb: bool,
}

impl Overlay {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, size: [u32; 2]) -> Self {
        let image = |label| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: size[0],
                        height: size[1],
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::R8Unorm,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let mask = image("selected face coverage");
        let horizontal = image("selected face horizontal halo");
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("selection overlay parameters"),
            size: std::mem::size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let texture_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("selection overlay"),
            entries: &[
                texture_entry(0),
                texture_entry(1),
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
        let group = |second: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("selection overlay"),
                layout: &layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&mask),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(second),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: buffer.as_entire_binding(),
                    },
                ],
            })
        };
        // Never bind the horizontal target while the spread pass writes it.
        let groups = [group(&mask), group(&horizontal)];
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("selection overlay"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/selection.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("selection overlay"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |label, entry, format, blend| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_fullscreen"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend,
                        write_mask: wgpu::ColorWrites::COLOR,
                    })],
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let spread = pipeline("selection halo horizontal", "fs_spread", wgpu::TextureFormat::R8Unorm, None);
        let composite = pipeline(
            "selection halo composite",
            "fs_composite",
            format,
            Some(wgpu::BlendState::ALPHA_BLENDING),
        );
        Self {
            mask,
            horizontal,
            groups,
            buffer,
            spread,
            composite,
            size,
            output_srgb: format.is_srgb(),
        }
    }

    pub fn update(&self, queue: &wgpu::Queue, color: [f32; 4], pulse: f32, scale: f32) {
        queue.write_buffer(
            &self.buffer,
            0,
            bytemuck::bytes_of(&Params {
                color,
                pulse,
                output_srgb: f32::from(self.output_srgb),
                scale: scale.clamp(0.75, 3.0),
                _pad: 0.0,
            }),
        );
    }

    pub fn begin_mask<'a>(&self, encoder: &'a mut wgpu::CommandEncoder, depth: &wgpu::TextureView) -> wgpu::RenderPass<'a> {
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("selected face coverage"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.mask,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: None,
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        })
    }

    pub fn encode(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        mut timestamps: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) {
        for (i, (view, pipeline)) in [(&self.horizontal, &self.spread), (target, &self.composite)]
            .into_iter()
            .enumerate()
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("selection exterior halo"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: if i == 0 {
                            wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                        } else {
                            wgpu::LoadOp::Load
                        },
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: if i == 1 { timestamps.take() } else { None },
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &self.groups[i], &[]);
            pass.draw(0..3, 0..1);
        }
    }
}
