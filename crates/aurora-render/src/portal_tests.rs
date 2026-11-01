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
    // Exercise the actual GTAO entry points too: a colour-only cut leaves
    // invisible limbs in occlusion if a new depth/normal pass omits it.
    let mut normal_passes = String::new();
    // the alpha threshold depth_contact reads
    let threshold = object.find("override DEPTH_CONTACT_ALPHA").unwrap();
    normal_passes.push_str(&object[threshold..threshold + object[threshold..].find('\n').unwrap() + 1]);
    for name in [
        "fs_depth_portal_normal",
        "fs_depth_mask_normal",
        "depth_contact",
        "fs_depth_contact_normal",
    ] {
        let start = object.find(&format!("fn {name}(")).unwrap();
        let end = start + object[start..].find("\n}").unwrap() + 2;
        normal_passes.push_str(
            &object[start..end]
                .replace("@builtin(front_facing) ", "")
                .replace("@location(0) ", ""),
        );
        normal_passes.push('\n');
    }
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
    let aperture_cases = cases.clone();
    let aperture_expected = expected.clone();
    for pass in 1..=3 {
        cases.extend(aperture_cases.iter().map(|case| Case {
            flags: [flags::PORTAL_CLIP, pass, 0, 0],
            ..*case
        }));
        expected.extend_from_slice(&aperture_expected);
    }
    // The normal contact pass must share the analytic portal core rather
    // than treating the decorative quad's corners as solid depth.
    for x in [-1.1_f32, -0.8, 0.0, 0.8, 1.1] {
        for y in [-1.1_f32, -0.8, 0.0, 0.8, 1.1] {
            cases.push(Case {
                clip: [0.0; 4],
                point: [x, y, 0.0, 1.0],
                flags: [flags::PORTAL_EFFECT, 3, 0, 0],
            });
            expected.push(x * x + y * y <= 1.0);
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
const FLAG_PORTAL_EFFECT:u32=32768u;
const PORTAL_HALF_WIDTH:f32={PORTAL_HALF_WIDTH:.1};
const PORTAL_HALF_HEIGHT:f32={PORTAL_HALF_HEIGHT:.1};
const PORTAL_SURFACE_SCALE:f32=1.16;
struct DrawRecord {{portal_clip:vec4<f32>,flags:vec4<u32>,tex:vec4<u32>,base_color:vec4<f32>,params:vec4<f32>}};
struct VsOut {{world_pos:vec3<f32>,record:u32,uv:vec2<f32>,normal:vec3<f32>}};
struct Case {{clip:vec4<f32>,point:vec4<f32>,flags:vec4<u32>}};
@group(0) @binding(0) var<storage,read> cases:array<Case>;
@group(0) @binding(1) var<storage,read> records:array<DrawRecord>;
fn sample_tex(i:u32,uv:vec2<f32>)->vec4<f32>{{return vec4<f32>(1.0);}}
fn depth_normal(n:vec3<f32>,front:bool)->vec4<f32>{{return vec4<f32>(1.0);}}
fn water_clip(p:vec3<f32>){{}}
{cut}
{normal_passes}
@vertex fn vs(@builtin(vertex_index) i:u32)->@builtin(position) vec4<f32>{{
    let p=array<vec2<f32>,3>(vec2<f32>(-1.0,-1.0),vec2<f32>(3.0,-1.0),vec2<f32>(-1.0,3.0));
    return vec4<f32>(p[i],0.0,1.0);
}}
@fragment fn fs(@builtin(position) p:vec4<f32>)->@location(0) vec4<f32>{{
    let c=cases[u32(p.x)+64u*u32(p.y)];
    let rec=DrawRecord(c.clip,c.flags,vec4<u32>(0u),vec4<f32>(1.0),vec4<f32>(0.5));
    let index=u32(p.x)+64u*u32(p.y);
    let v=VsOut(c.point.xyz,index,(c.point.xy/PORTAL_SURFACE_SCALE+vec2<f32>(1.0))*0.5,vec3<f32>(0.0,0.0,1.0));
    if(c.flags.y==1u){{return fs_depth_portal_normal(v,true);}}
    if(c.flags.y==2u){{return fs_depth_mask_normal(v,true);}}
    if(c.flags.y==3u){{return fs_depth_contact_normal(v,true);}}
    portal_clip(rec,c.point.xyz);
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
    let records: Vec<DrawRecord> = cases
        .iter()
        .map(|case| DrawRecord {
            portal_clip: case.clip,
            flags: case.flags,
            base_color: [1.0; 4],
            params: [0.5; 4],
            ..Default::default()
        })
        .collect();
    // The reduced WGSL record above includes only fields these passes read.
    let compact: Vec<f32> = records
        .iter()
        .flat_map(|rec| {
            rec.portal_clip
                .into_iter()
                .chain(rec.flags.map(f32::from_bits))
                .chain([0.0; 4])
                .chain(rec.base_color)
                .chain(rec.params)
        })
        .collect();
    let normal_records = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("portal normal pass records"),
        contents: bytemuck::cast_slice(&compact),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: input.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: normal_records.as_entire_binding(),
            },
        ],
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
