//! Enhanced water's finer wind-sea maps, worked out on the GPU each frame
//! (shaders/water_ocean.wgsl): the short cascade's waves on a grid twice as
//! fine as the CPU's (the same waves, so the surface the CPU floats
//! boats on is the one drawn), and a finest cascade of ripples down to some
//! 4 cm. The surface reads their slopes, folds and lingering foam per pixel
//! through mips (frame group binding 33, common.wgsl `water_fx_fine`): at
//! 128² the CPU's maps left the short waves two or three texels long, and
//! interpolated they drew round blobs instead of the sharp facets that
//! break the sun into glints.

use crate::pipeline_cache::CachedPipelines;
use crate::water_fft::{FINE_N, FINE_TILE, OCEAN_TILES, OceanMaps};

/// Map layers: per cascade its displacement and its slopes, then the foam.
const LAYERS: u32 = 5;
const MIPS: u32 = FINE_N.trailing_zeros() + 1;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    time: [f32; 4],
    wind: [f32; 4],
    /// x: the wind's speed (m/s), y, z: how fast each cascade's crests
    /// carry their foam (`water_fft::ride_speed`)
    ride: [f32; 4],
}

pub(crate) struct OceanGpu {
    h0: wgpu::Texture,
    params: wgpu::Buffer,
    #[cfg_attr(not(test), allow(dead_code))]
    maps: wgpu::Texture,
    /// The maps as the surface reads them (all mips and layers).
    pub view: wgpu::TextureView,
    spectrum: wgpu::ComputePipeline,
    rows: wgpu::ComputePipeline,
    columns: wgpu::ComputePipeline,
    unpack: wgpu::ComputePipeline,
    mip: wgpu::ComputePipeline,
    /// Per foam buffer read (the other is written): the passes' group.
    groups: [wgpu::BindGroup; 2],
    mip_groups: Vec<wgpu::BindGroup>,
    flip: usize,
    generation: Option<u64>,
    last_time: Option<f64>,
    /// Whether the maps hold this frame's sea (`encode` runs).
    pub ready: bool,
}

impl OceanGpu {
    pub fn new(device: &wgpu::Device) -> Self {
        let n = FINE_N as u32;
        let h0 = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("water fine spectrum"),
            size: wgpu::Extent3d {
                width: n,
                height: n,
                depth_or_array_layers: 2,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let maps = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("water fine maps"),
            size: wgpu::Extent3d {
                width: n,
                height: n,
                depth_or_array_layers: LAYERS,
            },
            mip_level_count: MIPS,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            // (copied out only by the tests)
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let array = |base: u32, count: u32| {
            maps.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                base_mip_level: base,
                mip_level_count: Some(count),
                ..Default::default()
            })
        };
        let view = array(0, MIPS);
        let params = crate::gpu_alloc::buffer_init(
            device,
            "water fine maps",
            bytemuck::bytes_of(&Params {
                time: [0.0; 4],
                wind: [1.0, 0.0, OCEAN_TILES[1], FINE_TILE],
                ride: [0.0; 4],
            }),
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        );
        let complex = (2 * 4 * FINE_N * FINE_N * 8) as u64;
        let buffer = |label: &str, size: u64| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            })
        };
        let fields = buffer("water fine fields", complex);
        let spare = buffer("water fine fields (rows)", complex);
        let foam = [
            // (per cascade the foam left behind, then the foam riding on)
            buffer("water fine foam", (4 * FINE_N * FINE_N * 4) as u64),
            buffer("water fine foam", (4 * FINE_N * FINE_N * 4) as u64),
        ];
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("water fine maps"),
            source: wgpu::ShaderSource::Wgsl(crate::water_shader_source(
                &["water_ocean.wgsl"],
                include_str!("shaders/water_ocean.wgsl"),
            )),
        });
        let mip_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("water fine maps' mips"),
            source: wgpu::ShaderSource::Wgsl(crate::water_shader_source(
                &["water_ocean_mip.wgsl"],
                include_str!("shaders/water_ocean_mip.wgsl"),
            )),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("water fine maps"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                storage(2, false),
                storage(3, false),
                storage(4, true),
                storage(5, false),
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba16Float,
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("water fine maps"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |entry: &str, module: &wgpu::ShaderModule, layout: Option<&wgpu::PipelineLayout>| {
            device.create_compute_pipeline_cached(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout,
                module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let spectrum = pipeline("cs_spectrum", &module, Some(&pipeline_layout));
        let rows = pipeline("cs_rows", &module, Some(&pipeline_layout));
        let columns = pipeline("cs_columns", &module, Some(&pipeline_layout));
        let unpack = pipeline("cs_unpack", &module, Some(&pipeline_layout));
        let mip = pipeline("cs_mip", &mip_module, None);
        let h0_view = h0.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let top = array(0, 1);
        let group = |read: usize| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("water fine maps"),
                layout: &layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: params.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&h0_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: fields.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: spare.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: foam[read].as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: foam[1 - read].as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 6,
                        resource: wgpu::BindingResource::TextureView(&top),
                    },
                ],
            })
        };
        let groups = [group(0), group(1)];
        let mip_layout = mip.get_bind_group_layout(0);
        let mip_groups = (1..MIPS)
            .map(|level| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("water fine maps' mip"),
                    layout: &mip_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&array(level - 1, 1)),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(&array(level, 1)),
                        },
                    ],
                })
            })
            .collect();
        Self {
            h0,
            params,
            maps,
            view,
            spectrum,
            rows,
            columns,
            unpack,
            mip,
            groups,
            mip_groups,
            flip: 0,
            generation: None,
            last_time: None,
            ready: false,
        }
    }

    /// Take the CPU's maps of this frame (`OceanMaps`): their spectrum when
    /// it was laid out again, and their time, chop, scale and wind. Without
    /// a spectrum nothing runs and the surface keeps the CPU's maps alone.
    pub fn prepare(&mut self, queue: &wgpu::Queue, maps: &OceanMaps) {
        let Some(fine) = maps.fine.as_ref().filter(|f| f.texels.len() == 2 * FINE_N * FINE_N) else {
            self.ready = false;
            return;
        };
        if self.generation != Some(fine.generation) {
            self.generation = Some(fine.generation);
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.h0,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                bytemuck::cast_slice(&fine.texels),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(FINE_N as u32 * 16),
                    rows_per_image: Some(FINE_N as u32),
                },
                wgpu::Extent3d {
                    width: FINE_N as u32,
                    height: FINE_N as u32,
                    depth_or_array_layers: 2,
                },
            );
        }
        // (the foam's step: none across a jump of the clock or the loop)
        let dt = match self.last_time {
            Some(before) if maps.fine_time >= before && maps.fine_time - before < 1.0 => (maps.fine_time - before) as f32,
            _ => 0.0,
        };
        self.last_time = Some(maps.fine_time);
        let params = Params {
            time: [maps.fine_time as f32, maps.chop, maps.scale, dt],
            wind: [maps.toward[0], maps.toward[1], OCEAN_TILES[1], FINE_TILE],
            ride: [
                maps.speed,
                crate::water_fft::ride_speed(1, maps.speed),
                crate::water_fft::ride_speed(2, maps.speed),
                0.0,
            ],
        };
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&params));
        self.flip ^= 1;
        self.ready = true;
    }

    pub fn encode(&self, encoder: &mut wgpu::CommandEncoder) {
        if !self.ready {
            return;
        }
        let n = FINE_N as u32;
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("water fine maps"),
            timestamp_writes: None,
        });
        pass.set_bind_group(0, &self.groups[self.flip], &[]);
        pass.set_pipeline(&self.spectrum);
        pass.dispatch_workgroups(n / 16, n / 16, 2);
        pass.set_pipeline(&self.rows);
        pass.dispatch_workgroups(n, 8, 1);
        pass.set_pipeline(&self.columns);
        pass.dispatch_workgroups(n, 8, 1);
        pass.set_pipeline(&self.unpack);
        pass.dispatch_workgroups(n / 16, n / 16, 1);
        pass.set_pipeline(&self.mip);
        for (i, group) in self.mip_groups.iter().enumerate() {
            let size = (n >> (i + 1)).max(1);
            pass.set_bind_group(0, group, &[]);
            pass.dispatch_workgroups(size.div_ceil(8), size.div_ceil(8), 1);
        }
    }
}

fn storage(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::water_fft::{FETCH, OCEAN_BLOCKS, OCEAN_N, Ocean, Spectrum};
    use glam::Vec2;

    fn half(bits: u16) -> f32 {
        let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
        let exp = i32::from((bits >> 10) & 0x1f);
        let frac = f32::from(bits & 0x3ff);
        sign * match exp {
            0 => frac * 2f32.powi(-24),
            31 => f32::INFINITY,
            _ => (1.0 + frac / 1024.0) * 2f32.powi(exp - 15),
        }
    }

    /// The GPU's short cascade is the CPU's, sampled twice as finely:
    /// every other texel of it is a texel of the CPU's (boats float on the
    /// surface that is drawn), and its finest cascade has waves of its own.
    #[test]
    fn the_gpu_short_cascade_is_the_cpu_s_and_the_finest_has_ripples() {
        let _gpu = crate::test_gpu::lock();
        let instance = wgpu::Instance::default();
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
            return;
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("GPU device");
        let mut ocean = Ocean::new();
        // hours online: the loop keeps the phases the same on both sides
        let maps = ocean.update(
            5.0 * 3600.0 + 7.25,
            Vec2::new(0.8, 0.6),
            12.0,
            Spectrum::Jonswap,
            0.9,
            FETCH,
            1.0,
            true,
        );
        let mut gpu = OceanGpu::new(&device);
        gpu.prepare(&queue, &maps);
        assert!(gpu.ready);
        let n = FINE_N as u32;
        let row = n * 8;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: crate::readback::readback_size(u64::from(row * n * LAYERS)),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        gpu.encode(&mut encoder);
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &gpu.maps,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(n),
                },
            },
            wgpu::Extent3d {
                width: n,
                height: n,
                depth_or_array_layers: LAYERS,
            },
        );
        queue.submit([encoder.finish()]);
        readback.map_async(wgpu::MapMode::Read, .., |r| r.expect("fine maps readback"));
        device.poll(wgpu::PollType::wait_indefinitely()).expect("GPU completion");
        let bytes = readback.get_mapped_range(..).expect("fine maps");
        let texel = |layer: u32, x: u32, y: u32| -> [f32; 4] {
            let at = ((layer * n + y) * n + x) as usize * 8;
            std::array::from_fn(|k| half(u16::from_le_bytes([bytes[at + 2 * k], bytes[at + 2 * k + 1]])))
        };
        let cpu = |block: usize, x: usize, y: usize| maps.texels[((OCEAN_BLOCKS + block) * OCEAN_N + y) * OCEAN_N + x];
        let mut worst = 0.0f32;
        let mut largest = 0.0f32;
        for y in (0..OCEAN_N).step_by(7) {
            for x in (0..OCEAN_N).step_by(5) {
                for block in 0..2 {
                    let (g, c) = (texel(block as u32, 2 * x as u32, 2 * y as u32), cpu(block, x, y));
                    for k in 0..4 {
                        worst = worst.max((g[k] - c[k]).abs());
                        largest = largest.max(c[k].abs());
                    }
                }
            }
        }
        assert!(largest > 0.01, "the short cascade has waves: {largest}");
        assert!(worst < 0.01 + 0.01 * largest, "GPU vs CPU: {worst} (largest {largest})");
        // the finest cascade: ripples, steep for their size
        let steepest = (0..n)
            .step_by(3)
            .map(|i| {
                let s = texel(3, i, (i * 7) % n);
                s[0].hypot(s[1])
            })
            .fold(0.0f32, f32::max);
        assert!(steepest > 0.02 && steepest.is_finite(), "{steepest}");
        let rms = |layer: u32| {
            let sum: f32 = (0..n * n)
                .map(|i| {
                    let s = texel(layer, i % n, i / n);
                    s[0] * s[0] + s[1] * s[1]
                })
                .sum();
            (sum / (n * n) as f32).sqrt()
        };
        let cpu_rms = |c: usize| {
            let sum: f32 = (0..OCEAN_N * OCEAN_N)
                .map(|i| {
                    let s = maps.texels[(c * OCEAN_BLOCKS + 1) * OCEAN_N * OCEAN_N + i];
                    s[0] * s[0] + s[1] * s[1]
                })
                .sum();
            (sum / (OCEAN_N * OCEAN_N) as f32).sqrt()
        };
        // the sea's slopes as Cox and Munk measured them from the sun's
        // glitter: a mean square slope of 0.003 + 0.00512 U, some 0.25 at
        // 12 m/s, most of it in the short waves
        let total = (cpu_rms(0).powi(2) + rms(1).powi(2) + rms(3).powi(2)).sqrt();
        assert!((0.15..0.4).contains(&total), "{total}");
        assert!((rms(1) - cpu_rms(1)).abs() < 0.01, "{} vs {}", rms(1), cpu_rms(1));
    }
}
