//! RenderResolutionDivisor, following LLPipeline::allocateScreenBuffer
//! (pipeline.cpp, originally LGPL 2.1). Only the 3D targets shrink; HUDs,
//! world labels, overlays and viewer controls retain their window resolution.
//! The user scale follows RenderResolutionMultiplier in the same reference:
//! an RLVa divisor takes precedence over the optional multiplier. User scaling
//! combines reconstruction and CAS sharpening; RLVa keeps nearest pixels.

pub(crate) fn size(window: (u32, u32), divisor: u32) -> (u32, u32) {
    let (w, h) = (window.0.max(1), window.1.max(1));
    let divisor = divisor.max(1).min(w.min(h).saturating_sub(1).max(1));
    ((w / divisor).max(1), (h / divisor).max(1))
}

pub(crate) fn pixel(pixel: (u32, u32), window: (u32, u32), scene: (u32, u32)) -> (u32, u32) {
    if pixel.0 >= window.0 || pixel.1 >= window.1 {
        return (u32::MAX, u32::MAX);
    }
    (
        (u64::from(pixel.0) * u64::from(scene.0) / u64::from(window.0.max(1))) as u32,
        (u64::from(pixel.1) * u64::from(scene.1) / u64::from(window.1.max(1))) as u32,
    )
}

pub(crate) fn render_size(window: (u32, u32), divisor: u32, percent: u32) -> (u32, u32) {
    let (w, h) = size(window, divisor);
    if divisor > 1 {
        return (w, h);
    }
    let percent = u64::from(percent.clamp(50, 100));
    (
        ((u64::from(w) * percent / 100) as u32).max(1),
        ((u64::from(h) * percent / 100) as u32).max(1),
    )
}

pub(crate) fn sharpening(strength: f32, hdr: bool, spatial_upscale: bool) -> (f32, f32) {
    let strength = if hdr { strength.clamp(0.0, 1.0) } else { 0.0 };
    if spatial_upscale { (0.0, strength) } else { (strength, 0.0) }
}

pub(crate) struct Scaled {
    pub smooth: bool,
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pipeline: wgpu::RenderPipeline,
    group: wgpu::BindGroup,
    params: wgpu::Buffer,
}

impl Scaled {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, size: (u32, u32), smooth: bool) -> Self {
        let (texture, view) = crate::renderer::make_view(
            device,
            "RLVa reduced scene",
            size.0,
            size.1,
            format,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("RLVa resolution"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/rlva_resolution.wgsl").into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("RLVa resolution"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions {
                    constants: &[
                        ("spatial_upscale", f64::from(smooth)),
                        ("encoded_input", f64::from(!format.is_srgb())),
                    ],
                    ..Default::default()
                },
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
            label: Some("world upscale sharpness"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("RLVa reduced scene"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: params.as_entire_binding(),
                },
            ],
        });
        Self {
            smooth,
            texture,
            view,
            pipeline,
            group,
            params,
        }
    }

    pub fn size(&self) -> (u32, u32) {
        (self.texture.width(), self.texture.height())
    }

    pub fn encode(&self, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, sharpness: f32) {
        queue.write_buffer(&self.params, 0, bytemuck::cast_slice(&[sharpness.clamp(0.0, 1.0), 0.0, 0.0, 0.0]));
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("RLVa scene upscale"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
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
        pass.set_bind_group(0, &self.group, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rlva_resolution_scales_real_targets_and_maps_window_picks_with_reference_bounds() {
        assert_eq!(size((1440, 1000), 4), (360, 250));
        assert_eq!(size((1441, 1001), 4), (360, 250));
        assert_eq!(size((1440, 1000), 0), (1440, 1000));
        assert_eq!(size((1440, 1000), u32::MAX), (1, 1));
        assert_eq!(size((1, 0), 4), (1, 1));
        assert_eq!(pixel((1439, 999), (1440, 1000), (360, 250)), (359, 249));
        assert_eq!(pixel((720, 500), (1440, 1000), (360, 250)), (180, 125));
        assert_eq!(pixel((1440, 0), (1440, 1000), (360, 250)), (u32::MAX, u32::MAX));
    }

    #[test]
    fn render_scale_preserves_native_size_and_rlva_restrictions() {
        assert_eq!(render_size((3456, 2112), 1, 100), (3456, 2112));
        assert_eq!(render_size((3456, 2112), 1, 50), (1728, 1056));
        assert_eq!(render_size((1441, 1001), 4, 75), (360, 250));
        assert_eq!(render_size((1, 0), u32::MAX, 0), (1, 1));
        assert_eq!(pixel((1728, 1056), (3456, 2112), (1728, 1056)), (864, 528));
    }

    #[test]
    fn sharpening_runs_once_and_hdr_off_disables_it() {
        assert_eq!(sharpening(0.4, true, false), (0.4, 0.0));
        assert_eq!(sharpening(0.4, true, true), (0.0, 0.4));
        assert_eq!(sharpening(1.5, true, true), (0.0, 1.0));
        assert_eq!(sharpening(-1.0, true, false), (0.0, 0.0));
        for scaled in [false, true] {
            assert_eq!(sharpening(0.8, false, scaled), (0.0, 0.0));
            assert_eq!(sharpening(0.0, true, scaled), (0.0, 0.0));
        }
    }
}
