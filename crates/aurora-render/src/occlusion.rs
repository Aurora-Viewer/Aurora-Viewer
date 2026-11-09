//! GPU occlusion culling (shaders/occlusion.wgsl): a Hi-Z pyramid of the depth
//! prepass and a compute test of each draw's bounding sphere, in two phases so
//! nothing pops: the prepass first draws what was visible last frame, the
//! pyramid is built from it, then the remaining visible draws are added in a
//! second prepass. The main passes read `instance_count` 0 for hidden draws.

use crate::pipeline_cache::CachedPipelines;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bytemuck::{Pod, Zeroable};

use crate::types::DrawIndexedIndirect;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CullParams {
    view_proj: [[f32; 4]; 4],
    screen: [f32; 2],
    levels: u32,
    count: u32,
}

/// Per main draw: bounding sphere and how it is tested.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct CullDraw {
    pub sphere: [f32; 4],
    pub kind: u32,
    pub _pad: [u32; 3],
}

/// Never tested (terrain, water, draws without bounds).
pub(crate) const KIND_NONE: u32 = 0;
/// Depth prepass draw, both phases.
pub(crate) const KIND_PREPASS: u32 = 1;
/// Tested only (blended, glow).
pub(crate) const KIND_TESTED: u32 = 2;

const HIZ_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;

struct Hiz {
    size: (u32, u32),
    levels: u32,
    /// Whole pyramid, sampled by the cull pass.
    view: wgpu::TextureView,
    /// [level]: level 0 from the depth, then each from the previous one.
    build_bgs: Vec<wgpu::BindGroup>,
    dims: Vec<(u32, u32)>,
}

pub(crate) struct Occlusion {
    first_layout: wgpu::BindGroupLayout,
    down_layout: wgpu::BindGroupLayout,
    cull_layout: wgpu::BindGroupLayout,
    first_pipeline: wgpu::ComputePipeline,
    down_pipeline: wgpu::ComputePipeline,
    prepare_pipeline: wgpu::ComputePipeline,
    cull_pipeline: wgpu::ComputePipeline,
    params: wgpu::Buffer,
    draws: wgpu::Buffer,
    pub pre1: wgpu::Buffer,
    pub pre2: wgpu::Buffer,
    visibility: wgpu::Buffer,
    hiz: Option<Hiz>,
    count: u32,
    /// Hidden draw counters and their readbacks (buffer, ready, pending).
    hidden: wgpu::Buffer,
    readback: Vec<(wgpu::Buffer, Arc<AtomicBool>, bool)>,
    /// Slot copied into by this frame, mapped after the submit.
    copied: Option<usize>,
    /// Draws hidden in the last measured frame.
    pub last_hidden: u32,
}

fn storage_entry(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn texture_entry(binding: u32, sample_type: wgpu::TextureSampleType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn hiz_dst_entry() -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding: 2,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format: HIZ_FORMAT,
            view_dimension: wgpu::TextureViewDimension::D2,
        },
        count: None,
    }
}

fn args_buffer(device: &wgpu::Device, label: &str, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage: wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::STORAGE,
        mapped_at_creation: false,
    })
}

fn buf(binding: u32, b: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: b.as_entire_binding(),
    }
}

fn groups(n: u32, size: u32) -> u32 {
    n.div_ceil(size).max(1)
}

impl Occlusion {
    pub fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("occlusion"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "{}
{}",
                    include_str!("shaders/hiz_test.wgsl"),
                    include_str!("shaders/occlusion.wgsl")
                )
                .into(),
            ),
        });
        let first_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("hiz first"),
            entries: &[texture_entry(0, wgpu::TextureSampleType::Depth), hiz_dst_entry()],
        });
        let down_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("hiz down"),
            entries: &[
                texture_entry(1, wgpu::TextureSampleType::Float { filterable: false }),
                hiz_dst_entry(),
            ],
        });
        let cull_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("occlusion cull"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 10,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                storage_entry(11, true),
                storage_entry(12, false),
                storage_entry(13, false),
                storage_entry(14, false),
                storage_entry(15, false),
                texture_entry(16, wgpu::TextureSampleType::Float { filterable: false }),
                storage_entry(17, false),
            ],
        });
        let pipeline = |layout: &wgpu::BindGroupLayout, entry: &str| {
            let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(entry),
                bind_group_layouts: &[Some(layout)],
                immediate_size: 0,
            });
            device.create_compute_pipeline_cached(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&pl),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let first_pipeline = pipeline(&first_layout, "cs_hiz_first");
        let down_pipeline = pipeline(&down_layout, "cs_hiz_down");
        let prepare_pipeline = pipeline(&cull_layout, "cs_prepare");
        let cull_pipeline = pipeline(&cull_layout, "cs_cull");
        let params = crate::gpu_alloc::buffer_init(
            device,
            "occlusion params",
            bytemuck::bytes_of(&CullParams::zeroed()),
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        );
        let cap = 4096u64;
        let draws = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("occlusion draws"),
            size: cap * std::mem::size_of::<CullDraw>() as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let args = cap * std::mem::size_of::<DrawIndexedIndirect>() as u64;
        Self {
            first_layout,
            down_layout,
            cull_layout,
            first_pipeline,
            down_pipeline,
            prepare_pipeline,
            cull_pipeline,
            params,
            draws,
            pre1: args_buffer(device, "prepass phase 1", args),
            pre2: args_buffer(device, "prepass phase 2", args),
            visibility: Self::make_visibility(device, 16384),
            hiz: None,
            count: 0,
            hidden: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("occlusion hidden"),
                size: 8,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            readback: (0..3)
                .map(|_| {
                    let b = device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("occlusion readback"),
                        size: crate::readback::readback_size(8),
                        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    });
                    (b, Arc::new(AtomicBool::new(false)), false)
                })
                .collect(),
            copied: None,
            last_hidden: 0,
        }
    }

    /// Collect the finished counter readbacks.
    pub fn harvest(&mut self) {
        for (buf, ready, pending) in self.readback.iter_mut() {
            if *pending && ready.load(Ordering::Acquire) {
                if let Ok(view) = buf.get_mapped_range(..) {
                    let n = crate::readback::u32_array::<2>(&view[..8]);
                    self.last_hidden = n[0] + n[1];
                }
                buf.unmap();
                *pending = false;
                ready.store(false, Ordering::Release);
            }
        }
    }

    /// Map this frame's counter copy (after the submit).
    pub fn after_submit(&mut self) {
        let Some(slot) = self.copied.take() else {
            return;
        };
        let (buf, ready, pending) = &mut self.readback[slot];
        let ready = ready.clone();
        *pending = true;
        buf.map_async(wgpu::MapMode::Read, .., move |r| {
            if r.is_ok() {
                ready.store(true, Ordering::Release);
            }
        });
    }

    fn make_visibility(device: &wgpu::Device, records: u64) -> wgpu::Buffer {
        // zero: nothing visible yet, the first frame draws all in phase 2
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("occlusion visibility"),
            size: records.max(1) * 4,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        })
    }

    /// Hi-Z pyramid for a depth of `width` x `height` (rebuilt on resize).
    fn ensure_hiz(&mut self, device: &wgpu::Device, depth: &wgpu::TextureView, width: u32, height: u32) {
        let size = (width.div_ceil(2).max(1), height.div_ceil(2).max(1));
        if self.hiz.as_ref().is_some_and(|h| h.size == size) {
            return;
        }
        let levels = 32 - size.0.max(size.1).leading_zeros();
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("hi-z"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HIZ_FORMAT,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let level_view = |l: u32| {
            tex.create_view(&wgpu::TextureViewDescriptor {
                label: Some("hi-z level"),
                base_mip_level: l,
                mip_level_count: Some(1),
                ..Default::default()
            })
        };
        let mut build_bgs = Vec::new();
        let mut dims = Vec::new();
        for l in 0..levels {
            dims.push(((size.0 >> l).max(1), (size.1 >> l).max(1)));
            let dst = level_view(l);
            let bg = if l == 0 {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("hi-z first"),
                    layout: &self.first_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(depth),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::TextureView(&dst),
                        },
                    ],
                })
            } else {
                let src = level_view(l - 1);
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("hi-z down"),
                    layout: &self.down_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(&src),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::TextureView(&dst),
                        },
                    ],
                })
            };
            build_bgs.push(bg);
        }
        let view = tex.create_view(&wgpu::TextureViewDescriptor {
            label: Some("hi-z"),
            ..Default::default()
        });
        self.hiz = Some(Hiz {
            size,
            levels,
            view,
            build_bgs,
            dims,
        });
    }

    /// Visible-last-frame flags by record (also read and written by the
    /// GPU draw lists, gpu_cull.rs).
    pub fn visibility(&self) -> &wgpu::Buffer {
        &self.visibility
    }

    /// Camera, screen and pyramid levels of this frame's test (uniform).
    pub fn params(&self) -> &wgpu::Buffer {
        &self.params
    }

    /// The whole Hi-Z pyramid, once `prepare` built it.
    pub fn hiz_view(&self) -> Option<&wgpu::TextureView> {
        self.hiz.as_ref().map(|h| &h.view)
    }

    /// Drop the pyramid (its bind groups hold the old depth after a resize).
    pub fn invalidate(&mut self) {
        self.hiz = None;
    }

    /// Upload this frame's draws; `records` sizes the visibility flags.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        depth: &wgpu::TextureView,
        width: u32,
        height: u32,
        view_proj: glam::Mat4,
        draws: &[CullDraw],
        records: usize,
    ) {
        self.ensure_hiz(device, depth, width, height);
        let need = draws.len().max(1) as u64;
        if need * std::mem::size_of::<CullDraw>() as u64 > self.draws.size() {
            let cap = need.next_power_of_two();
            self.draws = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("occlusion draws"),
                size: cap * std::mem::size_of::<CullDraw>() as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let args = cap * std::mem::size_of::<DrawIndexedIndirect>() as u64;
            self.pre1 = args_buffer(device, "prepass phase 1", args);
            self.pre2 = args_buffer(device, "prepass phase 2", args);
        }
        if records as u64 * 4 > self.visibility.size() {
            self.visibility = Self::make_visibility(device, (records as u64).next_power_of_two());
        }
        if !draws.is_empty() {
            queue.write_buffer(&self.draws, 0, bytemuck::cast_slice(draws));
        }
        self.count = draws.len() as u32;
        let levels = self.hiz.as_ref().map_or(1, |h| h.levels);
        let p = CullParams {
            view_proj: view_proj.to_cols_array_2d(),
            screen: [width as f32, height as f32],
            levels,
            count: self.count,
        };
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&p));
    }

    fn cull_bg(&self, device: &wgpu::Device, main_args: &wgpu::Buffer) -> Option<wgpu::BindGroup> {
        let hiz = self.hiz.as_ref()?;
        Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("occlusion cull"),
            layout: &self.cull_layout,
            entries: &[
                buf(10, &self.params),
                buf(11, &self.draws),
                buf(12, main_args),
                buf(13, &self.pre1),
                buf(14, &self.pre2),
                buf(15, &self.visibility),
                wgpu::BindGroupEntry {
                    binding: 16,
                    resource: wgpu::BindingResource::TextureView(&hiz.view),
                },
                buf(17, &self.hidden),
            ],
        }))
    }

    /// Phase 1 arguments: what was visible last frame.
    pub fn encode_prepare(&self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, main_args: &wgpu::Buffer) {
        let Some(bg) = self.cull_bg(device, main_args) else {
            return;
        };
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("occlusion prepare"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.prepare_pipeline);
        pass.set_bind_group(0, &bg, &[]);
        pass.dispatch_workgroups(groups(self.count, 64), 1, 1);
    }

    /// Hi-Z of the phase 1 depth, then the test of every draw.
    pub fn encode_cull(&mut self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, main_args: &wgpu::Buffer) {
        let (Some(hiz), Some(bg)) = (self.hiz.as_ref(), self.cull_bg(device, main_args)) else {
            return;
        };
        encoder.clear_buffer(&self.hidden, 0, None);
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("occlusion"),
            timestamp_writes: None,
        });
        for (l, build) in hiz.build_bgs.iter().enumerate() {
            let (w, h) = hiz.dims[l];
            pass.set_pipeline(if l == 0 { &self.first_pipeline } else { &self.down_pipeline });
            pass.set_bind_group(0, build, &[]);
            pass.dispatch_workgroups(groups(w, 8), groups(h, 8), 1);
        }
        pass.set_pipeline(&self.cull_pipeline);
        pass.set_bind_group(0, &bg, &[]);
        pass.dispatch_workgroups(groups(self.count, 64), 1, 1);
        drop(pass);
        if let Some(slot) = self.readback.iter().position(|r| !r.2) {
            encoder.copy_buffer_to_buffer(&self.hidden, 0, &self.readback[slot].0, 0, 8);
            self.copied = Some(slot);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline_cache::CachedPipelines;
    use glam::{Mat4, Vec3};
    use wgpu::util::DeviceExt;

    /// The occlusion test of `spheres` (camera at the origin looking down
    /// -Z) against a depth of `w` x `h` whose columns left of `wall` hold a
    /// near occluder and the others nothing.
    fn visible(w: u32, h: u32, wall: u32, proj: Mat4, spheres: &[(Vec3, f32)]) -> Vec<bool> {
        let _gpu = crate::test_gpu::lock();
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).expect("occlusion test requires a GPU");
        let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).expect("GPU device");
        let depth = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("occlusion test depth"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let depth_view = depth.create_view(&Default::default());
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("occlusion test occluder"),
            source: wgpu::ShaderSource::Wgsl(
                "@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
                    let p = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0))[i];
                    return vec4<f32>(p, 0.9, 1.0);
                }"
                .into(),
            ),
        });
        let occluder = device.create_render_pipeline_cached(&wgpu::RenderPipelineDescriptor {
            label: Some("occlusion test occluder"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: None,
            multiview_mask: None,
            cache: None,
        });
        let draws: Vec<CullDraw> = spheres
            .iter()
            .map(|(c, r)| CullDraw {
                sphere: [c.x, c.y, c.z, *r],
                kind: KIND_TESTED,
                _pad: [0; 3],
            })
            .collect();
        let args: Vec<u32> = (0..spheres.len() as u32).flat_map(|i| [3, 1, 0, 0, i]).collect();
        let main_args = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("occlusion test args"),
            contents: bytemuck::cast_slice(&args),
            usage: wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("occlusion test readback"),
            size: crate::readback::readback_size(main_args.size()),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut occlusion = Occlusion::new(&device);
        occlusion.prepare(&device, &queue, &depth_view, w, h, proj, &draws, spheres.len());
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("occlusion test occluder"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        // reverse-Z: 0 is the far plane, nothing drawn
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_pipeline(&occluder);
            pass.set_scissor_rect(0, 0, wall, h);
            pass.draw(0..3, 0..1);
        }
        occlusion.encode_cull(&device, &mut encoder, &main_args);
        encoder.copy_buffer_to_buffer(&main_args, 0, &readback, 0, main_args.size());
        queue.submit([encoder.finish()]);
        readback.map_async(wgpu::MapMode::Read, .., |r| r.expect("occlusion readback"));
        device.poll(wgpu::PollType::wait_indefinitely()).expect("occlusion GPU completion");
        let view = readback.get_mapped_range(..).expect("occlusion args");
        let out = crate::readback::u32s(&view[..main_args.size() as usize]);
        out.as_chunks::<5>().0.iter().map(|a| a[1] != 0).collect()
    }

    /// #2459: the pyramid's levels are floor-sized, and an odd level used to
    /// drop its source's last row and column, so at 1920 x 1080 a level
    /// holding only the left 1024 pixels judged an object of the open right
    /// part by the wall's depth. The pyramid now covers the whole depth.
    #[test]
    #[ignore = "requires a GPU"]
    fn objects_beside_a_near_wall_stay_visible_at_any_screen_size() {
        for (w, h) in [(1920u32, 1080u32), (1366, 768), (2560, 1440), (1374, 795), (1250, 883)] {
            let aspect = w as f32 / h as f32;
            let proj = glam::camera::rh::proj::directx::perspective_infinite_reverse(1.0, aspect, 0.1);
            // a sphere centred at `ndc_x` across the screen, `dist` ahead
            let at = |ndc_x: f32, dist: f32| Vec3::new(ndc_x * aspect * 0.5f32.tan() * dist, 0.0, -dist);
            let wall = w * 57 / 100;
            let open = (at(0.615, 6.0), 1.5);
            let small_open = (at(0.8, 20.0), 0.6);
            let behind = (at(-0.6, 6.0), 0.5);
            let got = visible(w, h, wall, proj, &[open, small_open, behind]);
            assert_eq!(got, [true, true, false], "{w} x {h}: open, small open, behind the wall");
        }
    }
}
