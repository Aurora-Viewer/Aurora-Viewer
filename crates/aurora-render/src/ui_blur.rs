//! Live modal backdrop: scene and lower UI layers are blurred on the GPU;
//! the modal and higher layers are painted afterwards at full resolution.
//! This presentation is an explicit Aurora UI choice, requested by the user.

use std::sync::Arc;

struct Boundary;

/// Place this before painting a modal on its layer. The marker survives
/// tessellation, so even meshes with the same texture cannot straddle it.
pub fn blurred_backdrop(ctx: &egui::Context, layer: egui::LayerId) {
    ctx.layer_painter(layer).add(egui::PaintCallback {
        rect: ctx.content_rect(),
        callback: Arc::new(Boundary),
    });
}

pub(crate) fn take_boundary(primitives: &mut Vec<egui::ClippedPrimitive>) -> Option<usize> {
    let at = primitives
        .iter()
        .position(|p| matches!(&p.primitive, egui::epaint::Primitive::Callback(c) if c.callback.is::<Boundary>()))?;
    primitives.remove(at);
    Some(at)
}

pub(crate) struct Blur {
    pub source: wgpu::Texture,
    pub view: wgpu::TextureView,
    size: [u32; 2],
    scale: f32,
    intermediate: wgpu::TextureView,
    pipeline: wgpu::RenderPipeline,
    groups: [wgpu::BindGroup; 2],
}

impl Blur {
    pub fn matches(&self, size: [u32; 2], scale: f32) -> bool {
        self.size == size && self.scale == scale
    }

    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, size: [u32; 2], scale: f32) -> Self {
        use wgpu::util::DeviceExt;
        let texture = |label, size: [u32; 2]| {
            device.create_texture(&wgpu::TextureDescriptor {
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
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        };
        let source = texture("UI backdrop", size);
        let view = source.create_view(&Default::default());
        let intermediate = texture("UI backdrop horizontal", size.map(|n| n.div_ceil(2))).create_view(&Default::default());
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("UI backdrop"),
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
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("UI backdrop"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        // Constant radius in UI points, independent of window size or DPI.
        let groups = [
            (&view, [3.0 * scale / size[0] as f32, 0.0, 0.0, 0.0]),
            (&intermediate, [0.0, 3.0 * scale / size[1] as f32, 0.0, 0.0]),
        ]
        .map(|(source, direction)| {
            let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("UI backdrop direction"),
                contents: bytemuck::cast_slice(&direction),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("UI backdrop"),
                layout: &layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(source),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: uniform.as_entire_binding(),
                    },
                ],
            })
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("UI backdrop"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/ui_blur.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("UI backdrop"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("UI backdrop"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
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
        Self {
            source,
            view,
            size,
            scale,
            intermediate,
            pipeline,
            groups,
        }
    }

    pub fn encode(&self, encoder: &mut wgpu::CommandEncoder, output: &wgpu::TextureView) {
        for (view, group) in [&self.intermediate, output].into_iter().zip(&self.groups) {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("UI backdrop blur"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, group, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tessellation_keeps_lower_layers_before_the_blur_and_modal_layers_after_it() {
        let ctx = egui::Context::default();
        let modal = egui::LayerId::new(egui::Order::Foreground, egui::Id::new("modal"));
        let output = ctx.run_ui(Default::default(), |_| {
            // Call order deliberately differs from layer order.
            blurred_backdrop(&ctx, modal);
            ctx.layer_painter(modal).rect_filled(
                egui::Rect::from_min_size(egui::pos2(20.0, 0.0), egui::vec2(20.0, 20.0)),
                0.0,
                egui::Color32::GREEN,
            );
            ctx.layer_painter(egui::LayerId::background()).rect_filled(
                egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(20.0, 20.0)),
                0.0,
                egui::Color32::RED,
            );
        });
        let mut primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
        let boundary = take_boundary(&mut primitives).expect("blur marker survives tessellation");
        let colors = |jobs: &[egui::ClippedPrimitive]| {
            jobs.iter()
                .flat_map(|p| match &p.primitive {
                    egui::epaint::Primitive::Mesh(mesh) => mesh.vertices.iter().map(|v| v.color).collect::<Vec<_>>(),
                    _ => Vec::new(),
                })
                .collect::<Vec<_>>()
        };
        assert!(colors(&primitives[..boundary]).contains(&egui::Color32::RED));
        assert!(!colors(&primitives[..boundary]).contains(&egui::Color32::GREEN));
        assert!(colors(&primitives[boundary..]).contains(&egui::Color32::GREEN));
        assert!(!colors(&primitives[boundary..]).contains(&egui::Color32::RED));
    }
}
