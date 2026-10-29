use super::*;

/// Exercise real Vulkan pipelines, including the two-pass modes and chained
/// targets. This catches feedback attachments and per-pass uniform reuse that
/// parsing a WGSL module cannot detect.
#[test]
#[ignore = "requires a Vulkan device"]
fn rlva_gpu_spheres_preserve_uniform_images_and_compose_black_masks_without_glow() {
    let _gpu = crate::test_gpu::lock();
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: crate::BACKENDS,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).expect("Vulkan adapter");
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: adapter.limits(),
        required_features: wgpu::Features::TEXTURE_BINDING_ARRAY
            | wgpu::Features::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING,
        ..Default::default()
    }))
    .expect("device");
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let size = (64, 64);
    let mut spheres = Spheres::new(&device, format);
    let (_, depth) = crate::renderer::make_view(
        &device,
        "RLVa test depth",
        size.0,
        size.1,
        wgpu::TextureFormat::Depth32Float,
        1,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
    );
    let (output, out) = crate::renderer::make_view(
        &device,
        "RLVa test output",
        size.0,
        size.1,
        format,
        1,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    );
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("RLVa test readback"),
        size: 64 * 64 * 4,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let base = SphereEffect {
        origin: glam::Vec3::ZERO,
        mode: 0,
        extend: 3,
        range: [0.5, 0.0, 0.5, 0.0],
        params: [0.0; 4],
    };
    let cases: Vec<(Vec<SphereEffect>, [u8; 4])> = vec![
        (vec![base], [102, 51, 26, 255]),
        (vec![base, base], [51, 26, 13, 255]),
        (
            vec![SphereEffect {
                mode: 1,
                range: [2.0, 0.0, 2.0, 0.0],
                ..base
            }],
            [204, 102, 51, 255],
        ),
        (
            vec![SphereEffect {
                mode: 2,
                range: [2.0, 0.0, 2.0, 0.0],
                params: [7.0, 0.0, 0.0, 0.0],
                ..base
            }],
            [204, 102, 51, 255],
        ),
        (
            vec![SphereEffect {
                mode: 3,
                params: [3.0, 0.0, -3.0, 0.0],
                ..base
            }],
            [204, 102, 51, 255],
        ),
        (
            vec![SphereEffect {
                mode: 4,
                params: [8.0, 8.0, 0.0, 0.0],
                ..base
            }],
            [204, 102, 51, 255],
        ),
    ];
    for (effects, expected) in cases {
        let source = spheres.prepare(&device, size, format);
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("RLVa composed scene fixture"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: source,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.8,
                            g: 0.4,
                            b: 0.2,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }
        spheres.encode(&device, &queue, &mut encoder, &depth, Mat4::IDENTITY, size, &effects, &out);
        encoder.copy_texture_to_buffer(
            output.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: Some(64),
                },
            },
            wgpu::Extent3d {
                width: 64,
                height: 64,
                depth_or_array_layers: 1,
            },
        );
        queue.submit([encoder.finish()]);
        readback.map_async(wgpu::MapMode::Read, .., |result| result.expect("RLVa readback"));
        device.poll(wgpu::PollType::wait_indefinitely()).expect("GPU completion");
        let bytes = readback.get_mapped_range(..).expect("mapped pixels");
        for pixel in bytes.as_chunks::<4>().0 {
            assert!(
                pixel.iter().zip(expected).all(|(value, expected)| value.abs_diff(expected) <= 1),
                "mode {}: {pixel:?}, expected {expected:?}",
                effects[0].mode
            );
        }
        drop(bytes);
        readback.unmap();
    }

    // The overlay must be an independent scene pass: egui is deliberately
    // absent, as it is in a snapshot without the viewer interface.
    let page = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("RLVa alpha holes"),
        size: wgpu::Extent3d {
            width: 2,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        page.as_image_copy(),
        &[255, 255, 255, 0, 255, 255, 255, 255],
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(8),
            rows_per_image: Some(1),
        },
        wgpu::Extent3d {
            width: 2,
            height: 1,
            depth_or_array_layers: 1,
        },
    );
    let view = page.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    });
    let sampler = device.create_sampler(&Default::default());
    let locations = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 4,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&locations, 0, &[0; 4]);
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: std::num::NonZeroU32::new(1),
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    });
    let textures = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureViewArray(&[&view]),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: locations.as_entire_binding(),
            },
        ],
    });
    let overlays = overlay::Overlays::new(&device, format, &layout);
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &out,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.8,
                        g: 0.4,
                        b: 0.2,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
    }
    overlays.encode(
        &queue,
        &mut encoder,
        &out,
        &textures,
        &[crate::OverlayEffect {
            color: [1.0, 0.0, 0.0, 0.5],
            texture: [0; 4],
        }],
    );
    encoder.copy_texture_to_buffer(
        output.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256),
                rows_per_image: Some(64),
            },
        },
        wgpu::Extent3d {
            width: 64,
            height: 64,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    readback.map_async(wgpu::MapMode::Read, .., |result| result.expect("overlay readback"));
    device.poll(wgpu::PollType::wait_indefinitely()).expect("GPU completion");
    let bytes = readback.get_mapped_range(..).expect("mapped pixels");
    for (x, expected) in [(8, [204, 102, 51, 255]), (56, [230, 51, 26, 255])] {
        let offset = (32 * 64 + x) * 4;
        let pixel = &bytes[offset..offset + 4];
        assert!(
            pixel.iter().zip(expected).all(|(v, e)| v.abs_diff(e) <= 1),
            "overlay at x={x}: {pixel:?}"
        );
    }
    drop(bytes);
    readback.unmap();
    for smooth in [false, true] {
        let scaled = resolution::Scaled::new(&device, format, (2, 1), smooth);
        queue.write_texture(
            scaled.texture.as_image_copy(),
            &[204, 102, 51, 255, 0, 204, 255, 255],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(8),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: 2,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        let mut encoder = device.create_command_encoder(&Default::default());
        scaled.encode(&mut encoder, &out);
        overlays.encode(
            &queue,
            &mut encoder,
            &out,
            &textures,
            &[crate::OverlayEffect {
                color: [1.0, 0.0, 0.0, 0.5],
                texture: [0; 4],
            }],
        );
        encoder.copy_texture_to_buffer(
            output.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: Some(64),
                },
            },
            wgpu::Extent3d {
                width: 64,
                height: 64,
                depth_or_array_layers: 1,
            },
        );
        queue.submit([encoder.finish()]);
        readback.map_async(wgpu::MapMode::Read, .., |result| result.expect("resolution readback"));
        device.poll(wgpu::PollType::wait_indefinitely()).expect("GPU completion");
        let bytes = readback.get_mapped_range(..).expect("mapped pixels");
        for (x, expected) in [(8, [204, 102, 51, 255]), (56, [128, 102, 128, 255])] {
            let offset = (32 * 64 + x) * 4;
            assert!(
                bytes[offset..offset + 4].iter().zip(expected).all(|(v, e)| v.abs_diff(e) <= 1),
                "upscale/overlay at x={x}"
            );
        }
        if smooth {
            let red = bytes[(32 * 64 + 31) * 4];
            assert!((90..150).contains(&red), "smooth transition: {red}");
        }
        drop(bytes);
        readback.unmap();
    }
}
