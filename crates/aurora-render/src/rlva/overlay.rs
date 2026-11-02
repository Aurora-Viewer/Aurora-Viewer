//! RlvOverlayEffect::run (rlveffects.cpp, originally LGPL 2.1): compose after
//! world labels and before HUD attachments. This also covers scene-only photos.
use crate::OverlayEffect;
use crate::pipeline_cache::CachedPipelines;

pub(crate) struct Overlays {
    pipeline: wgpu::RenderPipeline,
    params: wgpu::Buffer,
    group: wgpu::BindGroup,
}

impl Overlays {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, textures: &wgpu::BindGroupLayout) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("RLVa overlay parameters"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("RLVa overlay parameters"),
            size: (256 * std::mem::size_of::<OverlayEffect>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("RLVa overlay parameters"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: params.as_entire_binding(),
            }],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("RLVa overlays"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/rlva_overlay.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("RLVa overlays"),
            bind_group_layouts: &[Some(&layout), Some(textures)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline_cached(&wgpu::RenderPipelineDescriptor {
            label: Some("RLVa overlays"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        Self { pipeline, params, group }
    }

    pub fn encode(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        textures: &wgpu::BindGroup,
        effects: &[OverlayEffect],
    ) {
        let effects = &effects[..effects.len().min(256)];
        if effects.is_empty() {
            return;
        }
        queue.write_buffer(&self.params, 0, bytemuck::cast_slice(effects));
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("RLVa overlays"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.group, &[]);
        pass.set_bind_group(1, textures, &[]);
        pass.draw(0..3, 0..effects.len() as u32);
    }
}
