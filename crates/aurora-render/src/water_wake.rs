//! Fine, path-following avatar displacement in the lower water-atlas viewport.

use crate::pipeline_cache::CachedPipelines;

pub(crate) struct WakeGpu {
    pipeline: wgpu::RenderPipeline,
    group: wgpu::BindGroup,
}

impl WakeGpu {
    pub fn new(device: &wgpu::Device, uniform: &wgpu::Buffer, events: &wgpu::Buffer, ground: &wgpu::TextureView) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("swept avatar water displacement"),
            source: wgpu::ShaderSource::Wgsl(crate::water_shader_source(
                &["water_wake.wgsl"],
                include_str!("shaders/water_wake.wgsl"),
            )),
        });
        let additive = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        };
        let pipeline = device.create_render_pipeline_cached(&wgpu::RenderPipelineDescriptor {
            label: Some("avatar water furrow atlas"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_wake"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_wake"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba16Float,
                    blend: Some(wgpu::BlendState {
                        color: additive,
                        alpha: additive,
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("swept water contacts"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: events.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(ground),
                },
            ],
        });
        Self { pipeline, group }
    }

    /// The coarse simulation has already read the old top half. Clear the
    /// atlas, rasterize the fine half, then the caller copies in the new top.
    pub fn encode(&self, encoder: &mut wgpu::CommandEncoder, atlas: &wgpu::TextureView, size: u32, events: u32) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("avatar water furrows"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: atlas,
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
        if events > 0 {
            pass.set_viewport(0.0, size as f32, size as f32, size as f32, 0.0, 1.0);
            pass.set_scissor_rect(0, size, size, size);
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.group, &[]);
            pass.draw(0..6, 0..events);
        }
    }
}
