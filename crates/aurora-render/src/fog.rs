//! Aurora Weather's fog (an Aurora feature: the Legacy Viewer has the sky's
//! haze alone). Fog that pools low, drifts with the wind and scatters the
//! light: god rays through its shadows, a glow around every light in it.
//!
//! - **Off** (`FrameParams::fog` None): nothing here exists and the shaders
//!   take their haze alone (`weather_fog` returns at once on a uniform).
//! - **Height** ([`FogTier::Height`]): each surface works its fog out from
//!   the height alone (common.wgsl `fog_segment`), and the sky through it
//!   (`fs_fog_sky`). No volume, no pass but the sky's triangle.
//! - **Volumetric**: a froxel volume on the camera's frustum ([`grid`]),
//!   its slices spaced exponentially in depth. Two compute passes after the
//!   volumetric clouds, before scene pass A (fog.wgsl): `inject` (density,
//!   lighting by the sky and the sun's shadow map, the last frame's value
//!   reprojected and blended), then `integrate` (front to back). The main
//!   view's surfaces, blended faces and particles included, read the result
//!   (`fog_volume`, binding 33 of the frame group); the reflections and
//!   probes keep the height fog. Past the volume's far end the height fog
//!   goes on, from the same density.
//! - **Volumetric + local lights**: `inject` walks the local lights of each
//!   froxel's screen tile and depth (light_tiles.rs), as the surfaces do.
//!
//! Its density is the slider's, scaled by the region sky's haze when the
//! card says so; it falls with the height over the ground it pools on (the
//! viewer's `FogGround`: the terrain and the water, smoothed: it fills the
//! valleys and thins on the hills), is shaped by a tiling noise anchored to
//! the grid and carried by the wind, and keeps none of itself under cover
//! (the weather's shelter map, drawn while the fog is on).
//! Under the water the water's fog alone is drawn.

use crate::pipeline_cache::CachedPipelines;
use crate::types::{FOG_GROUND_SIZE, FogGround, FogLook, FogTier, SkyParams};
use bytemuck::{Pod, Zeroable};
use glam::{IVec3, Mat4, Vec2, Vec3, Vec4};
use std::sync::{Once, OnceLock};

/// The noise's tile (m). Must match fog.wgsl `FOG_NOISE_PERIOD`; the
/// viewer's drift wraps on a multiple of it.
pub const NOISE_PERIOD: f64 = 96.0;
/// The noise texture's side (texels).
const NOISE_SIZE: u32 = 64;
/// How fast the noise rises (m/s): over a UTC day, a whole number of tiles
/// (`0.05 * 86400 = 45 * 96`), so midnight shows no jump.
const NOISE_RISE: f32 = 0.05;
/// How much the noise shapes the fog (fog.wgsl `fp.noise.w`).
const NOISE_STRENGTH: f32 = 0.85;
/// The volume's nearest depth (m).
const NEAR: f32 = 0.5;
/// The share of the ground density left high up (Frame `fog_look[0].w`).
const HIGH_SHARE: f32 = 0.02;
/// The share of a new frame in the volume's history, still and moving.
const HISTORY_STILL: f32 = 0.08;
// (0.6 left so little of the history in a quick zoom that the slices'
// steps over a sloping fog layer showed as bands, on the grid; the walls
// keep their own rule, fog.wgsl `inject`)
const HISTORY_FAST: f32 = 0.35;
/// The integrated volume's format (and the scatter's).
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// Must match `shaders/fog.wgsl`'s workgroups.
const GROUP: u32 = 8;

/// The fog's flow grid (`Flow`): cells across, along and up, and a cell's
/// side (m). 48 x 48 x 16 m around the camera; past it the shelter map
/// alone says what is covered.
pub const FLOW_DIMS: [u32; 3] = [96, 96, 32];
pub const FLOW_CELL: f32 = 0.5;
/// The grid follows the camera in steps of this many metres (whole cells).
const FLOW_SNAP: f32 = 2.0;
/// The grid's bottom below the camera (m): the fog lies low.
const FLOW_BELOW: f32 = 8.0;
/// Steps of the flow a frame: the fog goes this many cells further each
/// frame (a room 10 m deep fills in five frames).
const FLOW_STEPS: u32 = 4;
// (even: the flow's last step writes the texture the scene reads,
// `flow_view`)
const _: () = assert!(FLOW_STEPS.is_multiple_of(2));
// (the coarse walls halve every side)
const _: () = assert!(FLOW_DIMS[0].is_multiple_of(2) && FLOW_DIMS[1].is_multiple_of(2) && FLOW_DIMS[2].is_multiple_of(2));
/// The walls are drawn again every this many frames, and whenever the grid
/// moves (a door opened shows within them).
const VOXEL_EVERY: u32 = 4;
/// The walls' drawing: two pixels a cell (thin faces at a slant leave no
/// gaps), the top view square, the side views as tall as the grid.
const VOXEL_PX: u32 = FLOW_DIMS[0] * 2;
const VOXEL_PX_UP: u32 = FLOW_DIMS[2] * 2;

/// `FOG_MODE_*` of common.wgsl (`FOG_MODE_OFF` is the zeroed uniform's).
pub(crate) const MODE_HEIGHT: f32 = 1.0;
pub(crate) const MODE_VOLUME: f32 = 2.0;
/// Added to the mode: the shelter map is drawn for the view's height fog
/// (common.wgsl `FOG_MODE_SHELTER`).
pub(crate) const MODE_SHELTER: f32 = 4.0;
/// Added to the main view's mode: the sun's and the moon's rays alone
/// (common.wgsl `FOG_MODE_RAYS`): the volume, nothing past it.
pub(crate) const MODE_RAYS: f32 = 8.0;

/// The rays' air at full strength (`FogLook::rays` 1): what it takes of the
/// light through it (1/m: 5 % over the rays' 128 m), and what it scatters
/// of the sun's (1/m, fog.wgsl `fp.look.w`), far more. A scattering above
/// the extinction is no physical air: it shows the light's shafts without
/// the veil a fog thick enough to show them would lay over the scene. (The
/// fog demo's sunset, its sun 5° up, showed nothing at 0.004 and a veil at
/// 0.2, 2026-11-27.)
const RAYS_EXTINCTION: f32 = 0.0004;
const RAYS_SCATTER: f32 = 0.1;
/// Their lean toward the light (Henyey-Greenstein g): bright looking toward
/// the sun, faint across its shafts, none away from it.
const RAYS_ANISOTROPY: f32 = 0.6;

/// The volume's froxels (x, y, slices) and its far end (m) for the
/// weather's quality (0 low, 1 medium, 2 high).
pub fn grid(quality: u32) -> ([u32; 3], f32) {
    match quality {
        0 => ([96, 54, 48], 96.0),
        1 => ([128, 72, 64], 128.0),
        // (96 slices: a fog layer seen from above at 30 m showed the steps
        // of 64 in a quick zoom)
        _ => ([160, 90, 96], 160.0),
    }
}

/// Extinction at the ground (1/m) for the density slider (0..1): 0.4, the
/// default, sees about 150 m; 1 about 25 m; 0 a faint mist.
pub fn base_extinction(density: f32) -> f32 {
    let d = density.clamp(0.0, 1.0);
    0.0015 + 0.1185 * d * d
}

/// The region sky's haze against the default sky's (the Legacy haze's
/// extinction: haze density times its density and distance multipliers),
/// softened: a hazy sky gives a thicker fog, a clear one a lighter.
pub fn sky_factor(sky: &SkyParams) -> f32 {
    let d = SkyParams::default();
    let of = |s: &SkyParams| s.haze_density * s.density_multiplier * s.distance_multiplier;
    let ratio = of(sky).max(0.0) / of(&d).max(1e-12);
    if !of(sky).is_finite() || !ratio.is_finite() {
        return 1.0;
    }
    ratio.powf(0.75).clamp(0.25, 6.0)
}

/// The smoothed ground level (m) under `xy` in `ground`, bilinear; None
/// outside it or without one.
pub fn ground_level_at(ground: &FogGround, xy: Vec2) -> Option<f32> {
    let n = FOG_GROUND_SIZE as usize;
    if ground.heights.len() != n * n || ground.extent <= 0.0 || !ground.extent.is_finite() {
        return None;
    }
    let uv = (xy - Vec2::from(ground.origin)) / ground.extent;
    if !(0.0..=1.0).contains(&uv.x) || !(0.0..=1.0).contains(&uv.y) {
        return None;
    }
    let pos = (uv * n as f32 - Vec2::splat(0.5)).clamp(Vec2::ZERO, Vec2::splat((n - 1) as f32));
    let (i, f) = (pos.floor(), pos.fract());
    let (x0, y0) = (i.x as usize, i.y as usize);
    let (x1, y1) = ((x0 + 1).min(n - 1), (y0 + 1).min(n - 1));
    let h = |x: usize, y: usize| ground.heights[y * n + x][1];
    let top = h(x0, y0) + (h(x1, y0) - h(x0, y0)) * f.x;
    let bottom = h(x0, y1) + (h(x1, y1) - h(x0, y1)) * f.x;
    Some(top + (bottom - top) * f.y)
}

/// The frame uniforms' `fog_look` for `look` (common.wgsl `Frame`): the
/// height fog's terms in every view, `forward` its camera's axis, `mode`
/// the view's (`MODE_*`).
pub(crate) fn frame_terms(look: &FogLook, sky: &SkyParams, ground_level: f32, forward: Vec3, mode: f32) -> [[f32; 4]; 4] {
    let (dims, far) = grid(look.quality);
    // the rays' air: even at every height (all of it left high up), white,
    // lit by the sun or the moon alone
    let (sigma, height, high, color, lean) = if look.is_rays() {
        (RAYS_EXTINCTION * look.rays.min(1.0), 1.0, 1.0, [1.0; 3], RAYS_ANISOTROPY)
    } else {
        (
            base_extinction(look.density) * if look.from_sky { sky_factor(sky) } else { 1.0 },
            look.height.max(0.1),
            HIGH_SHARE,
            look.color.unwrap_or([1.0; 3]),
            look.anisotropy.clamp(0.0, 0.95),
        )
    };
    [
        [sigma, height, ground_level, high],
        [color[0], color[1], color[2], lean],
        [NEAR, far, dims[2] as f32, mode],
        [1.0 / (far / NEAR).ln(), forward.x, forward.y, forward.z],
    ]
}

/// The share of this frame in the volume's history: small while the
/// camera holds still (the jitter of many frames averaged), larger as it
/// turns or moves (less of the last frames, so no ghost trails behind).
pub fn history_share(turned: f32, moved: f32) -> f32 {
    let t = turned.to_degrees().abs() * 0.04 + moved.abs() * 0.05;
    (HISTORY_STILL + t).clamp(HISTORY_STILL, HISTORY_FAST)
}

/// The noise's offset (fog.wgsl `fp.noise.xyz`): the region's corner on
/// the grid within the noise's tile, less the drift, and its slow rise.
pub fn noise_offset(region_origin: [f64; 2], drift: [f32; 2], clock: f32) -> Vec3 {
    let wrap = |v: f64| v.rem_euclid(NOISE_PERIOD) as f32;
    Vec3::new(
        wrap(region_origin[0]) - drift[0],
        wrap(region_origin[1]) - drift[1],
        -(clock * NOISE_RISE).rem_euclid(NOISE_PERIOD as f32),
    )
}

/// The mean of the fog's height share (its exponential part, 1 at and
/// below `base`) along a straight climb from `za` to `zb`: common.wgsl
/// `fog_optical_depth`'s, line for line (its tests check the shader's
/// formula).
#[cfg(test)]
fn mean_height_share(za: f32, zb: f32, base: f32, h: f32) -> f32 {
    let (lo, hi) = (za.min(zb), za.max(zb));
    let dz = hi - lo;
    if dz < 1e-3 {
        return (-(lo - base).max(0.0) / h).exp();
    }
    let below = ((base - lo) / dz).clamp(0.0, 1.0);
    let (a, b) = (lo.max(base), hi.max(base));
    below + h * ((-(a - base) / h).exp() - (-(b - base) / h).exp()) / dz
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FogU {
    prev_vp: [[f32; 4]; 4],
    forward: [f32; 4],
    grid: [u32; 4],
    look: [f32; 4],
    noise: [f32; 4],
    ground: [f32; 4],
    flow: [f32; 4],
    flow_dims: [u32; 4],
}

/// object.wgsl `ShadowParams`, for a view of the walls' drawing.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct VoxelViewU {
    vp: [[f32; 4]; 4],
    anim_clock: [u32; 4],
    kind: [u32; 4],
}

/// object.wgsl `FogVoxelGrid`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct VoxelGridU {
    origin: [f32; 4],
    dims: [u32; 4],
}

/// fog.wgsl `FlowStep`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FlowStepU {
    shift: [i32; 4],
}

/// The layout of the walls' drawing's group 3 (object.wgsl `fs_fog_voxel`):
/// the view, the solid cells, the grid. The scene's pipelines and the fog
/// make it alike (equal layouts are one to wgpu).
pub(crate) fn voxel_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let uniform = |binding, visibility| wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("fog voxels"),
        entries: &[
            uniform(0, wgpu::ShaderStages::VERTEX_FRAGMENT),
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::StorageTexture {
                    access: wgpu::StorageTextureAccess::WriteOnly,
                    format: wgpu::TextureFormat::R32Uint,
                    view_dimension: wgpu::TextureViewDimension::D3,
                },
                count: None,
            },
            uniform(3, wgpu::ShaderStages::FRAGMENT),
        ],
    })
}

/// The flow grid's corner for the camera at `camera`: snapped to
/// `FLOW_SNAP`, centred across, `FLOW_BELOW` under it.
pub fn flow_origin(camera: Vec3) -> Vec3 {
    let half = Vec2::new(FLOW_DIMS[0] as f32, FLOW_DIMS[1] as f32) * FLOW_CELL * 0.5;
    let snap = |v: f32| (v / FLOW_SNAP).floor() * FLOW_SNAP;
    Vec3::new(snap(camera.x) - half.x, snap(camera.y) - half.y, snap(camera.z) - FLOW_BELOW)
}

/// The three views of the walls' drawing, the grid at `origin`: from above
/// (x, y across the picture, z its depth), along x (y, z) and along y
/// (x, z); each maps the grid's box to the clip space's, so that nothing
/// outside it is drawn.
pub fn voxel_views(origin: Vec3) -> [Mat4; 3] {
    let size = Vec3::new(FLOW_DIMS[0] as f32, FLOW_DIMS[1] as f32, FLOW_DIMS[2] as f32) * FLOW_CELL;
    let centre = origin + size * 0.5;
    // a row of the clip space: `axis` from -1 to 1 across the box
    let across = |axis: usize| {
        let mut r = Vec4::ZERO;
        r[axis] = 2.0 / size[axis];
        r.w = -2.0 * centre[axis] / size[axis];
        r
    };
    // the depth: `axis` from 0 to 1 through the box
    let through = |axis: usize| {
        let mut r = Vec4::ZERO;
        r[axis] = 1.0 / size[axis];
        r.w = -origin[axis] / size[axis];
        r
    };
    let m = |r0: Vec4, r1: Vec4, r2: Vec4| Mat4::from_cols(r0, r1, r2, Vec4::W).transpose();
    [
        m(across(0), across(1), through(2)),
        m(across(1), across(2), through(0)),
        m(across(0), across(2), through(1)),
    ]
}

/// The fog's flow (fog.wgsl `flow_step`): its grid's solid cells, drawn
/// from the scene's walls (object.wgsl `fs_fog_voxel`), and how much fog
/// each cell holds, twice (this step's and the last), kept between frames.
struct Flow {
    solid: wgpu::TextureView,
    /// The opaque walls at 1 m (fog.wgsl `walls_coarse`, `fog_light_seen`),
    /// made again after each drawing of the walls (`coarse_due`).
    walls: wgpu::TextureView,
    coarse_due: bool,
    flow: [wgpu::TextureView; 2],
    /// The walls' drawing's stand-in target.
    target: wgpu::TextureView,
    views: [wgpu::Buffer; 3],
    grid: wgpu::Buffer,
    groups: [wgpu::BindGroup; 3],
    steps: [wgpu::Buffer; 2],
    /// The grid's corner (None: not placed yet), how many cells it moved
    /// since the flow last stepped, and whether it moved since its walls
    /// were drawn.
    origin: Option<Vec3>,
    shift: IVec3,
    moved: bool,
    /// Placed afresh (no last frame's flow to read: it starts empty).
    fresh: bool,
    /// The last walls' stamp: it only grows, so that the cells an earlier
    /// drawing marked never read as walls again; frames since they were
    /// drawn, and whether they were drawn since the grid was placed.
    stamp: u32,
    since: u32,
    walls_ok: bool,
    /// The steps' groups: step k reads flow[k % 2] and writes the other
    /// (`FLOW_STEPS` is even: the last writes flow[0], which the scene and
    /// `inject` read).
    groups_steps: Vec<wgpu::BindGroup>,
}

impl Flow {
    fn new(device: &wgpu::Device, layout: &wgpu::BindGroupLayout) -> Flow {
        let tex3 = |label: &str, format: wgpu::TextureFormat| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: FLOW_DIMS[0],
                        height: FLOW_DIMS[1],
                        depth_or_array_layers: FLOW_DIMS[2],
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D3,
                    format,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let solid = tex3("fog walls", wgpu::TextureFormat::R32Uint);
        let walls = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("fog walls coarse"),
                size: wgpu::Extent3d {
                    width: FLOW_DIMS[0] / 2,
                    height: FLOW_DIMS[1] / 2,
                    depth_or_array_layers: FLOW_DIMS[2] / 2,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D3,
                format: wgpu::TextureFormat::R32Uint,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default());
        let flow = [tex3("fog flow 0", FORMAT), tex3("fog flow 1", FORMAT)];
        let target = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("fog walls target"),
                size: wgpu::Extent3d {
                    width: VOXEL_PX,
                    height: VOXEL_PX,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&Default::default());
        let buffer = |label: &str, size: usize| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: size as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let views = std::array::from_fn(|_| buffer("fog walls view", std::mem::size_of::<VoxelViewU>()));
        let grid = buffer("fog walls grid", std::mem::size_of::<VoxelGridU>());
        let steps = std::array::from_fn(|_| buffer("fog flow step", std::mem::size_of::<FlowStepU>()));
        let groups = std::array::from_fn(|i: usize| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("fog walls"),
                layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: views[i].as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&solid),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: grid.as_entire_binding(),
                    },
                ],
            })
        });
        Flow {
            solid,
            walls,
            coarse_due: false,
            flow,
            target,
            views,
            grid,
            groups,
            steps,
            origin: None,
            shift: IVec3::ZERO,
            moved: false,
            fresh: true,
            stamp: 0,
            since: VOXEL_EVERY,
            walls_ok: false,
            groups_steps: Vec::new(),
        }
    }
}

/// What the scene's pass needs to draw the fog's walls this frame
/// (renderer.rs, with the shelter map's draws): the target, and each view's
/// group and viewport (width, height).
pub struct WallViews<'a> {
    pub target: &'a wgpu::TextureView,
    pub views: [(&'a wgpu::BindGroup, (u32, u32)); 3],
}

/// The volume's textures for one grid: the scatter (density and light)
/// twice, this frame's and the last frame's, and the integrated volume the
/// scene reads.
struct Targets {
    dims: [u32; 3],
    scatter: [wgpu::TextureView; 2],
    integrated: wgpu::TextureView,
}

/// The camera of the last frame the volume was made for.
#[derive(Clone, Copy)]
struct Last {
    /// Its view-projection of positions relative to its camera.
    vp: Mat4,
    camera: Vec3,
    forward: Vec3,
    dims: [u32; 3],
    /// It was the rays' air (`FogLook::is_rays`): the fog's history is
    /// not the rays', nor theirs the fog's.
    rays: bool,
}

pub struct Fog {
    inject: wgpu::ComputePipeline,
    integrate: wgpu::ComputePipeline,
    module: wgpu::ShaderModule,
    frame_layout: wgpu::BindGroupLayout,
    inject_layout: wgpu::BindGroupLayout,
    integrate_layout: wgpu::BindGroupLayout,
    sky_layout: wgpu::BindGroupLayout,
    sky_pipelines: Vec<((u32, Option<wgpu::TextureFormat>), wgpu::RenderPipeline)>,
    sky_group: Option<wgpu::BindGroup>,
    params: wgpu::Buffer,
    noise: wgpu::TextureView,
    repeat: wgpu::Sampler,
    ground: wgpu::Texture,
    ground_view: wgpu::TextureView,
    /// The ground uploaded (its corner and side; None: none yet), and its
    /// heights kept, to know a new map from it (`Arc::ptr_eq`: an address
    /// alone could be reused by the next map).
    ground_at: Option<([f32; 2], f32)>,
    ground_kept: Option<std::sync::Arc<[[f32; 2]]>>,
    targets: Option<Targets>,
    /// The scatter written last (index into `Targets::scatter`).
    flip: usize,
    last: Option<Last>,
    frame: u32,
    flow: Flow,
    flow_layout: wgpu::BindGroupLayout,
    flow_step: wgpu::ComputePipeline,
    walls_coarse: wgpu::ComputePipeline,
}

impl Fog {
    /// `noise` the noise's mip levels (`noise_ready`).
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, noise: &[Vec<u8>]) -> Fog {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fog"),
            source: wgpu::ShaderSource::Wgsl(format!("{}\n{}", crate::common_shader_source(), include_str!("shaders/fog.wgsl")).into()),
        });
        let cs = wgpu::ShaderStages::COMPUTE;
        let tex = |binding, visibility, sample_type, view_dimension| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Texture {
                sample_type,
                view_dimension,
                multisampled: false,
            },
            count: None,
        };
        let uniform = |binding, visibility| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let sampler = |binding, visibility, kind| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Sampler(kind),
            count: None,
        };
        let storage = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: cs,
            ty: wgpu::BindingType::StorageTexture {
                access: wgpu::StorageTextureAccess::WriteOnly,
                format: FORMAT,
                view_dimension: wgpu::TextureViewDimension::D3,
            },
            count: None,
        };
        let float = wgpu::TextureSampleType::Float { filterable: true };
        let unfiltered = wgpu::TextureSampleType::Float { filterable: false };
        let d2 = wgpu::TextureViewDimension::D2;
        let d3 = wgpu::TextureViewDimension::D3;
        // group 0: the frame's uniforms and shadow atlas, at their numbers
        // in common.wgsl
        let frame_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fog frame"),
            entries: &[
                uniform(0, cs),
                tex(1, cs, wgpu::TextureSampleType::Depth, d2),
                sampler(2, cs, wgpu::SamplerBindingType::Comparison),
                sampler(4, cs, wgpu::SamplerBindingType::Filtering),
            ],
        });
        let inject_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fog inject"),
            entries: &[
                uniform(0, cs),
                tex(1, cs, float, d3),
                sampler(2, cs, wgpu::SamplerBindingType::Filtering),
                tex(3, cs, unfiltered, d2),
                tex(4, cs, float, d3),
                storage(5),
                tex(8, cs, float, d3),
                tex(15, cs, wgpu::TextureSampleType::Uint, d3),
            ],
        });
        let flow_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fog flow"),
            entries: &[
                uniform(0, cs),
                tex(3, cs, unfiltered, d2),
                tex(10, cs, wgpu::TextureSampleType::Uint, d3),
                tex(11, cs, unfiltered, d3),
                storage(12),
                uniform(13, cs),
                wgpu::BindGroupLayoutEntry {
                    binding: 14,
                    visibility: cs,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::R32Uint,
                        view_dimension: d3,
                    },
                    count: None,
                },
            ],
        });
        let integrate_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fog integrate"),
            entries: &[uniform(0, cs), tex(6, cs, unfiltered, d3), storage(7)],
        });
        let fs = wgpu::ShaderStages::FRAGMENT;
        let sky_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fog sky"),
            entries: &[
                uniform(0, fs),
                sampler(4, fs, wgpu::SamplerBindingType::Filtering),
                tex(33, fs, float, d3),
            ],
        });
        let pipeline_layout = |second: &wgpu::BindGroupLayout| {
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("fog"),
                bind_group_layouts: &[Some(&frame_layout), Some(second)],
                immediate_size: 0,
            })
        };
        let make = |entry: &'static str, layout: &wgpu::PipelineLayout| {
            device.create_compute_pipeline_cached(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(layout),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let inject = make("inject", &pipeline_layout(&inject_layout));
        let integrate = make("integrate", &pipeline_layout(&integrate_layout));
        let flow_step = make("flow_step", &pipeline_layout(&flow_layout));
        let walls_coarse = make("walls_coarse", &pipeline_layout(&flow_layout));
        let flow = Flow::new(device, &voxel_layout(device));
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fog params"),
            size: std::mem::size_of::<FogU>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let noise = noise_texture(device, queue, noise);
        let repeat = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("fog noise"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let ground = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("fog ground"),
            size: wgpu::Extent3d {
                width: FOG_GROUND_SIZE,
                height: FOG_GROUND_SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rg32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let ground_view = ground.create_view(&Default::default());
        Fog {
            inject,
            integrate,
            module,
            frame_layout,
            inject_layout,
            integrate_layout,
            sky_layout,
            sky_pipelines: Vec::new(),
            sky_group: None,
            params,
            noise,
            repeat,
            ground,
            ground_view,
            ground_at: None,
            ground_kept: None,
            targets: None,
            flip: 0,
            last: None,
            frame: 0,
            flow,
            flow_layout,
            flow_step,
            walls_coarse,
        }
    }

    /// The flow grid placed for the camera at `camera` (its corner snapped,
    /// `flow_origin`): before the frame's uniforms are made, so that the
    /// scene, the flow and `inject` read it at the same place.
    pub fn place_flow(&mut self, camera: Vec3) {
        let f = &mut self.flow;
        let origin = flow_origin(camera);
        if f.origin == Some(origin) {
            return;
        }
        match f.origin {
            Some(old) => f.shift += ((origin - old) / FLOW_CELL).round().as_ivec3(),
            None => {
                f.fresh = true;
                f.walls_ok = false;
            }
        }
        f.origin = Some(origin);
        f.moved = true;
    }

    /// Whether the fog's walls are to be drawn this frame (the grid moved,
    /// or `VOXEL_EVERY` frames passed): when they are, their views and stamp
    /// are written (`anim_clock` the frame's, for animated textures) and
    /// `walls` gives what to draw them with.
    pub fn walls_due(&mut self, queue: &wgpu::Queue, anim_clock: [u32; 4]) -> bool {
        let f = &mut self.flow;
        let Some(origin) = f.origin else {
            return false;
        };
        f.since += 1;
        if !f.moved && f.since < VOXEL_EVERY {
            return false;
        }
        f.moved = false;
        f.since = 0;
        f.stamp = f.stamp.wrapping_add(1).max(1);
        f.walls_ok = true;
        f.coarse_due = true;
        for (buffer, vp) in f.views.iter().zip(voxel_views(origin)) {
            let u = VoxelViewU {
                vp: vp.to_cols_array_2d(),
                anim_clock,
                kind: [2, 0, 0, 0],
            };
            queue.write_buffer(buffer, 0, bytemuck::bytes_of(&u));
        }
        let g = VoxelGridU {
            origin: origin.extend(FLOW_CELL).to_array(),
            dims: [FLOW_DIMS[0], FLOW_DIMS[1], FLOW_DIMS[2], f.stamp],
        };
        queue.write_buffer(&f.grid, 0, bytemuck::bytes_of(&g));
        true
    }

    /// The flow as the scene reads it (binding 34 of the main view's frame
    /// group): its steps are even, so the last lands in the same texture
    /// each frame and the group stays bound to it.
    pub fn flow_view(&self) -> &wgpu::TextureView {
        &self.flow.flow[0]
    }

    /// The ground the fog pools on, as the scene reads it (binding 35 of
    /// every frame group: the far fog and the height fog lie on it), and its
    /// square (corner, side; None: none uploaded).
    pub fn ground_view(&self) -> &wgpu::TextureView {
        &self.ground_view
    }

    pub fn ground_rect(&self) -> Option<[f32; 3]> {
        self.ground_at.map(|(origin, extent)| [origin[0], origin[1], extent])
    }

    /// The flow grid's corner, when its walls were drawn since it was placed.
    pub fn flow_grid(&self) -> Option<Vec3> {
        self.flow.origin.filter(|_| self.flow.walls_ok)
    }

    /// What to draw the fog's walls with (after `walls_due`).
    pub fn walls(&self) -> WallViews<'_> {
        let f = &self.flow;
        let [a, b, c] = &f.groups;
        WallViews {
            target: &f.target,
            views: [
                (a, (VOXEL_PX, VOXEL_PX)),
                (b, (VOXEL_PX, VOXEL_PX_UP)),
                (c, (VOXEL_PX, VOXEL_PX_UP)),
            ],
        }
    }

    /// The volume's textures are made for `quality`'s grid: true when they
    /// were made again (the frame groups then bind the new volume).
    pub fn ensure_targets(&mut self, device: &wgpu::Device, quality: u32) -> bool {
        let (dims, _) = grid(quality);
        if self.targets.as_ref().is_some_and(|t| t.dims == dims) {
            return false;
        }
        let make = |label: &str, usage: wgpu::TextureUsages| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: dims[0],
                        height: dims[1],
                        depth_or_array_layers: dims[2],
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D3,
                    format: FORMAT,
                    usage,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let usage = wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING;
        self.targets = Some(Targets {
            dims,
            scatter: [make("fog scatter 0", usage), make("fog scatter 1", usage)],
            integrated: make("fog volume", usage),
        });
        self.last = None;
        true
    }

    /// The volume's textures are let go (the tier went down to height fog):
    /// true when there were some.
    pub fn release_targets(&mut self) -> bool {
        self.last = None;
        self.targets.take().is_some()
    }

    /// The integrated volume the main view's frame groups bind, if made.
    pub fn volume(&self) -> Option<&wgpu::TextureView> {
        self.targets.as_ref().map(|t| &t.integrated)
    }

    /// The ground the fog pools on, uploaded when it changed.
    pub fn upload_ground(&mut self, queue: &wgpu::Queue, ground: &FogGround) {
        let n = (FOG_GROUND_SIZE * FOG_GROUND_SIZE) as usize;
        if ground.heights.len() != n || ground.extent.is_nan() || ground.extent <= 0.0 {
            self.ground_at = None;
            self.ground_kept = None;
            return;
        }
        let key = (ground.origin, ground.extent);
        if self.ground_at == Some(key)
            && self
                .ground_kept
                .as_ref()
                .is_some_and(|k| std::sync::Arc::ptr_eq(k, &ground.heights))
        {
            return;
        }
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.ground,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&ground.heights),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(FOG_GROUND_SIZE * 8),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: FOG_GROUND_SIZE,
                height: FOG_GROUND_SIZE,
                depth_or_array_layers: 1,
            },
        );
        self.ground_at = Some(key);
        self.ground_kept = Some(ground.heights.clone());
    }

    /// Records the volume for this frame: `frame_buffer` the main view's
    /// uniforms (their `fog_look` set), `view` and `proj` its camera,
    /// `camera` its position. A `still` (a photo, a tile of one) neither
    /// reads nor keeps the live history: each tile has its own frustum, and
    /// the live view must not take a tile's fog after it.
    #[allow(clippy::too_many_arguments)]
    pub fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        frame_buffer: &wgpu::Buffer,
        shadow: &wgpu::TextureView,
        shadow_sampler: &wgpu::Sampler,
        lin_clamp: &wgpu::Sampler,
        look: &FogLook,
        view: Mat4,
        proj: Mat4,
        camera: Vec3,
        still: bool,
    ) {
        let Some(t) = &self.targets else {
            return;
        };
        let dims = t.dims;
        let rotation = Mat4::from_mat3(glam::Mat3::from_mat4(view));
        let vp = proj * rotation;
        let forward = crate::light_tiles::view_forward(view);
        // the last frame's clip space of a position relative to this camera
        let rays = look.is_rays();
        let (prev_vp, share) = match self.last.filter(|l| l.dims == dims && l.rays == rays && !still) {
            Some(l) => (
                l.vp * Mat4::from_translation(camera - l.camera),
                history_share(l.forward.angle_between(forward), (camera - l.camera).length()),
            ),
            None => (Mat4::IDENTITY, 1.0),
        };
        self.frame = self.frame.wrapping_add(1);
        let noise = noise_offset(look.region_origin, look.drift, look.clock);
        let ground = match self.ground_at {
            Some((origin, extent)) => [origin[0], origin[1], extent, 0.0],
            None => [0.0; 4],
        };
        let u = FogU {
            prev_vp: prev_vp.to_cols_array_2d(),
            // (the golden ratio's sequence: each frame's slice jitter
            // fills the gaps of the ones before)
            forward: forward
                .extend(if still { 0.5 } else { (self.frame as f32 * 0.618_034).fract() })
                .to_array(),
            grid: [dims[0], dims[1], dims[2], self.frame],
            look: [
                share,
                if look.tier == FogTier::VolumetricLights { 1.0 } else { 0.0 },
                // a still: its samples at its froxels' centres (no history to
                // average a jitter)
                if still { 1.0 } else { 0.0 },
                // the rays' scattering (0: the fog)
                if rays { RAYS_SCATTER * look.rays.min(1.0) } else { 0.0 },
            ],
            // (the rays' air has no wisps)
            noise: noise.extend(if rays { 0.0 } else { NOISE_STRENGTH }).to_array(),
            ground,
            flow: match self.flow_grid() {
                Some(o) => o.extend(FLOW_CELL).to_array(),
                None => [0.0; 4],
            },
            flow_dims: [FLOW_DIMS[0], FLOW_DIMS[1], FLOW_DIMS[2], self.flow.stamp],
        };
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&u));
        if still {
            self.last = None;
        } else {
            self.last = Some(Last {
                vp,
                camera,
                forward,
                dims,
                rays,
            });
        }

        let now = 1 - self.flip;
        let frame_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fog frame"),
            layout: &self.frame_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: frame_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(shadow),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(shadow_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(lin_clamp),
                },
            ],
        });
        let inject_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fog inject"),
            layout: &self.inject_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.noise),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.repeat),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&self.ground_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&t.scatter[self.flip]),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(&t.scatter[now]),
                },
                // (the flow as its steps below leave it)
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: wgpu::BindingResource::TextureView(&self.flow.flow[0]),
                },
                // (its walls: the local lights' light stops at them)
                wgpu::BindGroupEntry {
                    binding: 15,
                    resource: wgpu::BindingResource::TextureView(&self.flow.walls),
                },
            ],
        });
        // the flow's steps: the first reads the last frame's where the grid
        // moved from, or nothing where it was placed afresh
        let flowing = self.flow_grid().is_some();
        if flowing {
            let shift = std::mem::take(&mut self.flow.shift);
            let fresh = std::mem::take(&mut self.flow.fresh);
            let first = FlowStepU {
                shift: shift.extend(i32::from(fresh)).to_array(),
            };
            queue.write_buffer(&self.flow.steps[0], 0, bytemuck::bytes_of(&first));
            queue.write_buffer(&self.flow.steps[1], 0, bytemuck::bytes_of(&FlowStepU { shift: [0; 4] }));
            if self.flow.groups_steps.is_empty() {
                self.flow.groups_steps = (0..FLOW_STEPS as usize)
                    .map(|k| {
                        let from = k % 2;
                        device.create_bind_group(&wgpu::BindGroupDescriptor {
                            label: Some("fog flow"),
                            layout: &self.flow_layout,
                            entries: &[
                                wgpu::BindGroupEntry {
                                    binding: 0,
                                    resource: self.params.as_entire_binding(),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 3,
                                    resource: wgpu::BindingResource::TextureView(&self.ground_view),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 10,
                                    resource: wgpu::BindingResource::TextureView(&self.flow.solid),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 11,
                                    resource: wgpu::BindingResource::TextureView(&self.flow.flow[from]),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 12,
                                    resource: wgpu::BindingResource::TextureView(&self.flow.flow[1 - from]),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 13,
                                    resource: self.flow.steps[usize::from(k > 0)].as_entire_binding(),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 14,
                                    resource: wgpu::BindingResource::TextureView(&self.flow.walls),
                                },
                            ],
                        })
                    })
                    .collect();
            }
        }
        let integrate_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fog integrate"),
            layout: &self.integrate_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(&t.scatter[now]),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::TextureView(&t.integrated),
                },
            ],
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("fog"),
            timestamp_writes: None,
        });
        pass.set_bind_group(0, &frame_group, &[]);
        if flowing && std::mem::take(&mut self.flow.coarse_due) {
            pass.set_pipeline(&self.walls_coarse);
            pass.set_bind_group(1, &self.flow.groups_steps[0], &[]);
            pass.dispatch_workgroups(
                (FLOW_DIMS[0] / 2).div_ceil(4),
                (FLOW_DIMS[1] / 2).div_ceil(4),
                (FLOW_DIMS[2] / 2).div_ceil(4),
            );
        }
        if flowing {
            pass.set_pipeline(&self.flow_step);
            for g in &self.flow.groups_steps {
                pass.set_bind_group(1, g, &[]);
                pass.dispatch_workgroups(FLOW_DIMS[0].div_ceil(4), FLOW_DIMS[1].div_ceil(4), FLOW_DIMS[2].div_ceil(4));
            }
        }
        pass.set_pipeline(&self.inject);
        pass.set_bind_group(1, &inject_group, &[]);
        pass.dispatch_workgroups(dims[0].div_ceil(GROUP), dims[1].div_ceil(GROUP), dims[2]);
        pass.set_pipeline(&self.integrate);
        pass.set_bind_group(1, &integrate_group, &[]);
        pass.dispatch_workgroups(dims[0].div_ceil(GROUP), dims[1].div_ceil(GROUP), 1);
        drop(pass);
        self.flip = now;
    }

    /// A frame without the volume, or a region crossing: the history is
    /// dropped, the next volume starts afresh.
    pub fn skipped(&mut self) {
        self.last = None;
        self.drop_flow();
    }

    /// No flow grid: the rays' air has no walls (the sun's shafts come in
    /// through the windows), and nothing reads it.
    pub fn drop_flow(&mut self) {
        // (the flow's grid is placed again, its fog started afresh; the
        // stamp keeps growing: the old walls never read as walls again)
        self.flow.origin = None;
        self.flow.shift = IVec3::ZERO;
        self.flow.walls_ok = false;
    }

    /// Makes the sky's pipeline for the scene pass's sample count and
    /// G-buffer (`gbuf`: its format), and binds what it reads this frame:
    /// before the pass.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_sky(
        &mut self,
        device: &wgpu::Device,
        frame_buffer: &wgpu::Buffer,
        lin_clamp: &wgpu::Sampler,
        dummy_volume: &wgpu::TextureView,
        samples: u32,
        gbuf: Option<wgpu::TextureFormat>,
        color: wgpu::TextureFormat,
        depth: wgpu::TextureFormat,
    ) {
        let volume = self.targets.as_ref().map_or(dummy_volume, |t| &t.integrated);
        self.sky_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fog sky"),
            layout: &self.sky_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: frame_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(lin_clamp),
                },
                wgpu::BindGroupEntry {
                    binding: 33,
                    resource: wgpu::BindingResource::TextureView(volume),
                },
            ],
        }));
        if self.sky_pipelines.iter().any(|(k, _)| *k == (samples, gbuf)) {
            return;
        }
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fog sky"),
            bind_group_layouts: &[Some(&self.sky_layout)],
            immediate_size: 0,
        });
        // dst * T + L, the colour channels only: the scene's alpha holds the
        // glow amount (as the clouds' composite)
        let blend = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::SrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent::REPLACE,
        };
        let mut targets = vec![Some(wgpu::ColorTargetState {
            format: color,
            blend: Some(blend),
            write_mask: wgpu::ColorWrites::COLOR,
        })];
        if let Some(format) = gbuf {
            targets.push(Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::empty(),
            }));
        }
        let pipeline = device.create_render_pipeline_cached(&wgpu::RenderPipelineDescriptor {
            label: Some("fog sky"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &self.module,
                entry_point: Some("vs_fog_sky"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: depth,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: samples,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            fragment: Some(wgpu::FragmentState {
                module: &self.module,
                entry_point: Some(if gbuf.is_some() { "fs_fog_sky_g" } else { "fs_fog_sky" }),
                compilation_options: Default::default(),
                targets: &targets,
            }),
            multiview_mask: None,
            cache: None,
        });
        self.sky_pipelines.push(((samples, gbuf), pipeline));
    }

    /// Draws the sky's fog in scene pass A, after the sky and the clouds
    /// (everything drawn after covers it where it stands). `prepare_sky`
    /// was called for these `samples`.
    pub fn draw_sky(&self, pass: &mut wgpu::RenderPass<'_>, samples: u32, gbuf: Option<wgpu::TextureFormat>) {
        let (Some(group), Some((_, pipeline))) = (&self.sky_group, self.sky_pipelines.iter().find(|(k, _)| *k == (samples, gbuf))) else {
            return;
        };
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, group, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// The noise's mip levels (`noise_texels`, `noise_levels`), made once per
/// run on a thread of their own: none until they are ready. The first call
/// starts them, so turning the fog on does not hold the render thread while
/// they are made; the fog shows a few frames later instead.
pub fn noise_ready() -> Option<&'static [Vec<u8>]> {
    static LEVELS: OnceLock<Vec<Vec<u8>>> = OnceLock::new();
    static STARTED: Once = Once::new();
    let make = || noise_levels(noise_texels(NOISE_SIZE), NOISE_SIZE);
    STARTED.call_once(|| {
        let spawned = std::thread::Builder::new().name("fog noise".into()).spawn(move || {
            let _ = LEVELS.set(make());
        });
        if let Err(e) = spawned {
            log::warn!("fog noise: no thread ({e}), made here");
            let _ = LEVELS.set(make());
        }
    });
    LEVELS.get().map(Vec::as_slice)
}

/// The noise the fog is shaped by: `NOISE_SIZE`^3 texels that tile, a sum
/// of gradient noise (three octaves) and billowy cells (inverted Worley),
/// stretched to 0..1; `levels` its mip levels (`noise_ready`).
fn noise_texture(device: &wgpu::Device, queue: &wgpu::Queue, levels: &[Vec<u8>]) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("fog noise"),
        size: wgpu::Extent3d {
            width: NOISE_SIZE,
            height: NOISE_SIZE,
            depth_or_array_layers: NOISE_SIZE,
        },
        mip_level_count: levels.len() as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D3,
        format: wgpu::TextureFormat::R8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (level, texels) in levels.iter().enumerate() {
        let side = (NOISE_SIZE >> level).max(1);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: level as u32,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            texels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(side),
                rows_per_image: Some(side),
            },
            wgpu::Extent3d {
                width: side,
                height: side,
                depth_or_array_layers: side,
            },
        );
    }
    texture.create_view(&Default::default())
}

/// The noise's levels: `texels` (`size`^3, x fastest), then each the mean of
/// the 2 x 2 x 2 texels under it, down to one texel. A froxel reads the level
/// of its own size (fog.wgsl `fog_density`): far ones see the noise blurred
/// as wide as they are, not one point of it that swims as they move.
fn noise_levels(texels: Vec<u8>, size: u32) -> Vec<Vec<u8>> {
    let mut levels = vec![texels];
    let mut side = size as usize;
    while side > 1 {
        let prev = levels.last().expect("level 0 is there");
        let half = side / 2;
        let at = |x: usize, y: usize, z: usize| u32::from(prev[(z * side + y) * side + x]);
        let next = (0..half * half * half)
            .map(|i| {
                let (x, y, z) = (i % half * 2, i / half % half * 2, i / (half * half) * 2);
                let mut sum = 0;
                for k in 0..8 {
                    sum += at(x + (k & 1), y + ((k >> 1) & 1), z + (k >> 2));
                }
                ((sum + 4) / 8) as u8
            })
            .collect();
        levels.push(next);
        side = half;
    }
    levels
}

fn hash(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^ (x >> 16)
}

fn hash3(x: u32, y: u32, z: u32, seed: u32) -> u32 {
    hash(x ^ hash(y ^ hash(z ^ hash(seed))))
}

/// A unit gradient at a lattice point.
fn gradient(x: u32, y: u32, z: u32, seed: u32) -> Vec3 {
    let h = hash3(x, y, z, seed);
    let a = (h & 0xffff) as f32 / 65535.0 * std::f32::consts::TAU;
    let c = (h >> 16) as f32 / 65535.0 * 2.0 - 1.0;
    let s = (1.0 - c * c).max(0.0).sqrt();
    Vec3::new(s * a.cos(), s * a.sin(), c)
}

/// Gradient noise at `p` (lattice units) on a lattice of `period` cells
/// that wraps: about -1..1.
fn gradient_noise(p: Vec3, period: u32, seed: u32) -> f32 {
    let i = p.floor();
    let f = p - i;
    let fade = f * f * f * (f * (f * 6.0 - Vec3::splat(15.0)) + Vec3::splat(10.0));
    let wrap = |v: f32, d: u32| ((v as i64 + d as i64).rem_euclid(period as i64)) as u32;
    let mut corners = [0.0f32; 8];
    for (k, corner) in corners.iter_mut().enumerate() {
        let o = Vec3::new((k & 1) as f32, ((k >> 1) & 1) as f32, ((k >> 2) & 1) as f32);
        let g = gradient(
            wrap(i.x, k as u32 & 1),
            wrap(i.y, (k as u32 >> 1) & 1),
            wrap(i.z, (k as u32 >> 2) & 1),
            seed,
        );
        *corner = g.dot(f - o);
    }
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let x0 = lerp(corners[0], corners[1], fade.x);
    let x1 = lerp(corners[2], corners[3], fade.x);
    let x2 = lerp(corners[4], corners[5], fade.x);
    let x3 = lerp(corners[6], corners[7], fade.x);
    lerp(lerp(x0, x1, fade.y), lerp(x2, x3, fade.y), fade.z) * 1.6
}

/// The distance to the nearest of one random point per cell, on a lattice
/// of `period` cells that wraps (0 on a point, about 1 between).
fn worley(p: Vec3, period: u32, seed: u32) -> f32 {
    let i = p.floor();
    let mut best = f32::MAX;
    for dz in -1i64..=1 {
        for dy in -1i64..=1 {
            for dx in -1i64..=1 {
                let c = Vec3::new(i.x + dx as f32, i.y + dy as f32, i.z + dz as f32);
                let w = |v: f32| (v as i64).rem_euclid(period as i64) as u32;
                let h = hash3(w(c.x), w(c.y), w(c.z), seed);
                let jitter = Vec3::new(
                    (h & 0x3ff) as f32 / 1023.0,
                    ((h >> 10) & 0x3ff) as f32 / 1023.0,
                    ((h >> 20) & 0x3ff) as f32 / 1023.0,
                );
                best = best.min((c + jitter - p).length_squared());
            }
        }
    }
    best.sqrt()
}

/// The noise texture's texels (`size`^3, x fastest), 0..255: its layers
/// shared among the processor's threads (made once, when the fog first
/// shows).
fn noise_texels(size: u32) -> Vec<u8> {
    let n = size as usize;
    let value = |i: usize| {
        let p = Vec3::new((i % n) as f32, ((i / n) % n) as f32, (i / (n * n)) as f32) / size as f32;
        let mut fbm = 0.0;
        let mut amp = 0.5;
        for (octave, period) in [4u32, 8, 16].into_iter().enumerate() {
            fbm += amp * gradient_noise(p * period as f32, period, 11 + octave as u32);
            amp *= 0.5;
        }
        let cells = 1.0 - worley(p * 5.0, 5, 29).min(1.0);
        (fbm / 0.875) * 0.5 + 0.5 * 0.55 + cells * 0.45
    };
    let mut values = vec![0.0f32; n * n * n];
    let threads = std::thread::available_parallelism().map_or(4, |t| t.get()).clamp(1, 16);
    let chunk = (n * n * n).div_ceil(threads);
    std::thread::scope(|scope| {
        for (k, part) in values.chunks_mut(chunk).enumerate() {
            scope.spawn(move || {
                for (j, v) in part.iter_mut().enumerate() {
                    *v = value(k * chunk + j);
                }
            });
        }
    });
    let (lo, hi) = values.iter().fold((f32::MAX, f32::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
    let span = (hi - lo).max(1e-6);
    values.iter().map(|&v| (((v - lo) / span) * 255.0).round() as u8).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_density_sees_about_150_m_and_the_slider_climbs_steadily() {
        // three optical depths: what is left of a surface's light is 5 %
        let seen = |d: f32| 3.0 / base_extinction(d);
        assert!((120.0..180.0).contains(&seen(0.4)), "{}", seen(0.4));
        assert!(seen(1.0) < 30.0 && seen(0.0) > 1000.0);
        let mut last = 0.0;
        for i in 0..=10 {
            let e = base_extinction(i as f32 / 10.0);
            assert!(e > last);
            last = e;
        }
    }

    #[test]
    fn a_hazy_sky_thickens_the_fog_and_a_clear_one_thins_it() {
        let d = SkyParams::default();
        assert!((sky_factor(&d) - 1.0).abs() < 1e-4);
        let hazy = SkyParams { haze_density: 4.0, ..d };
        let clear = SkyParams { haze_density: 0.05, ..d };
        assert!(sky_factor(&hazy) > 2.0 && sky_factor(&hazy) <= 6.0);
        assert!(sky_factor(&clear) >= 0.25 && sky_factor(&clear) < 0.5);
        let broken = SkyParams {
            density_multiplier: f32::NAN,
            ..d
        };
        assert_eq!(sky_factor(&broken), 1.0);
    }

    #[test]
    fn the_frame_terms_give_the_volume_its_slices_and_the_shader_its_log() {
        let look = FogLook {
            tier: FogTier::Volumetric,
            quality: 2,
            density: 0.4,
            from_sky: false,
            height: 6.0,
            color: None,
            anisotropy: 0.5,
            drift: [0.0; 2],
            clock: 0.0,
            region_origin: [0.0; 2],
            rays: 0.0,
        };
        let t = frame_terms(&look, &SkyParams::default(), 21.0, Vec3::X, MODE_VOLUME);
        assert_eq!(t[0], [base_extinction(0.4), 6.0, 21.0, HIGH_SHARE]);
        assert_eq!(t[1], [1.0, 1.0, 1.0, 0.5]);
        let (dims, far) = grid(2);
        assert_eq!(t[2], [NEAR, far, dims[2] as f32, MODE_VOLUME]);
        // the slice coordinate of the far end is 1 (common.wgsl fog_volume_uvw)
        assert!(((far / NEAR).ln() * t[3][0] - 1.0).abs() < 1e-5);
        assert_eq!(t[3][1..], [1.0, 0.0, 0.0]);
        // its own colour is kept
        let own = frame_terms(
            &FogLook {
                color: Some([1.0, 0.0, 0.5]),
                ..look
            },
            &SkyParams::default(),
            0.0,
            Vec3::X,
            MODE_HEIGHT,
        );
        assert_eq!(own[1][..3], [1.0, 0.0, 0.5]);
        // the rays' air: thin, even at every height, white, whatever the
        // card's density, height, colour and the sky's haze
        let hazy = SkyParams {
            haze_density: 4.0,
            ..SkyParams::default()
        };
        let rays = FogLook {
            rays: 0.5,
            density: 1.0,
            from_sky: true,
            color: Some([1.0, 0.0, 0.5]),
            ..look
        };
        assert!(rays.is_rays() && !look.is_rays());
        let r = frame_terms(&rays, &hazy, 21.0, Vec3::X, MODE_VOLUME);
        assert_eq!(r[0], [RAYS_EXTINCTION * 0.5, 1.0, 21.0, 1.0]);
        assert_eq!(r[1], [1.0, 1.0, 1.0, RAYS_ANISOTROPY]);
        assert_eq!(r[2], t[2]);
        // a scene seen through the whole volume keeps nearly all of itself
        assert!((-r[0][0] * far / 0.5).exp() > 0.9);
        const { assert!(RAYS_SCATTER > RAYS_EXTINCTION) };
    }

    #[test]
    fn the_height_share_stays_between_zero_and_one_below_across_and_above_the_ground() {
        let numeric = |za: f32, zb: f32, base: f32, h: f32| {
            let n = 20_000;
            (0..n)
                .map(|i| {
                    let z = za + (zb - za) * (i as f32 + 0.5) / n as f32;
                    (-(z - base).max(0.0) / h).exp()
                })
                .sum::<f32>()
                / n as f32
        };
        // wholly below (the sea under the fog's ground: it was negative), across,
        // wholly above, and level
        for (za, zb) in [(10.0, 15.0), (15.0, 10.0), (10.0, 40.0), (25.0, 60.0), (30.0, 30.0)] {
            let got = mean_height_share(za, zb, 20.0, 6.0);
            assert!((0.0..=1.0).contains(&got), "{za} {zb}: {got}");
            assert!((got - numeric(za, zb, 20.0, 6.0)).abs() < 2e-3, "{za} {zb}: {got}");
        }
        assert!(include_str!("shaders/common.wgsl").contains("let b = max(hi, base);"));
    }

    #[test]
    fn the_walls_views_map_the_flow_grid_onto_clip_space() {
        let origin = flow_origin(Vec3::new(131.3, 77.9, 25.2));
        // snapped, centred across, the camera inside
        assert_eq!(origin, Vec3::new(130.0 - 24.0, 76.0 - 24.0, 24.0 - FLOW_BELOW));
        let size = Vec3::new(FLOW_DIMS[0] as f32, FLOW_DIMS[1] as f32, FLOW_DIMS[2] as f32) * FLOW_CELL;
        for (k, vp) in voxel_views(origin).iter().enumerate() {
            let lo = *vp * origin.extend(1.0);
            let hi = *vp * (origin + size).extend(1.0);
            // two corners of the box: the clip square's corners, the depth 0 and 1
            assert!(
                (lo.x + 1.0).abs() < 1e-4 && (lo.y + 1.0).abs() < 1e-4 && lo.z.abs() < 1e-4,
                "{k} {lo}"
            );
            assert!(
                (hi.x - 1.0).abs() < 1e-4 && (hi.y - 1.0).abs() < 1e-4 && (hi.z - 1.0).abs() < 1e-4,
                "{k} {hi}"
            );
            assert_eq!(lo.w, 1.0);
        }
        // the side views look along x and along y: z is the picture's height
        let [_, along_x, along_y] = voxel_views(origin);
        let up = origin + Vec3::new(0.0, 0.0, size.z);
        assert!(((along_x * up.extend(1.0)).y - 1.0).abs() < 1e-4);
        assert!(((along_y * up.extend(1.0)).y - 1.0).abs() < 1e-4);
        // the flow's shader steps cells of the grid's size, the walls' too
        // (a Windows checkout writes the shader with CRLF line ends)
        let wgsl = include_str!("shaders/fog.wgsl").replace("\r\n", "\n");
        assert!(wgsl.contains("@compute @workgroup_size(4, 4, 4)\nfn flow_step"));
        assert_eq!(std::mem::size_of::<FogU>(), 176);
    }

    #[test]
    fn the_history_keeps_more_while_still_and_less_while_moving() {
        assert_eq!(history_share(0.0, 0.0), HISTORY_STILL);
        let walking = history_share(0.0, 0.05);
        let turning = history_share(3f32.to_radians(), 0.0);
        assert!(walking > HISTORY_STILL && walking < HISTORY_FAST);
        assert!(turning > walking);
        assert_eq!(history_share(1.0, 30.0), HISTORY_FAST);
    }

    #[test]
    fn the_noise_is_anchored_to_the_grid_and_moves_with_the_drift_and_the_clock() {
        // two regions a tile apart show the same pattern at the same place
        let a = noise_offset([256_000.0, 256_000.0], [0.0; 2], 0.0);
        let b = noise_offset([256_000.0 + NOISE_PERIOD, 256_000.0], [0.0; 2], 0.0);
        assert!((a - b).length() < 1e-3);
        // the wind carries the wisps downwind (the offset goes the other way)
        let d = noise_offset([0.0; 2], [3.0, 1.0], 0.0);
        assert_eq!((d.x, d.y), (-3.0, -1.0));
        // the UTC day's last second and midnight are a tile apart: no jump
        let end = noise_offset([0.0; 2], [0.0; 2], 86_400.0 - 1e-3);
        let start = noise_offset([0.0; 2], [0.0; 2], 0.0);
        let gap = (end.z - start.z).rem_euclid(NOISE_PERIOD as f32);
        assert!(gap.min(NOISE_PERIOD as f32 - gap) < 0.01, "{gap}");
    }

    #[test]
    fn the_ground_level_is_read_bilinear_and_none_outside() {
        let n = FOG_GROUND_SIZE as usize;
        let heights: Vec<[f32; 2]> = (0..n * n).map(|i| [0.0, (i % n) as f32]).collect();
        let g = FogGround {
            origin: [100.0, 200.0],
            extent: n as f32,
            heights: heights.into(),
        };
        // a cell a metre: the level is the column number at its centre
        let at = ground_level_at(&g, Vec2::new(100.0 + 10.5, 230.0)).expect("inside");
        assert!((at - 10.0).abs() < 1e-4, "{at}");
        let between = ground_level_at(&g, Vec2::new(100.0 + 11.0, 230.0)).expect("inside");
        assert!((between - 10.5).abs() < 1e-4, "{between}");
        assert!(ground_level_at(&g, Vec2::new(50.0, 230.0)).is_none());
        assert!(ground_level_at(&FogGround::default(), Vec2::ZERO).is_none());
    }

    #[test]
    fn the_noise_levels_halve_down_to_one_texel_keeping_the_mean() {
        let t = noise_texels(16);
        let mean = |v: &[u8]| v.iter().map(|&x| f64::from(x)).sum::<f64>() / v.len() as f64;
        let levels = noise_levels(t.clone(), 16);
        assert_eq!(levels.iter().map(Vec::len).collect::<Vec<_>>(), vec![4096, 512, 64, 8, 1]);
        for l in &levels {
            assert!((mean(l) - mean(&t)).abs() < 1.5, "{} {}", mean(l), mean(&t));
        }
    }

    #[test]
    fn the_noise_tiles_and_uses_its_range() {
        let size = 16;
        let t = noise_texels(size);
        assert_eq!(t.len(), (size * size * size) as usize);
        assert_eq!((t.iter().min().copied(), t.iter().max().copied()), (Some(0), Some(255)));
        // the gradient noise wraps: its value at the lattice's far edge is
        // its value at the near one
        let p = Vec3::new(0.3, 1.7, 2.2);
        let a = gradient_noise(p, 4, 3);
        let b = gradient_noise(p + Vec3::new(4.0, 0.0, 0.0), 4, 3);
        assert!((a - b).abs() < 1e-5, "{a} {b}");
        let w0 = worley(p, 5, 9);
        let w1 = worley(p + Vec3::new(0.0, 5.0, 0.0), 5, 9);
        assert!((w0 - w1).abs() < 1e-5);
    }

    #[test]
    fn the_shader_compiles_with_common_and_matches_its_constants() {
        let src = format!("{}\n{}", crate::COMMON_SHADER, include_str!("shaders/fog.wgsl"));
        let module = naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&src)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
            .validate(&module)
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert!(src.contains(&format!("const FOG_NOISE_PERIOD: f32 = {NOISE_PERIOD:.1};")));
        assert!(include_str!("shaders/fog.wgsl").contains(&format!("@workgroup_size({GROUP}, {GROUP}, 1)")));
        // the fine octave repeats on the tile too
        assert!(include_str!("shaders/fog.wgsl").contains("q / (FOG_NOISE_PERIOD * 0.25)"));
    }
}
