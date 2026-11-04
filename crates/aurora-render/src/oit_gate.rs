//! The OIT's blended draws per band, without counted indirect draws.
//!
//! The band plan (oit.rs, oit.wgsl `cs_band_plan`) decides on the GPU how
//! many bands of rows the frame's blended faces are captured in. The
//! capture of a band is a draw of the whole blend list, clipped to the
//! band's rows, and a band the plan did not use must draw nothing, with no
//! wait on the CPU. A count read from the GPU
//! (`multi_draw_indexed_indirect_count`) would do it, but Metal has none in
//! wgpu. Here a compute pass, right after the plan, copies the arguments of
//! the blended draws once per band: a copy keeps its instances only when
//! its band has rows. The captures are then plain indirect draws of a copy,
//! on every backend.
use crate::pipeline_cache::CachedPipelines;
use bytemuck::{Pod, Zeroable};

/// Bytes of a DrawIndexedIndirect.
const STRIDE: u64 = 20;
const WORKGROUP: u32 = 64;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, PartialEq)]
struct Params {
    /// draws in each copy
    total: u32,
    /// copies (the bands)
    bands: u32,
    /// first draw in the indirect buffer
    src: u32,
    _pad: u32,
}

/// Byte offset in the copies of band `band`'s draws `offset..` (`count` of
/// them) of the indirect buffer, when the range holds them all.
fn locate(src: u32, total: u32, bands: u32, band: u32, offset: u64, count: u32) -> Option<u64> {
    if !offset.is_multiple_of(STRIDE) || band >= bands || total == 0 {
        return None;
    }
    let first = u32::try_from(offset / STRIDE).ok()?;
    if first < src || u64::from(first) + u64::from(count) > u64::from(src) + u64::from(total) {
        return None;
    }
    Some((u64::from(band) * u64::from(total) + u64::from(first - src)) * STRIDE)
}

pub(crate) struct Gate {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    params: wgpu::Buffer,
    last_params: Option<Params>,
    src: u32,
    total: u32,
    bands: u32,
    /// the copies, one per band
    args: Option<wgpu::Buffer>,
    /// the group and the buffers it binds (the indirect buffer and the
    /// pool's bands are made again when they grow)
    group: Option<(wgpu::BindGroup, wgpu::Buffer, wgpu::Buffer)>,
}

impl Gate {
    pub fn new(device: &wgpu::Device) -> Self {
        let stage = wgpu::ShaderStages::COMPUTE;
        let storage = |binding, read_only| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: stage,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("oit gate"),
            entries: &[
                storage(0, true),
                storage(1, true),
                storage(2, false),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: stage,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("oit gate"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/oit_gate.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("oit gate"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline_cached(&wgpu::ComputePipelineDescriptor {
            label: Some("oit gate"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("cs_gate"),
            compilation_options: Default::default(),
            cache: None,
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("oit gate parameters"),
            size: std::mem::size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Gate {
            pipeline,
            layout,
            params,
            last_params: None,
            src: 0,
            total: 0,
            bands: 0,
            args: None,
            group: None,
        }
    }

    /// The copies ready for this frame's `range` of `indirect` (byte
    /// offset, draws), one per band of `bands` (the pool's band table).
    /// `false` when their buffer cannot be made (video memory full): the
    /// faces are then blended as before.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        indirect: &wgpu::Buffer,
        bands: &wgpu::Buffer,
        range: (u64, u32),
        max_bands: u32,
    ) -> bool {
        self.src = u32::try_from(range.0 / STRIDE).unwrap_or(u32::MAX);
        self.total = range.1;
        self.bands = max_bands;
        let needed = (u64::from(self.total) * u64::from(max_bands) * STRIDE).max(STRIDE);
        if self.args.as_ref().is_none_or(|b| b.size() < needed) {
            let size = needed.next_power_of_two();
            match crate::gpu_alloc::try_buffer(
                device,
                &wgpu::BufferDescriptor {
                    label: Some("oit gated draws"),
                    size,
                    // (COPY_DST: `try_buffer` checks a buffer by clearing it)
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                },
            ) {
                Ok(buffer) => {
                    self.args = Some(buffer);
                    self.group = None;
                }
                Err(e) => {
                    log::warn!("oit: no gated draws buffer ({e})");
                    self.args = None;
                    return false;
                }
            }
        }
        let Some(args) = &self.args else {
            return false;
        };
        if self.group.as_ref().is_none_or(|(_, i, b)| i != indirect || b != bands) {
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("oit gate"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: indirect.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: bands.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: args.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: self.params.as_entire_binding(),
                    },
                ],
            });
            self.group = Some((group, indirect.clone(), bands.clone()));
        }
        let params = Params {
            total: self.total,
            bands: max_bands,
            src: self.src,
            _pad: 0,
        };
        if self.last_params != Some(params) {
            queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&params));
            self.last_params = Some(params);
        }
        true
    }

    /// Fills the copies, in the pass of the band plan, after it.
    pub fn dispatch(&self, pass: &mut wgpu::ComputePass) {
        if self.total == 0 {
            return;
        }
        if let Some((group, ..)) = &self.group {
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, group, &[]);
            pass.dispatch_workgroups((self.total * self.bands).div_ceil(WORKGROUP), 1, 1);
        }
    }

    /// The copy and the offset of band `band`'s draws `offset..` of the
    /// indirect buffer (`count` of them), when they are gated this frame.
    pub fn args(&self, band: u32, offset: u64, count: u32) -> Option<(&wgpu::Buffer, u64)> {
        let at = locate(self.src, self.total, self.bands, band, offset, count)?;
        Some((self.args.as_ref()?, at))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_band_has_its_copy_of_the_blend_list() {
        // the blend list at draw 100 (40 draws), 4 bands
        assert_eq!(locate(100, 40, 4, 0, 100 * STRIDE, 40), Some(0));
        assert_eq!(locate(100, 40, 4, 1, 100 * STRIDE, 40), Some(40 * STRIDE));
        // a run inside the list, in the third copy
        assert_eq!(locate(100, 40, 4, 2, 110 * STRIDE, 5), Some((80 + 10) * STRIDE));
    }

    #[test]
    fn draws_outside_the_list_or_the_bands_are_not_drawn_from_a_copy() {
        assert_eq!(locate(100, 40, 4, 0, 139 * STRIDE, 2), None, "past the end");
        assert_eq!(locate(100, 40, 4, 0, 99 * STRIDE, 1), None, "before it");
        assert_eq!(locate(100, 40, 4, 4, 100 * STRIDE, 1), None, "no such band");
        assert_eq!(locate(100, 0, 4, 0, 100 * STRIDE, 0), None, "an empty list gates nothing");
        assert_eq!(locate(100, 40, 4, 0, 100 * STRIDE + 4, 1), None, "not on a draw");
    }

    #[test]
    fn the_stride_is_a_draw_indexed_indirect() {
        assert_eq!(STRIDE, std::mem::size_of::<crate::types::DrawIndexedIndirect>() as u64);
    }
}
