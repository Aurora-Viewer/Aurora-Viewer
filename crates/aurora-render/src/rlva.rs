//! RLVa sphere post effects (rlveffects.cpp and deferred/rlvF.glsl,
//! originally LGPL 2.1). Targets and passes exist only while an effect is active.
use crate::SphereEffect;
use crate::pipeline_cache::CachedPipelines;
use bytemuck::{Pod, Zeroable};
use glam::Mat4;
pub(crate) mod overlay;
pub(crate) mod resolution;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    inv_vp: [[f32; 4]; 4],
    origin: [f32; 4],
    range: [f32; 4],
    effect: [f32; 4],
    mode: [u32; 4],
    screen: [f32; 4],
}

struct Targets {
    size: (u32, u32),
    views: [wgpu::TextureView; 2],
}

pub(crate) struct Spheres {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    uniforms: Vec<wgpu::Buffer>,
    targets: Option<Targets>,
}

impl Spheres {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
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
            label: Some("RLVa spheres"),
            entries: &[
                texture(0, wgpu::TextureSampleType::Float { filterable: true }),
                texture(1, wgpu::TextureSampleType::Depth),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
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
            label: Some("RLVa spheres"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/rlva.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("RLVa spheres"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline_cached(&wgpu::RenderPipelineDescriptor {
            label: Some("RLVa spheres"),
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
        let uniforms = (0..12)
            .map(|_| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("RLVa sphere parameters"),
                    size: std::mem::size_of::<Params>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            })
            .collect();
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            layout,
            pipeline,
            sampler,
            uniforms,
            targets: None,
        }
    }

    pub fn prepare(&mut self, device: &wgpu::Device, size: (u32, u32), format: wgpu::TextureFormat) -> &wgpu::TextureView {
        if self.targets.as_ref().is_none_or(|t| t.size != size) {
            let views = std::array::from_fn(|_| {
                crate::renderer::make_view(
                    device,
                    "RLVa sphere target",
                    size.0,
                    size.1,
                    format,
                    1,
                    wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                )
                .1
            });
            self.targets = Some(Targets { size, views });
        }
        &self.targets.as_ref().expect("targets made above").views[0]
    }

    /// Runs on the composed scene, after exposure/glow/tonemapping. Scene alpha
    /// carries glow before that stage, so it must never contain a vision mask.
    #[allow(clippy::too_many_arguments)]
    pub fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        depth: &wgpu::TextureView,
        inv_vp: Mat4,
        size: (u32, u32),
        effects: &[SphereEffect],
        output: &wgpu::TextureView,
    ) -> usize {
        let targets = self.targets.as_ref().expect("prepare called before scene composition");
        let total = effects
            .iter()
            .take(6)
            .map(|effect| if matches!(effect.mode, 1 | 2) { 2 } else { 1 })
            .sum::<usize>();
        let mut source = &targets.views[0];
        let mut count = 0;
        for effect in effects.iter().take(6) {
            let passes = if matches!(effect.mode, 1 | 2) { 2 } else { 1 };
            for axis in 0..passes {
                let params = Params {
                    inv_vp: inv_vp.to_cols_array_2d(),
                    origin: effect.origin.extend(1.0).to_array(),
                    range: effect.range,
                    effect: effect.params,
                    mode: [effect.mode, effect.extend, axis, 0],
                    screen: [size.0 as f32, size.1 as f32, 0.0, 0.0],
                };
                queue.write_buffer(&self.uniforms[count], 0, bytemuck::bytes_of(&params));
                let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("RLVa sphere input"),
                    layout: &self.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(source),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(depth),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::Sampler(&self.sampler),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: self.uniforms[count].as_entire_binding(),
                        },
                    ],
                });
                let target = if count + 1 == total {
                    output
                } else {
                    &targets.views[(count + 1) % 2]
                };
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("RLVa sphere"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: target,
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
                pass.set_bind_group(0, &group, &[]);
                pass.draw(0..3, 0..1);
                drop(pass);
                source = target;
                count += 1;
            }
        }
        total
    }
}

#[cfg(test)]
#[path = "rlva/tests.rs"]
mod tests;
