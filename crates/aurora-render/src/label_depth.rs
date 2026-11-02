//! The labels of the world (avatar name tags, floating texts) are hidden
//! by what the scene has in front of them: a wall cuts the name of someone
//! standing half behind it, and hides the names of those in the next room.
//!
//! Firestorm draws these labels in the world with the depth test
//! (LLHUDNameTag::render and LLHUDText::render in indra/newview:
//! `LLGLDepthTest gls_depth(GL_TRUE, GL_FALSE)`). Here the interface paints
//! them, as flat shapes on the layer that `ui_world` has drawn under the
//! HUD attachments, and egui's pipeline knows no depth. So the viewer marks
//! where in the world each label stands ([`world_label_at`]), and that part
//! of the interface is drawn by a pipeline of this module instead of
//! egui-wgpu's: the same vertices, textures and colours
//! (shaders/labels.wgsl), each fragment compared with the scene's depth at
//! its pixel. Nothing is read back and nothing is copied: the labels the
//! scene hides entirely cost their vertices, as before.

use crate::pipeline_cache::CachedPipelines;
use bytemuck::{Pod, Zeroable};
use egui::epaint::{ClippedPrimitive, Primitive};
use glam::{Mat4, Vec2, Vec3};
use std::ops::Range;
use std::sync::Arc;

/// Where the shapes that follow stand in the world; None for nowhere.
struct At(Option<Vec3>);

/// A guide segment has a different depth at each endpoint. Depth is linear
/// in screen space; its egui vertices retain that slope across the line.
struct Along(Vec3, Vec3);

/// What is painted for the world after this call (on the layer of
/// `ui::hud::world_painter`, before `end_world_labels`) stands at `at`: the
/// scene hides it wherever it is nearer to the camera than that point. None:
/// what follows is never hidden, like what precedes the first call.
pub fn world_label_at(ctx: &egui::Context, at: Option<Vec3>) {
    ctx.layer_painter(egui::LayerId::background()).add(egui::PaintCallback {
        rect: ctx.content_rect(),
        callback: Arc::new(At(at)),
    });
}

pub fn world_line_at(ctx: &egui::Context, a: Vec3, b: Vec3) {
    ctx.layer_painter(egui::LayerId::background()).add(egui::PaintCallback {
        rect: ctx.content_rect(),
        callback: Arc::new(Along(a, b)),
    });
}

fn line_plane(a: Vec3, b: Vec3, view: &LabelView) -> Option<[Vec3; 2]> {
    let (mut a, mut b) = (view.view_proj * a.extend(1.0), view.view_proj * b.extend(1.0));
    if !a.is_finite() || !b.is_finite() || (a.w <= 1e-5 && b.w <= 1e-5) {
        return None;
    }
    if a.w < 1e-5 {
        a = a.lerp(b, (1e-5 - a.w) / (b.w - a.w));
    }
    if b.w < 1e-5 {
        b = b.lerp(a, (1e-5 - b.w) / (a.w - b.w));
    }
    let project = |c: glam::Vec4| {
        Vec3::new(
            (c.x / c.w * 0.5 + 0.5) * view.size[0] as f32 / view.pixels_per_point,
            (0.5 - c.y / c.w * 0.5) * view.size[1] as f32 / view.pixels_per_point,
            c.z / c.w,
        )
    };
    Some([project(a), project(b)])
}

fn depth_along(line: [Vec3; 2], at: egui::Pos2) -> f32 {
    let [a, b] = line;
    let d = b.truncate() - a.truncate();
    let t = if d.length_squared() > 1e-8 {
        ((Vec2::new(at.x, at.y) - a.truncate()).dot(d) / d.length_squared()).clamp(0.0, 1.0)
    } else {
        0.5
    };
    a.z + (b.z - a.z) * t
}

#[cfg(test)]
#[test]
fn beacon_line_depth_tracks_both_ends_and_clips_the_near_plane() {
    let view = LabelView {
        view_proj: glam::camera::rh::proj::directx::perspective_infinite_reverse(1.0, 1.0, 0.1),
        pixels_per_point: 1.5,
        size: [900, 900],
        jitter: Vec2::ZERO,
    };
    let a = Vec3::new(-1.0, 0.0, -2.0);
    let b = Vec3::new(1.0, 0.0, -10.0);
    let [pa, pb] = line_plane(a, b, &view).unwrap();
    assert!((depth_along([pa, pb], egui::pos2(pa.x, pa.y)) - 0.05).abs() < 1e-6);
    assert!((depth_along([pa, pb], egui::pos2(pb.x, pb.y)) - 0.01).abs() < 1e-6);
    let mid = (pa + pb) * 0.5;
    assert!((depth_along([pa, pb], egui::pos2(mid.x, mid.y)) - 0.03).abs() < 1e-6);
    assert!(
        line_plane(a, Vec3::new(0.0, 0.0, 1.0), &view)
            .unwrap()
            .into_iter()
            .all(Vec3::is_finite)
    );
    assert!(line_plane(Vec3::Z, Vec3::Z * 2.0, &view).is_none());
}

/// Depth of what is never hidden: nearer than the near plane (reverse-Z).
const NEVER_HIDDEN: f32 = 2.0;

/// A vertex of egui with the depth its label stands at.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
struct Vertex {
    pos: [f32; 2],
    uv: [f32; 2],
    color: [u8; 4],
    depth: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Locals {
    screen_size: [f32; 2],
    screen_px: [f32; 2],
    jitter: [f32; 2],
    dithering: u32,
    _pad: u32,
}

/// One mesh of the labels: its clip in pixels, texture and indices.
#[derive(Clone, Debug, PartialEq)]
struct Draw {
    /// x, y, width, height.
    scissor: [u32; 4],
    texture: egui::TextureId,
    indices: Range<u32>,
    base_vertex: i32,
}

/// The camera and the surface of a frame.
#[derive(Clone, Copy)]
pub(crate) struct LabelView {
    /// Without the jitter of temporal anti-aliasing: the labels do not shake.
    pub view_proj: Mat4,
    pub pixels_per_point: f32,
    pub size: [u32; 2],
    /// From a pixel of the surface to where the scene's depth has it.
    pub jitter: Vec2,
}

/// `clip` in whole pixels of a surface of `size`, as egui-wgpu's
/// `ScissorRect`; None when nothing of it is on the surface.
fn scissor(clip: egui::Rect, pixels_per_point: f32, size: [u32; 2]) -> Option<[u32; 4]> {
    let px = |v: f32, limit: u32| ((v * pixels_per_point).round().max(0.0) as u32).min(limit);
    let (x0, y0) = (px(clip.min.x, size[0]), px(clip.min.y, size[1]));
    let (x1, y1) = (px(clip.max.x, size[0]), px(clip.max.y, size[1]));
    (x1 > x0 && y1 > y0).then(|| [x0, y0, x1 - x0, y1 - y0])
}

/// The meshes of `primitives` (what the interface painted for the world)
/// with the depth of the mark that precedes each, appended to `vertices`,
/// `indices` and `draws`. The primitives are left empty: egui-wgpu neither
/// uploads nor draws them, and their count does not change.
fn collect(
    primitives: &mut [ClippedPrimitive],
    view: &LabelView,
    vertices: &mut Vec<Vertex>,
    indices: &mut Vec<u32>,
    draws: &mut Vec<Draw>,
) {
    let mut depth = NEVER_HIDDEN;
    let mut along = None;
    for p in primitives {
        match &p.primitive {
            Primitive::Callback(c) => {
                if let Some(At(at)) = c.callback.downcast_ref::<At>() {
                    along = None;
                    depth = at
                        .map(|at| view.view_proj * at.extend(1.0))
                        // (behind the camera: its label is not on screen)
                        .filter(|clip| clip.w > 1e-6)
                        .map_or(NEVER_HIDDEN, |clip| clip.z / clip.w);
                } else if let Some(Along(a, b)) = c.callback.downcast_ref::<Along>() {
                    along = line_plane(*a, *b, view);
                }
            }
            Primitive::Mesh(mesh) => {
                if let Some(scissor) = scissor(p.clip_rect, view.pixels_per_point, view.size).filter(|_| !mesh.indices.is_empty()) {
                    let (first_vertex, first_index) = (vertices.len(), indices.len());
                    vertices.extend(mesh.vertices.iter().map(|v| Vertex {
                        pos: [v.pos.x, v.pos.y],
                        uv: [v.uv.x, v.uv.y],
                        color: v.color.to_array(),
                        depth: along.map_or(depth, |line| depth_along(line, v.pos)),
                    }));
                    // The marks keep the labels in meshes of their own.
                    // Here the depth is in the vertices: what follows a mesh
                    // with the same texture and clip joins its draw (the far
                    // names of a crowd, one after the other).
                    match draws.last_mut().filter(|d| d.texture == mesh.texture_id && d.scissor == scissor) {
                        Some(draw) => {
                            let offset = (first_vertex as i32 - draw.base_vertex) as u32;
                            indices.extend(mesh.indices.iter().map(|i| i + offset));
                            draw.indices.end = indices.len() as u32;
                        }
                        None => {
                            indices.extend_from_slice(&mesh.indices);
                            draws.push(Draw {
                                scissor,
                                texture: mesh.texture_id,
                                indices: first_index as u32..indices.len() as u32,
                                base_vertex: first_vertex as i32,
                            });
                        }
                    }
                }
            }
        }
        // An empty mesh must not reach egui-wgpu's draw (it binds the index
        // range of every mesh inside its clip): an empty clip only advances
        // its buffers.
        p.primitive = Primitive::Mesh(egui::Mesh::default());
        p.clip_rect = egui::Rect::NOTHING;
    }
}

pub(crate) struct LabelDepth {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    locals: wgpu::Buffer,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    /// This frame's meshes; the vectors keep their memory.
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
    draws: Vec<Draw>,
    /// Locals and the scene's depth: made again when that depth is (a
    /// resize, another anti-aliasing).
    bind_group: Option<(wgpu::TextureView, wgpu::BindGroup)>,
    /// False with AURORA_LABEL_OCCLUSION=0: egui-wgpu draws the labels over
    /// the scene, as before.
    enabled: bool,
}

/// Without this module, egui-wgpu draws the labels itself: the marks of
/// where they stand are not its callbacks, and leave no trace.
pub(crate) fn skip(primitives: &mut [ClippedPrimitive]) {
    for p in primitives {
        if matches!(&p.primitive, Primitive::Callback(c) if c.callback.is::<At>() || c.callback.is::<Along>()) {
            // (an empty mesh must have an empty clip, see `collect`)
            p.primitive = Primitive::Mesh(egui::Mesh::default());
            p.clip_rect = egui::Rect::NOTHING;
        }
    }
}

fn buffer(device: &wgpu::Device, label: &str, size: u64, usage: wgpu::BufferUsages) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage: usage | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

impl LabelDepth {
    /// `format`: the surface the interface is drawn on.
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> LabelDepth {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("labels"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/labels.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("labels"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
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
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        // The textures are egui-wgpu's, with their bind groups
        // (`egui_wgpu::Renderer::texture`): this layout is the one it makes
        // them with, entry for entry, which is what makes them fit here.
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("labels texture"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                // the texture's flags (egui 0.37: filter and wrap mode, read
                // only by egui's own shader when its filtering is predictable)
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(16),
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("labels"),
            bind_group_layouts: &[Some(&layout), Some(&texture_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline_cached(&wgpu::RenderPipelineDescriptor {
            label: Some("labels"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Uint32, 3 => Float32],
                })],
                compilation_options: Default::default(),
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some(if format.is_srgb() {
                    "fs_main_linear_framebuffer"
                } else {
                    "fs_main_gamma_framebuffer"
                }),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    // egui's: premultiplied alpha
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::OneMinusDstAlpha,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            multiview_mask: None,
            cache: None,
        });
        LabelDepth {
            pipeline,
            layout,
            locals: buffer(
                device,
                "labels locals",
                std::mem::size_of::<Locals>() as u64,
                wgpu::BufferUsages::UNIFORM,
            ),
            vertex_buffer: buffer(device, "labels vertices", 64 * 1024, wgpu::BufferUsages::VERTEX),
            index_buffer: buffer(device, "labels indices", 16 * 1024, wgpu::BufferUsages::INDEX),
            vertices: Vec::new(),
            indices: Vec::new(),
            draws: Vec::new(),
            bind_group: None,
            enabled: !std::env::var("AURORA_LABEL_OCCLUSION").is_ok_and(|v| v.trim() == "0"),
        }
    }

    /// Whether the labels are this module's to draw ([`Self::prepare`],
    /// [`Self::render`]); otherwise they stay egui-wgpu's, after [`skip`].
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Take `primitives` (what the interface painted for the world, its
    /// glass already turned into meshes) out of egui-wgpu's hands and get
    /// them ready for [`Self::render`], against `depth` (the scene's, of
    /// this frame). Before `egui_wgpu::Renderer::update_buffers`.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        depth: &wgpu::TextureView,
        primitives: &mut [ClippedPrimitive],
        view: &LabelView,
    ) {
        self.vertices.clear();
        self.indices.clear();
        self.draws.clear();
        collect(primitives, view, &mut self.vertices, &mut self.indices, &mut self.draws);
        if self.draws.is_empty() {
            return;
        }
        let grow = |device: &wgpu::Device, buffer: &mut wgpu::Buffer, label: &str, bytes: u64, usage| {
            if bytes > buffer.size() {
                *buffer = self::buffer(device, label, bytes.next_power_of_two(), usage);
            }
        };
        let (vertices, indices): (&[u8], &[u8]) = (bytemuck::cast_slice(&self.vertices), bytemuck::cast_slice(&self.indices));
        grow(
            device,
            &mut self.vertex_buffer,
            "labels vertices",
            vertices.len() as u64,
            wgpu::BufferUsages::VERTEX,
        );
        grow(
            device,
            &mut self.index_buffer,
            "labels indices",
            indices.len() as u64,
            wgpu::BufferUsages::INDEX,
        );
        queue.write_buffer(&self.vertex_buffer, 0, vertices);
        queue.write_buffer(&self.index_buffer, 0, indices);
        let [w, h] = view.size.map(|side| side.max(1) as f32);
        let locals = Locals {
            screen_size: [w / view.pixels_per_point, h / view.pixels_per_point],
            screen_px: [w, h],
            jitter: view.jitter.to_array(),
            // as `egui_wgpu::RendererOptions::default`
            dithering: 1,
            _pad: 0,
        };
        queue.write_buffer(&self.locals, 0, bytemuck::bytes_of(&locals));
        if !self.bind_group.as_ref().is_some_and(|(of, _)| of == depth) {
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("labels"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.locals.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(depth),
                    },
                ],
            });
            self.bind_group = Some((depth.clone(), bind_group));
        }
    }

    /// Whether the frame prepared has labels to draw.
    pub fn any(&self) -> bool {
        !self.draws.is_empty()
    }

    /// Draw the labels prepared, in order, with the textures of `egui`.
    pub fn render(&self, pass: &mut wgpu::RenderPass<'_>, egui: &egui_wgpu::Renderer) {
        let Some((_, bind_group)) = self.bind_group.as_ref().filter(|_| self.any()) else {
            return;
        };
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        let (mut scissor, mut bound) = (None, None);
        for draw in &self.draws {
            let Some(texture) = egui.texture(&draw.texture) else {
                log::warn!("label texture missing: {:?}", draw.texture);
                continue;
            };
            if scissor.replace(draw.scissor) != Some(draw.scissor) {
                let [x, y, w, h] = draw.scissor;
                pass.set_scissor_rect(x, y, w, h);
            }
            if bound.replace(draw.texture) != Some(draw.texture) {
                pass.set_bind_group(1, &texture.bind_group, &[]);
            }
            pass.draw_indexed(draw.indices.clone(), draw.base_vertex, 0..1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Color32, Rect, pos2, vec2};

    fn view() -> LabelView {
        let proj = glam::camera::rh::proj::directx::perspective_infinite_reverse(1.0, 800.0 / 600.0, 0.1);
        let look = glam::camera::rh::view::look_at_mat4(Vec3::ZERO, Vec3::X, Vec3::Z);
        LabelView {
            view_proj: proj * look,
            pixels_per_point: 1.0,
            size: [800, 600],
            jitter: Vec2::ZERO,
        }
    }

    fn depth_of(at: Vec3) -> f32 {
        let clip = view().view_proj * at.extend(1.0);
        clip.z / clip.w
    }

    /// What the interface painted for the world in a frame, tessellated:
    /// a square, then one at each of `marks`.
    fn labels(marks: &[Option<Vec3>]) -> Vec<ClippedPrimitive> {
        let ctx = egui::Context::default();
        let square = |k: usize| Rect::from_min_size(pos2(40.0 * k as f32, 10.0), vec2(20.0, 20.0));
        let mut output = ctx.run_ui(Default::default(), |_| {
            let painter = ctx.layer_painter(egui::LayerId::background());
            painter.rect_filled(square(0), 0.0, Color32::RED);
            for (k, at) in marks.iter().enumerate() {
                world_label_at(&ctx, *at);
                painter.rect_filled(square(k + 1), 0.0, Color32::GREEN);
            }
            crate::end_world_labels(&ctx);
            painter.rect_filled(square(9), 0.0, Color32::BLUE);
        });
        // no renderer applies the texture changes here
        output.textures_delta.clear();
        let mut primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
        let end = crate::ui_world::take_end(&mut primitives);
        primitives.truncate(end);
        primitives
    }

    fn collected(primitives: &mut [ClippedPrimitive]) -> (Vec<Vertex>, Vec<u32>, Vec<Draw>) {
        let (mut vertices, mut indices, mut draws) = (Vec::new(), Vec::new(), Vec::new());
        collect(primitives, &view(), &mut vertices, &mut indices, &mut draws);
        (vertices, indices, draws)
    }

    /// The square of `labels` a vertex belongs to.
    fn square_of(v: &Vertex) -> usize {
        ((v.pos[0] + 10.0) / 40.0) as usize
    }

    /// Depth of each of the first `n` squares of `labels`: that of all its
    /// vertices.
    fn depths(vertices: &[Vertex], n: usize) -> Vec<f32> {
        (0..n)
            .map(|k| {
                let mut of = vertices.iter().filter(|v| square_of(v) == k).map(|v| v.depth);
                let first = of.next().expect("the square is drawn");
                assert!(of.all(|d| d == first), "square {k}");
                first
            })
            .collect()
    }

    /// A square of `texture` at `x`, as a primitive clipped by `clip`.
    fn mesh(texture: u64, x: f32, clip: Rect) -> ClippedPrimitive {
        // (epaint only fills a rectangle in a mesh of its own texture)
        let mut mesh = egui::Mesh::default();
        mesh.add_colored_rect(Rect::from_min_size(pos2(x, 10.0), vec2(20.0, 20.0)), Color32::RED);
        mesh.texture_id = egui::TextureId::User(texture);
        ClippedPrimitive {
            clip_rect: clip,
            primitive: Primitive::Mesh(mesh),
        }
    }

    #[test]
    fn the_shader_is_valid() {
        let src = include_str!("shaders/labels.wgsl");
        let module = match naga::front::wgsl::parse_str(src) {
            Ok(m) => m,
            Err(e) => panic!("{}", e.emit_to_string(src)),
        };
        let mut v = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all());
        if let Err(e) = v.validate(&module) {
            panic!("{}", e.emit_to_string(src));
        }
    }

    #[test]
    fn each_mesh_stands_at_the_mark_that_precedes_it() {
        let (near, far) = (Vec3::new(5.0, 0.0, 1.0), Vec3::new(40.0, 3.0, 1.0));
        let mut primitives = labels(&[Some(far), Some(near), None]);
        let count = primitives.len();
        // the marks keep the squares in meshes of their own
        assert_eq!(count, 4 + 3);
        let (vertices, indices, draws) = collected(&mut primitives);
        assert_eq!(
            depths(&vertices, 4),
            vec![NEVER_HIDDEN, depth_of(far), depth_of(near), NEVER_HIDDEN]
        );
        // reverse-Z: the nearer, the larger; all under what is never hidden
        assert!(depth_of(near) > depth_of(far) && depth_of(near) < 1.0);
        // same texture, same clip: one draw, whose triangles each stay in
        // their square
        assert_eq!(draws.len(), 1);
        let d = &draws[0];
        assert_eq!((d.base_vertex, d.indices.clone()), (0, 0..indices.len() as u32));
        assert!(d.scissor[2] > 0 && d.scissor[3] > 0);
        for triangle in indices.as_chunks::<3>().0 {
            let squares: Vec<usize> = triangle.iter().map(|&i| square_of(&vertices[i as usize])).collect();
            assert!(squares.iter().all(|s| *s == squares[0]), "{squares:?}");
        }
        for k in 0..4 {
            assert!(indices.iter().any(|&i| square_of(&vertices[i as usize]) == k), "square {k}");
        }
        // nothing is left for egui-wgpu to upload or draw, and the count
        // the other marks of the interface rely on has not changed
        assert_eq!(primitives.len(), count);
        for p in &primitives {
            assert!(matches!(&p.primitive, Primitive::Mesh(m) if m.indices.is_empty()));
            assert!(!p.clip_rect.is_positive());
        }
    }

    #[test]
    fn a_mark_behind_the_camera_hides_nothing() {
        let mut primitives = labels(&[Some(Vec3::new(-5.0, 0.0, 1.0))]);
        let (vertices, _, _) = collected(&mut primitives);
        assert_eq!(depths(&vertices, 2), vec![NEVER_HIDDEN; 2]);
    }

    #[test]
    fn meshes_join_a_draw_only_with_its_texture_and_clip() {
        let (clip, other) = (Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 300.0)), Rect::EVERYTHING);
        let mut primitives = vec![
            mesh(0, 0.0, clip),
            mesh(0, 40.0, clip),
            mesh(1, 80.0, clip),
            mesh(0, 120.0, clip),
            mesh(0, 160.0, other),
        ];
        let (vertices, indices, draws) = collected(&mut primitives);
        let textures: Vec<_> = draws.iter().map(|d| d.texture).collect();
        let user = egui::TextureId::User;
        assert_eq!(textures, vec![user(0), user(1), user(0), user(0)]);
        // two squares, then one each: in order, nothing drawn twice
        let counts: Vec<u32> = draws.iter().map(|d| d.indices.end - d.indices.start).collect();
        assert_eq!(counts, vec![12, 6, 6, 6]);
        assert_eq!(draws[1].indices.start, draws[0].indices.end);
        for d in &draws {
            for &i in &indices[d.indices.start as usize..d.indices.end as usize] {
                assert!(((d.base_vertex as u32 + i) as usize) < vertices.len());
            }
        }
        // the second square's own vertices, not the first's again
        let second: Vec<usize> = indices[6..12].iter().map(|&i| square_of(&vertices[i as usize])).collect();
        assert_eq!(second, vec![1; 6]);
    }

    /// AURORA_LABEL_OCCLUSION=0: egui-wgpu draws the labels, and would warn
    /// about each mark, a callback that is not its own.
    #[test]
    fn switched_off_the_marks_are_gone_and_the_meshes_stay() {
        let mut primitives = labels(&[Some(Vec3::new(5.0, 0.0, 1.0)), None]);
        let count = primitives.len();
        let meshes = |primitives: &[ClippedPrimitive]| {
            primitives
                .iter()
                .filter(|p| matches!(&p.primitive, Primitive::Mesh(m) if !m.indices.is_empty()) && p.clip_rect.is_positive())
                .count()
        };
        assert_eq!(meshes(&primitives), 3);
        skip(&mut primitives);
        assert_eq!((primitives.len(), meshes(&primitives)), (count, 3));
        for p in &primitives {
            let Primitive::Mesh(mesh) = &p.primitive else {
                panic!("no callback is left");
            };
            assert_eq!(mesh.indices.is_empty(), !p.clip_rect.is_positive());
        }
    }

    #[test]
    fn a_mesh_clipped_out_of_the_surface_is_not_drawn() {
        let mut primitives = labels(&[Some(Vec3::new(5.0, 0.0, 1.0))]);
        for p in &mut primitives {
            p.clip_rect = Rect::from_min_size(pos2(900.0, 10.0), vec2(50.0, 50.0));
        }
        let (vertices, indices, draws) = collected(&mut primitives);
        assert!(vertices.is_empty() && indices.is_empty() && draws.is_empty());
    }

    #[test]
    fn the_clip_is_in_whole_pixels_inside_the_surface() {
        let clip = Rect::from_min_max(pos2(-10.0, 20.4), pos2(100.6, 2000.0));
        assert_eq!(scissor(clip, 1.0, [800, 600]), Some([0, 20, 101, 580]));
        assert_eq!(scissor(clip, 1.5, [800, 600]), Some([0, 31, 151, 569]));
        assert_eq!(scissor(Rect::NOTHING, 1.0, [800, 600]), None);
        assert_eq!(scissor(Rect::EVERYTHING, 1.0, [800, 600]), Some([0, 0, 800, 600]));
    }
}
