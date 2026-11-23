//! The traced global illumination's probes in the frame (gi.rs): the rays
//! of each frame's probes (shaders/gi_trace.wgsl) and the volume's update,
//! before the traced indirect light, which reads the volume at the hits its
//! view does not see (shaders/gi_hit.wgsl). The probes' own hits read it as
//! well, so the light bounces once more at each update.
//!
//! Majercik et al., "Dynamic Diffuse Global Illumination with Ray-Traced
//! Irradiance Fields" (JCGT 2019) and "Scaling Probe-Based Real-Time
//! Dynamic Global Illumination for Production" (JCGT 2021), written from the
//! papers. The Legacy Viewer has no traced or probe-based indirect light:
//! its ambient is the sky's (the indirect light's own origin, rt_gi.wgsl).

use crate::gi::{self, Clipmap, GiUniforms, GiVolume};
use crate::pipeline_cache::CachedPipelines;
use crate::rt_shadows::{self, Round, Timer};
use bytemuck::{Pod, Zeroable};

/// The probes' rays' constants (must match `TraceU` in gi_trace.wgsl).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct TraceU {
    lit: [[f32; 4]; 5],
    lamps: [f32; 4],
    frame: [u32; 4],
}

/// What the probes' rays see this frame, from the traced indirect light's.
pub(crate) struct ProbeFrame<'a> {
    pub camera: glam::Vec3,
    /// rt_light.wgsl's `HitLight`.
    pub lit: [[f32; 4]; 5],
    pub lamps: u32,
    pub lamps_shadowed: bool,
    /// The rays' reach (m).
    pub reach: f32,
    pub scene_tlas: &'a wgpu::Tlas,
    /// None without posed faces.
    pub posed_tlas: Option<&'a wgpu::Tlas>,
    pub lights: &'a wgpu::Buffer,
    pub scene_group: &'a wgpu::BindGroup,
    pub textures: &'a wgpu::BindGroup,
    /// The counters of `AURORA_RT_GI_DIAG=count` (rt_gi_counts.rs), when the
    /// tracer was made `counted`.
    pub counts: Option<&'a wgpu::Buffer>,
}

pub(crate) struct GiProbes {
    pub volume: GiVolume,
    clipmap: Clipmap,
    layout: wgpu::BindGroupLayout,
    trace: wgpu::ComputePipeline,
    buffer: wgpu::Buffer,
    /// `AURORA_GI=0`: the probes stay off whatever the settings say.
    forced_off: bool,
    /// Preferences › Graphics › Ray tracing › "Global illumination probes"
    /// (`RtTuning::probes`, on by default).
    wanted: bool,
    /// The probes updated a frame (`RtTuning::gi_probe_updates`, the ray
    /// tracing quality), within the volume's budget.
    updates: u32,
    /// `AURORA_RT_GI_VIS`: which reads of a probe trace its visibility ray
    /// (gi_sample.wgsl `GI_VIS_*`, `gi.frame.z`; a diagnosis).
    vis: u32,
    /// `AURORA_RT_GI_DIAG=coarse`: the first cascade is not read
    /// (gi_sample.wgsl `GI_COARSE`, `gi.frame.w`; a diagnosis).
    coarse: bool,
    /// The profile's GPU time of the rays and of the update.
    timer: Option<Timer>,
    /// Frames left in which the probes start again from nothing: the
    /// world's origin changed, and the scene writes every face at its new
    /// place over the next frames, so the light gathered in between would
    /// be of a scene half moved ([`REBASE_FRAMES`]).
    settling: u32,
}

/// The frames after a change of the world's origin (a border crossing, a
/// teleport) in which every probe starts again at each frame: the light of
/// a scene half moved lasts one frame at most, the hits take the sky's
/// hemisphere elsewhere, then the probes gather the new place's light.
const REBASE_FRAMES: u32 = 30;

/// The probes a frame updates for `RtTuning::gi_probe_updates`: one at
/// least (0 would never update them: the switch turns them off), the
/// volume's `budget` at most.
pub(crate) fn probe_updates(wanted: u32, budget: u32) -> u32 {
    wanted.clamp(1, budget.max(1))
}

/// `AURORA_RT_GI_VIS` (docs/TESTING.md), to compare: `all`, every read of
/// a probe traces its visibility ray; `p1`, only where the probe's depth
/// may be wrong; `p6` or nothing, the viewer's, those of them that weigh
/// enough (gi_sample.wgsl `gi_tiny`).
fn vis_by_env() -> u32 {
    let asked = std::env::var("AURORA_RT_GI_VIS").ok();
    let vis = vis_named(asked.as_deref().unwrap_or(""));
    if let Some(v) = asked.filter(|_| vis != VIS_TINY) {
        log::warn!(
            "gi probes: visibility rays {} (AURORA_RT_GI_VIS={}, a diagnosis)",
            VIS_NAMES[vis as usize],
            v.trim()
        );
    }
    vis
}

/// gi_sample.wgsl's `GI_VIS_*`, and their names in the log.
const VIS_TINY: u32 = 0;
const VIS_ALL: u32 = 1;
const VIS_SURE: u32 = 2;
const VIS_NAMES: [&str; 3] = [
    "where the depth is not sure, but tiny probes",
    "at every read",
    "where the depth is not sure",
];

/// `AURORA_RT_GI_VIS`'s value; the viewer's for anything else.
fn vis_named(name: &str) -> u32 {
    match name.trim().to_ascii_lowercase().as_str() {
        "all" => VIS_ALL,
        "p1" => VIS_SURE,
        _ => VIS_TINY,
    }
}

/// `AURORA_GI=0` leaves the probes out whatever the settings say: the hits
/// take the sky's hemisphere, as before them (docs/TESTING.md).
fn forced_off_by_env() -> bool {
    std::env::var("AURORA_GI").is_ok_and(|v| v.trim() == "0")
}

impl GiProbes {
    /// `counted`: with the counts of `AURORA_RT_GI_DIAG=count`, whose
    /// counters each frame's [`ProbeFrame::counts`] brings.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene_layout: &wgpu::BindGroupLayout,
        textures: &wgpu::BindGroupLayout,
        counted: bool,
    ) -> Self {
        let volume = GiVolume::new(device, gi::BUDGET);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gi trace"),
            source: wgpu::ShaderSource::Wgsl(trace_source(counted).into()),
        });
        let mut entries = vec![
            rt_shadows::uniform_entry::<TraceU>(),
            rt_shadows::tlas_entry(1),
            rt_shadows::tlas_entry(2),
            rt_shadows::storage_entry(3, true),
            rt_shadows::storage_entry(4, false),
        ];
        // (binding 24: the counters, as in rt_gi.wgsl's passes)
        if counted {
            entries.push(rt_shadows::storage_entry(24, false));
        }
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gi trace"),
            entries: &entries,
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gi trace"),
            bind_group_layouts: &[Some(&layout), Some(scene_layout), Some(textures), Some(&volume.sample_layout)],
            immediate_size: 0,
        });
        let trace = device.create_compute_pipeline_cached(&wgpu::ComputePipelineDescriptor {
            label: Some("gi trace"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("cs_gi_trace"),
            compilation_options: Default::default(),
            cache: None,
        });
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gi trace"),
            size: std::mem::size_of::<TraceU>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let forced_off = forced_off_by_env();
        log::info!(
            "ray tracing: global illumination probes {} ({} MB, {} probes a frame)",
            if forced_off { "off (AURORA_GI=0)" } else { "ready" },
            gi::memory_bytes(gi::BUDGET) / (1024 * 1024),
            gi::BUDGET
        );
        GiProbes {
            volume,
            clipmap: Clipmap::default(),
            layout,
            trace,
            buffer,
            forced_off,
            wanted: true,
            updates: gi::BUDGET,
            vis: vis_by_env(),
            coarse: rt_shadows::gi_diag_coarse(),
            timer: Timer::new(device, queue, 3),
            settling: 0,
        }
    }

    /// Whether the probes are traced: the setting on, and `AURORA_GI` not 0.
    pub fn enabled(&self) -> bool {
        self.wanted && !self.forced_off
    }

    /// Follows the setting (`RtTuning::probes`), at each frame. Turned on
    /// again, the probes start from nothing: what they held is of the
    /// place and the light of the moment they were turned off.
    pub fn set_wanted(&mut self, on: bool) {
        if on != self.wanted {
            log::info!("ray tracing: global illumination probes {}", if on { "on" } else { "off" });
            if on {
                self.clipmap.reset();
            }
            self.wanted = on;
        }
    }

    /// Follows the ray tracing quality (`RtTuning::gi_probe_updates`): the
    /// probes updated a frame, the volume's budget at most (the rays'
    /// buffer is made for it). Fewer make each probe's light follow a
    /// change more slowly, at the same resolution.
    pub fn set_updates(&mut self, updates: u32) {
        let updates = probe_updates(updates, self.volume.budget());
        if updates != self.updates {
            log::info!("ray tracing: global illumination probes, {updates} a frame");
            self.updates = updates;
        }
    }

    /// The world's origin changed (`Faces::rebased`): every probe starts
    /// again, once the scene's faces are at their new places.
    pub fn rebase(&mut self) {
        self.settling = REBASE_FRAMES;
    }

    /// Traces this frame's probes and folds their rays into the volume.
    /// Each round of a photo updates them again, from the same place: the
    /// probes converge further, and the indirect light of every round reads
    /// what the one before left.
    pub fn encode(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, f: &ProbeFrame, round: Round) {
        if let Some(timer) = self.timer.as_mut().filter(|_| round.first())
            && let Some((ms, frames)) = timer.harvest()
        {
            log::info!(
                "gi probes: trace {:.3} ms, update {:.3} ms (GPU, mean of {frames} frames, {} probes a frame)",
                ms[0],
                ms[1],
                self.updates
            );
        }
        if self.settling > 0 && round.first() {
            self.settling -= 1;
            self.clipmap.reset();
        }
        let enabled = self.enabled();
        let budget = if enabled { self.updates } else { 0 };
        let mut u: GiUniforms = self.volume.clamped(&self.clipmap.advance(f.camera, budget, enabled));
        u.frame[2] = self.vis;
        u.frame[3] = u32::from(self.coarse);
        rt_shadows::put(device, queue, encoder, &self.volume.uniforms, bytemuck::bytes_of(&u), round);
        let rays = u.frame[0] * gi::RAYS;
        if rays == 0 {
            return;
        }
        let t = TraceU {
            lit: f.lit,
            lamps: [
                f.lamps as f32,
                if f.lamps_shadowed { 1.0 } else { 0.0 },
                f.reach,
                if f.posed_tlas.is_some() { 1.0 } else { 0.0 },
            ],
            frame: [self.clipmap.frame() as u32, 0, 0, 0],
        };
        rt_shadows::put(device, queue, encoder, &self.buffer, bytemuck::bytes_of(&t), round);
        let counters = f.counts.map(|c| wgpu::BindGroupEntry {
            binding: 24,
            resource: c.as_entire_binding(),
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gi trace"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::AccelerationStructure(f.scene_tlas),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    // without posed faces the scene's stands in, unread
                    resource: wgpu::BindingResource::AccelerationStructure(f.posed_tlas.unwrap_or(f.scene_tlas)),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: f.lights.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: self.volume.rays.as_entire_binding(),
                },
            ]
            .into_iter()
            .chain(counters)
            .collect::<Vec<_>>(),
        });
        // (a photo's later rounds are not timed: the frame's count is one)
        let timer = self.timer.as_ref().filter(|_| round.first());
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("gi trace"),
                timestamp_writes: timer.map(|timer| wgpu::ComputePassTimestampWrites {
                    query_set: &timer.queries,
                    beginning_of_pass_write_index: Some(0),
                    end_of_pass_write_index: None,
                }),
            });
            pass.set_pipeline(&self.trace);
            pass.set_bind_group(0, &group, &[]);
            pass.set_bind_group(1, f.scene_group, &[]);
            pass.set_bind_group(2, f.textures, &[]);
            pass.set_bind_group(gi::SAMPLE_GROUP, &self.volume.sample_group, &[]);
            pass.dispatch_workgroups(rays.div_ceil(64), 1, 1);
        }
        self.volume.encode(
            encoder,
            &u,
            timer.map(|timer| wgpu::ComputePassTimestampWrites {
                query_set: &timer.queries,
                beginning_of_pass_write_index: Some(1),
                end_of_pass_write_index: Some(2),
            }),
        );
        if let Some(timer) = self.timer.as_mut().filter(|_| round.first()) {
            timer.resolve(encoder);
        }
    }
}

/// The probes' rays' shader: rt_common.wgsl, rt_light.wgsl, the volume's
/// read bound at its group, then gi_trace.wgsl; `counted`, with the hooks
/// of `AURORA_RT_GI_DIAG=count` (rt_gi_counts.rs).
pub(crate) fn trace_source(counted: bool) -> String {
    let read = if counted {
        gi::bound_sample_source_with(&crate::rt_gi_counts::hooks())
    } else {
        gi::bound_sample_source()
    };
    rt_shadows::source(&[rt_shadows::RT_LIGHT, &read, include_str!("shaders/gi_trace.wgsl")])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `AURORA_RT_GI_VIS`'s names, the viewer's rule for anything else.
    #[test]
    fn vis_names() {
        assert_eq!(vis_named(" All "), VIS_ALL);
        assert_eq!(vis_named("p1"), VIS_SURE);
        assert_eq!(vis_named("p6"), VIS_TINY);
        assert_eq!(vis_named(""), VIS_TINY);
        assert_eq!(vis_named("none"), VIS_TINY);
        assert_eq!(VIS_NAMES.len(), 3);
    }
}
