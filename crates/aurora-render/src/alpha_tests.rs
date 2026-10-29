//! Production alpha entry points: empty texels never evaluate lighting, and
//! blended ones reject before fetching the remaining material maps.
use crate::types::flags;
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Case {
    base_color: [f32; 4],
    params: [f32; 4],
    tex: [u32; 4],
    flags: [u32; 4],
}

fn function(source: &str, name: &str) -> String {
    let start = source.find(&format!("fn {name}(")).expect("production entry point");
    let end = start + source[start..].find("\n}").expect("function end") + 2;
    source[start..end]
        .replace("@builtin(front_facing) ", "")
        .replace("@location(0) ", "")
}

#[test]
fn alpha_entry_points_preserve_pixels_and_reject_before_material_work() {
    let _gpu = crate::test_gpu::lock();
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: crate::BACKENDS,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
        return;
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).expect("GPU device");
    // (entry: mask / G-buffer mask / blend, texture alpha, face alpha, cutoff, flags)
    let fixtures = [
        (0, 0.0, 1.0, 0.5, 0),
        (0, 0.5, 1.0, 0.5, 0),
        (0, 0.0, 1.0, 0.0, 0),
        (1, 0.4, 1.0, 0.5, 0),
        (1, 1.0, 1.0, 0.5, 0),
        (2, 0.0, 1.0, 0.0, 0),
        (2, 1.0, 0.0, 0.0, 0),
        (2, 0.0, 1.0, 0.0, flags::EMISSIVE_MASK),
        (2, 0.0, 0.0, 0.0, flags::EMISSIVE_MASK),
        (2, 0.2, 1.0, 0.5, flags::ALPHA_MASK),
        (2, 0.5, 1.0, 0.0, 0),
        (2, 0.8, 0.75, 0.0, 0),
        (2, 1.0, 0.001, 0.0, 0),
    ];
    let cases: Vec<Case> = fixtures
        .iter()
        .enumerate()
        .map(|(i, &(kind, tex_alpha, face_alpha, cutoff, bits))| Case {
            base_color: [1.0, 1.0, 1.0, face_alpha],
            params: [tex_alpha, 0.0, 0.0, cutoff],
            tex: [i as u32, 0, 0, 0],
            flags: [bits, kind, 0, 0],
        })
        .collect();
    let object = include_str!("shaders/object.wgsl");
    let entries: String = ["mask_color", "fs_mask", "fs_mask_g", "fs_blend"]
        .into_iter()
        .map(|n| function(object, n) + "\n")
        .collect();
    let source = format!(
        r#"
const FLAG_PORTAL_EFFECT:u32={portal}u;
const FLAG_ALPHA_MASK:u32={mask}u;
const FLAG_EMISSIVE_MASK:u32={emissive}u;
const PORTAL_SURFACE_SCALE:f32=1.16;
override ALPHA_TO_COVERAGE:bool=false;
override BLEND_EARLY_DISCARD:bool=true;
override BLEND_SPLIT:u32=0u;
struct Record {{base_color:vec4<f32>,params:vec4<f32>,tex:vec4<u32>,flags:vec4<u32>}};
alias DrawRecord=Record;
struct VsOut {{world_pos:vec3<f32>,uv:vec2<f32>,record:u32}};
struct Material {{color:vec4<f32>}};
struct Shaded {{color:vec4<f32>}};
struct GOut {{color:vec4<f32>,gbuf:vec4<f32>}};
struct Surface {{color:vec3<f32>,alpha:f32}};
struct Frame {{sky_cloud_pd2:vec4<f32>}};
const frame=Frame(vec4<f32>(1.0));
@group(0) @binding(0) var<storage,read> records:array<Record>;
@group(0) @binding(1) var<storage,read_write> costs:array<atomic<u32>>;
fn sample_tex(i:u32,uv:vec2<f32>)->vec4<f32>{{return vec4<f32>(0.2,0.4,0.6,records[i].params.x);}}
fn eval_material_sampled(v:VsOut,front:bool,base:vec4<f32>)->Material{{
    atomicAdd(&costs[v.record*2u],1u);
    let r=records[v.record];
    let alpha=select(base.a*r.base_color.a,r.base_color.a,(r.flags.x&FLAG_EMISSIVE_MASK)!=0u);
    return Material(vec4<f32>(base.rgb,alpha));
}}
fn eval_material(v:VsOut,front:bool)->Material{{return eval_material_sampled(v,front,sample_tex(v.record,v.uv));}}
fn shade_material(v:VsOut,m:Material)->Shaded{{atomicAdd(&costs[v.record*2u+1u],1u);return Shaded(m.color);}}
fn shade(v:VsOut,front:bool)->Shaded{{return shade_material(v,eval_material(v,front));}}
fn gbuffer(s:Shaded)->vec4<f32>{{return s.color;}}
fn portal_surface(r:Record,uv:vec2<f32>,aa:f32)->Surface{{return Surface(vec3<f32>(0.2),0.5);}}
fn apply_fog(c:vec3<f32>,p:vec3<f32>)->vec3<f32>{{return c;}}
fn rez_alpha(r:Record,v:VsOut)->f32{{return 1.0;}}
{entries}
@vertex fn vs(@builtin(vertex_index) i:u32)->@builtin(position) vec4<f32>{{
    let p=array<vec2<f32>,3>(vec2<f32>(-1.0,-1.0),vec2<f32>(3.0,-1.0),vec2<f32>(-1.0,3.0));return vec4<f32>(p[i],0.0,1.0);
}}
@fragment fn fs(@builtin(position) p:vec4<f32>)->@location(0) vec4<f32>{{
    let index=u32(p.x)/8u;let v=VsOut(vec3<f32>(0.0),p.xy/128.0,index);
    if(records[index].flags.y==0u){{return fs_mask(v,true);}}
    if(records[index].flags.y==1u){{return fs_mask_g(v,true).color;}}
    return fs_blend(v,true);
}}
"#,
        portal = flags::PORTAL_EFFECT,
        mask = flags::ALPHA_MASK,
        emissive = flags::EMISSIVE_MASK
    );
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("production alpha entry points"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let records = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&cases),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let zero = vec![0u32; cases.len() * 2];
    let costs = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&zero),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
    });
    let width = cases.len() as u32 * 8;
    let height = 16;
    let (image, view) = crate::renderer::make_view(
        &device,
        "alpha test pixels",
        width,
        height,
        wgpu::TextureFormat::Rgba8Unorm,
        1,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    );
    let row = (width * 4).div_ceil(256) * 256;
    let readback = |size| {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        })
    };
    let pixels = readback(u64::from(row * height));
    let counters = readback(costs.size());
    for a2c in [false, true] {
        for early in [false, true] {
            for split in 0..=2 {
                queue.write_buffer(&costs, 0, bytemuck::cast_slice(&zero));
                let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: None,
                    layout: None,
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: Some("vs"),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    primitive: Default::default(),
                    depth_stencil: None,
                    multisample: Default::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some("fs"),
                        compilation_options: wgpu::PipelineCompilationOptions {
                            constants: &[
                                ("ALPHA_TO_COVERAGE", f64::from(a2c)),
                                ("BLEND_EARLY_DISCARD", f64::from(early)),
                                ("BLEND_SPLIT", f64::from(split)),
                            ],
                            ..Default::default()
                        },
                        targets: &[Some(wgpu::TextureFormat::Rgba8Unorm.into())],
                    }),
                    multiview_mask: None,
                    cache: None,
                });
                let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: None,
                    layout: &pipeline.get_bind_group_layout(0),
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: records.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: costs.as_entire_binding(),
                        },
                    ],
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
                    image.as_image_copy(),
                    wgpu::TexelCopyBufferInfo {
                        buffer: &pixels,
                        layout: wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(row),
                            rows_per_image: Some(height),
                        },
                    },
                    wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                );
                encoder.copy_buffer_to_buffer(&costs, 0, &counters, 0, costs.size());
                queue.submit([encoder.finish()]);
                pixels.map_async(wgpu::MapMode::Read, .., |r| r.expect("pixels"));
                counters.map_async(wgpu::MapMode::Read, .., |r| r.expect("costs"));
                device.poll(wgpu::PollType::wait_indefinitely()).expect("GPU completion");
                let bytes = pixels.get_mapped_range(..).expect("pixels");
                let cost_bytes = counters.get_mapped_range(..).expect("costs");
                let counts: &[u32] = bytemuck::cast_slice(&cost_bytes);
                for (i, &(kind, tex_alpha, face_alpha, cutoff, bits)) in fixtures.iter().enumerate() {
                    let raw = tex_alpha * face_alpha;
                    let alpha = if bits & flags::EMISSIVE_MASK != 0 { face_alpha } else { raw };
                    let (material, lighting, visible, out_alpha) = if kind < 2 {
                        let visible = a2c || cutoff <= 0.0 || alpha >= cutoff;
                        let coverage = if cutoff <= 0.0 || !a2c {
                            1.0
                        } else {
                            ((alpha - cutoff) / 0.0001 + 0.5).clamp(0.0, 1.0)
                        };
                        (true, visible, visible, coverage)
                    } else {
                        let kept = (split == 0 || ((split == 1) == (raw >= 0.5)))
                            && !(early && alpha < 0.004)
                            && !(bits & flags::ALPHA_MASK != 0 && alpha < cutoff);
                        (kept, kept, kept && alpha >= 0.004, alpha)
                    };
                    assert_eq!(
                        counts[i * 2],
                        if material { 8 * height } else { 0 },
                        "material case {i}, a2c={a2c} early={early} split={split}"
                    );
                    assert_eq!(
                        counts[i * 2 + 1],
                        if lighting { 8 * height } else { 0 },
                        "lighting case {i}, a2c={a2c} early={early} split={split}"
                    );
                    let alpha = if visible { out_alpha } else { 0.0 };
                    let rgb = if visible { if kind == 2 { out_alpha } else { 1.0 } } else { 0.0 };
                    let expected = [0.2 * rgb, 0.4 * rgb, 0.6 * rgb, alpha].map(|v| (v * 255.0).round() as u8);
                    let offset = (row * 8 + i as u32 * 8 * 4 + 4 * 4) as usize;
                    assert!(
                        bytes[offset..offset + 4].iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 1),
                        "pixels case {i}, a2c={a2c} early={early} split={split}"
                    );
                }
                drop(bytes);
                drop(cost_bytes);
                pixels.unmap();
                counters.unmap();
            }
        }
    }
}
