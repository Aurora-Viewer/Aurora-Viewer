//! Volumetric clouds: a sky-attached layer raymarched in compute and
//! composited into the sky pass (shaders/clouds.wgsl), an Aurora choice
//! beside the Legacy Viewer's flat cloud layer (the Legacy path is the one
//! the sky shader already has; nothing here runs with the setting off).
//!
//! The layer sits at real cloud heights above the camera and follows it like
//! the Legacy Viewer's sky dome (`cloudsF.glsl`: the dome is drawn around the
//! camera), so it is never reached and never meets a build: it is sky,
//! behind everything. Its pattern is anchored to the world's horizontal
//! position (a little parallax when the camera moves), and shaped by the sky
//! settings the Legacy layer reads: the cloud texture and the density
//! parameters give the coverage (the weather map), `cloud_shadow` and the
//! density give the cloud type (height profile), the sun, moon and ambient
//! colours light them, `cloud_scale` sizes them and the scroll moves them.
//!
//! Per frame (Medium):
//!
//! 1. the weather map, drawn by the scene shader (env.wgsl
//!    `fs_cloud_weather`) only when its parameters changed ([`Params::weather_key`]);
//! 2. the noise textures: read from the shipped file (clouds_noise.rs) by a
//!    thread and uploaded the frame it is done, then nothing; when the file
//!    cannot be read, a slice of the base noise a frame for 16 frames;
//! 3. the trace: a quarter of the half resolution sky pixels, a march of 64
//!    steps (up to 96 at grazing angles; stills 128) with a light march per sample;
//! 4. the reconstruction: the half resolution image from the trace and the
//!    history, reprojected for the camera's rotation and its horizontal
//!    move, clamped to the trace's neighbourhood;
//! 5. in scene pass A, right after the sky: a full screen triangle blending
//!    `dst * T + L` ([`Clouds::draw`]). The haze, the stars, the moon and the
//!    sun already drawn are seen through the transmittance.
//!
//! Design: the Horizon Zero Dawn cloud model (Schneider & Vos, SIGGRAPH
//! 2015), Frostbite's energy conserving integration (Hillaire, SIGGRAPH
//! 2016).

use crate::pipeline_cache::CachedPipelines;
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};

/// Side of the weather map. Must match env.wgsl `CLOUD_WEATHER_SIZE`.
pub const WEATHER_SIZE: u32 = 512;
const BASE_SIZE: u32 = 128;
const DETAIL_SIZE: u32 = 32;
const CURL_SIZE: u32 = 128;
/// Layers of the base noise made a frame.
const SLICE: u32 = 8;
/// Frames the base noise takes (`BASE_SIZE / SLICE`).
pub const NOISE_FRAMES: u32 = BASE_SIZE / SLICE;
/// The pattern repeats every `PERIOD_KM` km: a multiple of the base noise
/// (10.5 km across), the detail noise (0.5 km), the curl noise (3 km) and of the
/// weather tiles ([`tile_km`]), so that wrapping the camera's position in it
/// shows nothing.
const PERIOD_KM: f64 = 105.0;
/// Steps of the march and taps of the light march (Medium).
const STEPS: u32 = 64;
const LIGHT_STEPS: u32 = 5;
/// A still (a photo, a tile of one, a cube face) is traced once, with no
/// history: every pixel of the frame, more steps, and a few sub-samples whose
/// jitter is spread over the step grid.
const STILL_STEPS: u32 = 128;
const STILL_LIGHT_STEPS: u32 = 6;
const STILL_SAMPLES: u32 = 4;

/// A frame that is drawn once and read back (a photo, one tile of a tiled
/// photo, a face of a 360 degree photo): it is traced in full and takes no
/// part in the live history.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StillSpec {
    /// The frame's first pixel in the whole image (0, 0 unless it is a
    /// tile): the jitter is keyed on the whole image's pixels, so that
    /// neighbouring tiles agree along their border.
    pub origin: (u32, u32),
}

/// The pixel of the whole photo that a tile starts at. `part` is the tile's
/// part of the photo in fractions (left, top, width, height: the renderer's
/// `photo_part`), `photo` the whole photo's size in pixels.
pub fn still_origin(part: [f32; 4], photo: (u32, u32)) -> (u32, u32) {
    let at = |fraction: f32, side: u32| (fraction.max(0.0) * side as f32).round() as u32;
    (at(part[0], photo.0), at(part[1], photo.1))
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CloudU {
    inv_vp: [[f32; 4]; 4],
    prev_vp: [[f32; 4]; 4],
    sun: [f32; 4],
    sun_col: [f32; 4],
    amb_col: [f32; 4],
    haze: [f32; 4],
    cam: [f32; 4],
    delta: [f32; 4],
    drift: [f32; 4],
    layer: [f32; 4],
    sizes: [u32; 4],
    misc: [u32; 4],
    gen_z: [u32; 4],
}

/// What a frame gives the clouds, worked out from the sky ([`Params::new`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Params {
    /// Rotation-only view-projection and its inverse.
    pub vp: Mat4,
    pub inv_vp: Mat4,
    /// Direction towards the light (z-up).
    pub light: Vec3,
    /// Linear radiance of the light, and of the sky light, on a cloud.
    pub sun_col: Vec3,
    pub amb_col: Vec3,
    /// Haze the far clouds melt into (linear), and its distance of 1/e (km).
    pub haze: Vec3,
    pub haze_km: f32,
    /// The camera's position on the grid (km, horizontal).
    pub camera_km: [f64; 2],
    /// The pattern's offset (km): the density parameters' position, which
    /// holds the scroll.
    pub pattern_km: [f64; 2],
    /// Bottom and top of the layer above the camera (km), and its type.
    pub layer: (f32, f32),
    pub kind: f32,
    /// Side of a weather tile (km).
    pub tile_km: f32,
    pub extinction: f32,
    /// Hash of what the weather map is made of ([`weather_key`]).
    pub weather_key: u64,
}

impl Params {
    /// None when the Legacy layer would draw nothing: no cloud scale, no
    /// cloud texture yet, or a density of at most [`LAYER_MIN`]
    /// (`cloud_pos_density1.z`: the Legacy shader scales the cloud term by it
    /// before it saturates, ×10 then min 1, cloudsF.glsl under
    /// indra/newview/app_settings/shaders/class1/deferred).
    pub fn new(f: &crate::types::FrameParams, c: &crate::types::CloudsFrame, vp: Mat4, inv_vp: Mat4) -> Option<Params> {
        let origin_m = c.region_origin;
        let s = &f.sky;
        if !layer_drawn(s.cloud_scale, s.cloud_texture, s.cloud_pos_density1.z) {
            return None;
        }
        let kind = cloud_type(coverage(s.cloud_shadow));
        let tile = tile_km(s.cloud_scale);
        let light = Lighting::new(s, f.sky_zenith);
        let cam = f.camera_pos;
        Some(Params {
            vp,
            inv_vp,
            light: s.light_norm,
            sun_col: light.sun,
            amb_col: light.ambient,
            haze: f.fog_color,
            haze_km: HAZE_KM,
            camera_km: [(origin_m[0] + cam.x as f64) / 1000.0, (origin_m[1] + cam.y as f64) / 1000.0],
            pattern_km: [
                s.cloud_pos_density1.x as f64 * tile as f64,
                s.cloud_pos_density1.y as f64 * tile as f64,
            ],
            layer: layer_km(kind),
            kind,
            tile_km: tile,
            extinction: extinction(s.cloud_pos_density1.z),
            weather_key: weather_key(
                s.cloud_shadow,
                s.cloud_pos_density1.z,
                s.cloud_pos_density2.z,
                s.cloud_texture,
                c.texture_size,
                c.texture_revision,
            ),
        })
    }

    /// The offset of the pattern against the layer (km), wrapped in the
    /// period the noises repeat with ([`PERIOD_KM`]).
    pub fn offset_km(&self) -> [f32; 2] {
        [0, 1].map(|i| (self.camera_km[i] + self.pattern_km[i]).rem_euclid(PERIOD_KM) as f32)
    }
}

/// `AURORA_CLOUDS_DEBUG=weather`: the weather map's coverage instead of the
/// clouds; `slice`: vertical slices of the density, for measuring the aspect
/// of the heaps (0: the clouds).
fn debug_view() -> u32 {
    static VIEW: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *VIEW.get_or_init(|| match std::env::var("AURORA_CLOUDS_DEBUG").as_deref().map(str::trim) {
        Ok("weather") => 1,
        Ok("slice") => 2,
        _ => 0,
    })
}

/// Distance at which a cloud melts into the haze (km).
const HAZE_KM: f32 = 40.0;

/// The weather map's coverage at a point (`fs_cloud_weather`, env.wgsl): the
/// cloud texture's value `n1` and its detail tap `detail`, the sky's
/// `cloud_pos_density2.z` (`detail_weight`) and `cloud_shadow`. Legacy
/// Viewer terms (`alpha1` of cloudsF.glsl: `density = 2 (cloud_shadow -
/// 0.25)` shifts the noise the cover is cut from), cut with a soft ramp so
/// that the 3D shape, not the map, draws the cloud's edge.
#[cfg(test)]
fn weather_coverage(n1: f32, detail: f32, detail_weight: f32, cloud_shadow: f32) -> f32 {
    let smooth = |lo: f32, hi: f32, x: f32| {
        let t = ((x - lo) / (hi - lo)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    let density = 2.0 * (cloud_shadow - 0.25);
    let fine = (n1 - 0.5) + (detail - 0.5) * detail_weight * 0.35;
    let wc0 = smooth(-0.05, 0.45, fine + density);
    let gc = smooth(0.0, 1.0, ((cloud_shadow - 0.1) / 0.56).clamp(0.0, 1.0));
    let wc1 = 0.55 + 0.45 * smooth(-0.25, 0.25, fine);
    let sheet = ((gc - 0.5) * 2.0).clamp(0.0, 1.0);
    let compressed = wc0.min(HEAP_KNEE) + (wc0 - HEAP_KNEE).max(0.0) * HEAP_SLOPE;
    let heaps = compressed + (wc0 - compressed) * sheet;
    heaps + (wc1 - heaps) * sheet
}

/// Where the heaps' cover starts to rise slowly, and how slowly, before the
/// sheet takes over (env.wgsl `HEAP_KNEE`, `HEAP_SLOPE`): broken cumulus keeps
/// its gaps. The cover tops out at `HEAP_KNEE + (1 - HEAP_KNEE) * HEAP_SLOPE`.
#[cfg(test)]
const HEAP_KNEE: f32 = 0.4;
#[cfg(test)]
const HEAP_SLOPE: f32 = 0.2;

/// The cover class of a sky's `cloud_shadow`, which gives the cloud type: the
/// mean cover of the weather map over the demo's stand-in cloud texture
/// (tileable value noise, mean 0.46, spread 0.21; the real texture is a grid
/// asset) is [`weather_coverage`] averaged, 0.01, 0.06, 0.14, 0.29, 0.58,
/// 0.72, 0.75 for 0.1, 0.2, 0.27, 0.36, 0.44, 0.5, 0.6 (the heaps' cap keeps
/// it under this ladder until the sheet takes over); this smoothstep is the
/// ladder.
pub fn coverage(cloud_shadow: f32) -> f32 {
    let t = ((cloud_shadow - 0.1) / 0.56).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The type of the clouds of a sky with this `cloud_shadow`.
pub fn type_of_shadow(cloud_shadow: f32) -> f32 {
    cloud_type(coverage(cloud_shadow))
}

/// The type of the clouds from the cover: 0 stratus (a grey sheet), 0.5
/// stratocumulus, 1 cumulus, with no step in between.
pub fn cloud_type(coverage: f32) -> f32 {
    let t = ((0.86 - coverage) / (0.86 - 0.38)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Bottom and top of the layer above the camera for a type (km, real
/// meteorological heights, WMO cloud atlas): stratus 0.4–0.9, stratocumulus
/// 0.6–1.25, fair-weather cumulus 1.2–2.0 (a heap is wider than tall; the
/// towers of congestus and cumulonimbus, to 6 and 12 km, come with their
/// own type later).
pub fn layer_km(kind: f32) -> (f32, f32) {
    let pick = |lo: f32, mid: f32, hi: f32| {
        if kind < 0.5 {
            lo + (mid - lo) * kind * 2.0
        } else {
            mid + (hi - mid) * (kind - 0.5) * 2.0
        }
    };
    (pick(0.4, 0.6, 1.2), pick(0.9, 1.25, 2.0))
}

/// Side of the weather tile (km): the Legacy tile (12.6 km at the default
/// scale 0.42, a quarter of the dome) taken wider for the layer's height,
/// scaled by `cloud_scale` within [0.5, 2] and rounded so that
/// [`PERIOD_KM`] holds a whole number of tiles.
pub fn tile_km(cloud_scale: f32) -> f32 {
    let wanted = PERIOD_KM / 8.0 * (cloud_scale / 0.42).clamp(0.5, 2.0) as f64;
    let n = (PERIOD_KM / wanted).round().max(1.0);
    (PERIOD_KM / n) as f32
}

/// Below this the Legacy layer is taken as off (a cloud scale or a density).
const LAYER_MIN: f32 = 0.001;

/// Whether the Legacy layer draws anything: a cloud scale of at least
/// [`LAYER_MIN`], a cloud texture and a density above [`LAYER_MIN`].
fn layer_drawn(cloud_scale: f32, cloud_texture: u32, density: f32) -> bool {
    cloud_scale >= LAYER_MIN && cloud_texture != 0 && density > LAYER_MIN
}

/// CPU mirror of the step count of `march` in shaders/clouds.wgsl (kept in
/// sync by `the_step_count_matches_the_shader`).
#[cfg(test)]
fn march_steps(steps: u32, len_km: f32) -> u32 {
    steps.max(((len_km / 0.3).ceil() as u32).min(96))
}

/// Extinction scale from the Legacy density (`cloud_pos_density1.z`, 1 by
/// default): thicker clouds, more opaque, fading out to nothing at 0 (the
/// layer is not drawn at all below 0.001, see [`Params::new`]).
pub fn extinction(density: f32) -> f32 {
    density.clamp(0.0, 1.5)
}

/// The light on a cloud, linear and scaled like the sky.
pub struct Lighting {
    pub sun: Vec3,
    pub ambient: Vec3,
}

fn srgb_to_linear(c: Vec3) -> Vec3 {
    let f = |x: f32| if x <= 0.04045 { x / 12.92 } else { ((x + 0.055) / 1.055).powf(2.4) };
    Vec3::new(f(c.x), f(c.y), f(c.z))
}

impl Lighting {
    /// The Legacy layer's terms (`sky_clouds`, common.wgsl): the sun light
    /// attenuated along its path through the atmosphere (low sun, red
    /// light), tinted by the cloud colour; the ambient (cloud shadow lifts
    /// it) tinted the same; both in the sky's sRGB-like space, made linear
    /// and scaled as the sky is.
    pub fn new(s: &crate::types::SkyParams, zenith: Vec3) -> Lighting {
        let off_axis = 1.0 / (s.light_norm.z * 2.0).max(1e-6);
        let light_atten = (s.blue_density + Vec3::splat(s.haze_density * 0.25)) * s.density_multiplier * s.max_y;
        let atten = (-light_atten * off_axis).to_array().map(f32::exp);
        // The haze between the camera and the layer, at a typical view of 40
        // degrees (the Legacy layer takes its own per pixel): it dims the lit
        // cloud (`combined_haze`) and adds the sky's colour below it
        // (`additiveColorBelowCloud`), which is what tints a Legacy cloud
        // blue (cloudsF.glsl).
        let sin_view = 0.64_f32;
        let combined = s.blue_density.abs() + Vec3::splat(s.haze_density.abs());
        let blue_w = s.blue_density / combined.max(Vec3::splat(1e-6));
        let haze_w = s.haze_density / combined.max(Vec3::splat(1e-6));
        let dist = s.max_y / sin_view * s.density_multiplier;
        let ch = Vec3::from((-combined * dist).to_array().map(f32::exp)).powf(0.5);
        let tmp_amb = s.ambient + (Vec3::ONE - s.ambient).max(Vec3::ZERO) * s.cloud_shadow * 0.5;
        let view_axis = 1.0 / (sin_view + s.light_norm.z).max(1e-6);
        let sun_below = s.sunlight * Vec3::from((-light_atten * view_axis).to_array().map(f32::exp)) * (1.0 - s.cloud_shadow);
        let glow = if s.sun_moon_glow_factor < 1.0 {
            0.0
        } else {
            s.glow.x.max(0.0).powf(s.glow.z) * s.sun_moon_glow_factor + 0.25
        };
        let below = s.blue_horizon * blue_w * (sun_below + tmp_amb) + s.haze_horizon * haze_w * (sun_below * glow + tmp_amb);
        let sun = (s.sunlight * Vec3::from(atten)).min(Vec3::ONE) * s.cloud_color * ch;
        let ambient = tmp_amb * s.cloud_color * ch + below * (Vec3::ONE - ch);
        // the Legacy layer also takes the light the sky scatters below it
        // (`add_below_cloud`): the sky's own colour, the hemisphere light of
        // the frame (`zenith`, linear), takes its place, tinted by the cloud
        let albedo = (s.cloud_color * 2.0).min(Vec3::ONE);
        Lighting {
            sun: srgb_to_linear((sun * 2.0).min(Vec3::splat(2.0))) * s.hdr_scale,
            ambient: srgb_to_linear((ambient * 2.0).min(Vec3::splat(2.0))) * s.hdr_scale + zenith * albedo,
        }
    }
}

/// The base noise is made in slices of [`SLICE`] layers, the small noises on
/// the first frame: what a frame since the setting was turned on makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NoiseStep {
    /// The detail and curl noises are made this frame.
    pub small: bool,
    /// First layer of the base noise's slice, if one is made.
    pub base_z: Option<u32>,
}

pub fn noise_step(frame: u32) -> NoiseStep {
    NoiseStep {
        small: frame == 0,
        base_z: (frame < NOISE_FRAMES).then_some(frame * SLICE),
    }
}

/// Hash of what the weather map is made of: the Legacy cloud parameters it
/// reads and the cloud texture (its slot, its size, which grows while it
/// loads, and its revision: a reload of the same size). The map is drawn
/// again only when it changes.
pub fn weather_key(
    cloud_shadow: f32,
    density1: f32,
    density2: f32,
    texture_slot: u32,
    texture_size: (u32, u32),
    texture_revision: u64,
) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325_u64;
    for v in [
        cloud_shadow.to_bits(),
        density1.to_bits(),
        density2.to_bits(),
        texture_slot,
        texture_size.0,
        texture_size.1,
        texture_revision as u32,
        (texture_revision >> 32) as u32,
    ] {
        h = (h ^ v as u64).wrapping_mul(0x0100_0000_01b3);
    }
    h
}

fn entry(binding: u32, resource: wgpu::BindingResource<'_>) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry { binding, resource }
}

/// Textures that depend on the size of the image.
struct Sized {
    size: (u32, u32),
    trace: wgpu::TextureView,
    depth: wgpu::TextureView,
    /// The half resolution image, one being written while the other is read.
    history: [wgpu::TextureView; 2],
}

struct Pipelines {
    gen_base: wgpu::ComputePipeline,
    gen_detail: wgpu::ComputePipeline,
    gen_curl: wgpu::ComputePipeline,
    trace: wgpu::ComputePipeline,
    trace_still: wgpu::ComputePipeline,
    recon: wgpu::ComputePipeline,
}

pub(crate) struct Clouds {
    module: wgpu::ShaderModule,
    pipelines: Pipelines,
    uniform: wgpu::Buffer,
    sampler: wgpu::Sampler,
    /// Clamped copy of the sampler for the screen-space history and the
    /// composite: the repeat sampler would blend the opposite edge in.
    screen_sampler: wgpu::Sampler,
    base: wgpu::TextureView,
    detail: wgpu::TextureView,
    curl: wgpu::TextureView,
    weather: wgpu::TextureView,
    weather_key: Option<u64>,
    /// Frames of noise generation made; [`NOISE_FRAMES`] when complete.
    noise_frames: u32,
    /// The thread reading the shipped noise file, until it answers.
    loading: Option<std::sync::mpsc::Receiver<Result<crate::clouds_noise::Noise, String>>>,
    sized: Option<Sized>,
    /// The full resolution image of a still, kept for the frames of the
    /// same photo and dropped by the first live frame after it.
    still: Option<((u32, u32), wgpu::TextureView)>,
    /// The history written last (index into `Sized::history`), if valid.
    latest: Option<usize>,
    prev_vp: Mat4,
    prev_camera_km: [f64; 2],
    frame: u32,
    layout: wgpu::BindGroupLayout,
    composite: Vec<((u32, bool), wgpu::RenderPipeline)>,
    composite_group: Option<wgpu::BindGroup>,
}

fn texture(
    device: &wgpu::Device,
    label: &str,
    size: [u32; 3],
    dimension: wgpu::TextureDimension,
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: size[2],
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension,
        format,
        usage,
        view_formats: &[],
    })
}

impl Clouds {
    pub fn new(device: &wgpu::Device) -> Clouds {
        use wgpu::TextureDimension::{D2, D3};
        use wgpu::TextureFormat::Rgba8Unorm;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("clouds"),
            // (AURORA_DEMO_WATER_SHADERS=1 reads it from the source folder
            // in the demo: tune without a build)
            source: wgpu::ShaderSource::Wgsl(crate::water_shader_source(&["clouds.wgsl"], include_str!("shaders/clouds.wgsl"))),
        });
        let compute = |entry: &'static str| {
            device.create_compute_pipeline_cached(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: None,
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let pipelines = Pipelines {
            gen_base: compute("gen_base"),
            gen_detail: compute("gen_detail"),
            gen_curl: compute("gen_curl"),
            trace: compute("trace"),
            trace_still: compute("trace_still"),
            recon: compute("recon"),
        };
        let noise = wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST;
        let view = |t: &wgpu::Texture| t.create_view(&Default::default());
        let base = texture(device, "cloud base noise", [BASE_SIZE; 3], D3, Rgba8Unorm, noise);
        let detail = texture(device, "cloud detail noise", [DETAIL_SIZE; 3], D3, Rgba8Unorm, noise);
        let curl = texture(device, "cloud curl noise", [CURL_SIZE, CURL_SIZE, 1], D2, Rgba8Unorm, noise);
        let weather_tex = texture(
            device,
            "cloud weather map",
            [WEATHER_SIZE, WEATHER_SIZE, 1],
            D2,
            Rgba8Unorm,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("clouds"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let screen_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("clouds screen"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("clouds"),
            size: std::mem::size_of::<CloudU>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // the composite's group: the uniform, the image, the sampler
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("clouds composite"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 9,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 15,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        Clouds {
            module,
            pipelines,
            uniform,
            sampler,
            screen_sampler,
            base: view(&base),
            detail: view(&detail),
            curl: view(&curl),
            weather: view(&weather_tex),
            weather_key: None,
            noise_frames: 0,
            loading: None,
            sized: None,
            still: None,
            latest: None,
            prev_vp: Mat4::IDENTITY,
            prev_camera_km: [0.0; 2],
            frame: 0,
            layout,
            composite: Vec::new(),
            composite_group: None,
        }
    }

    /// Starts reading the shipped noise file, once the textures it goes
    /// into are made (a refused `new` starts no thread).
    pub fn start_noise_read(&mut self) {
        if self.loading.is_none() && self.noise_frames == 0 {
            self.loading = Some(Self::read_noise());
        }
    }

    /// The targets for `size` (a still's, or the live ones) are made
    /// (`ensure_targets` would make nothing).
    pub fn has_targets(&self, size: (u32, u32), still: bool) -> bool {
        let size = (size.0.max(2), size.1.max(2));
        if still {
            self.still.as_ref().is_some_and(|(s, _)| *s == size)
        } else {
            self.sized.as_ref().is_some_and(|s| s.size == size)
        }
    }

    /// Makes the targets for `size`, if not made (the renderer calls it
    /// inside an error scope: renderer.rs `cloud_targets`). A still keeps
    /// its own image: the live history, its size and its camera stay as the
    /// last live frame left them, so that the first live frame after the
    /// photo does not flash.
    pub fn ensure_targets(&mut self, device: &wgpu::Device, size: (u32, u32), still: bool) {
        let size = (size.0.max(2), size.1.max(2));
        if self.has_targets(size, still) {
            return;
        }
        if still {
            use wgpu::TextureFormat::Rgba16Float;
            let usage = wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING;
            let view = texture(
                device,
                "cloud still",
                [size.0, size.1, 1],
                wgpu::TextureDimension::D2,
                Rgba16Float,
                usage,
            )
            .create_view(&Default::default());
            self.still = Some((size, view));
        } else {
            self.sized = Some(Self::make_sized(device, size));
            self.latest = None;
        }
    }

    /// Drops the size-bound targets (they were refused, or half made).
    pub fn release_targets(&mut self) {
        self.sized = None;
        self.still = None;
        self.latest = None;
        self.composite_group = None;
    }

    /// The noise is complete: the clouds can be drawn this frame.
    pub fn ready(&self) -> bool {
        self.noise_frames >= NOISE_FRAMES
    }

    /// Reads the noise file off the frame thread.
    fn read_noise() -> std::sync::mpsc::Receiver<Result<crate::clouds_noise::Noise, String>> {
        let (tx, rx) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new().name("cloud-noise".into()).spawn({
            let tx = tx.clone();
            move || {
                let start = std::time::Instant::now();
                let noise = crate::clouds_noise::load();
                if noise.is_ok() {
                    log::info!("cloud noise: file read and unpacked in {} ms", start.elapsed().as_millis());
                }
                let _ = tx.send(noise);
            }
        });
        if let Err(e) = spawned {
            let _ = tx.send(Err(format!("cloud noise: no thread: {e}")));
        }
        rx
    }

    /// Takes the noise file's textures if the thread is done: uploaded, they
    /// replace the 16 frames of generation; unreadable, the generation
    /// starts (logged). Returns true while the thread is still reading.
    fn poll_noise(&mut self, queue: &wgpu::Queue) -> bool {
        use std::sync::mpsc::TryRecvError;
        let Some(rx) = &self.loading else {
            return false;
        };
        match rx.try_recv() {
            Err(TryRecvError::Empty) => return true,
            Ok(Ok(noise)) => {
                let write = |view: &wgpu::TextureView, bytes: &[u8], size: [u32; 3]| {
                    queue.write_texture(
                        wgpu::TexelCopyTextureInfo {
                            texture: view.texture(),
                            mip_level: 0,
                            origin: wgpu::Origin3d::ZERO,
                            aspect: wgpu::TextureAspect::All,
                        },
                        bytes,
                        wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(size[0] * 4),
                            rows_per_image: Some(size[1]),
                        },
                        wgpu::Extent3d {
                            width: size[0],
                            height: size[1],
                            depth_or_array_layers: size[2],
                        },
                    );
                };
                write(&self.base, &noise.base, [BASE_SIZE; 3]);
                write(&self.detail, &noise.detail, [DETAIL_SIZE; 3]);
                write(&self.curl, &noise.curl, [CURL_SIZE, CURL_SIZE, 1]);
                self.noise_frames = NOISE_FRAMES;
            }
            Ok(Err(e)) => log::warn!("{e}; the cloud noise is generated on the GPU instead"),
            Err(TryRecvError::Disconnected) => log::warn!("cloud noise: the reader stopped; generating on the GPU instead"),
        }
        self.loading = None;
        false
    }

    /// The weather map must be drawn again for `key` (the first time, or the
    /// parameters changed).
    pub fn weather_stale(&self, key: u64) -> bool {
        self.weather_key != Some(key)
    }

    pub fn weather_drawn(&mut self, key: u64) {
        self.weather_key = Some(key);
    }

    pub fn weather_target(&self) -> &wgpu::TextureView {
        &self.weather
    }

    /// Records the noise slice, the trace and the reconstruction. `depth` is
    /// the single sample depth of the prepass (the sky is what it left at 0);
    /// `size` the image's, in pixels.
    pub fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        p: &Params,
        depth: &wgpu::TextureView,
        size: (u32, u32),
        still: Option<StillSpec>,
    ) {
        let size = (size.0.max(2), size.1.max(2));
        // (the targets are made, checked, before: `ensure_targets`. A live
        // frame drops the still of the photo before it)
        self.composite_group = None;
        if still.is_none() {
            self.still = None;
        }
        let reading = self.poll_noise(queue);
        // (while the file is read, nothing is generated: it is usually done
        // within a frame or two, and the generation is the fallback)
        let step = if reading {
            NoiseStep {
                small: false,
                base_z: None,
            }
        } else {
            noise_step(self.noise_frames)
        };
        let [ox, oy] = p.offset_km();
        let moved = [0, 1].map(|i| (p.camera_km[i] - self.prev_camera_km[i]) as f32);
        let (hw, hh) = (size.0 / 2, size.1 / 2);
        let reproject = self.latest.is_some() && still.is_none();
        let u = CloudU {
            inv_vp: p.inv_vp.to_cols_array_2d(),
            prev_vp: self.prev_vp.to_cols_array_2d(),
            sun: p.light.extend(0.0).to_array(),
            sun_col: p.sun_col.extend(0.0).to_array(),
            amb_col: p.amb_col.extend(0.0).to_array(),
            haze: p.haze.extend(p.haze_km).to_array(),
            cam: [ox, oy, 0.0, 0.0],
            delta: [moved[0], moved[1], 0.0, 1.0],
            drift: [0.0, 0.0, p.tile_km, p.extinction],
            layer: [p.layer.0, p.layer.1, p.kind, debug_view() as f32],
            sizes: [size.0, size.1, hw, hh],
            misc: match still {
                Some(_) => [0, STILL_STEPS, STILL_LIGHT_STEPS, 0],
                None => [self.frame, STEPS, LIGHT_STEPS, u32::from(reproject)],
            },
            // (the noise slice, then a still's origin and sub-samples)
            gen_z: match still {
                Some(s) => [step.base_z.unwrap_or(0), s.origin.0, s.origin.1, STILL_SAMPLES],
                None => [step.base_z.unwrap_or(0), 0, 0, 0],
            },
        };
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&u));
        let group = |pipeline: &wgpu::ComputePipeline, label: &str, entries: &[wgpu::BindGroupEntry<'_>]| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &pipeline.get_bind_group_layout(0),
                entries,
            })
        };
        let tv = wgpu::BindingResource::TextureView;
        let mut pass = |label: &'static str, pipeline: &wgpu::ComputePipeline, bind: wgpu::BindGroup, groups: [u32; 3]| {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some(label),
                timestamp_writes: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(groups[0], groups[1], groups[2]);
        };
        if step.small {
            pass(
                "cloud detail noise",
                &self.pipelines.gen_detail,
                group(&self.pipelines.gen_detail, "detail", &[entry(2, tv(&self.detail))]),
                [DETAIL_SIZE / 4; 3],
            );
            pass(
                "cloud curl noise",
                &self.pipelines.gen_curl,
                group(&self.pipelines.gen_curl, "curl", &[entry(3, tv(&self.curl))]),
                [CURL_SIZE / 8, CURL_SIZE / 8, 1],
            );
        }
        if step.base_z.is_some() {
            pass(
                "cloud base noise",
                &self.pipelines.gen_base,
                group(
                    &self.pipelines.gen_base,
                    "base",
                    &[entry(0, self.uniform.as_entire_binding()), entry(1, tv(&self.base))],
                ),
                [BASE_SIZE / 4, BASE_SIZE / 4, SLICE / 4],
            );
        }
        // (the clouds are drawn from the frame after the last slice: the
        // sky shader keeps the Legacy layer until then)
        let shown = self.ready();
        if !reading {
            self.noise_frames = (self.noise_frames + 1).min(NOISE_FRAMES);
        }
        if still.is_none() {
            self.frame += 1;
        }
        if !shown {
            return;
        }
        if still.is_some() {
            let Some((_, target)) = self.still.as_ref().filter(|(s, _)| *s == size) else {
                return;
            };
            pass(
                "cloud still",
                &self.pipelines.trace_still,
                group(
                    &self.pipelines.trace_still,
                    "trace still",
                    &[
                        entry(0, self.uniform.as_entire_binding()),
                        entry(5, tv(&self.base)),
                        entry(6, tv(&self.detail)),
                        entry(7, tv(&self.curl)),
                        entry(8, tv(&self.weather)),
                        entry(9, wgpu::BindingResource::Sampler(&self.sampler)),
                        entry(16, tv(target)),
                        entry(19, tv(depth)),
                    ],
                ),
                [size.0.div_ceil(8), size.1.div_ceil(8), 1],
            );
            self.composite_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("clouds composite"),
                layout: &self.layout,
                entries: &[
                    entry(0, self.uniform.as_entire_binding()),
                    entry(9, wgpu::BindingResource::Sampler(&self.screen_sampler)),
                    entry(15, tv(target)),
                ],
            }));
            return;
        }
        let Some(sized) = self.sized.as_ref().filter(|s| s.size == size) else {
            return;
        };
        let write = self.latest.map_or(0, |i| 1 - i);
        let read = self.latest.unwrap_or(1 - write);
        let ts = (hw.div_ceil(2), hh.div_ceil(2));
        pass(
            "cloud trace",
            &self.pipelines.trace,
            group(
                &self.pipelines.trace,
                "trace",
                &[
                    entry(0, self.uniform.as_entire_binding()),
                    entry(5, tv(&self.base)),
                    entry(6, tv(&self.detail)),
                    entry(7, tv(&self.curl)),
                    entry(8, tv(&self.weather)),
                    entry(9, wgpu::BindingResource::Sampler(&self.sampler)),
                    entry(10, tv(&sized.trace)),
                    entry(11, tv(&sized.depth)),
                    entry(19, tv(depth)),
                ],
            ),
            [ts.0.div_ceil(8), ts.1.div_ceil(8), 1],
        );
        pass(
            "cloud reconstruction",
            &self.pipelines.recon,
            group(
                &self.pipelines.recon,
                "recon",
                &[
                    entry(0, self.uniform.as_entire_binding()),
                    entry(9, wgpu::BindingResource::Sampler(&self.screen_sampler)),
                    entry(13, tv(&sized.trace)),
                    entry(14, tv(&sized.depth)),
                    entry(15, tv(&sized.history[read])),
                    entry(16, tv(&sized.history[write])),
                    entry(19, tv(depth)),
                ],
            ),
            [hw.div_ceil(8), hh.div_ceil(8), 1],
        );
        self.composite_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("clouds composite"),
            layout: &self.layout,
            entries: &[
                entry(0, self.uniform.as_entire_binding()),
                entry(9, wgpu::BindingResource::Sampler(&self.screen_sampler)),
                entry(15, tv(&sized.history[write])),
            ],
        }));
        self.latest = Some(write);
        self.prev_vp = p.vp;
        self.prev_camera_km = p.camera_km;
    }

    fn make_sized(device: &wgpu::Device, size: (u32, u32)) -> Sized {
        use wgpu::TextureDimension::D2;
        use wgpu::TextureFormat::Rgba16Float;
        let (hw, hh) = (size.0 / 2, size.1 / 2);
        let ts = (hw.div_ceil(2), hh.div_ceil(2));
        let both = wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING;
        let view = |label: &str, w: u32, h: u32| texture(device, label, [w, h, 1], D2, Rgba16Float, both).create_view(&Default::default());
        Sized {
            size,
            trace: view("cloud trace", ts.0, ts.1),
            depth: view("cloud trace depth", ts.0, ts.1),
            history: [view("cloud history 0", hw, hh), view("cloud history 1", hw, hh)],
        }
    }

    /// Makes the composite pipeline for the scene pass's sample count and
    /// G-buffer, if not made yet: before the pass begins.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        samples: u32,
        gbuf: bool,
        color: wgpu::TextureFormat,
        gbuf_format: wgpu::TextureFormat,
        depth: wgpu::TextureFormat,
    ) {
        if self.composite.iter().any(|(k, _)| *k == (samples, gbuf)) {
            return;
        }
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("clouds composite"),
            bind_group_layouts: &[Some(&self.layout)],
            immediate_size: 0,
        });
        // dst * T + L, the colour channels only: the scene's alpha holds the
        // glow amount
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
        if gbuf {
            targets.push(Some(wgpu::ColorTargetState {
                format: gbuf_format,
                blend: None,
                write_mask: wgpu::ColorWrites::empty(),
            }));
        }
        let pipeline = device.create_render_pipeline_cached(&wgpu::RenderPipelineDescriptor {
            label: Some("clouds composite"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &self.module,
                entry_point: Some("vs_clouds"),
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
                entry_point: Some(if gbuf { "fs_clouds_g" } else { "fs_clouds" }),
                compilation_options: Default::default(),
                targets: &targets,
            }),
            multiview_mask: None,
            cache: None,
        });
        self.composite.push(((samples, gbuf), pipeline));
    }

    /// Draws the clouds over the sky in scene pass A, if [`Self::encode`]
    /// made them this frame. `prepare` has been called for these `samples`.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, samples: u32, gbuf: bool) {
        let (Some(group), Some((_, pipeline))) = (&self.composite_group, self.composite.iter().find(|(k, _)| *k == (samples, gbuf))) else {
            return;
        };
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, group, &[]);
        pass.draw(0..3, 0..1);
    }

    /// The clouds were not recorded this frame: nothing to draw. A still
    /// leaves the live history alone.
    pub fn skipped(&mut self, still: bool) {
        self.composite_group = None;
        drop_live(still, &mut self.latest, &mut self.still);
    }
}

/// What a skipped frame forgets: a live one drops the history and the
/// still image of the last photo; a still (a photo's frame) leaves both.
fn drop_live<T>(still: bool, latest: &mut Option<usize>, kept_still: &mut Option<T>) {
    if !still {
        *latest = None;
        *kept_still = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_skipped_live_frame_drops_the_history_and_the_still() {
        let (mut latest, mut kept) = (Some(1), Some(()));
        drop_live(true, &mut latest, &mut kept);
        assert_eq!((latest, kept), (Some(1), Some(())), "a still leaves both");
        drop_live(false, &mut latest, &mut kept);
        assert_eq!((latest, kept), (None, None));
    }

    /// `jitter_still` of the shader.
    fn still_jitter(p: (u32, u32), s: u32, n: u32) -> f32 {
        let ign = (52.982_918 * (0.06711056 * p.0 as f32 + 0.00583715 * p.1 as f32).fract()).fract();
        (ign + s as f32 / n as f32).fract()
    }

    #[test]
    fn a_tile_starts_at_its_pixel_of_the_photo() {
        assert_eq!(still_origin([0.0, 0.0, 1.0, 1.0], (1920, 1080)), (0, 0));
        // the middle tile of a 3 x 3 grid of a 1000 x 700 photo
        let part = [334.0 / 1000.0, 233.0 / 700.0, 333.0 / 1000.0, 234.0 / 700.0];
        assert_eq!(still_origin(part, (1000, 700)), (334, 233));
    }

    #[test]
    fn the_jitter_follows_the_photo_pixel_not_the_tile() {
        // the pixel (400, 250) of a photo, seen from the tile that starts at
        // (334, 233) or from the one that starts at (300, 200)
        let a = (334 + 66, 233 + 17);
        let b = (300 + 100, 200 + 50);
        assert_eq!(a, b);
        for s in 0..STILL_SAMPLES {
            assert_eq!(still_jitter(a, s, STILL_SAMPLES), still_jitter(b, s, STILL_SAMPLES));
            assert!((0.0..1.0).contains(&still_jitter(a, s, STILL_SAMPLES)));
        }
        // the sub-samples are spread over the step grid
        let j: Vec<f32> = (0..STILL_SAMPLES).map(|s| still_jitter(a, s, STILL_SAMPLES)).collect();
        assert!(j.windows(2).all(|w| w[0] != w[1]));
    }

    #[test]
    fn the_step_count_matches_the_shader() {
        let src = include_str!("shaders/clouds.wgsl");
        assert!(src.contains("let n = max(steps, min(u32(ceil(len / STEP_MAX_KM)), 96u));"));
        assert!(src.contains("const STEP_MAX_KM: f32 = 0.3;"));
        assert_eq!(march_steps(STILL_STEPS, 20.0), 128);
        assert_eq!(march_steps(STILL_STEPS, 28.8), 128);
        assert_eq!(march_steps(STEPS, 28.8), 96);
        assert_eq!(march_steps(STEPS, 100.0), 96);
        assert_eq!(march_steps(STEPS, 10.0), 64);
        assert_eq!(march_steps(STEPS, 1.0), 64);
    }

    #[test]
    fn the_uniform_matches_the_shader() {
        // U in shaders/clouds.wgsl: 2 mat4x4 + 8 vec4 + 3 vec4<u32>
        assert_eq!(std::mem::size_of::<CloudU>(), 2 * 64 + 8 * 16 + 3 * 16);
        let src = include_str!("shaders/clouds.wgsl");
        assert!(src.contains("sizes: vec4u") && src.contains("gen: vec4u"));
    }

    #[test]
    fn the_weather_size_matches_the_scene_shader() {
        let env = include_str!("shaders/env.wgsl");
        assert!(env.contains(&format!("const CLOUD_WEATHER_SIZE: f32 = {WEATHER_SIZE}.0;")));
        assert!(env.contains(&format!("const HEAP_KNEE: f32 = {HEAP_KNEE};")));
        assert!(env.contains(&format!("const HEAP_SLOPE: f32 = {HEAP_SLOPE};")));
    }

    #[test]
    fn the_noise_is_made_in_sixteen_slices_then_left_alone() {
        let steps: Vec<_> = (0..NOISE_FRAMES + 2).map(noise_step).collect();
        assert!(steps[0].small && !steps[1].small);
        let slices: Vec<u32> = steps.iter().filter_map(|s| s.base_z).collect();
        assert_eq!(slices, (0..NOISE_FRAMES).map(|i| i * SLICE).collect::<Vec<_>>());
        assert_eq!(
            slices.last().copied(),
            Some(BASE_SIZE - SLICE),
            "the last slice ends at the texture's edge"
        );
        assert!(steps[NOISE_FRAMES as usize].base_z.is_none() && !steps[NOISE_FRAMES as usize].small);
    }

    /// Quantiles of a normal law (mean, deviation), clipped to [0, 1]: a
    /// deterministic stand-in for the cloud texture's values.
    fn normal_grid(mean: f32, sd: f32, n: usize) -> Vec<f32> {
        // Abramowitz-Stegun 26.2.23 inverse of the normal CDF
        let z = |p: f32| {
            let q = if p < 0.5 { p } else { 1.0 - p };
            let t = (-2.0 * q.ln()).sqrt();
            let v = t - (2.515517 + 0.802853 * t + 0.010328 * t * t) / (1.0 + 1.432788 * t + 0.189269 * t * t + 0.001308 * t * t * t);
            if p < 0.5 { -v } else { v }
        };
        (0..n)
            .map(|i| (mean + sd * z((i as f32 + 0.5) / n as f32)).clamp(0.0, 1.0))
            .collect()
    }

    #[test]
    fn the_cover_follows_the_weather_map() {
        // the texture's statistics (mean 0.46, spread 0.21), its detail tap alike
        let main = normal_grid(0.46, 0.21, 48);
        let detail = normal_grid(0.46, 0.21, 48);
        let mean_cover = |shadow: f32| {
            let mut sum = 0.0;
            for n in &main {
                for d in &detail {
                    sum += weather_coverage(*n, *d, 1.0, shadow);
                }
            }
            sum / (main.len() * detail.len()) as f32
        };
        // The map follows the cover class: the heaps' knee (`HEAP_KNEE`)
        // keeps it below `coverage()` (the ladder of the cloud types) while
        // the sky is broken, a sheet's map is a uniform 0.55 to 1.
        let (mut last, mut last_map) = (-1.0, -1.0);
        for shadow in [0.1_f32, 0.2, 0.27, 0.36, 0.44, 0.5, 0.6] {
            let (map, fit) = (mean_cover(shadow), coverage(shadow));
            assert!(map >= last_map, "the map's cover grows with the shadow");
            assert!(
                shadow > 0.5 || (map - fit).abs() < 0.16,
                "cloud_shadow {shadow}: map {map}, coverage() {fit}"
            );
            assert!(shadow < 0.5 || map > 0.7, "a closed sky: map {map}");
            assert!(fit > last, "coverage() grows with the shadow");
            (last, last_map) = (fit, map);
        }
        // no cloud texture value gives cloud under a clear sky, all of them a closed one
        assert_eq!(weather_coverage(0.5, 0.5, 1.0, 0.0), 0.0);
        assert!(weather_coverage(0.5, 0.5, 1.0, 0.9) > 0.7);
        // heaps keep their gaps: no cover above the cap before the sheet
        assert!(weather_coverage(0.9, 0.9, 1.0, 0.37) <= HEAP_KNEE + (1.0 - HEAP_KNEE) * HEAP_SLOPE + 1e-6);
    }

    #[test]
    fn the_cover_gives_the_type() {
        // a fair sky is cumulus, a broken one stratocumulus-ish, a closed one stratus
        assert_eq!(cloud_type(0.3), 1.0);
        assert_eq!(cloud_type(0.95), 0.0);
        let broken = cloud_type(coverage(0.45));
        assert!(broken > 0.3 && broken < 1.0, "{broken}");
        // no step: monotonic
        let mut last = 1.0;
        for i in 0..=100 {
            let t = cloud_type(i as f32 / 100.0);
            assert!(t <= last + 1e-6);
            last = t;
        }
        // the Legacy default sky (cloud_shadow 0.27) is a fair sky
        assert!(coverage(0.27) < 0.3 && cloud_type(coverage(0.27)) > 0.9);
        assert_eq!(coverage(-1.0), 0.0);
        assert_eq!(coverage(5.0), 1.0);
    }

    #[test]
    fn the_layers_follow_the_real_heights() {
        assert_eq!(layer_km(0.0), (0.4, 0.9));
        assert_eq!(layer_km(1.0), (1.2, 2.0));
        let sc = layer_km(0.5);
        assert!((sc.0 - 0.6).abs() < 1e-6 && (sc.1 - 1.25).abs() < 1e-6);
        for i in 0..=20 {
            let (b, t) = layer_km(i as f32 / 20.0);
            assert!(b > 0.0 && t > b);
        }
    }

    #[test]
    fn the_weather_tiles_fill_the_period() {
        for scale in [0.0, 0.1, 0.42, 0.75, 1.0, 5.0] {
            let tile = tile_km(scale) as f64;
            let n = PERIOD_KM / tile;
            assert!((n - n.round()).abs() < 1e-4, "{scale}: {tile} km is {n} tiles of the period");
            assert!((5.0..=55.0).contains(&tile), "{tile}");
        }
        // bigger scale, bigger clouds
        assert!(tile_km(0.84) > tile_km(0.42));
    }

    #[test]
    fn the_period_holds_every_noise() {
        for km in [10.5, 5.0, 0.5, 3.0] {
            let n = PERIOD_KM / km;
            assert!((n - n.round()).abs() < 1e-9, "{km}");
        }
    }

    #[test]
    fn the_weather_map_is_drawn_again_only_for_a_change() {
        let k = weather_key(0.27, 1.0, 1.0, 12, (256, 256), 5);
        assert_eq!(k, weather_key(0.27, 1.0, 1.0, 12, (256, 256), 5));
        for other in [
            weather_key(0.28, 1.0, 1.0, 12, (256, 256), 5),
            weather_key(0.27, 0.9, 1.0, 12, (256, 256), 5),
            weather_key(0.27, 1.0, 0.9, 12, (256, 256), 5),
            weather_key(0.27, 1.0, 1.0, 13, (256, 256), 5),
            // the texture grew while it loaded
            weather_key(0.27, 1.0, 1.0, 12, (512, 512), 5),
            // the same texture, loaded again at the same size
            weather_key(0.27, 1.0, 1.0, 12, (256, 256), 6),
            weather_key(0.27, 1.0, 1.0, 12, (256, 256), 5 + (1 << 32)),
        ] {
            assert_ne!(k, other);
        }
    }

    fn sky() -> crate::types::SkyParams {
        crate::types::SkyParams::default()
    }

    #[test]
    fn the_light_follows_the_sun_and_the_cloud_colour() {
        let noon = Lighting::new(
            &crate::types::SkyParams {
                light_norm: Vec3::new(0.0, 0.3, 0.95).normalize(),
                ..sky()
            },
            Vec3::splat(0.2),
        );
        let dusk = Lighting::new(
            &crate::types::SkyParams {
                light_norm: Vec3::new(0.0, 0.95, 0.12).normalize(),
                ..sky()
            },
            Vec3::splat(0.2),
        );
        assert!(noon.sun.is_finite() && noon.ambient.is_finite());
        assert!(noon.sun.min_element() > 0.0);
        // the low sun reaches the clouds redder and dimmer
        assert!(dusk.sun.z / dusk.sun.x < noon.sun.z / noon.sun.x);
        assert!(dusk.sun.element_sum() < noon.sun.element_sum());
        // black clouds take no light
        let black = Lighting::new(
            &crate::types::SkyParams {
                cloud_color: Vec3::ZERO,
                ..sky()
            },
            Vec3::splat(0.2),
        );
        assert_eq!(black.sun, Vec3::ZERO);
        // only the sky's haze below them is left (as the Legacy layer's
        // `additiveColorBelowCloud`), blue
        assert!(
            black.ambient.z > black.ambient.x && black.ambient.max_element() < 0.1,
            "{:?}",
            black.ambient
        );
        // the sun below the horizon (night): the sun light is gone
        let night = Lighting::new(
            &crate::types::SkyParams {
                light_norm: Vec3::new(0.0, 0.0, -0.1),
                ..sky()
            },
            Vec3::splat(0.2),
        );
        assert!(night.sun.max_element() < 1e-6);
    }

    #[test]
    fn the_cloud_colour_tints_the_light() {
        let red = Lighting::new(
            &crate::types::SkyParams {
                cloud_color: Vec3::new(0.9, 0.2, 0.2),
                ..sky()
            },
            Vec3::ZERO,
        );
        assert!(red.sun.x > 2.0 * red.sun.z, "{:?}", red.sun);
        assert!(red.ambient.x > red.ambient.z, "{:?}", red.ambient);
    }

    #[test]
    fn no_density_draws_no_layer() {
        assert!(layer_drawn(0.42, 1, 1.0));
        assert!(layer_drawn(0.42, 1, 0.05));
        assert!(!layer_drawn(0.42, 1, 0.0));
        assert!(!layer_drawn(0.42, 1, 0.001));
        assert!(!layer_drawn(0.0, 1, 1.0));
        assert!(!layer_drawn(0.42, 0, 1.0));
    }

    #[test]
    fn extinction_fades_to_nothing_with_the_density() {
        assert_eq!(extinction(0.0), 0.0);
        assert_eq!(extinction(-1.0), 0.0);
        let mut last = -1.0;
        for i in 0..=40 {
            let e = extinction(i as f32 * 0.05);
            assert!(e >= last, "{e} < {last} at {i}");
            last = e;
        }
        assert_eq!(extinction(1.0), 1.0);
        assert_eq!(extinction(9.0), 1.5);
    }

    #[test]
    fn the_offset_wraps_in_the_period() {
        let p = Params {
            vp: Mat4::IDENTITY,
            inv_vp: Mat4::IDENTITY,
            light: Vec3::Z,
            sun_col: Vec3::ONE,
            amb_col: Vec3::ONE,
            haze: Vec3::ONE,
            haze_km: 40.0,
            camera_km: [1000.0 * PERIOD_KM + 3.0, -2.0],
            pattern_km: [0.5, 0.0],
            layer: (1.2, 3.0),
            kind: 1.0,
            tile_km: 26.25,
            extinction: 1.0,
            weather_key: 0,
        };
        let [x, y] = p.offset_km();
        assert!((x - 3.5).abs() < 1e-3, "{x}");
        assert!((y - (PERIOD_KM as f32 - 2.0)).abs() < 1e-3, "{y}");
    }

    #[test]
    fn the_clouds_shader_is_valid_for_vulkan_and_metal() {
        let source = include_str!("shaders/clouds.wgsl");
        let module = naga::front::wgsl::parse_str(source).unwrap_or_else(|e| panic!("{}", e.emit_to_string(source)));
        let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::empty())
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
        assert_eq!(module.entry_points.len(), 9);
    }
}
