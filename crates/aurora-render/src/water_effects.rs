//! Local visual water only. The simulator remains authoritative for motion.
//! A world-anchored height/velocity/foam field runs on the GPU; no readback.

use bytemuck::{Pod, Zeroable};
use glam::Vec2;

pub const MAX_WATER_EVENTS: usize = 256;
pub const WATER_GROUND_SIZE: u32 = 64;
const MAX_WATER_IMPULSES: usize = 192;
// Three connected quads per sheet instance; ordinary droplets use the first
// quad and collapse the others. Counts remain within the shared particle cap.
pub(crate) const WATER_PARTICLE_VERTICES: u32 = 18;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WaterEffects {
    pub enabled: bool,
    pub quality: u8,
    /// Share the viewer particle budget with scripted particles.
    pub particle_budget: u32,
    pub waves: bool,
    pub absorption: bool,
    pub fresnel: bool,
    pub wet_ground: bool,
    pub shore_foam: bool,
    pub contact_foam: bool,
    pub crest_foam: bool,
    pub interactions: bool,
    pub caustics: bool,
    pub splashes: bool,
    pub bubbles: bool,
}

impl Default for WaterEffects {
    fn default() -> Self {
        Self {
            enabled: true,
            quality: 1,
            particle_budget: 4096,
            waves: true,
            absorption: true,
            fresnel: true,
            wet_ground: true,
            shore_foam: true,
            contact_foam: true,
            crest_foam: true,
            interactions: true,
            caustics: true,
            splashes: true,
            bubbles: true,
        }
    }
}

impl WaterEffects {
    pub fn flags(self) -> u32 {
        if !self.enabled {
            return 0;
        }
        [
            self.waves,
            self.shore_foam,
            self.contact_foam,
            self.crest_foam,
            self.interactions,
            self.caustics,
            self.splashes,
            self.bubbles,
            self.absorption,
            self.fresnel,
            self.wet_ground,
        ]
        .iter()
        .enumerate()
        .fold(0, |bits, (i, on)| bits | (u32::from(*on) << i))
    }
    pub fn resolution(self) -> u32 {
        if self.flags() & 0x16 == 0 {
            return 1;
        }
        [128, 256, 384, 512][self.quality.min(3) as usize]
    }
    pub fn particle_count(self) -> u32 {
        [4, 6, 8, 12][self.quality.min(3) as usize]
    }
    pub fn range(self) -> f32 {
        [64.0, 80.0, 96.0, 128.0][self.quality.min(3) as usize]
    }
}

/// Height of the displayed macro surface for visual contacts and the camera.
/// Invert the Gerstner horizontal offset so the query uses rendered world XY.
/// Fine interaction ripples do not feed back into their own emitters.
pub fn surface_level(xy: Vec2, time: f32, settings: WaterEffects, level: f32, ground: f32) -> f32 {
    if !settings.enabled || !settings.waves {
        return level;
    }
    let t = ((level - ground - 0.08) / 2.32).clamp(0.0, 1.0);
    let shore = t * t * (3.0 - 2.0 * t);
    let sample = |p: Vec2| {
        let mut offset = glam::Vec3::ZERO;
        for (i, (direction, k, amplitude)) in [
            (Vec2::new(0.88, 0.475), 0.19f32, 0.24),
            (Vec2::new(0.62, 0.785), 0.32, 0.115),
            (Vec2::new(-0.22, 0.975), 0.48, 0.055),
            (Vec2::new(0.97, -0.243), 0.71, 0.025),
        ]
        .into_iter()
        .take((settings.quality.min(3) + 2).min(4) as usize)
        .enumerate()
        {
            let phase = p.dot(direction) * k - time * (9.81 * k).sqrt() + i as f32 * 1.73;
            offset += (direction * phase.cos() * amplitude * 0.85).extend(phase.sin() * amplitude);
        }
        offset * shore
    };
    let mut source = xy;
    for _ in 0..3 {
        source = xy - sample(source).truncate();
    }
    level + sample(source).z
}

/// Splash impact (0), sparse air release (1), obstacle footprint (2), or
/// displacement without particles (3), an intersecting avatar's surface wake (4),
/// a radial dive/cannonball sheet (5), a water-entry displacement ring (6),
/// a directed limb/bow fan (7), or an anatomical surface strike (8).
/// Footprints are bounded approximations, not a replacement for mesh physics.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, Pod, Zeroable)]
pub struct WaterEvent {
    /// xyz world position, w birth time on FrameParams::time.
    pub position: [f32; 4],
    /// xyz velocity (obstacles: horizontal unit axis), w impulse strength.
    pub motion: [f32; 4],
    /// x radius / obstacle half length, y kind, z water height.
    /// w: obstacle half width, swept segment length (4), sheet span (5), or footfall flag (7/8).
    pub params: [f32; 4],
}

#[derive(Debug, Clone, Default)]
pub struct WaterGround {
    pub region: u64,
    pub origin: [f32; 2],
    pub extent: f32,
    /// ground height, water height; unknown cells are deep void water.
    pub heights: std::sync::Arc<[[f32; 2]]>,
}

impl WaterGround {
    /// Camera-local macro waterline, sharing the displayed swell and shore
    /// damping with visual contacts. Never replace the region's physical plane.
    pub(crate) fn camera_level(&self, xy: Vec2, time: f32, settings: WaterEffects, fallback: f32) -> f32 {
        if self.heights.len() != 4096 || !self.extent.is_finite() || self.extent <= 0.0 {
            return fallback;
        }
        let [ground, level] = self.sample(xy).unwrap_or([fallback - 20.0, fallback]);
        surface_level(xy, time, settings, level, ground)
    }

    /// Same bilinear bathymetry reconstruction as water_fx_bed. Contacts must
    /// meet the displayed, shore-damped surface rather than a finer CPU bed.
    pub fn sample(&self, xy: Vec2) -> Option<[f32; 2]> {
        if self.heights.len() != 4096 || !self.extent.is_finite() || self.extent <= 0.0 || !xy.is_finite() {
            return None;
        }
        let uv = (xy - Vec2::from_array(self.origin)) / self.extent;
        if uv.cmplt(Vec2::ZERO).any() || uv.cmpgt(Vec2::ONE).any() {
            return None;
        }
        let cell = (uv * 64.0 - Vec2::splat(0.5)).clamp(Vec2::ZERO, Vec2::splat(63.0));
        let i = cell.floor().as_uvec2();
        let j = (i + glam::UVec2::ONE).min(glam::UVec2::splat(63));
        let f = cell.fract();
        let a = Vec2::from_array(self.heights[(i.y * 64 + i.x) as usize]);
        let b = Vec2::from_array(self.heights[(i.y * 64 + j.x) as usize]);
        let c = Vec2::from_array(self.heights[(j.y * 64 + i.x) as usize]);
        let d = Vec2::from_array(self.heights[(j.y * 64 + j.x) as usize]);
        Some(a.lerp(b, f.x).lerp(c.lerp(d, f.x), f.y).to_array())
    }
}

/// Particle emitters occupy the prefix read by the instanced particle shader.
/// Ripples remain available to compute without generating invisible billboards;
/// newest emitters take priority when the shared particle budget is exhausted.
fn pack_events(events: &[WaterEvent], settings: WaterEffects, packed: &mut [WaterEvent]) -> (usize, usize) {
    let flags = settings.flags();
    let particle =
        |e: &&WaterEvent| (matches!(e.params[1], 0.0 | 5.0 | 7.0) && flags & 64 != 0) || (e.params[1] == 1.0 && flags & 128 != 0);
    let recent = &events[events.len().saturating_sub(packed.len())..];
    let mut n = 0;
    for e in recent.iter().rev().filter(particle) {
        packed[n] = *e;
        n += 1;
    }
    let emitters = n;
    for e in recent.iter().rev().filter(|e| !particle(e) && !matches!(e.params[1], 4.0 | 8.0)) {
        packed[n] = *e;
        n += 1;
    }
    for e in recent.iter().rev().filter(|e| matches!(e.params[1], 4.0 | 8.0)) {
        packed[n] = *e;
        n += 1;
    }
    (n, emitters)
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniform {
    area: [f32; 4],     // origin xy, extent, inverse texel size
    previous: [f32; 4], // old origin xy, reset, dt
    clock: [f32; 4],    // now, previous time, fine-wake origin xy
    counts: [u32; 4],   // events, contacts, flags, quality
}

pub(crate) struct WaterGpu {
    pub view: wgpu::TextureView,
    pub caustics_view: wgpu::TextureView,
    wake: crate::water_wake::WakeGpu,
    event_count: u32,
    caustics_pipeline: wgpu::RenderPipeline,
    caustics_group: wgpu::BindGroup,
    textures: [wgpu::Texture; 2],
    pub uniform: wgpu::Buffer,
    pub events: wgpu::Buffer,
    ground: wgpu::Texture,
    pub ground_view: wgpu::TextureView,
    ground_snapshot: Option<std::sync::Arc<[[f32; 2]]>>,
    group: wgpu::BindGroup,
    pipeline: wgpu::ComputePipeline,
    size: u32,
    origin: Vec2,
    last_time: Option<f32>,
    region: u64,
    flags: u32,
    quality: u8,
    pub particles: u32,
}

impl WaterGpu {
    pub fn new(device: &wgpu::Device, size: u32) -> Self {
        use wgpu::util::DeviceExt;
        let textures = std::array::from_fn(|index| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("water ripple/foam field"),
                size: wgpu::Extent3d {
                    width: size,
                    height: if index == 0 { size * 2 } else { size },
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::STORAGE_BINDING
                    | wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::COPY_DST
                    | wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
        });
        let view = textures[0].create_view(&Default::default());
        let output = textures[1].create_view(&Default::default());
        let ground = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("water bathymetry"),
            size: wgpu::Extent3d {
                width: WATER_GROUND_SIZE,
                height: WATER_GROUND_SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rg32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("water effects"),
            contents: bytemuck::bytes_of(&Uniform::zeroed()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let events = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("water interaction events"),
            size: (MAX_WATER_EVENTS * std::mem::size_of::<WaterEvent>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("water simulation"),
            source: wgpu::ShaderSource::Wgsl(crate::water_shader_source(
                &["water_sim.wgsl"],
                include_str!("shaders/water_sim.wgsl"),
            )),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("water simulation"),
            layout: None,
            module: &module,
            entry_point: Some("simulate"),
            compilation_options: Default::default(),
            cache: None,
        });
        let ground_view = ground.create_view(&Default::default());
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("water simulation"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: events.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&output),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&ground_view),
                },
            ],
        });
        let caustics = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("refracted water light"),
            size: wgpu::Extent3d {
                width: 256,
                height: 256,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let caustics_view = caustics.create_view(&Default::default());
        let caustics_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("refracted water caustics"),
            source: wgpu::ShaderSource::Wgsl(crate::water_shader_source(
                &["water_spectrum.wgsl", "water_caustics.wgsl"],
                concat!(
                    include_str!("shaders/water_spectrum.wgsl"),
                    "\n",
                    include_str!("shaders/water_caustics.wgsl")
                ),
            )),
        });
        let additive = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        };
        let caustics_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("refracted water caustics"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &caustics_shader,
                entry_point: Some("vs_caustic"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &caustics_shader,
                entry_point: Some("fs_caustic"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba16Float,
                    blend: Some(wgpu::BlendState {
                        color: additive,
                        alpha: additive,
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let caustics_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("caustics controls"),
            layout: &caustics_pipeline.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        let wake = crate::water_wake::WakeGpu::new(device, &uniform, &events, &ground_view);
        Self {
            wake,
            event_count: 0,
            view,
            caustics_view,
            caustics_pipeline,
            caustics_group,
            textures,
            uniform,
            events,
            ground,
            ground_view,
            ground_snapshot: None,
            group,
            pipeline,
            size,
            origin: Vec2::ZERO,
            last_time: None,
            region: 0,
            flags: 0,
            quality: 0,
            particles: 0,
        }
    }

    /// Returns whether the sampled view changed (the frame groups then rebind it).
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        settings: WaterEffects,
        time: f32,
        eye: Vec2,
        ground: &WaterGround,
        events: &[WaterEvent],
        contacts: &[WaterEvent],
    ) -> bool {
        let valid_ground = ground.heights.len() == (WATER_GROUND_SIZE * WATER_GROUND_SIZE) as usize && ground.extent > 0.0;
        let resolution = if valid_ground { settings.resolution() } else { 1 };
        let resized = self.size != resolution;
        if resized {
            *self = Self::new(device, resolution);
        }
        let origin = Vec2::from_array(ground.origin);
        let extent = ground.extent.max(1.0);
        let reset = self.region != ground.region
            || self.last_time.is_none()
            || time < self.last_time.unwrap_or(time)
            || self.origin.distance(origin) >= extent;
        let previous_time = self.last_time.unwrap_or(time);
        let dt = (time - previous_time).clamp(0.0, 1.0 / 30.0);
        let mut packed = [WaterEvent::default(); MAX_WATER_IMPULSES];
        let (n, emitters) = pack_events(events, settings, &mut packed);
        let wakes = packed[..n].iter().filter(|e| matches!(e.params[1], 4.0 | 8.0)).count();
        let regular = n - wakes;
        let c = contacts.len().min(MAX_WATER_EVENTS - n);
        // Compute sees only impulses and obstacles. Fine historical segments
        // follow those ranges, so the coarse simulation never loops over them.
        self.event_count = (n + c) as u32;
        if regular > 0 {
            queue.write_buffer(&self.events, 0, bytemuck::cast_slice(&packed[..regular]));
        }
        if c > 0 {
            queue.write_buffer(
                &self.events,
                (regular * std::mem::size_of::<WaterEvent>()) as u64,
                bytemuck::cast_slice(&contacts[..c]),
            );
        }
        if wakes > 0 {
            queue.write_buffer(
                &self.events,
                ((regular + c) * std::mem::size_of::<WaterEvent>()) as u64,
                bytemuck::cast_slice(&packed[regular..n]),
            );
        }
        let flags = settings.flags();
        let u = Uniform {
            area: [origin.x, origin.y, extent, self.size as f32 / extent],
            previous: [
                self.origin.x,
                self.origin.y,
                if reset {
                    1.0
                } else if (self.flags ^ flags) & 6 != 0 {
                    2.0
                } else {
                    0.0
                },
                dt,
            ],
            clock: {
                let cell = 32.0 / self.size as f32;
                let fine_origin = (eye / cell).floor() * cell - Vec2::splat(16.0);
                [time, previous_time, fine_origin.x, fine_origin.y]
            },
            counts: [
                regular as u32,
                c as u32,
                if valid_ground { flags } else { 0 },
                settings.quality.min(3) as u32,
            ],
        };
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&u));
        if valid_ground
            && flags != 0
            && self
                .ground_snapshot
                .as_ref()
                .is_none_or(|old| !std::sync::Arc::ptr_eq(old, &ground.heights))
        {
            self.ground_snapshot = Some(ground.heights.clone());
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.ground,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                bytemuck::cast_slice(ground.heights.as_ref()),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(WATER_GROUND_SIZE * 8),
                    rows_per_image: None,
                },
                wgpu::Extent3d {
                    width: WATER_GROUND_SIZE,
                    height: WATER_GROUND_SIZE,
                    depth_or_array_layers: 1,
                },
            );
        }
        self.particles = if flags & 0xc0 != 0 {
            (emitters as u32 * settings.particle_count()).min(settings.particle_budget)
        } else {
            0
        };
        self.flags = if valid_ground { flags } else { 0 };
        self.quality = settings.quality.min(3);
        self.region = ground.region;
        self.origin = origin;
        self.last_time = Some(time);
        resized
    }

    pub fn encode(&self, encoder: &mut wgpu::CommandEncoder) {
        if self.flags & 32 != 0 {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("refracted water light"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.caustics_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.caustics_pipeline);
            pass.set_bind_group(0, &self.caustics_group, &[]);
            let n = [96, 160, 224, 256][self.quality as usize];
            pass.draw(0..6, 0..n * n * 3);
        }
        if self.size == 1 {
            return;
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("water ripples and foam"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.group, &[]);
            pass.dispatch_workgroups(self.size.div_ceil(8), self.size.div_ceil(8), 1);
        }
        self.wake.encode(
            encoder,
            &self.view,
            self.size,
            if self.flags & 16 != 0 { self.event_count } else { 0 },
        );
        encoder.copy_texture_to_texture(
            self.textures[1].as_image_copy(),
            self.textures[0].as_image_copy(),
            wgpu::Extent3d {
                width: self.size,
                height: self.size,
                depth_or_array_layers: 1,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_waterline_follows_crests_and_troughs_and_respects_flat_modes() {
        let ground = WaterGround {
            origin: [0.0; 2],
            extent: 256.0,
            heights: vec![[0.0, 20.0]; 4096].into(),
            ..Default::default()
        };
        let xy = Vec2::new(43.0, 118.0);
        for quality in 0..4 {
            let settings = WaterEffects {
                quality,
                ..Default::default()
            };
            let levels: Vec<_> = (0..200).map(|i| ground.camera_level(xy, i as f32 * 0.1, settings, 99.0)).collect();
            assert!(
                levels.iter().any(|level| *level > 20.15),
                "crests must submerge a camera above native water"
            );
            assert!(
                levels.iter().any(|level| *level < 19.85),
                "troughs must uncover a camera below native water"
            );
            for flat in [
                WaterEffects {
                    enabled: false,
                    ..settings
                },
                WaterEffects { waves: false, ..settings },
            ] {
                assert_eq!(ground.camera_level(xy, 7.3, flat, 99.0), 20.0);
            }
        }
        assert_eq!(WaterGround::default().camera_level(xy, 7.3, WaterEffects::default(), 20.0), 20.0);
        let shallow = WaterGround {
            heights: vec![[19.95, 20.0]; 4096].into(),
            ..ground
        };
        assert_eq!(shallow.camera_level(xy, 7.3, WaterEffects::default(), 20.0), 20.0);
    }

    #[test]
    fn furrow_atlas_has_a_trough_and_outgoing_shoulders_without_particles_or_foam() {
        let _gpu = crate::test_gpu::lock();
        let instance = wgpu::Instance::default();
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
            return;
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("GPU device");
        let mut gpu = WaterGpu::new(&device, 256);
        let mut settings = WaterEffects {
            waves: false,
            shore_foam: false,
            contact_foam: false,
            crest_foam: false,
            caustics: false,
            splashes: false,
            bubbles: false,
            ..Default::default()
        };
        let ground = WaterGround {
            region: 1,
            origin: [-80.0; 2],
            extent: 160.0,
            heights: vec![[0.0, 20.0]; 4096].into(),
        };
        let mut events: Vec<_> = (0..25)
            .map(|i| {
                let age = i as f32 * 0.08;
                WaterEvent {
                    position: [-age * 1.5, 0.0, 19.94, 10.0 - age],
                    motion: [1.5, 0.0, 0.0, 0.3],
                    params: [0.35, 4.0, 20.0, 0.12],
                }
            })
            .collect();
        events.push(WaterEvent {
            position: [4.0, 0.0, 20.0, 9.8],
            motion: [0.0, 0.0, -2.0, 0.3],
            params: [0.07, 8.0, 20.0, 0.0],
        });
        events.push(WaterEvent {
            position: [4.0, 4.0, 20.0, 9.8],
            motion: [0.0, 0.0, -2.0, 0.3],
            params: [0.07, 8.0, 20.0, 1.0],
        });
        // Read the production atlas as f32 on the GPU, avoiding a second Rust
        // implementation of the wake or assumptions about half-float storage.
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("furrow atlas regression probe"),
            source: wgpu::ShaderSource::Wgsl(
                r#"
@group(0) @binding(0) var atlas:texture_2d<f32>;
@group(0) @binding(1) var<storage,read_write> result:array<vec4<f32>>;
@compute @workgroup_size(8,8)
fn probe(@builtin(global_invocation_id) id:vec3<u32>) {
    result[id.y*256u+id.x]=textureLoad(atlas,vec2<i32>(id.xy)+vec2<i32>(0,256),0);
}"#
                .into(),
            ),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: None,
            layout: None,
            module: &shader,
            entry_point: Some("probe"),
            compilation_options: Default::default(),
            cache: None,
        });
        let bytes = 256 * 256 * 16;
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&gpu.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: output.as_entire_binding(),
                },
            ],
        });
        for (enabled, bed, level) in [(true, 0.0, 20.0), (false, 0.0, 20.0), (true, 21.0, 20.0), (true, 0.0, 22.0)] {
            let ground = WaterGround {
                heights: vec![[bed, level]; 4096].into(),
                ..ground.clone()
            };
            settings.interactions = enabled;
            settings.shore_foam = !enabled; // Keep the same atlas allocation while disabling interactions.
            gpu.prepare(&device, &queue, settings, 10.0, Vec2::ZERO, &ground, &events, &[]);
            assert_eq!(gpu.particles, 0);
            let mut encoder = device.create_command_encoder(&Default::default());
            gpu.encode(&mut encoder);
            {
                let mut pass = encoder.begin_compute_pass(&Default::default());
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &group, &[]);
                pass.dispatch_workgroups(32, 32, 1);
            }
            encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, bytes);
            queue.submit([encoder.finish()]);
            readback.map_async(wgpu::MapMode::Read, .., |r| r.expect("atlas readback"));
            device.poll(wgpu::PollType::wait_indefinitely()).expect("GPU completion");
            let mapped = readback.get_mapped_range(..).expect("mapped atlas");
            let samples: &[[f32; 4]] = bytemuck::cast_slice(&mapped);
            assert!(samples.iter().flatten().all(|v| v.is_finite()));
            if enabled && bed == 0.0 && level == 20.0 {
                let at = |x: f32, y: f32| samples[((y + 16.0) * 8.0) as usize * 256 + ((x + 16.0) * 8.0) as usize];
                assert!(
                    at(4.0, 0.125)[2] > 0.005,
                    "a small vertical hand/head strike has its own expanding ring"
                );
                assert!(
                    at(4.0, 4.125)[2] > 0.003 && at(4.0, 4.125)[2] < at(4.0, 0.125)[2] * 0.8,
                    "a footfall makes a shallower local ring, not a large water-entry wave"
                );
                assert!(at(-0.25, 0.0)[2] < -0.02, "a real depression behind the body: {:?}", at(-0.25, 0.0));
                assert!(at(-0.5, 0.5)[2] > 0.004, "raised shoulder: {:?}", at(-0.5, 0.5));
                assert!(at(-1.5, 1.0)[2] > 0.001, "older waves spread outward");
                assert!(at(-0.5, 0.25)[1] > 0.01, "normal tilts up the side of the furrow");
                assert!(
                    samples.iter().all(|p| p[2].abs() < 0.2 && p[3] == 0.0),
                    "bounded displacement, independently disabled foam"
                );
            } else {
                assert!(
                    samples.iter().flatten().all(|v| *v == 0.0),
                    "disabled interactions, dry ground and another water level cannot retain a furrow"
                );
            }
            drop(mapped);
            readback.unmap();
        }
        let source = format!(
            "{}\n{}",
            include_str!("shaders/water_spectrum.wgsl"),
            r#"
@group(0) @binding(0) var<storage,read_write> result:array<vec4<f32>>;
@compute @workgroup_size(1)
fn probe(@builtin(global_invocation_id) id:vec3<u32>) {
    let query=id.x/4u; let quality=id.x%4u;
    let xy=vec2<f32>(f32(query)*13.7-20.0,f32(query)*2.3);
    let time=7.3+f32(query)*0.4;
    let depth=array<f32,3>(0.1,0.7,3.0)[query%3u];
    let shore=smoothstep(0.08,2.4,depth);
    var p=xy;
    for(var i=0u;i<3u;i++){p=xy-water_swell(p,time,quality).offset.xy*shore;}
    result[id.x]=vec4<f32>(20.0+water_swell(p,time,quality).offset.z*shore,0.0,0.0,0.0);
}
"#
        );
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("CPU contact / rendered swell agreement"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: None,
            layout: None,
            module: &shader,
            entry_point: Some("probe"),
            compilation_options: Default::default(),
            cache: None,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: output.as_entire_binding(),
            }],
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(24, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, 24 * 16);
        queue.submit([encoder.finish()]);
        readback.map_async(wgpu::MapMode::Read, ..24 * 16, |r| r.expect("swell readback"));
        device.poll(wgpu::PollType::wait_indefinitely()).expect("GPU completion");
        let mapped = readback.get_mapped_range(..24 * 16).expect("mapped swell");
        let samples: &[[f32; 4]] = bytemuck::cast_slice(&mapped);
        for (index, sample) in samples.iter().enumerate() {
            let query = index / 4;
            let quality = (index % 4) as u8;
            let xy = Vec2::new(query as f32 * 13.7 - 20.0, query as f32 * 2.3);
            let time = 7.3 + query as f32 * 0.4;
            let ground = 20.0 - [0.1, 0.7, 3.0][query % 3];
            let settings = WaterEffects {
                quality,
                ..Default::default()
            };
            let cpu = surface_level(xy, time, settings, 20.0, ground);
            assert!(
                (cpu - sample[0]).abs() < 0.0003,
                "CPU contact must meet rendered water: {cpu} vs {}",
                sample[0]
            );
            assert_eq!(
                surface_level(xy, time, WaterEffects { waves: false, ..settings }, 20.0, ground),
                20.0
            );
        }
        drop(mapped);
        readback.unmap();
    }
    #[test]
    fn entry_sheets_and_surface_fans_are_in_the_particle_prefix_but_rings_are_not() {
        let events: Vec<_> = [6.0, 5.0, 1.0, 0.0, 7.0]
            .into_iter()
            .map(|kind| WaterEvent {
                params: [0.2, kind, 20.0, 0.0],
                ..Default::default()
            })
            .collect();
        let mut packed = [WaterEvent::default(); 5];
        assert_eq!(pack_events(&events, WaterEffects::default(), &mut packed), (5, 4));
        assert!(packed[..4].iter().all(|e| e.params[1] != 6.0));
        let settings = WaterEffects {
            splashes: false,
            ..Default::default()
        };
        assert_eq!(pack_events(&events, settings, &mut packed), (5, 1));
        assert_eq!(packed[0].params[1], 1.0);
    }

    #[test]
    fn effects_disable_independently_and_bound_work() {
        let mut s = WaterEffects::default();
        assert_eq!(s.flags(), 2047);
        s.bubbles = false;
        assert_eq!(s.flags(), 1919);
        s.quality = 255;
        assert_eq!(s.resolution(), 512);
        s.enabled = false;
        assert_eq!(s.flags(), 0);
        assert_eq!(s.resolution(), 1);
    }

    #[test]
    fn only_enabled_particle_emitters_occupy_the_draw_prefix() {
        let event = |kind, time| WaterEvent {
            params: [0.2, kind, 0.0, 0.0],
            position: [0.0, 0.0, 0.0, time],
            ..Default::default()
        };
        let events = [event(0.0, 1.0), event(3.0, 2.0), event(1.0, 3.0), event(3.0, 4.0)];
        let mut packed = [WaterEvent::default(); 4];
        let mut settings = WaterEffects::default();
        assert_eq!(pack_events(&events, settings, &mut packed), (4, 2));
        assert_eq!([packed[0].position[3], packed[1].position[3]], [3.0, 1.0]);
        settings.bubbles = false;
        assert_eq!(pack_events(&events, settings, &mut packed), (4, 1));
        assert_eq!(packed[0].params[1], 0.0);
        settings.splashes = false;
        assert_eq!(pack_events(&events, settings, &mut packed), (4, 0));
        assert_eq!(pack_events(&events, WaterEffects::default(), &mut packed[..2]), (2, 1));
        assert_eq!(packed[0].position[3], 3.0);
    }
}
