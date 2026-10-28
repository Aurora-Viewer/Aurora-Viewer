//! Automatic exposure of skies that are not classic, like the Legacy
//! Viewer's dynamic exposure (LLPipeline::generateLuminance /
//! generateExposure, pipeline.cpp, originally LGPL 2.1): each frame the
//! average luminance of the middle of the scene picks an exposure between
//! 1 / hdr_scale and hdr_scale of the sky (`sqrt(gamma) x 2`), eased towards
//! over about two seconds, and the post pass multiplies Preferences'
//! exposure by it. The Legacy Viewer reduces a 256 x 256 luminance target by
//! its mip chain and reads one texel; here one compute workgroup averages
//! the same grid (`shaders/exposure.wgsl`) and the result is copied into the
//! post pass's uniforms, with no read back to the CPU. Classic skies (probe
//! ambiance 0) get the exposure 1, as there.

use bytemuck::{Pod, Zeroable};
use glam::Mat4;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant};

/// RenderDynamicExposureCoefficient (settings.xml): the average luminance
/// at which the exposure reaches its lowest value.
pub(crate) const COEFFICIENT: f32 = 0.5;
/// RenderDiffuseLuminanceScale (settings.xml): the weight of everything but
/// the sky in that average.
pub(crate) const DIFFUSE_SCALE: f32 = 0.5;
/// RenderDynamicExposureSpeedError / RenderDynamicExposureSpeedTarget
/// (pipeline.cpp defaults): what is left of a change after that many
/// seconds.
const SPEED_ERROR: f32 = 0.1;
const SPEED_TARGET: f32 = 2.0;
/// How often the exposure is read back for the log (to compare with the
/// Legacy Viewer on the grid), and how long after a change of sky.
const LOG_INTERVAL: Duration = Duration::from_secs(30);
const LOG_AFTER_CHANGE: Duration = Duration::from_secs(4);

const IDLE: u8 = 0;
const ENCODED: u8 = 1;
const WAITING: u8 = 2;
const MAPPED: u8 = 3;
const FAILED: u8 = 4;

/// The range of the exposure under a sky (generateExposure without
/// RenderUseExposureSkySettings): [1 / hdr_scale, hdr_scale] on a sky with
/// a probe ambiance whose hdr scale is above 1, else exactly 1.
pub(crate) fn range(tone_mapped: bool, hdr_scale: f32) -> (f32, f32) {
    if tone_mapped && hdr_scale > 1.0 {
        (1.0 / hdr_scale, hdr_scale)
    } else {
        (1.0, 1.0)
    }
}

/// exposureF.glsl's target for an average luminance (mirrors the shader,
/// for the tests).
#[cfg(test)]
fn target(average: f32, (lo, hi): (f32, f32)) -> f32 {
    let l = average.clamp(0.0, COEFFICIENT) / COEFFICIENT;
    hi + (lo - hi) * l * l
}

/// `AURORA_AUTO_EXPOSURE`: `0` off (exposure 1 on every sky), `1` or unset
/// on, `alt` on one second out of two (to compare in the same session).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Override {
    On,
    Off,
    Alternate,
}

impl Override {
    pub(crate) fn parse(v: Option<&str>) -> Override {
        match v.map(str::trim) {
            Some("0") | Some("off") => Override::Off,
            Some("alt") => Override::Alternate,
            _ => Override::On,
        }
    }

    fn from_env() -> Override {
        static ENV: std::sync::OnceLock<Override> = std::sync::OnceLock::new();
        *ENV.get_or_init(|| Override::parse(std::env::var("AURORA_AUTO_EXPOSURE").ok().as_deref()))
    }

    /// Whether the automatic exposure is on at `secs` seconds into the
    /// session.
    fn on(self, secs: u64) -> bool {
        match self {
            Override::On => true,
            Override::Off => false,
            Override::Alternate => secs.is_multiple_of(2),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ExposureU {
    inv_view_proj: [[f32; 4]; 4],
    a: [f32; 4],
    b: [f32; 4],
}

/// What the frame needs from the automatic exposure.
pub(crate) struct FrameExposure {
    /// The sky is tone mapped (probe ambiance above 0).
    pub(crate) tone_mapped: bool,
    /// "HDR and Emissive" (`RenderSettings::hdr`): off, the exposure is 1
    /// at once, without the easing.
    pub(crate) hdr: bool,
    /// Its hdr scale (`SkyParams::hdr_scale`).
    pub(crate) hdr_scale: f32,
    /// A photo: keep the exposure of the live view.
    pub(crate) hold: bool,
    /// The glow of the last frame is there to add (RenderGlow).
    pub(crate) glow: bool,
    pub(crate) inv_view_proj: Mat4,
}

pub(crate) struct Exposure {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::ComputePipeline,
    params: wgpu::Buffer,
    state: wgpu::Buffer,
    bg: Option<wgpu::BindGroup>,
    last: Option<Instant>,
    start: Instant,
    /// The automatic exposure was off last frame (`AURORA_AUTO_EXPOSURE`).
    was_off: bool,
    /// The log's read back: a copy of `state`, its map state, when the next
    /// one is due, and the range it was taken with.
    readback: wgpu::Buffer,
    read_state: Arc<AtomicU8>,
    next_log: Instant,
    range: (f32, f32),
    read_range: (f32, f32),
}

impl Exposure {
    pub(crate) fn new(device: &wgpu::Device) -> Exposure {
        let tex = |binding, sample_type| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Texture {
                sample_type,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("auto exposure"),
            entries: &[
                tex(0, wgpu::TextureSampleType::Float { filterable: false }),
                tex(1, wgpu::TextureSampleType::Depth),
                tex(2, wgpu::TextureSampleType::Float { filterable: false }),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("auto exposure"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/exposure.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("auto exposure"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("auto exposure"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("cs_exposure"),
            compilation_options: Default::default(),
            cache: None,
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("auto exposure params"),
            size: std::mem::size_of::<ExposureU>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // starts at 0: the first update takes its target at once
        let state = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("auto exposure state"),
            size: 16,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("auto exposure read back"),
            size: 16,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let now = Instant::now();
        Exposure {
            layout,
            pipeline,
            params,
            state,
            bg: None,
            last: None,
            start: now,
            was_off: false,
            readback,
            read_state: Arc::new(AtomicU8::new(IDLE)),
            next_log: now + LOG_AFTER_CHANGE,
            range: (1.0, 1.0),
            read_range: (1.0, 1.0),
        }
    }

    /// Maps the read back copied this frame, once its commands are
    /// submitted (no wait: the log line comes a few frames later).
    pub(crate) fn after_submit(&mut self) {
        if self.read_state.load(Ordering::Acquire) != ENCODED {
            return;
        }
        self.read_state.store(WAITING, Ordering::Release);
        let state = self.read_state.clone();
        self.readback.map_async(wgpu::MapMode::Read, .., move |r| {
            state.store(if r.is_ok() { MAPPED } else { FAILED }, Ordering::Release);
        });
    }

    /// Logs the exposure the GPU delivered, and copies the next one when
    /// due.
    fn log(&mut self, encoder: &mut wgpu::CommandEncoder, now: Instant) {
        match self.read_state.load(Ordering::Acquire) {
            MAPPED => {
                if let Ok(view) = self.readback.get_mapped_range(..)
                    && let Some(b) = view.get(..12)
                {
                    let f = |i: usize| f32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
                    let (lo, hi) = self.read_range;
                    log::info!(
                        "auto exposure: {:.3} (target {:.3}, average luminance {:.3}, range {lo:.3} to {hi:.3})",
                        f(0),
                        f(4),
                        f(8)
                    );
                }
                self.readback.unmap();
                self.read_state.store(IDLE, Ordering::Release);
            }
            FAILED => self.read_state.store(IDLE, Ordering::Release),
            IDLE if now >= self.next_log => {
                encoder.copy_buffer_to_buffer(&self.state, 0, &self.readback, 0, 16);
                self.read_state.store(ENCODED, Ordering::Release);
                self.read_range = self.range;
                self.next_log = now + LOG_INTERVAL;
            }
            _ => {}
        }
    }

    /// Binds the scene image (resolved, before the tone mapping), its depth
    /// prepass and the glow; again whenever the targets are made anew.
    pub(crate) fn bind(&mut self, device: &wgpu::Device, scene: &wgpu::TextureView, depth: &wgpu::TextureView, glow: &wgpu::TextureView) {
        let view = |binding, v| wgpu::BindGroupEntry {
            binding,
            resource: wgpu::BindingResource::TextureView(v),
        };
        self.bg = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("auto exposure"),
            layout: &self.layout,
            entries: &[
                view(0, scene),
                view(1, depth),
                view(2, glow),
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: self.state.as_entire_binding(),
                },
            ],
        }));
    }

    /// Updates the exposure from the scene image and copies it to the 4
    /// bytes at `offset` of `post` (the post pass's uniforms, which the
    /// glow, the bloom and the tone mapping read afterwards).
    pub(crate) fn encode(
        &mut self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        f: &FrameExposure,
        post: &wgpu::Buffer,
        offset: u64,
    ) {
        let Some(bg) = &self.bg else {
            return;
        };
        let now = Instant::now();
        let dt = self.last.map_or(0.0, |t| now.duration_since(t).as_secs_f32());
        if !f.hold {
            self.last = Some(now);
        }
        let on = Override::from_env().on(now.duration_since(self.start).as_secs());
        let (lo, hi) = if on { range(f.tone_mapped, f.hdr_scale) } else { (1.0, 1.0) };
        if (lo, hi) != self.range && !f.hold {
            self.range = (lo, hi);
            self.next_log = self.next_log.min(now + LOG_AFTER_CHANGE);
        }
        // an A/B switch shows at once, without the easing; and without HDR
        // there is no exposure to ease (LLPipeline::renderFinalize skips
        // generateExposure): 1 at once
        let reset = on == self.was_off || Override::from_env() == Override::Alternate || !f.hdr;
        self.was_off = !on;
        let u = ExposureU {
            inv_view_proj: f.inv_view_proj.to_cols_array_2d(),
            a: [dt, DIFFUSE_SCALE, if f.glow { 1.0 } else { 0.0 }, if reset { 1.0 } else { 0.0 }],
            b: [COEFFICIENT, lo, hi, if f.hold { -1.0 } else { -SPEED_ERROR.ln() / SPEED_TARGET }],
        };
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&u));
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("auto exposure"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, bg, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&self.state, 0, post, offset, 4);
        if !f.hold {
            self.log(encoder, now);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classic_skies_keep_the_exposure_one() {
        assert_eq!(range(false, 2.0), (1.0, 1.0));
        // a gamma of 0.25 or less gives no range either
        assert_eq!(range(true, 1.0), (1.0, 1.0));
        assert_eq!(range(true, 0.5), (1.0, 1.0));
    }

    #[test]
    fn midday_ranges_from_half_to_twice() {
        // the library Midday: gamma 1, hdr scale sqrt(1) x 2
        let r = range(true, 2.0);
        assert_eq!(r, (0.5, 2.0));
        // a dark scene opens up, a bright one closes down, on a square curve
        assert_eq!(target(0.0, r), 2.0);
        assert_eq!(target(COEFFICIENT, r), 0.5);
        assert_eq!(target(10.0, r), 0.5);
        assert!((target(COEFFICIENT * 0.5, r) - 1.625).abs() < 1e-6);
    }

    #[test]
    fn override_parses() {
        assert_eq!(Override::parse(None), Override::On);
        assert_eq!(Override::parse(Some("0")), Override::Off);
        assert_eq!(Override::parse(Some(" alt ")), Override::Alternate);
        assert!(Override::Alternate.on(4));
        assert!(!Override::Alternate.on(5));
    }
}
