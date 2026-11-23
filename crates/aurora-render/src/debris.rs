//! Aurora Weather's debris (Aurora's own; the Legacy Viewer has none):
//! leaves, sand, paper and embers blown by the wind, for this viewer only.
//!
//! A pool of particles lives on the GPU (`STRIDE` bytes each), simulated by
//! `cs_debris` (shaders/debris_sim.wgsl) once a frame after the shadows,
//! whose shelter map gives the topmost surface around the camera: what the
//! debris lands on, stops against, and the cover it never blows under. The
//! same buffer is the instance buffer of `vs_debris` (shaders/debris.wgsl,
//! in the scene module), drawn with the rain. The air that carries it is the
//! weather's wind (`RainLook::wind`), a curl-noise field for its eddies and
//! the one gust curve (`RainLook::gust`) the drops and the fog share.
//!
//! Leaves fall from foliage: the canopy map (`Canopy`, beside the shelter
//! map) marks where the topmost surface is a foliage texture, and leaves
//! start just under it; with none, nothing or a thin stream from upwind
//! (`DebrisLook::stream`). Off (`FrameParams::debris` None), nothing is made
//! or run.

use crate::pipeline_cache::CachedPipelines;
use crate::types::{DebrisKind, DebrisLook};
use bytemuck::{Pod, Zeroable};
use glam::{Vec2, Vec3};

/// A particle's bytes: four vec4 (debris_sim.wgsl's layout).
pub const STRIDE: u64 = 64;
/// The vertices of one particle's draw (a creased card's 12; the others
/// use 6 and leave the rest degenerate).
pub const VERTICES: u32 = 12;
/// How far from the camera debris lives (m): inside the shelter map's 40 m
/// half side (renderer.rs `rain_shelter_matrix`), past which nothing of the
/// ground is known (debris_sim.wgsl `REACH`).
pub const REACH: f32 = 38.0;
/// The canopy map's texels a side (`Canopy`): 80 m at about 31 cm.
pub const CANOPY_PX: u32 = 256;

/// debris_sim.wgsl `FLAG_*`.
const FLAG_STREAM: u32 = 1;
const FLAG_CANOPY: u32 = 2;

/// The pool's size at a weather quality (0 low, 1 medium, 2 high): the most
/// particles alive at once.
pub fn pool_size(quality: u32) -> u32 {
    match quality {
        0 => 2048,
        1 => 6144,
        _ => 16384,
    }
}

/// How long a particle of `kind` lasts on average (s, in the air and lying
/// before it fades; debris_sim.wgsl `ds_kind`).
fn mean_life(kind: DebrisKind) -> f32 {
    match kind {
        DebrisKind::Leaves => 30.0,
        DebrisKind::Sand => 3.0,
        DebrisKind::Paper => 32.0,
        DebrisKind::Embers => 6.0,
        DebrisKind::Motes => 30.0,
    }
}

/// Where `kind` starts, nearest and farthest from the camera (m): mostly
/// mid distance (debris_sim.wgsl adds a few close passes); sand, too fine
/// to be seen far off, streams past near the camera.
pub fn spawn_ring(kind: DebrisKind) -> (f32, f32) {
    match kind {
        DebrisKind::Sand => (1.0, 14.0),
        // motes are only seen near
        DebrisKind::Motes => (0.8, 8.0),
        _ => (8.0, REACH - 2.0),
    }
}

/// The share of the pool alive wanted: the density slider, more in the
/// wind and more again in its gusts (a still day drops few leaves).
pub fn wanted_share(look: &DebrisLook, wind: Vec2, gust: f32) -> f32 {
    let windy = (wind.length() / 8.0).clamp(0.0, 1.0);
    let base = match look.kind {
        // sand and paper only move in a wind (sand streams by near the
        // camera: a share of the pool is plenty)
        DebrisKind::Sand => (0.1 + 0.9 * windy) * 0.45,
        DebrisKind::Paper => windy,
        // motes hang in still air; a wind clears them away
        DebrisKind::Motes => 0.1 * (1.0 - 0.7 * windy),
        _ => 0.15 + 0.85 * windy,
    };
    (look.density.clamp(0.0, 1.0) * base * (0.6 + 0.8 * gust.clamp(0.0, 1.0))).clamp(0.0, 0.95)
}

/// The chance a free slot takes a new particle this frame, so that a share
/// `share` of the pool stays alive with particles lasting `life` seconds:
/// alive slots leave at `1 / life` a second, free ones fill at `p / dt`, and
/// at the balance `share / (1 - share) = (p / dt) * life`.
pub fn spawn_chance(share: f32, life: f32, dt: f32) -> f32 {
    if share <= 0.0 || life <= 0.0 || dt <= 0.0 {
        return 0.0;
    }
    let share = share.min(0.95);
    (dt * share / ((1.0 - share) * life)).clamp(0.0, 1.0)
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct DebrisU {
    camera: [f32; 4],
    wind: [f32; 4],
    look: [u32; 4],
    rates: [f32; 4],
}

pub struct Debris {
    sim: wgpu::ComputePipeline,
    frame_layout: wgpu::BindGroupLayout,
    own_layout: wgpu::BindGroupLayout,
    params: wgpu::Buffer,
    particles: wgpu::Buffer,
    /// The canopy map (debris_sim.wgsl `debris_canopy`), drawn by the
    /// renderer with the shelter map's casters; until then, empty.
    canopy_view: wgpu::TextureView,
    /// The floor under the foliage (debris_sim.wgsl `debris_floor`).
    floor_view: wgpu::TextureView,
    canopy_depth: wgpu::TextureView,
    canopy_drawn: bool,
    pool: u32,
    frame: u32,
    kind: Option<DebrisKind>,
}

impl Debris {
    pub fn new(device: &wgpu::Device, quality: u32) -> Debris {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("debris"),
            source: wgpu::ShaderSource::Wgsl(
                format!("{}\n{}", crate::common_shader_source(), include_str!("shaders/debris_sim.wgsl")).into(),
            ),
        });
        let cs = wgpu::ShaderStages::COMPUTE;
        // group 0: the frame's uniforms and shadow atlas, at their numbers in
        // common.wgsl
        let frame_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("debris frame"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: cs,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: cs,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let own_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("debris"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: cs,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: cs,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: cs,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: cs,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("debris"),
            bind_group_layouts: &[Some(&frame_layout), Some(&own_layout)],
            immediate_size: 0,
        });
        let sim = device.create_compute_pipeline_cached(&wgpu::ComputePipelineDescriptor {
            label: Some("debris"),
            layout: Some(&layout),
            module: &module,
            entry_point: Some("cs_debris"),
            compilation_options: Default::default(),
            cache: None,
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("debris params"),
            size: std::mem::size_of::<DebrisU>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let pool = pool_size(quality);
        let particles = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("debris particles"),
            size: u64::from(pool) * STRIDE,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let canopy = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("debris canopy"),
            size: wgpu::Extent3d {
                width: CANOPY_PX,
                height: CANOPY_PX,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: CANOPY_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let canopy_view = canopy.create_view(&Default::default());
        let floor_view = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("debris floor"),
                size: wgpu::Extent3d {
                    width: CANOPY_PX,
                    height: CANOPY_PX,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: FLOOR_FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&Default::default());
        let canopy_depth = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("debris canopy depth"),
                size: wgpu::Extent3d {
                    width: CANOPY_PX,
                    height: CANOPY_PX,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth32Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&Default::default());
        Debris {
            sim,
            frame_layout,
            own_layout,
            params,
            particles,
            canopy_view,
            floor_view,
            canopy_depth,
            canopy_drawn: false,
            pool,
            frame: 0,
            kind: None,
        }
    }

    /// The pool's size: a new quality makes a new `Debris`.
    pub fn pool(&self) -> u32 {
        self.pool
    }

    /// The canopy map's target and its depth, and the floor's depth, for
    /// the renderer to draw the shelter map's casters into (object.wgsl
    /// `fs_canopy`, `fs_canopy_floor`); from then on the leaves fall from
    /// it.
    pub fn canopy_targets(&mut self) -> [&wgpu::TextureView; 3] {
        self.canopy_drawn = true;
        [&self.canopy_view, &self.floor_view, &self.canopy_depth]
    }

    /// Everything goes (a region crossing: the particles' places and the
    /// canopy map are the old region's): the pool emptied at the next step,
    /// the canopy map not read until drawn again. Nothing is made again.
    pub fn reset(&mut self) {
        self.kind = None;
        self.canopy_drawn = false;
    }

    /// The particles, as the instance buffer of `vs_debris`.
    pub fn instances(&self) -> &wgpu::Buffer {
        &self.particles
    }

    /// One step of the simulation: spawn into free slots, move the rest.
    /// `wind` the weather's wind now (m/s), `gust` where it stands between
    /// its lulls and gusts, `clock` seconds, `dt` this frame's step.
    #[allow(clippy::too_many_arguments)]
    pub fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        frame_buffer: &wgpu::Buffer,
        shadow: &wgpu::TextureView,
        look: &DebrisLook,
        camera: Vec3,
        (wind, gust): (Vec2, f32),
        clock: f32,
        dt: f32,
    ) {
        if self.kind != Some(look.kind) {
            // another kind (or a reset): what was there goes at once, and
            // the canopy map waits to be drawn again
            encoder.clear_buffer(&self.particles, 0, None);
            self.kind = Some(look.kind);
            self.canopy_drawn = false;
        }
        self.frame = self.frame.wrapping_add(1);
        let dt = dt.clamp(0.0, 0.1);
        let share = wanted_share(look, wind, gust);
        let mut flags = 0;
        if look.stream {
            flags |= FLAG_STREAM;
        }
        if self.canopy_drawn {
            flags |= FLAG_CANOPY;
        }
        let u = DebrisU {
            camera: [camera.x, camera.y, camera.z, dt],
            wind: [wind.x, wind.y, gust.clamp(0.0, 1.0), clock],
            look: [look.kind.index(), self.frame, self.pool, flags],
            rates: {
                let (near, far) = spawn_ring(look.kind);
                [spawn_chance(share, mean_life(look.kind), dt), near, far, look.size.clamp(0.5, 2.0)]
            },
        };
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&u));
        let frame_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("debris frame"),
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
            ],
        });
        let own = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("debris"),
            layout: &self.own_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.particles.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&self.canopy_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&self.floor_view),
                },
            ],
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("debris"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.sim);
        pass.set_bind_group(0, &frame_group, &[]);
        pass.set_bind_group(1, &own, &[]);
        pass.dispatch_workgroups(self.pool.div_ceil(64), 1, 1);
    }
}

/// The canopy map's format: x 1 where the top is foliage, y its height (m).
pub const CANOPY_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg32Float;
/// The floor's: the depth of the topmost surface that is not foliage, on
/// the shelter map's projection (the shadow maps' format, so that the
/// shelter's terrain pipeline draws into it).
pub const FLOOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawning_keeps_the_wanted_share_alive() {
        // simulate the balance: free slots fill at p, alive ones leave at
        // dt / life
        for (share, life) in [(0.1, 30.0), (0.5, 3.0), (0.9, 5.0)] {
            let dt = 1.0 / 60.0;
            let p = spawn_chance(share, life, dt);
            let mut alive = 0.0f32;
            for _ in 0..60 * 600 {
                alive += (1.0 - alive) * p - alive * dt / life;
            }
            assert!((alive - share).abs() < 0.02, "share {share} life {life}: {alive}");
        }
        assert_eq!(spawn_chance(0.0, 10.0, 0.016), 0.0);
        assert_eq!(spawn_chance(0.5, 0.0, 0.016), 0.0);
    }

    #[test]
    fn calm_air_drops_few_leaves_and_no_sand() {
        let leaves = DebrisLook {
            kind: DebrisKind::Leaves,
            ..DebrisLook::default()
        };
        let sand = DebrisLook {
            kind: DebrisKind::Sand,
            ..DebrisLook::default()
        };
        let calm = wanted_share(&leaves, Vec2::ZERO, 0.0);
        let windy = wanted_share(&leaves, Vec2::new(8.0, 0.0), 1.0);
        assert!(calm > 0.0 && calm < windy * 0.2, "{calm} {windy}");
        // (a little sand stirs even in still air: a breeze moves some)
        assert!(wanted_share(&sand, Vec2::ZERO, 1.0) < 0.1);
        assert!(wanted_share(&sand, Vec2::new(10.0, 0.0), 0.5) > 0.15);
        // never the whole pool: slots are left to spawn into
        assert!(windy <= 0.95);
    }

    #[test]
    fn the_pool_grows_with_the_quality() {
        assert!(pool_size(0) < pool_size(1) && pool_size(1) < pool_size(2));
        assert_eq!(u64::from(pool_size(2)) * STRIDE, 1 << 20);
    }

    #[test]
    fn the_shaders_hold_the_same_numbers() {
        let sim = include_str!("shaders/debris_sim.wgsl");
        let draw = include_str!("shaders/debris.wgsl");
        let common = include_str!("shaders/common.wgsl");
        assert!(sim.contains(&format!("const REACH: f32 = {REACH:?};")), "REACH");
        assert!(sim.contains(&format!("const FLAG_STREAM: u32 = {FLAG_STREAM}u;")));
        assert!(sim.contains(&format!("const FLAG_CANOPY: u32 = {FLAG_CANOPY}u;")));
        for (kind, name) in [
            (DebrisKind::Leaves, "LEAVES"),
            (DebrisKind::Sand, "SAND"),
            (DebrisKind::Paper, "PAPER"),
            (DebrisKind::Embers, "EMBERS"),
            (DebrisKind::Motes, "MOTES"),
        ] {
            assert!(sim.contains(&format!("const DK_{name}: u32 = {}u;", kind.index())), "DK_{name}");
            assert!(draw.contains(&format!("const DD_{name}: u32 = {}u;", kind.index())), "DD_{name}");
        }
        // a particle is four vec4
        assert_eq!(STRIDE, 4 * 16);
        assert!(common.contains(&format!("const FLAG_FOLIAGE: u32 = {}u;", crate::types::flags::FOLIAGE)));
    }

    #[test]
    fn the_shaders_compile_with_common() {
        for (name, src) in [
            (
                "debris_sim",
                format!("{}\n{}", crate::COMMON_SHADER, include_str!("shaders/debris_sim.wgsl")),
            ),
            (
                "debris draw",
                format!(
                    "{}\n{}\n{}\n{}",
                    crate::COMMON_SHADER,
                    include_str!("shaders/object.wgsl"),
                    include_str!("shaders/env.wgsl"),
                    include_str!("shaders/debris.wgsl")
                ),
            ),
        ] {
            let module = naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("{name}: {}", e.emit_to_string(&src)));
            let mut v = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all());
            if let Err(e) = v.validate(&module) {
                panic!("{name}: {}", e.emit_to_string(&src));
            }
        }
    }
}
