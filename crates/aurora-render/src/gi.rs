//! Real-time diffuse global illumination by ray-traced irradiance probes:
//! the "Traced" indirect light, an Aurora choice beside the Legacy Viewer's
//! reflection-probe ambiance (`probes.rs`) and GTAO's screen-space indirect
//! light (`gtao.rs`, `main_bitmask`). No Legacy Viewer origin: written from
//! Majercik, Guertin, Nowrouzezahrai and McGuire, "Dynamic Diffuse Global
//! Illumination with Ray-Traced Irradiance Fields" (JCGT 8(2), 2019), and
//! Majercik, Marrs, Spjut and McGuire, "Scaling Probe-Based Real-Time
//! Dynamic Global Illumination for Production" (JCGT 10(2), 2021).
//!
//! - The volume: [`CASCADES`] grids of [`DIMS`] probes around the camera,
//!   [`SPACING`] apart, finer near it. They scroll like a clipmap: a probe's
//!   storage slot is its world cell modulo the grid ([`cell_of_slot`]), so
//!   a camera that crosses a cell only resets the slice of probes that
//!   entered ([`entered`], `cs_reset`). The grids follow the camera, so a
//!   skybox thousands of metres up and the ground are handled alike.
//! - Each frame a budget of probes ([`plan`]) traces [`RAYS`] rays each, in
//!   directions turned by the frame's rotation ([`rotation`]). The tracer
//!   is the shared ray-tracing module's and writes `rays`: the radiance
//!   reaching the probe and the hit distance, negative when the ray left the
//!   scene (rgb is then the sky's). The update passes
//!   (`shaders/gi_update.wgsl`) fold them into each texel of the probe's
//!   octahedral tiles, irradiance and distance moments, with a hysteresis
//!   that grows with the probe's age ([`hysteresis`]), and write the atlases
//!   that `shaders/gi_sample.wgsl` reads for the shading.
//! - Units: the irradiance is the cosine-weighted mean of the incoming
//!   radiance (irradiance over pi), the units of `amb` in `shade_pbr`
//!   (common.wgsl): a Lambert surface of albedo c sends back c times it.
//!
//! Every shader function of `gi_common.wgsl` has its twin here; the tests
//! run the passes on the GPU against them. Not drawn yet: the tracer and the
//! shading's read come with the ray-tracing module (TASKS.md, "Real-time
//! global illumination").

use crate::pipeline_cache::CachedPipelines;
use bytemuck::{Pod, Zeroable};
use glam::{IVec3, Mat3, Quat, UVec2, UVec3, Vec2, Vec3};

/// Grids of probes, from the finest.
pub const CASCADES: usize = 3;
/// Probes per axis of a grid (x, y horizontal, z up).
pub const DIMS: UVec3 = UVec3::new(24, 24, 12);
/// Probes per grid.
pub const PROBES: u32 = DIMS.x * DIMS.y * DIMS.z;
/// Distance between two probes of each grid (m): 36, 96 and 288 m wide.
/// The finest is under the thickness of a room, so that a wall between two
/// rooms has probes on both sides of it.
pub const SPACING: [f32; CASCADES] = [1.5, 4.0, 12.0];
/// Rays per probe update.
pub const RAYS: u32 = 128;
/// The first rays of each update: always the same directions (not turned
/// by the frame's rotation), not shaded and not folded into the tiles, for
/// a stable view of what is around the probe: its relocation and whether
/// it sits inside a wall (Majercik et al. 2021, sections 4.1 and 4.2;
/// RTXGI's 32 fixed rays).
pub const FIXED_RAYS: u32 = 32;
/// A probe nearer a surface than this (spacings) moves away from it.
pub const MIN_FRONT: f32 = 0.2;
/// The share of a probe's fixed rays that meet a face's back over which the
/// probe is inside a wall or a closed object: it moves out through the
/// nearest back face, and its light is read by nothing until it is out.
pub const BACKFACE_RATIO: f32 = 0.25;
/// How far a probe may move from its grid point: inside the ellipsoid of
/// this radius in spacings (the paper's 0.45, so that two probes never
/// cross).
pub const MAX_OFFSET: f32 = 0.45;
/// A ray's `w`: its hit's distance on a face's front, minus this share of
/// it on a face's back, [`MISS`] for no hit.
pub const BACK_SCALE: f32 = 0.2;
pub const MISS: f32 = -1.0e30;
/// Probes updated per frame, shared by the grids by [`SHARES`].
pub const BUDGET: u32 = 2048;
/// The finest grid, where the light changes most under the eye, takes half
/// the budget: the whole of it every 7 frames, the next every 10, the
/// coarsest every 21.
const SHARES: [u32; CASCADES] = [3, 2, 1];
/// Interior texels a side of an irradiance tile, and the tile with its
/// border of one texel (copies across the octahedral fold, so that the
/// bilinear filter never mixes two tiles).
pub const IRR_INTERIOR: u32 = 6;
pub const IRR_TILE: u32 = IRR_INTERIOR + 2;
/// The same for the distance moments.
pub const DEPTH_INTERIOR: u32 = 8;
pub const DEPTH_TILE: u32 = DEPTH_INTERIOR + 2;
/// The most of its old light a probe keeps at each update (the paper's
/// hysteresis): about 1 s to follow a change of the light at 60 fps for the
/// finest grid.
pub const MAX_HYSTERESIS: f32 = 0.97;
/// Exponent of the cosine lobe of the distance texels (the paper's 50).
pub const DEPTH_SHARPNESS: f32 = 50.0;
/// The longest distance a probe's depth keeps, in spacings: farther hits
/// hide nothing between two probes.
const FAR_SPACINGS: f32 = 2.0;
/// Format of both atlases: a storage format of every backend.
pub const ATLAS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// Bit of a probe's state: its tiles hold light gathered at its cell.
pub const VALID: u32 = 0x8000_0000;
/// Bit of a probe's state: its last fixed rays found it inside a wall.
pub const INSIDE: u32 = 0x4000_0000;
const AGE: u32 = 0x00ff_ffff;

/// The `GI_*` constants of the shaders, from the ones above.
fn constants() -> String {
    format!(
        "const GI_CASCADES: u32 = {}u;\nconst GI_RAYS: u32 = {}u;\nconst GI_IRR_INTERIOR: u32 = {}u;\n\
         const GI_IRR_TILE: u32 = {}u;\nconst GI_DEPTH_INTERIOR: u32 = {}u;\nconst GI_DEPTH_TILE: u32 = {}u;\n\
         const GI_MAX_HYSTERESIS: f32 = {:?};\nconst GI_DEPTH_SHARPNESS: f32 = {:?};\n\
         const GI_FIXED_RAYS: u32 = {}u;\nconst GI_MIN_FRONT: f32 = {:?};\nconst GI_BACKFACE_RATIO: f32 = {:?};\n\
         const GI_MAX_OFFSET: f32 = {:?};\nconst GI_BACK_SCALE: f32 = {:?};\nconst GI_MISS: f32 = {:e};\n\
         const GI_CHANGE: f32 = {:?};\nconst GI_JUMP: f32 = {:?};\n",
        CASCADES,
        RAYS,
        IRR_INTERIOR,
        IRR_TILE,
        DEPTH_INTERIOR,
        DEPTH_TILE,
        MAX_HYSTERESIS,
        DEPTH_SHARPNESS,
        FIXED_RAYS,
        MIN_FRONT,
        BACKFACE_RATIO,
        MAX_OFFSET,
        BACK_SCALE,
        MISS,
        CHANGE,
        JUMP
    )
}

/// The update passes' module.
pub fn update_source() -> String {
    constants() + include_str!("shaders/gi_common.wgsl") + include_str!("shaders/gi_update.wgsl")
}

/// What a shading module includes to read the volume (it declares the
/// bindings, see `gi_sample.wgsl`).
pub fn sample_source() -> String {
    constants() + include_str!("shaders/gi_common.wgsl") + include_str!("shaders/gi_sample.wgsl")
}

/// The volume's read with its bindings at group [`SAMPLE_GROUP`]
/// ([`GiVolume::sample_layout`]) and `gi_hit_light`, for a ray-traced
/// shader (after rt_light.wgsl): the probes' tracer and the indirect light's
/// hits (rt_gi.wgsl).
pub fn bound_sample_source() -> String {
    bound_sample_source_with(PLAIN_HOOKS)
}

/// [`bound_sample_source`] with `hooks` in the place of [`PLAIN_HOOKS`]:
/// rt_gi_counts.rs's, which count the reads' work.
pub fn bound_sample_source_with(hooks: &str) -> String {
    sample_source() + SAMPLE_BINDINGS + include_str!("shaders/gi_hit.wgsl") + hooks
}

/// What gi_sample.wgsl and gi_hit.wgsl ask of their includer, without the
/// counts of `AURORA_RT_GI_DIAG=count`: `gi_note` counts nothing,
/// `gi_count_start` and `gi_count_flush` (rt_gi.wgsl's passes) do nothing,
/// and a probe's visibility is rt_light.wgsl's shadow ray against the
/// scene's faces.
pub const PLAIN_HOOKS: &str = "
fn gi_note(kind: u32) {}
fn gi_note_add(kind: u32, n: u32) {}
fn gi_count_start(pass_index: u32, group: vec3<u32>) {}
fn gi_count_flush() {}
fn gi_probe_ray(origin: vec3<f32>, dir: vec3<f32>, reach: f32) -> bool {
    return unblocked(origin, dir, reach, false);
}
";

/// The group a ray-traced pass binds [`GiVolume::sample_group`] at (its
/// groups 0 to 2 are its own, the scene's tables and the textures).
pub const SAMPLE_GROUP: u32 = 3;
const SAMPLE_BINDINGS: &str = "
@group(3) @binding(0) var<uniform> gi: GiUniforms;
@group(3) @binding(1) var<storage, read> gi_state: array<u32>;
@group(3) @binding(2) var gi_irr_tex: texture_2d<f32>;
@group(3) @binding(3) var gi_depth_tex: texture_2d<f32>;
@group(3) @binding(4) var gi_sampler: sampler;
@group(3) @binding(5) var<storage, read> gi_offsets: array<vec4<f32>>;
";

/// `gi_oct_encode`: a unit direction to the square [-1, 1]².
pub fn oct_encode(n: Vec3) -> Vec2 {
    let p = Vec2::new(n.x, n.y) / (n.x.abs() + n.y.abs() + n.z.abs());
    if n.z <= 0.0 {
        (Vec2::ONE - Vec2::new(p.y.abs(), p.x.abs())) * sign_not_zero(p)
    } else {
        p
    }
}

/// `gi_oct_decode`.
pub fn oct_decode(e: Vec2) -> Vec3 {
    let mut v = Vec3::new(e.x, e.y, 1.0 - e.x.abs() - e.y.abs());
    if v.z < 0.0 {
        let xy = (Vec2::ONE - Vec2::new(v.y.abs(), v.x.abs())) * sign_not_zero(Vec2::new(v.x, v.y));
        v = Vec3::new(xy.x, xy.y, v.z);
    }
    v.normalize()
}

fn sign_not_zero(v: Vec2) -> Vec2 {
    Vec2::new(if v.x >= 0.0 { 1.0 } else { -1.0 }, if v.y >= 0.0 { 1.0 } else { -1.0 })
}

/// `gi_texel_dir`: the direction of interior texel `t` of a tile.
pub fn texel_dir(t: UVec2, interior: u32) -> Vec3 {
    oct_decode(texel_point(t.as_ivec2(), interior))
}

/// The point of the octahedral square at the centre of interior texel `t`
/// (beyond the square for a border texel, at -1 or `interior`).
fn texel_point(t: glam::IVec2, interior: u32) -> Vec2 {
    (t.as_vec2() + Vec2::splat(0.5)) / interior as f32 * 2.0 - Vec2::ONE
}

/// The interior texel a border texel of a tile copies, in tile coordinates
/// (border included: 0 and `interior + 1` are the border). The square folds
/// along its edges: the point just past an edge is the point just inside
/// it, mirrored along that edge; past a corner, the opposite corner.
pub fn border_source(b: UVec2, interior: u32) -> UVec2 {
    let s = interior + 1;
    // a coordinate on the border comes back inside: across its own edge
    // (0 → 1, s → interior), or, at a corner, on the far side (0 →
    // interior, s → 1); the other coordinate of an edge texel is mirrored
    let inward = |a: u32, corner: bool| match (a == 0, corner) {
        (true, false) => 1,
        (true, true) => interior,
        (false, false) => interior,
        (false, true) => 1,
    };
    let on = |a: u32| a == 0 || a == s;
    match (on(b.x), on(b.y)) {
        (true, true) => UVec2::new(inward(b.x, true), inward(b.y, true)),
        (true, false) => UVec2::new(inward(b.x, false), s - b.y),
        (false, true) => UVec2::new(s - b.x, inward(b.y, false)),
        (false, false) => b,
    }
}

/// The border texels interior texel `t` (0-based, without the border) is
/// copied to, as `gi_store_irr` / `gi_store_depth` write them.
pub fn border_targets(t: UVec2, interior: u32) -> Vec<UVec2> {
    let (n, s) = (interior, interior + 1);
    let mut out = Vec::new();
    if t.x == 0 {
        out.push(UVec2::new(0, s - t.y - 1));
    }
    if t.x == n - 1 {
        out.push(UVec2::new(s, s - t.y - 1));
    }
    if t.y == 0 {
        out.push(UVec2::new(s - t.x - 1, 0));
    }
    if t.y == n - 1 {
        out.push(UVec2::new(s - t.x - 1, s));
    }
    if t.x == n - 1 && t.y == n - 1 {
        out.push(UVec2::new(0, 0));
    }
    if t.x == 0 && t.y == n - 1 {
        out.push(UVec2::new(s, 0));
    }
    if t.x == n - 1 && t.y == 0 {
        out.push(UVec2::new(0, s));
    }
    if t.x == 0 && t.y == 0 {
        out.push(UVec2::new(s, s));
    }
    out
}

/// `gi_fibonacci`: point `i` of a spherical Fibonacci set of `n`.
pub fn fibonacci(i: u32, n: u32) -> Vec3 {
    let golden = std::f32::consts::GOLDEN_RATIO;
    let fi = i as f32;
    let phi = 2.0 * std::f32::consts::PI * (fi / golden).fract();
    let cos_theta = 1.0 - (2.0 * fi + 1.0) / n as f32;
    let sin_theta = (1.0 - cos_theta * cos_theta).max(0.0).sqrt();
    Vec3::new(phi.cos() * sin_theta, phi.sin() * sin_theta, cos_theta)
}

/// The rays' rotation of frame `frame`: a uniform random rotation
/// (Shoemake, "Uniform Random Rotations", Graphics Gems III) from a hash of
/// the frame, so that the texels see other directions at each update.
pub fn rotation(frame: u64) -> Mat3 {
    let mut x = frame.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut next = || {
        // splitmix64
        x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = x;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        ((z ^ (z >> 31)) >> 40) as f32 / (1u64 << 24) as f32
    };
    let (u1, u2, u3) = (next(), next(), next());
    let tau = std::f32::consts::TAU;
    let (a, b) = ((1.0 - u1).sqrt(), u1.sqrt());
    let q = Quat::from_xyzw(
        a * (tau * u2).sin(),
        a * (tau * u2).cos(),
        b * (tau * u3).sin(),
        b * (tau * u3).cos(),
    );
    Mat3::from_quat(q.normalize())
}

/// `gi_ray_dir`: ray `i` of an update, the [`FIXED_RAYS`] first as they
/// are, the others a set of their own under `rotation`.
pub fn ray_dir(i: u32, rotation: &Mat3) -> Vec3 {
    if i < FIXED_RAYS {
        return fibonacci(i, FIXED_RAYS);
    }
    (*rotation * fibonacci(i - FIXED_RAYS, RAYS - FIXED_RAYS)).normalize()
}

/// `gi_relocate`: a probe's new offset from its grid point, and whether it
/// is inside a wall, from its offset and the `w` of its fixed rays (see
/// [`BACK_SCALE`]). RTXGI's probe relocation (ProbeRelocationCS), written
/// from the 2021 paper's section 4.1: inside, out through the nearest back
/// face; too near a front face, towards the farthest one unless it lies the
/// same way; far from everything, back towards its grid point; never past
/// [`MAX_OFFSET`].
pub fn relocate(offset: Vec3, w: &[f32], spacing: f32) -> (Vec3, bool) {
    let far = 1.0e6;
    let mut backfaces = 0u32;
    let mut closest_back = (far, Vec3::ZERO);
    let mut closest_front = (far, Vec3::ZERO);
    let mut farthest_front = (0.0, Vec3::ZERO);
    for (r, &w) in w.iter().enumerate().take(FIXED_RAYS as usize) {
        let dir = fibonacci(r as u32, FIXED_RAYS);
        if w < 0.0 && w > MISS * 0.5 {
            backfaces += 1;
            let d = -w / BACK_SCALE;
            if d < closest_back.0 {
                closest_back = (d, dir);
            }
            continue;
        }
        let d = if w < 0.0 { far } else { w };
        if d < closest_front.0 {
            closest_front = (d, dir);
        }
        if d > farthest_front.0 {
            farthest_front = (d, dir);
        }
    }
    let min_front = MIN_FRONT * spacing;
    let inside = backfaces as f32 > BACKFACE_RATIO * FIXED_RAYS as f32;
    let full = if inside {
        offset + closest_back.1 * (closest_back.0 + min_front * 0.5)
    } else if closest_front.0 < min_front {
        if closest_front.1.dot(farthest_front.1) <= 0.0 {
            offset + farthest_front.1 * farthest_front.0.min(0.25 * spacing)
        } else {
            offset
        }
    } else {
        let len = offset.length();
        let back = (closest_front.0 - min_front).min(len);
        if len > 1e-5 { offset - offset / len * back } else { offset }
    };
    let n = full / spacing;
    (if n.dot(n) < MAX_OFFSET * MAX_OFFSET { full } else { offset }, inside)
}

/// `gi_slot`: the slot of probe `i` of a grid.
pub fn slot(i: u32) -> UVec3 {
    UVec3::new(i % DIMS.x, (i / DIMS.x) % DIMS.y, i / (DIMS.x * DIMS.y))
}

/// `gi_slot_index`.
pub fn slot_index(s: UVec3) -> u32 {
    s.x + s.y * DIMS.x + s.z * DIMS.x * DIMS.y
}

/// `gi_wrap`: a cell's slot.
pub fn wrap(a: IVec3) -> UVec3 {
    a.rem_euclid(DIMS.as_ivec3()).as_uvec3()
}

/// `gi_cell_of_slot`: the world cell slot `s` holds while the grid starts
/// at `origin`.
pub fn cell_of_slot(s: UVec3, origin: IVec3) -> IVec3 {
    origin + wrap(s.as_ivec3() - origin).as_ivec3()
}

/// Whether `cell` was outside the grid that started at `old`: its slot
/// held another cell, and `cs_reset` clears it.
pub fn entered(cell: IVec3, old: IVec3) -> bool {
    cell.cmplt(old).any() || cell.cmpge(old + DIMS.as_ivec3()).any()
}

/// `gi_tile_origin`: the corner of slot `s`'s tile in grid `c`'s rows of an
/// atlas of `tile` texels a tile.
pub fn tile_origin(c: u32, s: UVec3, tile: u32) -> UVec2 {
    UVec2::new(s.x + s.z * DIMS.x, s.y + c * DIMS.y) * tile
}

/// Size of an atlas of `tile` texels a tile: the grids one under the other,
/// the layers of a grid side by side.
pub fn atlas_size(tile: u32) -> UVec2 {
    UVec2::new(DIMS.x * DIMS.z * tile, DIMS.y * CASCADES as u32 * tile)
}

/// The first cell of grid `c` for a camera at `camera`: the camera in its
/// middle.
pub fn origin(camera: Vec3, c: usize) -> IVec3 {
    (camera / SPACING[c]).floor().as_ivec3() - (DIMS / 2).as_ivec3()
}

/// `gi_hysteresis`: the share of its old light a probe keeps, none until
/// its tiles hold light, then 1/2, 2/3, 3/4… up to [`MAX_HYSTERESIS`].
pub fn hysteresis(state: u32) -> f32 {
    if state & VALID == 0 {
        return 0.0;
    }
    let age = (state & AGE) as f32;
    (1.0 - 1.0 / (age + 1.0)).min(MAX_HYSTERESIS)
}

/// A texel's light past which its change counts as the light's own (a lamp
/// on or off, a door opened) and not the rays' noise: the hysteresis drops
/// by 0.75, so that the probe follows it within a few updates.
pub const CHANGE: f32 = 0.25;
/// A texel's light past which a rise is taken a quarter at a time (a ray
/// that met a lamp's glint would light the whole probe).
pub const JUMP: f32 = 1.0;

/// `gi_blend`: a texel's light after an update, from its `old` light, the
/// rays' `fresh` one and the probe's state (RTXGI's ProbeBlendingCS, from
/// the 2021 paper; the drop of the hysteresis takes a change both ways,
/// where RTXGI's takes a darkening only).
pub fn blend(old: Vec3, fresh: Vec3, state: u32) -> Vec3 {
    let mut h = hysteresis(state);
    let mut fresh = fresh;
    if state & VALID != 0 {
        let delta = fresh - old;
        if delta.abs().max_element() > CHANGE {
            h = (h - 0.75).max(0.0);
        }
        if delta.max_element() > JUMP {
            fresh = old + delta * 0.25;
        }
    }
    fresh.lerp(old, h)
}

/// `gi_blend_sky`: a texel's sight of the sky (the share of its cosine lobe
/// whose rays met nothing) after an update, as [`blend`] for its light.
pub fn blend_sky(old: f32, fresh: f32, state: u32) -> f32 {
    let mut h = hysteresis(state);
    if state & VALID != 0 && (fresh - old).abs() > CHANGE {
        h = (h - 0.75).max(0.0);
    }
    fresh + (old - fresh) * h
}

/// The layout of [`GiVolume::sample_group`] (gi.rs `SAMPLE_BINDINGS`), the
/// same for every pass that reads the volume: wgpu takes two layouts of the
/// same entries as one, so a pass built with it binds the volume's group or
/// [`empty_sample_group`].
pub fn sample_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    // (the shading of the traced hits and the probes' own rays: compute;
    // later the faces' shading: fragment)
    let read = |binding, ty| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE | wgpu::ShaderStages::FRAGMENT,
        ty,
        count: None,
    };
    let buf = |ty| wgpu::BindingType::Buffer {
        ty,
        has_dynamic_offset: false,
        min_binding_size: None,
    };
    let sampled = wgpu::BindingType::Texture {
        sample_type: wgpu::TextureSampleType::Float { filterable: true },
        view_dimension: wgpu::TextureViewDimension::D2,
        multisampled: false,
    };
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("gi sample"),
        entries: &[
            read(0, buf(wgpu::BufferBindingType::Uniform)),
            read(1, buf(wgpu::BufferBindingType::Storage { read_only: true })),
            read(2, sampled),
            read(3, sampled),
            read(4, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
            read(5, buf(wgpu::BufferBindingType::Storage { read_only: true })),
        ],
    })
}

/// Bilinear inside a tile, the border texels make it seamless; clamped so
/// that the atlas's last tiles never read past its edge.
fn sample_sampler(device: &wgpu::Device) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("gi sample"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    })
}

/// A group of [`sample_layout`] with no volume behind it, for a pass that
/// reads the volume when there is none (the traced indirect light off):
/// its uniforms are zero, so `gi_read` finds nothing (`gi.frame.y` 0) and
/// `gi_sky` lets the whole sky through.
pub fn empty_sample_group(device: &wgpu::Device) -> wgpu::BindGroup {
    let buffer = |label, size: u64, usage| {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage,
            mapped_at_creation: false,
        })
    };
    // (a new buffer is zeroed)
    let uniforms = buffer("gi none", std::mem::size_of::<GiUniforms>() as u64, wgpu::BufferUsages::UNIFORM);
    let state = buffer("gi none state", 16, wgpu::BufferUsages::STORAGE);
    let offsets = buffer("gi none offsets", 16, wgpu::BufferUsages::STORAGE);
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("gi none"),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let sampler = sample_sampler(device);
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("gi none"),
        layout: &sample_layout(device),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: state.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: offsets.as_entire_binding(),
            },
        ],
    })
}

/// The probes of a frame: for each grid, `count` probes from `first`
/// (wrapping), entries `offset ..` of the frame's update list.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub first: [u32; CASCADES],
    pub count: [u32; CASCADES],
    pub offset: [u32; CASCADES],
}

impl Plan {
    /// Entries of the update list: the rays the tracer writes are
    /// `total() * RAYS`.
    pub fn total(&self) -> u32 {
        self.count.iter().sum()
    }
}

/// The probes frame `frame` updates within `budget`: each grid its share,
/// in turn, so that every probe of a grid is updated once every
/// `PROBES / count` frames.
pub fn plan(frame: u64, budget: u32) -> Plan {
    let shares: u32 = SHARES.iter().sum();
    let mut p = Plan::default();
    let mut offset = 0;
    for c in 0..CASCADES {
        let count = (u64::from(budget) * u64::from(SHARES[c]) / u64::from(shares)).min(u64::from(PROBES)) as u32;
        p.count[c] = count;
        p.first[c] = ((frame % u64::from(PROBES)) * u64::from(count) % u64::from(PROBES)) as u32;
        p.offset[c] = offset;
        offset += count;
    }
    p
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct CascadeGpu {
    pub origin: [i32; 4],
    pub old_origin: [i32; 4],
    pub spacing: [f32; 4],
    pub plan: [u32; 4],
}

/// `GiUniforms` of gi_common.wgsl (its size is checked against naga's).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct GiUniforms {
    pub cascades: [CascadeGpu; CASCADES],
    pub dims: [u32; 4],
    pub atlas: [f32; 4],
    pub frame: [u32; 4],
    pub rotation: [[f32; 4]; 3],
}

/// The uniforms of a frame whose grids start at `origins` (at `old` last
/// frame), updating `plan` with the rays turned by `rotation`.
pub fn uniforms(origins: [IVec3; CASCADES], old: [IVec3; CASCADES], plan: &Plan, rotation: &Mat3, enabled: bool) -> GiUniforms {
    let cascades = std::array::from_fn(|c| CascadeGpu {
        origin: origins[c].extend(0).to_array(),
        old_origin: old[c].extend(0).to_array(),
        spacing: [SPACING[c], 1.0 / SPACING[c], SPACING[c] * FAR_SPACINGS, 0.0],
        plan: [plan.first[c], plan.count[c], plan.offset[c], 0],
    });
    let (irr, depth) = (atlas_size(IRR_TILE).as_vec2(), atlas_size(DEPTH_TILE).as_vec2());
    let col = |v: Vec3| [v.x, v.y, v.z, 0.0];
    GiUniforms {
        cascades,
        dims: [DIMS.x, DIMS.y, DIMS.z, PROBES],
        atlas: [1.0 / irr.x, 1.0 / irr.y, 1.0 / depth.x, 1.0 / depth.y],
        frame: [plan.total(), u32::from(enabled), 0, 0],
        rotation: [col(rotation.x_axis), col(rotation.y_axis), col(rotation.z_axis)],
    }
}

/// The volume's frames on the CPU: where the grids are, and which frame.
#[derive(Debug, Default)]
pub struct Clipmap {
    origins: Option<[IVec3; CASCADES]>,
    frame: u64,
}

impl Clipmap {
    /// Forgets the grids: the next frame resets every probe (a teleport, a
    /// region change, the volume turned on).
    pub fn reset(&mut self) {
        self.origins = None;
    }

    /// The frames advanced so far.
    pub fn frame(&self) -> u64 {
        self.frame
    }

    /// This frame's uniforms for a camera at `camera`. A camera position
    /// that is not finite keeps the grids where they were.
    pub fn advance(&mut self, camera: Vec3, budget: u32, enabled: bool) -> GiUniforms {
        let origins = match (camera.is_finite(), self.origins) {
            (true, _) => std::array::from_fn(|c| origin(camera, c)),
            (false, Some(o)) => o,
            (false, None) => std::array::from_fn(|c| origin(Vec3::ZERO, c)),
        };
        // a grid that was nowhere: every probe entered
        let old = self.origins.unwrap_or(origins.map(|o| o + DIMS.as_ivec3() * 2));
        let u = uniforms(origins, old, &plan(self.frame, budget), &rotation(self.frame), enabled);
        self.origins = Some(origins);
        self.frame += 1;
        u
    }
}

/// Video memory of the volume with a ray buffer for `budget` probes.
pub fn memory_bytes(budget: u32) -> u64 {
    let probes = u64::from(PROBES) * CASCADES as u64;
    let texels = |n: u32| u64::from(n * n);
    let history = probes * (texels(IRR_INTERIOR) + texels(DEPTH_INTERIOR)) * 8;
    let atlas = |s: UVec2| u64::from(s.x) * u64::from(s.y) * 8;
    // (the states and the offsets: 4 and 16 bytes a probe)
    history + atlas(atlas_size(IRR_TILE)) + atlas(atlas_size(DEPTH_TILE)) + probes * 20 + u64::from(budget * RAYS) * 16
}

/// The volume's GPU side: the rays, the history of every texel, the
/// probes' states, the two atlases, and the update passes.
pub struct GiVolume {
    pub uniforms: wgpu::Buffer,
    /// What the tracer writes: [`RAYS`] `vec4<f32>` per entry of the update
    /// list (rgb radiance, w distance, negative for a miss).
    pub rays: wgpu::Buffer,
    pub state: wgpu::Buffer,
    /// Each probe's move from its grid point (xyz, m; `relocate`).
    pub offsets: wgpu::Buffer,
    pub irr_atlas: wgpu::Texture,
    pub depth_atlas: wgpu::Texture,
    /// The read of the volume ([`bound_sample_source`]'s group
    /// [`SAMPLE_GROUP`]): uniforms, states, both atlases, a linear sampler.
    pub sample_layout: wgpu::BindGroupLayout,
    pub sample_group: wgpu::BindGroup,
    budget: u32,
    bind_group: wgpu::BindGroup,
    reset: wgpu::ComputePipeline,
    blend_irr: wgpu::ComputePipeline,
    blend_depth: wgpu::ComputePipeline,
    settle: wgpu::ComputePipeline,
}

impl GiVolume {
    pub fn new(device: &wgpu::Device, budget: u32) -> GiVolume {
        let probes = u64::from(PROBES) * CASCADES as u64;
        let buffer = |label: &str, size: u64, usage: wgpu::BufferUsages| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: size.max(16),
                usage,
                mapped_at_creation: false,
            })
        };
        let storage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC;
        let uniforms = buffer(
            "gi uniforms",
            std::mem::size_of::<GiUniforms>() as u64,
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        );
        let rays = buffer("gi rays", u64::from(budget * RAYS) * 16, storage);
        let irr_hist = buffer(
            "gi irradiance history",
            probes * u64::from(IRR_INTERIOR * IRR_INTERIOR) * 8,
            storage,
        );
        let depth_hist = buffer("gi depth history", probes * u64::from(DEPTH_INTERIOR * DEPTH_INTERIOR) * 8, storage);
        let state = buffer("gi probe states", probes * 4, storage);
        let offsets = buffer("gi probe offsets", probes * 16, storage);
        let atlas = |label: &str, tile: u32| {
            let size = atlas_size(tile);
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: size.x,
                    height: size.y,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: ATLAS_FORMAT,
                usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        };
        let irr_atlas = atlas("gi irradiance atlas", IRR_TILE);
        let depth_atlas = atlas("gi depth atlas", DEPTH_TILE);
        let entry = |binding: u32, ty: wgpu::BindingType| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty,
            count: None,
        };
        let buf = |ty: wgpu::BufferBindingType| wgpu::BindingType::Buffer {
            ty,
            has_dynamic_offset: false,
            min_binding_size: None,
        };
        let out = wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format: ATLAS_FORMAT,
            view_dimension: wgpu::TextureViewDimension::D2,
        };
        let rw = wgpu::BufferBindingType::Storage { read_only: false };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gi update"),
            entries: &[
                entry(0, buf(wgpu::BufferBindingType::Uniform)),
                entry(1, buf(wgpu::BufferBindingType::Storage { read_only: true })),
                entry(2, buf(rw)),
                entry(3, buf(rw)),
                entry(4, buf(rw)),
                entry(5, out),
                entry(6, out),
                entry(7, buf(rw)),
            ],
        });
        let irr_view = irr_atlas.create_view(&Default::default());
        let depth_view = depth_atlas.create_view(&Default::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gi update"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: rays.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: irr_hist.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: depth_hist.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: state.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(&irr_view),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(&depth_view),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: offsets.as_entire_binding(),
                },
            ],
        });
        let sample_layout = sample_layout(device);
        let sampler = sample_sampler(device);
        let sample_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gi sample"),
            layout: &sample_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: state.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&irr_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&depth_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: offsets.as_entire_binding(),
                },
            ],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gi update"),
            source: wgpu::ShaderSource::Wgsl(update_source().into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gi update"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |entry: &str| {
            device.create_compute_pipeline_cached(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        GiVolume {
            reset: pipeline("cs_reset"),
            blend_irr: pipeline("cs_blend_irradiance"),
            blend_depth: pipeline("cs_blend_depth"),
            settle: pipeline("cs_settle"),
            uniforms,
            rays,
            state,
            offsets,
            irr_atlas,
            depth_atlas,
            sample_layout,
            sample_group,
            budget,
            bind_group,
        }
    }

    /// The probes one frame's rays hold room for.
    pub fn budget(&self) -> u32 {
        self.budget
    }

    /// Writes the frame's uniforms. A list longer than the rays' room is
    /// cut to it: the probes past it wait for their next turn.
    pub fn write(&self, queue: &wgpu::Queue, u: &GiUniforms) {
        queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&self.clamped(u)));
    }

    /// `u` with its list cut to the rays' room, as [`GiVolume::write`]
    /// writes it (for a caller that writes the uniforms its own way).
    pub fn clamped(&self, u: &GiUniforms) -> GiUniforms {
        let mut u = *u;
        u.frame[0] = u.frame[0].min(self.budget);
        u
    }

    /// The frame's update, after the tracer wrote the rays of `u`'s list
    /// (written with [`GiVolume::write`] first).
    /// `timestamps`: the profile's, around the pass.
    pub fn encode(&self, encoder: &mut wgpu::CommandEncoder, u: &GiUniforms, timestamps: Option<wgpu::ComputePassTimestampWrites>) {
        let updates = u.frame[0].min(self.budget);
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("gi update"),
            timestamp_writes: timestamps,
        });
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_pipeline(&self.reset);
        pass.dispatch_workgroups((PROBES * CASCADES as u32).div_ceil(64), 1, 1);
        if updates > 0 {
            pass.set_pipeline(&self.blend_depth);
            pass.dispatch_workgroups(updates, 1, 1);
            pass.set_pipeline(&self.blend_irr);
            pass.dispatch_workgroups(updates, 1, 1);
            pass.set_pipeline(&self.settle);
            pass.dispatch_workgroups(updates.div_ceil(64), 1, 1);
        }
    }
}

#[cfg(test)]
mod tests;
