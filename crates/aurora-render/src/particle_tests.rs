//! Render the production particle geometry, coverage and fragment outputs with
//! controlled lighting. Compare every supported blend pair with the reference
//! equation; in particular zero-alpha destination masks must not be discarded.

use crate::pipeline_cache::CachedPipelines;
use crate::{ParticleInstance, particle_flags, particles};
use wgpu::util::DeviceExt;

#[test]
fn particle_gpu_blends_glow_and_optional_intersection_fade_match_reference_equations() {
    let _gpu = crate::test_gpu::lock();
    let instance = wgpu::Instance::default();
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
        eprintln!("no GPU adapter; particle blend regression skipped");
        return;
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).expect("particle GPU device");
    let object = include_str!("shaders/object.wgsl");
    let section = &object[object.find("const PART_EMISSIVE:").expect("particle shader")
        ..object
            .find("// ------------------------------------------------------------- impostors")
            .expect("particle end")];
    let a = section.find("fn particle_light(").expect("lighting");
    let b = section.find("fn particle_tex(").expect("texture");
    let shader = format!(
        r#"
struct Frame {{ view_proj:mat4x4<f32>,cam_right:vec4<f32>,cam_up:vec4<f32>,camera_pos:vec4<f32>,tex:vec4<f32>,depth:vec4<f32> }};
@group(0) @binding(0) var<uniform> frame:Frame;
fn srgb_to_linear(c:vec3<f32>)->vec3<f32> {{
    return select(pow((c+vec3<f32>(0.055))/1.055,vec3<f32>(2.4)),c/12.92,c<=vec3<f32>(0.04045));
}}
fn sample_tex(t:u32,uv:vec2<f32>)->vec4<f32> {{ return frame.tex; }}
fn finite_hdr(c:vec3<f32>)->vec3<f32> {{ return c; }}
fn screen_uv(p:vec2<f32>)->vec2<f32> {{ return p; }}
fn scene_depth_at(p:vec2<f32>)->f32 {{ return frame.depth.x; }}
fn world_from_depth(p:vec2<f32>,d:f32)->vec3<f32> {{ return vec3<f32>(0.0,0.0,frame.depth.y); }}
{}
fn particle_light(base:vec3<f32>,input:PartOut)->vec3<f32> {{ return base; }}
{}
"#,
        &section[..a],
        &section[b..]
    );
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("production particle blending and glow"),
        source: wgpu::ShaderSource::Wgsl(shader.into()),
    });
    let make_pipeline = |blend, fragment: &'static str, mask| {
        device.create_render_pipeline_cached(&wgpu::RenderPipelineDescriptor {
            label: Some("particle regression"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_particle"),
                compilation_options: Default::default(),
                buffers: &[Some(crate::renderer::particle_layout())],
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some(fragment),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: Some(blend),
                    write_mask: mask,
                })],
            }),
            multiview_mask: None,
            cache: None,
        })
    };
    let pipelines: Vec<_> = (0..particles::BLEND_COUNT)
        .map(|i| make_pipeline(particles::blend_state(i), "fs_particle", wgpu::ColorWrites::ALL))
        .collect();
    let glow = make_pipeline(
        wgpu::BlendState {
            color: wgpu::BlendComponent::REPLACE,
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        },
        "fs_particle_glow",
        wgpu::ColorWrites::ALPHA,
    );
    const WIRES: [u8; 8] = [0, 1, 2, 3, 4, 5, 7, 9];
    const WIDTH: u32 = 64 * 9;
    const HEIGHT: u32 = 5 * 9;
    const STRIDE: u32 = WIDTH * 4;
    let (target, view) = crate::renderer::make_view(
        &device,
        "particle blend equations",
        WIDTH,
        HEIGHT,
        wgpu::TextureFormat::Rgba8Unorm,
        1,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    );
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(STRIDE * HEIGHT),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    for row in 0..5 {
        let mut frame = [0.0_f32; 36];
        frame[..16].copy_from_slice(&glam::Mat4::IDENTITY.to_cols_array());
        frame[16] = 1.0;
        frame[19] = if row == 4 { 0.5 } else { 0.0 };
        frame[21] = 1.0;
        frame[26] = 5.0;
        frame[28..32].copy_from_slice(&[
            0.5,
            0.7,
            0.9,
            if row == 0 {
                0.0
            } else if row == 2 {
                1.0
            } else {
                0.5
            },
        ]);
        frame[32..34].copy_from_slice(&[1.0, 0.4]);
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&frame),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let mut data = Vec::new();
        for src in WIRES {
            for dst in WIRES {
                data.push(ParticleInstance {
                    pos: [0.0, 0.0, 0.5],
                    size: [2.0, 2.0],
                    color: [if row == 2 { 128 } else { 255 }, 255, 255, if row == 3 { 51 } else { 255 }],
                    texture: 1,
                    glow: if row == 3 { 0.6 } else { 0.0 },
                    flags: particle_flags::EMISSIVE | if src != 7 && dst != 7 { particle_flags::KEEP_ZERO_ALPHA } else { 0 },
                    ..Default::default()
                });
            }
        }
        let instances = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&data),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: if row == 0 {
                        wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.2,
                            g: 0.4,
                            b: 0.6,
                            a: 0.4,
                        })
                    } else {
                        wgpu::LoadOp::Load
                    },
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_vertex_buffer(0, instances.slice(..));
        for (column, (src, dst)) in WIRES.into_iter().flat_map(|s| WIRES.map(move |d| (s, d))).enumerate() {
            let pipeline = &pipelines[particles::blend_index(src, dst) as usize];
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &pipeline.get_bind_group_layout(0),
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                }],
            });
            pass.set_viewport(column as f32 * 9.0, row as f32 * 9.0, 9.0, 9.0, 0.0, 1.0);
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.draw(0..6, column as u32..column as u32 + 1);
            if row == 3 {
                let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: None,
                    layout: &glow.get_bind_group_layout(0),
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniform.as_entire_binding(),
                    }],
                });
                pass.set_pipeline(&glow);
                pass.set_bind_group(0, &group, &[]);
                pass.draw(0..6, column as u32..column as u32 + 1);
            }
        }
    }
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(STRIDE),
                rows_per_image: None,
            },
        },
        target.size(),
    );
    queue.submit([encoder.finish()]);
    readback.map_async(wgpu::MapMode::Read, .., |r| r.expect("particle pixels"));
    device.poll(wgpu::PollType::wait_indefinitely()).expect("particle completion");
    let pixels = readback.get_mapped_range(..).expect("particle readback");
    let linear = |v: f32| ((v + 0.055) / 1.055).powf(2.4);
    let source = [linear(0.5), linear(0.7), linear(0.9)];
    let destination = [0.2, 0.4, 0.6];
    let factor = |wire: u8, s: f32, d: f32, a: f32| match wire {
        0 => 1.0,
        1 => 0.0,
        2 => d,
        3 => s,
        4 => 1.0 - d,
        5 => 1.0 - s,
        7 => a,
        _ => 1.0 - a,
    };
    for row in 0..5 {
        let alpha = match row {
            0 => 0.0,
            1 => 0.5,
            2 => 1.0,
            _ => 0.1,
        };
        for (column, (src, dst)) in WIRES.into_iter().flat_map(|s| WIRES.map(move |d| (s, d))).enumerate() {
            let offset = (row * 9 + 4) * STRIDE as usize + (column * 9 + 4) * 4;
            for channel in 0..4 {
                let expected = if channel == 3 {
                    0.4 * (1.0 - alpha) + if row == 3 { 0.3 } else { 0.0 }
                } else if row == 0 && (src == 7 || dst == 7) {
                    destination[channel]
                } else {
                    let s = if row == 2 && channel == 0 {
                        linear(0.5 * 128.0 / 255.0)
                    } else {
                        source[channel]
                    };
                    let d = destination[channel];
                    s * factor(src, s, d, alpha) + d * factor(dst, s, d, alpha)
                };
                let actual = f32::from(pixels[offset + channel]) / 255.0;
                assert!(
                    (actual - expected.clamp(0.0, 1.0)).abs() < 2.1 / 255.0,
                    "row {row}, factors {src}/{dst}, channel {channel}: {actual} vs {expected}"
                );
            }
        }
    }
}
