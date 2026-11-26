//! The see-through faces of a studio preview (hair, veils, lenses) in their
//! exact order, pixel by pixel, as the world draws its own: the lists and the
//! shader functions of `blend_exact`. Without them the faces came in the order
//! of their file, one layer of hair over another that lies in front of it.
//! Where the device has not what it takes, `blended` of preview.rs draws them.

use crate::blend_exact::{self, Lists};
use crate::pipeline_cache::CachedPipelines;

const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// The pipelines of one sample count of the opaque faces.
pub(crate) struct Order {
    once: wgpu::RenderPipeline,
    compose: wgpu::RenderPipeline,
    depth_layout: wgpu::BindGroupLayout,
    solid_layout: wgpu::BindGroupLayout,
}

/// What one preview holds for it.
pub(crate) struct Exact {
    size: [u32; 2],
    samples: u32,
    lists: Lists,
    /// What the opaque faces drew, under the others.
    pub solid: wgpu::TextureView,
    solid_group: wgpu::BindGroup,
    depth_group: wgpu::BindGroup,
}

/// The shader that lights and files a fragment.
fn once_source(samples: u32) -> String {
    let depth = if samples > 1 {
        "texture_depth_multisampled_2d"
    } else {
        "texture_depth_2d"
    };
    [
        include_str!("shaders/preview.wgsl"),
        &blend_exact::order_source(),
        &format!("const SAMPLES: i32 = {samples};\n"),
        &include_str!("shaders/preview_blend.wgsl").replace("DEPTH_TEXTURE", depth),
    ]
    .join("\n")
}

fn compose_source() -> String {
    [&blend_exact::order_source(), include_str!("shaders/preview_compose.wgsl")].join("\n")
}

impl Order {
    /// `material` and `shadow`: groups 0 and 1 of preview.wgsl.
    pub fn new(
        device: &wgpu::Device,
        material: &wgpu::BindGroupLayout,
        shadow: &wgpu::BindGroupLayout,
        vertices: wgpu::VertexBufferLayout<'static>,
        samples: u32,
    ) -> Self {
        let texture = |label, sample_type, multisampled| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some(label),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled,
                    },
                    count: None,
                }],
            })
        };
        let depth_layout = texture("Preview depth read", wgpu::TextureSampleType::Depth, samples > 1);
        let solid_layout = texture("Preview solid", wgpu::TextureSampleType::Float { filterable: false }, false);
        let [write, read] = blend_exact::layouts(device);
        let layout = |label, groups: &[Option<&wgpu::BindGroupLayout>]| {
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: groups,
                immediate_size: 0,
            })
        };
        let once_layout = layout(
            "Preview blend once",
            &[Some(material), Some(shadow), Some(&depth_layout), Some(&write)],
        );
        let compose_layout = layout("Preview compose", &[Some(&solid_layout), None, None, Some(&read)]);
        let module = |label, source: String| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            })
        };
        let once = module("Preview blend once", once_source(samples));
        let compose = module("Preview compose", compose_source());
        let target = |format, blend| {
            Some(wgpu::ColorTargetState {
                format,
                blend,
                write_mask: wgpu::ColorWrites::ALL,
            })
        };
        let pipeline = |label, layout, module, vertex, buffers: &[Option<wgpu::VertexBufferLayout>], fragment, targets: &[_]| {
            device.create_render_pipeline_cached(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module,
                    entry_point: Some(vertex),
                    compilation_options: Default::default(),
                    buffers,
                },
                fragment: Some(wgpu::FragmentState {
                    module,
                    entry_point: Some(fragment),
                    compilation_options: Default::default(),
                    targets,
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        Self {
            once: pipeline(
                "Preview blend once",
                &once_layout,
                &once,
                "vs",
                &[Some(vertices)],
                "fs_once",
                &[
                    target(blend_exact::HAZE_FORMAT, Some(blend_exact::once_blend())),
                    target(blend_exact::range_format(device.features()), Some(blend_exact::range_blend())),
                ],
            ),
            compose: pipeline(
                "Preview compose",
                &compose_layout,
                &compose,
                "vs_screen",
                &[],
                "fs_compose",
                &[target(wgpu::TextureFormat::Rgba8Unorm, None)],
            ),
            depth_layout,
            solid_layout,
        }
    }
}

impl Exact {
    /// Whether the device has what it takes.
    pub fn possible(device: &wgpu::Device) -> bool {
        device.features().contains(blend_exact::EARLY_TEST)
    }

    /// None when the device or its memory refuses.
    pub fn new(device: &wgpu::Device, order: &Order, size: [u32; 2], samples: u32, depth: &wgpu::TextureView) -> Option<Self> {
        let (lists, refused) = crate::gpu_alloc::scoped(device, || Lists::new(device, (size[0], size[1]), DEPTH));
        let lists = lists.filter(|_| !refused)?;
        let solid = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("Preview solid"),
                size: wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default());
        let group = |label, layout, view| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(view),
                }],
            })
        };
        Some(Self {
            size,
            samples,
            lists,
            solid_group: group("Preview solid", &order.solid_layout, &solid),
            depth_group: group("Preview depth read", &order.depth_layout, depth),
            solid,
        })
    }

    /// It is for a picture of this size.
    pub fn fits(&self, size: [u32; 2], samples: u32) -> bool {
        (self.size, self.samples) == (size, samples)
    }

    /// The pool of the lists as the last picture asked for it.
    pub fn follow(&mut self, device: &wgpu::Device) {
        self.lists.after_submit();
        let Some(pages) = self.lists.pool_wanted(device) else { return };
        let (lists, refused) = crate::gpu_alloc::scoped(device, || Lists::with_pool(device, (self.size[0], self.size[1]), DEPTH, pages));
        match lists.filter(|_| !refused) {
            Some(lists) => self.lists = lists,
            None => self.lists.pool_refused(),
        }
    }

    /// A larger pool is on its way: the picture is worth drawing again.
    pub fn short(&self, device: &wgpu::Device) -> bool {
        self.lists.after_submit();
        self.lists.pool_wanted(device).is_some()
    }

    /// File the faces `draw` draws, and compose them over `solid` into `color`.
    pub fn record(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        order: &Order,
        shadow: &wgpu::BindGroup,
        color: &wgpu::TextureView,
        draw: impl FnOnce(&mut wgpu::RenderPass<'_>),
    ) {
        self.lists.clear(encoder);
        let attached = |view| {
            Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })
        };
        let pass = |label, targets: &[_], encoder: &mut wgpu::CommandEncoder| {
            encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some(label),
                    color_attachments: targets,
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime()
        };
        {
            let mut pass = pass(
                "Preview blend once",
                &[attached(&self.lists.haze), attached(&self.lists.range)],
                encoder,
            );
            pass.set_pipeline(&order.once);
            pass.set_bind_group(1, shadow, &[]);
            pass.set_bind_group(2, &self.depth_group, &[]);
            pass.set_bind_group(3, &self.lists.write, &[]);
            draw(&mut pass);
        }
        self.lists.count_pool(encoder);
        let mut pass = pass("Preview compose", &[attached(color)], encoder);
        pass.set_pipeline(&order.compose);
        pass.set_bind_group(0, &self.solid_group, &[]);
        pass.set_bind_group(3, &self.lists.read, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_shaders_of_the_previews_order_are_valid() {
        for source in [super::once_source(1), super::once_source(4), super::compose_source()] {
            let module = naga::front::wgsl::parse_str(&source).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
            naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
                .validate(&module)
                .unwrap_or_else(|e| panic!("{e:?}"));
        }
    }
}
