//! What the last frame drew with, for the motion vectors: each record's
//! model matrix and the joint palettes as they were. At the end of a frame
//! the renderer gathers the records' matrices into `SceneGpu::prev_models`
//! and copies the palettes into `SceneGpu::prev_palettes`, after every pass
//! that reads them; the next frame's queue writes then change the current
//! ones only. A frame whose scene moved as a whole (`motion_cut`) takes
//! the current ones as the previous ones before it draws.
//!
//! What the main thread seeds itself (main_thread.rs, arena.rs): a record
//! handed out starts with its own model, a palette slot given to a new
//! skeleton with its first pose, and grown buffers keep or restart theirs.

use crate::packet::SceneGpu;
use crate::pipeline_cache::CachedPipelines;

pub(crate) struct MotionHistory {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    /// The gather's bind group, and the `SceneGpu::records_key` it was made for.
    bind_group: Option<(wgpu::BindGroup, (u64, u64))>,
}

fn storage_entry(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

impl MotionHistory {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("motion history"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/motion_history.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("motion history"),
            entries: &[storage_entry(0, true), storage_entry(1, false)],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("motion history"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline_cached(&wgpu::ComputePipelineDescriptor {
            label: Some("motion history"),
            layout: Some(&pl),
            module: &module,
            entry_point: Some("gather"),
            compilation_options: Default::default(),
            cache: None,
        });
        MotionHistory {
            pipeline,
            layout,
            bind_group: None,
        }
    }

    /// Record the copy of the current matrices into the previous ones.
    pub(crate) fn encode(&mut self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, scene: &SceneGpu) {
        if self.bind_group.as_ref().is_none_or(|(_, key)| *key != scene.records_key) {
            let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("motion history"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: scene.records.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: scene.prev_models.as_entire_binding(),
                    },
                ],
            });
            self.bind_group = Some((bg, scene.records_key));
        }
        let groups = workgroups(scene.record_slots);
        if groups > 0
            && let Some((bg, _)) = &self.bind_group
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("motion history"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, bg, &[]);
            pass.dispatch_workgroups(groups, 1, 1);
        }
        // (the previous palettes are smaller only when video memory was
        // full as they grew)
        let bytes = scene.palettes.size().min(scene.prev_palettes.size());
        encoder.copy_buffer_to_buffer(&scene.palettes, 0, &scene.prev_palettes, 0, bytes);
    }
}

/// Workgroups of 64 that cover `slots` records, within one dispatch
/// dimension.
fn workgroups(slots: usize) -> u32 {
    slots.div_ceil(64).min(65535) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_gather_reads_a_whole_draw_record() {
        // motion_history.wgsl skips what follows the model matrix
        let rest = (std::mem::size_of::<crate::types::DrawRecord>() - 64) / 16;
        let src = include_str!("shaders/motion_history.wgsl");
        assert!(
            src.contains(&format!("rest: array<vec4<f32>, {rest}>")),
            "the gather's Record skips {rest} vectors"
        );
    }

    #[test]
    fn workgroups_cover_every_slot() {
        assert_eq!(workgroups(0), 0);
        assert_eq!(workgroups(1), 1);
        assert_eq!(workgroups(64), 1);
        assert_eq!(workgroups(65), 2);
        assert_eq!(workgroups(usize::MAX / 2), 65535);
    }
}
