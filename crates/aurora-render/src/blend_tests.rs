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
// the generic pipeline's answers (MATERIAL_CLASS 0, the full SCENE_VARIANT)
fn rec_portal_effect(f:u32)->bool {{return (f & FLAG_PORTAL_EFFECT) != 0u;}}
fn scene_rez()->bool {{return true;}}
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
const FLAG_PICK_IGNORES_ALPHA:u32={ignores_alpha}u;
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
        ignores_alpha = flags::PICK_IGNORES_ALPHA,
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

/// The source of the shader function `name`, with the comment-free
/// constants named before it.
fn shader_function(shader: &str, name: &str, constants: &[&str]) -> String {
    let mut source = String::new();
    for constant in constants {
        let line = shader
            .lines()
            .find(|line| line.starts_with(&format!("const {constant}:")) || line.starts_with(&format!("override {constant}:")))
            .unwrap_or_else(|| panic!("production constant {constant}"));
        source.push_str(&line.replacen("override", "const", 1));
        source.push('\n');
    }
    let start = shader
        .find(&format!("fn {name}("))
        .unwrap_or_else(|| panic!("production function {name}"));
    let end = start + shader[start..].find("\n}").expect("function end") + 2;
    source.push_str(&shader[start..end]);
    source.push('\n');
    source
}

/// `rule(v: vec4<f32>) -> f32` of `source` for each of `inputs`, run on
/// the GPU; none without one.
fn evaluate(source: &str, inputs: &[[f32; 4]]) -> Option<Vec<f32>> {
    let _gpu = crate::test_gpu::lock();
    let instance = wgpu::Instance::default();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).expect("rule GPU device");
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("production blended rule"),
        source: wgpu::ShaderSource::Wgsl(
            format!(
                "{source}
@group(0) @binding(0) var<storage,read> inputs:array<vec4<f32>>;
@group(0) @binding(1) var<storage,read_write> outputs:array<f32>;
@compute @workgroup_size(1) fn main(@builtin(global_invocation_id) id:vec3<u32>) {{
    outputs[id.x]=rule(inputs[id.x]);
}}"
            )
            .into(),
        ),
    });
    let pipeline = device.create_compute_pipeline_cached(&wgpu::ComputePipelineDescriptor {
        label: Some("production blended rule"),
        layout: None,
        module: &shader,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let given = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("rule inputs"),
        contents: bytemuck::cast_slice(inputs),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let size = (inputs.len() * 4) as u64;
    let [results, readback] = [
        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
    ]
    .map(|usage| {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rule results"),
            size,
            usage,
            mapped_at_creation: false,
        })
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("rule"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: given.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: results.as_entire_binding(),
            },
        ],
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(inputs.len() as u32, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&results, 0, &readback, 0, size);
    queue.submit([encoder.finish()]);
    readback.map_async(wgpu::MapMode::Read, .., |result| result.expect("rule readback"));
    device.poll(wgpu::PollType::wait_indefinitely()).expect("rule completion");
    let bytes = readback.get_mapped_range(..).expect("rule results");
    Some(bytemuck::pod_collect_to_vec(&bytes))
}

/// A layer that fades out over a skin (a tattoo, a blush, a second skin):
/// its texels not yet solid take the occlusion and the contact shadow of
/// the skin they lie on, whole, as the solid ones do. A step there drew a
/// line around the layer.
#[test]
fn a_faint_layer_takes_the_screen_terms_of_the_surface_it_lies_on_gpu() {
    let common = include_str!("shaders/common.wgsl").replace("\r\n", "\n");
    let source = shader_function(&common, "layer_on_surface", &["SCREEN_LAYER_REACH"])
        + "fn rule(v:vec4<f32>)->f32 {return layer_on_surface(v.x,v.y);}";
    // (here, there): the fragment and the surface in its pixel, from the
    // camera (m)
    let near = [0.0, 0.001, 0.004, 0.02, 0.029].map(|gap: f32| [2.5, 2.5 + gap, 0.0, 0.0]);
    // in front of it or just behind (under a solid texel of another layer,
    // some millimetres nearer than the skin)
    let behind = [[2.5, 2.496, 0.0, 0.0]];
    // a pane in front of a room, a strand before a face: none (#556)
    let far = [0.07, 0.2, 5.0].map(|gap: f32| [2.5, 2.5 + gap, 0.0, 0.0]);
    // far from the camera the reach is a hundredth of the distance
    let distant = [[50.0, 50.4, 0.0, 0.0], [50.0, 51.0, 0.0, 0.0]];
    // leaving the surface: no line either
    let leaving: Vec<[f32; 4]> = (0..=130).map(|step| [2.5, 2.5 + step as f32 * 0.0005, 0.0, 0.0]).collect();
    let inputs = [&near[..], &behind, &far, &distant, &leaving].concat();
    let Some(shares) = evaluate(&source, &inputs) else {
        return;
    };
    let (near, rest) = shares.split_at(near.len());
    assert!(near.iter().all(|&share| share == 1.0), "on the surface: {near:?}");
    assert_eq!(rest[0], 1.0, "just behind it");
    assert!(rest[1..4].iter().all(|&share| share == 0.0), "away from it: {:?}", &rest[1..4]);
    assert_eq!(rest[4..6], [1.0, 0.0], "50 m away");
    let leaving = &rest[6..];
    assert_eq!((leaving[0], leaving[130]), (1.0, 0.0));
    for pair in leaving.windows(2) {
        assert!(
            pair[1] <= pair[0] && pair[0] - pair[1] < 0.03,
            "half a millimetre further: {pair:?}"
        );
    }
}

/// The same layer in the occlusion's depth: it rises out of the skin it
/// lies on, from the skin's depth at one half to its own where it is opaque
/// to the eye, where it stood as a ledge the occlusion drew a line along.
/// Away from a solid surface a texel is in that depth only once opaque to
/// the eye: the occlusion of what stands behind a pane of glass stays its
/// own.
#[test]
fn a_fading_layer_rises_out_of_the_surface_under_it_gpu() {
    let common = include_str!("shaders/common.wgsl").replace("\r\n", "\n");
    let object = include_str!("shaders/object.wgsl").replace("\r\n", "\n");
    let enters = crate::renderer::SCREEN_BLEND_ALPHA as f32;
    let source = shader_function(&common, "blend_screen_share", &["SCREEN_BLEND_ALPHA", "SCREEN_SOLID_ALPHA"])
        + &shader_function(&object, "risen_depth", &["DEPTH_CONTACT_ALPHA", "OCCLUSION_RISE"])
        + "fn rule(v:vec4<f32>)->f32 {return risen_depth(vec4<f32>(0.0,0.0,v.x,v.y),v.w,v.z);}";
    // the main view's depth: the near plane over the distance; a fragment's
    // position holds it and one over the distance
    let (near, distance) = (0.05_f32, 2.5_f32);
    let alphas: Vec<f32> = (0..=500).map(|step| 0.5 + step as f32 * 0.001).collect();
    // how far behind itself each texel is written (m), over a solid
    // surface `gap` behind it (none: the sky)
    let behind = |gap: Option<f32>| -> Option<Vec<f32>> {
        let solid = gap.map_or(0.0, |gap| near / (distance + gap));
        let inputs: Vec<[f32; 4]> = alphas
            .iter()
            .map(|&alpha| [near / distance, 1.0 / distance, alpha, solid])
            .collect();
        let depths = evaluate(&source, &inputs)?;
        Some(
            depths
                .iter()
                .map(|depth| if *depth > 0.0 { near / depth - distance } else { f32::INFINITY })
                .collect(),
        )
    };
    // a layer 4 mm over a skin: the nearer of the two is kept. From the
    // skin where the layer is half covered to the layer where it is opaque
    // to the eye, by less than a fiftieth of a millimetre per thousandth
    // of alpha (4 mm at once before)
    let Some(layer) = behind(Some(0.004)) else {
        return;
    };
    assert!(layer[0].is_infinite(), "half covered, not written: {}", layer[0]);
    let kept: Vec<f32> = layer.iter().map(|behind| behind.min(0.004)).collect();
    for (alpha, behind) in alphas.iter().zip(&kept) {
        // never nearer than the face: the entry point promises the depth
        // test as much (`@early_depth_test(less_equal)`)
        assert!(*behind > -1e-5, "alpha {alpha}: {behind} m");
        if *alpha >= enters {
            assert!(behind.abs() < 1e-5, "alpha {alpha}, where it is: {behind} m");
        }
    }
    assert!(
        (kept[1] - 0.004).abs() < 1e-5,
        "the skin's depth where the layer starts to rise: {}",
        kept[1]
    );
    for pair in kept.windows(2) {
        assert!(pair[1] <= pair[0] + 1e-5 && pair[0] - pair[1] < 0.00002, "a ledge: {pair:?}");
    }
    // a pane 25 cm or 2 cm in front of a wall, or before the sky: not
    // written until it is opaque to the eye
    for gap in [Some(0.25), Some(0.02), None] {
        let pane = behind(gap).expect("the same GPU");
        for (alpha, behind) in alphas.iter().zip(&pane) {
            if *alpha >= enters {
                assert!(behind.abs() < 1e-5, "{gap:?}, alpha {alpha}, where it is: {behind} m");
            } else {
                assert!(behind.is_infinite(), "{gap:?}, alpha {alpha}: {behind} m");
            }
        }
    }
    // between the two, a face leaving its surface: it rises less and less
    let leaving = behind(Some(0.012)).expect("the same GPU");
    let at = |alpha: f32| alphas.iter().position(|a| (*a - alpha).abs() < 5e-4).expect("a step");
    assert!(
        leaving[at(0.87)] > 0.004 && leaving[at(0.87)] < 0.008,
        "12 mm over it: {}",
        leaving[at(0.87)]
    );
}

/// A blended fragment not yet solid takes the screen-space terms where it
/// lies on a solid surface, measured against the solid depth: in the
/// occlusion's own depth a texel between `SCREEN_BLEND_ALPHA` and
/// `SCREEN_SOLID_ALPHA` would find itself, and a pane of glass would take
/// them whole like a layer on a skin.
#[test]
fn a_blended_fragment_is_measured_against_the_solid_surfaces_only() {
    let common = include_str!("shaders/common.wgsl").replace("\r\n", "\n");
    let object = include_str!("shaders/object.wgsl").replace("\r\n", "\n");
    let renderer = include_str!("renderer.rs").replace("\r\n", "\n");
    let body = |source: &str, name: &str| {
        let from = source
            .find(&format!("fn {name}("))
            .unwrap_or_else(|| panic!("production function {name}"));
        source[from..from + source[from..].find("\n}").expect("function end")].to_owned()
    };
    let on_surface = body(&common, "on_occlusion_surface");
    assert!(on_surface.contains("textureLoad(solid_depth,"), "the depth it is measured against");
    assert!(!common.contains("var occlusion_depth"), "one depth for it");
    let shade = body(&object, "shade_material");
    assert!(shade.contains("on = on_occlusion_surface(in.clip.xy, in.world_pos);"));
    assert!(shade.contains("max(share, on)"), "the larger of the two shares");
    // the solid depth is copied between the solid texels and the others;
    // the rising ones read it and write the depth alone
    let pass = &renderer[renderer
        .find("label: Some(\"occlusion depth\")")
        .expect("the occlusion's depth pass")..];
    let solid = pass.find("draw_sided(&mut pass, solid, rg.blend, 0);").expect("the solid texels");
    let copy = pass.find("solid_tex.as_image_copy()").expect("the solid depth");
    let rising = pass
        .find("label: Some(\"occlusion depth, rising texels\")")
        .expect("the rising texels");
    assert!(solid < copy && copy < rising);
    let rising = &pass[rising..];
    let rising = &rising[..rising
        .find("draw_sided(&mut pass, &self.pipelines.pre_contact, rg.blend, 0);")
        .expect("their pipeline")];
    assert!(rising.contains("color_attachments: &[],"), "no normal written");
    assert!(
        rising.contains("bind_mesh(&mut pass, &self.groups.blend);"),
        "the solid depth bound"
    );
    assert!(body(&object, "fs_depth_occlusion").contains("risen_depth(in.clip, textureLoad(solid_depth,"));
    for (name, limit) in [
        ("prepass solid", "SCREEN_SOLID_ALPHA"),
        ("prepass solid normals", "SCREEN_SOLID_ALPHA"),
        ("prepass contact normals", "SCREEN_BLEND_ALPHA"),
    ] {
        let at = renderer
            .find(&format!("base(\"{name}\""))
            .unwrap_or_else(|| panic!("pipeline {name}"));
        let described = &renderer[renderer[..at].rfind("pipe(PipeDesc {").expect("its description")..at + 120];
        assert!(
            described.contains(&format!("(\"DEPTH_CONTACT_ALPHA\", {limit})")),
            "{name}: {described}"
        );
        assert!(described.contains("fs_depth_contact"), "{name}: {described}");
    }
    assert!(renderer.contains("pre(\"prepass contact\", \"vs_main\", Some(\"fs_depth_occlusion\"), blend_cull(side != 0))"));
    // one value each for the renderer and the shaders
    for (name, value) in [
        ("SCREEN_BLEND_ALPHA", crate::renderer::SCREEN_BLEND_ALPHA),
        ("SCREEN_SOLID_ALPHA", crate::renderer::SCREEN_SOLID_ALPHA),
    ] {
        assert!(common.contains(&format!("\nconst {name}: f32 = {value};\n")), "{name}");
    }
}

/// The screen-space occlusion, indirect light and contact shadows of a
/// blended face fade in with its opacity from the one at which it enters
/// the occlusion's depth: a step there drew a ragged line along the texels
/// of a glass texture whose opacity crosses it.
#[test]
fn blended_faces_take_the_screen_terms_by_their_opacity_without_a_step_on_gpu() {
    const STEPS: u32 = 1000;
    let common = include_str!("shaders/common.wgsl").replace("\r\n", "\n");
    let object = include_str!("shaders/object.wgsl").replace("\r\n", "\n");
    let share = shader_function(&common, "blend_screen_share", &["SCREEN_BLEND_ALPHA", "SCREEN_SOLID_ALPHA"]);
    // both terms of a blended face go through it
    let shade = &object[object.find("fn shade_material(").expect("shade_material")..];
    let shade = &shade[..shade.find("\n}").expect("shade_material end")];
    assert!(shade.contains("blend_screen_share(m.color.a)"), "the shares of a blended fragment");
    assert!(!shade.contains("m.color.a >="), "no step on the opacity");
    // the opacity from which fs_depth_occlusion writes the face into that
    // depth
    let depth_alpha = crate::renderer::SCREEN_BLEND_ALPHA as f32;
    let source = format!(
        r#"
{share}
@group(0) @binding(0) var<storage,read_write> shares:array<f32>;
@compute @workgroup_size(1) fn cs(@builtin(global_invocation_id) id:vec3<u32>) {{
    shares[id.x]=blend_screen_share(f32(id.x)/{STEPS}.0);
}}
"#
    );
    let _gpu = crate::test_gpu::lock();
    let instance = wgpu::Instance::default();
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
        return;
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).expect("share GPU device");
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("production blended share"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device.create_compute_pipeline_cached(&wgpu::ComputePipelineDescriptor {
        label: Some("blended share"),
        layout: None,
        module: &shader,
        entry_point: Some("cs"),
        compilation_options: Default::default(),
        cache: None,
    });
    let size = u64::from(STEPS + 1) * 4;
    let shares = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("shares"),
        size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("shares readback"),
        size: crate::readback::readback_size(size),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: shares.as_entire_binding(),
        }],
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(STEPS + 1, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&shares, 0, &readback, 0, size);
    queue.submit([encoder.finish()]);
    readback.map_async(wgpu::MapMode::Read, .., |r| r.expect("shares readback"));
    device.poll(wgpu::PollType::wait_indefinitely()).expect("shares completion");
    // (the readback is rounded up: only the shares computed)
    let shares = crate::readback::f32s(&readback.get_mapped_range(..).expect("shares"));
    let shares = &shares[..=STEPS as usize];
    let alpha = |i: usize| i as f32 / STEPS as f32;
    for (i, share) in shares.iter().enumerate() {
        assert!((0.0..=1.0).contains(share), "alpha {}: {share}", alpha(i));
        if alpha(i) < depth_alpha {
            // not in the occlusion's depth: the terms there are those of
            // what lies behind the face (#556)
            assert_eq!(*share, 0.0, "alpha {}", alpha(i));
        }
        if alpha(i) >= crate::renderer::SCREEN_SOLID_ALPHA as f32 {
            // solid to the eye: as the opaque faces beside it (#882)
            assert!(*share > 0.99, "alpha {}: {share}", alpha(i));
        }
    }
    assert_eq!(shares[STEPS as usize], 1.0);
    for (i, pair) in shares.windows(2).enumerate() {
        let rise = pair[1] - pair[0];
        assert!(
            (0.0..0.03).contains(&rise),
            "alpha {} to {}: {} to {}",
            alpha(i),
            alpha(i + 1),
            pair[0],
            pair[1]
        );
    }
}
