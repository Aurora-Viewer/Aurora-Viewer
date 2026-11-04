//! Short contact-shadow history, independent of the ambient-occlusion choice.
use crate::pipeline_cache::CachedPipelines;
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec2, Vec3};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniform {
    inverse_current: [[f32; 4]; 4],
    previous: [[f32; 4]; 4],
    jitter_reset: [f32; 4],
}

#[derive(Clone, Copy)]
pub(crate) struct Frame {
    pub index: u64,
    pub projection: Mat4,
    pub view: Mat4,
    pub jitter: Vec2,
    pub sun: Vec3,
    pub reset: bool,
}

pub(crate) struct History {
    pipeline: wgpu::ComputePipeline,
    buffer: wgpu::Buffer,
    previous: Option<(u64, u64, Mat4, Vec2, Vec3)>,
    slot: usize,
}

pub(crate) struct Targets {
    groups: [[wgpu::BindGroup; 2]; 2],
    generation: u64,
    size: (u32, u32),
    pub motion: wgpu::TextureView,
}

pub(crate) fn needed() -> bool {
    std::env::var("AURORA_CONTACT_TEMPORAL").as_deref() != Ok("0")
}

impl History {
    pub fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("contact shadow history"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/contact_history.wgsl").into()),
        });
        let pipeline = device.create_compute_pipeline_cached(&wgpu::ComputePipelineDescriptor {
            label: Some("contact shadow history"),
            layout: None,
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("contact shadow history parameters"),
            size: std::mem::size_of::<Uniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            buffer,
            previous: None,
            slot: 0,
        }
    }

    pub fn targets(
        &self,
        device: &wgpu::Device,
        size: (u32, u32),
        depths: [&wgpu::TextureView; 2],
        current: &wgpu::TextureView,
        result: &wgpu::TextureView,
        motion: Option<&wgpu::TextureView>,
    ) -> Targets {
        let make = |label, format, usage| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: size.0,
                        height: size.1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let sampled = wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING;
        let masks = std::array::from_fn::<_, 2, _>(|_| make("contact history", wgpu::TextureFormat::Rgba8Unorm, sampled));
        let metadata = std::array::from_fn::<_, 2, _>(|_| make("contact history metadata", wgpu::TextureFormat::Rg32Uint, sampled));
        let motion = motion.cloned().unwrap_or_else(|| {
            make(
                "contact normals and motion",
                crate::gtao::NORMAL_FORMAT,
                wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
            )
        });
        let groups = std::array::from_fn(|depth_index| {
            std::array::from_fn(|slot| {
                let views = [
                    current,
                    depths[depth_index],
                    &motion,
                    &masks[1 - slot],
                    &metadata[1 - slot],
                    &masks[slot],
                    &metadata[slot],
                    result,
                ];
                let mut entries = vec![wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.buffer.as_entire_binding(),
                }];
                entries.extend(views.iter().enumerate().map(|(i, v)| wgpu::BindGroupEntry {
                    binding: i as u32 + 1,
                    resource: wgpu::BindingResource::TextureView(v),
                }));
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("contact history"),
                    layout: &self.pipeline.get_bind_group_layout(0),
                    entries: &entries,
                })
            })
        });
        static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Targets {
            groups,
            generation: GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            size,
            motion,
        }
    }

    pub fn write(&mut self, queue: &wgpu::Queue, t: &Targets, frame: Frame) {
        let Frame {
            index,
            projection,
            view,
            jitter,
            sun,
            reset,
        } = frame;
        let current = projection * view;
        let prior = self.previous.filter(|(generation, frame, _, _, light)| {
            *generation == t.generation && frame.checked_add(1) == Some(index) && light.dot(sun) > 0.99999 && !reset
        });
        let previous = prior.map_or(current, |p| p.2);
        let delta = prior.map_or(Vec2::ZERO, |p| p.3 - jitter);
        self.slot = index as usize & 1;
        let uniform = Uniform {
            inverse_current: current.inverse().to_cols_array_2d(),
            previous: previous.to_cols_array_2d(),
            jitter_reset: [delta.x, delta.y, if prior.is_none() { 1.0 } else { 0.0 }, 0.0],
        };
        queue.write_buffer(&self.buffer, 0, bytemuck::bytes_of(&uniform));
        self.previous = (!reset).then_some((t.generation, index, current, jitter, sun));
    }

    pub fn encode(&self, encoder: &mut wgpu::CommandEncoder, t: &Targets, more: bool) {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("contact shadow history"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &t.groups[usize::from(more)][self.slot], &[]);
        pass.dispatch_workgroups(t.size.0.div_ceil(8), t.size.1.div_ceil(8), 1);
    }
}
