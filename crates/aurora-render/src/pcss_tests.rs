//! In-memory PCSS contracts: contact hardening, world-space scale and atlas isolation.
use crate::pipeline_cache::CachedPipelines;

#[test]
#[ignore = "requires a GPU"]
fn pcss_penumbra_grows_in_metres_and_stays_inside_its_cascade() {
    let _gpu = crate::test_gpu::lock();
    let instance = wgpu::Instance::default();
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
        return;
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("test device");
    let format = wgpu::TextureFormat::Depth32Float;
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("PCSS synthetic atlas"),
        size: wgpu::Extent3d {
            width: 512,
            height: 512,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let depth = texture.create_view(&Default::default());
    // Three maps of the same one-metre blocker at different cascade scales.
    // The fourth tile is deliberately all near-depth: any atlas bleed is obvious.
    let fixture = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("PCSS fixture"),
        source: wgpu::ShaderSource::Wgsl(
            r#"
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}
@fragment fn fs(@builtin(position) p: vec4<f32>) -> @builtin(frag_depth) f32 {
    let tile = vec2<u32>(p.xy) / 256u;
    let k = tile.x + tile.y * 2u;
    if (k == 3u) { return 0.0; }
    let radius = f32(1u << k);
    let local = p.xy / 256.0 - vec2<f32>(tile);
    let half_width = 0.25 / radius;
    let inside = all(abs(local - 0.5) < vec2<f32>(half_width));
    return select(1.0, 0.4, inside);
}
"#
            .into(),
        ),
    });
    let raster = device.create_render_pipeline_cached(&wgpu::RenderPipelineDescriptor {
        label: Some("PCSS fixture"),
        layout: None,
        vertex: wgpu::VertexState {
            module: &fixture,
            entry_point: Some("vs"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: Default::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Always),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &fixture,
            entry_point: Some("fs"),
            compilation_options: Default::default(),
            targets: &[],
        }),
        multiview_mask: None,
        cache: None,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("PCSS fixture"),
            color_attachments: &[],
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
        pass.set_pipeline(&raster);
        pass.draw(0..3, 0..1);
    }
    queue.submit([encoder.finish()]);
    let body = crate::COMMON_SHADER
        .split("// PCSS BEGIN")
        .nth(1)
        .expect("PCSS start")
        .split("// PCSS END")
        .next()
        .expect("PCSS body");
    let source = format!(
        r#"
@group(0) @binding(0) var shadow_map: texture_depth_2d;
@group(0) @binding(1) var shadow_sampler: sampler_comparison;
{body}
const TEST_CURVATURE: f32 = 0.0;
struct Query {{ a: vec4<f32>, b: vec4<f32> }};
@group(0) @binding(2) var<storage, read> queries: array<Query>;
@group(0) @binding(3) var<storage, read_write> results: array<vec4<f32>>;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {{
    if (id.x >= arrayLength(&queries)) {{ return; }}
    let q = queries[id.x];
    let texel = 1.0 / 256.0;
    let bias = q.b.w * 0.001;
    let span = 1.5 * texel;
    let minimum_radius = 2.0 * q.a.w * span * sqrt(5.0 / 9.0);
    let soft = pcss_shadow(q.a.xy, q.b.xy, texel, q.a.w, q.a.z, bias,
        vec2<f32>(q.b.z, 0.0), q.b.w, TEST_CURVATURE, minimum_radius);
    let fallback = directional_pcf(q.a.xy, q.b.xy, texel, q.a.z, bias, span, 4u);
    let curvature = receiver_curvature(vec3<f32>(q.a.w, 0.0, 0.0), vec3<f32>(0.6 * q.a.w, 0.2 * q.a.w, 0.0),
        vec3<f32>(q.a.w * q.b.z, 0.0, 0.0), vec3<f32>(0.6 * q.a.w * q.b.z, 0.0, 0.0));
    results[id.x] = vec4<f32>(mix(fallback, max(soft.x, 0.0), soft.y), soft.x,
        receiver_plane_confidence(curvature, PCSS_MAX_RADIUS, q.b.w), curvature);
}}
"#
    );
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("PCSS contract"),
        source: wgpu::ShaderSource::Wgsl(source.clone().into()),
    });
    let pipeline = device.create_compute_pipeline_cached(&wgpu::ComputePipelineDescriptor {
        label: Some("PCSS contract"),
        layout: None,
        module: &module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    // A tangent plane is valid on a sloping flat receiver, but extrapolating
    // it across a face-sized curved surface creates false triangular blockers.
    // Camera zoom / active resolution scale both derivative pairs together;
    // the sheared cylindrical probe also checks anisotropic camera footprints.
    for scale in [0.0001, 0.01, 1.0, 10.0] {
        let probes: Vec<[f32; 8]> = [0.0, 0.01, 0.5, 5.0, 10.0]
            .into_iter()
            .map(|curvature| [0.25, 0.25, 0.4, scale, 0.0, 0.0, curvature, 0.005])
            .collect();
        let results = run_queries(&device, &queue, &pipeline, &depth, &probes);
        for (i, result) in results.iter().enumerate() {
            let curvature = probes[i][6];
            assert!(
                (result[3] - curvature).abs() < 0.0001,
                "curvature must not change with magnification: {results:?}"
            );
            if i < 2 {
                assert!(result[2] > 0.999, "flat/gently curved receivers retain plane correction");
            } else {
                assert!(result[2] < 0.001, "strongly curved receivers reject plane extrapolation");
            }
        }
    }
    let mut queries = Vec::<[f32; 8]>::new();
    for scale in [0.01, 0.001] {
        for cascade in 0..3u32 {
            let radius = (1u32 << cascade) as f32;
            for separation in [0.05, 3.0, 8.0] {
                for i in 0..81 {
                    let world_x = 0.3 + i as f32 * 0.005;
                    queries.push([
                        0.5 + world_x / (2.0 * radius),
                        0.5,
                        0.4 + separation * scale,
                        radius,
                        (cascade % 2) as f32,
                        (cascade / 2) as f32,
                        0.0,
                        scale,
                    ]);
                }
            }
        }
    }
    let profiles = queries.len();
    for cascade in 0..3u32 {
        for uv in [[0.001, 0.5], [0.999, 0.5], [0.5, 0.001], [0.5, 0.999]] {
            queries.push([
                uv[0],
                uv[1],
                0.48,
                (1u32 << cascade) as f32,
                (cascade % 2) as f32,
                (cascade / 2) as f32,
                0.0,
                0.01,
            ]);
        }
    }
    let values: Vec<f32> = run_queries(&device, &queue, &pipeline, &depth, &queries)
        .iter()
        .map(|r| r[0])
        .collect();
    assert!(values.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)));
    for scale in 0..2 {
        for cascade in 0..3 {
            let mut widths = Vec::new();
            for separation in 0..3 {
                let at = ((scale * 3 + cascade) * 3 + separation) * 81;
                let row = &values[at..at + 81];
                assert!(row[0] < 0.001 && row[80] > 0.999, "uniform umbra and lit regions");
                assert!(row.windows(2).all(|p| p[1] + 1e-4 >= p[0]), "monotone shadow edge");
                widths.push(row.iter().filter(|v| **v > 0.05 && **v < 0.95).count());
            }
            assert!(widths[1] > widths[0] && widths[2] > widths[1] * 2, "contact hardening: {widths:?}");
        }
    }
    for cascade in 0..3 {
        for i in 0..243 {
            assert!(
                (values[cascade * 243 + i] - values[729 + cascade * 243 + i]).abs() < 0.002,
                "arbitrary light depth padding must not change softness"
            );
        }
    }
    for cascade in 1..3 {
        for i in 0..81 {
            assert!(
                (values[162 + i] - values[cascade * 243 + 162 + i]).abs() < 0.12,
                "same world-space penumbra across cascades"
            );
        }
    }
    assert!(
        values[profiles..].iter().all(|v| *v > 0.999),
        "a neighbouring atlas tile must not shadow these receivers"
    );

    // An infinitesimal blocker-distance change across the small/wide
    // filter threshold must not visibly change shadow coverage.
    let transition_source = source.replace("const PCSS_AA_RADIUS: f32 = 0.01;", "const PCSS_AA_RADIUS: f32 = 0.0;");
    let transition_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("PCSS kernel transition"),
        source: wgpu::ShaderSource::Wgsl(transition_source.into()),
    });
    let transition_pipeline = device.create_compute_pipeline_cached(&wgpu::ComputePipelineDescriptor {
        label: Some("PCSS kernel transition"),
        layout: None,
        module: &transition_module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let mut transition_queries = Vec::new();
    for u in [0.746_f32, 0.747, 0.748, 0.749, 0.750, 0.751, 0.752, 0.753, 0.754] {
        for gap in [0.436732_f32 - 0.0001, 0.436732 + 0.0001] {
            transition_queries.push([u, 0.5, 0.4 + 0.01 * gap, 1.0, 0.0, 0.0, 0.0, 0.01]);
        }
    }
    let transition = run_queries(&device, &queue, &transition_pipeline, &depth, &transition_queries);
    assert!(
        transition
            .as_chunks::<2>()
            .0
            .iter()
            .all(|pair| (pair[0][0] - pair[1][0]).abs() < 0.005),
        "small/wide PCSS transition must be continuous: {transition:?}"
    );

    // A sloping, unoccluded receiver must not become its own blocker as
    // sub-texel sample positions change. The point depth is at a texel centre.
    let plane = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("PCSS receiver plane"),
        source: wgpu::ShaderSource::Wgsl(
            r#"
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}
@fragment fn fs(@builtin(position) p: vec4<f32>) -> @builtin(frag_depth) f32 {
    return 0.4 + 0.06 * (fract(p.x / 256.0) - 0.5) + 2.0 * 0.06 / 256.0;
}

"#
            .into(),
        ),
    });
    let make_plane_raster = |entry| {
        device.create_render_pipeline_cached(&wgpu::RenderPipelineDescriptor {
            label: Some("PCSS fixture"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &plane,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &plane,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                targets: &[],
            }),
            multiview_mask: None,
            cache: None,
        })
    };
    let plane_raster = make_plane_raster("fs");
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("PCSS fixture"),
            color_attachments: &[],
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
        pass.set_pipeline(&plane_raster);
        pass.draw(0..3, 0..1);
    }
    queue.submit([encoder.finish()]);
    let mut tilted = Vec::new();
    for cascade in 0..3u32 {
        for u in [0.1123, 0.3751, 0.54321, 0.8766] {
            for v in [0.2, 0.5, 0.8] {
                tilted.push([
                    u,
                    v,
                    0.4 + 0.06 * (u - 0.5),
                    (1u32 << cascade) as f32,
                    (cascade % 2) as f32,
                    (cascade / 2) as f32,
                    0.06,
                    0.01,
                ]);
            }
        }
    }
    let samples = run_queries(&device, &queue, &pipeline, &depth, &tilted);
    assert!(
        samples.iter().all(|s| s[0] > 0.999),
        "bilinear filtering must not shadow an unoccluded receiver plane: {samples:?}"
    );
    assert!(
        samples.iter().all(|s| s[1] < 0.0),
        "a receiver plane must not produce false blockers: {samples:?}"
    );

    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("PCSS near blocker"),
            color_attachments: &[],
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
        pass.set_pipeline(&raster);
        pass.draw(0..3, 0..1);
    }
    queue.submit([encoder.finish()]);
    // Close blockers must preserve the ordinary filter, even when receiver
    // normals or curvature change. Vary the irrelevant curvature field over
    // sub-texel positions and require the same mask as the baseline.
    let mut contact_probes = Vec::new();
    for phase in 0..257 {
        let u = 0.746 + phase as f32 / (256.0 * 256.0);
        contact_probes.push([u, 0.5, 0.4005, 1.0, 0.0, 0.0, 0.0, 0.01]);
    }
    let contact_source = source
        .replace("const TEST_CURVATURE: f32 = 0.0;", "const TEST_CURVATURE: f32 = 1000000.0;")
        .replace("let bias = q.b.w * 0.001;", "let bias = 4.0 * q.a.w * texel * q.b.w;")
        .replace(
            "receiver_plane_confidence(curvature, PCSS_MAX_RADIUS, q.b.w), curvature",
            "fallback, curvature",
        );
    let contact_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("PCSS contact filter parity"),
        source: wgpu::ShaderSource::Wgsl(contact_source.into()),
    });
    let contact_pipeline = device.create_compute_pipeline_cached(&wgpu::ComputePipelineDescriptor {
        label: Some("PCSS contact filter parity"),
        layout: None,
        module: &contact_module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let contact = run_queries(&device, &queue, &contact_pipeline, &depth, &contact_probes);
    let low = contact.iter().map(|s| s[2]).fold(1.0_f32, f32::min);
    let high = contact.iter().map(|s| s[2]).fold(0.0_f32, f32::max);
    assert!(
        high - low > 0.1,
        "the parity fixture must cross a shadow edge, not a uniformly lit region"
    );
    assert!(
        contact.iter().all(|s| (s[0] - s[2]).abs() < 1e-6),
        "sub-resolution PCSS must match the ordinary shadow mask through motion: {contact:?}"
    );
}

/// Projectors must use linear blocker distances, not perspective depth ratios.
#[test]
#[ignore = "requires a GPU"]
fn projector_pcss_hardens_at_contact_and_isolates_atlas_tiles() {
    let _gpu = crate::test_gpu::lock();
    let instance = wgpu::Instance::default();
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
        return;
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("test device");
    let format = wgpu::TextureFormat::Depth32Float;
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("PCSS synthetic atlas"),
        size: wgpu::Extent3d {
            width: 512,
            height: 512,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let depth = texture.create_view(&Default::default());
    let fixture = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("PCSS fixture"),
        source: wgpu::ShaderSource::Wgsl(
            r#"
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}
@fragment fn fs(@builtin(position) p: vec4<f32>) -> @builtin(frag_depth) f32 {
    // Two projector maps, with different near planes and hostile neighbours.
    if (p.x < 256.0 || p.y < 256.0 || p.y >= 384.0) { return 0.0; }
    let k = u32(p.x - 256.0) / 128u;
    let local = (p.xy - vec2<f32>(256.0 + f32(k) * 128.0, 256.0)) / 128.0;
    let near = select(0.05, 0.5, k == 1u);
    let a = 20.0 / (20.0 - near);
    let b = -near * a;
    return select(1.0, a + b / 2.0, local.x < 0.5 && abs(local.y - 0.5) < 0.4);
}
"#
            .into(),
        ),
    });
    let raster = device.create_render_pipeline_cached(&wgpu::RenderPipelineDescriptor {
        label: Some("PCSS fixture"),
        layout: None,
        vertex: wgpu::VertexState {
            module: &fixture,
            entry_point: Some("vs"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: Default::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Always),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &fixture,
            entry_point: Some("fs"),
            compilation_options: Default::default(),
            targets: &[],
        }),
        multiview_mask: None,
        cache: None,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("PCSS fixture"),
            color_attachments: &[],
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
        pass.set_pipeline(&raster);
        pass.draw(0..3, 0..1);
    }
    queue.submit([encoder.finish()]);

    let body = crate::COMMON_SHADER
        .split("// PCSS BEGIN")
        .nth(1)
        .expect("PCSS start")
        .split("// PCSS END")
        .next()
        .expect("PCSS end");
    let source = format!(
        r#"
@group(0) @binding(0) var shadow_map: texture_depth_2d;
@group(0) @binding(1) var shadow_sampler: sampler_comparison;
{body}
struct Query {{ a: vec4<f32>, b: vec4<f32> }};
@group(0) @binding(2) var<storage, read> queries: array<Query>;
@group(0) @binding(3) var<storage, read_write> results: array<vec4<f32>>;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {{
    if (id.x >= arrayLength(&queries)) {{ return; }}
    let q = queries[id.x];
    let texel = 1.0 / 128.0;
    let gradient = vec2<f32>(0.0);
    let soft = spot_pcss(q.a.xy, q.b.xy, texel, q.a.z, gradient, q.a.w,
        q.b.zw, vec2<f32>(4.0));
    let fallback = spot_pcss_compare(q.a.xy, vec2<f32>(0.0), q.b.xy, texel, q.a.z, gradient);
    results[id.x] = vec4<f32>(select(soft, fallback, soft < 0.0), soft, 0.0, 0.0);
}}
"#
    );
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("projector PCSS contract"),
        source: wgpu::ShaderSource::Wgsl(source.clone().into()),
    });
    let pipeline = device.create_compute_pipeline_cached(&wgpu::ComputePipelineDescriptor {
        label: Some("projector PCSS contract"),
        layout: None,
        module: &module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let mut queries = Vec::<[f32; 8]>::new();
    for (k, near) in [0.05f32, 0.5].into_iter().enumerate() {
        let a = 20.0 / (20.0 - near);
        let b = -near * a;
        for z in [2.05, 4.0, 8.0] {
            for i in 0..241 {
                queries.push([0.44 + i as f32 * 0.0005, 0.5, a + b / z, z, 0.5 + k as f32 * 0.25, 0.5, a, b]);
            }
        }
    }
    let profiles = queries.len();
    for k in 0..2 {
        let near = if k == 0 { 0.05 } else { 0.5 };
        let a = 20.0 / (20.0 - near);
        let b = -near * a;
        for uv in [[0.999, 0.5], [0.8, 0.001], [0.8, 0.999]] {
            queries.push([uv[0], uv[1], a + b / 8.0, 8.0, 0.5 + k as f32 * 0.25, 0.5, a, b]);
        }
    }
    let results = run_queries(&device, &queue, &pipeline, &depth, &queries);
    assert!(results.iter().all(|r| r[0].is_finite() && (0.0..=1.0).contains(&r[0])));
    for k in 0..2 {
        let widths: Vec<_> = (0..3)
            .map(|step| {
                let at = (k * 3 + step) * 241;
                let row = &results[at..at + 241];
                assert!(row[0][0] < 0.01 && row[240][0] > 0.99, "umbra and lit surface survive");
                assert!(row.windows(2).all(|w| w[1][0] + 0.002 >= w[0][0]), "monotone edge");
                row.iter().filter(|r| r[0] > 0.1 && r[0] < 0.9).count()
            })
            .collect();
        assert!(widths[1] > widths[0] * 2 && widths[2] > widths[1], "contact hardening: {widths:?}");
    }
    for i in 0..3 * 241 {
        assert!(
            (results[i][0] - results[i + 3 * 241][0]).abs() < 0.003,
            "near plane must not change physical softness: {i}"
        );
    }
    assert!(results[profiles..].iter().all(|r| r[0] > 0.999), "no neighbouring-tile shadows");
    let plane = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("PCSS receiver plane"),
        source: wgpu::ShaderSource::Wgsl(
            r#"
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}
@fragment fn fs(@builtin(position) p: vec4<f32>) -> @builtin(frag_depth) f32 {
    return 0.4 + 0.06 * (fract(p.x / 128.0) - 0.5);
}
"#
            .into(),
        ),
    });
    let plane_raster = device.create_render_pipeline_cached(&wgpu::RenderPipelineDescriptor {
        label: Some("PCSS fixture"),
        layout: None,
        vertex: wgpu::VertexState {
            module: &plane,
            entry_point: Some("vs"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: Default::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Always),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &plane,
            entry_point: Some("fs"),
            compilation_options: Default::default(),
            targets: &[],
        }),
        multiview_mask: None,
        cache: None,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("PCSS fixture"),
            color_attachments: &[],
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
        pass.set_pipeline(&plane_raster);
        pass.draw(0..3, 0..1);
    }
    queue.submit([encoder.finish()]);
    let tilted_source = source.replace("let gradient = vec2<f32>(0.0);", "let gradient = vec2<f32>(0.06, 0.0);");
    let tilted_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("tilted projector PCSS"),
        source: wgpu::ShaderSource::Wgsl(tilted_source.into()),
    });
    let tilted_pipeline = device.create_compute_pipeline_cached(&wgpu::ComputePipelineDescriptor {
        label: Some("tilted projector PCSS"),
        layout: None,
        module: &tilted_module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let mut tilted = Vec::new();
    for k in 0..2 {
        let a = 1.025;
        let b = -0.05 * a;
        for u in [0.1123, 0.3751, 0.54321, 0.8766] {
            let receiver = 0.4 + 0.06 * (u - 0.5);
            let z = b / (receiver - a);
            tilted.push([u, 0.5, receiver, z, 0.5 + k as f32 * 0.25, 0.5, a, b]);
        }
    }
    let samples = run_queries(&device, &queue, &tilted_pipeline, &depth, &tilted);
    assert!(
        samples.iter().all(|s| s[1] < 0.0 && s[0] > 0.999),
        "projector plane must not shadow itself: {samples:?}"
    );
}

fn run_queries(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pipeline: &wgpu::ComputePipeline,
    depth: &wgpu::TextureView,
    queries: &[[f32; 8]],
) -> Vec<[f32; 4]> {
    let input = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("PCSS queries"),
        size: (queries.len() * 32) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&input, 0, bytemuck::cast_slice(queries));
    let bytes = (queries.len() * 16) as u64;
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("PCSS results"),
        size: bytes,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("PCSS readback"),
        size: bytes,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("PCSS compare"),
        compare: Some(wgpu::CompareFunction::LessEqual),
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let layout = pipeline.get_bind_group_layout(0);
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("PCSS queries"),
        layout: &layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(depth),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: input.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: output.as_entire_binding(),
            },
        ],
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups((queries.len() as u32).div_ceil(64), 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, bytes);
    queue.submit([encoder.finish()]);
    readback.slice(..).map_async(wgpu::MapMode::Read, |r| r.expect("map"));
    device.poll(wgpu::PollType::wait_indefinitely()).expect("GPU test wait");
    let data = readback.slice(..).get_mapped_range().expect("mapped results");
    data.as_chunks::<16>()
        .0
        .iter()
        .map(|bytes| std::array::from_fn(|i| f32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().expect("float"))))
        .collect()
}

#[test]
#[ignore = "requires a GPU"]
fn pcss_receiver_fit_preserves_signed_curvature_and_flat_surfaces() {
    let _gpu = crate::test_gpu::lock();
    let instance = wgpu::Instance::default();
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
        return;
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("test device");
    let smooth_body = crate::COMMON_SHADER
        .split("// SMOOTH RECEIVER BEGIN")
        .nth(1)
        .expect("smooth receiver start")
        .split("// SMOOTH RECEIVER END")
        .next()
        .expect("smooth receiver end");
    let source = format!(
        r#"
const SMOOTH_RECEIVER_MAX_OFFSET: f32 = 0.05;
{smooth_body}
@group(0) @binding(0) var shadow_map: texture_depth_2d;
@group(0) @binding(1) var shadow_sampler: sampler_comparison;
struct Query {{ a: vec4<f32>, b: vec4<f32> }};
@group(0) @binding(2) var<storage, read> queries: array<Query>;
@group(0) @binding(3) var<storage, read_write> results: array<vec4<f32>>;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {{
    if (id.x >= arrayLength(&queries)) {{ return; }}
    let q = queries[id.x];
    var factor = 1.0;
    var offset = vec3<f32>(0.0);
    offset = smooth_receiver_offset(q.a.xyz, q.b.xyz, q.a.w);
    results[id.x] = vec4<f32>(factor, textureSampleCompareLevel(shadow_map, shadow_sampler, vec2<f32>(0.5), 0.0), offset.z, length(offset.xy));
}}
"#
    );
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("smooth receiver contract"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device.create_compute_pipeline_cached(&wgpu::ComputePipelineDescriptor {
        label: Some("smooth receiver contract"),
        layout: None,
        module: &module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("receiver test map"),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let depth = texture.create_view(&Default::default());
    let mut queries = Vec::new();
    let mut expected = Vec::new();
    let mut expected_offsets = vec![[0.0_f32; 2]; queries.len()];
    let mut probe = |raw: glam::DVec3, position: glam::DVec3, moment: f64, offset: glam::DVec3| {
        queries.push([
            raw.x as f32,
            raw.y as f32,
            raw.z as f32,
            moment as f32,
            position.x as f32,
            position.y as f32,
            position.z as f32,
            1.0,
        ]);
        expected.push(1.0);
        expected_offsets.push([offset.z as f32, offset.truncate().length() as f32]);
    };
    // Analytic signed sphere/cylinder sag, independent of eye translation
    // and orientation; vertices remain fixed and large corrections are capped.
    for radius in [-20.0_f64, -1.0, -0.2, 0.05, 0.2, 1.0, 20.0] {
        for size in [0.8_f64, 0.95, 1.0] {
            for angle in [0.0_f64, 0.5, 1.2] {
                let axis = glam::DVec3::new(angle.sin(), 0.0, angle.cos());
                let raw = axis * size;
                for eye_shift in [
                    glam::DVec3::ZERO,
                    glam::DVec3::new(100.0, -50.0, 20.0),
                    glam::DVec3::new(-200.0, 120.0, -70.0),
                ] {
                    let position = axis * (radius * size) + eye_shift;
                    let moment = radius + eye_shift.dot(raw);
                    let offset = axis * (radius * (1.0 - size)).clamp(-0.05, 0.05);
                    probe(raw, position, moment, offset);
                }
            }
        }
    }
    // Flat faces and folded normals must not invent surface displacement.
    probe(glam::DVec3::Z, glam::DVec3::new(100.0, 40.0, 20.0), 20.0, glam::DVec3::ZERO);
    probe(glam::DVec3::Z * 0.1, glam::DVec3::ZERO, 1.0, glam::DVec3::ZERO);
    // Invalid geometric metadata must not introduce a non-finite shadow query.
    probe(glam::DVec3::splat(f64::NAN), glam::DVec3::ZERO, 0.0, glam::DVec3::ZERO);
    probe(glam::DVec3::splat(f64::INFINITY), glam::DVec3::ZERO, 0.0, glam::DVec3::ZERO);
    probe(glam::DVec3::Z * 0.95, glam::DVec3::ZERO, f64::NAN, glam::DVec3::ZERO);
    // Symmetric small quadratic patches: positive, negative and mixed
    // curvatures recover the smooth centre to second order, including saddles.
    for (kx, ky) in [(5.0_f64, 5.0_f64), (-5.0, -5.0), (5.0, -2.0), (5.0, -5.0)] {
        let mut position = glam::DVec3::ZERO;
        let mut raw = glam::DVec3::ZERO;
        let mut moment = 0.0;
        for (x, y) in [(0.005_f64, 0.0), (-0.005, 0.0), (0.0, 0.005), (0.0, -0.005)] {
            let p = glam::DVec3::new(x, y, 0.5 * (kx * x * x + ky * y * y));
            let n = glam::DVec3::new(-kx * x, -ky * y, 1.0).normalize();
            position += p * 0.25;
            raw += n * 0.25;
            moment += p.dot(n) * 0.25;
        }
        probe(raw, position, moment, -position);
    }
    let result = run_queries(&device, &queue, &pipeline, &depth, &queries);
    for (i, (got, reference)) in result.iter().zip(expected).enumerate() {
        assert!(
            got[0].is_finite() && (0.0..=1.0).contains(&got[0]),
            "finite bounded response at {i}: {got:?}"
        );
        assert!(
            (got[2] - expected_offsets[i][0]).abs() < 2e-5 && (got[3] - expected_offsets[i][1]).abs() < 2e-5,
            "analytic smooth receiver at {i}: {got:?}, expected {:?}",
            expected_offsets[i]
        );
        // The exact horizon's f32 cosine may land either side of zero.
        let q = queries[i];
        if q[7] < 0.5 && q[1].cos().abs() > 1e-6 && (q[0] - q[1]).cos().abs() > 1e-6 {
            assert!(
                (f64::from(got[0]) - reference).abs() < 2e-5,
                "paper reference at {i}: {} vs {reference}",
                got[0]
            );
        }
        if q[0] == 0.0 {
            assert_eq!(got[0], 1.0, "no normal perturbation must preserve the original response");
        }
    }
}
