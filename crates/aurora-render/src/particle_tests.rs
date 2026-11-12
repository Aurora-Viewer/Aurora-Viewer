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
    let shader = format!(
        "{}\n@fragment fn fs_particle_uv(in: PartOut) -> @location(0) vec4<f32> {{ return vec4<f32>(in.uv, 0.0, 1.0); }}",
        particle_shader()
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
    let uv_pipeline = make_pipeline(wgpu::BlendState::REPLACE, "fs_particle_uv", wgpu::ColorWrites::ALL);
    const WIRES: [u8; 8] = [0, 1, 2, 3, 4, 5, 7, 9];
    const WIDTH: u32 = 64 * 9;
    const HEIGHT: u32 = 6 * 9;
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
        size: crate::readback::readback_size(u64::from(STRIDE * HEIGHT)),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    for row in 0..6 {
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
        if row == 5 {
            for (column, particle) in data.iter_mut().enumerate() {
                if column % 2 == 0 {
                    particle.flags |= particle_flags::AXIS;
                    particle.pos[1] = -1.0;
                    particle.end_pos = [0.0, 1.0, 0.5];
                    particle.axis = [1.0, 0.0, 0.0];
                    particle.end_axis = particle.axis;
                    particle.end_width = 2.0;
                }
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
            let pipeline = if row == 5 {
                &uv_pipeline
            } else {
                &pipelines[particles::blend_index(src, dst) as usize]
            };
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
    // LLVOPartGroup's parent corners have u=0 and child corners u=1;
    // v spans their width. Probe both axes away from the centre so a
    // rotated or mirrored ribbon texture cannot pass as an ordinary sprite.
    for column in 0..64 {
        for (x, y) in [(2, 4), (6, 4), (4, 2), (4, 6)] {
            let pixel = [(x as f32 + 0.5) / 9.0, (y as f32 + 0.5) / 9.0];
            let expected = if column % 2 == 0 { [pixel[1], 1.0 - pixel[0]] } else { pixel };
            let offset = (5 * 9 + y) * STRIDE as usize + (column * 9 + x) * 4;
            for channel in 0..2 {
                let actual = f32::from(pixels[offset + channel]) / 255.0;
                assert!(
                    (actual - expected[channel]).abs() < 1.1 / 255.0,
                    "column {column}, pixel {x}/{y}, UV {channel}: {actual} vs {}",
                    expected[channel]
                );
            }
        }
    }
}

fn particle_shader() -> String {
    let object = include_str!("shaders/object.wgsl");
    let section = &object[object.find("override PARTICLE_GLOW_PASS:").expect("particle shader")
        ..object
            .find("// ------------------------------------------------------------- impostors")
            .expect("particle end")];
    let a = section.find("fn particle_light(").expect("lighting");
    let b = section.find("fn particle_tex(").expect("texture");
    format!(
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
    )
}

#[test]
fn ordered_particle_commands_match_reference_pixels_after_data_and_binding_changes() {
    let _gpu = crate::test_gpu::lock();
    let instance = wgpu::Instance::default();
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
        eprintln!("no GPU adapter; particle command regression skipped");
        return;
    };
    // The default feature set verifies that command chunks need no optional
    // hardware capabilities.
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).expect("particle command GPU");
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("particle command regression"),
        source: wgpu::ShaderSource::Wgsl(particle_shader().into()),
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
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
    let mut commands = crate::particle_commands::Commands::default();
    let mut previous_pixels = Vec::new();
    for samples in [1, 4, 4] {
        let make = |blend, fragment, glow_pass: bool| {
            device.create_render_pipeline_cached(&wgpu::RenderPipelineDescriptor {
                label: Some("particle commands"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_particle"),
                    compilation_options: wgpu::PipelineCompilationOptions {
                        constants: &[("PARTICLE_GLOW_PASS", f64::from(u8::from(glow_pass)))],
                        ..Default::default()
                    },
                    buffers: &[Some(crate::renderer::particle_layout())],
                },
                primitive: wgpu::PrimitiveState {
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState {
                    count: samples,
                    ..Default::default()
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(fragment),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        blend: Some(blend),
                        write_mask: if fragment == "fs_particle_glow" {
                            wgpu::ColorWrites::ALPHA
                        } else {
                            wgpu::ColorWrites::ALL
                        },
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let alpha = make(particles::blend_state(0), "fs_particle", false);
        let replace_index = particles::blend_index(0, 1) as usize;
        let replace = make(particles::blend_state(replace_index), "fs_particle", false);
        let reference_pipelines: Vec<_> = (0..particles::BLEND_COUNT)
            .map(|i| if i == replace_index { replace.clone() } else { alpha.clone() })
            .collect();
        let pipelines =
            crate::renderer::ParticlePipelines::new(&device, &module, &pipeline_layout, samples, wgpu::TextureFormat::Rgba8Unorm, None);
        assert_eq!(pipelines.compiled_count(), 1, "a new generation builds only ordinary alpha");
        let glow_state = wgpu::BlendState {
            color: wgpu::BlendComponent::REPLACE,
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        };
        let reference_glow = make(glow_state, "fs_particle_glow", false);
        let glow = make(glow_state, "fs_particle_glow", true);
        // Keep the texture itself for readback.
        let (target, resolved) = crate::renderer::make_view(
            &device,
            "particle command pixels",
            128,
            64,
            wgpu::TextureFormat::Rgba8Unorm,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let msaa = (samples > 1).then(|| {
            crate::renderer::make_view(
                &device,
                "particle command samples",
                128,
                64,
                wgpu::TextureFormat::Rgba8Unorm,
                samples,
                wgpu::TextureUsages::RENDER_ATTACHMENT,
            )
        });
        let mut data = vec![
            ParticleInstance {
                pos: [0.0, 0.0, 0.5],
                size: [1.8, 1.8],
                texture: 1,
                color: [255, 40, 20, 160],
                glow: 0.5,
                flags: particle_flags::EMISSIVE,
                ..Default::default()
            };
            6
        ];
        data[1].pos[0] = 5.0;
        data[1].blend = replace_index as u32;
        data[2].color = [20, 170, 255, 120];
        data[2].glow = 0.25;
        data[3].color[3] = 0;
        data[3].glow = 0.0;
        data[4].pos = [0.3, 0.3, 0.5];
        data[4].size = [0.4; 2];
        data[4].color = [20, 255, 40, 0];
        data[4].glow = 0.0;
        data[4].flags |= particle_flags::KEEP_ZERO_ALPHA;
        data[4].blend = replace_index as u32;
        data[5].flags |= particle_flags::AXIS;
        data[5].pos = [3.0, -0.3, 0.5];
        data[5].axis = [0.0, 1.0, 0.0];
        data[5].size = [0.2, 1.0];
        data[5].end_pos = [-0.5, -0.3, 0.5];
        data[5].end_axis = [0.0, 1.0, 0.0];
        data[5].end_width = 0.5;
        data[5].end_color = [60, 255, 120, 180];
        data[5].glow = 0.0;
        data[5].end_glow = 0.4;
        for i in 0..1200 {
            data.push(ParticleInstance {
                pos: [-0.85 + (i % 10) as f32 * 0.016, -0.85 + (i / 10) as f32 * 0.004, 0.5],
                size: [0.02; 2],
                texture: 1,
                color: [100, 120, 255, 160],
                flags: particle_flags::EMISSIVE | particle_flags::KEEP_ZERO_ALPHA,
                blend: if i % 2 == 0 { 0 } else { replace_index as u32 },
                ..Default::default()
            });
        }
        let mut vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&data),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });
        let mut frame = [0.0_f32; 36];
        frame[..16].copy_from_slice(&glam::Mat4::IDENTITY.to_cols_array());
        frame[16] = 1.0;
        frame[21] = 1.0;
        frame[26] = 5.0;
        frame[28..32].copy_from_slice(&[0.5, 0.7, 0.9, 0.65]);
        frame[32..34].copy_from_slice(&[1.0, 0.4]);
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&frame),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let bind = || {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                }],
            })
        };
        let mut group = bind();
        let prepare = |commands: &mut crate::particle_commands::Commands, data: &[ParticleInstance]| {
            commands.plan.prepare(data, true, &particles::Frustum::new(glam::Mat4::IDENTITY));
            commands.prepare(
                &device,
                &[&group],
                &vertices,
                &pipelines,
                &glow,
                wgpu::TextureFormat::Rgba8Unorm,
                None,
                samples,
            )
        };
        prepare(&mut commands, &[]);
        prepare(&mut commands, &data[..2]);
        assert_eq!(
            pipelines.compiled_count(),
            1,
            "empty lists and culled pairs do not compile variants"
        );
        let stats = prepare(&mut commands, &data[..5]);
        assert_eq!(stats.bundles, 0, "short lists prepare variants even without command bundles");
        assert_eq!(
            pipelines.compiled_count(),
            2,
            "the first visible replacement draw compiles its pair"
        );
        let replacement = pipelines[replace_index].clone();
        for step in 0..3 {
            if step == 1 {
                // Adding an unused pair must not invalidate commands for the
                // generation's existing pairs (nor recompile those pairs).
                pipelines.ensure(&device, particles::blend_index(7, 0) as usize);
                data[0].color = [50, 255, 180, 160];
                data[0].glow = 0.3;
                queue.write_buffer(&vertices, 0, bytemuck::cast_slice(&data));
            } else if step == 2 {
                data.swap(2, 4);
                vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: None,
                    contents: bytemuck::cast_slice(&data),
                    usage: wgpu::BufferUsages::VERTEX,
                });
                frame[31] = 0.35;
                frame[19] = 0.5;
                queue.write_buffer(&uniform, 0, bytemuck::cast_slice(&frame));
                group = bind();
            }
            commands.plan.prepare(&data, true, &particles::Frustum::new(glam::Mat4::IDENTITY));
            let stats = commands.prepare(
                &device,
                &[&group],
                &vertices,
                &pipelines,
                &glow,
                wgpu::TextureFormat::Rgba8Unorm,
                None,
                samples,
            );
            assert_eq!(pipelines.compiled_count(), if step == 0 { 2 } else { 3 });
            assert_eq!(pipelines[replace_index], replacement, "repeated prepares reuse the pipeline");
            assert!(stats.culled >= 1);
            if step == 1 {
                assert_eq!(stats.bundle_builds, 0, "data updates reuse exact command ranges");
            } else {
                assert!(stats.bundle_builds > 0, "bindings/sample changes invalidate chunks");
            }
            let readback = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: crate::readback::readback_size(128 * 64 * 4),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: msaa.as_ref().map_or(&resolved, |t| &t.1),
                        depth_slice: None,
                        resolve_target: (samples > 1).then_some(&resolved),
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: 0.1,
                                g: 0.2,
                                b: 0.3,
                                a: 0.4,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
                pass.set_viewport(0.0, 0.0, 64.0, 64.0, 0.0, 1.0);
                pass.set_bind_group(0, &group, &[]);
                pass.set_vertex_buffer(0, vertices.slice(..));
                for batch in particles::batches(&data, true) {
                    pass.set_pipeline(&reference_pipelines[batch.blend]);
                    pass.draw(0..6, batch.instances.clone());
                    if batch.glow {
                        pass.set_pipeline(&reference_glow);
                        pass.draw(0..6, batch.instances);
                    }
                }
                pass.set_viewport(64.0, 0.0, 64.0, 64.0, 0.0, 1.0);
                commands.draw(&mut pass, &pipelines, &glow);
            }
            encoder.copy_texture_to_buffer(
                target.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(128 * 4),
                        rows_per_image: None,
                    },
                },
                target.size(),
            );
            queue.submit([encoder.finish()]);
            readback.map_async(wgpu::MapMode::Read, .., |r| r.expect("particle command map"));
            device
                .poll(wgpu::PollType::wait_indefinitely())
                .expect("particle command completion");
            let pixels = readback.get_mapped_range(..).expect("particle command pixels");
            let mut reference = Vec::new();
            for y in 0..64usize {
                let row = &pixels[y * 512..(y + 1) * 512];
                assert_eq!(&row[..256], &row[256..], "samples={samples}, step={step}, row={y}");
                reference.extend_from_slice(&row[..256]);
            }
            if step == 1 {
                assert_ne!(reference, previous_pixels, "the fixture must detect changed data");
            }
            assert!(
                reference.as_chunks::<4>().0.iter().any(|p| p.as_slice() != &reference[..4]),
                "the fixture rendered visible pixels"
            );
            previous_pixels = reference;
        }
    }
}
