//! The smaller sizes of a preview's textures: without them a fine pattern, or
//! the relief and the gloss of a material, sparkles once it is drawn small.

use crate::pipeline_cache::CachedPipelines;

const SRGB: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const LINEAR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// How many sizes a texture has, down to one pixel.
pub(crate) fn levels(size: [u32; 2]) -> u32 {
    32 - size[0].max(size[1]).max(1).leading_zeros()
}

pub(crate) struct Mips {
    layout: wgpu::BindGroupLayout,
    srgb: wgpu::RenderPipeline,
    linear: wgpu::RenderPipeline,
}

impl Mips {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Preview mips"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/preview_mips.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Preview mips"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Preview mips"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |format: wgpu::TextureFormat| {
            device.create_render_pipeline_cached(&wgpu::RenderPipelineDescriptor {
                label: Some("Preview mips"),
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
                    targets: &[Some(format.into())],
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        Self {
            srgb: pipeline(SRGB),
            linear: pipeline(LINEAR),
            layout,
        }
    }

    /// Record the smaller sizes of `texture`, its first one written.
    pub fn record(&self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, texture: &wgpu::Texture) {
        let pipeline = if texture.format() == SRGB { &self.srgb } else { &self.linear };
        let view = |level| {
            texture.create_view(&wgpu::TextureViewDescriptor {
                base_mip_level: level,
                mip_level_count: Some(1),
                ..Default::default()
            })
        };
        for level in 1..texture.mip_level_count() {
            let (from, to) = (view(level - 1), view(level));
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Preview mip"),
                layout: &self.layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&from),
                }],
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Preview mip"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &to,
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
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::levels;

    #[test]
    fn levels_go_down_to_one_pixel() {
        assert_eq!(levels([1, 1]), 1);
        assert_eq!(levels([512, 512]), 10);
        assert_eq!(levels([300, 20]), 9);
    }
}
