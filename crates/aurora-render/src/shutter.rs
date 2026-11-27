//! The Director's shutter, Aurora's own (the Legacy Viewer records no
//! film): an offline film frame drawn as several sub-frames over the time
//! the shutter stays open, each a whole frame of the world at its own
//! instant, averaged as a camera's sensor gathers the light while it is
//! open (`shaders/shutter.wgsl`). The sub-frames' pictures are added up in
//! linear light into a target of 16-bit floats; the last one writes the
//! average back into its own picture, which is read back as any film
//! frame (film_readback.rs). Only the last sub-frame is answered.
//!
//! The pictures are averaged once tone mapped, as the film keeps them: a
//! highlight clipped to white streaks as white, never brighter. Its target
//! exists only while a render uses it.

use crate::film_readback::Shutter;
use crate::pipeline_cache::CachedPipelines;
use bytemuck::{Pod, Zeroable};

/// The sum of the sub-frames, in linear light.
const SUM_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ShutterU {
    params: [f32; 4],
}

/// The uniforms for `count` sub-frames of a picture in `format`.
fn uniforms(count: u32, format: wgpu::TextureFormat) -> ShutterU {
    ShutterU {
        params: [1.0 / count.max(1) as f32, if by_hand(format) { 1.0 } else { 0.0 }, 0.0, 0.0],
    }
}

/// Whether the picture's texels are sRGB values the shader reads and
/// writes as they are (a Unorm format holding the encoded image), rather
/// than through the format's own sRGB conversion.
fn by_hand(format: wgpu::TextureFormat) -> bool {
    !format.is_srgb()
}

pub(crate) struct ShutterAccum {
    layout: wgpu::BindGroupLayout,
    module: wgpu::ShaderModule,
    pipeline_layout: wgpu::PipelineLayout,
    buffer: wgpu::Buffer,
    add: wgpu::RenderPipeline,
    /// The resolve's pipeline for each picture format met.
    resolve: Vec<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
    /// The sum and its size.
    sum: Option<((u32, u32), wgpu::TextureView)>,
}

impl ShutterAccum {
    pub(crate) fn new(device: &wgpu::Device) -> ShutterAccum {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shutter"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
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
            label: Some("shutter params"),
            size: std::mem::size_of::<ShutterU>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shutter"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/shutter.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shutter"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let add = pipeline(device, &module, &pipeline_layout, "fs_add", SUM_FORMAT, Some(add_blend()));
        ShutterAccum {
            layout,
            module,
            pipeline_layout,
            buffer,
            add,
            resolve: Vec::new(),
            sum: None,
        }
    }

    /// Whether the sum for `size` is made.
    pub(crate) fn has_target(&self, size: (u32, u32)) -> bool {
        self.sum.as_ref().is_some_and(|(s, _)| *s == size)
    }

    /// The sum for `size`, made if needed (the renderer makes it first,
    /// checked: renderer.rs `first_use`).
    pub(crate) fn ensure_target(&mut self, device: &wgpu::Device, size: (u32, u32)) {
        if !self.has_target(size) {
            self.sum = None;
            let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
            let view = crate::renderer::make_view(device, "shutter sum", size.0, size.1, SUM_FORMAT, 1, usage).1;
            self.sum = Some((size, view));
        }
    }

    /// The sum is let go (the render is over, or memory is short).
    pub(crate) fn release(&mut self) {
        self.sum = None;
    }

    /// Sub-frame `shutter` of the picture `texture` added to the sum (the
    /// first starts it); the last writes the average back into `texture`.
    /// The sum is made by the caller (`ensure_target`).
    pub(crate) fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        texture: &wgpu::Texture,
        shutter: Shutter,
    ) {
        let size = (texture.width(), texture.height());
        let format = texture.format();
        if !self.has_target(size) {
            return;
        }
        queue.write_buffer(&self.buffer, 0, bytemuck::bytes_of(&uniforms(shutter.count, format)));
        if !self.resolve.iter().any(|(f, _)| *f == format) {
            let p = pipeline(device, &self.module, &self.pipeline_layout, "fs_resolve", format, None);
            self.resolve.push((format, p));
        }
        let Some((_, sum)) = &self.sum else {
            return;
        };
        let picture = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let group = |view: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("shutter"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: self.buffer.as_entire_binding(),
                    },
                ],
            })
        };
        let run = |encoder: &mut wgpu::CommandEncoder,
                   label: &str,
                   target: &wgpu::TextureView,
                   load: wgpu::LoadOp<wgpu::Color>,
                   pipe: &wgpu::RenderPipeline,
                   bg: &wgpu::BindGroup| {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(label),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipe);
            pass.set_bind_group(0, bg, &[]);
            pass.draw(0..3, 0..1);
        };
        let load = if shutter.index == 0 {
            wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
        } else {
            wgpu::LoadOp::Load
        };
        run(encoder, "shutter add", sum, load, &self.add, &group(&picture));
        if shutter.last() {
            let resolve = &self.resolve.iter().find(|(f, _)| *f == format).expect("made above").1;
            run(encoder, "shutter resolve", &picture, wgpu::LoadOp::Load, resolve, &group(sum));
        }
    }
}

fn add_blend() -> wgpu::BlendState {
    let add = wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Add,
    };
    wgpu::BlendState { color: add, alpha: add }
}

fn pipeline(
    device: &wgpu::Device,
    module: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    fs: &str,
    format: wgpu::TextureFormat,
    blend: Option<wgpu::BlendState>,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline_cached(&wgpu::RenderPipelineDescriptor {
        label: Some(fs),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("vs_shutter"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(fs),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shader_is_valid_for_vulkan_and_metal_and_reads_the_uniforms_as_written() {
        let source = include_str!("shaders/shutter.wgsl");
        let module = naga::front::wgsl::parse_str(source).unwrap_or_else(|e| panic!("{}", e.emit_to_string(source)));
        let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::empty())
            .validate(&module)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(source)));
        assert_eq!(module.entry_points.len(), 3);
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
            .find(|(_, t)| t.name.as_deref() == Some("Shutter"))
            .map(|(_, t)| match &t.inner {
                naga::TypeInner::Struct { span, .. } => *span,
                other => panic!("{other:?}"),
            })
            .expect("Shutter");
        assert_eq!(span as usize, std::mem::size_of::<ShutterU>());
    }

    #[test]
    fn each_sub_frame_weighs_its_share_and_a_unorm_picture_is_converted_by_hand() {
        let u = uniforms(8, wgpu::TextureFormat::Bgra8Unorm);
        assert_eq!(u.params[..2], [0.125, 1.0]);
        let u = uniforms(0, wgpu::TextureFormat::Rgba8UnormSrgb);
        assert_eq!(u.params[..2], [1.0, 0.0]);
    }

    #[test]
    fn only_the_last_sub_frame_ends_the_frame() {
        assert!(!Shutter { index: 0, count: 4 }.last());
        assert!(Shutter { index: 3, count: 4 }.last());
        assert!(Shutter { index: 0, count: 1 }.last());
        assert!(Shutter { index: 0, count: 0 }.last());
    }
}
