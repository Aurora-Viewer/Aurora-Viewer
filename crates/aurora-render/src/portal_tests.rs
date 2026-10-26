//! Render the production fragment cut: CPU picking and every surface pass
//! must agree at the oval's sides/corners, from both entry directions.
use crate::{DrawRecord, PORTAL_HALF_HEIGHT, PORTAL_HALF_WIDTH, PortalClip, arena::RecordStore, types::flags};
use glam::Vec3;
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Case {
    clip: [f32; 4],
    point: [f32; 4],
    flags: [u32; 4],
}

#[test]
fn portal_aperture_on_gpu_keeps_side_and_corner_geometry_and_matches_picking() {
    let object = include_str!("shaders/object.wgsl").replace("\r\n", "\n");
    let cut = &object[object.find("fn portal_clip(").unwrap()..object.find("// One analytically bounded Aurora surface").unwrap()];
    let _gpu = crate::test_gpu::lock();
    let instance = wgpu::Instance::default();
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
        return;
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).expect("GPU device");
    let mut store = RecordStore::new(&device);
    let slot = store.alloc(DrawRecord::default());
    // All-zero data is a real opening at the origin, not an off sentinel.
    store.set_portal_clip(slot, PortalClip::new(Vec3::ZERO, Vec3::X));
    assert_ne!(store.get(slot).unwrap().flags[0] & flags::PORTAL_CLIP, 0);
    store.set_portal_clip(slot, None);
    assert_eq!(store.get(slot).unwrap().flags[0] & flags::PORTAL_CLIP, 0);

    let mut cases = Vec::new();
    let mut expected = Vec::new();
    for center in [Vec3::ZERO, Vec3::new(212.0, 91.0, 60.0)] {
        for step in 0..8 {
            let yaw = step as f32 * std::f32::consts::TAU / 8.0;
            let normal = Vec3::new(yaw.cos(), yaw.sin(), 0.0);
            let across = Vec3::new(-normal.y, normal.x, 0.0);
            for side in [-1.0, 1.0] {
                let clip = PortalClip::new(center, normal * side).unwrap();
                for depth in [-0.2, 0.2] {
                    for x in [-1.1, -0.8, 0.0, 0.8, 1.1] {
                        for z in [-1.1, -0.8, 0.0, 0.8, 1.1] {
                            let point =
                                center + normal * side * depth + across * (x * PORTAL_HALF_WIDTH) + Vec3::Z * (z * PORTAL_HALF_HEIGHT);
                            cases.push(Case {
                                clip: clip.data,
                                point: point.extend(1.0).to_array(),
                                flags: [flags::PORTAL_CLIP, 0, 0, 0],
                            });
                            expected.push(clip.keeps_point(point));
                        }
                    }
                }
            }
        }
    }
    let width = 64;
    let height = (cases.len() as u32).div_ceil(width);
    cases.resize((width * height) as usize, cases[0]);
    let input = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("portal cut samples"),
        contents: bytemuck::cast_slice(&cases),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let source = format!(
        r#"
const FLAG_PORTAL_CLIP:u32=16384u;
const PORTAL_HALF_WIDTH:f32={PORTAL_HALF_WIDTH:.1};
const PORTAL_HALF_HEIGHT:f32={PORTAL_HALF_HEIGHT:.1};
struct DrawRecord {{portal_clip:vec4<f32>,flags:vec4<u32>}};
struct Case {{clip:vec4<f32>,point:vec4<f32>,flags:vec4<u32>}};
@group(0) @binding(0) var<storage,read> cases:array<Case>;
{cut}
@vertex fn vs(@builtin(vertex_index) i:u32)->@builtin(position) vec4<f32>{{
    let p=array<vec2<f32>,3>(vec2<f32>(-1.0,-1.0),vec2<f32>(3.0,-1.0),vec2<f32>(-1.0,3.0));
    return vec4<f32>(p[i],0.0,1.0);
}}
@fragment fn fs(@builtin(position) p:vec4<f32>)->@location(0) vec4<f32>{{
    let c=cases[u32(p.x)+64u*u32(p.y)];
    portal_clip(DrawRecord(c.clip,c.flags),c.point.xyz);
    return vec4<f32>(1.0);
}}
"#
    );
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("production portal cut"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("portal aperture regression"),
        layout: None,
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState {
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::TextureFormat::Rgba8Unorm.into())],
        }),
        multiview_mask: None,
        cache: None,
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: input.as_entire_binding(),
        }],
    });
    let (texture, view) = crate::renderer::make_view(
        &device,
        "portal aperture results",
        width,
        height,
        wgpu::TextureFormat::Rgba8Unorm,
        1,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    );
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(height) * 256,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
    }
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: Default::default(),
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    readback.map_async(wgpu::MapMode::Read, .., |result| result.expect("portal readback"));
    device.poll(wgpu::PollType::wait_indefinitely()).expect("GPU completion");
    let pixels = readback.get_mapped_range(..).expect("portal pixels");
    for (index, visible) in expected.into_iter().enumerate() {
        assert_eq!(pixels[index * 4] > 0, visible, "portal sample {index}: {:?}", cases[index].point);
    }
}
