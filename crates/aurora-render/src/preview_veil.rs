//! The veil of a preview: its bottom band blurred more and more toward the
//! bottom edge, under the caption an interface lays there (the AO window's
//! preview). The picture is copied with its smaller sizes (preview_mips.rs),
//! then the band is drawn again from them, a deeper row from a smaller size
//! (preview_veil.wgsl). Drawn only after the picture itself is drawn again,
//! so that it never blurs a blurred picture.

use crate::pipeline_cache::CachedPipelines;
use bytemuck::{Pod, Zeroable};

/// The band blurred: where it starts, from the top of the picture (0) to
/// its bottom (1), and how blurred its bottom row is (the size it reads:
/// each step halves the picture).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Veil {
    pub from: f32,
    pub depth: f32,
}

/// Smaller sizes than this many halvings add nothing a band shows.
const DEEPEST: u32 = 6;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniform {
    band: [f32; 4],
}

pub(crate) struct Pipeline {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
}

impl Pipeline {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Preview veil"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/preview_veil.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Preview veil"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
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
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Preview veil"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline_cached(&wgpu::RenderPipelineDescriptor {
            label: Some("Preview veil"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Preview veil"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        Self { layout, pipeline, sampler }
    }
}

/// A target's copy of its picture, with its smaller sizes.
pub(crate) struct Copy {
    size: [u32; 2],
    texture: wgpu::Texture,
    uniform: wgpu::Buffer,
    group: wgpu::BindGroup,
}

impl Copy {
    fn new(device: &wgpu::Device, pipeline: &Pipeline, size: [u32; 2]) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Preview veil copy"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: crate::preview_mips::levels(size).min(DEEPEST + 1),
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Preview veil"),
            size: std::mem::size_of::<Uniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let view = texture.create_view(&Default::default());
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Preview veil"),
            layout: &pipeline.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&pipeline.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: uniform.as_entire_binding(),
                },
            ],
        });
        Self {
            size,
            texture,
            uniform,
            group,
        }
    }
}

/// Blur the band of `veil` in `color`, a picture of `size` just drawn.
#[allow(clippy::too_many_arguments)]
pub(crate) fn record(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    encoder: &mut wgpu::CommandEncoder,
    pipeline: &mut Option<Pipeline>,
    mips: &crate::preview_mips::Mips,
    copy: &mut Option<Copy>,
    color: &wgpu::TextureView,
    size: [u32; 2],
    veil: Veil,
) {
    let pipeline = pipeline.get_or_insert_with(|| Pipeline::new(device));
    if copy.as_ref().is_none_or(|c| c.size != size) {
        *copy = Some(Copy::new(device, pipeline, size));
    }
    let Some(copy) = copy.as_ref() else { return };
    let levels = copy.texture.mip_level_count();
    let uniform = Uniform {
        band: [
            veil.from.clamp(0.0, 1.0),
            veil.depth.clamp(0.0, (levels - 1) as f32),
            1.0 / size[0] as f32,
            1.0 / size[1] as f32,
        ],
    };
    queue.write_buffer(&copy.uniform, 0, bytemuck::bytes_of(&uniform));
    encoder.copy_texture_to_texture(
        color.texture().as_image_copy(),
        copy.texture.as_image_copy(),
        wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
    );
    mips.record(device, encoder, &copy.texture);
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("Preview veil"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: color,
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
    // only the band is drawn again
    let top = ((veil.from.clamp(0.0, 1.0) * size[1] as f32) as u32).min(size[1].saturating_sub(1));
    pass.set_scissor_rect(0, top, size[0], size[1] - top);
    pass.set_pipeline(&pipeline.pipeline);
    pass.set_bind_group(0, &copy.group, &[]);
    pass.draw(0..3, 0..1);
}
