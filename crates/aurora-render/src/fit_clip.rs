//! LiveMesh's Fit lab clipping map (Aurora's own, the Legacy Viewer has
//! none): on the avatars the viewer names (ours, the mannequins), the
//! pixels where the body pokes through the garment being made.
//!
//! Screen space, every frame, with no read back: the garment's outer
//! surface (`fs_fit_garment`: the front faces of a single-sided material,
//! both sides of a double-sided one) goes to a depth of its own;
//! the body's faces (the system body and the rigged meshes worn) that are
//! the visible surface go to a coverage mask (`fs_fit_body`, read-only test
//! against the scene's depth, as the selection's coverage). Where a body
//! pixel shows and the garment's surface lies just behind it, the body came
//! through: the composite paints it, amber where they meet, red where the
//! body is out, up to a reach beyond which the body is another part in
//! front (an arm before the chest). Both passes share the main pass's
//! skinning (`vs_fit`), so the map follows the animations and the shapes.

use crate::pipeline_cache::CachedPipelines;
use bytemuck::{Pod, Zeroable};
use glam::Mat4;

/// The map's colours and reach, from the viewer (linear RGBA).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FitClipParams {
    /// Where the body is out of the garment.
    pub clip_color: [f32; 4],
    /// Where body and garment meet.
    pub touch_color: [f32; 4],
    /// The farthest the body may stand in front of the garment and still
    /// count as through it (metres).
    pub reach: f32,
    /// The band either side of the garment's surface where they meet
    /// (metres).
    pub touch: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    clip_color: [f32; 4],
    touch_color: [f32; 4],
    depth: [f32; 4],
    output_srgb: f32,
    _pad: [f32; 3],
}

pub(crate) struct FitClip {
    pub size: [u32; 2],
    coverage: wgpu::TextureView,
    garment: wgpu::TextureView,
    layout: wgpu::BindGroupLayout,
    buffer: wgpu::Buffer,
    composite: wgpu::RenderPipeline,
    output_srgb: bool,
}

impl FitClip {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, depth_format: wgpu::TextureFormat, size: [u32; 2]) -> Self {
        let image = |label, format| {
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
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let coverage = image("fit lab body coverage", wgpu::TextureFormat::R8Unorm);
        let garment = image("fit lab garment depth", depth_format);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fit lab clipping parameters"),
            size: std::mem::size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
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
            label: Some("fit lab clipping"),
            entries: &[
                texture(0, wgpu::TextureSampleType::Float { filterable: false }),
                texture(1, wgpu::TextureSampleType::Depth),
                texture(2, wgpu::TextureSampleType::Depth),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
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
            label: Some("fit lab clipping"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/fit_clip.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fit lab clipping"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let composite = device.create_render_pipeline_cached(&wgpu::RenderPipelineDescriptor {
            label: Some("fit lab clipping composite"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_fullscreen"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_composite_fit"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::COLOR,
                })],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        Self {
            size,
            coverage,
            garment,
            layout,
            buffer,
            composite,
            output_srgb: format.is_srgb(),
        }
    }

    /// This frame's colours and reach, with the projection that made the
    /// depths (P32 and P22 of a reversed infinite projection).
    pub fn update(&self, queue: &wgpu::Queue, params: &FitClipParams, proj: Mat4) {
        queue.write_buffer(
            &self.buffer,
            0,
            bytemuck::bytes_of(&Params {
                clip_color: params.clip_color,
                touch_color: params.touch_color,
                depth: [proj.w_axis.z, proj.z_axis.z, params.reach.max(0.001), params.touch.max(0.0)],
                output_srgb: f32::from(self.output_srgb),
                _pad: [0.0; 3],
            }),
        );
    }

    /// The garment's outer surface, nearest first, in its own depth.
    pub fn begin_garment<'a>(&self, encoder: &'a mut wgpu::CommandEncoder) -> wgpu::RenderPass<'a> {
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("fit lab garment depth"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.garment,
                depth_ops: Some(wgpu::Operations {
                    // reversed depth: 0 is nothing there
                    load: wgpu::LoadOp::Clear(0.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        })
    }

    /// The body's visible pixels, tested against the scene's depth, which
    /// it only reads.
    pub fn begin_body<'a>(&self, encoder: &'a mut wgpu::CommandEncoder, depth: &wgpu::TextureView) -> wgpu::RenderPass<'a> {
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("fit lab body coverage"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.coverage,
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

    /// The map over `target` (the finished image, at `size`), from the
    /// coverage, the scene's depth and the garment's.
    pub fn encode(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        timestamps: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) {
        // (the scene's depth view changes with the upscaler and the size:
        // a group of this frame's, two textures and a buffer)
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fit lab clipping"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&self.coverage),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(depth),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&self.garment),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.buffer.as_entire_binding(),
                },
            ],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("fit lab clipping composite"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            // (the frame's last pass when no HUD follows: its end closes
            // the frame's GPU time)
            timestamp_writes: timestamps,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.composite);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
    }
}
