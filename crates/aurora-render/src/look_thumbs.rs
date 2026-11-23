//! The photo filters' catalogue pictures (Aurora's own; the Legacy Viewer's
//! filter list has none): a small still of the scene, taken now and then
//! from the frame before its looks (the photo's part of it, with its depth
//! for the depth looks), and the catalogue's looks drawn on it one a frame
//! with the photo's own passes (`looks`), each into a picture the interface
//! shows as it is: the catalogue paints a placeholder for a picture's slot
//! (`paint`), which the render thread swaps for its texture (`resolve`, as
//! the previews do), so nothing is read back nor waited for.
//!
//! The still is the input image of a `Looks` of its own, at the pictures'
//! size: the looks' passes only read their input, so it stays as it was
//! taken until the next one, for every look drawn on it in between.

use crate::looks::{LookDepth, Looks, PhotoLook};
use crate::pipeline_cache::CachedPipelines;

/// The pictures' size (4:3, the catalogue's tiles).
pub const THUMB_SIZE: (u32, u32) = (192, 144);
/// Pictures kept at most: every built-in look (30 + 36 and the creators'
/// tables) and a few of the user's; past that the viewer reuses the slot of
/// a picture not shown.
pub const MAX_THUMBS: usize = 96;
/// Frames without a request before the pictures are let go.
pub(crate) const KEEP_FRAMES: u32 = 3600;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// What the frame does for the catalogue's pictures (`FrameParams::look_thumbs`).
#[derive(Debug, Clone, Default)]
pub struct ThumbRequest {
    /// Take a new still from this part of the frame (0..1 of its width and
    /// height: the photo's frame, cropped to the pictures' proportions).
    pub capture: Option<[f32; 4]>,
    /// Draw this look on the still into picture `slot`.
    pub draw: Option<(usize, PhotoLook)>,
}

pub(crate) struct LookThumbs {
    pub(crate) format: wgpu::TextureFormat,
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    uniform: wgpu::Buffer,
    looks: Looks,
    depth: wgpu::TextureView,
    slots: Vec<(wgpu::TextureView, egui::TextureId)>,
    /// The projection of the frame the still was taken from (its depth's).
    proj: Option<glam::Mat4>,
    /// Frames since the last request.
    pub(crate) idle: u32,
}

impl LookThumbs {
    pub(crate) fn new(device: &wgpu::Device, egui: &mut egui_wgpu::Renderer, format: wgpu::TextureFormat) -> LookThumbs {
        let fragment = wgpu::ShaderStages::FRAGMENT;
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("look thumbnails"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: fragment,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: fragment,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: fragment,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: fragment,
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
            label: Some("look thumbnails"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/look_thumbs.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("look thumbnails"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline_cached(&wgpu::RenderPipelineDescriptor {
            label: Some("look thumbnails still"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("look thumbnails"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("look thumbnails source"),
            size: 32,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut looks = Looks::new(device);
        looks.prepare(device, format, THUMB_SIZE);
        let size = wgpu::Extent3d {
            width: THUMB_SIZE.0,
            height: THUMB_SIZE.1,
            depth_or_array_layers: 1,
        };
        let texture = |label: &str, format: wgpu::TextureFormat| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let depth = texture("look thumbnails depth", DEPTH);
        let slots = (0..MAX_THUMBS)
            .map(|_| {
                let view = texture("look thumbnail", format);
                let id = egui.register_native_texture(device, &view, wgpu::FilterMode::Linear);
                (view, id)
            })
            .collect();
        LookThumbs {
            format,
            layout,
            pipeline,
            sampler,
            uniform,
            looks,
            depth,
            slots,
            proj: None,
            idle: 0,
        }
    }

    /// The pictures let go (their egui textures freed).
    pub(crate) fn free(self, egui: &mut egui_wgpu::Renderer) {
        for (_, id) in &self.slots {
            egui.free_texture(id);
        }
    }

    /// A new still from `rect` (0..1) of the frame's image before its looks
    /// and of its depth, the frame drawn with `proj`.
    pub(crate) fn capture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        scene: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        rect: [f32; 4],
        proj: glam::Mat4,
    ) {
        let Some(still) = self.looks.input() else {
            return;
        };
        let rect = rect.map(|v| v.clamp(0.0, 1.0));
        let params: [f32; 8] = [
            rect[0],
            rect[1],
            rect[2],
            rect[3],
            THUMB_SIZE.0 as f32,
            THUMB_SIZE.1 as f32,
            0.0,
            0.0,
        ];
        queue.write_buffer(&self.uniform, 0, bytemuck::cast_slice(&params));
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("look thumbnails still"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(scene),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(depth),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.uniform.as_entire_binding(),
                },
            ],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("look thumbnails still"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: still,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.draw(0..3, 0..1);
        self.proj = Some(proj);
    }

    /// `look` drawn on the still into picture `slot`; the passes drawn (none
    /// before the first still).
    pub(crate) fn draw(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        slot: usize,
        look: &PhotoLook,
    ) -> u32 {
        let (Some(proj), Some((target, _))) = (self.proj, self.slots.get(slot)) else {
            return 0;
        };
        let depth = LookDepth { view: &self.depth, proj };
        self.looks.encode(device, queue, encoder, look, target, Some(depth))
    }
}

/// A picture's place in the interface, swapped for its texture on the
/// render thread (`resolve`).
pub struct ThumbRegion {
    slot: usize,
    tint: egui::Color32,
    radius: f32,
}

/// Paints the picture of `slot` in `rect`, tinted, its corners rounded by
/// `radius` (nothing until the renderer has drawn it: what is under it
/// shows).
pub fn paint(painter: &egui::Painter, rect: egui::Rect, slot: usize, tint: egui::Color32, radius: f32) {
    painter.add(egui::Shape::Callback(egui::PaintCallback {
        rect,
        callback: std::sync::Arc::new(ThumbRegion { slot, tint, radius }),
    }));
}

/// A picture's mesh: `rect` with its corners rounded by `radius`, the
/// texture stretched over it whole (a fan from the middle).
fn rounded(rect: egui::Rect, radius: f32, texture: egui::TextureId, tint: egui::Color32) -> egui::Mesh {
    const STEPS: usize = 6;
    let r = radius.clamp(0.0, rect.width().min(rect.height()) / 2.0);
    let uv = |at: egui::Pos2| egui::pos2((at.x - rect.left()) / rect.width(), (at.y - rect.top()) / rect.height());
    let mut mesh = egui::Mesh::with_texture(texture);
    let vertex = |at: egui::Pos2| egui::epaint::Vertex {
        pos: at,
        uv: uv(at),
        color: tint,
    };
    mesh.vertices.push(vertex(rect.center()));
    // the corners' arcs, clockwise from the top left
    let corners = [
        (egui::pos2(rect.left() + r, rect.top() + r), std::f32::consts::PI),
        (egui::pos2(rect.right() - r, rect.top() + r), 1.5 * std::f32::consts::PI),
        (egui::pos2(rect.right() - r, rect.bottom() - r), 0.0),
        (egui::pos2(rect.left() + r, rect.bottom() - r), 0.5 * std::f32::consts::PI),
    ];
    for (centre, start) in corners {
        for k in 0..=STEPS {
            let a = start + k as f32 / STEPS as f32 * std::f32::consts::FRAC_PI_2;
            mesh.vertices.push(vertex(centre + r * egui::vec2(a.cos(), a.sin())));
        }
    }
    let ring = (mesh.vertices.len() - 1) as u32;
    for i in 0..ring {
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % ring);
    }
    mesh
}

/// Whether a paint callback is one of the catalogue's pictures.
pub(crate) fn is(callback: &egui::PaintCallback) -> bool {
    callback.callback.downcast_ref::<ThumbRegion>().is_some()
}

/// The pictures painted by the interface swapped for their textures (none
/// while the renderer has no pictures: nothing is drawn there).
pub(crate) fn resolve(thumbs: Option<&LookThumbs>, primitives: &mut [egui::ClippedPrimitive]) {
    for primitive in primitives {
        let egui::epaint::Primitive::Callback(callback) = &primitive.primitive else {
            continue;
        };
        let Some(region) = callback.callback.downcast_ref::<ThumbRegion>() else {
            continue;
        };
        let rect = callback.rect;
        let texture = thumbs.and_then(|t| t.slots.get(region.slot)).map(|(_, id)| *id);
        match texture.filter(|_| primitive.clip_rect.intersects(rect)) {
            Some(texture) => {
                primitive.primitive = egui::epaint::Primitive::Mesh(rounded(rect, region.radius, texture, region.tint));
            }
            None => {
                // (an empty mesh needs an empty clip too: see preview.rs)
                primitive.primitive = egui::epaint::Primitive::Mesh(egui::Mesh::default());
                primitive.clip_rect = egui::Rect::NOTHING;
            }
        }
    }
}

/// The part of the frame a still is taken from: the photo's frame (`frame`,
/// 0..1 of the window: left, top, right, bottom) cropped around its middle
/// to the pictures' proportions, for a window of `window` pixels.
pub fn still_rect(frame: [f32; 4], window: (u32, u32)) -> [f32; 4] {
    let (w, h) = (window.0.max(1) as f32, window.1.max(1) as f32);
    let width = (frame[2] - frame[0]) * w;
    let height = (frame[3] - frame[1]) * h;
    if width <= 0.0 || height <= 0.0 {
        return frame;
    }
    let want = THUMB_SIZE.0 as f32 / THUMB_SIZE.1 as f32;
    let (cw, ch) = if width / height > want {
        (height * want, height)
    } else {
        (width, width / want)
    };
    let (cx, cy) = ((frame[0] + frame[2]) / 2.0 * w, (frame[1] + frame[3]) / 2.0 * h);
    [(cx - cw / 2.0) / w, (cy - ch / 2.0) / h, (cx + cw / 2.0) / w, (cy + ch / 2.0) / h]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shader_is_valid() {
        let module = naga::front::wgsl::parse_str(include_str!("shaders/look_thumbs.wgsl")).expect("parses");
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
            .validate(&module)
            .expect("valid");
    }

    /// The rounded picture covers its rect with the texture whole over it,
    /// and stays inside it (the corners cut).
    #[test]
    fn a_rounded_picture_fills_its_rect_and_cuts_its_corners() {
        let rect = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(96.0, 72.0));
        let mesh = rounded(rect, 5.0, egui::TextureId::User(1), egui::Color32::WHITE);
        assert!(mesh.is_valid());
        assert!(mesh.vertices.iter().all(|v| rect.expand(0.001).contains(v.pos)));
        assert!(!mesh.vertices.iter().any(|v| v.pos == rect.min), "the corner is cut");
        for v in &mesh.vertices {
            assert!((v.uv.x * 96.0 + 10.0 - v.pos.x).abs() < 0.01);
            assert!((v.uv.y * 72.0 + 20.0 - v.pos.y).abs() < 0.01);
        }
        // no radius: the corners themselves
        let square = rounded(rect, 0.0, egui::TextureId::User(1), egui::Color32::WHITE);
        assert!(square.vertices.iter().any(|v| (v.pos - rect.min).length() < 0.001));
    }

    /// A wide photo gives its middle at 4:3; a tall one, its middle band.
    #[test]
    fn the_still_is_the_middle_of_the_photo_at_the_pictures_proportions() {
        let close = |a: [f32; 4], b: [f32; 4]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-4);
        // a 1600 x 900 window, the photo filling it: a 1200 x 900 middle
        assert!(close(still_rect([0.0, 0.0, 1.0, 1.0], (1600, 900)), [0.125, 0.0, 0.875, 1.0]));
        // a square photo in the middle of the same window: 900 x 675
        let square = [350.0 / 1600.0, 0.0, 1250.0 / 1600.0, 1.0];
        let r = still_rect(square, (1600, 900));
        assert!(close(r, [350.0 / 1600.0, 112.5 / 900.0, 1250.0 / 1600.0, 787.5 / 900.0]), "{r:?}");
        assert_eq!(
            still_rect([0.5, 0.5, 0.5, 0.5], (100, 100)),
            [0.5, 0.5, 0.5, 0.5],
            "an empty frame as it is"
        );
    }
}
