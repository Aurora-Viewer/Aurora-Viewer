//! The panorama viewer: a 2:1 equirectangular image looked into by a
//! virtual camera (the inventory's 360° texture preview, the photo window's
//! 360° result). A paint-callback region like the other previews
//! (`preview.rs`): the render thread draws it into a target of its own,
//! which egui shows as a mesh, so the viewer needs no readback and no GPU
//! wait, and redraws only when the view or the image changes.
//!
//! The image keeps a full mip chain (a 2x2 box blit per level, in the frame's
//! encoder) and is sampled with explicit gradients taken from the view
//! direction, which is continuous where the longitude wraps from +PI to -PI:
//! the implicit derivatives of the texture coordinate would jump there and
//! pick the smallest level for a one-pixel seam (`shaders/panorama.wgsl`).
//! Mips and the target are Rgba8Unorm and filtered in gamma space, on
//! purpose: the values egui shows are the image's own, as the flat preview's
//! images are (an sRGB view would average in linear light and shift the
//! picture's tones).

use bytemuck::{Pod, Zeroable};
use glam::{Vec2, Vec3};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Weak},
};

pub use crate::preview::Image;

const PI: f32 = std::f32::consts::PI;
const TAU: f32 = std::f32::consts::TAU;

/// Where the virtual camera looks: the compass heading (0 north = +Y, east
/// = +X is a quarter turn), the pitch up, and the vertical field of view,
/// all in radians.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PanoParams {
    pub yaw: f32,
    pub pitch: f32,
    pub fov_y: f32,
}

/// The unit direction a heading and a pitch look along (the z-up world).
pub fn direction(yaw: f32, pitch: f32) -> Vec3 {
    let c = pitch.cos();
    Vec3::new(yaw.sin() * c, yaw.cos() * c, pitch.sin())
}

/// The forward axis and the right and up axes scaled by the half-extent of
/// the view at distance one, for a viewport of width over height `aspect`.
pub fn view_basis(p: PanoParams, aspect: f32) -> [Vec3; 3] {
    let forward = direction(p.yaw, p.pitch);
    let right = forward.cross(Vec3::Z).normalize_or_zero();
    let up = right.cross(forward);
    let half = (p.fov_y * 0.5).tan();
    [forward, right * half * aspect, up * half]
}

/// The image coordinates (0..1, top left) of a direction: the inverse of
/// `equirect::pixel_direction`.
pub fn uv_of(d: Vec3) -> Vec2 {
    Vec2::new(d.x.atan2(d.y) / TAU + 0.5, 0.5 - d.z.clamp(-1.0, 1.0).asin() / PI)
}

/// The gradients of `uv_of` along two screen directions, from those of the
/// unit direction. Continuous across the wrap, bounded at the poles.
pub fn uv_grad(d: Vec3, ddx: Vec3, ddy: Vec3) -> (Vec2, Vec2) {
    let r2 = (d.x * d.x + d.y * d.y).max(1e-6);
    let k = 1.0 / (1.0 - d.z * d.z).max(1e-6).sqrt();
    let one = |dd: Vec3| Vec2::new(((d.y * dd.x - d.x * dd.y) / r2 / TAU).clamp(-0.5, 0.5), -dd.z * k / PI);
    (one(ddx), one(ddy))
}

/// A panorama to draw into the rectangle of an egui callback.
pub struct PanoRegion {
    id: egui::Id,
    image: Arc<Image>,
    params: PanoParams,
}

impl PanoRegion {
    pub(crate) fn id(&self) -> egui::Id {
        self.id
    }
}

/// Adds the viewer to `painter` over `rect`. `id` names its render target.
pub fn paint_in(painter: &egui::Painter, rect: egui::Rect, id: egui::Id, image: Arc<Image>, params: PanoParams) {
    painter.add(egui::Shape::Callback(egui::PaintCallback {
        rect,
        callback: Arc::new(PanoRegion { id, image, params }),
    }));
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniform {
    forward: [f32; 4],
    right: [f32; 4],
    up: [f32; 4],
}

impl Uniform {
    fn new(params: PanoParams, aspect: f32) -> Self {
        let [f, r, u] = view_basis(params, aspect);
        Uniform {
            forward: f.extend(0.0).to_array(),
            right: r.extend(0.0).to_array(),
            up: u.extend(0.0).to_array(),
        }
    }
}

struct Pipelines {
    layout: wgpu::BindGroupLayout,
    pano: wgpu::RenderPipeline,
    blit: wgpu::RenderPipeline,
    view_sampler: wgpu::Sampler,
    blit_sampler: wgpu::Sampler,
}

impl Pipelines {
    fn new(device: &wgpu::Device) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("panorama"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("panorama"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("panorama"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/panorama.wgsl").into()),
        });
        let pipeline = |entry: &str| {
            use crate::pipeline_cache::CachedPipelines;
            device.create_render_pipeline_cached(&wgpu::RenderPipelineDescriptor {
                label: Some("panorama"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_full"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        // anisotropic filtering needs linear filters throughout; across the
        // longitude the image repeats, over the poles it clamps
        let view_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("panorama"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 16,
            ..Default::default()
        });
        let blit_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("panorama mips"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            layout,
            pano: pipeline("fs_pano"),
            blit: pipeline("fs_blit"),
            view_sampler,
            blit_sampler,
        }
    }
}

/// The image on the GPU, with its mips.
struct Source {
    view: wgpu::TextureView,
}

fn mip_count(width: u32, height: u32) -> u32 {
    32 - width.max(height).max(1).leading_zeros()
}

impl Source {
    /// Uploads `image` (level 0) and records the blits of the other levels
    /// in `encoder`. None when the image is empty, malformed or larger than
    /// the card takes.
    fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        pipelines: &Pipelines,
        image: &Image,
    ) -> Option<Self> {
        let [w, h] = image.size;
        let limit = device.limits().max_texture_dimension_2d;
        if w == 0 || h == 0 || w > limit || h > limit || image.rgba.len() != w as usize * h as usize * 4 {
            return None;
        }
        let levels = mip_count(w, h);
        let desc = wgpu::TextureDescriptor {
            label: Some("panorama"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        };
        // (up to 180 MB with its mips: refused, the viewer stays black)
        let bytes = u64::from(w) * u64::from(h) * 4 * 4 / 3;
        let Ok((texture, _)) = crate::gpu_alloc::try_texture(device, &desc, &Default::default(), bytes, false) else {
            log::warn!("panorama: the {w} x {h} image was refused by the card");
            return None;
        };
        queue.write_texture(
            texture.as_image_copy(),
            &image.rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * 4),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("panorama mips"),
            size: std::mem::size_of::<Uniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM,
            mapped_at_creation: false,
        });
        let level_view = |level| {
            texture.create_view(&wgpu::TextureViewDescriptor {
                base_mip_level: level,
                mip_level_count: Some(1),
                ..Default::default()
            })
        };
        for level in 1..levels {
            let from = level_view(level - 1);
            let to = level_view(level);
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("panorama mip"),
                layout: &pipelines.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&from),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&pipelines.blit_sampler),
                    },
                ],
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("panorama mip"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &to,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&pipelines.blit);
            pass.set_bind_group(0, &group, &[]);
            pass.draw(0..3, 0..1);
        }
        Some(Source {
            view: texture.create_view(&Default::default()),
        })
    }
}

/// What a draw is made of.
struct Draw<'a> {
    uniform: &'a wgpu::Buffer,
    source: Option<&'a Source>,
    target: &'a wgpu::TextureView,
    size: [u32; 2],
    params: PanoParams,
}

/// Draws the view of a source into a target.
fn draw(device: &wgpu::Device, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, pipelines: &Pipelines, d: Draw) {
    queue.write_buffer(
        d.uniform,
        0,
        bytemuck::bytes_of(&Uniform::new(d.params, d.size[0] as f32 / d.size[1].max(1) as f32)),
    );
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("panorama"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: d.target,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    // (an image the card cannot take stays black)
    let Some(source) = d.source else {
        return;
    };
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("panorama"),
        layout: &pipelines.layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: d.uniform.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&source.view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(&pipelines.view_sampler),
            },
        ],
    });
    pass.set_pipeline(&pipelines.pano);
    pass.set_bind_group(0, &group, &[]);
    pass.draw(0..3, 0..1);
}

struct Target {
    size: [u32; 2],
    color: wgpu::TextureView,
    texture: egui::TextureId,
    uniform: wgpu::Buffer,
    /// What was drawn: the image (a weak reference, which keeps its address
    /// from being taken by another image) and the view.
    last: Option<(Weak<Image>, PanoParams)>,
}

/// The viewers' targets and the images on the GPU, owned by `preview::Cache`.
#[derive(Default)]
pub(crate) struct Cache {
    pipelines: Option<Pipelines>,
    targets: HashMap<egui::Id, Target>,
    sources: HashMap<usize, (Weak<Image>, Option<Source>)>,
    /// The images drawn this frame: the others are let go (a closed window's
    /// panorama is not kept on the GPU).
    used: HashSet<usize>,
}

impl Cache {
    /// The mesh that shows `region` in `rect`, the target drawn first when
    /// the view or the image changed.
    pub(crate) fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        egui: &mut egui_wgpu::Renderer,
        region: &PanoRegion,
        rect: egui::Rect,
        scale: f32,
    ) -> egui::Mesh {
        let pipelines = self.pipelines.get_or_insert_with(|| Pipelines::new(device));
        let size = crate::preview::target_size(rect.size(), scale);
        if self.targets.get(&region.id).is_none_or(|t| t.size != size) {
            if let Some(old) = self.targets.remove(&region.id) {
                egui.free_texture(&old.texture);
            }
            let color = device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("panorama view"),
                    size: wgpu::Extent3d {
                        width: size[0],
                        height: size[1],
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default());
            let texture = egui.register_native_texture(device, &color, wgpu::FilterMode::Linear);
            let uniform = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("panorama view"),
                size: std::mem::size_of::<Uniform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.targets.insert(
                region.id,
                Target {
                    size,
                    color,
                    texture,
                    uniform,
                    last: None,
                },
            );
        }
        let key = Arc::as_ptr(&region.image) as *const () as usize;
        self.used.insert(key);
        // (an entry whose image is gone is made again, whatever the address)
        if self
            .sources
            .get(&key)
            .is_some_and(|(owner, _)| owner.upgrade().is_none_or(|a| !Arc::ptr_eq(&a, &region.image)))
        {
            self.sources.remove(&key);
        }
        let source = self.sources.entry(key).or_insert_with(|| {
            (
                Arc::downgrade(&region.image),
                Source::new(device, queue, encoder, pipelines, &region.image),
            )
        });
        let target = self.targets.get_mut(&region.id).expect("panorama target initialized");
        let drawn = target
            .last
            .as_ref()
            .is_some_and(|(image, params)| *params == region.params && image.upgrade().is_some_and(|a| Arc::ptr_eq(&a, &region.image)));
        if !drawn {
            target.last = Some((Arc::downgrade(&region.image), region.params));
            draw(
                device,
                queue,
                encoder,
                pipelines,
                Draw {
                    uniform: &target.uniform,
                    source: source.1.as_ref(),
                    target: &target.color,
                    size: target.size,
                    params: region.params,
                },
            );
        }
        let mut mesh = egui::Mesh::with_texture(target.texture);
        mesh.add_rect_with_uv(
            rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
        mesh
    }

    /// Forgets the viewers not drawn this frame and the images nobody holds.
    pub(crate) fn retain(&mut self, seen: &HashSet<egui::Id>, egui: &mut egui_wgpu::Renderer) {
        self.targets.retain(|id, target| {
            if seen.contains(id) {
                true
            } else {
                egui.free_texture(&target.texture);
                false
            }
        });
        let used = std::mem::take(&mut self.used);
        self.sources
            .retain(|key, (owner, _)| owner.strong_count() > 0 && used.contains(key));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::equirect::pixel_direction;

    #[test]
    fn uv_is_the_inverse_of_the_projection_pixels() {
        let (w, h) = (4096u32, 2048u32);
        for (x, y) in [(2048.0, 1024.0), (3072.0, 1024.0), (1.0, 700.0), (4095.0, 300.0), (1500.5, 1900.5)] {
            let uv = uv_of(pixel_direction(x, y, w, h));
            assert!(
                (uv.x - x / w as f32).abs() < 1e-4 && (uv.y - y / h as f32).abs() < 1e-4,
                "({x}, {y}): {uv:?}"
            );
        }
    }

    /// The direction of a screen position of a view, as the shader makes it.
    fn ray(p: PanoParams, aspect: f32, ndc: Vec2) -> Vec3 {
        let [f, r, u] = view_basis(p, aspect);
        (f + r * ndc.x + u * ndc.y).normalize()
    }

    #[test]
    fn the_view_centre_looks_along_its_heading_and_east_is_to_its_right() {
        let p = PanoParams {
            yaw: 0.0,
            pitch: 0.0,
            fov_y: 1.0,
        };
        assert!((ray(p, 1.0, Vec2::ZERO) - Vec3::Y).length() < 1e-5);
        assert!(ray(p, 1.0, Vec2::new(0.5, 0.0)).x > 0.0);
        assert!(ray(p, 1.0, Vec2::new(0.0, 0.5)).z > 0.0);
        let east = PanoParams { yaw: PI / 2.0, ..p };
        assert!((ray(east, 1.0, Vec2::ZERO) - Vec3::X).length() < 1e-5);
    }

    /// Gradients of the same pixels astride the wrap agree, though the
    /// coordinate itself jumps from nearly 1 to nearly 0.
    #[test]
    fn gradients_are_continuous_across_the_wrap() {
        let p = PanoParams {
            yaw: PI,
            pitch: 0.2,
            fov_y: 1.2,
        };
        let (aspect, step) = (1.5, 1e-3);
        let at = |x: f32, y: f32| ray(p, aspect, Vec2::new(x, y));
        let grad = |x: f32| {
            let d = at(x, 0.0);
            uv_grad(d, (at(x + step, 0.0) - d) / step * 0.01, (at(x, step) - d) / step * 0.01)
        };
        let (before, after) = (-1e-3, 1e-3);
        let (a, b) = (uv_of(at(before, 0.0)), uv_of(at(after, 0.0)));
        assert!((a.x - b.x).abs() > 0.9, "the coordinate wraps: {a:?} {b:?}");
        let (ga, gb) = (grad(before), grad(after));
        assert!((ga.0 - gb.0).length() < 1e-4 && (ga.1 - gb.1).length() < 1e-4, "{ga:?} {gb:?}");
        assert!(ga.0.x.abs() > 0.0);
    }

    #[test]
    fn gradients_stay_finite_and_bounded_at_the_poles() {
        for z in [1.0f32, -1.0, 0.999_999] {
            let d = Vec3::new(1e-4, 0.0, z).normalize();
            let (gx, gy) = uv_grad(d, Vec3::new(0.01, 0.0, 0.0), Vec3::new(0.0, 0.01, 0.0));
            for g in [gx, gy] {
                assert!(g.x.is_finite() && g.y.is_finite());
                assert!(g.x.abs() <= 0.5);
            }
        }
    }

    #[test]
    fn the_uniform_has_the_shader_layout() {
        assert_eq!(std::mem::size_of::<Uniform>(), 48);
        assert_eq!(mip_count(2048, 1024), 12);
        assert_eq!(mip_count(1, 1), 1);
    }

    #[test]
    fn panorama_shader_is_valid_for_vulkan_and_metal() {
        let source = include_str!("shaders/panorama.wgsl");
        let module = naga::front::wgsl::parse_str(source).unwrap_or_else(|e| panic!("{}", e.emit_to_string(source)));
        let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::empty())
            .validate(&module)
            .unwrap_or_else(|e| panic!("{e:?}"));
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
    }

    /// Renders `image` seen from `params` into a `size` target and returns
    /// its pixels (RGBA8, rows top to bottom); None without a GPU.
    fn render_test(image: &Image, params: PanoParams, size: [u32; 2]) -> Option<Vec<u8>> {
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
        let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).ok()?;
        let pipelines = Pipelines::new(&device);
        let mut encoder = device.create_command_encoder(&Default::default());
        let source = Source::new(&device, &queue, &mut encoder, &pipelines, image).expect("a source");
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("panorama test"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("panorama test"),
            size: std::mem::size_of::<Uniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        draw(
            &device,
            &queue,
            &mut encoder,
            &pipelines,
            Draw {
                uniform: &uniform,
                source: Some(&source),
                target: &target.create_view(&Default::default()),
                size,
                params,
            },
        );
        let row = (size[0] * 4).div_ceil(256) * 256;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("panorama test"),
            size: crate::readback::readback_size(u64::from(row) * u64::from(size[1])),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(size[1]),
                },
            },
            wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
        );
        queue.submit([encoder.finish()]);
        readback.map_async(wgpu::MapMode::Read, .., |r| r.expect("readback"));
        device.poll(wgpu::PollType::wait_indefinitely()).expect("GPU completion");
        let view = readback.get_mapped_range(..).expect("pixels");
        let mut out = Vec::new();
        for y in 0..size[1] as usize {
            out.extend_from_slice(&view[y * row as usize..y * row as usize + size[0] as usize * 4]);
        }
        Some(out)
    }

    /// A smooth periodic image (red = 0.5 + 0.5 cos(2 PI u)) viewed across
    /// its wrap: no column of the seam may fall back to a coarse mip (which
    /// would average the whole image to 0.5), and the picture follows the
    /// analytic value everywhere.
    #[test]
    fn the_wrap_leaves_no_seam() {
        let _gpu = crate::test_gpu::lock();
        let (w, h) = (512u32, 256u32);
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for _y in 0..h {
            for x in 0..w {
                let u = (x as f32 + 0.5) / w as f32;
                rgba.extend_from_slice(&[((0.5 + 0.5 * (TAU * u).cos()) * 255.0).round() as u8, 40, 40, 255]);
            }
        }
        let image = Image {
            size: [w, h],
            rgba: rgba.into(),
        };
        let params = PanoParams {
            yaw: PI,
            pitch: 0.0,
            fov_y: 1.0,
        };
        let size = [128u32, 64];
        let Some(pixels) = render_test(&image, params, size) else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let y = size[1] / 2;
        let aspect = size[0] as f32 / size[1] as f32;
        let mut worst: f32 = 0.0;
        for x in 0..size[0] {
            let ndc = Vec2::new((x as f32 + 0.5) / size[0] as f32 * 2.0 - 1.0, 0.0);
            let uv = uv_of(ray(params, aspect, ndc));
            let expected = 0.5 + 0.5 * (TAU * uv.x).cos();
            let got = f32::from(pixels[((y * size[0] + x) * 4) as usize]) / 255.0;
            worst = worst.max((got - expected).abs());
        }
        assert!(worst < 0.04, "worst error {worst} along the horizon");
    }

    /// Each level of the chain is the mean of the four texels above it: a
    /// minified checkerboard turns grey.
    #[test]
    fn the_mip_chain_averages_the_levels() {
        let _gpu = crate::test_gpu::lock();
        let (w, h) = (64u32, 32u32);
        let mut rgba = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let v = if (x + y) % 2 == 0 { 255 } else { 0 };
                rgba.extend_from_slice(&[v, v, v, 255]);
            }
        }
        let image = Image {
            size: [w, h],
            rgba: rgba.into(),
        };
        let params = PanoParams {
            yaw: 0.0,
            pitch: 0.0,
            fov_y: 1.5,
        };
        let Some(pixels) = render_test(&image, params, [16, 8]) else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let centre = pixels[((4 * 16 + 8) * 4) as usize];
        assert!(
            (i32::from(centre) - 128).abs() < 40,
            "a minified checkerboard is grey, got {centre}"
        );
    }
}
