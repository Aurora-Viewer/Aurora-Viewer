use super::*;

/// Exercise both actual GPU passes on a ring and disconnected islands:
/// coverage interiors stay byte-identical after the pulse, including holes.
#[test]
#[ignore = "requires a Vulkan device"]
fn halo_preserves_material_and_tracks_holes_and_disconnected_islands() {
    let _gpu = crate::test_gpu::lock();
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: crate::BACKENDS,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).expect("Vulkan adapter");
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).expect("device");
    let mut overlay = Overlay::new(&device, wgpu::TextureFormat::Rgba8Unorm, [64, 64]);
    let texture = |label, format, usage| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: 64,
                height: 64,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        })
    };
    let mask = texture(
        "test coverage",
        wgpu::TextureFormat::R8Unorm,
        wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
    );
    let mut coverage = vec![0u8; 64 * 64];
    for y in 0..64i32 {
        for x in 0..64i32 {
            let r2 = (x - 22).pow(2) + (y - 32).pow(2);
            let ring = (100..=256).contains(&r2);
            let island = (48..=54).contains(&x) && ((12..=20).contains(&y) || (43..=51).contains(&y));
            if ring || island {
                coverage[y as usize * 64 + x as usize] = 255;
            }
        }
    }
    coverage[33 * 64 + 60] = 255;
    queue.write_texture(
        mask.as_image_copy(),
        &coverage,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(64),
            rows_per_image: Some(64),
        },
        wgpu::Extent3d {
            width: 64,
            height: 64,
            depth_or_array_layers: 1,
        },
    );
    overlay.mask = mask.create_view(&Default::default());
    let layout = overlay.spread.get_bind_group_layout(0);
    for (i, second) in [&overlay.mask, &overlay.horizontal].into_iter().enumerate() {
        overlay.groups[i] = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("test selection"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&overlay.mask),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(second),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: overlay.buffer.as_entire_binding(),
                },
            ],
        });
    }
    let output = texture(
        "test material",
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    );
    let view = output.create_view(&Default::default());
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 64 * 64 * 4,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let original = [40u8, 80, 120, 255];
    for (pulse, scale) in [(0.0, 1.0), (0.18, 1.0), (0.0, 1.5), (0.0, 2.0)] {
        overlay.update(&queue, [0.4, 0.1, 0.9, 1.0], pulse, scale);
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("test original material"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: f64::from(original[0]) / 255.0,
                            g: f64::from(original[1]) / 255.0,
                            b: f64::from(original[2]) / 255.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }
        overlay.encode(&mut encoder, &view, None);
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
        readback.map_async(wgpu::MapMode::Read, .., |result| result.expect("readback"));
        device.poll(wgpu::PollType::wait_indefinitely()).expect("GPU completion");
        let bytes = readback.get_mapped_range(..).expect("mapped pixels");
        let pixel = |x: usize, y: usize| &bytes[(y * 64 + x) * 4..(y * 64 + x + 1) * 4];
        assert_eq!(pixel(0, 0), original, "outside the halo is untouched");
        assert_ne!(pixel(59, 33), original, "thin island at any DPI");
        if scale == 1.0 {
            assert_eq!(pixel(22, 32), original, "centre of the ring hole is untouched");
        }
        assert_ne!(pixel(22, 23), original, "halo follows the inner ring silhouette");
        assert_ne!(pixel(22, 15), original, "halo follows the outer ring silhouette");
        assert_ne!(pixel(47, 16), original, "first disconnected island");
        assert_ne!(pixel(47, 47), original, "second disconnected island");
        for (i, &covered) in coverage.iter().enumerate() {
            if covered != 0 {
                if pulse == 0.0 {
                    assert_eq!(&bytes[i * 4..i * 4 + 4], original, "interior {i}");
                } else {
                    assert_ne!(&bytes[i * 4..i * 4 + 4], original, "pulse {i}");
                }
            }
        }
        drop(bytes);
        readback.unmap();
    }
}
