//! Execute the actual production WGSL on synthetic, geometrically defined depth.
//! CPU intersections define expected geometry, never substitute for the WGSL.
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3, Vec4};
use serde_json::{Value, json};
use wgpu::util::DeviceExt;

const WAVE_SIZE: i32 = 64;
const SAMPLE_COUNT: f32 = 60.0;
const HARD_SHADOW_SAMPLES: f32 = 4.0;
const SURFACE_THICKNESS: f32 = 0.005;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Dispatch {
    pub wave_count: [i32; 3],
    pub wave_offset: [i32; 2],
}

/// The dispatches of one light (`DispatchList`): at most eight.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct DispatchList {
    /// The light on screen: x, y in pixels (y down), z its depth, w 1 in
    /// front of the camera, -1 behind.
    pub light: [f32; 4],
    pub dispatch: [Dispatch; 8],
    pub count: usize,
}

impl DispatchList {
    pub(crate) fn dispatches(&self) -> &[Dispatch] {
        &self.dispatch[..self.count]
    }
}

include!("production_bend.rs");
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    light: [f32; 4],
    wave_offset: [i32; 4],
    tuning: [f32; 4],
    reach: [f32; 4],
}
struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    depth_pipeline: wgpu::RenderPipeline,
    bend: wgpu::ComputePipeline,
    clear_light: wgpu::ComputePipeline,
    filter: wgpu::ComputePipeline,
}

fn compute_entry(
    device: &wgpu::Device,
    text: String,
    name: &str,
    entry: &str,
) -> wgpu::ComputePipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(name),
        source: wgpu::ShaderSource::Wgsl(text.into()),
    });
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(name),
        layout: None,
        module: &module,
        entry_point: Some(entry),
        compilation_options: Default::default(),
        cache: None,
    })
}

impl Gpu {
    fn new() -> (Self, Value) {
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = wgpu::Backends::VULKAN;
        let instance = wgpu::Instance::new(descriptor);
        let adapter = pollster::block_on(instance.request_adapter(&Default::default()))
            .expect("a Vulkan adapter is mandatory; this test never silently skips");
        let info = adapter.get_info();
        println!("Executing production WGSL on {info:?}");
        let adapter_json = json!({"name":info.name,"driver":info.driver,
            "driver_info":info.driver_info,"backend":format!("{:?}",info.backend),
            "device_type":format!("{:?}",info.device_type)});
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Default::default())).expect("Vulkan device");
        let reach = std::env::var("BEND_REACH")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(480);
        let source = shader_source(reach, 4);
        let bend = compute_entry(&device, source.clone(), "production Bend", "main");
        let clear_light = compute_entry(&device, source, "exact Bend clear-light", "clear_light");
        let filter = compute_entry(
            &device,
            include_str!("shaders/contact_shadows_filter.wgsl").to_owned(),
            "exact production filter",
            "main",
        );
        let depth_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("synthetic camera depth"),
            source: wgpu::ShaderSource::Wgsl(
                "\
@group(0) @binding(0) var<storage, read> depths: array<f32>;
@group(0) @binding(1) var<uniform> dims: vec4<u32>;
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}
@fragment fn fs(@builtin(position) p: vec4<f32>) -> @builtin(frag_depth) f32 {
    return depths[u32(p.y) * dims.x + u32(p.x)];
}"
                .into(),
            ),
        });
        let depth_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("synthetic camera depth"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &depth_module,
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
            fragment: Some(wgpu::FragmentState {
                module: &depth_module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[],
            }),
            multiview_mask: None,
            cache: None,
        });
        (
            Self {
                device,
                queue,
                depth_pipeline,
                bend,
                clear_light,
                filter,
            },
            adapter_json,
        )
    }

    fn run(&self, scene: &Scene, depth_m: f32) -> (Vec<[u8; 4]>, Vec<[u8; 4]>) {
        let device = &self.device;
        let extent = wgpu::Extent3d {
            width: scene.w,
            height: scene.h,
            depth_or_array_layers: 1,
        };
        let depth = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(&scene.name),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let result = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("actual production RGBA mask"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let filtered = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("production bilateral output"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        // A sentinel verifies every invocation overwrites all four channels.
        self.queue.write_texture(
            result.as_image_copy(),
            &vec![123u8; (scene.w * scene.h * 4) as usize],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(scene.w * 4),
                rows_per_image: Some(scene.h),
            },
            extent,
        );
        let depths = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("synthetic projected depths"),
            contents: bytemuck::cast_slice(&scene.depth),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let dims = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&[scene.w, scene.h, 0u32, 0u32]),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let depth_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.depth_pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: depths.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: dims.as_entire_binding(),
                },
            ],
        });
        assert_eq!(std::mem::size_of::<Params>(), 64);
        let depth_view = depth.create_view(&Default::default());
        let result_view = result.create_view(&Default::default());
        let filtered_view = filtered.create_view(&Default::default());
        let filter_options = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("hole repair disabled"),
            contents: bytemuck::cast_slice(&[0.0f32; 4]),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let filter_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.filter.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&depth_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&result_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&filtered_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: filter_options.as_entire_binding(),
                },
            ],
        });
        let light_clip = (scene.proj * scene.light.normalize().extend(0.0)).to_array();
        let list = build_dispatch_list(
            light_clip,
            [scene.w as i32, scene.h as i32],
            [0, 0],
            [scene.w as i32, scene.h as i32],
            WAVE_SIZE,
        );
        assert!(list.count > 0);
        let mut groups = Vec::new();
        for d in list.dispatches() {
            let world_per_ray =
                2.0 / (scene.proj.y_axis.y.abs() * scene.h as f32 * SURFACE_THICKNESS);
            let params = Params {
                light: list.light,
                wave_offset: [d.wave_offset[0], d.wave_offset[1], 0, 0],
                tuning: [SURFACE_THICKNESS, 0.02, 4.0, occluder_units(depth_m, 0.01)],
                reach: [0., 0., 0., world_per_ray],
            };
            let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("exact production Bend uniform values"),
                contents: bytemuck::bytes_of(&params),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let entries = [
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&depth_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&result_view),
                },
            ];
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &self.bend.get_bind_group_layout(0),
                entries: &entries,
            });
            // clear_light does not statically use the depth binding and its auto layout differs.
            let clear_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &self.clear_light.get_bind_group_layout(0),
                entries: &[entries[0].clone(), entries[2].clone()],
            });
            groups.push((uniform, group, clear_group, d.wave_count));
        }
        let row = (scene.w * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(row * scene.h * 2),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.depth_pipeline);
            pass.set_bind_group(0, &depth_group, &[]);
            pass.draw(0..3, 0..1);
        }
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&self.clear_light);
            pass.set_bind_group(0, &groups[0].2, &[]);
            pass.dispatch_workgroups(1, 1, 1);
            pass.set_pipeline(&self.bend);
            for (_, group, _, count) in &groups {
                pass.set_bind_group(0, group, &[]);
                pass.dispatch_workgroups(count[0] as u32, count[1] as u32, count[2] as u32);
            }
        }
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&self.filter);
            pass.set_bind_group(0, &filter_group, &[]);
            pass.dispatch_workgroups(scene.w.div_ceil(8), scene.h.div_ceil(8), 1);
        }
        encoder.copy_texture_to_buffer(
            filtered.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: u64::from(row * scene.h),
                    bytes_per_row: Some(row),
                    rows_per_image: Some(scene.h),
                },
            },
            extent,
        );
        encoder.copy_texture_to_buffer(
            result.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(scene.h),
                },
            },
            extent,
        );
        self.queue.submit([encoder.finish()]);
        let slice = readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |r| r.expect("map production output"));
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let mapped = slice.get_mapped_range().unwrap();
        let pixels: Vec<[u8; 4]> = mapped
            .chunks(row as usize)
            .flat_map(|line| {
                line[..(scene.w * 4) as usize]
                    .chunks(4)
                    .map(|p| p.try_into().unwrap())
            })
            .collect();
        drop(mapped);
        readback.unmap();
        let count = (scene.w * scene.h) as usize;
        assert!(
            pixels.iter().all(|p| *p != [123u8; 4]),
            "A contact pass left unwritten pixels"
        );
        (pixels[..count].to_vec(), pixels[count..].to_vec())
    }
}

struct Scene {
    name: String,
    w: u32,
    h: u32,
    proj: Mat4,
    depth: Vec<f32>,
    receiver: Vec<bool>,
    light: Vec3,
}
mod depth_grid;
mod map;
mod rim;
fn main() {
    if let Some(directory) = std::env::args_os().nth(1) {
        depth_grid::run(std::path::Path::new(&directory)).expect("depth-grid replay");
    } else {
        rim::run();
    }
}
