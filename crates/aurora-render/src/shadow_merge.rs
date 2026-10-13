//! Opaque shadow casters drawn in one cascade (`shadow_cull::path::MERGE`).
//!
//! A caster near the camera used to be drawn in every cascade its shadow
//! reaches: in a crowd, each avatar in two or three. The cascades look along
//! the same light, so the finer map already holds, inside its square, what
//! the coarser one would have drawn there. The culling leaves such casters
//! out of the coarser cascade (`shadow_cull::merged_from_finer`) and, between
//! the two cascades' draws, this module copies the finer map into its part
//! of the coarser one: read at the centre of each coarser texel — the point
//! a draw would have sampled — so the coarser map, and the image, stay what
//! they were. Two small passes per cascade pair (shadow_merge.wgsl): the
//! finer tile to a scratch map (the atlas cannot be read while it is drawn
//! into), then the scratch over the coarser tile, nearest depth kept.
//!
//! Only the opaque casters are shared: the alpha-tested ones are drawn
//! after the copy, in every cascade as before, since a finer map cuts their
//! texture out differently.

use bytemuck::{Pod, Zeroable};

use crate::shadow_cull::Merge;

/// Pairs of cascades merged: the first into the second, the second into
/// the third.
pub const STEPS: usize = 2;

/// Must match `MergeParams` in shadow_merge.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MergeU {
    part: [u32; 4],
    tiles: [u32; 4],
    to_fine: [f32; 4],
    depth: [f32; 4],
    size: [f32; 4],
}

pub struct ShadowMerge {
    extract: wgpu::RenderPipeline,
    merge: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    params: [wgpu::Buffer; STEPS],
    scratch: wgpu::TextureView,
    scratch_size: u32,
    /// Per step: the atlas for `fs_extract`, the scratch for `fs_merge`.
    groups: [(wgpu::BindGroup, wgpu::BindGroup); STEPS],
}

/// Atlas origin of a cascade's tile (two tiles a row).
fn tile_origin(cascade: usize, shadow_size: u32) -> [u32; 2] {
    [(cascade as u32 % 2) * shadow_size, (cascade as u32 / 2) * shadow_size]
}

impl ShadowMerge {
    pub fn new(device: &wgpu::Device, atlas: &wgpu::TextureView, shadow_size: u32, format: wgpu::TextureFormat) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shadow merge"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/shadow_merge.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shadow merge"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shadow merge"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |label, fs, compare| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_full"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[],
                }),
                primitive: Default::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(compare),
                    stencil: wgpu::StencilState::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let extract = pipeline("shadow merge: extract", "fs_extract", wgpu::CompareFunction::Always);
        // (the cascades' own compare: the nearest caster wins)
        let merge = pipeline("shadow merge: merge", "fs_merge", wgpu::CompareFunction::LessEqual);
        let params = std::array::from_fn(|_| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("shadow merge"),
                size: std::mem::size_of::<MergeU>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        });
        let (scratch, scratch_size) = Self::make_scratch(device, shadow_size, format);
        let groups = Self::make_groups(device, &layout, &params, atlas, &scratch);
        ShadowMerge {
            extract,
            merge,
            layout,
            params,
            scratch,
            scratch_size,
            groups,
        }
    }

    /// The scratch map: half a tile a side, which the finer square of a
    /// cascade pair fits in at any field of view the viewer offers (the
    /// squares grow by about 3.5 from one cascade to the next; a pair whose
    /// part is larger is not merged, `shadow_cull::merge_of`).
    fn make_scratch(device: &wgpu::Device, shadow_size: u32, format: wgpu::TextureFormat) -> (wgpu::TextureView, u32) {
        let size = (shadow_size / 2).max(1);
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shadow merge scratch"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        (tex.create_view(&wgpu::TextureViewDescriptor::default()), size)
    }

    fn make_groups(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        params: &[wgpu::Buffer; STEPS],
        atlas: &wgpu::TextureView,
        scratch: &wgpu::TextureView,
    ) -> [(wgpu::BindGroup, wgpu::BindGroup); STEPS] {
        std::array::from_fn(|step| {
            let group = |source: &wgpu::TextureView| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("shadow merge"),
                    layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: params[step].as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(source),
                        },
                    ],
                })
            };
            (group(atlas), group(scratch))
        })
    }

    /// Side of the scratch map in texels (`shadow_cull::merge_of`).
    pub fn scratch_size(&self) -> u32 {
        self.scratch_size
    }

    /// The shadow atlas was created again, at `shadow_size` texels a tile.
    pub fn atlas_changed(&mut self, device: &wgpu::Device, atlas: &wgpu::TextureView, shadow_size: u32, format: wgpu::TextureFormat) {
        (self.scratch, self.scratch_size) = Self::make_scratch(device, shadow_size, format);
        self.groups = Self::make_groups(device, &self.layout, &self.params, atlas, &self.scratch);
    }

    /// Parameters of a step of the frame: cascade `step` into cascade
    /// `step + 1`.
    pub fn write(&self, queue: &wgpu::Queue, step: usize, m: &Merge, shadow_size: u32) {
        let (fine, coarse) = (tile_origin(step, shadow_size), tile_origin(step + 1, shadow_size));
        let u = MergeU {
            part: [m.origin[0], m.origin[1], m.size[0], m.size[1]],
            tiles: [fine[0], fine[1], coarse[0], coarse[1]],
            to_fine: m.to_fine,
            depth: m.depth,
            size: [shadow_size as f32, 0.0, 0.0, 0.0],
        };
        queue.write_buffer(&self.params[step], 0, bytemuck::bytes_of(&u));
    }

    /// The finer tile of `step`, as its opaque casters left it, to the
    /// scratch map. Between two passes on the atlas.
    pub fn extract(&self, encoder: &mut wgpu::CommandEncoder, step: usize, m: &Merge) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("shadow merge: extract"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.scratch,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_viewport(0.0, 0.0, self.scratch_size as f32, self.scratch_size as f32, 0.0, 1.0);
        pass.set_scissor_rect(0, 0, m.size[0], m.size[1]);
        pass.set_pipeline(&self.extract);
        pass.set_bind_group(0, &self.groups[step].0, &[]);
        pass.draw(0..3, 0..1);
    }

    /// The scratch map over its part of the coarser tile of `step`, in the
    /// atlas pass. Leaves the pass with that part as its scissor.
    pub fn merge(&self, pass: &mut wgpu::RenderPass, step: usize, m: &Merge, shadow_size: u32) {
        let [x, y] = tile_origin(step + 1, shadow_size);
        pass.set_viewport(x as f32, y as f32, shadow_size as f32, shadow_size as f32, 0.0, 1.0);
        pass.set_scissor_rect(x + m.origin[0], y + m.origin[1], m.size[0], m.size[1]);
        pass.set_pipeline(&self.merge);
        pass.set_bind_group(0, &self.groups[step].1, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn shader_and_uniform_agree() {
        let src = include_str!("shaders/shadow_merge.wgsl");
        // five vectors, in the order of `MergeU`
        assert_eq!(std::mem::size_of::<super::MergeU>(), 5 * 16);
        let fields: Vec<usize> = ["part:", "tiles:", "to_fine:", "depth:", "size:"]
            .iter()
            .map(|f| src.find(f).expect("field of MergeParams"))
            .collect();
        assert!(fields.windows(2).all(|w| w[0] < w[1]));
        let mut validator = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all());
        let module = naga::front::wgsl::parse_str(src).expect("shadow_merge.wgsl parses");
        validator.validate(&module).expect("shadow_merge.wgsl is valid");
    }
}
