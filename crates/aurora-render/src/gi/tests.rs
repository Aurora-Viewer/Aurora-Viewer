use super::*;
use glam::IVec2;

fn close(a: Vec3, b: Vec3, eps: f32) -> bool {
    (a - b).abs().max_element() <= eps
}

/// Directions spread over the sphere, poles and axes included.
fn directions() -> Vec<Vec3> {
    let mut d: Vec<Vec3> = (0..500).map(|i| ray_dir(i % RAYS, &rotation(u64::from(i) / 7))).collect();
    d.extend([
        Vec3::X,
        -Vec3::X,
        Vec3::Y,
        -Vec3::Y,
        Vec3::Z,
        -Vec3::Z,
        Vec3::new(1.0, 1.0, 0.0).normalize(),
    ]);
    d
}

#[test]
fn the_octahedral_map_goes_back_and_forth() {
    for n in directions() {
        let e = oct_encode(n);
        assert!(e.abs().max_element() <= 1.0 + 1e-6, "{n} → {e}");
        assert!(close(oct_decode(e), n, 1e-5), "{n} → {e} → {}", oct_decode(e));
    }
}

#[test]
fn the_texels_of_a_tile_cover_both_hemispheres() {
    for interior in [IRR_INTERIOR, DEPTH_INTERIOR] {
        let dirs: Vec<Vec3> = (0..interior * interior)
            .map(|i| texel_dir(UVec2::new(i % interior, i / interior), interior))
            .collect();
        assert!(dirs.iter().all(|d| (d.length() - 1.0).abs() < 1e-5));
        let mean = dirs.iter().copied().sum::<Vec3>() / dirs.len() as f32;
        assert!(mean.length() < 0.05, "{interior}: mean {mean}");
        assert!(dirs.iter().any(|d| d.z > 0.9) && dirs.iter().any(|d| d.z < -0.9));
    }
}

/// Folds a point past the octahedral square's edges back inside it: the
/// point just past an edge is the one just inside it, mirrored along it.
fn fold(mut e: Vec2) -> Vec2 {
    if e.x.abs() > 1.0 {
        e = Vec2::new(e.x.signum() * 2.0 - e.x, -e.y);
    }
    if e.y.abs() > 1.0 {
        e = Vec2::new(-e.x, e.y.signum() * 2.0 - e.y);
    }
    e
}

#[test]
fn a_border_texel_copies_the_texel_across_the_fold() {
    for interior in [IRR_INTERIOR, DEPTH_INTERIOR] {
        let s = interior + 1;
        for y in 0..=s {
            for x in 0..=s {
                let b = UVec2::new(x, y);
                let src = border_source(b, interior);
                assert!((1..=interior).contains(&src.x) && (1..=interior).contains(&src.y), "{b} → {src}");
                let virtual_point = texel_point(b.as_ivec2() - IVec2::ONE, interior);
                let source_point = texel_point(src.as_ivec2() - IVec2::ONE, interior);
                assert!((fold(virtual_point) - source_point).length() < 1e-5, "{interior}: {b} → {src}");
            }
        }
    }
}

#[test]
fn every_border_texel_is_written_by_the_texel_it_copies() {
    for interior in [IRR_INTERIOR, DEPTH_INTERIOR] {
        let s = interior + 1;
        let mut written = vec![0; ((s + 1) * (s + 1)) as usize];
        for y in 0..interior {
            for x in 0..interior {
                let t = UVec2::new(x, y);
                for b in border_targets(t, interior) {
                    assert_eq!(border_source(b, interior), t + UVec2::ONE, "{b}");
                    written[(b.y * (s + 1) + b.x) as usize] += 1;
                }
            }
        }
        for y in 0..=s {
            for x in 0..=s {
                let border = x == 0 || y == 0 || x == s || y == s;
                assert_eq!(written[(y * (s + 1) + x) as usize], u32::from(border), "({x}, {y})");
            }
        }
    }
}

#[test]
fn the_rays_cover_the_sphere_evenly() {
    let r = rotation(0);
    let dirs: Vec<Vec3> = (0..RAYS).map(|i| ray_dir(i, &r)).collect();
    assert!(dirs.iter().all(|d| (d.length() - 1.0).abs() < 1e-5));
    assert!((dirs.iter().copied().sum::<Vec3>() / RAYS as f32).length() < 0.02);
    // every octant takes about an eighth of them
    for octant in 0..8 {
        let s = Vec3::new(
            if octant & 1 == 0 { 1.0 } else { -1.0 },
            if octant & 2 == 0 { 1.0 } else { -1.0 },
            if octant & 4 == 0 { 1.0 } else { -1.0 },
        );
        let n = dirs.iter().filter(|d| (**d * s).cmpgt(Vec3::ZERO).all()).count();
        assert!((10..=22).contains(&n), "octant {octant}: {n}");
    }
}

#[test]
fn the_fixed_rays_stay_and_the_others_turn() {
    let (a, b) = (rotation(1), rotation(2));
    for i in 0..FIXED_RAYS {
        assert_eq!(ray_dir(i, &a), ray_dir(i, &b));
    }
    assert!((FIXED_RAYS..RAYS).all(|i| !close(ray_dir(i, &a), ray_dir(i, &b), 1e-3)));
    // the fixed set alone covers the sphere: every octant has some
    let fixed: Vec<Vec3> = (0..FIXED_RAYS).map(|i| ray_dir(i, &a)).collect();
    for octant in 0..8 {
        let s = Vec3::new(
            if octant & 1 == 0 { 1.0 } else { -1.0 },
            if octant & 2 == 0 { 1.0 } else { -1.0 },
            if octant & 4 == 0 { 1.0 } else { -1.0 },
        );
        assert!(fixed.iter().any(|d| (*d * s).cmpgt(Vec3::ZERO).all()), "octant {octant}");
    }
}

/// The fixed rays' `w` of a probe at `p` beside a wall, the plane x = `wall`
/// (its front facing -x), seen from `p.x` < `wall` (front) or > (back).
fn wall_rays(p: Vec3, wall: f32) -> Vec<f32> {
    (0..FIXED_RAYS)
        .map(|i| {
            let d = fibonacci(i, FIXED_RAYS);
            let t = (wall - p.x) / d.x;
            if !t.is_finite() || t <= 0.0 {
                MISS
            } else if p.x < wall {
                t
            } else {
                -BACK_SCALE * t
            }
        })
        .collect()
}

#[test]
fn a_probe_inside_a_wall_moves_out_through_its_nearest_back_face() {
    // the probe in a box from -0.3 to 0.3 on every axis: every ray meets
    // the inside of its faces
    let spacing = 1.5;
    let w: Vec<f32> = (0..FIXED_RAYS)
        .map(|i| {
            let d = fibonacci(i, FIXED_RAYS);
            let t = 0.3 / d.abs().max_element();
            -BACK_SCALE * t
        })
        .collect();
    let (offset, inside) = relocate(Vec3::ZERO, &w, spacing);
    assert!(inside);
    // out of the box by half the least distance to a face
    let out = offset.abs().max_element();
    assert!((out - (0.3 + 0.5 * MIN_FRONT * spacing)).abs() < 0.05, "{offset}");
    assert!((offset / spacing).length() < MAX_OFFSET);
}

#[test]
fn a_probe_too_near_a_face_moves_away_and_one_far_from_all_goes_back() {
    let spacing = 1.5;
    // 0.1 m before the wall, under MIN_FRONT (0.3 m): away from it, along -x
    let (offset, inside) = relocate(Vec3::ZERO, &wall_rays(Vec3::ZERO, 0.1), spacing);
    assert!(!inside);
    assert!(offset.x < -0.05, "{offset}");
    assert!((offset / spacing).length() < MAX_OFFSET);
    // with nothing near, a moved probe goes back to its grid point
    let far = vec![MISS; FIXED_RAYS as usize];
    let (back, inside) = relocate(Vec3::new(0.3, 0.0, 0.0), &far, spacing);
    assert!(!inside);
    assert_eq!(back, Vec3::ZERO);
    // behind an endless one-sided wall, half its rays meet the back: the
    // probe counts as inside (the wall closes the space it looks into) and
    // goes through to the front, as RTXGI's do
    let (offset, inside) = relocate(Vec3::ZERO, &wall_rays(Vec3::new(1.0, 0.0, 0.0), 0.5), spacing);
    assert!(inside);
    assert!(offset.x < -0.5, "{offset}");
}

#[test]
fn a_probe_never_moves_past_its_ellipsoid() {
    let spacing = 1.5;
    let start = Vec3::new(0.6, 0.0, 0.0);
    let w = vec![-BACK_SCALE * 0.6; FIXED_RAYS as usize];
    let (offset, inside) = relocate(start, &w, spacing);
    assert!(inside);
    // 0.6 + 0.6 + 0.15 m would be past 0.45 spacings: it stays
    assert_eq!(offset, start);
}

#[test]
fn the_sight_of_the_sky_follows_a_change_and_keeps_its_noise_smooth() {
    let aged = VALID | 30;
    let h = hysteresis(aged);
    // a few rays more or less: the full hysteresis
    assert!((blend_sky(0.5, 0.6, aged) - (0.6 + (0.5 - 0.6) * h)).abs() < 1e-6);
    // a roof over the probe: followed within a few updates
    assert!(blend_sky(1.0, 0.0, aged) < 0.25);
    // a probe's first sight is the rays' own
    assert_eq!(blend_sky(0.0, 0.7, 0), 0.7);
}

#[test]
fn a_change_of_the_light_is_followed_and_a_jump_softened() {
    let old = Vec3::splat(0.5);
    let aged = VALID | 30;
    // noise: the full hysteresis
    let small = blend(old, Vec3::splat(0.55), aged);
    let h = hysteresis(aged);
    assert!(close(small, Vec3::splat(0.55).lerp(old, h), 1e-6));
    // the light went out: followed within a few updates
    let out = blend(old, Vec3::ZERO, aged);
    assert!(out.max_element() < 0.5 * 0.25, "{out}");
    // a lamp came on, far brighter: a quarter of the rise at most
    let on = blend(old, Vec3::splat(4.5), aged);
    assert!(on.x > 0.5 && on.x <= 0.5 + 4.0 * 0.25 + 1e-6, "{on}");
    // a probe's first light is the rays' own
    assert_eq!(blend(Vec3::splat(9.0), Vec3::splat(0.2), 0), Vec3::splat(0.2));
}

#[test]
fn the_rotation_is_a_rotation_and_changes_every_frame() {
    let mut previous = Mat3::IDENTITY;
    for frame in 0..200 {
        let r = rotation(frame);
        assert!((r.determinant() - 1.0).abs() < 1e-4, "{frame}");
        assert!((r * r.transpose()).abs_diff_eq(Mat3::IDENTITY, 1e-4), "{frame}");
        assert!(!r.abs_diff_eq(previous, 1e-3), "{frame}");
        previous = r;
    }
}

#[test]
fn slots_and_cells() {
    for i in [0, 1, DIMS.x - 1, DIMS.x, PROBES / 2, PROBES - 1] {
        assert_eq!(slot_index(slot(i)), i);
        assert!(slot(i).cmplt(DIMS).all());
    }
    let origin = IVec3::new(-37, 5, 211);
    for i in (0..PROBES).step_by(97) {
        let s = slot(i);
        let cell = cell_of_slot(s, origin);
        assert!(
            cell.cmpge(origin).all() && cell.cmplt(origin + DIMS.as_ivec3()).all(),
            "{s} → {cell}"
        );
        assert_eq!(wrap(cell), s);
        assert!(!entered(cell, origin));
    }
}

#[test]
fn a_grid_moved_by_one_cell_resets_one_slice() {
    let old = IVec3::new(10, -3, 40);
    let new = old + IVec3::X;
    let reset = (0..PROBES).filter(|&i| entered(cell_of_slot(slot(i), new), old)).count() as u32;
    assert_eq!(reset, DIMS.y * DIMS.z);
    let far = old + DIMS.as_ivec3() * 2;
    assert!((0..PROBES).all(|i| entered(cell_of_slot(slot(i), far), old)));
}

#[test]
fn the_camera_sits_in_the_middle_of_each_grid() {
    for camera in [Vec3::ZERO, Vec3::new(128.3, 17.9, 22.0), Vec3::new(-5.0, 250.0, 3500.0)] {
        for (c, spacing) in SPACING.iter().enumerate() {
            let cell = (camera / *spacing).floor().as_ivec3();
            assert_eq!(cell - origin(camera, c), (DIMS / 2).as_ivec3());
        }
    }
}

#[test]
fn the_tiles_fill_the_atlas_without_overlap() {
    for tile in [IRR_TILE, DEPTH_TILE] {
        let size = atlas_size(tile);
        assert!(size.max_element() <= 8192, "{size}");
        let mut seen = std::collections::HashSet::new();
        for c in 0..CASCADES as u32 {
            for i in 0..PROBES {
                let o = tile_origin(c, slot(i), tile);
                assert!((o + UVec2::splat(tile)).cmple(size).all());
                assert!(seen.insert(o));
            }
        }
    }
}

#[test]
fn the_plan_shares_the_budget_and_visits_every_probe() {
    let p = plan(0, BUDGET);
    assert!(p.total() <= BUDGET);
    assert_eq!(p.offset, [0, p.count[0], p.count[0] + p.count[1]]);
    assert!(p.count[0] > p.count[1] && p.count[1] > p.count[2]);
    for c in 0..CASCADES {
        let frames = PROBES.div_ceil(p.count[c]);
        let mut seen = vec![false; PROBES as usize];
        for f in 0..u64::from(frames) {
            let q = plan(f, BUDGET);
            for k in 0..q.count[c] {
                seen[((q.first[c] + k) % PROBES) as usize] = true;
            }
        }
        assert!(seen.iter().all(|&s| s), "grid {c} in {frames} frames");
    }
    assert_eq!(plan(5, 0).total(), 0);
    assert!(plan(5, u32::MAX / 8).count.iter().all(|&n| n == PROBES));
}

#[test]
fn the_hysteresis_grows_with_the_age() {
    assert_eq!(hysteresis(0), 0.0);
    assert_eq!(hysteresis(5), 0.0);
    assert_eq!(hysteresis(VALID), 0.0);
    assert_eq!(hysteresis(VALID | 1), 0.5);
    assert!((hysteresis(VALID | 2) - 2.0 / 3.0).abs() < 1e-6);
    assert_eq!(hysteresis(VALID | 1000), MAX_HYSTERESIS);
}

#[test]
fn the_clipmap_resets_everything_first_then_follows_the_camera() {
    let mut map = Clipmap::default();
    let first = map.advance(Vec3::new(3.0, 4.0, 25.0), BUDGET, true);
    for k in &first.cascades {
        let (o, old) = (IVec3::from_slice(&k.origin[..3]), IVec3::from_slice(&k.old_origin[..3]));
        assert!((0..PROBES).all(|i| entered(cell_of_slot(slot(i), o), old)));
    }
    let second = map.advance(Vec3::new(3.0 + SPACING[0], 4.0, 25.0), BUDGET, true);
    assert_eq!(second.cascades[0].old_origin, first.cascades[0].origin);
    assert_eq!(second.cascades[0].origin[0], first.cascades[0].origin[0] + 1);
    assert_ne!(second.rotation, first.rotation);
    assert_ne!(second.cascades[0].plan, first.cascades[0].plan);
    let lost = map.advance(Vec3::NAN, BUDGET, true);
    assert_eq!(lost.cascades[0].origin, second.cascades[0].origin);
    map.reset();
    let again = map.advance(Vec3::new(3.0 + SPACING[0], 4.0, 25.0), BUDGET, true);
    assert_ne!(again.cascades[0].old_origin, again.cascades[0].origin);
    assert_eq!(first.frame[1], 1);
    assert_eq!(Clipmap::default().advance(Vec3::ZERO, BUDGET, false).frame[1], 0);
}

#[test]
fn the_volume_fits_in_its_video_memory() {
    let bytes = memory_bytes(BUDGET);
    assert!(bytes < 64 << 20, "{} MB", bytes >> 20);
    assert!(bytes > 16 << 20, "{} MB", bytes >> 20);
}

fn validate(source: &str) -> naga::Module {
    let module = naga::front::wgsl::parse_str(source).unwrap_or_else(|e| panic!("{}", e.emit_to_string(source)));
    // pack2x16float, as gtao.wgsl uses it (no f16 type: the f32 packing
    // every backend has)
    let caps = naga::valid::Capabilities::SHADER_FLOAT16_IN_FLOAT32;
    let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), caps)
        .validate(&module)
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(source)));
    // what the drivers get: Vulkan's SPIR-V and Metal's MSL, one entry
    // point at a time (as wgpu writes them)
    for ep in &module.entry_points {
        let spv = naga::back::spv::PipelineOptions {
            shader_stage: ep.stage,
            entry_point: ep.name.clone(),
        };
        naga::back::spv::write_vec(&module, &info, &naga::back::spv::Options::default(), Some(&spv))
            .unwrap_or_else(|e| panic!("{}: {e:?}", ep.name));
        let msl = naga::back::msl::PipelineOptions {
            entry_point: Some((ep.stage, ep.name.clone())),
            ..Default::default()
        };
        naga::back::msl::write_string(
            &module,
            &info,
            &naga::back::msl::Options {
                lang_version: (2, 4),
                ..Default::default()
            },
            &msl,
        )
        .unwrap_or_else(|e| panic!("{}: {e:?}", ep.name));
    }
    module
}

/// The bindings a shading module declares to read the volume, and an entry
/// point that reads it.
const SAMPLE_STUB: &str = "
@group(0) @binding(0) var<uniform> gi: GiUniforms;
@group(0) @binding(1) var<storage, read> gi_state: array<u32>;
@group(0) @binding(2) var gi_irr_tex: texture_2d<f32>;
@group(0) @binding(3) var gi_depth_tex: texture_2d<f32>;
@group(0) @binding(4) var gi_sampler: sampler;
@group(0) @binding(5) var<storage, read> gi_offsets: array<vec4<f32>>;
fn gi_probe_seen(probe: vec3<f32>, point: vec3<f32>) -> bool {
    return true;
}
fn gi_note(kind: u32) {}
fn gi_note_add(kind: u32, n: u32) {}
@fragment
fn fs_probe(@location(0) p: vec3<f32>, @location(1) n: vec3<f32>) -> @location(0) vec4<f32> {
    return gi_irradiance(p, normalize(n), vec3<f32>(0.0, 0.0, 1.0));
}
";

#[test]
fn the_gi_shaders_are_valid_for_vulkan_and_metal() {
    let update = validate(&update_source());
    assert_eq!(update.entry_points.len(), 4);
    let sample = validate(&(sample_source() + SAMPLE_STUB));
    assert_eq!(sample.entry_points.len(), 1);
}

#[test]
fn the_uniforms_are_laid_out_as_the_shaders_read_them() {
    let module = validate(&update_source());
    let span = module
        .types
        .iter()
        .find(|(_, t)| t.name.as_deref() == Some("GiUniforms"))
        .map(|(_, t)| match &t.inner {
            naga::TypeInner::Struct { span, .. } => *span,
            other => panic!("{other:?}"),
        })
        .expect("GiUniforms");
    assert_eq!(span as usize, std::mem::size_of::<GiUniforms>());
}

fn f16(bits: u16) -> f32 {
    let sign = if bits >> 15 == 1 { -1.0 } else { 1.0 };
    let exp = i32::from((bits >> 10) & 31);
    let mant = f32::from(bits & 1023);
    sign * match exp {
        0 => mant * 2f32.powi(-24),
        31 => f32::INFINITY,
        e => (1.0 + mant / 1024.0) * 2f32.powi(e - 15),
    }
}

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
}

fn gpu() -> Option<Gpu> {
    let instance = wgpu::Instance::default();
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()?;
    Some(Gpu { device, queue })
}

impl Gpu {
    fn read_buffer(&self, buffer: &wgpu::Buffer, offset: u64, size: u64) -> Vec<u8> {
        let out = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gi test read back"),
            size: crate::readback::readback_size(size),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_buffer_to_buffer(buffer, offset, &out, 0, size);
        self.queue.submit([enc.finish()]);
        out.map_async(wgpu::MapMode::Read, .., |r| r.expect("map"));
        self.device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
        out.get_mapped_range(..size).expect("mapped").to_vec()
    }

    /// The `tile` x `tile` texels of an atlas at `origin`, rgba.
    fn read_tile(&self, texture: &wgpu::Texture, origin: UVec2, tile: u32) -> Vec<[f32; 4]> {
        let out = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gi test tile"),
            size: crate::readback::readback_size(256 * u64::from(tile)),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: origin.x,
                    y: origin.y,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &out,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width: tile,
                height: tile,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([enc.finish()]);
        out.map_async(wgpu::MapMode::Read, .., |r| r.expect("map"));
        self.device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
        let bytes = out.get_mapped_range(..).expect("mapped").to_vec();
        let mut texels = Vec::new();
        for y in 0..tile as usize {
            for x in 0..tile as usize {
                let at = y * 256 + x * 8;
                let half = |i: usize| f16(u16::from_le_bytes([bytes[at + 2 * i], bytes[at + 2 * i + 1]]));
                texels.push([half(0), half(1), half(2), half(3)]);
            }
        }
        texels
    }

    fn run(&self, volume: &GiVolume, u: &GiUniforms, rays: &[[f32; 4]]) {
        self.queue.write_buffer(&volume.rays, 0, bytemuck::cast_slice(rays));
        volume.write(&self.queue, u);
        let mut enc = self.device.create_command_encoder(&Default::default());
        volume.encode(&mut enc, u, None);
        self.queue.submit([enc.finish()]);
        self.device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
    }
}

/// A smooth light, so that the GPU's f16 result can be compared with the
/// CPU's sum: brighter towards +x, bluer up.
fn light(d: Vec3) -> Vec3 {
    Vec3::new(0.6 + 0.4 * d.x, 0.5, 0.5 + 0.5 * d.z)
}

/// A ray's distance; upwards past 60 degrees it meets nothing (the sky).
fn distance(d: Vec3) -> f32 {
    if d.z > 0.5 { MISS } else { 1.0 + 0.5 * d.y }
}

/// What a texel of direction `dir` should hold after its first update: its
/// light, its sight of the sky, its distances.
fn expected(dir: Vec3, r: &Mat3) -> (Vec3, f32, Vec2) {
    let (mut irr, mut sky, mut w_irr, mut depth, mut w_depth) = (Vec3::ZERO, 0.0, 0.0, Vec2::ZERO, 0.0);
    let far = SPACING[0] * FAR_SPACINGS;
    for i in FIXED_RAYS..RAYS {
        let d = ray_dir(i, r);
        let w = dir.dot(d).max(0.0);
        let t = distance(d);
        irr += light(d) * w;
        if t == MISS {
            sky += w;
        }
        w_irr += w;
        let wd = w.powf(DEPTH_SHARPNESS);
        let t = if t == MISS { far } else { t.min(far) };
        depth += Vec2::new(t, t * t) * wd;
        w_depth += wd;
    }
    (irr / w_irr, sky / w_irr, depth / w_depth)
}

#[test]
fn the_update_passes_fold_the_rays_into_the_tiles() {
    let _gpu = crate::test_gpu::lock();
    let Some(gpu) = gpu() else {
        eprintln!("no GPU adapter: skipped");
        return;
    };
    let volume = GiVolume::new(&gpu.device, 4);
    let origins = [IVec3::new(-12, -12, -6), IVec3::new(-5, -5, -3), IVec3::new(-2, -2, -1)];
    // two probes of the finest grid, one of the next
    let p = Plan {
        first: [12, 7, 0],
        count: [2, 1, 0],
        offset: [0, 2, 3],
    };
    let r = rotation(42);
    let u = uniforms(origins, origins, &p, &r, true);
    // probe 13 (entry 1) sits in a box of 0.3 m: its fixed rays meet the
    // inside of the faces, it moves out and is inside until it is out
    let boxed = |d: Vec3| -BACK_SCALE * 0.3 / d.abs().max_element();
    let rays: Vec<[f32; 4]> = (0..p.total() * RAYS)
        .map(|k| {
            let d = ray_dir(k % RAYS, &r);
            let l = light(d);
            let w = if k / RAYS == 1 && k % RAYS < FIXED_RAYS {
                boxed(d)
            } else {
                distance(d)
            };
            [l.x, l.y, l.z, w]
        })
        .collect();
    gpu.run(&volume, &u, &rays);

    let state = gpu.read_buffer(&volume.state, 0, u64::from(PROBES * CASCADES as u32) * 4);
    let state: &[u32] = bytemuck::cast_slice(&state);
    assert_eq!(state[12], VALID | 1);
    assert_eq!(state[13], VALID | INSIDE | 1);
    let offsets = gpu.read_buffer(&volume.offsets, 12 * 16, 32);
    let offsets: &[[f32; 4]] = bytemuck::cast_slice(&offsets);
    let w13: Vec<f32> = rays[RAYS as usize..(RAYS + FIXED_RAYS) as usize].iter().map(|r| r[3]).collect();
    let w12: Vec<f32> = rays[..FIXED_RAYS as usize].iter().map(|r| r[3]).collect();
    for (got, w) in [(offsets[0], &w12), (offsets[1], &w13)] {
        let (want, _) = relocate(Vec3::ZERO, w, SPACING[0]);
        assert!(close(Vec3::from_slice(&got[..3]), want, 1e-4), "{got:?} vs {want}");
    }
    assert!(Vec3::from_slice(&offsets[1][..3]).length() > 0.3);
    assert_eq!(state[(PROBES + 7) as usize], VALID | 1);
    assert_eq!(state.iter().filter(|&&s| s != 0).count(), 3);

    let irr = gpu.read_tile(&volume.irr_atlas, tile_origin(0, slot(12), IRR_TILE), IRR_TILE);
    let depth = gpu.read_tile(&volume.depth_atlas, tile_origin(0, slot(12), DEPTH_TILE), DEPTH_TILE);
    for (tile, texels, interior) in [(IRR_TILE, &irr, IRR_INTERIOR), (DEPTH_TILE, &depth, DEPTH_INTERIOR)] {
        for y in 0..tile {
            for x in 0..tile {
                let b = UVec2::new(x, y);
                let src = border_source(b, interior);
                let got = texels[(y * tile + x) as usize];
                let want_at = texels[(src.y * tile + src.x) as usize];
                assert_eq!(got, want_at, "tile {tile}: {b} copies {src}");
                let dir = texel_dir(src - UVec2::ONE, interior);
                let (want_irr, want_sky, want_depth) = expected(dir, &r);
                if tile == IRR_TILE {
                    assert!(close(Vec3::from_slice(&got[..3]), want_irr, 4e-3), "{b}: {got:?} vs {want_irr}");
                    assert!((got[3] - want_sky).abs() < 2e-3, "{b}: sky {} vs {want_sky}", got[3]);
                } else {
                    let got = Vec2::new(got[0], got[1]);
                    // (f16 history: a step of 1/1024 of the value, the far
                    // distance of the sky's rays included)
                    let tolerance = 4e-3 * want_depth.max_element().max(1.0);
                    assert!((got - want_depth).abs().max_element() < tolerance, "{b}: {got} vs {want_depth}");
                }
            }
        }
    }

    // a second update: the light went out, a change past CHANGE, so the
    // probe follows it at once (`blend`)
    // (no hit anywhere: probe 13, out of its box, goes back to its point)
    let dark = vec![[0.0, 0.0, 0.0, MISS]; (p.total() * RAYS) as usize];
    gpu.run(&volume, &u, &dark);
    let again = gpu.read_tile(&volume.irr_atlas, tile_origin(0, slot(12), IRR_TILE), IRR_TILE);
    for (a, b) in again.iter().zip(&irr) {
        let want = blend(Vec3::from_slice(&b[..3]), Vec3::ZERO, VALID | 1);
        assert!(close(Vec3::from_slice(&a[..3]), want, 2e-3), "{a:?} vs {want}");
        // every ray met nothing: the whole sky
        let want = blend_sky(b[3], 1.0, VALID | 1);
        assert!((a[3] - want).abs() < 2e-3, "sky {} vs {want}", a[3]);
    }
    let state = gpu.read_buffer(&volume.state, 48, 8);
    assert_eq!(bytemuck::cast_slice::<u8, u32>(&state), &[VALID | 2, VALID | 2]);

    // the grid moves one cell along x: the slice that entered loses its
    // light (probe 12, slot x 12, is the cell that entered), probe 13 keeps it
    let moved = origins.map(|o| o + IVec3::X);
    let none = Plan::default();
    let u = uniforms(moved, origins, &none, &r, true);
    gpu.run(&volume, &u, &[]);
    let state = gpu.read_buffer(&volume.state, 0, u64::from(PROBES * CASCADES as u32) * 4);
    let state: &[u32] = bytemuck::cast_slice(&state);
    for i in [12u32, 13, PROBES + 7] {
        let c = i / PROBES;
        let cell = cell_of_slot(slot(i % PROBES), moved[c as usize]);
        let kept = !entered(cell, origins[c as usize]);
        assert_eq!(state[i as usize] != 0, kept, "probe {i}");
    }
}
