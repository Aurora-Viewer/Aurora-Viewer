//! Rasterize both windings through the production blend/depth fragment cuts.
use crate::pipeline_cache::CachedPipelines;
use crate::types::flags;
use wgpu::util::DeviceExt;

#[test]
fn transparent_pbr_sidedness_matches_colour_and_depth_on_gpu() {
    let object = include_str!("shaders/object.wgsl").replace("\r\n", "\n");
    let mut fragments = String::new();
    for name in [
        "fs_blend",
        "depth_contact",
        "fs_depth_contact",
        "fs_depth_contact_normal",
        "fs_depth_opaque",
        "water_clip",
    ] {
        let start = object.find(&format!("fn {name}(")).expect("production fragment");
        let end = start + object[start..].find("\n}").expect("fragment end") + 2;
        fragments.push_str(
            &object[start..end]
                .replace("@builtin(front_facing) ", "")
                .replace("@location(0) ", ""),
        );
        fragments.push('\n');
    }
    let source = format!(
        r#"
const FLAG_DOUBLE_SIDED:u32={double_sided}u;
const FLAG_PORTAL_EFFECT:u32={portal}u;
const FLAG_ALPHA_MASK:u32={mask}u;
const PORTAL_SURFACE_SCALE:f32=1.16;
const BLEND_EARLY_DISCARD:bool=true;
const BLEND_SPLIT:u32=0u;
const DEPTH_CONTACT_ALPHA:f32=0.5;
struct DrawRecord {{flags:vec4<u32>,tex:vec4<u32>,base_color:vec4<f32>,params:vec4<f32>}};
@group(0) @binding(0) var<storage,read> records:array<DrawRecord>;
struct Frame {{sky_cloud_pd2:vec4<f32>,params:vec4<f32>,camera_pos:vec4<f32>}};
const frame=Frame(vec4<f32>(1.0),vec4<f32>(0.0),vec4<f32>(0.0,0.0,1.0,1.0));
override WATER_CLIP:u32=0u;
struct VsOut {{@builtin(position) clip:vec4<f32>,@location(0) @interpolate(flat) record:u32,
    @location(1) world_pos:vec3<f32>,@location(2) uv:vec2<f32>}};
struct Material {{color:vec4<f32>}};
struct Surface {{alpha:f32,color:vec3<f32>}};
struct DepthMaskOut {{record:u32,uv:vec2<f32>,world_pos:vec3<f32>,normal:vec3<f32>,motion_cur:vec4<f32>,motion_prev:vec4<f32>}};
fn portal_surface(r:DrawRecord,uv:vec2<f32>,aa:f32)->Surface {{return Surface(1.0,vec3<f32>(1.0));}}
fn apply_fog(c:vec3<f32>,p:vec3<f32>)->vec3<f32> {{return c;}}
fn portal_clip(r:DrawRecord,p:vec3<f32>) {{}}
fn sample_tex(t:u32,uv:vec2<f32>)->vec4<f32> {{return vec4<f32>(1.0);}}
fn eval_material(v:VsOut,front:bool)->Material {{return Material(records[v.record].base_color);}}
fn legacy_may_glare(r:DrawRecord,m:Material,p:vec3<f32>,f:vec2<f32>)->bool {{return false;}}
fn shade_material(v:VsOut,m:Material)->Material {{return m;}}
fn rez_alpha(r:DrawRecord,v:VsOut)->f32 {{return 1.0;}}
fn depth_normal(n:vec3<f32>,front:bool,cur:vec4<f32>,prev:vec4<f32>,c:f32)->vec4<f32> {{return vec4<f32>(1.0);}}
fn normal_composition(r:DrawRecord)->f32 {{return 0.0;}}
{fragments}
@vertex fn vs(@builtin(vertex_index) vertex:u32,@builtin(instance_index) instance:u32)->VsOut {{
    let positions=array<vec2<f32>,3>(vec2<f32>(-1.0,-1.0),vec2<f32>(3.0,-1.0),vec2<f32>(-1.0,3.0));
    let index=select(vertex,2u-vertex,records[instance].flags.y!=0u);
    let height=select(-1.0,1.0,records[instance].flags.z!=0u);
    return VsOut(vec4<f32>(positions[index],0.5,1.0),instance,vec3<f32>(0.0,0.0,height),vec2<f32>(0.5));
}}
@fragment fn colour(v:VsOut,@builtin(front_facing) front:bool)->@location(0) vec4<f32> {{return fs_blend(v,front);}}
@fragment fn contact(v:VsOut,@builtin(front_facing) front:bool)->@location(0) vec4<f32> {{
    fs_depth_contact(v,front); return vec4<f32>(1.0);
}}
@fragment fn normal(v:VsOut,@builtin(front_facing) front:bool)->@location(0) vec4<f32> {{
    let m=DepthMaskOut(v.record,v.uv,v.world_pos,vec3<f32>(0.0,0.0,1.0),vec4<f32>(0.0),vec4<f32>(0.0));
    return fs_depth_contact_normal(m,front);
}}
@fragment fn opaque(v:VsOut,@builtin(front_facing) front:bool)->@location(0) vec4<f32> {{
    fs_depth_opaque(v,front); return vec4<f32>(1.0);
}}
"#,
        double_sided = flags::DOUBLE_SIDED,
        portal = flags::PORTAL_EFFECT,
        mask = flags::ALPHA_MASK,
    );
    let _gpu = crate::test_gpu::lock();
    let instance = wgpu::Instance::default();
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
        return;
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).expect("sidedness GPU device");
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("production blended sidedness"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let mut records = Vec::<[u32; 16]>::new();
    // Every colour/depth cut, above and below the plane, for unclipped,
    // post-water and pre-water pipelines. This also covers the #1788 overlap.
    for row in 0..24 {
        for (double, back) in [(false, false), (false, true), (true, false), (true, true)] {
            let mut rec = [0; 16];
            rec[0] = flags::PBR | flags::ALPHA_BLEND | if double { flags::DOUBLE_SIDED } else { 0 };
            rec[1] = u32::from(back);
            rec[2] = u32::from((row / 4) % 2 != 0);
            for value in &mut rec[8..12] {
                *value = 1.0_f32.to_bits();
            }
            rec[11] = if row % 4 == 3 { 0.98_f32 } else { 0.6_f32 }.to_bits();
            records.push(rec);
        }
    }
    let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("front/back single/double materials"),
        contents: bytemuck::cast_slice(&records),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let (texture, view) = crate::renderer::make_view(
        &device,
        "sidedness pixels",
        64,
        384,
        wgpu::TextureFormat::Rgba8Unorm,
        1,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    );
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("sidedness readback"),
        size: crate::readback::readback_size(384 * 256),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    for row in 0..24 {
        let entry = ["colour", "contact", "normal", "opaque"][row % 4];
        let water_clip = (row / 8) as f64;
        let pipelines = [false, true].map(|double_sided| {
            let pipeline = device.create_render_pipeline_cached(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: None,
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState {
                    cull_mode: crate::renderer::blend_cull(double_sided),
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: wgpu::PipelineCompilationOptions {
                        constants: &[("WATER_CLIP", water_clip)],
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
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                }],
            });
            (pipeline, group)
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: if row == 0 {
                        wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                    } else {
                        wgpu::LoadOp::Load
                    },
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        for column in 0..4 {
            let (pipeline, group) = &pipelines[usize::from(column >= 2)];
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, group, &[]);
            pass.set_viewport(column as f32 * 16.0, row as f32 * 16.0, 16.0, 16.0, 0.0, 1.0);
            let record = row as u32 * 4 + column;
            pass.draw(0..3, record..record + 1);
        }
    }
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256),
                rows_per_image: None,
            },
        },
        texture.size(),
    );
    queue.submit([encoder.finish()]);
    readback.map_async(wgpu::MapMode::Read, .., |r| r.expect("sidedness readback"));
    device.poll(wgpu::PollType::wait_indefinitely()).expect("sidedness completion");
    let pixels = readback.get_mapped_range(..).expect("sidedness pixels");
    for row in 0..24 {
        let clip = row / 8;
        let above = (row / 4) % 2 != 0;
        for column in 0..4 {
            let offset = (row * 16 + 8) * 256 + (column * 16 + 8) * 4;
            let visible = column != 1 && (clip == 0 || (clip == 1) == above);
            assert_eq!(pixels[offset] > 0, visible, "pass {row}, front/back case {column}");
        }
    }
}

#[test]
fn world_blend_pick_gpu_uses_legacy_threshold_without_changing_worn_or_mask_rules() {
    let object = include_str!("shaders/object.wgsl").replace("\r\n", "\n");
    let cutoff = object
        .lines()
        .find(|line| line.starts_with("const PICK_ALPHA_CUTOFF:"))
        .expect("production cutoff");
    let mut fragments = String::new();
    for name in ["fs_pick", "fs_pick_visible"] {
        let start = object.find(&format!("fn {name}(")).expect("production fragment");
        let end = start + object[start..].find("\n}").expect("fragment end") + 2;
        fragments.push_str(&object[start..end].replace("@location(0) ", ""));
        fragments.push('\n');
    }
    let source = format!(
        r#"
{cutoff}
const FLAG_WORLD_BLEND_PICK:u32={world}u;
const FLAG_ALPHA_MASK:u32={mask}u;
const FLAG_ALPHA_BLEND:u32={blend}u;
const FLAG_PBR:u32={pbr}u;
const FLAG_TERRAIN:u32={terrain}u;
const FLAG_EMISSIVE_MASK:u32={emissive}u;
override WORLD_LEFT_PICK:bool=false;
struct DrawRecord {{flags:vec4<u32>,tex:vec4<u32>,base_color:vec4<f32>,params:vec4<f32>}};
@group(0) @binding(0) var<storage,read> records:array<DrawRecord>;
struct PickOut {{@builtin(position) clip:vec4<f32>,@location(0) uv:vec2<f32>,
    @location(1) st:vec2<f32>,@location(2) @interpolate(flat) record:u32,@location(3) world_pos:vec3<f32>}};
fn portal_clip(r:DrawRecord,p:vec3<f32>) {{}}
fn sample_tex(t:u32,uv:vec2<f32>)->vec4<f32> {{return vec4<f32>(1.0,1.0,1.0,f32(t)/255.0);}}
{fragments}
@vertex fn vs(@builtin(vertex_index) v:u32,@builtin(instance_index) r:u32)->PickOut {{
    let p=array<vec2<f32>,3>(vec2<f32>(-1.0,-1.0),vec2<f32>(3.0,-1.0),vec2<f32>(-1.0,3.0))[v];
    return PickOut(vec4<f32>(p,0.5,1.0),vec2<f32>(0.5),vec2<f32>(0.5),r,vec3<f32>(0.0));
}}
@fragment fn interaction(v:PickOut)->@location(0) vec4<u32> {{return fs_pick(v);}}
@fragment fn visible(v:PickOut)->@location(0) vec4<u32> {{return fs_pick_visible(v);}}
"#,
        world = flags::WORLD_BLEND_PICK,
        mask = flags::ALPHA_MASK,
        blend = flags::ALPHA_BLEND,
        pbr = flags::PBR,
        terrain = flags::TERRAIN,
        emissive = flags::EMISSIVE_MASK,
    );
    let _gpu = crate::test_gpu::lock();
    let instance = wgpu::Instance::default();
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
        return;
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).expect("pick GPU device");
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("production pick thresholds"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let alphas = [0, 1, 31, 32, 33, 128, 255];
    let mut records = Vec::<[u32; 16]>::new();
    for row in 0..32 {
        for alpha in alphas {
            let kind = row % 4;
            let mut rec = [0; 16];
            rec[0] = if kind == 1 || kind == 3 {
                flags::ALPHA_MASK
            } else {
                flags::ALPHA_BLEND
            };
            rec[3] = if kind < 2 { flags::WORLD_BLEND_PICK } else { 0 };
            rec[4] = alpha;
            rec[11] = if row % 8 < 4 { 1.0_f32 } else { 0.01_f32 }.to_bits();
            rec[15] = 0.5_f32.to_bits();
            records.push(rec);
        }
    }
    let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("world/worn/mask pick cases"),
        contents: bytemuck::cast_slice(&records),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let (texture, view) = crate::renderer::make_view(
        &device,
        "pick threshold pixels",
        7,
        32,
        crate::drawn_pick::FORMAT,
        1,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    );
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("pick threshold readback"),
        size: crate::readback::readback_size(32 * 256),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    for mode in 0..4 {
        let visible = mode >= 2;
        let left = mode % 2 != 0;
        let pipeline = device.create_render_pipeline_cached(&wgpu::RenderPipelineDescriptor {
            label: Some("pick threshold pipeline"),
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
                entry_point: Some(if visible { "visible" } else { "interaction" }),
                compilation_options: wgpu::PipelineCompilationOptions {
                    constants: &[("WORLD_LEFT_PICK", f64::from(u8::from(left)))],
                    ..Default::default()
                },
                targets: &[Some(crate::drawn_pick::FORMAT.into())],
            }),
            multiview_mask: None,
            cache: None,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: if mode == 0 {
                        wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                    } else {
                        wgpu::LoadOp::Load
                    },
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        for row in mode * 8..(mode + 1) * 8 {
            for column in 0..7 {
                pass.set_viewport(column as f32, row as f32, 1.0, 1.0, 0.0, 1.0);
                let record = row * 7 + column;
                pass.draw(0..3, record..record + 1);
            }
        }
    }
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256),
                rows_per_image: Some(32),
            },
        },
        wgpu::Extent3d {
            width: 7,
            height: 32,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    readback.map_async(wgpu::MapMode::Read, .., |result| result.expect("pick readback"));
    device.poll(wgpu::PollType::wait_indefinitely()).expect("GPU completion");
    let bytes = readback.get_mapped_range(..).expect("mapped pick pixels");
    for row in 0..32usize {
        let visible = row / 8 >= 2;
        let left = (row / 8) % 2 != 0;
        let kind = row % 4;
        let tint = if row % 8 < 4 { 1.0 } else { 0.01 };
        for (column, alpha) in alphas.into_iter().enumerate() {
            let blocks = if visible || (left && kind < 2) {
                alpha > 32
            } else if kind == 1 || kind == 3 {
                alpha as f32 / 255.0 * tint >= 0.5
            } else {
                true
            };
            let offset = row * 256 + column * 16;
            let actual = u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("record"));
            let expected = if blocks { (row * 7 + column + 1) as u32 } else { 0 };
            assert_eq!(
                actual, expected,
                "left={left}, visible={visible}, kind={kind}, tint={tint}, alpha={alpha}"
            );
        }
    }
}
