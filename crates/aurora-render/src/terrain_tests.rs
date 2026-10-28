//! Exercise the production terrain material mixer with real GPU texture samples.
use crate::DrawRecord;
use wgpu::util::DeviceExt;

#[test]
fn pbr_terrain_samples_all_layers_maps_and_triplanar_normals_on_gpu() {
    let _guard = crate::test_gpu::lock();
    let instance = wgpu::Instance::default();
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
        return;
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).expect("terrain GPU device");
    let common = include_str!("shaders/common.wgsl");
    let env = include_str!("shaders/env.wgsl");
    let declaration = |source: &str, name: &str| {
        let start = source.find(&format!("struct {name} {{")).unwrap();
        let end = source[start..].find("};").unwrap() + start + 2;
        source[start..end].to_owned()
    };
    let start = env.find("const TERRAIN_ALPHA_RAMP:").unwrap();
    let end = env.find("fn terrain_legacy_color").unwrap();
    // One page is sufficient here. The production binding-array path is also
    // covered by scene_shader_is_valid; all UV/mixing/sample code is unchanged.
    let functions = env[start..end].replace("tex_pages[loc >> 16u]", "tex_pages");
    let source = format!(
        r#"
{}
{}
@group(0) @binding(0) var<storage, read> records: array<DrawRecord>;
@group(0) @binding(1) var<storage, read> tex_loc: array<u32>;
@group(0) @binding(2) var tex_pages: texture_2d_array<f32>;
@group(0) @binding(3) var tex_sampler: sampler;
@group(0) @binding(4) var<storage, read_write> result: array<vec4<f32>>;
{}
{}
@compute @workgroup_size(1)
fn probe(@builtin(global_invocation_id) id: vec3<u32>) {{
    let compositions = array<f32, 8>(0.0, 1.0, 2.0, 3.0, 0.5, 1.5, 2.5, 0.0);
    var input: TerrainOut;
    input.normal = select(vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(1.0, 0.0, 0.0), id.x == 7u);
    input.region_pos = vec3<f32>(f32(id.x) * 3.0, 2.0, 20.0);
    input.comp = compositions[id.x];
    input.record = 0u;
    let material = terrain_material(input, vec3<f32>(0.01, 0.0, 0.0), vec3<f32>(0.0, 0.01, 0.0), false);
    result[id.x * 4u] = material.color;
    result[id.x * 4u + 1u] = vec4<f32>(material.orm, material.cutoff);
    result[id.x * 4u + 2u] = vec4<f32>(material.normal, 1.0);
    result[id.x * 4u + 3u] = vec4<f32>(material.emissive, 1.0);
}}
"#,
        declaration(common, "DrawRecord"),
        declaration(env, "TerrainOut"),
        &common[common.find("fn srgb_to_linear(").unwrap()..common.find("fn sample_tex(").unwrap()],
        functions
    );
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("production PBR terrain samples"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("PBR terrain regression"),
        layout: None,
        module: &module,
        entry_point: Some("probe"),
        compilation_options: Default::default(),
        cache: None,
    });
    let mut records = vec![DrawRecord {
        tex: [1, 2, 3, 4],
        ..Default::default()
    }];
    for i in 0..4 {
        records.push(DrawRecord {
            base_color: [0.8, 0.6, 0.4, 1.0],
            emissive: [1.0, 0.5, 0.2, 0.0],
            uv_st: [0.125, 0.125, 0.0, 0.0],
            params: [0.0, 0.1 + i as f32 * 0.25, 0.2 + i as f32 * 0.2, if i == 3 { 0.4 } else { 0.0 }],
            tex: [2 + i, 1, 6, 7],
            ..Default::default()
        });
    }
    let record_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&records),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let locations = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&[0u32, 1, 2, 3, 4, 5, 6, 7]),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("terrain material maps"),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 8,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let pixels = [
        [255u8; 4],
        [128, 128, 255, 255],
        [255, 0, 0, 255],
        [0, 255, 0, 255],
        [0, 0, 255, 255],
        [255, 255, 0, 255],
        [128, 64, 192, 255],
        [64, 128, 255, 255],
    ];
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: Default::default(),
            aspect: wgpu::TextureAspect::All,
        },
        bytemuck::cast_slice(&pixels),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4),
            rows_per_image: Some(1),
        },
        texture.size(),
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    });
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::Repeat,
        ..Default::default()
    });
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 512,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 512,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: record_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: locations.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: output.as_entire_binding(),
            },
        ],
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(8, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, 512);
    queue.submit([encoder.finish()]);
    readback.map_async(wgpu::MapMode::Read, .., |r| r.expect("terrain readback"));
    device.poll(wgpu::PollType::wait_indefinitely()).expect("terrain GPU completion");
    let mapped = readback.get_mapped_range(..).expect("terrain samples");
    let samples: &[[f32; 4]] = bytemuck::cast_slice(&mapped);
    let expected = [
        [0.8, 0.0, 0.0, 1.0],
        [0.0, 0.6, 0.0, 1.0],
        [0.0, 0.0, 0.4, 1.0],
        [0.8, 0.6, 0.0, 1.0],
    ];
    for (i, color) in expected.into_iter().enumerate() {
        for (got, want) in samples[i * 4].into_iter().zip(color) {
            assert!((got - want).abs() < 0.001, "layer {i}: {samples:?}");
        }
        let orm = samples[i * 4 + 1];
        assert!((orm[0] - 128.0 / 255.0).abs() < 0.001);
        assert!((orm[1] - (64.0 / 255.0) * (0.2 + i as f32 * 0.2)).abs() < 0.001);
        assert!((orm[2] - (192.0 / 255.0) * (0.1 + i as f32 * 0.25)).abs() < 0.001);
        assert_eq!(orm[3], if i == 3 { 0.4 } else { 0.0 });
        assert!(samples[i * 4 + 3][2] > 0.19, "emissive map must contribute");
    }
    assert!(samples[16][0] > 0.1 && samples[16][1] > 0.1, "two materials blend in linear space");
    for i in 0..7 {
        assert!(samples[i * 4 + 2][2] > 0.999);
    }
    assert!(
        samples[30][0] > 0.999,
        "steep terrain retains the geometric normal with a flat normal map"
    );
}
