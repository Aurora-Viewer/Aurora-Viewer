//! A cut-out photo (`FrameParams::cutout`, the viewer's photo isolation,
//! issue #977): the frame draws no sky, terrain or water, and the scene's
//! alpha, which otherwise holds the glow amount, holds what shows through
//! each pixel. This pass writes it once the opaque and masked faces are
//! drawn, from their depth: 1 where nothing was drawn, 0 elsewhere, sample
//! by sample under MSAA. The blended faces and the particles keep their
//! own alpha factors, which already multiply it by what they let through,
//! so no pipeline is duplicated; the glow is off in such a frame. The post
//! pass turns it into a coverage (post.wgsl, `PostParams::cutout`), SMAA
//! carries it, and the snapshot's readback keeps it.

/// The pipelines and their layouts, made on first use.
pub(crate) struct Cutout {
    single: (wgpu::BindGroupLayout, wgpu::RenderPipeline),
    /// The MSAA variant, for its sample count.
    msaa: Option<(u32, wgpu::BindGroupLayout, wgpu::RenderPipeline)>,
    module: wgpu::ShaderModule,
}

fn layout(device: &wgpu::Device, binding: u32, multisampled: bool) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("cutout depth"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Depth,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled,
            },
            count: None,
        }],
    })
}

fn pipeline(
    device: &wgpu::Device,
    module: &wgpu::ShaderModule,
    layout: &wgpu::BindGroupLayout,
    entry: &str,
    format: wgpu::TextureFormat,
    samples: u32,
) -> wgpu::RenderPipeline {
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("cutout"),
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("cutout coverage"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("vs_fullscreen"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(entry),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALPHA,
            })],
        }),
        primitive: Default::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState {
            count: samples,
            ..Default::default()
        },
        multiview_mask: None,
        cache: None,
    })
}

impl Cutout {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cutout"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/cutout.wgsl").into()),
        });
        let single_layout = layout(device, 0, false);
        let single = pipeline(device, &module, &single_layout, "fs_single", format, 1);
        Cutout {
            single: (single_layout, single),
            msaa: None,
            module,
        }
    }

    /// Write what shows through into the alpha of `color` (the scene's
    /// target, multisampled when `depth` is), from `depth`, the depth the
    /// scene pass ended with.
    pub fn encode(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        color: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        samples: u32,
    ) {
        let (layout, pipe, binding) = if samples > 1 {
            if self.msaa.as_ref().is_none_or(|(n, _, _)| *n != samples) {
                let l = layout(device, 1, true);
                let p = pipeline(device, &self.module, &l, "fs_msaa", format, samples);
                self.msaa = Some((samples, l, p));
            }
            let Some((_, l, p)) = &self.msaa else {
                return;
            };
            (l, p, 1)
        } else {
            (&self.single.0, &self.single.1, 0)
        };
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cutout depth"),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding,
                resource: wgpu::BindingResource::TextureView(depth),
            }],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("cutout coverage"),
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
        pass.set_pipeline(pipe);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
    }
}
