//! The OIT's gated draws without counted indirect draws.
//!
//! When the node pool overflows, the classify (oit.rs) skips the composite
//! and the blended faces are drawn again in order; otherwise the composite
//! is drawn and the replay is not. Both are decided on the GPU, with no wait
//! on the CPU. A count read from the GPU (`multi_draw_indexed_indirect_count`)
//! did it, but Metal has none in wgpu. Here a compute pass, right after the
//! classify, copies the arguments of the draws that may be gated (the blend
//! list and the creation preview) twice: the replay's copy keeps their
//! instances only when the pool overflowed, the composite's copy only when it
//! did not. The gated draws are then plain indirect draws of a copy, on every
//! backend.

use crate::pipeline_cache::CachedPipelines;
use bytemuck::{Pod, Zeroable};

/// Ranges of the indirect buffer a frame may gate (the blend list, the
/// creation preview).
pub(crate) const SPANS: usize = 2;
/// Bytes of a DrawIndexedIndirect.
const STRIDE: u64 = 20;
const WORKGROUP: u32 = 64;

/// Which copy a gated draw reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Copy {
    /// drawn only when the pool overflowed
    Replay,
    /// drawn only when it did not
    Composited,
}

/// A range of the indirect buffer and where its draws start in each copy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Span {
    /// first draw in the indirect buffer
    src: u32,
    count: u32,
    /// first draw in each copy
    dst: u32,
}

/// The ranges laid out one after the other in each copy, and the draws a
/// copy holds. `ranges`: (byte offset in the indirect buffer, draws).
fn layout(ranges: [(u64, u32); SPANS]) -> ([Span; SPANS], u32) {
    let mut spans = [Span::default(); SPANS];
    let mut total = 0u32;
    for (span, (offset, count)) in spans.iter_mut().zip(ranges) {
        *span = Span {
            src: u32::try_from(offset / STRIDE).unwrap_or(u32::MAX),
            count,
            dst: total,
        };
        total = total.saturating_add(count);
    }
    (spans, total)
}

/// Byte offset in the copies of the draws `offset..` (`count` of them) of
/// the indirect buffer, when one range holds them all.
fn locate(spans: &[Span; SPANS], total: u32, copy: Copy, offset: u64, count: u32) -> Option<u64> {
    if !offset.is_multiple_of(STRIDE) {
        return None;
    }
    let first = u32::try_from(offset / STRIDE).ok()?;
    let span = spans
        .iter()
        .find(|s| s.count > 0 && first >= s.src && u64::from(first) + u64::from(count) <= u64::from(s.src) + u64::from(s.count))?;
    let base = match copy {
        Copy::Replay => 0,
        Copy::Composited => total,
    };
    Some(u64::from(base + span.dst + (first - span.src)) * STRIDE)
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, PartialEq)]
struct Params {
    total: u32,
    _pad: [u32; 3],
    /// x: first draw in the indirect buffer, y: draws, z: first gated draw
    span: [[u32; 4]; SPANS],
}

pub(crate) struct Gate {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    params: wgpu::Buffer,
    last_params: Option<Params>,
    spans: [Span; SPANS],
    total: u32,
    /// both copies, the replay's then the composite's
    args: Option<wgpu::Buffer>,
    /// the group and the buffers it binds (the indirect buffer and the
    /// pool's flags are made again when they grow)
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
            spans: [Span::default(); SPANS],
            total: 0,
            args: None,
            group: None,
        }
    }

    /// The copies ready for this frame's `ranges` of `indirect`. `false`
    /// when their buffer cannot be made (video memory full): the faces are
    /// then blended as before.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        indirect: &wgpu::Buffer,
        flags: &wgpu::Buffer,
        ranges: [(u64, u32); SPANS],
    ) -> bool {
        let (spans, total) = layout(ranges);
        self.spans = spans;
        self.total = total;
        let needed = (u64::from(total) * 2 * STRIDE).max(STRIDE * 2);
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
        if self.group.as_ref().is_none_or(|(_, i, f)| i != indirect || f != flags) {
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
                        resource: flags.as_entire_binding(),
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
            self.group = Some((group, indirect.clone(), flags.clone()));
        }
        let mut params = Params {
            total,
            _pad: [0; 3],
            span: [[0; 4]; SPANS],
        };
        for (p, s) in params.span.iter_mut().zip(&spans) {
            *p = [s.src, s.count, s.dst, 0];
        }
        if self.last_params != Some(params) {
            queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&params));
            self.last_params = Some(params);
        }
        true
    }

    /// Fills both copies, in the pass of the classify, after it.
    pub fn dispatch(&self, pass: &mut wgpu::ComputePass) {
        if self.total == 0 {
            return;
        }
        if let Some((group, ..)) = &self.group {
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, group, &[]);
            pass.dispatch_workgroups(self.total.div_ceil(WORKGROUP), 1, 1);
        }
    }

    /// The copy and the offset of the draws `offset..` of the indirect
    /// buffer (`count` of them), when they are gated this frame.
    pub fn args(&self, copy: Copy, offset: u64, count: u32) -> Option<(&wgpu::Buffer, u64)> {
        let at = locate(&self.spans, self.total, copy, offset, count)?;
        Some((self.args.as_ref()?, at))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ranges_follow_each_other_in_each_copy() {
        // the blend list at draw 100 (40 draws), the creation preview at 7 (2)
        let (spans, total) = layout([(100 * STRIDE, 40), (7 * STRIDE, 2)]);
        assert_eq!(total, 42);
        // the replay's copy first, the composite's after it
        assert_eq!(locate(&spans, total, Copy::Replay, 100 * STRIDE, 40), Some(0));
        assert_eq!(locate(&spans, total, Copy::Composited, 100 * STRIDE, 40), Some(42 * STRIDE));
        // a run inside the blend list, then the creation preview
        assert_eq!(locate(&spans, total, Copy::Replay, 110 * STRIDE, 5), Some(10 * STRIDE));
        assert_eq!(locate(&spans, total, Copy::Composited, 7 * STRIDE, 2), Some((42 + 40) * STRIDE));
    }

    #[test]
    fn draws_outside_the_gated_ranges_are_not_drawn_from_a_copy() {
        let (spans, total) = layout([(100 * STRIDE, 40), (0, 0)]);
        assert_eq!(locate(&spans, total, Copy::Replay, 139 * STRIDE, 2), None, "past the end");
        assert_eq!(locate(&spans, total, Copy::Replay, 99 * STRIDE, 1), None, "before it");
        assert_eq!(locate(&spans, total, Copy::Replay, 0, 0), None, "an empty range gates nothing");
        assert_eq!(locate(&spans, total, Copy::Replay, 100 * STRIDE + 4, 1), None, "not on a draw");
    }

    #[test]
    fn the_stride_is_a_draw_indexed_indirect() {
        assert_eq!(STRIDE, std::mem::size_of::<crate::types::DrawIndexedIndirect>() as u64);
    }
}
