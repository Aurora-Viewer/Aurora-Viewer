//! Rasterize both windings through the production blend/depth fragment cuts.
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
fn shade_material(v:VsOut,m:Material)->Material {{return m;}}
fn rez_alpha(r:DrawRecord,v:VsOut)->f32 {{return 1.0;}}
fn depth_normal(n:vec3<f32>,front:bool,cur:vec4<f32>,prev:vec4<f32>)->vec4<f32> {{return vec4<f32>(1.0);}}
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
        size: 384 * 256,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    for row in 0..24 {
        let entry = ["colour", "contact", "normal", "opaque"][row % 4];
        let water_clip = (row / 8) as f64;
        let pipelines = [false, true].map(|double_sided| {
            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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
