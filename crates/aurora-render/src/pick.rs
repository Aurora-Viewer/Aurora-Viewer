//! Depth under the cursor, read back from the GPU. A one-texel compute copy
//! of the prepass depth replaces copying the whole depth texture (depth
//! formats only allow whole-subresource copies). Hover requests are read a
//! frame or two later without waiting for the GPU; clicks still wait, but
//! for 4 bytes. Before, the hover cursor copied the full screen depth and
//! waited for the GPU every frame.

use glam::{Mat4, Vec3, Vec4};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

const SHADER: &str = r#"
@group(0) @binding(0) var depth: texture_depth_2d;
@group(0) @binding(1) var<uniform> pixel: vec4<u32>;
@group(0) @binding(2) var<storage, read_write> out: array<f32, 4>;

@compute @workgroup_size(1)
fn main() {
    out[0] = textureLoad(depth, vec2<i32>(pixel.xy), 0);
}
"#;

/// Hover readbacks in flight (one per frame, read 1–2 frames later).
const SLOTS: usize = 3;
const BYTES: u64 = 16;
/// Readback states.
const IDLE: u8 = 0;
const MAPPED: u8 = 1;
const FAILED: u8 = 2;

/// A pixel and the camera it was rendered with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PickPixel {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub inv_view_proj: Mat4,
}

impl PickPixel {
    /// World position of a reverse-Z depth at this pixel; None for the sky
    /// (far plane at 0) or a broken depth.
    pub fn world(&self, depth: f32) -> Option<Vec3> {
        if !depth.is_finite() || depth <= 0.0 {
            return None;
        }
        let ndc = Vec4::new(
            (self.x as f32 + 0.5) / self.width as f32 * 2.0 - 1.0,
            1.0 - (self.y as f32 + 0.5) / self.height as f32 * 2.0,
            depth,
            1.0,
        );
        let p = self.inv_view_proj * ndc;
        (p.w.abs() > 1e-9).then(|| p.truncate() / p.w).filter(|v| v.is_finite())
    }
}

struct Slot {
    buffer: wgpu::Buffer,
    state: Arc<AtomicU8>,
    pending: Option<PickPixel>,
    /// Copied this frame: to map once submitted.
    encoded: bool,
}

pub struct DepthPick {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    /// [hover, click] parameters and outputs (a click never waits on the
    /// buffers of a hover in flight).
    params: [wgpu::Buffer; 2],
    out: [wgpu::Buffer; 2],
    slots: Vec<Slot>,
    request: Option<(u32, u32)>,
    latest: Option<Vec3>,
}

impl DepthPick {
    pub fn new(device: &wgpu::Device) -> DepthPick {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("depth pick"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("depth pick"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
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
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("depth pick"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("depth pick"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let buffer = |label: &str, usage: wgpu::BufferUsages| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: BYTES,
                usage,
                mapped_at_creation: false,
            })
        };
        let uniform = wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST;
        let storage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC;
        let slots = (0..SLOTS)
            .map(|_| Slot {
                buffer: buffer("depth pick readback", wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ),
                state: Arc::new(AtomicU8::new(IDLE)),
                pending: None,
                encoded: false,
            })
            .collect();
        DepthPick {
            pipeline,
            layout,
            params: [buffer("depth pick hover", uniform), buffer("depth pick click", uniform)],
            out: [buffer("depth pick hover out", storage), buffer("depth pick click out", storage)],
            slots,
            request: None,
            latest: None,
        }
    }

    /// Ask for the depth under a pixel in the next frame; returns the last
    /// hover answer (a frame or two old; None for the sky or not yet known).
    pub fn hover(&mut self, x: u32, y: u32) -> Option<Vec3> {
        self.request = Some((x, y));
        self.latest
    }

    /// Read the hover answers the GPU has delivered (start of a frame).
    pub fn harvest(&mut self) {
        for s in &mut self.slots {
            let Some(px) = s.pending else {
                continue;
            };
            match s.state.load(Ordering::Acquire) {
                MAPPED => {
                    if let Ok(view) = s.buffer.get_mapped_range(..)
                        && let Some(b) = view.get(..4)
                    {
                        self.latest = px.world(f32::from_le_bytes([b[0], b[1], b[2], b[3]]));
                    }
                    s.buffer.unmap();
                }
                FAILED => {}
                _ => continue,
            }
            s.state.store(IDLE, Ordering::Release);
            s.pending = None;
        }
    }

    fn encode_copy(&self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, depth: &wgpu::TextureView, which: usize) {
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("depth pick"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(depth),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.params[which].as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.out[which].as_entire_binding(),
                },
            ],
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("depth pick"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.dispatch_workgroups(1, 1, 1);
    }

    /// Copy the requested texel into a free readback slot (after the prepass
    /// has written `depth` this frame). A request without a free slot waits
    /// for the next frame.
    pub fn encode_hover(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        depth: &wgpu::TextureView,
        size: (u32, u32),
        inv_view_proj: Mat4,
    ) {
        let Some((x, y)) = self.request else {
            return;
        };
        let Some(i) = self.slots.iter().position(|s| s.pending.is_none()) else {
            return;
        };
        self.request = None;
        if x >= size.0 || y >= size.1 {
            return;
        }
        queue.write_buffer(&self.params[0], 0, bytemuck::cast_slice(&[x, y, 0u32, 0u32]));
        self.encode_copy(device, encoder, depth, 0);
        let s = &mut self.slots[i];
        encoder.copy_buffer_to_buffer(&self.out[0], 0, &s.buffer, 0, BYTES);
        s.pending = Some(PickPixel {
            x,
            y,
            width: size.0,
            height: size.1,
            inv_view_proj,
        });
        s.encoded = true;
    }

    /// Map the slots copied this frame, once their commands are submitted.
    pub fn after_submit(&mut self) {
        for s in &mut self.slots {
            if std::mem::take(&mut s.encoded) {
                let state = s.state.clone();
                s.buffer.map_async(wgpu::MapMode::Read, .., move |r| {
                    state.store(if r.is_ok() { MAPPED } else { FAILED }, Ordering::Release);
                });
            }
        }
    }

    /// The depth under a pixel of the last frame, waiting for the GPU (clicks).
    pub fn blocking(&self, device: &wgpu::Device, queue: &wgpu::Queue, depth: &wgpu::TextureView, px: PickPixel) -> Option<Vec3> {
        if px.x >= px.width || px.y >= px.height {
            return None;
        }
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("depth pick click readback"),
            size: BYTES,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        queue.write_buffer(&self.params[1], 0, bytemuck::cast_slice(&[px.x, px.y, 0u32, 0u32]));
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("pick") });
        self.encode_copy(device, &mut encoder, depth, 1);
        encoder.copy_buffer_to_buffer(&self.out[1], 0, &readback, 0, BYTES);
        queue.submit([encoder.finish()]);
        readback.map_async(wgpu::MapMode::Read, .., |_| {});
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        let depth = {
            let view = readback.get_mapped_range(..).ok()?;
            let b = view.get(..4)?;
            f32::from_le_bytes([b[0], b[1], b[2], b[3]])
        };
        readback.unmap();
        px.world(depth)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(view_proj: Mat4) -> PickPixel {
        PickPixel {
            x: 50,
            y: 50,
            width: 101,
            height: 101,
            inv_view_proj: view_proj.inverse(),
        }
    }

    #[test]
    fn center_pixel_depth_back_to_the_world() {
        let proj = glam::camera::rh::proj::directx::perspective_infinite_reverse(1.0, 1.0, 0.5);
        let view = glam::camera::rh::view::look_at_mat4(Vec3::ZERO, Vec3::X, Vec3::Z);
        let vp = proj * view;
        let target = Vec3::new(10.0, 0.0, 0.0);
        let clip = vp * target.extend(1.0);
        let p = pixel(vp).world(clip.z / clip.w).expect("a surface");
        assert!(p.distance(target) < 1e-3, "{p:?}");
    }

    #[test]
    fn sky_and_broken_depths_give_nothing() {
        let px = pixel(Mat4::IDENTITY);
        assert_eq!(px.world(0.0), None);
        assert_eq!(px.world(f32::NAN), None);
    }
}
