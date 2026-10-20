//! The luminance histogram the photo viewfinder shows: counted on the GPU
//! from the scene image (`shaders/histogram.wgsl`), only on the frames that
//! ask for it, and read back without waiting (the counts come a few frames
//! later, which a histogram does not mind).

use bytemuck::{Pod, Zeroable};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

/// Bins of the histogram, from black to white.
pub const HISTOGRAM_BINS: usize = 64;
const BYTES: u64 = (HISTOGRAM_BINS * 4) as u64;

const IDLE: u8 = 0;
const ENCODED: u8 = 1;
const WAITING: u8 = 2;
const MAPPED: u8 = 3;
const FAILED: u8 = 4;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct HistogramU {
    a: [f32; 4],
}

pub(crate) struct Histogram {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::ComputePipeline,
    params: wgpu::Buffer,
    bins: wgpu::Buffer,
    readback: wgpu::Buffer,
    read_state: Arc<AtomicU8>,
    bg: Option<wgpu::BindGroup>,
    /// The last counts read back.
    latest: Option<Box<[u32; HISTOGRAM_BINS]>>,
}

impl Histogram {
    pub(crate) fn new(device: &wgpu::Device) -> Histogram {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("histogram"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
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
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("histogram"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/histogram.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("histogram"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("histogram"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("cs_histogram"),
            compilation_options: Default::default(),
            cache: None,
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("histogram params"),
            size: std::mem::size_of::<HistogramU>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bins = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("histogram bins"),
            size: BYTES,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("histogram read back"),
            size: BYTES,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Histogram {
            layout,
            pipeline,
            params,
            bins,
            readback,
            read_state: Arc::new(AtomicU8::new(IDLE)),
            bg: None,
            latest: None,
        }
    }

    /// Binds the scene image (resolved, before the tone mapping); again
    /// whenever the targets are made anew.
    pub(crate) fn bind(&mut self, device: &wgpu::Device, scene: &wgpu::TextureView) {
        self.bg = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("histogram"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(scene),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.bins.as_entire_binding(),
                },
            ],
        }));
    }

    /// Counts this frame's image shown at `exposure`, when the last counts
    /// have been read (one count in flight at a time); takes in the counts
    /// that came back.
    pub(crate) fn encode(&mut self, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, exposure: f32) {
        self.collect();
        let Some(bg) = &self.bg else {
            return;
        };
        if self.read_state.load(Ordering::Acquire) != IDLE {
            return;
        }
        let u = HistogramU {
            a: [exposure.max(0.0), 0.0, 0.0, 0.0],
        };
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&u));
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("histogram"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, bg, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&self.bins, 0, &self.readback, 0, BYTES);
        self.read_state.store(ENCODED, Ordering::Release);
    }

    /// Maps the counts copied this frame, once its commands are submitted.
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

    fn collect(&mut self) {
        match self.read_state.load(Ordering::Acquire) {
            MAPPED => {
                if let Ok(view) = self.readback.get_mapped_range(..) {
                    let mut out = Box::new([0u32; HISTOGRAM_BINS]);
                    for (o, b) in out.iter_mut().zip(view.as_chunks::<4>().0) {
                        *o = u32::from_le_bytes(*b);
                    }
                    self.latest = Some(out);
                }
                self.readback.unmap();
                self.read_state.store(IDLE, Ordering::Release);
            }
            FAILED => self.read_state.store(IDLE, Ordering::Release),
            _ => {}
        }
    }

    /// The last counts read back, once.
    pub(crate) fn take(&mut self) -> Option<Box<[u32; HISTOGRAM_BINS]>> {
        self.collect();
        self.latest.take()
    }
}
