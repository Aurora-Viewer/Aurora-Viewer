//! VR picture adjustments (exposure, brightness, contrast, saturation,
//! gamma) and the comfort vignette, applied to each finished eye image.
//!
//! The eye is copied aside, then drawn back through one full-screen pass.
//! Nothing runs while the preferences are neutral and the vignette is off,
//! so the default picture costs nothing.
use crate::VrDisplay;
use crate::pipeline_cache::CachedPipelines;
use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct Params {
    /// 2^exposure, brightness, contrast, saturation.
    a: [f32; 4],
    /// 1/gamma, vignette strength, unused, unused.
    b: [f32; 4],
}

pub(crate) struct Adjust {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    params: wgpu::Buffer,
    /// Per eye: the copy the pass reads, and its binding.
    scratch: Vec<(wgpu::Texture, wgpu::BindGroup)>,
    format: wgpu::TextureFormat,
}

impl Adjust {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("VR picture"),
            source: wgpu::ShaderSource::Wgsl(
                r#"
struct Params { a: vec4<f32>, b: vec4<f32> }
@group(0) @binding(0) var image: texture_2d<f32>;
@group(0) @binding(1) var<uniform> p: Params;
@vertex fn vs(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let v = array<vec2<f32>,3>(vec2(-1.,-1.),vec2(3.,-1.),vec2(-1.,3.));
    return vec4(v[index],0.,1.);
}
@fragment fn fs(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    let size = vec2<f32>(textureDimensions(image));
    var c = textureLoad(image, vec2<i32>(at.xy), 0).rgb;
    c = c * p.a.x;
    c = (c - 0.5) * p.a.z + 0.5 + p.a.y;
    let luma = dot(c, vec3(0.2126, 0.7152, 0.0722));
    c = mix(vec3(luma), c, p.a.w);
    c = pow(max(c, vec3(0.0)), vec3(p.b.x));
    // comfort vignette: the edges darken, the clear centre shrinking with
    // the strength
    let r = length((at.xy / size - 0.5) * 2.0);
    let inner = mix(1.5, 0.45, p.b.y);
    let dark = smoothstep(inner, inner + 0.35, r) * step(0.001, p.b.y);
    return vec4(clamp(c, vec3(0.0), vec3(1.0)) * (1.0 - dark), 1.0);
}
"#
                .into(),
            ),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("VR picture"),
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
                        min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<Params>() as u64),
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("VR picture"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline_cached(&wgpu::RenderPipelineDescriptor {
            label: Some("VR picture"),
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
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("VR picture"),
            size: std::mem::size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            layout,
            params,
            scratch: Vec::new(),
            format,
        }
    }

    /// Adjust both eyes in place; nothing when the picture is untouched.
    pub fn apply(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, eyes: [&wgpu::Texture; 2], display: VrDisplay) {
        let vignette = display.vignette.clamp(0.0, 1.0);
        if !display.adjusts() && vignette < 0.001 {
            return;
        }
        let params = Params {
            a: [
                2f32.powf(display.exposure.clamp(-8.0, 8.0)),
                display.brightness,
                display.contrast.max(0.0),
                display.saturation.max(0.0),
            ],
            b: [1.0 / display.gamma.max(0.05), vignette, 0.0, 0.0],
        };
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&params));
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("VR picture"),
        });
        for (i, eye) in eyes.into_iter().enumerate() {
            let size = eye.size();
            if self.scratch.get(i).is_none_or(|(t, _)| t.size() != size) {
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("VR picture copy"),
                    size,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: self.format,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                });
                let view = texture.create_view(&Default::default());
                let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("VR picture"),
                    layout: &self.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: self.params.as_entire_binding(),
                        },
                    ],
                });
                if i < self.scratch.len() {
                    self.scratch[i] = (texture, group);
                } else {
                    self.scratch.push((texture, group));
                }
            }
            let (copy, group) = &self.scratch[i];
            encoder.copy_texture_to_texture(eye.as_image_copy(), copy.as_image_copy(), size);
            let view = eye.create_view(&Default::default());
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("VR picture"),
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
            pass.set_bind_group(0, group, &[]);
            pass.draw(0..3, 0..1);
        }
        queue.submit([encoder.finish()]);
    }
}
