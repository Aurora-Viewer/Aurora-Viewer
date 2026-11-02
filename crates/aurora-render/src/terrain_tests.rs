//! Exercise the production terrain material mixer with real GPU texture samples.
use crate::DrawRecord;
use crate::pipeline_cache::CachedPipelines;
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
    let object = include_str!("shaders/object.wgsl");
    let declaration = |source: &str, name: &str| {
        let start = source.find(&format!("struct {name} {{")).unwrap();
        let end = source[start..].find("};").unwrap() + start + 2;
        source[start..end].to_owned()
    };
    let start = env.find("const TERRAIN_ALPHA_RAMP:").unwrap();
    let end = env.find("fn terrain_color(").unwrap();
    // Direct bindings model the detail and ramp pages here. The production
    // binding-array path is covered by scene_shader_is_valid; sampling is unchanged.
    let terrain_vertex = env[..env.find("fn vs_terrain(").unwrap()].rfind("@vertex").unwrap();
    let tangent = &env[env.find("fn terrain_transform_tangent(").unwrap()..terrain_vertex];
    let functions = format!("{}\n{tangent}\n{}", wgsl_function(object, "terrain_direction"), &env[start..end])
        .replace("tex_pages[loc >> 16u]", "tex_pages")
        .replace("tex_pages[ramp_loc >> 16u]", "alpha_ramp")
        .replace("let ramp_loc = tex_loc[6u];", "let ramp_loc = 0u;");
    let source = format!(
        r#"
{}
{}
@group(0) @binding(0) var<storage, read> records: array<DrawRecord>;
@group(0) @binding(1) var<storage, read> tex_loc: array<u32>;
@group(0) @binding(2) var tex_pages: texture_2d_array<f32>;
@group(0) @binding(3) var tex_sampler: sampler;
@group(0) @binding(4) var<storage, read_write> result: array<vec4<f32>>;
@group(0) @binding(5) var lin_clamp: sampler;
@group(0) @binding(6) var alpha_ramp: texture_2d_array<f32>;
{}
{}
@compute @workgroup_size(1)
fn probe(@builtin(global_invocation_id) id: vec3<u32>) {{
    if (id.x >= 32u) {{
        let noise = array<f32, 4>(0.1, 0.5, 0.9, 0.5)[id.x - 32u];
        let c = select(0.5, 1.5, id.x == 35u);
        result[id.x * 4u] = terrain_noise_weights(c, noise, vec2<f32>(0.0), vec2<f32>(0.0), lin_clamp);
        return;
    }}
    let compositions = array<f32, 8>(0.0, 1.0, 2.0, 3.0, 0.5, 1.5, 2.5, 0.0);
    var input: TerrainPbrOut;
    input.normal = select(vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(1.0, 0.0, 0.0), id.x == 7u);
    input.region_pos = vec3<f32>(f32(id.x) * 3.0, 2.0, 20.0);
    input.comp = compositions[min(id.x, 7u)];
    input.blend_noise = 0.5;
    input.record = 0u;
    if (id.x >= 8u && id.x < 15u) {{ input.record = id.x - 3u; }}
    if (id.x >= 15u && id.x < 23u) {{
        input.normal = terrain_direction(records[id.x - 3u].anim);
        input.comp = 0.0;
        input.record = id.x - 3u;
    }}
    if (id.x >= 23u && id.x < 31u) {{
        input.normal = vec3<f32>(0,0,1);
        input.region_pos = vec3<f32>(0.375, 0.875, 20.0);
        input.comp = 0.0;
        input.record = select(id.x + 5u, id.x + 9u, id.x >= 27u);
    }}
    if (id.x == 31u) {{
        input.normal = vec3<f32>(0,0,1);
        input.comp = 0.0;
        input.record = 12u;
        input.tangent0 = vec4<f32>(0.5,0.25,0.0,1.0);
    }}
    if (id.x != 31u) {{
        let parent = records[input.record];
        let n = normalize(input.normal);
        var t = vec3<f32>(n.z, 0.0, -n.x);
        if (dot(t,t) < 0.000001) {{ t = vec3<f32>(0.0,1.0,0.0); }}
        let raw = vec4<f32>(normalize(t),1.0);
        input.tangent0 = terrain_transform_tangent(records[parent.tex.x], n, raw);
        input.tangent1 = terrain_transform_tangent(records[parent.tex.y], n, raw);
        input.tangent2 = terrain_transform_tangent(records[parent.tex.z], n, raw);
        input.tangent3 = terrain_transform_tangent(records[parent.tex.w], n, raw);
    }}
    if (id.x >= 15u && id.x < 23u) {{
        let parent = records[input.record];
        let raw = vec4<f32>(terrain_direction(vec4<u32>(round(parent.anim_xf * 255.0))), 1.0);
        input.tangent0 = terrain_transform_tangent(records[parent.tex.x], input.normal, raw);
    }}
    let material = terrain_material(input, vec3<f32>(0.01, 0.0, 0.0), vec3<f32>(0.0, 0.01, 0.0), vec2<f32>(0.0), vec2<f32>(0.0), false, lin_clamp);
    result[id.x * 4u] = material.color;
    result[id.x * 4u + 1u] = vec4<f32>(material.orm, material.cutoff);
    result[id.x * 4u + 2u] = vec4<f32>(material.normal, 1.0);
    result[id.x * 4u + 3u] = vec4<f32>(material.emissive, 1.0);
}}
"#,
        declaration(common, "DrawRecord"),
        declaration(env, "TerrainPbrOut"),
        &common[common.find("fn srgb_to_linear(").unwrap()..common.find("fn sample_tex(").unwrap()],
        functions
    );
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("production PBR terrain samples"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device.create_compute_pipeline_cached(&wgpu::ComputePipelineDescriptor {
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
    for i in 0..7 {
        records.push(DrawRecord {
            tex: [1, 2, 3, 4],
            flags: [0, 0, 8 + i, 0],
            mat_uv: [256.0, 256.0, 0.0, 0.0],
            ..Default::default()
        });
    }
    for i in 0..8 {
        let normal = [
            glam::Vec3::Z,
            glam::Vec3::X,
            glam::Vec3::NEG_X,
            glam::Vec3::Y,
            glam::Vec3::NEG_Y,
            glam::Vec3::Z,
            glam::Vec3::new(0.6, -0.2, 0.77).normalize(),
            glam::Vec3::new(-0.4, 0.5, 0.768).normalize(),
        ][i as usize];
        let tangent = glam::Vec3::new(normal.z, 0.0, -normal.x).try_normalize().unwrap_or(glam::Vec3::X);
        let frame = crate::pack_terrain_frame(normal, tangent);
        records.push(DrawRecord {
            tex: [20 + i; 4],
            anim: frame.joints.map(u32::from),
            anim_xf: frame.weights.map(|b| f32::from(b) / 255.0),
            ..Default::default()
        });
    }
    for i in 0..8 {
        records.push(DrawRecord {
            tex: [2, 15, 6, 7],
            uv_st: [if i >= 6 { -0.125 } else { 0.125 }, 0.125, 0.0, 0.0],
            params: [if i == 5 || i == 7 { std::f32::consts::FRAC_PI_2 } else { 0.0 }, 0.0, 1.0, 0.0],
            ..Default::default()
        });
    }
    for i in 0..4 {
        records.push(DrawRecord {
            tex: [32 + i; 4],
            ..Default::default()
        });
    }
    for i in 0..4 {
        records.push(DrawRecord {
            tex: [16, 1, 6, 7],
            uv_st: [if i == 2 { -1.0 } else { 1.0 }, if i == 3 { -1.0 } else { 1.0 }, 0.0, 0.0],
            params: [if i == 1 || i == 3 { std::f32::consts::FRAC_PI_2 } else { 0.0 }, 0.0, 1.0, 0.0],
            ..Default::default()
        });
    }
    for i in 0..4 {
        records.push(DrawRecord {
            tex: [40 + i; 4],
            ..Default::default()
        });
    }
    for i in 0..4 {
        let base = records[32 + i];
        records.push(DrawRecord {
            tex: [2, 17, 6, 7],
            ..base
        });
    }
    let record_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&records),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let locations = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&(0u32..18).collect::<Vec<_>>()),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("terrain material maps"),
        size: wgpu::Extent3d {
            width: 4,
            height: 4,
            depth_or_array_layers: 18,
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
        [0, 0, 0, 255],
        [255, 0, 0, 255],
        [0, 255, 0, 255],
        [0, 0, 255, 255],
        [51, 77, 26, 255],
        [255, 255, 255, 255],
        [5, 0, 0, 255],
        [230, 170, 240, 255],
        [0, 0, 0, 255],
        [128, 128, 192, 255],
    ];
    let mut texels = Vec::new();
    for (i, pixel) in pixels.into_iter().enumerate() {
        let mut layer = Vec::new();
        for y in 0..4 {
            for x in 0..4 {
                layer.push(if i == 16 {
                    [x * 64, y * 64, 0, 255]
                } else if i == 17 {
                    [128 + x * 32, 128 + y * 32, 192, 255]
                } else {
                    pixel
                });
            }
        }
        if i >= 16 {
            // Actual SL asset encode/decode, not a symmetric hand-uploaded
            // texel: JPEG2000 rows arrive top-down, unlike LLImageRaw.
            let encoded =
                aurora_assets::j2k::encode_texture_j2k(4, 4, 4, bytemuck::cast_slice(&layer), true).expect("asymmetric terrain asset");
            let decoded = aurora_assets::j2k::decode_j2k(&encoded, 0).expect("decode terrain asset");
            assert_eq!(decoded.data, bytemuck::cast_slice::<[u8; 4], u8>(&layer));
            texels.extend(decoded.data.as_chunks::<4>().0.iter().copied());
        } else {
            texels.extend(layer);
        }
    }
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: Default::default(),
            aspect: wgpu::TextureAspect::All,
        },
        bytemuck::cast_slice(&texels),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(16),
            rows_per_image: Some(4),
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
    let clamp_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let ramp_view = alpha_ramp_view(&device, &queue);
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 2304,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 2304,
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
            wgpu::BindGroupEntry {
                binding: 5,
                resource: wgpu::BindingResource::Sampler(&clamp_sampler),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: wgpu::BindingResource::TextureView(&ramp_view),
            },
        ],
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(36, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, 2304);
    queue.submit([encoder.finish()]);
    readback.map_async(wgpu::MapMode::Read, .., |r| r.expect("terrain readback"));
    device.poll(wgpu::PollType::wait_indefinitely()).expect("terrain GPU completion");
    let mapped = readback.get_mapped_range(..).expect("terrain samples");
    let samples: &[[f32; 4]] = bytemuck::cast_slice(&mapped);
    // Independent bilinear samples of IMG_ALPHA_GRAD_2D. A 1D ramp, an
    // inverted row convention or nearest filtering cannot satisfy these.
    for (i, expected) in [
        [0.5984283, 0.3976501, 0.0039062, 0.0000154],
        [0.3076125, 0.688_466, 0.0039062, 0.0000154],
        [0.6570212, 0.3390573, 0.0039062, 0.0000154],
        [0.0024221, 0.3064014, 0.688_466, 0.0027105],
    ]
    .into_iter()
    .enumerate()
    {
        for (got, want) in samples[(32 + i) * 4].into_iter().zip(expected) {
            assert!((got - want).abs() < 0.0001, "2D terrain ramp {i}: {got} != {want}");
        }
    }
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
    for i in 0..4 {
        for (got, want) in samples[(i + 8) * 4].into_iter().zip(expected[i]) {
            assert!((got - want).abs() < 0.001, "paintmap slot {i}");
        }
    }
    let other = glam::Vec3::new(51.0, 77.0, 26.0) / 255.0;
    let first = 1.0 - other.element_sum();
    let mixed = [(first + other.z) * 0.8, (other.x + other.z) * 0.6, other.y * 0.4];
    for (got, want) in samples[12 * 4].into_iter().zip(mixed) {
        assert!((got - want).abs() < 0.001, "linear RGB paint weights");
    }
    for (got, want) in samples[13 * 4].into_iter().zip([0.8 / 3.0, 0.4, 0.4 / 3.0]) {
        assert!((got - want).abs() < 0.001, "overflow paint weights normalize");
    }
    assert!(
        (samples[14 * 4][1] - 5.0 / 255.0 * 0.6).abs() < 0.0001,
        "paintmaps keep small weights without the height/noise cutoff"
    );
    let normals = [
        glam::Vec3::Z,
        glam::Vec3::X,
        glam::Vec3::NEG_X,
        glam::Vec3::Y,
        glam::Vec3::NEG_Y,
        glam::Vec3::Z,
        glam::Vec3::new(0.6, -0.2, 0.77).normalize(),
        glam::Vec3::new(-0.4, 0.5, 0.768).normalize(),
    ];
    for (i, n) in normals.into_iter().enumerate() {
        let angle = if i == 5 || i == 7 { std::f32::consts::FRAC_PI_2 } else { 0.0 };
        let expected = legacy_normal(n, angle, if i >= 6 { -1.0 } else { 1.0 });
        let got = glam::Vec3::from_slice(&samples[(15 + i) * 4 + 2]);
        assert!(got.distance(expected) < 0.001, "normal {i}: got {got}, Legacy {expected}");
    }
    let linear = |v: f32| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
    for (i, (x, y)) in [(1.0, 0.0), (0.0, 2.0), (2.0, 0.0), (3.0, 2.0)].into_iter().enumerate() {
        let got = samples[(23 + i) * 4];
        assert!(
            (got[0] - linear(x * 64.0 / 255.0)).abs() < 0.001 && (got[1] - linear(y * 64.0 / 255.0)).abs() < 0.001,
            "terrain KHR rotation/mirror {i}: {got:?}"
        );
    }
    for (i, v) in [
        glam::Vec3::new(65.0, 1.0, 129.0),
        glam::Vec3::new(129.0, -1.0, 129.0),
        glam::Vec3::new(-129.0, 1.0, 129.0),
        glam::Vec3::new(129.0, 193.0, 129.0),
    ]
    .into_iter()
    .enumerate()
    {
        let got = glam::Vec3::from_slice(&samples[(27 + i) * 4 + 2]);
        assert!(got.distance(v.normalize()) < 0.001, "decoded normal map orientation {i}: {got}");
    }
    let expected = glam::Vec3::new(0.5 * 205.0 - 0.25 * 85.0, 0.25 * 205.0 + 0.5 * 85.0, 225.0).normalize();
    assert!(
        glam::Vec3::from_slice(&samples[31 * 4 + 2]).distance(expected) < 0.001,
        "Mikk tangent must remain interpolated"
    );
}

/// pbrterrainUtilF.glsl's _t_normal_post_x/y/z followed by mikktspace;
/// computed independently on the CPU for the GPU's sampled normal texel.
fn legacy_normal(n: glam::Vec3, angle: f32, scale_u: f32) -> glam::Vec3 {
    use glam::Vec3;
    let texel = Vec3::new(230.0, 170.0, 240.0) * (2.0 / 255.0) - Vec3::ONE;
    let mut weights = n.abs().powf(8.0);
    weights /= weights.element_sum();
    weights = (weights - Vec3::splat(0.01)).max(Vec3::ZERO);
    weights /= weights.element_sum();
    let x = texel * Vec3::new(n.x.signum(), n.x.signum(), 1.0);
    let y = texel * Vec3::new(n.y.signum(), n.y.signum(), 1.0);
    let z = texel * Vec3::new(n.z.signum(), n.z.signum(), 1.0);
    let mapped = Vec3::new(-x.y * scale_u, x.x * scale_u, x.z) * weights.x + Vec3::new(-y.x, -y.y, y.z) * weights.y + z * weights.z;
    let tangent = Vec3::new(n.z, 0.0, -n.x).try_normalize().unwrap_or(Vec3::X);
    let t = (tangent * angle.cos() * scale_u - n.cross(tangent) * angle.sin()).normalize();
    (t * mapped.x + scale_u * n.cross(t) * mapped.y + n * mapped.z).normalize()
}

fn wgsl_function<'a>(source: &'a str, name: &str) -> &'a str {
    let start = source.find(&format!("fn {name}(")).expect("production WGSL function");
    let mut depth = 0;
    for (i, ch) in source[start..].char_indices() {
        if ch == '{' {
            depth += 1;
        }
        if ch == '}' {
            depth -= 1;
            if depth == 0 {
                return &source[start..start + i + 1];
            }
        }
    }
    panic!("unterminated WGSL function");
}

#[test]
fn pbr_terrain_shadow_depth_and_paintmap_holes_agree_on_gpu() {
    let _guard = crate::test_gpu::lock();
    let instance = wgpu::Instance::default();
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
        return;
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).expect("terrain masks GPU");
    let common = include_str!("shaders/common.wgsl");
    let env = include_str!("shaders/env.wgsl");
    let object = include_str!("shaders/object.wgsl");
    let structure = |source: &str, name: &str| {
        let start = source.find(&format!("struct {name} {{")).unwrap();
        let end = source[start..].find("};").unwrap() + start + 2;
        source[start..end].to_owned()
    };
    let mut source = String::from(
        r#"
@group(0) @binding(0) var<storage, read> records: array<DrawRecord>;
@group(0) @binding(1) var<storage, read> tex_loc: array<u32>;
@group(0) @binding(2) var tex_pages: texture_2d_array<f32>;
@group(0) @binding(3) var tex_sampler: sampler;
@group(0) @binding(4) var lin_clamp: sampler;
@group(0) @binding(6) var alpha_ramp: texture_2d_array<f32>;
@group(3) @binding(1) var shadow_clamp: sampler;
fn shadow_surface(vi: u32, sample_index: u32) -> TerrainShadowOut {
    let uv = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));
    var o: TerrainShadowOut;
    o.clip = vec4<f32>(uv * 2.0 - vec2<f32>(1.0), 0.25, 1.0);
    o.record = sample_index;
    o.normal = select(vec3<f32>(0,0,1), vec3<f32>(1,0,0), sample_index == 1u);
    o.region_pos = vec3<f32>(uv.x * 8.0, uv.y * 8.0, uv.x * 8.0);
    o.comp = select(0.0, 0.5, sample_index == 2u);
    o.blend_noise = 0.5;
    o.world_pos = o.region_pos;
    return o;
}
@vertex fn shadow_vertex(@builtin(vertex_index) vi: u32, @builtin(instance_index) sample_index: u32) -> TerrainShadowOut {
    return shadow_surface(vi, sample_index);
}
@vertex fn depth_vertex(@builtin(vertex_index) vi: u32, @builtin(instance_index) sample_index: u32) -> TerrainPbrOut {
    let s = shadow_surface(vi, sample_index);
    var o: TerrainPbrOut;
    o.clip = s.clip;
    o.normal = s.normal;
    o.region_pos = s.region_pos;
    o.comp = s.comp;
    o.blend_noise = s.blend_noise;
    o.record = s.record;
    return o;
}
"#,
    );
    for text in [
        structure(common, "DrawRecord"),
        structure(env, "TerrainPbrOut"),
        structure(env, "TerrainShadowOut"),
        common[common.find("const FLAG_FULLBRIGHT:").unwrap()..common.find("@group(0)").unwrap()].to_owned(),
        common[common.find("fn srgb_to_linear(").unwrap()..common.find("fn in_reflection_pass(").unwrap()].to_owned(),
        wgsl_function(env, "terrain_transform_tangent").to_owned(),
        env[env.find("const TERRAIN_ALPHA_RAMP:").unwrap()..env.find("fn terrain_color(").unwrap()].to_owned(),
        wgsl_function(object, "portal_clip").to_owned(),
        wgsl_function(env, "terrain_shadow_mask").to_owned(),
        wgsl_function(env, "terrain_depth_mask").to_owned(),
        format!("@fragment {}", wgsl_function(env, "fs_shadow_terrain")),
        format!("@fragment {}", wgsl_function(env, "fs_terrain_depth")),
    ] {
        source.push_str(&text);
        source.push('\n');
    }
    // The one-page binding substitution leaves both production fragment
    // entry points and their alpha/gradient/material code unchanged.
    let source = source
        .replace("tex_pages[loc >> 16u]", "tex_pages")
        .replace("tex_pages[ramp_loc >> 16u]", "alpha_ramp")
        .replace("let ramp_loc = tex_loc[6u];", "let ramp_loc = 0u;");
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("terrain shadow regression"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let mut records = vec![
        DrawRecord {
            tex: [6, 7, 8, 9],
            flags: [crate::flags::TERRAIN | crate::flags::PBR | crate::flags::ALPHA_MASK, 0, 0, 0],
            mat_uv: [8.0, 8.0, 0.0, 0.0],
            ..Default::default()
        };
        6
    ];
    records[3].flags[2] = 3; // green paintmap selects slot 3, independent of comp
    records[4].flags[0] &= !crate::flags::ALPHA_MASK;
    // (an ordinary masked caster keeps fs_shadow, which never sees terrain)
    for i in 0..4 {
        records.push(DrawRecord {
            tex: [if i < 2 { 1 } else { 2 }, 0, 0, 0],
            uv_st: [0.125, 0.125, 0.0, 0.0],
            params: [0.0, 0.0, 1.0, if i == 1 { 0.8 } else { 0.5 }],
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
        contents: bytemuck::cast_slice(&[0u32, 1, 2, 3]),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: 16,
            height: 16,
            depth_or_array_layers: 4,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let mut pixels = vec![[255u8; 4]; 16 * 16 * 4];
    for y in 0..16 {
        for x in 0..16 {
            pixels[256 + y * 16 + x][3] = if x < 8 { 0 } else { 255 };
            pixels[512 + y * 16 + x][3] = if x < 8 { 255 } else { 0 };
            pixels[768 + y * 16 + x] = [0, 255, 0, 255];
        }
    }
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
            bytes_per_row: Some(64),
            rows_per_image: Some(16),
        },
        texture.size(),
    );
    let texture_view = texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    });
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::Repeat,
        ..Default::default()
    });
    let clamp_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let ramp_view = alpha_ramp_view(&device, &queue);
    let layout_entries = [
        wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        },
        wgpu::BindGroupLayoutEntry {
            binding: 1,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        },
        wgpu::BindGroupLayoutEntry {
            binding: 2,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2Array,
                multisampled: false,
            },
            count: None,
        },
        wgpu::BindGroupLayoutEntry {
            binding: 3,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        },
        wgpu::BindGroupLayoutEntry {
            binding: 6,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2Array,
                multisampled: false,
            },
            count: None,
        },
        wgpu::BindGroupLayoutEntry {
            binding: 4,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        },
    ];
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &layout_entries,
    });
    let group_entries = [
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
            resource: wgpu::BindingResource::TextureView(&texture_view),
        },
        wgpu::BindGroupEntry {
            binding: 3,
            resource: wgpu::BindingResource::Sampler(&sampler),
        },
        wgpu::BindGroupEntry {
            binding: 6,
            resource: wgpu::BindingResource::TextureView(&ramp_view),
        },
        wgpu::BindGroupEntry {
            binding: 4,
            resource: wgpu::BindingResource::Sampler(&clamp_sampler),
        },
    ];
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &layout,
        entries: &group_entries,
    });
    // Omit the scene-only sampler from the shadow pass, as the real backend
    // does. A mistaken dependency on lin_clamp must fail pipeline creation.
    let shadow_resources_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &layout_entries[..5],
    });
    let shadow_resources = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &shadow_resources_layout,
        entries: &group_entries[..5],
    });
    // Exercise the actual shadow layout and bind-group constructor, too:
    // this pass deliberately has no scene frame group to supply samplers.
    let shadow_layout = crate::renderer::shadow_layout(&device);
    let shadow_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 80,
        usage: wgpu::BufferUsages::UNIFORM,
        mapped_at_creation: false,
    });
    let shadow_group = crate::renderer::shadow_bind_group(&device, &shadow_layout, &shadow_buffer, &clamp_sampler);
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[Some(&layout), None, None, Some(&shadow_layout)],
        immediate_size: 0,
    });
    let shadow_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[Some(&shadow_resources_layout), None, None, Some(&shadow_layout)],
        immediate_size: 0,
    });
    let mut depth_maps = Vec::new();
    for (vs, fs) in [("shadow_vertex", "fs_shadow_terrain"), ("depth_vertex", "fs_terrain_depth")] {
        let pipeline = device.create_render_pipeline_cached(&wgpu::RenderPipelineDescriptor {
            label: Some(fs),
            layout: Some(if fs == "fs_shadow_terrain" {
                &shadow_pipeline_layout
            } else {
                &pipeline_layout
            }),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some(vs),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some(fs),
                compilation_options: Default::default(),
                targets: &[],
            }),
            multiview_mask: None,
            cache: None,
        });
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: 64,
                height: 48,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 64 * 48 * 4,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(fs),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, if fs == "fs_shadow_terrain" { &shadow_resources } else { &group }, &[]);
            pass.set_bind_group(3, &shadow_group, &[]);
            for case in 0..5 {
                pass.set_viewport(0.0, (case * 8) as f32, 64.0, 8.0, 0.0, 1.0);
                pass.draw(0..3, case..case + 1);
            }
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target,
                mip_level: 0,
                origin: Default::default(),
                aspect: wgpu::TextureAspect::DepthOnly,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: Some(48),
                },
            },
            target.size(),
        );
        queue.submit([encoder.finish()]);
        readback.map_async(wgpu::MapMode::Read, .., |r| r.expect("mask readback"));
        device.poll(wgpu::PollType::wait_indefinitely()).expect("mask GPU completion");
        let bytes = readback.get_mapped_range(..).expect("mask depth");
        depth_maps.push(bytemuck::cast_slice::<u8, f32>(&bytes).to_vec());
    }
    assert_eq!(
        &depth_maps[0][..64 * 40],
        &depth_maps[1][..64 * 40],
        "sun/projector fs_shadow_terrain must cut exactly the camera depth holes"
    );
    for case in 0..5 {
        let kept = depth_maps[0][case * 512..(case + 1) * 512].iter().filter(|v| **v < 0.5).count();
        assert_eq!(kept, if case == 4 { 512 } else { 256 }, "mask case {case}");
    }
    assert_ne!(
        &depth_maps[0][..512],
        &depth_maps[0][3 * 512..4 * 512],
        "paintmap selects the inverse alpha layer"
    );
}

/// Separate texture sizes model two production bindless pages, without
/// requiring binding-array support on the adapter used by these tests.
fn alpha_ramp_view(device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("reference 2D terrain ramp"),
        size: wgpu::Extent3d {
            width: 256,
            height: 256,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        texture.as_image_copy(),
        crate::textures::TERRAIN_RAMP_PIXELS,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(256),
            rows_per_image: Some(256),
        },
        texture.size(),
    );
    texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    })
}
