//! GPU previews embedded in egui. Requests travel with the ordinary frame;
//! only the render thread uploads, draws and registers their textures. The
//! UI gets an image in the same frame without a readback or a GPU wait.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Weak},
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
    /// Affine UV rows per map, in the preview's top-left image space.
    pub uv: [[[f32; 4]; 2]; 4],
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
            uv: [[[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0]]; 4],
        }
    }
}
#[derive(Clone)]
pub struct Part {
    pub diagnostic: bool,
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
    pub diagnostic_colors: [[f32; 4]; 2],
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
            diagnostic_colors: [[1.0; 4]; 2],
            ambient: [0.2; 3],
            edges: false,
            uv: false,
            material: None,
        }
    }
}
impl View {
    /// Reset only the camera; keep display and material settings intact.
    pub fn reset_camera(&mut self) {
        let defaults = Self::default();
        self.yaw = defaults.yaw;
        self.pitch = defaults.pitch;
        self.distance = defaults.distance;
        self.pan = defaults.pan;
    }

    fn camera(&self, model: &Model, aspect: f32) -> (Vec3, Vec3) {
        let outward = Vec3::new(
            self.yaw.cos() * self.pitch.cos(),
            self.yaw.sin() * self.pitch.cos(),
            self.pitch.sin(),
        );
        let right = Vec3::Z.cross(outward).normalize_or_zero();
        let up = outward.cross(right);
        let height = 2.0 * model.radius.max(0.001) * self.distance * (0.75_f32 * 0.5).tan();
        // Pan in screen coordinates at any orbit/zoom, following the pointer.
        let center = model.center - right * self.pan[0] * height * aspect + up * self.pan[1] * height;
        (center + outward * model.radius.max(0.001) * self.distance, center)
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

/// Appearance hints are buttons in a scrolling list; they do not consume its
/// wheel or change the camera when the user clicks a minimum/maximum image.
pub fn paint_hint(ui: &mut egui::Ui, id: egui::Id, model: Arc<Model>, view: View, size: egui::Vec2) -> egui::Response {
    paint_tile(ui, id, model, view, size, egui::Sense::click())
}

/// A material swatch supplies inventory cargo rather than rotating its camera.
pub fn paint_swatch(ui: &mut egui::Ui, id: egui::Id, model: Arc<Model>, view: View, size: egui::Vec2) -> egui::Response {
    paint_tile(ui, id, model, view, size, egui::Sense::click_and_drag())
}

fn paint_tile(ui: &mut egui::Ui, id: egui::Id, model: Arc<Model>, view: View, size: egui::Vec2, sense: egui::Sense) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, sense);
    if ui.is_rect_visible(rect) {
        ui.painter().add(egui::Shape::Callback(egui::PaintCallback {
            rect,
            callback: Arc::new(Region {
                id,
                model,
                view,
                pose: None,
            }),
        }));
    }
    response
}
pub fn paint_with_pose(
    ui: &mut egui::Ui,
    id: egui::Id,
    model: Arc<Model>,
    view: &mut View,
    size: egui::Vec2,
    pose: Option<Pose>,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
    if response.dragged_by(egui::PointerButton::Primary) || response.dragged_by(egui::PointerButton::Middle) {
        let delta = ui.input(|i| i.pointer.delta());
        if response.dragged_by(egui::PointerButton::Middle) || ui.input(|i| i.modifiers.shift) {
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
            ui.input_mut(|i| i.smooth_scroll_delta = egui::Vec2::ZERO);
            view.distance = (view.distance * (-wheel * 0.002).exp()).clamp(0.2, 20.0);
        }
    }
    if response.double_clicked() {
        view.reset_camera();
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
    uv: [[[f32; 4]; 2]; 4],
}
/// The palette a part's skin shares with its other parts (0: no skin).
fn skin_key(skin: Option<&Arc<Skin>>) -> usize {
    skin.map_or(0, |skin| Arc::as_ptr(skin) as usize)
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
    blended: wgpu::RenderPipeline,
    edges: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    assets: SharedAssets,
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

type ImageMap = HashMap<(usize, bool), (Option<Weak<Image>>, wgpu::TextureView)>;
#[derive(Default)]
struct SharedAssets {
    images: ImageMap,
    vertices: HashMap<usize, (Weak<[Vertex]>, wgpu::Buffer)>,
    indices: HashMap<usize, (Weak<[u32]>, wgpu::Buffer, wgpu::Buffer)>,
}
impl SharedAssets {
    fn retain(&mut self) {
        self.images
            .retain(|_, (owner, _)| owner.as_ref().is_none_or(|owner| owner.strong_count() > 0));
        self.vertices.retain(|_, (owner, _)| owner.strong_count() > 0);
        self.indices.retain(|_, (owner, _, _)| owner.strong_count() > 0);
    }
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
        let pipeline = |topology, entry, depth_write| {
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
                    depth_write_enabled: Some(depth_write),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let triangles = pipeline(wgpu::PrimitiveTopology::TriangleList, "fs", true);
        let blended = pipeline(wgpu::PrimitiveTopology::TriangleList, "fs", false);
        let edges = pipeline(wgpu::PrimitiveTopology::LineList, "fs_edge", false);
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
            blended,
            edges,
            sampler,
            assets: SharedAssets::default(),
        }
    }
}

impl Target {
    fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        egui: &mut egui_wgpu::Renderer,
        pipeline: &mut Pipelines,
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
        let assets = &mut pipeline.assets;
        let mut parts = Vec::new();
        let mut bones = HashMap::new();
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
            let key = Arc::as_ptr(&part.vertices) as *const () as usize;
            let vertices = if let Some((owner, buffer)) = assets.vertices.get(&key)
                && owner.strong_count() > 0
            {
                buffer.clone()
            } else {
                let vertices = buffer("Preview vertices", bytemuck::cast_slice(&part.vertices), wgpu::BufferUsages::VERTEX);
                assets.vertices.insert(key, (Arc::downgrade(&part.vertices), vertices.clone()));
                vertices
            };
            let key = Arc::as_ptr(&part.indices) as *const () as usize;
            let (indices, edges) = if let Some((owner, indices, edges)) = assets.indices.get(&key)
                && owner.strong_count() > 0
            {
                (indices.clone(), edges.clone())
            } else {
                let indices = buffer("Preview indices", bytemuck::cast_slice(&part.indices), wgpu::BufferUsages::INDEX);
                let mut lines = Vec::with_capacity(part.indices.len() * 2);
                for triangle in part.indices.as_chunks::<3>().0 {
                    lines.extend_from_slice(&[triangle[0], triangle[1], triangle[1], triangle[2], triangle[2], triangle[0]]);
                }
                let edges = buffer("Preview edges", bytemuck::cast_slice(&lines), wgpu::BufferUsages::INDEX);
                assets
                    .indices
                    .insert(key, (Arc::downgrade(&part.indices), indices.clone(), edges.clone()));
                (indices, edges)
            };
            let uniform = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Preview parameters"),
                size: std::mem::size_of::<Uniform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            // The faces of a mesh, the parts of a body, share their skin: one
            // palette each, not one per part (an avatar's hint has hundreds).
            let bones = bones
                .entry(skin_key(part.skin.as_ref()))
                .or_insert_with(|| {
                    let matrices = part
                        .skin
                        .as_ref()
                        .map(|skin| skin.rest.iter().map(Mat4::to_cols_array_2d).collect::<Vec<_>>())
                        .unwrap_or_else(|| vec![Mat4::IDENTITY.to_cols_array_2d()]);
                    buffer(
                        "Preview bones",
                        bytemuck::cast_slice(&matrices),
                        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    )
                })
                .clone();
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
                    if let Some((owner, view)) = assets.images.get(&key)
                        && owner.as_ref().is_none_or(|owner| owner.strong_count() > 0)
                    {
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
                    assets.images.insert(key, (image.as_ref().map(Arc::downgrade), view.clone()));
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
                edge_count: (part.indices.len() / 3 * 6) as u32,
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
        let (eye, center) = view.camera(&self.model, self.size[0] as f32 / self.size[1] as f32);
        let projection =
            glam::camera::rh::proj::directx::perspective(0.75, self.size[0] as f32 / self.size[1] as f32, radius * 0.01, radius * 100.0)
                * glam::camera::rh::view::look_at_mat4(eye, center, Vec3::Z);
        let mut posed = HashSet::new();
        for part in &self.parts {
            let source = &self.model.parts[part.source];
            if let Some(skin) = &source.skin
                && posed.insert(skin_key(Some(skin)))
            {
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
            let material = effective_material(source, &view);
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
                edge_color: if source.diagnostic {
                    view.diagnostic_colors[1]
                } else {
                    view.edge_color
                },
                ambient: [view.ambient[0], view.ambient[1], view.ambient[2], 1.0],
                surface: [material.normal_scale, if material.double_sided { 1.0 } else { 0.0 }, 0.0, 0.0],
                uv: material.uv,
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
        // Like the world passes, skin/masked faces precede blended cosmetics.
        // A transparent layer must never write depth over the face behind it.
        for blended in [false, true] {
            pass.set_pipeline(if blended { &pipeline.blended } else { &pipeline.triangles });
            for part in &self.parts {
                if (effective_material(&self.model.parts[part.source], &view).alpha_mode == 1) != blended {
                    continue;
                }
                pass.set_bind_group(0, &part.group, &[]);
                pass.set_vertex_buffer(0, part.vertices.slice(..));
                pass.set_index_buffer(part.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..part.count, 0, 0..1);
            }
        }
        if view.edges || self.model.parts.iter().any(|p| p.diagnostic) {
            pass.set_pipeline(&pipeline.edges);
            for part in &self.parts {
                if !view.edges && !self.model.parts[part.source].diagnostic {
                    continue;
                }
                pass.set_bind_group(0, &part.group, &[]);
                pass.set_vertex_buffer(0, part.vertices.slice(..));
                pass.set_index_buffer(part.edges.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..part.edge_count, 0, 0..1);
            }
        }
    }
}

fn effective_material(source: &Part, view: &View) -> Material {
    if source.diagnostic {
        Material {
            color: view.diagnostic_colors[0],
            alpha_mode: if view.diagnostic_colors[0][3] < 1.0 { 1 } else { 0 },
            ..Default::default()
        }
    } else {
        view.material.unwrap_or(source.material)
    }
}

/// Longest side of a preview target, in physical pixels.
const MAX_TARGET: f32 = 2048.0;

/// The render target of a preview of `size` points. A large preview is
/// scaled down as a whole: clamping each side alone gave the target another
/// shape than its rectangle, and egui stretched the picture to fill it.
fn target_size(size: egui::Vec2, scale: f32) -> [u32; 2] {
    let pixels = size * scale;
    let fit = (MAX_TARGET / pixels.max_elem()).min(1.0);
    [
        (pixels.x * fit).ceil().clamp(1.0, MAX_TARGET) as u32,
        (pixels.y * fit).ceil().clamp(1.0, MAX_TARGET) as u32,
    ]
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
                // egui-wgpu binds the zero-length buffer of an empty mesh
                // unless its clip is empty too. Keep the primitive count so
                // the viewer's glass-layer boundary remains valid.
                primitive.clip_rect = egui::Rect::NOTHING;
                continue;
            }
            let size = target_size(rect.size(), scale);
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
        // Image UUIDs are immutable; all min/max views share their GPU maps.
        // Weak owners bound the cache to the images still held by previews.
        if let Some(pipeline) = &mut self.pipeline {
            pipeline.assets.retain();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_preview_targets_keep_the_shape_of_their_rectangle() {
        // small previews keep one texel per pixel, as before
        assert_eq!(target_size(egui::vec2(230.0, 170.0), 1.0), [230, 170]);
        assert_eq!(target_size(egui::vec2(230.0, 170.0), 1.5), [345, 255]);
        // a wide window past the limit: both sides shrink together
        let [w, h] = target_size(egui::vec2(1760.0, 740.0), 1.5);
        assert_eq!(w, 2048);
        assert!(((w as f32 / h as f32) - 1760.0 / 740.0).abs() < 0.01, "{w}×{h}");
        let [w, h] = target_size(egui::vec2(700.0, 2400.0), 1.0);
        assert_eq!(h, 2048);
        assert!(((w as f32 / h as f32) - 700.0 / 2400.0).abs() < 0.01, "{w}×{h}");
        // an empty rectangle still gets a valid texture
        assert_eq!(target_size(egui::Vec2::ZERO, 1.0), [1, 1]);
    }

    #[test]
    fn transparent_mesh_layers_never_hide_the_textured_skin_behind_them() {
        let _gpu = crate::test_gpu::lock();
        let instance = wgpu::Instance::default();
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
            return;
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).expect("GPU device");
        let mut pipeline = Pipelines::new(&device);
        let mut egui = egui_wgpu::Renderer::new(&device, wgpu::TextureFormat::Rgba8Unorm, Default::default());
        let quad = |x, rgba: [u8; 4], mode| Part {
            diagnostic: false,
            vertices: [[x, -0.8, -0.8], [x, 0.8, -0.8], [x, 0.8, 0.8], [x, -0.8, 0.8]]
                .map(|p| Vertex::new(p, [1.0, 0.0, 0.0], [0.5; 2]))
                .into(),
            indices: Arc::from([0, 1, 2, 0, 2, 3]),
            textures: [
                Some(Arc::new(Image {
                    size: [1, 1],
                    rgba: Arc::from(rgba),
                })),
                None,
                None,
                None,
            ],
            material: Material {
                alpha_mode: mode,
                ..Default::default()
            },
            skin: None,
        };
        let skin = quad(0.0, [220, 80, 30, 255], 0);
        let ghost = quad(0.02, [0; 4], 1);
        let render = |parts: Vec<Part>, pipeline: &mut Pipelines, egui: &mut egui_wgpu::Renderer| {
            const SIZE: u32 = 64;
            let model = Arc::new(Model {
                parts,
                center: Vec3::ZERO,
                radius: 1.0,
            });
            let mut target = Target::new(&device, &queue, egui, pipeline, model, [SIZE; 2]);
            let color = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Layered head test"),
                size: wgpu::Extent3d {
                    width: SIZE,
                    height: SIZE,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            target.color = color.create_view(&Default::default());
            let mut encoder = device.create_command_encoder(&Default::default());
            target.draw(
                &queue,
                &mut encoder,
                pipeline,
                View {
                    yaw: 0.0,
                    pitch: 0.0,
                    distance: 3.0,
                    ..Default::default()
                },
                None,
            );
            let pixels = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Layer pixels"),
                size: u64::from(SIZE * SIZE * 4),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            encoder.copy_texture_to_buffer(
                color.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &pixels,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(SIZE * 4),
                        rows_per_image: Some(SIZE),
                    },
                },
                wgpu::Extent3d {
                    width: SIZE,
                    height: SIZE,
                    depth_or_array_layers: 1,
                },
            );
            queue.submit([encoder.finish()]);
            pixels.map_async(wgpu::MapMode::Read, .., |result| result.expect("pixel readback"));
            device.poll(wgpu::PollType::wait_indefinitely()).expect("test GPU completion");
            let bytes = pixels.get_mapped_range(..).expect("pixels");
            let at = ((SIZE / 2 * SIZE + SIZE / 2) * 4) as usize;
            let pixel: [u8; 4] = bytes[at..at + 4].try_into().expect("pixel");
            egui.free_texture(&target.texture);
            pixel
        };
        let reference = render(vec![skin.clone()], &mut pipeline, &mut egui);
        assert!(reference[0] > reference[1] + 20, "skin is visibly colored");
        assert_eq!(render(vec![ghost.clone(), skin.clone()], &mut pipeline, &mut egui), reference);
        assert_eq!(render(vec![skin, ghost], &mut pipeline, &mut egui), reference);
    }

    #[test]
    fn preview_gestures_pan_with_shift_or_middle_button_and_orbit_without_them() {
        for (button, shift, pan) in [
            (egui::PointerButton::Primary, false, false),
            (egui::PointerButton::Primary, true, true),
            (egui::PointerButton::Middle, false, true),
        ] {
            let ctx = egui::Context::default();
            let model = Arc::new(Model {
                parts: Vec::new(),
                center: Vec3::ZERO,
                radius: 1.0,
            });
            let modifiers = egui::Modifiers {
                shift,
                ..Default::default()
            };
            let mut view = View::default();
            let frame = |mut events: Vec<egui::Event>, view: &mut View| {
                events.insert(0, egui::Event::ModifiersChanged(modifiers));
                let mut rect = egui::Rect::NOTHING;
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(500.0, 400.0))),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        rect = paint(ui, egui::Id::unique("gesture"), model.clone(), view, egui::vec2(200.0, 200.0)).rect;
                    },
                );
                output.textures_delta.clear();
                rect
            };
            let pos = frame(Vec::new(), &mut view).center();
            frame(
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button,
                        pressed: true,
                        modifiers,
                    },
                ],
                &mut view,
            );
            frame(vec![egui::Event::PointerMoved(pos + egui::vec2(30.0, 40.0))], &mut view);
            if pan {
                assert_eq!((view.yaw, view.pitch), (View::default().yaw, View::default().pitch));
                assert!(view.pan[0] > 0.0 && view.pan[1] > 0.0);
            } else {
                assert_eq!(view.pan, [0.0; 2]);
                assert_ne!(view.yaw, View::default().yaw);
            }
        }
    }

    #[test]
    fn pan_follows_the_pointer_in_screen_coordinates_at_every_orbit_and_zoom() {
        let model = Model {
            parts: Vec::new(),
            center: Vec3::new(2.0, -3.0, 1.0),
            radius: 0.6,
        };
        for yaw in [0.0, 1.2, 3.0, -1.5] {
            for pitch in [-1.0, 0.0, 0.8] {
                for distance in [1.0, 3.0, 12.0] {
                    for aspect in [0.7, 1.0, 2.0] {
                        let mut view = View {
                            yaw,
                            pitch,
                            distance,
                            ..Default::default()
                        };
                        let project = |view: &View| {
                            let (eye, center) = view.camera(&model, aspect);
                            let projection = glam::camera::rh::proj::directx::perspective(0.75, aspect, 0.006, 60.0)
                                * glam::camera::rh::view::look_at_mat4(eye, center, Vec3::Z);
                            projection.project_point3(model.center)
                        };
                        let before = project(&view);
                        view.pan = [0.12, 0.08];
                        let delta = project(&view) - before;
                        assert!((delta.x - 0.24).abs() < 0.0001);
                        assert!((delta.y + 0.16).abs() < 0.0001);
                        view.background = [0.1, 0.2, 0.3, 1.0];
                        view.edges = true;
                        view.reset_camera();
                        let default = View::default();
                        assert_eq!(
                            (view.yaw, view.pitch, view.distance, view.pan),
                            (default.yaw, default.pitch, default.distance, default.pan)
                        );
                        assert!(view.edges);
                        assert_eq!(view.background, [0.1, 0.2, 0.3, 1.0]);
                    }
                }
            }
        }
    }

    const SIZE: u32 = 64;

    /// Draws a quad that faces the camera (screen right is +Y, up is +Z)
    /// and carries one dome, then returns where the dome is lit: the offset
    /// of its brightness from the centre, x to the right and y up the image.
    fn lit_side(device: &wgpu::Device, queue: &wgpu::Queue, pipeline: &Pipelines, mirrored: bool, back: bool) -> [f32; 2] {
        let facing = if back { -1.0 } else { 1.0 };
        let corner = |y: f32, z: f32| {
            // seen from its own front side, the face has u to the right
            // unless mirrored; v runs down the top-left-origin image
            let u = (y * facing + 1.0) * 0.5;
            Vertex::new(
                [0.0, y, z],
                [facing, 0.0, 0.0],
                [if mirrored { 1.0 - u } else { u }, (1.0 - z) * 0.5],
            )
        };
        let vertices = [corner(-1.0, -1.0), corner(1.0, -1.0), corner(1.0, 1.0), corner(-1.0, 1.0)];
        let indices: [u32; 6] = if back { [0, 2, 1, 0, 3, 2] } else { [0, 1, 2, 0, 2, 3] };
        // SL / glTF convention: +X right, +Y up the image, rows running down
        let mut dome = Vec::with_capacity((SIZE * SIZE * 4) as usize);
        for y in 0..SIZE {
            for x in 0..SIZE {
                let half = SIZE as f32 * 0.5;
                let (dx, dy) = ((x as f32 + 0.5 - half) / (half * 0.8), (y as f32 + 0.5 - half) / (half * 0.8));
                let slope = (1.0 - dx * dx - dy * dy).max(0.0) * 3.0;
                let n = Vec3::new(slope * dx, -slope * dy, 1.0).normalize();
                dome.extend([n.x, n.y, n.z, 1.0].map(|c| ((c * 0.5 + 0.5) * 255.0) as u8));
            }
        }
        let texture = |format, usage, size: u32, data: Option<&[u8]>| {
            let descriptor = wgpu::TextureDescriptor {
                label: Some("Preview test"),
                size: wgpu::Extent3d {
                    width: size,
                    height: size,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            };
            match data {
                Some(data) => device.create_texture_with_data(queue, &descriptor, wgpu::util::TextureDataOrder::LayerMajor, data),
                None => device.create_texture(&descriptor),
            }
        };
        let sampled = wgpu::TextureUsages::TEXTURE_BINDING;
        let attachment = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC;
        let maps = [
            texture(wgpu::TextureFormat::Rgba8UnormSrgb, sampled, 1, Some(&[255; 4])),
            texture(wgpu::TextureFormat::Rgba8Unorm, sampled, SIZE, Some(&dome)),
            texture(wgpu::TextureFormat::Rgba8Unorm, sampled, 1, Some(&[255; 4])),
            texture(wgpu::TextureFormat::Rgba8UnormSrgb, sampled, 1, Some(&[255; 4])),
        ]
        .map(|map| map.create_view(&Default::default()));
        let eye = Vec3::new(4.0, 0.0, 0.0);
        let projection = glam::camera::rh::proj::directx::perspective(0.55, 1.0, 0.1, 10.0)
            * glam::camera::rh::view::look_at_mat4(eye, Vec3::ZERO, Vec3::Z);
        let buffer = |bytes: &[u8], usage| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Preview test"),
                contents: bytes,
                usage,
            })
        };
        let uniform = buffer(
            bytemuck::bytes_of(&Uniform {
                projection: projection.to_cols_array_2d(),
                eye: [eye.x, eye.y, eye.z, 1.0],
                color: [1.0; 4],
                emissive_metallic: [0.0; 4],
                params: [0.9, 0.5, 0.0, 0.0],
                edge_color: [0.0; 4],
                ambient: [0.2, 0.2, 0.2, 1.0],
                surface: [1.0, 1.0, 0.0, 0.0],
                uv: Material::default().uv,
            }),
            wgpu::BufferUsages::UNIFORM,
        );
        let bones = buffer(
            bytemuck::cast_slice(&[Mat4::IDENTITY.to_cols_array_2d()]),
            wgpu::BufferUsages::STORAGE,
        );
        let mut entries = vec![wgpu::BindGroupEntry {
            binding: 0,
            resource: uniform.as_entire_binding(),
        }];
        for (index, view) in maps.iter().enumerate() {
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
            label: Some("Preview test"),
            layout: &pipeline.layout,
            entries: &entries,
        });
        let vertex_buffer = buffer(bytemuck::cast_slice(&vertices), wgpu::BufferUsages::VERTEX);
        let index_buffer = buffer(bytemuck::cast_slice(&indices), wgpu::BufferUsages::INDEX);
        let color = texture(wgpu::TextureFormat::Rgba8Unorm, attachment, SIZE, None);
        let depth = texture(wgpu::TextureFormat::Depth32Float, attachment, SIZE, None).create_view(&Default::default());
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Preview test readback"),
            size: u64::from(SIZE * SIZE * 4),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Preview test"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color.create_view(&Default::default()),
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth,
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
            pass.set_bind_group(0, &group, &[]);
            pass.set_vertex_buffer(0, vertex_buffer.slice(..));
            pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..6, 0, 0..1);
        }
        encoder.copy_texture_to_buffer(
            color.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(SIZE * 4),
                    rows_per_image: Some(SIZE),
                },
            },
            wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
        );
        queue.submit([encoder.finish()]);
        readback.map_async(wgpu::MapMode::Read, .., |result| result.expect("readback"));
        device.poll(wgpu::PollType::wait_indefinitely()).expect("GPU completion");
        let bytes = readback.get_mapped_range(..).expect("mapped pixels");
        let brightness = |x: u32, y: u32| f32::from(bytes[((y * SIZE + x) * 4) as usize]);
        // the quad covers the middle of the target; (7, 7) is on it, outside the dome
        let flat = brightness(7, 7);
        assert!(flat > 0.0, "the flat part of the quad is lit and drawn");
        let mut side = [0.0f32; 2];
        for y in 8..SIZE - 8 {
            for x in 8..SIZE - 8 {
                let weight = brightness(x, y) - flat;
                side[0] += weight * (x as f32 + 0.5 - SIZE as f32 * 0.5);
                side[1] -= weight * (y as f32 + 0.5 - SIZE as f32 * 0.5);
            }
        }
        side
    }

    /// The fixed light of preview.wgsl, (0.4, -0.6, 0.8), is up and to the
    /// left of this camera. A bump is lit on that side whatever the UV
    /// handedness; from behind a double-sided face it is a dent, lit on the
    /// far side (pbropaqueF.glsl negates the whole mapped normal there).
    #[test]
    fn normal_map_bumps_face_the_light() {
        let _gpu = crate::test_gpu::lock();
        let instance = wgpu::Instance::default();
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).expect("GPU device");
        let pipeline = Pipelines::new(&device);
        for (mirrored, back, name) in [
            (false, false, "regular UV"),
            (true, false, "mirrored UV"),
            (false, true, "back side"),
        ] {
            let [x, y] = lit_side(&device, &queue, &pipeline, mirrored, back);
            let toward_light = if back { -1.0 } else { 1.0 };
            assert!(
                x * toward_light < -10_000.0 && y * toward_light > 10_000.0,
                "{name}: lit side ({x}, {y})"
            );
        }
    }
}
