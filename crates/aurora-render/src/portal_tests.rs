//! Render the production dissolution: CPU picking and every surface pass
//! must share the same fragments throughout the contact band, in both directions.
use crate::pipeline_cache::CachedPipelines;
use crate::{
    DrawRecord, PORTAL_DISSOLVE_CELLS_PER_METER, PORTAL_HALF_HEIGHT, PORTAL_HALF_WIDTH, PortalClip, arena::RecordStore, types::flags,
};
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
fn portal_dissolution_on_gpu_matches_picking_and_depth_through_the_contact_band() {
    let object = include_str!("shaders/object.wgsl").replace("\r\n", "\n");
    let cut = &object[object.find("fn portal_clip(").unwrap()..object.find("// One analytically bounded Aurora surface").unwrap()];
    let opening = &object[object.find("fn portal_noise(").unwrap()..object.find("// Deform only the shared decorative geometry").unwrap()];
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
        "fs_glow",
        "glow_amount",
        "blend_glow",
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
    let glow_start = object.find("fn portal_rez_glow(").unwrap();
    let glow_end = glow_start + object[glow_start..].find("\n}").unwrap() + 2;
    normal_passes.push_str(&object[glow_start..glow_end]);
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
                for depth in [-0.5, -0.0001, 0.0, 0.05, 0.15, 0.225, 0.35, 0.6] {
                    for x in [-1.1, -0.8, 0.0, 0.8, 1.1] {
                        for z in [-1.1, -0.8, 0.0, 0.8, 1.1] {
                            // Stay inside noise cells so coordinate-rounding at
                            // rotated cell boundaries cannot select a neighbour.
                            let sample =
                                |value: f32| ((value * PORTAL_DISSOLVE_CELLS_PER_METER).floor() + 0.375) / PORTAL_DISSOLVE_CELLS_PER_METER;
                            let point = center
                                + normal * side * depth
                                + across * sample(x * PORTAL_HALF_WIDTH)
                                + Vec3::Z * sample(z * PORTAL_HALF_HEIGHT);
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
    for center in [Vec3::ZERO, Vec3::new(140.0, 90.0, 32.0)] {
        for normal in [Vec3::Z, Vec3::NEG_Z] {
            let clip = PortalClip::new(center, normal).unwrap();
            for depth in [-0.1, 0.0, 0.05, 0.15, 0.3, 0.6] {
                for x in [-0.8, 0.1, 0.9] {
                    for y in [-0.7, 0.2, 0.85] {
                        let point = center + normal * depth + Vec3::new(x, y, 0.0);
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
    let aperture_cases = cases.clone();
    let aperture_expected = expected.clone();
    for pass in 1..=5 {
        cases.extend(aperture_cases.iter().map(|case| Case {
            flags: [flags::PORTAL_CLIP, pass, 0, 0],
            ..*case
        }));
        if pass >= 4 {
            expected.extend(aperture_cases.iter().zip(&aperture_expected).map(|(case, &kept)| {
                kept && PortalClip { data: case.clip }
                    .normal()
                    .dot(Vec3::from_slice(&case.point[..3]) - Vec3::from_slice(&case.clip[..3]))
                    < crate::PORTAL_DISSOLVE_WIDTH
            }));
        } else {
            expected.extend_from_slice(&aperture_expected);
        }
    }
    // Completed travelers have no remaining body, feet, attachments or glow,
    // even after moving beside the opening while the teleport is pending.
    for pass in 0..=5 {
        for point in [Vec3::ZERO, Vec3::new(2.0, 4.0, -2.0), Vec3::new(-2.0, -4.0, 3.0)] {
            let clip = PortalClip::hidden();
            assert!(!clip.keeps_point(point));
            cases.push(Case {
                clip: clip.data,
                point: point.extend(1.0).to_array(),
                flags: [flags::PORTAL_CLIP, pass, 0, 0],
            });
            expected.push(false);
        }
    }
    for pass in 0..=5 {
        cases.push(Case {
            clip: [0.0; 4],
            point: [-0.5, 0.0, 0.0, 1.0],
            flags: [0, pass, 0, 0],
        });
        expected.push(pass < 4);
    }
    // A contact glow cannot fill transparent texture holes or invisible
    // blended faces. Opaque material texture alpha remains non-opacity data.
    for (material, texture, alpha, glow) in [
        (flags::ALPHA_MASK, 1, 0, false),
        (flags::PBR | flags::ALPHA_MASK, 1, 0, false),
        (flags::ALPHA_BLEND, 0, 1, false),
        (0, 1, 0, true),
    ] {
        for pass in [4, 5] {
            cases.push(Case {
                clip: [0.0; 4],
                point: [0.225, 0.0, 0.0, 1.0],
                flags: [flags::PORTAL_CLIP | material, pass, texture, alpha],
            });
            expected.push(glow);
        }
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
    // A cloud cannot write the depth of a fully open oval before its tear
    // exists. The vertical slit opens into that same physical aperture.
    for (elapsed, x, visible) in [
        (0, 0.0, false),
        (160, 0.0, false),
        (560, 0.5, false),
        (645, 0.8, false),
        (720, 0.0, true),
        (720, 0.8, true),
        (900, 0.8, true),
        (1040, 0.2, true),
        (1800, 0.8, true),
    ] {
        cases.push(Case {
            clip: [0.0; 4],
            point: [x, 0.0, 0.0, 1.0],
            flags: [flags::PORTAL_EFFECT, 3, 0, 8000 - elapsed],
        });
        expected.push(visible);
    }
    // A core hidden for the follow camera must not leave contact depth behind.
    cases.push(Case {
        clip: [0.0; 4],
        point: [0.0; 4],
        flags: [flags::PORTAL_EFFECT, 3, 0, 0],
    });
    expected.push(false);
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
const FLAG_PBR:u32=8u;
const FLAG_ALPHA_MASK:u32=2u;
const FLAG_ALPHA_BLEND:u32=4u;
const FLAG_DOUBLE_SIDED:u32=2048u;
const PORTAL_HALF_WIDTH:f32={PORTAL_HALF_WIDTH:.1};
const PORTAL_HALF_HEIGHT:f32={PORTAL_HALF_HEIGHT:.1};
const PORTAL_SURFACE_SCALE:f32=1.16;
struct DrawRecord {{portal_clip:vec4<f32>,flags:vec4<u32>,tex:vec4<u32>,base_color:vec4<f32>,params:vec4<f32>,emissive:vec4<f32>}};
struct VsOut {{world_pos:vec3<f32>,record:u32,uv:vec2<f32>,normal:vec3<f32>,uv_e:vec2<f32>}};
struct DepthMaskOut {{world_pos:vec3<f32>,record:u32,uv:vec2<f32>,normal:vec3<f32>,motion_cur:vec4<f32>,motion_prev:vec4<f32>}};
struct Case {{clip:vec4<f32>,point:vec4<f32>,flags:vec4<u32>}};
struct Frame {{anim_clock:vec4<u32>}};
const frame=Frame(vec4<u32>(8000u,0u,0u,0u));
@group(0) @binding(0) var<storage,read> cases:array<Case>;
@group(0) @binding(1) var<storage,read> records:array<DrawRecord>;
fn sample_tex(i:u32,uv:vec2<f32>)->vec4<f32>{{return vec4<f32>(1.0,1.0,1.0,select(1.0,0.0,i==1u));}}
fn blend_sample(i:u32,uv:vec2<f32>,dx:vec2<f32>,dy:vec2<f32>)->vec4<f32>{{return sample_tex(i,uv);}}
fn depth_normal(n:vec3<f32>,front:bool,cur:vec4<f32>,prev:vec4<f32>,c:f32)->vec4<f32>{{return vec4<f32>(1.0);}}
fn srgb_to_linear(c:vec3<f32>)->vec3<f32>{{return c;}}
var<private> rez_fw:vec3<f32>;
fn rez_lines(p:vec3<f32>,n:vec3<f32>,step:f32)->f32{{return 0.5;}}
fn rez_glow(rec:DrawRecord,v:VsOut)->f32{{return 0.0;}}
struct PortalSurface{{glow:f32}};
fn portal_surface(rec:DrawRecord,uv:vec2<f32>,p:vec3<f32>,n:vec3<f32>,aa:f32)->PortalSurface{{return PortalSurface(0.0);}}
fn normal_composition(r:DrawRecord)->f32{{return 0.0;}}
fn water_clip(p:vec3<f32>){{}}
{cut}
{opening}
{normal_passes}
@vertex fn vs(@builtin(vertex_index) i:u32)->@builtin(position) vec4<f32>{{
    let p=array<vec2<f32>,3>(vec2<f32>(-1.0,-1.0),vec2<f32>(3.0,-1.0),vec2<f32>(-1.0,3.0));
    return vec4<f32>(p[i],0.0,1.0);
}}
@fragment fn fs(@builtin(position) p:vec4<f32>)->@location(0) vec4<f32>{{
    let c=cases[u32(p.x)+64u*u32(p.y)];
    let rec=records[u32(p.x)+64u*u32(p.y)];
    let index=u32(p.x)+64u*u32(p.y);
    let v=VsOut(c.point.xyz,index,(c.point.xy/PORTAL_SURFACE_SCALE+vec2<f32>(1.0))*0.5,vec3<f32>(0.0,0.0,1.0),vec2<f32>(0.0));
    let m=DepthMaskOut(v.world_pos,v.record,v.uv,v.normal,vec4<f32>(0.0,0.0,0.0,1.0),vec4<f32>(0.0,0.0,0.0,1.0));
    if(c.flags.y==1u){{return fs_depth_portal_normal(m,true);}}
    if(c.flags.y==2u){{return fs_depth_mask_normal(m,true);}}
    if(c.flags.y==3u){{return fs_depth_contact_normal(m,true);}}
    if(c.flags.y==4u){{return fs_glow(v,true);}}
    if(c.flags.y==5u){{
        portal_clip(rec,c.point.xyz);
        rez_fw=fwidth(v.world_pos);
        return vec4<f32>(0.0,0.0,0.0,blend_glow(v,vec2<f32>(0.0),vec2<f32>(0.0)));
    }}
    portal_clip(rec,c.point.xyz);
    return vec4<f32>(1.0);
}}
"#
    );
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("production portal cut"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device.create_render_pipeline_cached(&wgpu::RenderPipelineDescriptor {
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
            tex: [case.flags[2], 0, 0, 0],
            params: [
                0.5,
                0.5,
                0.5,
                if case.flags[0] & flags::PORTAL_EFFECT != 0 {
                    case.point[3]
                } else {
                    0.5
                },
            ],
            ..Default::default()
        })
        .map(|mut record| {
            if record.flags[3] == 1 {
                record.base_color[3] = 0.0;
            }
            record
        })
        .collect();
    // The reduced WGSL record above includes only fields these passes read.
    let compact: Vec<f32> = records
        .iter()
        .flat_map(|rec| {
            rec.portal_clip
                .into_iter()
                .chain(rec.flags.map(f32::from_bits))
                .chain(rec.tex.map(f32::from_bits))
                .chain(rec.base_color)
                .chain(rec.params)
                .chain([0.0; 4])
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
        size: crate::readback::readback_size(u64::from(height) * 256),
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
        assert_eq!(
            pixels[index * 4 + 3] > 0,
            visible,
            "portal sample {index}: {:?}",
            cases[index].point
        );
    }
}
