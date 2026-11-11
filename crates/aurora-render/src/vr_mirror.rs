//! One-eye desktop mirror: reuse a submitted eye image, never draw the world again.
use crate::pipeline_cache::CachedPipelines;
use std::time::{Duration, Instant};

pub(crate) struct Mirror {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    bindings: Vec<(wgpu::Texture, wgpu::BindGroup)>,
    config: wgpu::SurfaceConfiguration,
    configured: bool,
    last: Option<Instant>,
}
impl Mirror {
    pub fn new(device: &wgpu::Device, config: &wgpu::SurfaceConfiguration, modes: &[wgpu::PresentMode]) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("VR desktop mirror"),
            source: wgpu::ShaderSource::Wgsl(
                r#"
@group(0) @binding(0) var image: texture_2d<f32>;
@group(0) @binding(1) var linear_sampler: sampler;
struct Vertex { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> }
@vertex fn vs(@builtin(vertex_index) index: u32) -> Vertex {
    let p = array<vec2<f32>,3>(vec2(-1.,-1.),vec2(3.,-1.),vec2(-1.,3.));
    var v: Vertex; v.position=vec4(p[index],0.,1.);
    v.uv=vec2(p[index].x*0.5+0.5,0.5-p[index].y*0.5); return v;
}
@fragment fn fs(v: Vertex) -> @location(0) vec4<f32> {
    return vec4(textureSample(image,linear_sampler,v.uv).rgb,1.);
}
"#
                .into(),
            ),
        });
        let pipeline = device.create_render_pipeline_cached(&wgpu::RenderPipelineDescriptor {
            label: Some("VR mirror blit"),
            layout: None,
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
                    format: config.format,
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
        let layout = pipeline.get_bind_group_layout(0);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("VR mirror linear"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let mut config = config.clone();
        // Desktop FIFO must not pace a higher-refresh headset. This only changes
        // presentation during VR; Backend restores the normal configuration on exit.
        config.present_mode = crate::present::choose(modes, false, None);
        config.desired_maximum_frame_latency = 1;
        Self {
            pipeline,
            layout,
            sampler,
            bindings: Vec::new(),
            config,
            configured: false,
            last: None,
        }
    }
    pub fn present(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface: &wgpu::Surface<'_>,
        source: &wgpu::Texture,
        size: (u32, u32),
        egui: &mut egui_wgpu::Renderer,
        ui: Option<&crate::ui_frame::UiFrame>,
    ) -> bool {
        let (width, height) = size;
        if width == 0 || height == 0 || self.last.is_some_and(|t| t.elapsed() < Duration::from_secs_f64(1.0 / 30.0)) {
            return false;
        }
        self.last = Some(Instant::now());
        if !self.configured || self.config.width != width || self.config.height != height {
            self.config.width = width;
            self.config.height = height;
            crate::present::configure(surface, device, &self.config);
            self.configured = true;
        }
        let target = match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
                self.configured = false;
                return false;
            }
            _ => return false,
        };
        let slot = self.bindings.iter().position(|(texture, _)| texture == source).unwrap_or_else(|| {
            let view = source.create_view(&Default::default());
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("VR mirror eye"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });
            self.bindings.push((source.clone(), group));
            self.bindings.len() - 1
        });
        let view = target.texture.create_view(&Default::default());
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("VR mirror"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            // Letterbox instead of stretching or clipping the eye's field of view.
            let scale = (width as f32 / source.width() as f32).min(height as f32 / source.height() as f32);
            let w = source.width() as f32 * scale;
            let h = source.height() as f32 * scale;
            pass.set_viewport((width as f32 - w) * 0.5, (height as f32 - h) * 0.5, w, h, 0., 1.);
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bindings[slot].1, &[]);
            pass.draw(0..3, 0..1);
        }
        let mut commands = Vec::new();
        if let Some(ui) = ui {
            let screen = egui_wgpu::ScreenDescriptor {
                size_in_pixels: [width, height],
                pixels_per_point: ui.pixels_per_point,
            };
            commands = egui.update_buffers(device, queue, &mut encoder, &ui.primitives, &screen);
            let jobs = crate::ui_frame::paint_jobs(&ui.primitives, crate::ui_frame::meshes_uploaded(&ui.primitives));
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("VR desktop exit hint"),
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
                })
                .forget_lifetime();
            egui.render(&mut pass, &jobs, &screen);
        }
        // Return the sampled runtime image to its required layout before XR release.
        encoder.transition_resources(
            std::iter::empty(),
            std::iter::once(wgpu::TextureTransition {
                texture: source,
                selector: None,
                state: wgpu::TextureUses::COLOR_TARGET,
            }),
        );
        commands.push(encoder.finish());
        queue.submit(commands);
        queue.present(target);
        true
    }
}

pub(crate) const HEADSET_GRADE_SHADER: &str = r#"
@group(0) @binding(0) var image: texture_2d<f32>;
@group(0) @binding(1) var<uniform> grade: vec4<f32>;
struct Vertex { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> }
@vertex fn vs(@builtin(vertex_index) index: u32) -> Vertex {
    let p = array<vec2<f32>,3>(vec2(-1.,-1.),vec2(3.,-1.),vec2(-1.,3.));
    var v: Vertex; v.position=vec4(p[index],0.,1.);
    v.uv=vec2(p[index].x*0.5+0.5,0.5-p[index].y*0.5); return v;
}
fn decode_srgb(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c + vec3(0.055)) / 1.055, vec3(2.4)), c / 12.92, c <= vec3(0.04045));
}
fn encode_srgb(c: vec3<f32>) -> vec3<f32> {
    return select(1.055 * pow(max(c, vec3(0.0)), vec3(1.0 / 2.4)) - vec3(0.055), c * 12.92, c <= vec3(0.0031308));
}
@fragment fn fs(v: Vertex) -> @location(0) vec4<f32> {
    let sampled = textureLoad(image, vec2<i32>(v.position.xy), 0).rgb;
    let rgb = select(sampled, encode_srgb(sampled), grade.z > 0.5);
    let luma = dot(rgb, vec3(0.2126, 0.7152, 0.0722));
    let colour = clamp(mix(vec3(luma), rgb, grade.y), vec3(0.0), vec3(1.0));
    let encoded = pow(colour, vec3(1.0 / grade.x));
    return vec4(select(encoded, decode_srgb(encoded), grade.w > 0.5), 1.0);
}
"#;

/// Final headset-only colour pass. The desktop mirror samples the ungraded eye.
pub(crate) struct HeadsetGrade {
    pub sources: [wgpu::Texture; 2],
    pipeline: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    bindings: [wgpu::BindGroup; 2],
}

impl HeadsetGrade {
    pub fn new(device: &wgpu::Device, target: &wgpu::Texture) -> Self {
        let sources = std::array::from_fn(|_| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("VR ungraded eye"),
                size: target.size(),
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: target.format(),
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        });
        Self::from_sources(device, sources, target.format())
    }

    /// Reuse the session's staging eyes for colour-correct runtime submission.
    pub fn from_sources(device: &wgpu::Device, sources: [wgpu::Texture; 2], format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("VR headset grade"),
            source: wgpu::ShaderSource::Wgsl(HEADSET_GRADE_SHADER.into()),
        });
        let pipeline = device.create_render_pipeline_cached(&wgpu::RenderPipelineDescriptor {
            label: Some("VR headset grade"),
            layout: None,
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
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("VR grade parameters"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = pipeline.get_bind_group_layout(0);
        let bindings = std::array::from_fn(|i| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("VR grade eye"),
                layout: &layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&sources[i].create_view(&Default::default())),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: uniform.as_entire_binding(),
                    },
                ],
            })
        });
        Self {
            sources,
            pipeline,
            uniform,
            bindings,
        }
    }

    pub fn encode(&self, device: &wgpu::Device, queue: &wgpu::Queue, targets: &[wgpu::Texture; 2], grade: [f32; 2]) {
        let source_srgb = self.sources[0].format().is_srgb();
        let output_linear = targets[0].format().is_srgb() || targets[0].format() != self.sources[0].format();
        self.encode_colour(device, queue, targets, grade, [source_srgb, output_linear]);
    }

    pub fn encode_colour(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        targets: &[wgpu::Texture; 2],
        grade: [f32; 2],
        [source_srgb, output_linear]: [bool; 2],
    ) {
        let params = [
            grade[0],
            grade[1],
            if source_srgb { 1.0 } else { 0.0 },
            if output_linear { 1.0 } else { 0.0 },
        ];
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&params));
        let mut encoder = device.create_command_encoder(&Default::default());
        for (target, binding) in targets.iter().zip(&self.bindings) {
            let view = target.create_view(&Default::default());
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("VR headset grade"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, binding, &[]);
            pass.draw(0..3, 0..1);
        }
        queue.submit([encoder.finish()]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headset_colour_matches_srgb_and_linear_runtime_formats_on_gpu() {
        let _gpu = crate::test_gpu::lock();
        let instance = wgpu::Instance::default();
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
            return;
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).expect("colour GPU");
        let pixels = [[32_u8, 32, 32, 255], [128, 128, 128, 255], [200, 64, 32, 255], [32, 128, 200, 255]];
        let size = wgpu::Extent3d {
            width: 4,
            height: 1,
            depth_or_array_layers: 1,
        };
        let texture = |format| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("headset colour test"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::COPY_DST
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        };
        let decode = |c: f32| if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) };
        for source_format in [wgpu::TextureFormat::Rgba8Unorm, wgpu::TextureFormat::Rgba8UnormSrgb] {
            for target_format in [wgpu::TextureFormat::Rgba8Unorm, wgpu::TextureFormat::Rgba8UnormSrgb] {
                let sources = std::array::from_fn(|_| texture(source_format));
                for source in &sources {
                    queue.write_texture(
                        source.as_image_copy(),
                        bytemuck::cast_slice(&pixels),
                        wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(16),
                            rows_per_image: None,
                        },
                        size,
                    );
                }
                let pass = HeadsetGrade::from_sources(&device, sources, target_format);
                let targets = std::array::from_fn(|_| texture(target_format));
                for grade in [[1.0_f32, 1.0], [0.8, 1.1]] {
                    pass.encode_colour(&device, &queue, &targets, grade, [source_format.is_srgb(), true]);
                    let readback = device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("headset colour pixels"),
                        size: crate::readback::readback_size(256),
                        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    });
                    let mut encoder = device.create_command_encoder(&Default::default());
                    encoder.copy_texture_to_buffer(
                        targets[0].as_image_copy(),
                        wgpu::TexelCopyBufferInfo {
                            buffer: &readback,
                            layout: wgpu::TexelCopyBufferLayout {
                                offset: 0,
                                bytes_per_row: Some(256),
                                rows_per_image: None,
                            },
                        },
                        size,
                    );
                    queue.submit([encoder.finish()]);
                    readback.map_async(wgpu::MapMode::Read, .., |r| r.expect("colour mapping"));
                    device.poll(wgpu::PollType::wait_indefinitely()).expect("colour completion");
                    let bytes = readback.get_mapped_range(..).expect("colour pixels");
                    for (index, pixel) in pixels.iter().enumerate() {
                        let rgb = [pixel[0], pixel[1], pixel[2]].map(|v| f32::from(v) / 255.0);
                        let luma = rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722;
                        for channel in 0..3 {
                            let encoded = (luma + (rgb[channel] - luma) * grade[1]).clamp(0.0, 1.0).powf(1.0 / grade[0]);
                            let expected = if target_format.is_srgb() { encoded } else { decode(encoded) };
                            let error = (f32::from(bytes[index * 4 + channel]) - expected * 255.0).abs();
                            assert!(
                                error <= 2.0,
                                "{source_format:?} -> {target_format:?}, grade {grade:?}, pixel {index}, channel {channel}: error {error}"
                            );
                        }
                    }
                }
            }
        }
    }
}
