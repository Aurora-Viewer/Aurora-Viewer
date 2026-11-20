//! GPU checks of the ray query features the ray-traced shadows build on
//! (task `rt-shadows`, step 0), on every backend of this PC that has ray
//! queries: the TLAS instance flags (facing cull off for double-sided faces,
//! flipped facing for mirrored instances, forced opacity) and the candidate
//! loop of a non-opaque BLAS, where an alpha-masked face accepts or refuses
//! a hit. Ignored by default, like the crate's other GPU tests: run with
//! `cargo test --workspace rt_flags -- --ignored --nocapture`.
//!
//! The scene: one triangle, counter-clockwise seen from +z, instanced eight
//! times along x, each instance with its own mask bit so that a ray can
//! only meet the one it aims at. Rays come from +z ("front") or -z ("back").

use wgpu::util::DeviceExt;

const SHADER: &str = r#"
enable wgpu_ray_query;

struct Case {
    origin: vec4<f32>,
    dir: vec4<f32>,
    // x: ray flags, y: cull mask, z: 1 = alpha test in the candidate loop
    // (accept the hit only where the first barycentric is over 0.5)
    params: vec4<u32>,
}

@group(0) @binding(0) var tlas: acceleration_structure;
@group(0) @binding(1) var<storage, read> cases: array<Case>;
// x: hit, y: front face, z: candidates seen, w: instance custom data
@group(0) @binding(2) var<storage, read_write> results: array<vec4<u32>>;

@compute @workgroup_size(1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let c = cases[id.x];
    var rq: ray_query;
    rayQueryInitialize(&rq, tlas, RayDesc(c.params.x, c.params.y, 0.001, 100.0, c.origin.xyz, c.dir.xyz));
    var candidates = 0u;
    while (rayQueryProceed(&rq)) {
        let cand = rayQueryGetCandidateIntersection(&rq);
        candidates += 1u;
        if (c.params.z == 0u || cand.barycentrics.x > 0.5) {
            rayQueryConfirmIntersection(&rq);
        }
    }
    let hit = rayQueryGetCommittedIntersection(&rq);
    var out = vec4<u32>(0u, 0u, candidates, 0u);
    if (hit.kind != RAY_QUERY_INTERSECTION_NONE) {
        out = vec4<u32>(1u, select(0u, 1u, hit.front_face), candidates, hit.instance_custom_data);
    }
    results[id.x] = out;
}
"#;

// RayDesc flags (naga's RAY_FLAG_*).
const NONE: u32 = 0;
const CULL_BACK: u32 = 0x10;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Case {
    origin: [f32; 4],
    dir: [f32; 4],
    params: [u32; 4],
}

/// What one ray met.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Seen {
    hit: bool,
    front: bool,
    candidates: u32,
    custom: u32,
}

/// The instance spacing along x.
const STEP: f32 = 4.0;

/// Barycentrics of the triangle (-1,-1), (1,-1), (0,1): (x, y) = (0.5, -0.8)
/// has u = 0.7 (the alpha test accepts it), (-0.5, -0.8) has u = 0.2 (it
/// refuses it).
const ACCEPTED: (f32, f32) = (0.5, -0.8);
const REFUSED: (f32, f32) = (-0.5, -0.8);

fn case(instance: u32, front: bool, flags: u32, at: (f32, f32), alpha: bool) -> Case {
    let z = if front { 5.0 } else { -5.0 };
    Case {
        origin: [instance as f32 * STEP + at.0, at.1, z, 0.0],
        dir: [0.0, 0.0, -z.signum(), 0.0],
        params: [flags, 1 << instance, alpha as u32, 0],
    }
}

/// A device with ray queries on `backend`, or why there is none.
fn device(backend: wgpu::Backends) -> Result<(wgpu::Device, wgpu::Queue, String), String> {
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
    desc.backends = backend;
    let instance = wgpu::Instance::new(desc);
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).map_err(|e| format!("no adapter: {e}"))?;
    let info = adapter.get_info();
    let name = format!("{:?} {} ({})", info.backend, info.name, info.driver_info);
    if !adapter.features().contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY) {
        return Err(format!("{name}: no ray query"));
    }
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("rt flags test"),
        required_features: wgpu::Features::EXPERIMENTAL_RAY_QUERY,
        required_limits: wgpu::Limits::default().using_minimum_supported_acceleration_structure_values(),
        // SAFETY: a test of the experimental ray query API itself; the
        // device lives only for this test.
        experimental_features: unsafe { wgpu::ExperimentalFeatures::enabled() },
        ..Default::default()
    }))
    .map_err(|e| format!("{name}: {e}"))?;
    Ok((device, queue, name))
}

struct Scene {
    _vertices: wgpu::Buffer,
    _opaque: wgpu::Blas,
    _masked: wgpu::Blas,
    tlas: wgpu::Tlas,
}

/// The triangle as an opaque BLAS and as a non-opaque one, and a TLAS of
/// the eight instances (`instances`: BLAS non-opaque?, mirrored?, flags).
fn scene(device: &wgpu::Device, queue: &wgpu::Queue, instances: &[(bool, bool, wgpu::TlasInstanceFlags)]) -> Scene {
    let positions: [[f32; 3]; 3] = [[-1.0, -1.0, 0.0], [1.0, -1.0, 0.0], [0.0, 1.0, 0.0]];
    let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("rt flags triangle"),
        contents: bytemuck::cast_slice(&positions),
        usage: wgpu::BufferUsages::BLAS_INPUT,
    });
    let size = |flags| wgpu::BlasTriangleGeometrySizeDescriptor {
        vertex_format: wgpu::VertexFormat::Float32x3,
        vertex_count: 3,
        index_format: None,
        index_count: None,
        flags,
    };
    let opaque_size = size(wgpu::AccelerationStructureGeometryFlags::OPAQUE);
    let masked_size = size(wgpu::AccelerationStructureGeometryFlags::empty());
    let blas = |label, size: &wgpu::BlasTriangleGeometrySizeDescriptor| {
        device.create_blas(
            &wgpu::CreateBlasDescriptor {
                label: Some(label),
                flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
                update_mode: wgpu::AccelerationStructureUpdateMode::Build,
            },
            wgpu::BlasGeometrySizeDescriptors::Triangles {
                descriptors: vec![size.clone()],
            },
        )
    };
    let opaque = blas("rt flags opaque", &opaque_size);
    let masked = blas("rt flags masked", &masked_size);
    let mut tlas = device.create_tlas(&wgpu::CreateTlasDescriptor {
        label: Some("rt flags tlas"),
        max_instances: instances.len() as u32,
        flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
        update_mode: wgpu::AccelerationStructureUpdateMode::Build,
    });
    for (i, &(non_opaque, mirrored, flags)) in instances.iter().enumerate() {
        let sx = if mirrored { -1.0 } else { 1.0 };
        let transform = [sx, 0.0, 0.0, i as f32 * STEP, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        let blas = if non_opaque { &masked } else { &opaque };
        tlas[i] = Some(wgpu::TlasInstance::new(blas, transform, 100 + i as u32, 1 << i).with_flags(flags));
    }
    let entry = |blas, size| wgpu::BlasBuildEntry {
        blas,
        geometry: wgpu::BlasGeometries::TriangleGeometries(vec![wgpu::BlasTriangleGeometry {
            size,
            vertex_buffer: &vertices,
            first_vertex: 0,
            vertex_stride: std::mem::size_of::<[f32; 3]>() as wgpu::BufferAddress,
            index_buffer: None,
            first_index: None,
            transform_buffer: None,
            transform_buffer_offset: None,
        }]),
    };
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.build_acceleration_structures([&entry(&opaque, &opaque_size), &entry(&masked, &masked_size)], [&tlas]);
    queue.submit([encoder.finish()]);
    Scene {
        _vertices: vertices,
        _opaque: opaque,
        _masked: masked,
        tlas,
    }
}

/// Traces `cases` through `scene` and reads back what each ray met.
fn trace(device: &wgpu::Device, queue: &wgpu::Queue, scene: &Scene, cases: &[Case]) -> Vec<Seen> {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("rt flags shader"),
        source: wgpu::ShaderSource::Wgsl(SHADER.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("rt flags pipeline"),
        layout: None,
        module: &module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let input = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("rt flags cases"),
        contents: bytemuck::cast_slice(cases),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let bytes = (cases.len() * 16) as u64;
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("rt flags results"),
        size: bytes,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let read = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("rt flags read"),
        size: bytes,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("rt flags bind"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::AccelerationStructure(&scene.tlas),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: input.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: output.as_entire_binding(),
            },
        ],
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.dispatch_workgroups(cases.len() as u32, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &read, 0, bytes);
    queue.submit([encoder.finish()]);
    read.slice(..).map_async(wgpu::MapMode::Read, |r| r.expect("map"));
    device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
    let data = read.slice(..).get_mapped_range().expect("mapped");
    bytemuck::cast_slice::<u8, [u32; 4]>(&data)
        .iter()
        .map(|r| Seen {
            hit: r[0] == 1,
            front: r[1] == 1,
            candidates: r[2],
            custom: r[3],
        })
        .collect()
}

/// The checks on one backend; `Err` names the first one that failed.
fn check(backend: wgpu::Backends) -> Result<String, String> {
    use wgpu::TlasInstanceFlags as F;
    let (device, queue, name) = match device(backend) {
        Ok(d) => d,
        Err(why) => return Ok(format!("skipped: {why}")),
    };
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let built = scene(
        &device,
        &queue,
        &[
            (false, false, F::empty()),                      // 0 plain
            (false, false, F::TRIANGLE_FACING_CULL_DISABLE), // 1 double-sided
            (false, false, F::TRIANGLE_FLIP_FACING),         // 2 flipped
            (false, true, F::empty()),                       // 3 mirrored
            (false, true, F::TRIANGLE_FLIP_FACING),          // 4 mirrored, flipped
            (true, false, F::empty()),                       // 5 non-opaque
            (true, false, F::FORCE_OPAQUE),                  // 6 non-opaque, forced opaque
            (false, false, F::FORCE_NO_OPAQUE),              // 7 opaque, forced non-opaque
        ],
    );
    let o = (0.0, -0.5);
    let cases = [
        case(0, true, NONE, o, false),       // 0
        case(0, false, NONE, o, false),      // 1
        case(0, true, CULL_BACK, o, false),  // 2
        case(0, false, CULL_BACK, o, false), // 3
        case(1, true, CULL_BACK, o, false),  // 4
        case(1, false, CULL_BACK, o, false), // 5
        case(2, true, NONE, o, false),       // 6
        case(2, true, CULL_BACK, o, false),  // 7
        case(3, true, NONE, o, false),       // 8
        case(4, true, NONE, o, false),       // 9
        case(5, true, NONE, ACCEPTED, true), // 10
        case(5, true, NONE, REFUSED, true),  // 11
        case(6, true, NONE, REFUSED, true),  // 12
        case(7, true, NONE, REFUSED, true),  // 13
        case(7, true, NONE, ACCEPTED, true), // 14
    ];
    let s = trace(&device, &queue, &built, &cases);
    if let Some(e) = pollster::block_on(scope.pop()) {
        return Err(format!("{name}: validation error: {e}"));
    }
    let fail = |what: &str| Err(format!("{name}: {what}; seen {s:#?}"));
    let front_from_plus_z = s[0].front;

    if !(s[0].hit && s[1].hit) || s[0].front == s[1].front {
        return fail("a plain instance is hit from both sides, front on one side only");
    }
    if s.iter().take(10).any(|r| r.hit && r.candidates != 0) {
        return fail("an opaque BLAS gave a candidate");
    }
    if s[0].custom != 100 || s[6].custom != 102 || s[10].custom != 105 {
        return fail("instance custom data");
    }
    // CULL_BACK keeps the front-facing side only.
    if s[2].hit != front_from_plus_z || s[3].hit == front_from_plus_z {
        return fail("CULL_BACK_FACING on a plain instance");
    }
    // Double-sided: CULL_BACK no longer culls either side.
    if !(s[4].hit && s[5].hit) {
        return fail("TRIANGLE_FACING_CULL_DISABLE");
    }
    // Flipped: the front side changes, and CULL_BACK follows it.
    if !s[6].hit || s[6].front == front_from_plus_z || s[7].hit != s[6].front {
        return fail("TRIANGLE_FLIP_FACING");
    }
    // Mirrored then flipped: the flag inverts whatever the mirror gives.
    if !(s[8].hit && s[9].hit) || s[8].front == s[9].front {
        return fail("TRIANGLE_FLIP_FACING on a mirrored instance");
    }
    // Candidate loop: the alpha test accepts one point and refuses the other.
    if !(s[10].hit && s[10].candidates == 1) || s[11].hit || s[11].candidates != 1 {
        return fail("candidate loop of a non-opaque BLAS");
    }
    // Forced opaque: no candidate, the refused point is hit.
    if !s[12].hit || s[12].candidates != 0 {
        return fail("FORCE_OPAQUE");
    }
    // Forced non-opaque: an opaque BLAS goes through the candidate loop.
    if s[13].hit || s[13].candidates != 1 || !s[14].hit {
        return fail("FORCE_NO_OPAQUE");
    }

    // Both opacity overrides at once: refused when the TLAS is built.
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    drop(scene(&device, &queue, &[(false, false, F::FORCE_OPAQUE | F::FORCE_NO_OPAQUE)]));
    if pollster::block_on(scope.pop()).is_none() {
        return Err(format!("{name}: FORCE_OPAQUE | FORCE_NO_OPAQUE was not refused"));
    }

    Ok(format!(
        "{name}: ok; seen from +z the counter-clockwise triangle is {}, a mirrored instance {}",
        if front_from_plus_z { "front-facing" } else { "back-facing" },
        if s[8].front == front_from_plus_z {
            "keeps that facing (decided in object space)"
        } else {
            "inverts it"
        }
    ))
}

#[test]
#[ignore]
fn rt_flags_on_every_backend() {
    let _gpu = crate::test_gpu::lock();
    let mut failures = Vec::new();
    for backend in [wgpu::Backends::VULKAN, wgpu::Backends::DX12, wgpu::Backends::METAL] {
        match check(backend) {
            Ok(report) => eprintln!("rt flags, {backend:?}: {report}"),
            Err(failure) => failures.push(failure),
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}
