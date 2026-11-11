//! The viewer's interface on a floating screen in VR.
//!
//! The frame's whole egui interface is drawn into a texture the size of the
//! desktop window, which is then drawn as a world-space rectangle in both
//! eyes, over the world and under the controllers and the beam. The viewer
//! places the rectangle and turns the beam's hit on it into pointer events.
use crate::VrPanel;
use crate::pipeline_cache::CachedPipelines;
use bytemuck::{Pod, Zeroable};
use glam::Mat4;

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct Vertex {
    position: [f32; 3],
    uv: [f32; 2],
}

const SLOT: u64 = 256;

pub(crate) struct Panel {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniforms: wgpu::Buffer,
    vertices: wgpu::Buffer,
    /// The interface image and its binding, remade when the window's size changes.
    target: Option<(wgpu::Texture, wgpu::BindGroup)>,
    format: wgpu::TextureFormat,
}

impl Panel {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("VR interface panel"),
            source: wgpu::ShaderSource::Wgsl(
                r#"
@group(0) @binding(0) var<uniform> view_proj: mat4x4<f32>;
@group(0) @binding(1) var image: texture_2d<f32>;
@group(0) @binding(2) var linear_sampler: sampler;
struct In { @location(0) position: vec3<f32>, @location(1) uv: vec2<f32> }
struct Out { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> }
@vertex fn vs(v: In) -> Out {
    var o: Out; o.position = view_proj * vec4(v.position, 1.0); o.uv = v.uv; return o;
}
@fragment fn fs(o: Out) -> @location(0) vec4<f32> { return textureSample(image, linear_sampler, o.uv); }
"#
                .into(),
            ),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("VR interface panel"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(64),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("VR interface panel"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline_cached(&wgpu::RenderPipelineDescriptor {
            label: Some("VR interface panel"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("VR interface panel"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("VR interface panel eyes"),
            size: SLOT * 2,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let vertices = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("VR interface panel quad"),
            size: (std::mem::size_of::<Vertex>() * 6) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            layout,
            sampler,
            uniforms,
            vertices,
            target: None,
            format,
        }
    }

    /// Draw the frame's interface into the panel's image, over a dark,
    /// slightly see-through background.
    pub fn paint(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        egui: &mut egui_wgpu::Renderer,
        ui: &crate::ui_frame::UiFrame,
        size: (u32, u32),
    ) {
        let (width, height) = (size.0.clamp(1, 4096), size.1.clamp(1, 4096));
        if self
            .target
            .as_ref()
            .is_none_or(|(t, _)| t.width() != width || t.height() != height)
        {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("VR interface panel image"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("VR interface panel"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &self.uniforms,
                            offset: 0,
                            size: wgpu::BufferSize::new(64),
                        }),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });
            self.target = Some((texture, group));
        }
        let Some((texture, _)) = &self.target else { return };
        let view = texture.create_view(&Default::default());
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("VR interface panel"),
        });
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [width, height],
            pixels_per_point: ui.pixels_per_point,
        };
        let mut commands = egui.update_buffers(device, queue, &mut encoder, &ui.primitives, &screen);
        {
            let jobs = crate::ui_frame::paint_jobs(&ui.primitives, crate::ui_frame::meshes_uploaded(&ui.primitives));
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("VR interface panel"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            // premultiplied: a dark blue-grey at 88%
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: 0.035,
                                g: 0.04,
                                b: 0.06,
                                a: 0.88,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                })
                .forget_lifetime();
            egui.render(&mut pass, &jobs, &screen);
        }
        commands.push(encoder.finish());
        queue.submit(commands);
    }

    /// Draw the panel's image in both eyes at `panel`.
    pub fn draw(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, eyes: [(&wgpu::Texture, Mat4); 2], panel: VrPanel) {
        let Some((_, group)) = &self.target else { return };
        let corner = |x: f32, y: f32, u: f32, v: f32| Vertex {
            position: (panel.center + panel.right * x + panel.up * y).to_array(),
            uv: [u, v],
        };
        let (tl, tr, br, bl) = (
            corner(-1.0, 1.0, 0.0, 0.0),
            corner(1.0, 1.0, 1.0, 0.0),
            corner(1.0, -1.0, 1.0, 1.0),
            corner(-1.0, -1.0, 0.0, 1.0),
        );
        queue.write_buffer(&self.vertices, 0, bytemuck::cast_slice(&[tl, tr, br, tl, br, bl]));
        for (i, (_, view_proj)) in eyes.iter().enumerate() {
            queue.write_buffer(&self.uniforms, SLOT * i as u64, bytemuck::bytes_of(&view_proj.to_cols_array()));
        }
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("VR interface panel"),
        });
        for (i, (texture, _)) in eyes.iter().enumerate() {
            let view = texture.create_view(&Default::default());
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("VR interface panel"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, group, &[(SLOT * i as u64) as u32]);
            pass.set_vertex_buffer(0, self.vertices.slice(..));
            pass.draw(0..6, 0..1);
        }
        queue.submit([encoder.finish()]);
    }
}
