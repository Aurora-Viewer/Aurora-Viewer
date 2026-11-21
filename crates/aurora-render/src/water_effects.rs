//! Local visual water only. The simulator remains authoritative for motion.
//! A world-anchored height/velocity/foam field runs on the GPU; no readback.

use crate::pipeline_cache::CachedPipelines;
use crate::water_sea::{Sea, SeaPoint, SwellPoint};
use bytemuck::{Pod, Zeroable};
use glam::Vec2;

pub const MAX_WATER_EVENTS: usize = 256;
pub const WATER_GROUND_SIZE: u32 = 64;
/// The sea grid (`WaterGround::sea`): cells a side and the metres it
/// covers around the camera, wider than the water field's so that shores
/// far out get their waves too. In the bathymetry texture below the 64
/// bathymetry rows.
pub const SEA_GRID: usize = 128;
/// Where the spectrum's maps start in the bathymetry texture (under the
/// two sea grids) and how many rows they take (`water_fx_ocean`).
const OCEAN_ROW: u32 = WATER_GROUND_SIZE + 2 * SEA_GRID as u32;
const OCEAN_ROWS: u32 = (crate::water_fft::OCEAN_TILES.len() * 2 * crate::water_fft::OCEAN_N) as u32;
const _: () = assert!(crate::water_fft::OCEAN_N == SEA_GRID && OCEAN_ROW == 320);
pub const SEA_EXTENT: f32 = 512.0;
const MAX_WATER_IMPULSES: usize = 192;
// Three connected quads per sheet instance; ordinary droplets use the first
// quad and collapse the others. Counts remain within the shared particle cap.
pub(crate) const WATER_PARTICLE_VERTICES: u32 = 18;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WaterEffects {
    pub enabled: bool,
    pub quality: u8,
    /// Share the viewer particle budget with scripted particles.
    pub particle_budget: u32,
    pub waves: bool,
    pub absorption: bool,
    pub fresnel: bool,
    pub wet_ground: bool,
    pub shore_foam: bool,
    pub contact_foam: bool,
    pub crest_foam: bool,
    pub interactions: bool,
    pub caustics: bool,
    pub splashes: bool,
    pub bubbles: bool,
}

impl Default for WaterEffects {
    fn default() -> Self {
        Self {
            enabled: true,
            quality: 1,
            particle_budget: 4096,
            waves: true,
            absorption: true,
            fresnel: true,
            wet_ground: true,
            shore_foam: true,
            contact_foam: true,
            crest_foam: true,
            interactions: true,
            caustics: true,
            splashes: true,
            bubbles: true,
        }
    }
}

impl WaterEffects {
    pub fn flags(self) -> u32 {
        if !self.enabled {
            return 0;
        }
        [
            self.waves,
            self.shore_foam,
            self.contact_foam,
            self.crest_foam,
            self.interactions,
            self.caustics,
            self.splashes,
            self.bubbles,
            self.absorption,
            self.fresnel,
            self.wet_ground,
        ]
        .iter()
        .enumerate()
        .fold(0, |bits, (i, on)| bits | (u32::from(*on) << i))
    }
    pub fn resolution(self) -> u32 {
        if self.flags() & 0x16 == 0 {
            return 1;
        }
        [128, 256, 384, 512][self.quality.min(3) as usize]
    }
    pub fn particle_count(self) -> u32 {
        [4, 6, 8, 12][self.quality.min(3) as usize]
    }
    pub fn range(self) -> f32 {
        [64.0, 80.0, 96.0, 128.0][self.quality.min(3) as usize]
    }
}

/// Height of the displayed macro surface for visual contacts and the camera.
/// Invert the Gerstner horizontal offset so the query uses rendered world XY.
/// Fine interaction ripples do not feed back into their own emitters.
/// `at`: where the sea is read there (`WaterGround::sea_point`).
pub fn surface_level(xy: Vec2, time: f32, settings: WaterEffects, sea: &Sea, level: f32, at: &SeaPoint) -> f32 {
    if !settings.enabled || !settings.waves {
        return level;
    }
    sea.surface(xy, time, (settings.quality.min(3) + 2).min(4) as usize, level, at)
}

/// What the water's particles may use of the particle limit `max` when
/// the region's own take `used`: what is left, but never less than a share
/// of their own (`WATER_PARTICLE_FLOOR`, within `max`). Without it, a
/// region at the limit left them a few particles, a different few each
/// frame: splashes and spray blinked in and out.
pub fn water_particle_budget(max: u32, used: u32) -> u32 {
    max.saturating_sub(used).max(WATER_PARTICLE_FLOOR.min(max))
}

/// The water particles' own share of the particle limit.
pub const WATER_PARTICLE_FLOOR: u32 = 1024;

/// Splash impact (0), sparse air release (1), obstacle footprint (2), or
/// displacement without particles (3), an intersecting avatar's surface wake (4),
/// a radial dive/cannonball sheet (5), a water-entry displacement ring (6),
/// a directed limb/bow fan (7), an anatomical surface strike (8), or the
/// spray of a wave crashing on a wall or a rock (9: motion xy off the face,
/// w its power; params x half the burst's width, w its seed), or the white
/// water it leaves (10: as 9, motion z the face's height above the water).
/// Footprints are bounded approximations, not a replacement for mesh physics.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, Pod, Zeroable)]
pub struct WaterEvent {
    /// xyz world position, w birth time on FrameParams::time.
    pub position: [f32; 4],
    /// xyz velocity (obstacles: horizontal unit axis), w impulse strength.
    pub motion: [f32; 4],
    /// x radius / obstacle half length, y kind, z water height.
    /// w: obstacle half width, swept segment length (4), sheet span (5), or footfall flag (7/8).
    pub params: [f32; 4],
}

/// Rain prototype: the terrain's height in a square around the camera
/// (`RAIN_GROUND_SIZE` cells a side, row by row from `origin`), for the
/// puddles that gather in its dips. Empty: none.
#[derive(Debug, Clone, Default)]
pub struct RainGround {
    pub origin: [f32; 2],
    pub extent: f32,
    pub heights: std::sync::Arc<[f32]>,
}

pub const RAIN_GROUND_SIZE: u32 = 128;
/// Rain prototype: cells a side of the wetness memory (`fs_rain_wet_update`).
pub const RAIN_WET_SIZE: u32 = 512;

/// The wetness memory's two textures (`wet_textures`): the one drawn each
/// frame and its view, the copy the next frame and the wet ground read and
/// its view.
pub(crate) type WetTextures = (wgpu::Texture, wgpu::TextureView, wgpu::Texture, wgpu::TextureView);

/// The wetness memory's two textures `size` texels a side.
pub(crate) fn wet_textures(device: &wgpu::Device, size: u32) -> WetTextures {
    let wet = |label, usage| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            // x the height reached, y how wet, z how deep the snow lies
            format: wgpu::TextureFormat::Rgba32Float,
            usage,
            view_formats: &[],
        })
    };
    let target = wet(
        "rain wetness (drawn)",
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    );
    let target_view = target.create_view(&Default::default());
    let read = wet("rain wetness", wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST);
    let read_view = read.create_view(&Default::default());
    (target, target_view, read, read_view)
}
/// Snow footprints: texels a side of the trail map (`fs_snow_trail_update`)
/// and the metres it covers around the camera (3 cm a texel).
pub const SNOW_TRAIL_SIZE: u32 = 1024;
pub const SNOW_TRAIL_EXTENT: f32 = 32.0;

#[derive(Debug, Clone, Default)]
pub struct WaterGround {
    pub region: u64,
    pub origin: [f32; 2],
    pub extent: f32,
    /// ground height, water height; unknown cells are deep void water.
    pub heights: std::sync::Arc<[[f32; 2]]>,
    /// What the waves read, `SEA_GRID` cells a side over `sea_extent` m
    /// from `sea_origin` (scene/water_shelter.rs): per cell the wave depth
    /// (the depth smoothed over a few metres, land below 0 down to
    /// `-LAND_DEPTH`, water bounded by `OPEN_DEPTH`, so that no step of the
    /// bed, a region edge or a cell of unknown ground raises a wall of
    /// water), how calm what stands in the water keeps it (1 open, less
    /// around a pier, a hull, a wall) and the clearance (m) up to the
    /// lowest underside above the water (a deck, a floor on stilts).
    /// Empty: open, deep water everywhere.
    pub sea: std::sync::Arc<[[f32; 6]]>,
    /// The grid baked before it, as `sea`: its refraction offset and
    /// shadow, faded out over a few seconds (`Sea::shore[0]`, 0 the old
    /// grid, 1 the new) so that a new grid never snaps the swell. Empty:
    /// the new grid alone.
    pub sea_prev: std::sync::Arc<[[f32; 3]]>,
    pub sea_origin: [f32; 2],
    pub sea_extent: f32,
    /// The spectrum's maps of the wind sea this frame (water_fft.rs,
    /// `SeaDriver::maps`; empty: none), uploaded under the sea grids.
    pub ocean: std::sync::Arc<crate::water_fft::OceanMaps>,
}

/// The clearance of a cell with nothing above its water, and the most any
/// cell keeps: high enough for a gale's crests, low enough that it blends
/// over a cell into a deck's beside it.
pub const OPEN_CLEARANCE: f32 = 6.0;
/// Water this deep is open sea to the waves (m); land counts down to
/// `-LAND_DEPTH` (where the swash runs up).
pub const OPEN_DEPTH: f32 = 20.0;
pub const LAND_DEPTH: f32 = 3.0;
/// Outside the grid, its edge fades into open sea over this (m).
const EDGE_FADE: f32 = 30.0;

/// One cell of the sea grid as the bathymetry texture's w holds it (the
/// calm in 1/255, the clearance in centimetres): exact in an f32, decoded
/// texel by texel before the shaders' bilinear mix (`water_fx_sea_texel`).
pub fn pack_shelter(calm: f32, clearance: f32) -> f32 {
    let calm = (calm.clamp(0.0, 1.0) * 255.0).round();
    let clearance = (clearance.clamp(0.0, OPEN_CLEARANCE) * 100.0).round();
    clearance * 256.0 + calm
}

/// A cell's swell reach and chop as the texture keeps them (1/255 each),
/// decoded texel by texel before any mix (`water_fx_exposure`).
pub fn pack_exposure(reach: f32, chop: f32) -> f32 {
    (reach.clamp(0.0, 1.0) * 255.0).round() * 256.0 + (chop.clamp(0.0, 1.0) * 255.0).round()
}

/// `pack_exposure`'s quantization, for the CPU's copy of the grid.
pub fn quantize_exposure(reach: f32, chop: f32) -> [f32; 2] {
    let v = pack_exposure(reach, chop);
    let high = (v / 256.0).floor();
    [high / 255.0, (v - high * 256.0) / 255.0]
}

/// `pack_shelter`'s quantization, for the CPU's copy of the grid.
pub fn quantize_shelter(calm: f32, clearance: f32) -> [f32; 2] {
    let v = pack_shelter(calm, clearance);
    let high = (v / 256.0).floor();
    [(v - high * 256.0) / 255.0, high / 100.0]
}

impl WaterGround {
    /// Camera-local macro waterline, sharing the displayed swell and shore
    /// damping with visual contacts. Never replace the region's physical plane.
    pub(crate) fn camera_level(&self, xy: Vec2, time: f32, settings: WaterEffects, sea: &Sea, fallback: f32) -> f32 {
        if self.heights.len() != 4096 || !self.extent.is_finite() || self.extent <= 0.0 {
            return fallback;
        }
        let [ground, level] = self.sample(xy).unwrap_or([fallback - 20.0, fallback]);
        surface_level(xy, time, settings, sea, level, &self.sea_point(xy, sea, level - ground))
    }

    /// Where the sea is read at `xy`: the wave depth and the swell's travel
    /// with their gradients two cells either side, the sea the shelter leaves
    /// (`Sea::shelter_scale`) and the refracted main swell, as
    /// `water_fx_sea_at`; open water `depth` deep without a grid.
    pub fn sea_point(&self, xy: Vec2, sea: &Sea, depth: f32) -> SeaPoint {
        let d0 = Vec2::new(sea.trains[0][0], sea.trains[0][1]);
        let ocean = (!self.ocean.texels.is_empty()).then(|| self.ocean.clone());
        let Some([d, calm, clearance, travel, reach, chop]) = self.sea_cell(xy, d0, sea.shore[0]) else {
            return SeaPoint {
                ocean,
                ..SeaPoint::open(depth)
            };
        };
        let h = (self.sea_extent / SEA_GRID as f32 * 2.0).max(0.25);
        let at = |o: Vec2| {
            self.sea_cell(xy + o, d0, sea.shore[0])
                .unwrap_or([d, calm, clearance, travel, reach, chop])
        };
        let (e, w, n, s) = (at(Vec2::X * h), at(-Vec2::X * h), at(Vec2::Y * h), at(-Vec2::Y * h));
        let slope = Vec2::new(e[0] - w[0], n[0] - s[0]) / (2.0 * h);
        // the travel's gradient: the way the swell goes, and how much
        // shorter it is than in open water (1 to 6 times)
        let grad = Vec2::new(e[3] - w[3], n[3] - s[3]) / (2.0 * h);
        let len = grad.length();
        let k0 = sea.trains[0][2];
        let wave = if len > 0.2 {
            grad / len.max(1e-4) * len.clamp(1.0, 6.0)
        } else {
            d0
        } * k0;
        let anchor = Vec2::new(sea.frame[0], sea.frame[1]);
        SeaPoint {
            depth: d,
            slope,
            shelter: calm.clamp(0.0, 1.0),
            cap: sea.clearance_cap(clearance),
            swell: Some(SwellPoint {
                phase: k0 * (travel + (Vec2::from_array(self.sea_origin) - anchor).dot(d0)),
                wave,
                shadow: reach,
                chop,
            }),
            ocean,
            bed: depth,
        }
    }

    /// The sea grid at `xy` (depth, calm, clearance, travel, shadow),
    /// bilinear, the previous grid's refraction faded out by `blend`;
    /// outside the grid its nearest edge, fading into open sea over
    /// `EDGE_FADE`. The travel is the plane wave along `d0` (the swell now)
    /// plus the grid's refraction offset.
    fn sea_cell(&self, xy: Vec2, d0: Vec2, blend: f32) -> Option<[f32; 6]> {
        if self.sea.len() != SEA_GRID * SEA_GRID || !self.sea_extent.is_finite() || self.sea_extent <= 0.0 || !xy.is_finite() {
            return None;
        }
        let lo = Vec2::from_array(self.sea_origin);
        let inside = xy.clamp(lo, lo + Vec2::splat(self.sea_extent));
        let mut v = bilinear(&self.sea, SEA_GRID, self.sea_origin, self.sea_extent, inside)?;
        if let Some(old) = bilinear(&self.sea_prev, SEA_GRID, self.sea_origin, self.sea_extent, inside) {
            for k in 0..3 {
                v[k + 3] = old[k] + (v[k + 3] - old[k]) * blend;
            }
        }
        let t = (xy.distance(inside) / EDGE_FADE).clamp(0.0, 1.0);
        let t = t * t * (3.0 - 2.0 * t);
        let open = [OPEN_DEPTH, 1.0, OPEN_CLEARANCE, 0.0, 1.0, 1.0];
        let mut cell: [f32; 6] = std::array::from_fn(|i| v[i] + (open[i] - v[i]) * t);
        // the travel: the plane wave along the swell now, and the grid's
        // refraction offset (fading out past the grid)
        cell[3] = (xy - lo).dot(d0) + v[3] * (1.0 - t);
        Some(cell)
    }

    /// Same bilinear bathymetry reconstruction as water_fx_bed. Contacts must
    /// meet the displayed, shore-damped surface rather than a finer CPU bed.
    pub fn sample(&self, xy: Vec2) -> Option<[f32; 2]> {
        if self.heights.len() != 4096 {
            return None;
        }
        bilinear(&self.heights, 64, self.origin, self.extent, xy)
    }
}

/// A `size`-square grid of `extent` metres from `origin`, read bilinearly
/// between cell centres as the shaders do (water_fx_bed, water_fx_sea_cell).
fn bilinear<const N: usize>(grid: &[[f32; N]], size: usize, origin: [f32; 2], extent: f32, xy: Vec2) -> Option<[f32; N]> {
    if grid.len() != size * size || !extent.is_finite() || extent <= 0.0 || !xy.is_finite() {
        return None;
    }
    let uv = (xy - Vec2::from_array(origin)) / extent;
    if uv.cmplt(Vec2::ZERO).any() || uv.cmpgt(Vec2::ONE).any() {
        return None;
    }
    let n = size as u32;
    let cell = (uv * size as f32 - Vec2::splat(0.5)).clamp(Vec2::ZERO, Vec2::splat((n - 1) as f32));
    let i = cell.floor().as_uvec2();
    let j = (i + glam::UVec2::ONE).min(glam::UVec2::splat(n - 1));
    let f = cell.fract();
    let at = |x: u32, y: u32| grid[(y * n + x) as usize];
    let (a, b, c, d) = (at(i.x, i.y), at(j.x, i.y), at(i.x, j.y), at(j.x, j.y));
    Some(std::array::from_fn(|k| {
        let top = a[k] + (b[k] - a[k]) * f.x;
        let bottom = c[k] + (d[k] - c[k]) * f.x;
        top + (bottom - top) * f.y
    }))
}

/// Particle emitters occupy the prefix read by the instanced particle shader.
/// Ripples remain available to compute without generating invisible billboards;
/// newest emitters take priority when the shared particle budget is exhausted.
fn pack_events(events: &[WaterEvent], settings: WaterEffects, packed: &mut [WaterEvent]) -> (usize, usize) {
    let flags = settings.flags();
    let particle = |e: &&WaterEvent| {
        (matches!(e.params[1], 0.0 | 5.0 | 7.0 | 9.0 | 10.0) && flags & 64 != 0) || (e.params[1] == 1.0 && flags & 128 != 0)
    };
    let recent = &events[events.len().saturating_sub(packed.len())..];
    let mut n = 0;
    for e in recent.iter().rev().filter(particle) {
        packed[n] = *e;
        n += 1;
    }
    let emitters = n;
    for e in recent.iter().rev().filter(|e| !particle(e) && !matches!(e.params[1], 4.0 | 8.0)) {
        packed[n] = *e;
        n += 1;
    }
    for e in recent.iter().rev().filter(|e| matches!(e.params[1], 4.0 | 8.0)) {
        packed[n] = *e;
        n += 1;
    }
    (n, emitters)
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniform {
    area: [f32; 4],     // origin xy, extent, inverse texel size
    previous: [f32; 4], // old origin xy, reset, dt
    clock: [f32; 4],    // now, previous time, fine-wake origin xy
    counts: [u32; 4],   // events, contacts, flags, quality
    sea_area: [f32; 4], // sea grid origin xy, extent
    sea: Sea,           // the wind's sea (water_sea.rs, WaterSea)
}

pub(crate) struct WaterGpu {
    pub view: wgpu::TextureView,
    pub caustics_view: wgpu::TextureView,
    wake: crate::water_wake::WakeGpu,
    event_count: u32,
    caustics_pipeline: wgpu::RenderPipeline,
    caustics_group: wgpu::BindGroup,
    textures: [wgpu::Texture; 2],
    pub uniform: wgpu::Buffer,
    pub events: wgpu::Buffer,
    ground: wgpu::Texture,
    pub ground_view: wgpu::TextureView,
    rain_ground: wgpu::Texture,
    pub rain_ground_view: wgpu::TextureView,
    rain_ground_snapshot: Option<std::sync::Arc<[f32]>>,
    /// Where the uploaded rain ground is: origin x, y, extent (0: none).
    pub rain_area: [f32; 3],
    /// Rain prototype: the wetness memory, drawn into `rain_wet_target`
    /// each frame and copied into `rain_wet` (frame group binding 27),
    /// which the next update and the wet ground read.
    pub rain_wet_target: wgpu::Texture,
    pub rain_wet_target_view: wgpu::TextureView,
    pub rain_wet: wgpu::Texture,
    pub rain_wet_view: wgpu::TextureView,
    /// Its texels a side (`resize_wet`): 4 a metre, over the renderer's
    /// memory square.
    pub rain_wet_size: u32,
    /// Snow footprints: how deep the snow is pressed down (R), drawn into
    /// `snow_trail_target` and copied into `snow_trail` (frame binding 28).
    pub snow_trail_target: wgpu::Texture,
    pub snow_trail_target_view: wgpu::TextureView,
    pub snow_trail: wgpu::Texture,
    pub snow_trail_view: wgpu::TextureView,
    ground_snapshot: Option<std::sync::Arc<[[f32; 2]]>>,
    sea_snapshot: Option<std::sync::Arc<[[f32; 6]]>>,
    sea_prev_snapshot: Option<std::sync::Arc<[[f32; 3]]>>,
    ocean_snapshot: Option<std::sync::Arc<crate::water_fft::OceanMaps>>,
    group: wgpu::BindGroup,
    pipeline: wgpu::ComputePipeline,
    size: u32,
    origin: Vec2,
    last_time: Option<f32>,
    region: u64,
    flags: u32,
    quality: u8,
    pub particles: u32,
}

impl WaterGpu {
    /// Rain prototype: the terrain heights around the camera, when they
    /// changed.
    pub fn upload_rain_ground(&mut self, queue: &wgpu::Queue, ground: &RainGround) {
        let n = (RAIN_GROUND_SIZE * RAIN_GROUND_SIZE) as usize;
        if ground.heights.len() != n || ground.extent.is_nan() || ground.extent <= 0.0 {
            self.rain_area = [0.0; 3];
            return;
        }
        self.rain_area = [ground.origin[0], ground.origin[1], ground.extent];
        if self
            .rain_ground_snapshot
            .as_ref()
            .is_some_and(|old| std::sync::Arc::ptr_eq(old, &ground.heights))
        {
            return;
        }
        self.rain_ground_snapshot = Some(ground.heights.clone());
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.rain_ground,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(ground.heights.as_ref()),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(RAIN_GROUND_SIZE * 4),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: RAIN_GROUND_SIZE,
                height: RAIN_GROUND_SIZE,
                depth_or_array_layers: 1,
            },
        );
    }

    /// The wetness memory at `size` texels a side (`wet_textures`, made by
    /// the renderer under its video memory check), its old one dropped (the
    /// renderer starts its texels again and binds the new views).
    pub(crate) fn install_wet(&mut self, made: WetTextures, size: u32) {
        let (target, target_view, read, read_view) = made;
        self.rain_wet_target = target;
        self.rain_wet_target_view = target_view;
        self.rain_wet = read;
        self.rain_wet_view = read_view;
        self.rain_wet_size = size;
    }

    pub fn new(device: &wgpu::Device, size: u32) -> Self {
        let textures = std::array::from_fn(|index| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("water ripple/foam field"),
                size: wgpu::Extent3d {
                    width: size,
                    height: if index == 0 { size * 2 } else { size },
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::STORAGE_BINDING
                    | wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::COPY_DST
                    | wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
        });
        let view = textures[0].create_view(&Default::default());
        let output = textures[1].create_view(&Default::default());
        let ground = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("water bathymetry"),
            // the 64 bathymetry rows, then the sea grid's (SEA_GRID wide),
            // then the previous sea grid's, fading out, then the spectrum's
            // maps (water_fft.rs, OCEAN_N wide: per cascade its displacement
            // then its slopes)
            size: wgpu::Extent3d {
                width: SEA_GRID as u32,
                height: OCEAN_ROW + OCEAN_ROWS,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            // bathymetry: ground, water; sea grid: wave depth, packed
            // shelter (pack_shelter)
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let uniform = crate::gpu_alloc::buffer_init(
            device,
            "water effects",
            bytemuck::bytes_of(&Uniform::zeroed()),
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        );
        let events = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("water interaction events"),
            size: (MAX_WATER_EVENTS * std::mem::size_of::<WaterEvent>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("water simulation"),
            source: wgpu::ShaderSource::Wgsl(crate::water_shader_source(
                &["water_sim.wgsl"],
                include_str!("shaders/water_sim.wgsl"),
            )),
        });
        let pipeline = device.create_compute_pipeline_cached(&wgpu::ComputePipelineDescriptor {
            label: Some("water simulation"),
            layout: None,
            module: &module,
            entry_point: Some("simulate"),
            compilation_options: Default::default(),
            cache: None,
        });
        let ground_view = ground.create_view(&Default::default());
        let rain_ground = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("rain ground heights"),
            size: wgpu::Extent3d {
                width: RAIN_GROUND_SIZE,
                height: RAIN_GROUND_SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let rain_ground_view = rain_ground.create_view(&Default::default());
        let (rain_wet_target, rain_wet_target_view, rain_wet, rain_wet_view) = wet_textures(device, RAIN_WET_SIZE);
        let trail = |label, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: SNOW_TRAIL_SIZE,
                    height: SNOW_TRAIL_SIZE,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R16Float,
                usage,
                view_formats: &[],
            })
        };
        let snow_trail_target = trail(
            "snow footprints (drawn)",
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let snow_trail_target_view = snow_trail_target.create_view(&Default::default());
        let snow_trail = trail(
            "snow footprints",
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        let snow_trail_view = snow_trail.create_view(&Default::default());
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("water simulation"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: events.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&output),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&ground_view),
                },
            ],
        });
        let caustics = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("refracted water light"),
            size: wgpu::Extent3d {
                width: 256,
                height: 256,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let caustics_view = caustics.create_view(&Default::default());
        let caustics_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("refracted water caustics"),
            source: wgpu::ShaderSource::Wgsl(crate::water_shader_source(
                &["water_spectrum.wgsl", "water_caustics.wgsl"],
                concat!(
                    include_str!("shaders/water_spectrum.wgsl"),
                    "\n",
                    include_str!("shaders/water_caustics.wgsl")
                ),
            )),
        });
        let additive = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        };
        let caustics_pipeline = device.create_render_pipeline_cached(&wgpu::RenderPipelineDescriptor {
            label: Some("refracted water caustics"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &caustics_shader,
                entry_point: Some("vs_caustic"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &caustics_shader,
                entry_point: Some("fs_caustic"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba16Float,
                    blend: Some(wgpu::BlendState {
                        color: additive,
                        alpha: additive,
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let caustics_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("caustics controls"),
            layout: &caustics_pipeline.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        let wake = crate::water_wake::WakeGpu::new(device, &uniform, &events, &ground_view);
        Self {
            wake,
            event_count: 0,
            view,
            caustics_view,
            caustics_pipeline,
            caustics_group,
            textures,
            uniform,
            events,
            ground,
            ground_view,
            rain_ground,
            rain_ground_view,
            rain_ground_snapshot: None,
            rain_area: [0.0; 3],
            rain_wet_target,
            rain_wet_target_view,
            rain_wet,
            rain_wet_view,
            rain_wet_size: RAIN_WET_SIZE,
            snow_trail_target,
            snow_trail_target_view,
            snow_trail,
            snow_trail_view,
            ground_snapshot: None,
            sea_snapshot: None,
            sea_prev_snapshot: None,
            ocean_snapshot: None,
            group,
            pipeline,
            size,
            origin: Vec2::ZERO,
            last_time: None,
            region: 0,
            flags: 0,
            quality: 0,
            particles: 0,
        }
    }

    /// Returns whether the sampled view changed (the frame groups then rebind it).
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        settings: WaterEffects,
        time: f32,
        eye: Vec2,
        ground: &WaterGround,
        sea: &Sea,
        events: &[WaterEvent],
        contacts: &[WaterEvent],
    ) -> bool {
        let valid_ground = ground.heights.len() == (WATER_GROUND_SIZE * WATER_GROUND_SIZE) as usize && ground.extent > 0.0;
        let resolution = if valid_ground { settings.resolution() } else { 1 };
        let resized = self.size != resolution;
        if resized {
            *self = Self::new(device, resolution);
        }
        let origin = Vec2::from_array(ground.origin);
        let extent = ground.extent.max(1.0);
        let reset = self.region != ground.region
            || self.last_time.is_none()
            || time < self.last_time.unwrap_or(time)
            || self.origin.distance(origin) >= extent;
        let previous_time = self.last_time.unwrap_or(time);
        let dt = (time - previous_time).clamp(0.0, 1.0 / 30.0);
        let mut packed = [WaterEvent::default(); MAX_WATER_IMPULSES];
        let (n, emitters) = pack_events(events, settings, &mut packed);
        let wakes = packed[..n].iter().filter(|e| matches!(e.params[1], 4.0 | 8.0)).count();
        let regular = n - wakes;
        let c = contacts.len().min(MAX_WATER_EVENTS - n);
        // Compute sees only impulses and obstacles. Fine historical segments
        // follow those ranges, so the coarse simulation never loops over them.
        self.event_count = (n + c) as u32;
        if regular > 0 {
            queue.write_buffer(&self.events, 0, bytemuck::cast_slice(&packed[..regular]));
        }
        if c > 0 {
            queue.write_buffer(
                &self.events,
                (regular * std::mem::size_of::<WaterEvent>()) as u64,
                bytemuck::cast_slice(&contacts[..c]),
            );
        }
        if wakes > 0 {
            queue.write_buffer(
                &self.events,
                ((regular + c) * std::mem::size_of::<WaterEvent>()) as u64,
                bytemuck::cast_slice(&packed[regular..n]),
            );
        }
        let flags = settings.flags();
        let u = Uniform {
            area: [origin.x, origin.y, extent, self.size as f32 / extent],
            previous: [
                self.origin.x,
                self.origin.y,
                if reset {
                    1.0
                } else if (self.flags ^ flags) & 6 != 0 {
                    2.0
                } else {
                    0.0
                },
                dt,
            ],
            clock: {
                let cell = 32.0 / self.size as f32;
                let fine_origin = (eye / cell).floor() * cell - Vec2::splat(16.0);
                [time, previous_time, fine_origin.x, fine_origin.y]
            },
            counts: [
                regular as u32,
                c as u32,
                if valid_ground { flags } else { 0 },
                settings.quality.min(3) as u32,
            ],
            sea_area: [ground.sea_origin[0], ground.sea_origin[1], ground.sea_extent.max(1.0), 0.0],
            sea: *sea,
        };
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&u));
        if valid_ground
            && flags != 0
            && self
                .ground_snapshot
                .as_ref()
                .is_none_or(|old| !std::sync::Arc::ptr_eq(old, &ground.heights))
        {
            self.ground_snapshot = Some(ground.heights.clone());
            let texels: Vec<[f32; 4]> = ground.heights.iter().map(|&[g, w]| [g, w, 0.0, 0.0]).collect();
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.ground,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                bytemuck::cast_slice(&texels),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(WATER_GROUND_SIZE * 16),
                    rows_per_image: None,
                },
                wgpu::Extent3d {
                    width: WATER_GROUND_SIZE,
                    height: WATER_GROUND_SIZE,
                    depth_or_array_layers: 1,
                },
            );
        }
        if flags != 0
            && ground.sea.len() == SEA_GRID * SEA_GRID
            && self
                .sea_snapshot
                .as_ref()
                .is_none_or(|old| !std::sync::Arc::ptr_eq(old, &ground.sea))
        {
            self.sea_snapshot = Some(ground.sea.clone());
            let texels: Vec<[f32; 4]> = ground
                .sea
                .iter()
                .map(|&[depth, calm, clear, travel, reach, chop]| [depth, pack_shelter(calm, clear), travel, pack_exposure(reach, chop)])
                .collect();
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.ground,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: 0,
                        y: WATER_GROUND_SIZE,
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                bytemuck::cast_slice(&texels),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(SEA_GRID as u32 * 16),
                    rows_per_image: None,
                },
                wgpu::Extent3d {
                    width: SEA_GRID as u32,
                    height: SEA_GRID as u32,
                    depth_or_array_layers: 1,
                },
            );
        }
        if flags != 0
            && self
                .sea_prev_snapshot
                .as_ref()
                .is_none_or(|old| !std::sync::Arc::ptr_eq(old, &ground.sea_prev))
        {
            self.sea_prev_snapshot = Some(ground.sea_prev.clone());
            // (none: the new grid's own values, so the fade changes nothing)
            let texels: Vec<[f32; 4]> = if ground.sea_prev.len() == SEA_GRID * SEA_GRID {
                ground
                    .sea_prev
                    .iter()
                    .map(|&[offset, reach, chop]| [offset, pack_exposure(reach, chop), 0.0, 0.0])
                    .collect()
            } else {
                ground.sea.iter().map(|c| [c[3], pack_exposure(c[4], c[5]), 0.0, 0.0]).collect()
            };
            if texels.len() == SEA_GRID * SEA_GRID {
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &self.ground,
                        mip_level: 0,
                        origin: wgpu::Origin3d {
                            x: 0,
                            y: WATER_GROUND_SIZE + SEA_GRID as u32,
                            z: 0,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    bytemuck::cast_slice(&texels),
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(SEA_GRID as u32 * 16),
                        rows_per_image: None,
                    },
                    wgpu::Extent3d {
                        width: SEA_GRID as u32,
                        height: SEA_GRID as u32,
                        depth_or_array_layers: 1,
                    },
                );
            }
        }
        if flags != 0
            && ground.ocean.texels.len() == OCEAN_ROWS as usize * crate::water_fft::OCEAN_N
            && self
                .ocean_snapshot
                .as_ref()
                .is_none_or(|old| !std::sync::Arc::ptr_eq(old, &ground.ocean))
        {
            self.ocean_snapshot = Some(ground.ocean.clone());
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.ground,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x: 0, y: OCEAN_ROW, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                bytemuck::cast_slice(&ground.ocean.texels),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(crate::water_fft::OCEAN_N as u32 * 16),
                    rows_per_image: None,
                },
                wgpu::Extent3d {
                    width: crate::water_fft::OCEAN_N as u32,
                    height: OCEAN_ROWS,
                    depth_or_array_layers: 1,
                },
            );
        }
        self.particles = if flags & 0xc0 != 0 {
            // (whole events only: a cut through one drew part of a splash)
            let per = settings.particle_count().max(1);
            (emitters as u32 * per).min(settings.particle_budget / per * per)
        } else {
            0
        };
        self.flags = if valid_ground { flags } else { 0 };
        self.quality = settings.quality.min(3);
        self.region = ground.region;
        self.origin = origin;
        self.last_time = Some(time);
        resized
    }

    pub fn encode(&self, encoder: &mut wgpu::CommandEncoder) {
        if self.flags & 32 != 0 {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("refracted water light"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.caustics_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.caustics_pipeline);
            pass.set_bind_group(0, &self.caustics_group, &[]);
            let n = [96, 160, 224, 256][self.quality as usize];
            pass.draw(0..6, 0..n * n * 3);
        }
        if self.size == 1 {
            return;
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("water ripples and foam"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.group, &[]);
            pass.dispatch_workgroups(self.size.div_ceil(8), self.size.div_ceil(8), 1);
        }
        self.wake.encode(
            encoder,
            &self.view,
            self.size,
            if self.flags & 16 != 0 { self.event_count } else { 0 },
        );
        encoder.copy_texture_to_texture(
            self.textures[1].as_image_copy(),
            self.textures[0].as_image_copy(),
            wgpu::Extent3d {
                width: self.size,
                height: self.size,
                depth_or_array_layers: 1,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_waterline_follows_crests_and_troughs_and_respects_flat_modes() {
        let ground = WaterGround {
            origin: [0.0; 2],
            extent: 256.0,
            heights: vec![[0.0, 20.0]; 4096].into(),
            ..Default::default()
        };
        let xy = Vec2::new(43.0, 118.0);
        for quality in 0..4 {
            let settings = WaterEffects {
                quality,
                ..Default::default()
            };
            let levels: Vec<_> = (0..200)
                .map(|i| ground.camera_level(xy, i as f32 * 0.1, settings, &Sea::default(), 99.0))
                .collect();
            assert!(
                levels.iter().any(|level| *level > 20.15),
                "crests must submerge a camera above native water"
            );
            assert!(
                levels.iter().any(|level| *level < 19.85),
                "troughs must uncover a camera below native water"
            );
            for flat in [
                WaterEffects {
                    enabled: false,
                    ..settings
                },
                WaterEffects { waves: false, ..settings },
            ] {
                assert_eq!(ground.camera_level(xy, 7.3, flat, &Sea::default(), 99.0), 20.0);
            }
        }
        assert_eq!(
            WaterGround::default().camera_level(xy, 7.3, WaterEffects::default(), &Sea::default(), 20.0),
            20.0
        );
        let shallow = WaterGround {
            heights: vec![[19.95, 20.0]; 4096].into(),
            ..ground
        };
        assert_eq!(shallow.camera_level(xy, 7.3, WaterEffects::default(), &Sea::default(), 20.0), 20.0);
    }

    #[test]
    fn furrow_atlas_has_a_trough_and_outgoing_shoulders_without_particles_or_foam() {
        let _gpu = crate::test_gpu::lock();
        let instance = wgpu::Instance::default();
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
            return;
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("GPU device");
        let mut gpu = WaterGpu::new(&device, 256);
        let mut settings = WaterEffects {
            waves: false,
            shore_foam: false,
            contact_foam: false,
            crest_foam: false,
            caustics: false,
            splashes: false,
            bubbles: false,
            ..Default::default()
        };
        let ground = WaterGround {
            region: 1,
            origin: [-80.0; 2],
            extent: 160.0,
            heights: vec![[0.0, 20.0]; 4096].into(),
            ..Default::default()
        };
        let mut events: Vec<_> = (0..25)
            .map(|i| {
                let age = i as f32 * 0.08;
                WaterEvent {
                    position: [-age * 1.5, 0.0, 19.94, 10.0 - age],
                    motion: [1.5, 0.0, 0.0, 0.3],
                    params: [0.35, 4.0, 20.0, 0.12],
                }
            })
            .collect();
        events.push(WaterEvent {
            position: [4.0, 0.0, 20.0, 9.8],
            motion: [0.0, 0.0, -2.0, 0.3],
            params: [0.07, 8.0, 20.0, 0.0],
        });
        events.push(WaterEvent {
            position: [4.0, 4.0, 20.0, 9.8],
            motion: [0.0, 0.0, -2.0, 0.3],
            params: [0.07, 8.0, 20.0, 1.0],
        });
        // Read the production atlas as f32 on the GPU, avoiding a second Rust
        // implementation of the wake or assumptions about half-float storage.
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("furrow atlas regression probe"),
            source: wgpu::ShaderSource::Wgsl(
                r#"
@group(0) @binding(0) var atlas:texture_2d<f32>;
@group(0) @binding(1) var<storage,read_write> result:array<vec4<f32>>;
@compute @workgroup_size(8,8)
fn probe(@builtin(global_invocation_id) id:vec3<u32>) {
    result[id.y*256u+id.x]=textureLoad(atlas,vec2<i32>(id.xy)+vec2<i32>(0,256),0);
}"#
                .into(),
            ),
        });
        let pipeline = device.create_compute_pipeline_cached(&wgpu::ComputePipelineDescriptor {
            label: None,
            layout: None,
            module: &shader,
            entry_point: Some("probe"),
            compilation_options: Default::default(),
            cache: None,
        });
        let bytes = 256 * 256 * 16;
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: crate::readback::readback_size(bytes),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&gpu.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: output.as_entire_binding(),
                },
            ],
        });
        for (enabled, bed, level) in [(true, 0.0, 20.0), (false, 0.0, 20.0), (true, 21.0, 20.0), (true, 0.0, 22.0)] {
            let ground = WaterGround {
                heights: vec![[bed, level]; 4096].into(),
                ..ground.clone()
            };
            settings.interactions = enabled;
            settings.shore_foam = !enabled; // Keep the same atlas allocation while disabling interactions.
            gpu.prepare(&device, &queue, settings, 10.0, Vec2::ZERO, &ground, &Sea::default(), &events, &[]);
            assert_eq!(gpu.particles, 0);
            let mut encoder = device.create_command_encoder(&Default::default());
            gpu.encode(&mut encoder);
            {
                let mut pass = encoder.begin_compute_pass(&Default::default());
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &group, &[]);
                pass.dispatch_workgroups(32, 32, 1);
            }
            encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, bytes);
            queue.submit([encoder.finish()]);
            readback.map_async(wgpu::MapMode::Read, .., |r| r.expect("atlas readback"));
            device.poll(wgpu::PollType::wait_indefinitely()).expect("GPU completion");
            let mapped = readback.get_mapped_range(..).expect("mapped atlas");
            let samples = crate::readback::f32x4s(&mapped);
            assert!(samples.iter().flatten().all(|v| v.is_finite()));
            if enabled && bed == 0.0 && level == 20.0 {
                let at = |x: f32, y: f32| samples[((y + 16.0) * 8.0) as usize * 256 + ((x + 16.0) * 8.0) as usize];
                assert!(
                    at(4.0, 0.125)[2] > 0.005,
                    "a small vertical hand/head strike has its own expanding ring"
                );
                assert!(
                    at(4.0, 4.125)[2] > 0.003 && at(4.0, 4.125)[2] < at(4.0, 0.125)[2] * 0.8,
                    "a footfall makes a shallower local ring, not a large water-entry wave"
                );
                assert!(at(-0.25, 0.0)[2] < -0.02, "a real depression behind the body: {:?}", at(-0.25, 0.0));
                assert!(at(-0.5, 0.5)[2] > 0.004, "raised shoulder: {:?}", at(-0.5, 0.5));
                assert!(at(-1.5, 1.0)[2] > 0.001, "older waves spread outward");
                assert!(at(-0.5, 0.25)[1] > 0.01, "normal tilts up the side of the furrow");
                assert!(
                    samples.iter().all(|p| p[2].abs() < 0.2 && p[3] == 0.0),
                    "bounded displacement, independently disabled foam"
                );
            } else {
                assert!(
                    samples.iter().flatten().all(|v| *v == 0.0),
                    "disabled interactions, dry ground and another water level cannot retain a furrow"
                );
            }
            drop(mapped);
            readback.unmap();
        }
        // a fresh gale's sea, anchored away from the origin, an epoch behind,
        // read at 64 points four times (each quality)
        const PROBES: u32 = 256;
        let mut driver = crate::water_sea::SeaDriver::default();
        driver.advance(3.0, Vec2::new(4.0, 1.0), Vec2::new(-30.0, 12.0));
        let sea = driver.advance(5.0, Vec2::new(14.0, 6.0), Vec2::new(-28.0, 13.0));
        let sea_buffer = crate::gpu_alloc::buffer_init(&device, "sea", bytemuck::bytes_of(&sea), wgpu::BufferUsages::UNIFORM);
        let source = format!(
            "{}\n{}",
            include_str!("shaders/water_spectrum.wgsl"),
            r#"
@group(0) @binding(0) var<storage,read_write> result:array<vec4<f32>>;
@group(0) @binding(1) var<uniform> sea:WaterSea;
@compute @workgroup_size(1)
fn probe(@builtin(global_invocation_id) id:vec3<u32>) {
    let query=id.x/4u; let quality=id.x%4u;
    let xy=vec2<f32>(f32(query)*13.7-20.0,f32(query)*2.3);
    let time=7.3+f32(query)*0.4;
    let depth=array<f32,5>(0.2,1.6,6.0,-0.25,1.2)[query%5u];
    // a beach, a flat, a shoal's flank, the sand above the still water
    // (the swash, its backwash's sheet) and the foot of a rock face
    let slope=array<vec2<f32>,5>(vec2<f32>(0.08,0.0),vec2<f32>(0.0),vec2<f32>(0.02,-0.015),vec2<f32>(0.06,0.02),vec2<f32>(-0.7,0.2))[query%5u];
    var at:WaterSeaAt;
    at.depth=depth;at.slope=slope;at.bed=depth;
    at.shelter=array<f32,2>(1.0,0.4)[query%2u];
    at.cap=array<f32,3>(1.0,0.7,0.2)[query%3u];
    // a refracted swell: its phase at xy, wave vector and shadow
    at.phase=f32(query)*0.77;
    at.wave=array<vec2<f32>,3>(vec2<f32>(0.07,0.02),vec2<f32>(-0.05,0.11),vec2<f32>(0.2,-0.03))[query%3u];
    at.shadow=array<f32,2>(1.0,0.5)[query%2u];
    at.chop=array<f32,3>(1.0,0.6,0.3)[query%3u];
    var here=at;
    var p=xy;
    for(var i=0u;i<3u;i++){here.phase=at.phase+dot(at.wave,p-xy);p=xy-water_sea(p,time,quality,sea,here,WATER_OCEAN_NONE,0.0).offset.xy;}
    here.phase=at.phase+dot(at.wave,p-xy);
    result[id.x]=vec4<f32>(20.0+water_sea(p,time,quality,sea,here,WATER_OCEAN_NONE,0.0).offset.z,0.0,0.0,0.0);
}
"#
        );
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("CPU contact / rendered swell agreement"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let pipeline = device.create_compute_pipeline_cached(&wgpu::ComputePipelineDescriptor {
            label: None,
            layout: None,
            module: &shader,
            entry_point: Some("probe"),
            compilation_options: Default::default(),
            cache: None,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: output.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: sea_buffer.as_entire_binding(),
                },
            ],
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(PROBES, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, u64::from(PROBES) * 16);
        queue.submit([encoder.finish()]);
        readback.map_async(wgpu::MapMode::Read, ..u64::from(PROBES) * 16, |r| r.expect("swell readback"));
        device.poll(wgpu::PollType::wait_indefinitely()).expect("GPU completion");
        let mapped = readback.get_mapped_range(..u64::from(PROBES) * 16).expect("mapped swell");
        let samples = crate::readback::f32x4s(&mapped);
        for (index, sample) in samples.iter().enumerate() {
            let query = index / 4;
            let quality = (index % 4) as u8;
            let xy = Vec2::new(query as f32 * 13.7 - 20.0, query as f32 * 2.3);
            let time = 7.3 + query as f32 * 0.4;
            let ground = 20.0 - [0.2, 1.6, 6.0, -0.25, 1.2][query % 5];
            let settings = WaterEffects {
                quality,
                ..Default::default()
            };
            let at = SeaPoint {
                depth: 20.0 - ground,
                slope: [
                    Vec2::new(0.08, 0.0),
                    Vec2::ZERO,
                    Vec2::new(0.02, -0.015),
                    Vec2::new(0.06, 0.02),
                    Vec2::new(-0.7, 0.2),
                ][query % 5],
                shelter: [1.0, 0.4][query % 2],
                cap: [1.0, 0.7, 0.2][query % 3],
                swell: Some(SwellPoint {
                    phase: query as f32 * 0.77,
                    wave: [Vec2::new(0.07, 0.02), Vec2::new(-0.05, 0.11), Vec2::new(0.2, -0.03)][query % 3],
                    shadow: [1.0, 0.5][query % 2],
                    chop: [1.0, 0.6, 0.3][query % 3],
                }),
                ocean: None,
                bed: 20.0 - ground,
            };
            let cpu = surface_level(xy, time, settings, &sea, 20.0, &at);
            assert!(
                (cpu - sample[0]).abs() < 0.0003,
                "CPU contact must meet rendered water: {cpu} vs {}",
                sample[0]
            );
            assert_eq!(
                surface_level(xy, time, WaterEffects { waves: false, ..settings }, &sea, 20.0, &at),
                20.0
            );
        }
        drop(mapped);
        readback.unmap();
    }
    #[test]
    fn the_water_keeps_its_share_of_particles_in_a_region_full_of_them() {
        assert_eq!(water_particle_budget(8192, 100), 8092);
        // a region at the limit: the water still has its share, steady
        assert_eq!(water_particle_budget(8192, 8192), WATER_PARTICLE_FLOOR);
        assert_eq!(water_particle_budget(8192, 8180), WATER_PARTICLE_FLOOR);
        // never past the limit itself
        assert_eq!(water_particle_budget(500, 500), 500);
    }

    #[test]
    fn entry_sheets_and_surface_fans_are_in_the_particle_prefix_but_rings_are_not() {
        let events: Vec<_> = [6.0, 5.0, 1.0, 0.0, 7.0]
            .into_iter()
            .map(|kind| WaterEvent {
                params: [0.2, kind, 20.0, 0.0],
                ..Default::default()
            })
            .collect();
        let mut packed = [WaterEvent::default(); 5];
        assert_eq!(pack_events(&events, WaterEffects::default(), &mut packed), (5, 4));
        assert!(packed[..4].iter().all(|e| e.params[1] != 6.0));
        let settings = WaterEffects {
            splashes: false,
            ..Default::default()
        };
        assert_eq!(pack_events(&events, settings, &mut packed), (5, 1));
        assert_eq!(packed[0].params[1], 1.0);
    }

    #[test]
    fn effects_disable_independently_and_bound_work() {
        let mut s = WaterEffects::default();
        assert_eq!(s.flags(), 2047);
        s.bubbles = false;
        assert_eq!(s.flags(), 1919);
        s.quality = 255;
        assert_eq!(s.resolution(), 512);
        s.enabled = false;
        assert_eq!(s.flags(), 0);
        assert_eq!(s.resolution(), 1);
    }

    #[test]
    fn only_enabled_particle_emitters_occupy_the_draw_prefix() {
        let event = |kind, time| WaterEvent {
            params: [0.2, kind, 0.0, 0.0],
            position: [0.0, 0.0, 0.0, time],
            ..Default::default()
        };
        let events = [event(0.0, 1.0), event(3.0, 2.0), event(1.0, 3.0), event(3.0, 4.0)];
        let mut packed = [WaterEvent::default(); 4];
        let mut settings = WaterEffects::default();
        assert_eq!(pack_events(&events, settings, &mut packed), (4, 2));
        assert_eq!([packed[0].position[3], packed[1].position[3]], [3.0, 1.0]);
        settings.bubbles = false;
        assert_eq!(pack_events(&events, settings, &mut packed), (4, 1));
        assert_eq!(packed[0].params[1], 0.0);
        settings.splashes = false;
        assert_eq!(pack_events(&events, settings, &mut packed), (4, 0));
        assert_eq!(pack_events(&events, WaterEffects::default(), &mut packed[..2]), (2, 1));
        assert_eq!(packed[0].position[3], 3.0);
    }
}
