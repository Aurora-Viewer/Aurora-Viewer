//! GPU previews embedded in egui. Requests travel with the ordinary frame;
//! only the render thread uploads, draws and registers their textures. The
//! UI gets an image in the same frame without a readback or a GPU wait.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, Default)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub joints: [u32; 4],
    pub weights: [f32; 4],
}
impl Vertex {
    pub fn new(position: [f32; 3], normal: [f32; 3], uv: [f32; 2]) -> Self {
        Self {
            position,
            normal,
            uv,
            ..Default::default()
        }
    }
}
pub struct Skin {
    pub joints: Vec<usize>,
    pub inverse_bind: Vec<Mat4>,
    pub rest: Vec<Mat4>,
}
pub type Pose = Arc<[[[f32; 4]; 4]]>;
#[derive(Clone)]
pub struct Image {
    pub size: [u32; 2],
    pub rgba: Arc<[u8]>,
}
#[derive(Clone, Copy, PartialEq)]
pub struct Material {
    pub color: [f32; 4],
    pub emissive: [f32; 3],
    pub metallic: f32,
    pub roughness: f32,
    pub alpha_cutoff: f32,
    pub alpha_mode: u32,
    pub normal_scale: f32,
    pub double_sided: bool,
}
impl Default for Material {
    fn default() -> Self {
        Self {
            color: [1.0; 4],
            emissive: [0.0; 3],
            metallic: 0.0,
            roughness: 0.6,
            alpha_cutoff: 0.5,
            alpha_mode: 0,
            normal_scale: 1.0,
            double_sided: true,
        }
    }
}
#[derive(Clone)]
pub struct Part {
    pub vertices: Arc<[Vertex]>,
    pub indices: Arc<[u32]>,
    pub textures: [Option<Arc<Image>>; 4],
    pub material: Material,
    pub skin: Option<Arc<Skin>>,
}
pub struct Model {
    pub parts: Vec<Part>,
    pub center: Vec3,
    pub radius: f32,
}
#[derive(Clone, Copy, PartialEq)]
pub struct View {
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
    pub pan: [f32; 2],
    pub background: [f32; 4],
    pub edge_color: [f32; 4],
    pub ambient: [f32; 3],
    pub edges: bool,
    pub uv: bool,
    pub material: Option<Material>,
}
impl Default for View {
    fn default() -> Self {
        Self {
            yaw: 0.5,
            pitch: 0.2,
            distance: 3.0,
            pan: [0.0; 2],
            background: [0.0, 0.0, 0.0, 1.0],
            edge_color: [1.0; 4],
            ambient: [0.2; 3],
            edges: false,
            uv: false,
            material: None,
        }
    }
}
struct Region {
    id: egui::Id,
    model: Arc<Model>,
    view: View,
    pose: Option<Pose>,
}
pub fn paint(ui: &mut egui::Ui, id: egui::Id, model: Arc<Model>, view: &mut View, size: egui::Vec2) -> egui::Response {
    paint_with_pose(ui, id, model, view, size, None)
}
pub fn paint_with_pose(
    ui: &mut egui::Ui,
    id: egui::Id,
    model: Arc<Model>,
    view: &mut View,
    size: egui::Vec2,
    pose: Option<Pose>,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::drag());
    if response.dragged_by(egui::PointerButton::Primary) {
        let delta = ui.input(|i| i.pointer.delta());
        if ui.input(|i| i.modifiers.shift) {
            view.pan[0] += delta.x / rect.width();
            view.pan[1] += delta.y / rect.height();
        } else {
            view.yaw -= delta.x * 0.01;
            view.pitch = (view.pitch + delta.y * 0.01).clamp(-1.5, 1.5);
        }
    }
    if response.hovered() {
        let wheel = ui.input(|i| i.smooth_scroll_delta.y);
        if wheel != 0.0 {
            view.distance = (view.distance * (-wheel * 0.002).exp()).clamp(0.2, 20.0);
        }
    }
    if response.double_clicked() {
        view.pan = [0.0; 2];
        view.distance = 3.0;
    }
    ui.painter().add(egui::Shape::Callback(egui::PaintCallback {
        rect,
        callback: Arc::new(Region {
            id,
            model,
            view: *view,
            pose,
        }),
    }));
    response
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniform {
    projection: [[f32; 4]; 4],
    eye: [f32; 4],
    color: [f32; 4],
    emissive_metallic: [f32; 4],
    params: [f32; 4],
    edge_color: [f32; 4],
    ambient: [f32; 4],
    surface: [f32; 4],
}
struct GpuPart {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    edges: wgpu::Buffer,
    count: u32,
    edge_count: u32,
    uniform: wgpu::Buffer,
    bones: wgpu::Buffer,
    source: usize,
    group: wgpu::BindGroup,
}
struct Pipelines {
    layout: wgpu::BindGroupLayout,
    triangles: wgpu::RenderPipeline,
    edges: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
}
struct Target {
    model: Arc<Model>,
    parts: Vec<GpuPart>,
    size: [u32; 2],
    color: wgpu::TextureView,
    depth: wgpu::TextureView,
    texture: egui::TextureId,
    last: Option<View>,
    last_pose: Option<Pose>,
}
#[derive(Default)]
pub(crate) struct Cache {
    pipeline: Option<Pipelines>,
    targets: HashMap<egui::Id, Target>,
}

impl Pipelines {
    fn new(device: &wgpu::Device) -> Self {
        let mut entries = vec![wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }];
        for binding in 1..=4 {
            entries.push(wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            });
        }
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: 5,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        });
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: 6,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Preview material"),
            entries: &entries,
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Preview"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Asset preview"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/preview.wgsl").into()),
        });
        let pipeline = |topology, entry| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Asset preview"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<Vertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2, 3 => Uint32x4, 4 => Float32x4],
                    })],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology,
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(topology == wgpu::PrimitiveTopology::TriangleList),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let triangles = pipeline(wgpu::PrimitiveTopology::TriangleList, "fs");
        let edges = pipeline(wgpu::PrimitiveTopology::LineList, "fs_edge");
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Preview textures"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            layout,
            triangles,
            edges,
            sampler,
        }
    }
}

impl Target {
    fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        egui: &mut egui_wgpu::Renderer,
        pipeline: &Pipelines,
        model: Arc<Model>,
        size: [u32; 2],
    ) -> Self {
        let target = |label, format| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: size[0],
                        height: size[1],
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let color = target("Preview color", wgpu::TextureFormat::Rgba8Unorm);
        let depth = target("Preview depth", wgpu::TextureFormat::Depth32Float);
        let texture = egui.register_native_texture(device, &color, wgpu::FilterMode::Linear);
        let mut parts = Vec::new();
        let mut images = HashMap::<(usize, bool), wgpu::TextureView>::new();
        for (source, part) in model.parts.iter().enumerate() {
            if part.vertices.is_empty() || part.indices.is_empty() {
                continue;
            }
            let buffer = |label, bytes: &[u8], usage| {
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(label),
                    contents: bytes,
                    usage,
                })
            };
            let vertices = buffer("Preview vertices", bytemuck::cast_slice(&part.vertices), wgpu::BufferUsages::VERTEX);
            let indices = buffer("Preview indices", bytemuck::cast_slice(&part.indices), wgpu::BufferUsages::INDEX);
            let mut lines = Vec::with_capacity(part.indices.len() * 2);
            for triangle in part.indices.as_chunks::<3>().0 {
                lines.extend_from_slice(&[triangle[0], triangle[1], triangle[1], triangle[2], triangle[2], triangle[0]]);
            }
            let edges = buffer("Preview edges", bytemuck::cast_slice(&lines), wgpu::BufferUsages::INDEX);
            let uniform = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Preview parameters"),
                size: std::mem::size_of::<Uniform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let matrices = part
                .skin
                .as_ref()
                .map(|skin| skin.rest.iter().map(Mat4::to_cols_array_2d).collect::<Vec<_>>())
                .unwrap_or_else(|| vec![Mat4::IDENTITY.to_cols_array_2d()]);
            let bones = buffer(
                "Preview bones",
                bytemuck::cast_slice(&matrices),
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            );
            let views: Vec<_> = part
                .textures
                .iter()
                .enumerate()
                .map(|(slot, image)| {
                    let srgb = slot == 0 || slot == 3;
                    let key = (
                        image.as_ref().map_or(if slot == 1 { 1 } else { 2 }, |i| Arc::as_ptr(i) as usize),
                        srgb,
                    );
                    if let Some(view) = images.get(&key) {
                        return view.clone();
                    }
                    let defaults = [[255u8; 4], [128, 128, 255, 255], [255; 4], [255; 4]];
                    let (size, data) = image
                        .as_ref()
                        .map(|i| (i.size, i.rgba.as_ref()))
                        .unwrap_or(([1, 1], &defaults[slot]));
                    let texture = device.create_texture(&wgpu::TextureDescriptor {
                        label: Some("Preview map"),
                        size: wgpu::Extent3d {
                            width: size[0],
                            height: size[1],
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: if slot == 0 || slot == 3 {
                            wgpu::TextureFormat::Rgba8UnormSrgb
                        } else {
                            wgpu::TextureFormat::Rgba8Unorm
                        },
                        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                        view_formats: &[],
                    });
                    queue.write_texture(
                        texture.as_image_copy(),
                        data,
                        wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(size[0] * 4),
                            rows_per_image: Some(size[1]),
                        },
                        wgpu::Extent3d {
                            width: size[0],
                            height: size[1],
                            depth_or_array_layers: 1,
                        },
                    );
                    let view = texture.create_view(&Default::default());
                    images.insert(key, view.clone());
                    view
                })
                .collect();
            let mut entries = vec![wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }];
            for (index, view) in views.iter().enumerate() {
                entries.push(wgpu::BindGroupEntry {
                    binding: index as u32 + 1,
                    resource: wgpu::BindingResource::TextureView(view),
                });
            }
            entries.push(wgpu::BindGroupEntry {
                binding: 5,
                resource: wgpu::BindingResource::Sampler(&pipeline.sampler),
            });
            entries.push(wgpu::BindGroupEntry {
                binding: 6,
                resource: bones.as_entire_binding(),
            });
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Preview part"),
                layout: &pipeline.layout,
                entries: &entries,
            });
            parts.push(GpuPart {
                vertices,
                indices,
                edges,
                count: part.indices.len() as u32,
                edge_count: lines.len() as u32,
                uniform,
                bones,
                source,
                group,
            });
        }
        Self {
            model,
            parts,
            size,
            color,
            depth,
            texture,
            last: None,
            last_pose: None,
        }
    }
    fn draw(&mut self, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, pipeline: &Pipelines, view: View, pose: Option<&Pose>) {
        let same_pose = match (pose, self.last_pose.as_ref()) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        if self.last == Some(view) && same_pose {
            return;
        }
        self.last = Some(view);
        self.last_pose = pose.cloned();
        let radius = self.model.radius.max(0.001);
        let center = self.model.center + Vec3::new(view.pan[0] * radius, 0.0, -view.pan[1] * radius);
        let eye = center
            + Vec3::new(
                view.yaw.cos() * view.pitch.cos(),
                view.yaw.sin() * view.pitch.cos(),
                view.pitch.sin(),
            ) * radius
                * view.distance;
        let projection =
            glam::camera::rh::proj::directx::perspective(0.75, self.size[0] as f32 / self.size[1] as f32, radius * 0.01, radius * 100.0)
                * glam::camera::rh::view::look_at_mat4(eye, center, Vec3::Z);
        for part in &self.parts {
            let source = &self.model.parts[part.source];
            if let Some(skin) = &source.skin {
                let matrices = skin
                    .joints
                    .iter()
                    .enumerate()
                    .map(|(i, j)| {
                        pose.and_then(|p| p.get(*j))
                            .map(|m| (Mat4::from_cols_array_2d(m) * skin.inverse_bind[i]).to_cols_array_2d())
                            .unwrap_or_else(|| skin.rest[i].to_cols_array_2d())
                    })
                    .collect::<Vec<_>>();
                queue.write_buffer(&part.bones, 0, bytemuck::cast_slice(&matrices));
            }
            let material = view.material.unwrap_or(source.material);
            let uniforms = Uniform {
                projection: projection.to_cols_array_2d(),
                eye: [eye.x, eye.y, eye.z, 1.0],
                color: material.color,
                emissive_metallic: [material.emissive[0], material.emissive[1], material.emissive[2], material.metallic],
                params: [
                    material.roughness,
                    material.alpha_cutoff,
                    material.alpha_mode as f32,
                    if view.uv { 1.0 } else { 0.0 },
                ],
                edge_color: view.edge_color,
                ambient: [view.ambient[0], view.ambient[1], view.ambient[2], 1.0],
                surface: [material.normal_scale, if material.double_sided { 1.0 } else { 0.0 }, 0.0, 0.0],
            };
            queue.write_buffer(&part.uniform, 0, bytemuck::bytes_of(&uniforms));
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Asset preview"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.color,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: view.background[0] as f64,
                        g: view.background[1] as f64,
                        b: view.background[2] as f64,
                        a: 1.0,
                    }),
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
        pass.set_pipeline(&pipeline.triangles);
        for part in &self.parts {
            pass.set_bind_group(0, &part.group, &[]);
            pass.set_vertex_buffer(0, part.vertices.slice(..));
            pass.set_index_buffer(part.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..part.count, 0, 0..1);
        }
        if view.edges {
            pass.set_pipeline(&pipeline.edges);
            for part in &self.parts {
                pass.set_bind_group(0, &part.group, &[]);
                pass.set_vertex_buffer(0, part.vertices.slice(..));
                pass.set_index_buffer(part.edges.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..part.edge_count, 0, 0..1);
            }
        }
    }
}

impl Cache {
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        egui: &mut egui_wgpu::Renderer,
        primitives: &mut [egui::ClippedPrimitive],
        scale: f32,
    ) {
        let mut seen = HashSet::new();
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
                continue;
            }
            let size = [
                (rect.width() * scale).ceil().clamp(1.0, 1024.0) as u32,
                (rect.height() * scale).ceil().clamp(1.0, 1024.0) as u32,
            ];
            let pipeline = self.pipeline.get_or_insert_with(|| Pipelines::new(device));
            let changed = self
                .targets
                .get(&region.id)
                .is_none_or(|target| target.size != size || !Arc::ptr_eq(&target.model, &region.model));
            if changed {
                if let Some(old) = self.targets.remove(&region.id) {
                    egui.free_texture(&old.texture);
                }
                self.targets
                    .insert(region.id, Target::new(device, queue, egui, pipeline, region.model.clone(), size));
            }
            let target = self.targets.get_mut(&region.id).expect("preview target initialized");
            target.draw(queue, encoder, pipeline, region.view, region.pose.as_ref());
            seen.insert(region.id);
            let mut mesh = egui::Mesh::with_texture(target.texture);
            mesh.add_rect_with_uv(
                rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
            primitive.primitive = egui::epaint::Primitive::Mesh(mesh);
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
