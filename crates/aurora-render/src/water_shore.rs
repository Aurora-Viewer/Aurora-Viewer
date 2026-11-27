//! The sea grid's shore map: the meshes standing in the water, by their
//! real shape. The sea grid (aurora-viewer's scene/water_shelter.rs) calms
//! the sea round what crosses the water and keeps the crests under what
//! stands low over it; a prim is near enough its box for that, a mesh is
//! not (a reef rock's box covers far more water than the rock, a dock's
//! takes in the water between its pilings), so the meshes are drawn here,
//! seen from straight below over the grid's 512 m, and the grid reads
//! where they are.
//!
//! The view looks up from just under the water (`low`, the lowest water
//! less the depth a crossing must reach) to the highest underside that
//! still holds the sea (`high`): the near plane cuts every solid that
//! crosses the water, which is then seen from inside, its top first,
//! turned up; anything else is first seen by its underside, turned down
//! (object.wgsl `fs_shore`). A compute pass sums each grid cell's texels
//! (shaders/water_shore.wgsl), and only those cells are read back, a few
//! frames later, without a wait: the viewer asks for a map when what makes
//! its grid changes, and bakes the grid again once the answer comes. The
//! targets are made on the first request and freed when none came for
//! `IDLE`. Aurora's own: the Legacy Viewer's water has no waves to shelter.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant};

use glam::{Mat4, Vec2, Vec4};

use crate::pipeline_cache::CachedPipelines;
use crate::types::DrawCmd;
use crate::water_effects::SEA_GRID;

/// Texels a side of the map: 25 cm over the grid's 512 m, so that a
/// mesh's piling some decimetres across is still seen.
pub const MAP_TEXELS: u32 = 2048;
/// Texels a side of one grid cell.
const CELL_TEXELS: u32 = MAP_TEXELS / SEA_GRID as u32;
/// The map: per texel the nearest face's height over `low` and which way it
/// faces (the heights span a few metres: half floats keep millimetres).
const COLOR: wgpu::TextureFormat = wgpu::TextureFormat::Rg16Float;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// A cell's height where it has none (shaders/water_shore.wgsl NONE).
pub const NONE: f32 = 1.0e9;
/// Maps in flight at once: they are asked for when the grid's makings
/// change, a few times a minute at most; past this a request is dropped
/// (its caller keeps the boxes).
const IN_FLIGHT: usize = 2;
/// The targets are freed when no map was asked for this long.
const IDLE: Duration = Duration::from_secs(120);

/// A map asked for: the grid's square (`origin` its south-west corner,
/// `extent` its side, m), the heights it spans (`low` the lowest water
/// less the depth a crossing must reach, `high` the highest water plus
/// the highest underside that holds the sea) and the faces drawn into it,
/// tagged by its caller.
#[derive(Debug, Clone, Default)]
pub struct ShoreRequest {
    pub tag: u64,
    pub origin: [f32; 2],
    pub extent: f32,
    pub low: f32,
    pub high: f32,
    pub draws: Vec<DrawCmd>,
}

/// One grid cell of a map: the share of its texels inside a solid that
/// crosses the water and the lowest top seen from inside; the share under
/// an underside and the lowest underside (world heights, `NONE` without).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ShoreCell {
    pub inside: f32,
    pub top: f32,
    pub under: f32,
    pub underside: f32,
}

/// A map made: its request's tag and its cells, row by row from the
/// south-west (as the sea grid's).
#[derive(Debug, Clone, PartialEq)]
pub struct ShoreMap {
    pub tag: u64,
    pub cells: Arc<[ShoreCell]>,
}

/// The map's view-projection: the square seen from below, x and y as on
/// the ground seen from above (+Y up: a face turned up keeps its winding),
/// depth 0 at `low` rising to 1 at `high`, so that the nearest face is the
/// lowest and what lies under `low` is clipped away.
pub fn shore_matrix(origin: Vec2, extent: f32, low: f32, high: f32) -> Mat4 {
    let half = (extent * 0.5).max(1e-3);
    let c = origin + Vec2::splat(half);
    let span = (high - low).max(1e-3);
    Mat4::from_cols(
        Vec4::new(1.0 / half, 0.0, 0.0, 0.0),
        Vec4::new(0.0, 1.0 / half, 0.0, 0.0),
        Vec4::new(0.0, 0.0, 1.0 / span, 0.0),
        Vec4::new(-c.x / half, -c.y / half, -low / span, 1.0),
    )
}

/// The cells read back (`SEA_GRID`² of four floats), or none when the
/// bytes are short.
pub fn decode_cells(bytes: &[u8]) -> Option<Vec<ShoreCell>> {
    let n = SEA_GRID * SEA_GRID;
    if bytes.len() < n * 16 {
        return None;
    }
    let float = |at: usize| f32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
    Some(
        (0..n)
            .map(|i| {
                let [inside, top, under, underside] = std::array::from_fn(|k| float(i * 16 + k * 4));
                ShoreCell {
                    inside,
                    top,
                    under,
                    underside,
                }
            })
            .collect(),
    )
}

const ENCODED: u8 = 0;
const WAITING: u8 = 1;
const MAPPED: u8 = 2;
const FAILED: u8 = 3;

struct Flight {
    tag: u64,
    buffer: wgpu::Buffer,
    state: Arc<AtomicU8>,
}

/// The map's targets, made on the first request.
struct Targets {
    color: wgpu::TextureView,
    depth: wgpu::TextureView,
    cells_group: wgpu::BindGroup,
}

/// The shore maps: their view's uniform (the shadows' `ShadowParams`, group
/// 3 of the shadow pipelines), the reduction to cells, and the maps in
/// flight.
pub(crate) struct ShoreMaps {
    pub(crate) uniform: wgpu::Buffer,
    pub(crate) group: wgpu::BindGroup,
    params: wgpu::Buffer,
    cells: wgpu::Buffer,
    layout: wgpu::BindGroupLayout,
    reduce: wgpu::ComputePipeline,
    targets: Option<Targets>,
    asked_at: Option<Instant>,
    flights: Vec<Flight>,
    done: Vec<ShoreMap>,
}

impl ShoreMaps {
    /// `shadow_layout` and `sampler`: the shadow pipelines' group 3 and its
    /// sampler; `uniform_size` the size of their `ShadowParams`.
    pub(crate) fn new(device: &wgpu::Device, shadow_layout: &wgpu::BindGroupLayout, sampler: &wgpu::Sampler, uniform_size: u64) -> Self {
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shore map vp"),
            size: uniform_size,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let group = crate::renderer::shadow_bind_group(device, shadow_layout, &uniform, sampler);
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shore map cells"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let cells = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shore map cells"),
            size: (SEA_GRID * SEA_GRID * 16) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shore map cells"),
            source: wgpu::ShaderSource::Wgsl(crate::water_shader_source(
                &["water_shore.wgsl"],
                include_str!("shaders/water_shore.wgsl"),
            )),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shore map cells"),
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
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
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
            label: Some("shore map cells"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let reduce = device.create_compute_pipeline_cached(&wgpu::ComputePipelineDescriptor {
            label: Some("cs_shore_cells"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("cs_shore_cells"),
            compilation_options: Default::default(),
            cache: None,
        });
        Self {
            uniform,
            group,
            params,
            cells,
            layout,
            reduce,
            targets: None,
            asked_at: None,
            flights: Vec::new(),
            done: Vec::new(),
        }
    }

    /// Whether a map can be drawn this frame (not too many in flight), its
    /// targets made if they were not. The caller then draws the map
    /// (`views`) and calls `encode`.
    pub(crate) fn begin(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, request: &ShoreRequest) -> bool {
        if self.flights.len() >= IN_FLIGHT {
            log::debug!("water shore map {} dropped: {} in flight", request.tag, self.flights.len());
            return false;
        }
        self.asked_at = Some(Instant::now());
        if self.targets.is_none() {
            let texture = |label: &str, format: wgpu::TextureFormat, usage: wgpu::TextureUsages| {
                device
                    .create_texture(&wgpu::TextureDescriptor {
                        label: Some(label),
                        size: wgpu::Extent3d {
                            width: MAP_TEXELS,
                            height: MAP_TEXELS,
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
            let color = texture(
                "shore map",
                COLOR,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            );
            let depth = texture("shore map depth", DEPTH, wgpu::TextureUsages::RENDER_ATTACHMENT);
            let cells_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("shore map cells"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.params.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&color),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: self.cells.as_entire_binding(),
                    },
                ],
            });
            self.targets = Some(Targets { color, depth, cells_group });
            log::info!("water shore map: targets made ({MAP_TEXELS} texels a side)");
        }
        let params = [request.low, CELL_TEXELS as f32, SEA_GRID as f32, 0.0];
        queue.write_buffer(&self.params, 0, bytemuck::cast_slice(&params));
        true
    }

    /// The map's colour and depth targets, while they are made.
    pub(crate) fn views(&self) -> Option<(&wgpu::TextureView, &wgpu::TextureView)> {
        self.targets.as_ref().map(|t| (&t.color, &t.depth))
    }

    /// The cells of the map drawn this frame, and their copy to be read
    /// back once the frame's commands are submitted.
    pub(crate) fn encode(&mut self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, tag: u64) {
        let Some(targets) = &self.targets else {
            return;
        };
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("shore map cells"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.reduce);
            pass.set_bind_group(0, &targets.cells_group, &[]);
            let groups = (SEA_GRID as u32).div_ceil(8);
            pass.dispatch_workgroups(groups, groups, 1);
        }
        let size = self.cells.size();
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shore map read back"),
            size: crate::readback::readback_size(size),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_buffer_to_buffer(&self.cells, 0, &buffer, 0, size);
        self.flights.push(Flight {
            tag,
            buffer,
            state: Arc::new(AtomicU8::new(ENCODED)),
        });
    }

    /// Maps the copies encoded this frame, once its commands are submitted;
    /// takes in the maps the GPU is through with; frees the targets when no
    /// map was asked for a while.
    pub(crate) fn after_submit(&mut self) {
        for flight in &self.flights {
            if flight.state.load(Ordering::Acquire) != ENCODED {
                continue;
            }
            flight.state.store(WAITING, Ordering::Release);
            let state = flight.state.clone();
            flight.buffer.map_async(wgpu::MapMode::Read, .., move |r| {
                state.store(if r.is_ok() { MAPPED } else { FAILED }, Ordering::Release);
            });
        }
        let mut index = 0;
        while index < self.flights.len() {
            match self.flights[index].state.load(Ordering::Acquire) {
                MAPPED => {
                    let flight = self.flights.swap_remove(index);
                    let cells = flight.buffer.get_mapped_range(..).ok().and_then(|bytes| decode_cells(&bytes));
                    flight.buffer.unmap();
                    match cells {
                        Some(cells) => self.done.push(ShoreMap {
                            tag: flight.tag,
                            cells: cells.into(),
                        }),
                        None => log::warn!("water shore map {}: its cells could not be read", flight.tag),
                    }
                }
                FAILED => {
                    let flight = self.flights.swap_remove(index);
                    log::warn!("water shore map {}: its read back failed", flight.tag);
                }
                _ => index += 1,
            }
        }
        if self.flights.is_empty() && self.targets.is_some() && self.asked_at.is_none_or(|at| at.elapsed() > IDLE) {
            self.targets = None;
            log::info!("water shore map: targets freed (none asked for {} s)", IDLE.as_secs());
        }
    }

    /// The maps made since the last call.
    pub(crate) fn take(&mut self) -> Vec<ShoreMap> {
        std::mem::take(&mut self.done)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    /// The view keeps the ground's x and y the way they lie seen from
    /// above, and its depth rises from `low` (0) to `high` (1): the nearest
    /// face is the lowest, what lies under `low` is clipped away.
    #[test]
    fn the_view_looks_up_from_under_the_water_over_the_grid() {
        let m = shore_matrix(Vec2::new(100.0, -50.0), 512.0, 19.7, 26.0);
        let ndc = |p: Vec3| {
            let c = m * p.extend(1.0);
            c.truncate() / c.w
        };
        // the square's corners and centre
        assert!(
            ndc(Vec3::new(100.0, -50.0, 20.0))
                .truncate()
                .abs_diff_eq(glam::Vec2::new(-1.0, -1.0), 1e-5)
        );
        assert!(
            ndc(Vec3::new(612.0, 462.0, 20.0))
                .truncate()
                .abs_diff_eq(glam::Vec2::new(1.0, 1.0), 1e-5)
        );
        assert!(ndc(Vec3::new(356.0, 206.0, 20.0)).truncate().abs_diff_eq(glam::Vec2::ZERO, 1e-5));
        // north is up on the map, east to the right: a face turned up keeps
        // its winding (fs_shore's `front`)
        let (a, b, c) = (
            ndc(Vec3::new(200.0, 0.0, 20.0)),
            ndc(Vec3::new(210.0, 0.0, 20.0)),
            ndc(Vec3::new(200.0, 10.0, 20.0)),
        );
        let winding = (b - a).truncate().perp_dot((c - a).truncate());
        assert!(winding > 0.0, "counter-clockwise seen from above stays so: {winding}");
        // depth: 0 at low, 1 at high, rising with the height
        assert!(ndc(Vec3::new(300.0, 0.0, 19.7)).z.abs() < 1e-5);
        assert!((ndc(Vec3::new(300.0, 0.0, 26.0)).z - 1.0).abs() < 1e-5);
        assert!(ndc(Vec3::new(300.0, 0.0, 19.0)).z < 0.0, "under the water's cut: clipped");
        assert!(ndc(Vec3::new(300.0, 0.0, 21.0)).z < ndc(Vec3::new(300.0, 0.0, 22.0)).z);
    }

    #[test]
    fn the_cells_come_back_as_they_were_written_or_not_at_all() {
        let n = SEA_GRID * SEA_GRID;
        let mut bytes = Vec::with_capacity(n * 16);
        for i in 0..n {
            for v in [i as f32 / n as f32, 20.5, 0.0, NONE] {
                bytes.extend_from_slice(&v.to_le_bytes());
            }
        }
        let cells = decode_cells(&bytes).expect("a whole grid");
        assert_eq!(cells.len(), n);
        assert_eq!(
            cells[5],
            ShoreCell {
                inside: 5.0 / n as f32,
                top: 20.5,
                under: 0.0,
                underside: NONE
            }
        );
        assert!(decode_cells(&bytes[..bytes.len() - 4]).is_none());
    }

    #[test]
    fn the_cells_shader_is_valid_for_vulkan_and_metal() {
        let source = include_str!("shaders/water_shore.wgsl");
        let module = naga::front::wgsl::parse_str(source).unwrap_or_else(|e| panic!("{}", e.emit_to_string(source)));
        let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::empty())
            .validate(&module)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(source)));
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
        assert_eq!(module.entry_points.len(), 1);
    }
}
