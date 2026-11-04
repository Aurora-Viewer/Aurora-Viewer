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
