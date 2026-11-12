//! Rasterize the production part shaders with their production pipeline states.
use super::*;
use crate::types::flags;
use wgpu::util::DeviceExt;

fn declaration(source: &str, name: &str) -> String {
    let start = source.find(&format!("struct {name} {{")).expect("production struct");
    let end = start + source[start..].find("};").expect("struct end") + 2;
    source[start..end].to_owned()
}

fn function(source: &str, name: &str) -> String {
    let start = source.find(&format!("fn {name}(")).expect("production function");
    let end = start + source[start..].find("\n}").expect("function end") + 2;
    source[start..end].to_owned()
}

fn shader() -> String {
    let object = include_str!("shaders/object.wgsl").replace("\r\n", "\n");
    let common = include_str!("shaders/common.wgsl");
    let mut source = format!(
        r#"
const FLAG_SKINNED:u32={skinned}u;
const FLAG_ALPHA_MASK:u32={mask}u;
const FLAG_ALPHA_BLEND:u32={blend}u;
const FLAG_DOUBLE_SIDED:u32={sided}u;
{record}
struct SkinBind {{joint:vec4<u32>}};
const skin_binds=array<SkinBind,2>(SkinBind(vec4<u32>(0u)),SkinBind(vec4<u32>(1u,0u,0u,0u)));
struct Frame {{eye:vec4<f32>,part_joints:array<vec4<u32>,2>,part_classes:array<vec4<u32>,8>}};
// Joint 0 belongs to Body (1), joint 1 to the left hand (3).
const frame=Frame(vec4<f32>(0.0),array<vec4<u32>,2>(vec4<u32>(2u,0u,0u,0u),vec4<u32>(0u)),
    array<vec4<u32>,8>(vec4<u32>(49u,0u,0u,0u),vec4<u32>(0u),vec4<u32>(0u),vec4<u32>(0u),
        vec4<u32>(0u),vec4<u32>(0u),vec4<u32>(0u),vec4<u32>(0u)));
@group(0) @binding(0) var<storage,read> records:array<DrawRecord>;
struct VsIn {{pos:vec3<f32>,weights:vec4<f32>,joints:vec4<u32>,instance:u32}};
fn eye_model(r:DrawRecord,v:VsIn)->mat4x4<f32> {{return r.model;}}
fn vertex_position(r:DrawRecord,v:VsIn)->vec3<f32> {{return v.pos;}}
fn eye_clip(r:DrawRecord,p:vec4<f32>)->vec4<f32> {{return p;}}
fn poser_part_uv(r:DrawRecord,v:VsIn)->vec2<f32> {{return vec2<f32>(0.0);}}
fn sample_tex(t:u32,uv:vec2<f32>)->vec4<f32> {{return vec4<f32>(1.0);}}
fn portal_clip(r:DrawRecord,p:vec3<f32>) {{}}
fn input(vertex:u32,instance:u32)->VsIn {{
    let r=records[instance];
    let index=(vertex+r.tex.x)%3u;
    let positions=array<vec2<f32>,3>(vec2<f32>(-1.0,-1.0),vec2<f32>(3.0,-1.0),vec2<f32>(-1.0,3.0));
    var physical=index;
    if(r.tex.y!=0u && index!=0u) {{physical=3u-index;}}
    var joint=select(1u,0u,physical==0u);
    if(r.tex.z==1u) {{joint=0u;}}
    if(r.tex.z==2u) {{joint=1u;}}
    return VsIn(vec3<f32>(positions[physical],r.params.x),vec4<f32>(1.0,0.0,0.0,0.0),vec4<u32>(joint,0u,0u,0u),instance);
}}
"#,
        skinned = flags::SKINNED,
        mask = flags::ALPHA_MASK,
        blend = flags::ALPHA_BLEND,
        sided = flags::DOUBLE_SIDED,
        record = declaration(common, "DrawRecord"),
    );
    for name in ["PoserPartOut", "PoserPickOut"] {
        source.push_str(&declaration(&object, name));
    }
    source.push_str(
        object
            .lines()
            .find(|line| line.starts_with("const POSER_PART_BLEND_ALPHA:"))
            .expect("production blend threshold"),
    );
    for name in [
        "poser_part_texel_shown",
        "in_poser_part",
        "vs_part_mask",
        "fs_part_mask",
        "poser_pick_class",
        "vs_part_pick",
        "fs_part_pick",
    ] {
        if name.starts_with("fs_") {
            source.push_str("\n@fragment\n");
        }
        source.push_str(&function(&object, name));
    }
    source.push_str(
        r#"
@vertex fn pick(@builtin(vertex_index) v:u32,@builtin(instance_index) i:u32)->PoserPickOut {return vs_part_pick(input(v,i));}
@vertex fn mask(@builtin(vertex_index) v:u32,@builtin(instance_index) i:u32)->PoserPartOut {return vs_part_mask(input(v,i));}
"#,
    );
    source
}

#[test]
fn poser_parts_match_interpolated_weights_sidedness_and_nearest_visible_coverage_on_gpu() {
    let _gpu = crate::test_gpu::lock();
    let instance = wgpu::Instance::default();
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
        return;
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).expect("part GPU device");
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("production poser parts regression"),
        source: wgpu::ShaderSource::Wgsl(shader().into()),
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let pipeline = |mask: bool| {
        let base = PipeDesc {
            label: "part regression",
            vs: if mask { "mask" } else { "pick" },
            fs: Some(if mask { "fs_part_mask" } else { "fs_part_pick" }),
            vb: Vb::None,
            cull: None,
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
            depth_write: false,
            depth_compare: wgpu::CompareFunction::GreaterEqual,
            a2c: false,
            samples: 1,
            format: None,
            gbuf: false,
            depth_format: Some(DEPTH_FORMAT),
            depth_bias: Default::default(),
            constants: &[],
            polygon_line: false,
            unclipped_depth: false,
        };
        make_pipeline(
            &device,
            &module,
            &pipeline_layout,
            part_desc(
                base,
                if mask {
                    wgpu::TextureFormat::Rg8Unorm
                } else {
                    crate::part_pick::FORMAT
                },
                if mask {
                    wgpu::ColorWrites::RED | wgpu::ColorWrites::GREEN
                } else {
                    wgpu::ColorWrites::RED
                },
            ),
        )
    };
    let pick_pipeline = pipeline(false);
    let mask_pipeline = pipeline(true);
    let (scene, scene_view) = make_view(
        &device,
        "part source depth",
        16,
        16,
        DEPTH_FORMAT,
        1,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    );
    let (depth, depth_view) = make_view(
        &device,
        "part private depth",
        16,
        16,
        DEPTH_FORMAT,
        1,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_DST,
    );
    let (pick, pick_view) = make_view(
        &device,
        "part result",
        16,
        16,
        crate::part_pick::FORMAT,
        1,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    );
    let (mask, mask_view) = make_view(
        &device,
        "part mask",
        16,
        16,
        wgpu::TextureFormat::Rg8Unorm,
        1,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    );
    let mut overlay = crate::selection::Overlay::new_part(&device, wgpu::TextureFormat::Rgba8Unorm, [16, 16]);
    overlay.mask = mask_view;
    let face = |flags: u32, z: f32, rotate: u32, back: bool, joint_mode: u32, alpha: f32| DrawRecord {
        flags: [flags, 0, 0, 0],
        params: [z, 0.0, 0.0, 0.5],
        tex: [rotate, u32::from(back), joint_mode, 0],
        base_color: [1.0, 1.0, 1.0, alpha],
        ..Default::default()
    };
    let skin = flags::SKINNED;
    let mixed = face(skin, 0.6, 0, false, 0, 1.0);
    let hand = face(skin, 0.8, 0, false, 2, 1.0);
    let covered_hand = face(skin, 0.6, 0, false, 2, 1.0);
    let body = face(skin, 0.6, 0, false, 1, 1.0);
    let accessory = face(flags::ALPHA_BLEND, 0.8, 0, false, 1, 1.0);
    let cases = [
        ("mixed weights", vec![mixed], 3, [255, 255]),
        ("cyclic indices", vec![face(skin, 0.6, 1, false, 0, 1.0)], 3, [255, 255]),
        ("invisible back", vec![face(skin, 0.6, 0, true, 2, 1.0)], 0, [0, 0]),
        (
            "double-sided back",
            vec![face(skin | flags::DOUBLE_SIDED, 0.6, 0, true, 2, 1.0)],
            3,
            [255, 255],
        ),
        ("accessory first", vec![accessory, covered_hand], 0, [0, 255]),
        ("accessory last", vec![covered_hand, accessory], 0, [0, 255]),
        ("hand first", vec![hand, body], 3, [255, 255]),
        ("hand last", vec![body, hand], 3, [255, 255]),
        (
            "clear blend",
            vec![face(flags::ALPHA_BLEND, 0.8, 0, false, 1, 0.05), body],
            1,
            [0, 255],
        ),
        (
            "masked hole",
            vec![face(flags::ALPHA_MASK, 0.8, 0, false, 1, 0.2), hand],
            3,
            [255, 255],
        ),
        ("occluded by scene", vec![face(skin, 0.2, 0, false, 2, 1.0)], 0, [0, 0]),
    ];
    for (name, faces, expected_part, expected_mask) in cases {
        let records = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&faces),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: records.as_entire_binding(),
            }],
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: crate::readback::readback_size(768),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &scene_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.5),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
        }
        // This is a private copy: the part passes must never modify scene depth.
        encoder.copy_texture_to_texture(scene.as_image_copy(), depth.as_image_copy(), scene.size());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &pick_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_scissor_rect(14, 1, 1, 1);
            pass.set_pipeline(&pick_pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.draw(0..3, 0..faces.len() as u32);
        }
        {
            let mut pass = overlay.begin_mask(&mut encoder, &depth_view);
            pass.set_pipeline(&mask_pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.draw(0..3, 0..faces.len() as u32);
        }
        // WebGPU requires whole-subresource copies for depth textures.
        let scene_readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: crate::readback::readback_size(4096),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            scene.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &scene_readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: Some(16),
                },
            },
            scene.size(),
        );
        for (i, texture) in [&pick, &mask].into_iter().enumerate() {
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x: 14, y: 1, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: i as u64 * 256,
                        bytes_per_row: Some(256),
                        rows_per_image: Some(1),
                    },
                },
                wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
            );
        }
        queue.submit([encoder.finish()]);
        readback.map_async(wgpu::MapMode::Read, .., |r| r.expect("part readback"));
        scene_readback.map_async(wgpu::MapMode::Read, .., |r| r.expect("scene depth readback"));
        device.poll(wgpu::PollType::wait_indefinitely()).expect("part GPU completion");
        let bytes = readback.get_mapped_range(..).expect("part bytes");
        assert_eq!(
            u32::from_le_bytes(bytes[..4].try_into().unwrap()),
            expected_part,
            "{name}: picked part"
        );
        assert_eq!(&bytes[256..258], &expected_mask, "{name}: mask share and coverage");
        let depth_bytes = scene_readback.get_mapped_range(..).expect("scene depth bytes");
        assert_eq!(
            f32::from_le_bytes(depth_bytes[312..316].try_into().unwrap()),
            0.5,
            "{name}: scene depth unchanged"
        );
    }
}
