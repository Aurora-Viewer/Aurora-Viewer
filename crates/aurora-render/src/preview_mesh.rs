//! Cached GPU antialiasing for small, prelit 3D thumbnails. Render above
//! display resolution with MSAA, then box-filter premultiplied coverage.
//! Targets travel through the normal frame, without readback or GPU waits.

use crate::pipeline_cache::CachedPipelines;
use bytemuck::{Pod, Zeroable};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Weak};

const MSAA: u32 = 4;
const MAX_SIDE: f32 = 512.0;

struct Region {
    id: egui::Id,
    mesh: Arc<egui::Mesh>,
}

/// Paint an untextured, prelit mesh with the same gamma/premultiplied colors
/// as egui. The caller retains its response; this adds no input handling.
pub fn paint(ui: &egui::Ui, id: egui::Id, rect: egui::Rect, mesh: Arc<egui::Mesh>) {
    if ui.is_rect_visible(rect) && rect.is_positive() && !mesh.indices.is_empty() {
        ui.painter().add(egui::PaintCallback {
            rect,
            callback: Arc::new(Region { id, mesh }),
        });
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    position: [f32; 2],
    color: [u8; 4],
}

struct Pipelines {
    mesh: wgpu::RenderPipeline,
    resolve: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
}

impl Pipelines {
    fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Antialiased thumbnail"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/preview_mesh.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Thumbnail downsample"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let mesh_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Thumbnail mesh"),
            bind_group_layouts: &[],
            immediate_size: 0,
        });
        let resolve_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Thumbnail downsample"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |label, layout: &wgpu::PipelineLayout, vertex, fragment, buffers: &[_], samples, blend| {
            device.create_render_pipeline_cached(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vertex),
                    compilation_options: Default::default(),
                    buffers,
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fragment),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState {
                    count: samples,
                    ..Default::default()
                },
                multiview_mask: None,
                cache: None,
            })
        };
        let attributes = wgpu::vertex_attr_array![0 => Float32x2, 1 => Unorm8x4];
        let mesh_buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &attributes,
        })];
        let mesh = pipeline(
            "Thumbnail mesh",
            &mesh_layout,
            "vs_mesh",
            "fs_mesh",
            &mesh_buffers,
            MSAA,
            Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
        );
        let resolve = pipeline("Thumbnail downsample", &resolve_layout, "vs_fullscreen", "fs_resolve", &[], 1, None);
        Self { mesh, resolve, layout }
    }
}

fn target_size(size: egui::Vec2, scale: f32) -> ([u32; 2], u32) {
    let pixels = size * scale;
    let fit = (MAX_SIDE / pixels.max_elem()).min(1.0);
    let size = [
        (pixels.x * fit).ceil().clamp(1.0, MAX_SIDE) as u32,
        (pixels.y * fit).ceil().clamp(1.0, MAX_SIDE) as u32,
    ];
    // Small silhouettes get 64 coverage samples per display pixel. Larger
    // HiDPI tiles need less enlargement, bounding transient GPU memory.
    let factor = if size[0].max(size[1]) <= 128 { 4 } else { 2 };
    (size, factor)
}

struct Target {
    size: [u32; 2],
    source: Weak<egui::Mesh>,
    texture: egui::TextureId,
    #[cfg(test)]
    color: wgpu::Texture,
}

struct Scratch {
    msaa: wgpu::TextureView,
    high: wgpu::TextureView,
}

impl Scratch {
    fn new(device: &wgpu::Device, size: [u32; 2]) -> Self {
        let view = |label, samples, usage| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: size[0],
                        height: size[1],
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: samples,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        Self {
            msaa: view("Thumbnail MSAA", MSAA, wgpu::TextureUsages::RENDER_ATTACHMENT),
            high: view(
                "Thumbnail supersample",
                1,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            ),
        }
    }
}

impl Target {
    fn new(
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        egui: &mut egui_wgpu::Renderer,
        pipeline: &Pipelines,
        scratch: &Scratch,
        rect: egui::Rect,
        mesh: &Arc<egui::Mesh>,
        size: [u32; 2],
        factor: u32,
    ) -> Self {
        let texture = |label, size: [u32; 2], samples, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage,
                view_formats: &[],
            })
        };
        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        #[cfg(test)]
        let usage = usage | wgpu::TextureUsages::COPY_SRC;
        let color = texture("Thumbnail display", size, 1, usage);
        let color_view = color.create_view(&Default::default());
        let vertices: Vec<Vertex> = mesh
            .vertices
            .iter()
            .map(|v| Vertex {
                position: [
                    (v.pos.x - rect.left()) / rect.width() * 2.0 - 1.0,
                    1.0 - (v.pos.y - rect.top()) / rect.height() * 2.0,
                ],
                color: v.color.to_array(),
            })
            .collect();
        let buffer = |label, bytes: &[u8], usage| crate::gpu_alloc::buffer_init(device, label, bytes, usage);
        let vertices = buffer("Thumbnail vertices", bytemuck::cast_slice(&vertices), wgpu::BufferUsages::VERTEX);
        let indices = buffer("Thumbnail indices", bytemuck::cast_slice(&mesh.indices), wgpu::BufferUsages::INDEX);
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Thumbnail MSAA"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &scratch.msaa,
                    depth_slice: None,
                    resolve_target: Some(&scratch.high),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&pipeline.mesh);
            pass.set_vertex_buffer(0, vertices.slice(..));
            pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..mesh.indices.len() as u32, 0, 0..1);
        }
        let options = buffer(
            "Thumbnail filter",
            bytemuck::cast_slice(&[factor, 0u32, 0, 0]),
            wgpu::BufferUsages::UNIFORM,
        );
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Thumbnail filter"),
            layout: &pipeline.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&scratch.high),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: options.as_entire_binding(),
                },
            ],
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Thumbnail downsample"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&pipeline.resolve);
            pass.set_bind_group(0, &group, &[]);
            pass.draw(0..3, 0..1);
        }
        // Only this small, display-sized texture stays cached. MSAA and
        // supersample attachments retire after this frame's GPU work.
        let texture = egui.register_native_texture(device, &color_view, wgpu::FilterMode::Linear);
        Self {
            size,
            source: Arc::downgrade(mesh),
            texture,
            #[cfg(test)]
            color,
        }
    }
}

#[derive(Default)]
pub(crate) struct Cache {
    pipeline: Option<Pipelines>,
    targets: HashMap<egui::Id, Target>,
}

impl Cache {
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        egui: &mut egui_wgpu::Renderer,
        primitives: &mut [egui::ClippedPrimitive],
        scale: f32,
    ) {
        let mut seen = HashSet::new();
        // Reuse scratch attachments across every equal-sized tile in this
        // frame. Each downsample finishes before the next tile overwrites it.
        let mut scratch = HashMap::new();
        for primitive in primitives {
            let egui::epaint::Primitive::Callback(callback) = &primitive.primitive else {
                continue;
            };
            let Some(region) = callback.callback.downcast_ref::<Region>() else {
                continue;
            };
            let rect = callback.rect;
            if !primitive.clip_rect.intersects(rect) {
                primitive.primitive = egui::epaint::Primitive::Mesh(egui::Mesh::default());
                primitive.clip_rect = egui::Rect::NOTHING;
                continue;
            }
            let (size, factor) = target_size(rect.size(), scale);
            let changed = self
                .targets
                .get(&region.id)
                .is_none_or(|t| t.size != size || !t.source.ptr_eq(&Arc::downgrade(&region.mesh)));
            if changed {
                if let Some(old) = self.targets.remove(&region.id) {
                    egui.free_texture(&old.texture);
                }
                let pipeline = self.pipeline.get_or_insert_with(|| Pipelines::new(device));
                let scratch = scratch
                    .entry(size.map(|v| v * factor))
                    .or_insert_with_key(|size| Scratch::new(device, *size));
                self.targets.insert(
                    region.id,
                    Target::new(device, encoder, egui, pipeline, scratch, rect, &region.mesh, size, factor),
                );
            }
            if let Some(target) = self.targets.get(&region.id) {
                seen.insert(region.id);
                let mut mesh = egui::Mesh::with_texture(target.texture);
                mesh.add_rect_with_uv(
                    rect,
                    egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
                primitive.primitive = egui::epaint::Primitive::Mesh(mesh);
            }
        }
        self.targets.retain(|id, target| {
            if seen.contains(id) {
                true
            } else {
                egui.free_texture(&target.texture);
                false
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumbnail_targets_follow_display_scale_and_bound_oversampling() {
        assert_eq!(target_size(egui::vec2(48.0, 32.0), 1.0), ([48, 32], 4));
        assert_eq!(target_size(egui::vec2(48.0, 32.0), 2.0), ([96, 64], 4));
        assert_eq!(target_size(egui::vec2(48.0, 32.0), 3.0), ([144, 96], 2));
        assert_eq!(target_size(egui::vec2(600.0, 300.0), 2.0), ([512, 256], 2));
        assert_eq!(target_size(egui::Vec2::ZERO, 1.0), ([1, 1], 4));
    }

    #[test]
    fn gpu_thumbnail_edges_have_smooth_coverage_without_dark_fringes_at_each_display_scale() {
        let _gpu = crate::test_gpu::lock();
        let instance = wgpu::Instance::default();
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
            return;
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).expect("test GPU");
        let mut egui = egui_wgpu::Renderer::new(&device, wgpu::TextureFormat::Rgba8Unorm, Default::default());
        let origin = egui::pos2(10.35, 12.9);
        let rect = egui::Rect::from_min_size(origin, egui::Vec2::splat(64.0));
        let color = egui::Color32::from_rgb(200, 125, 70);
        let mut mesh = egui::Mesh::default();
        for point in [egui::vec2(5.25, 5.6), egui::vec2(57.3, 12.8), egui::vec2(13.75, 57.1)] {
            mesh.colored_vertex(origin + point, color);
        }
        mesh.add_triangle(0, 1, 2);
        let mesh = Arc::new(mesh);
        let mut other = (*mesh).clone();
        for vertex in &mut other.vertices {
            vertex.color = egui::Color32::from_rgb(30, 80, 190);
        }
        let other = Arc::new(other);
        let id = egui::Id::unique("antialiased thumbnail regression");
        let other_id = id.with("shared scratch, different material");
        let mut cache = Cache::default();
        for scale in [1.0, 1.5, 3.0] {
            let primitives = || {
                vec![
                    egui::ClippedPrimitive {
                        clip_rect: rect,
                        primitive: egui::epaint::Primitive::Callback(egui::PaintCallback {
                            rect,
                            callback: Arc::new(Region { id, mesh: mesh.clone() }),
                        }),
                    },
                    egui::ClippedPrimitive {
                        clip_rect: rect,
                        primitive: egui::epaint::Primitive::Callback(egui::PaintCallback {
                            rect,
                            callback: Arc::new(Region {
                                id: other_id,
                                mesh: other.clone(),
                            }),
                        }),
                    },
                ]
            };
            let mut encoder = device.create_command_encoder(&Default::default());
            cache.render(&device, &mut encoder, &mut egui, &mut primitives(), scale);
            let target = &cache.targets[&id];
            let texture = target.texture;
            let [width, height] = target.size;
            let stride = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
            let pixels = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Antialiased thumbnail pixels"),
                size: crate::readback::readback_size(u64::from(stride * height)),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            encoder.copy_texture_to_buffer(
                target.color.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &pixels,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(stride),
                        rows_per_image: Some(height),
                    },
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
            queue.submit([encoder.finish()]);
            pixels.map_async(wgpu::MapMode::Read, .., |result| result.expect("thumbnail readback"));
            device.poll(wgpu::PollType::wait_indefinitely()).expect("test GPU completion");
            let bytes = pixels.get_mapped_range(..).expect("thumbnail pixels");
            let mut coverage = std::collections::BTreeSet::new();
            let mut opaque = 0;
            for y in 0..height {
                for x in 0..width {
                    let at = (y * stride + x * 4) as usize;
                    let pixel = &bytes[at..at + 4];
                    match pixel[3] {
                        0 => assert_eq!(&pixel[..3], &[0; 3]),
                        255 => {
                            opaque += 1;
                            assert_eq!(&pixel[..3], &color.to_array()[..3]);
                        }
                        alpha => {
                            coverage.insert(alpha);
                            for (value, base) in pixel[..3].iter().zip(color.to_array()) {
                                let error = (i32::from(*value) * 255 - i32::from(base) * i32::from(alpha)).abs();
                                assert!(error <= 510, "premultiplied edge at scale {scale}: {pixel:?}");
                            }
                        }
                    }
                }
            }
            assert!(opaque > 100, "solid interior at scale {scale}");
            assert!(coverage.len() > 4, "supersampled edge at scale {scale}: {coverage:?}");
            drop(bytes);
            pixels.unmap();
            let mut encoder = device.create_command_encoder(&Default::default());
            cache.render(&device, &mut encoder, &mut egui, &mut primitives(), scale);
            assert_eq!(cache.targets[&id].texture, texture, "unchanged thumbnail reuses its GPU image");
        }
        let mut encoder = device.create_command_encoder(&Default::default());
        cache.render(&device, &mut encoder, &mut egui, &mut [], 1.0);
        assert!(cache.targets.is_empty(), "closing the palette frees its GPU images");
    }
}
