//! The motion vectors shown instead of the image (AURORA_DEBUG_MOTION=1):
//! the direction of each pixel's motion as its hue, its length as its
//! brightness. Where the depth prepass wrote none (the sky, or a frame
//! without GTAO's mesh normals, which carry them), the camera's motion
//! alone, from `FrameU::motion_reproj`.

use bytemuck::{Pod, Zeroable};
use glam::Mat4;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    reproj: [[f32; 4]; 4],
    /// xy: the output's size; z: 1 when the prepass wrote the motion.
    screen: [f32; 4],
}

pub(crate) struct MotionDebug {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    buffer: wgpu::Buffer,
    /// Bound in place of the motion when the prepass wrote none.
    empty: wgpu::TextureView,
}

/// Whether AURORA_DEBUG_MOTION asks for the view.
pub(crate) fn asked() -> bool {
    std::env::var_os("AURORA_DEBUG_MOTION").is_some_and(|v| v != "0")
}

impl MotionDebug {
    pub(crate) fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let texture = |binding, sample_type| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("motion debug"),
            entries: &[
                texture(0, wgpu::TextureSampleType::Float { filterable: false }),
                texture(1, wgpu::TextureSampleType::Depth),
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
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("motion debug"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/motion_debug.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("motion debug"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("motion debug"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("motion debug"),
            size: std::mem::size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let empty = crate::renderer::make_view(
            device,
            "motion debug empty",
            1,
            1,
            crate::gtao::NORMAL_FORMAT,
            1,
            wgpu::TextureUsages::TEXTURE_BINDING,
        )
        .1;
        Self {
            layout,
            pipeline,
            buffer,
            empty,
        }
    }

    /// Draw the view over `output`, of `size` pixels. `motion` is the
    /// prepass's target when it wrote the motion this frame.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encode(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        motion: Option<&wgpu::TextureView>,
        depth: &wgpu::TextureView,
        reproj: Mat4,
        size: (u32, u32),
        output: &wgpu::TextureView,
    ) {
        let params = Params {
            reproj: reproj.to_cols_array_2d(),
            screen: [size.0 as f32, size.1 as f32, if motion.is_some() { 1.0 } else { 0.0 }, 0.0],
        };
        queue.write_buffer(&self.buffer, 0, bytemuck::bytes_of(&params));
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("motion debug"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(motion.unwrap_or(&self.empty)),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(depth),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.buffer.as_entire_binding(),
                },
            ],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("motion debug"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: output,
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
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
    }
}
