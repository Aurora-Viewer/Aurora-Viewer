//! The interface of a frame, from the main thread to the render thread.
//!
//! egui ends its pass with shapes (rectangles, rows of text already laid
//! out, paths, meshes). Turning them into triangles needs nothing the main
//! thread is busy with: it hands the shapes over as they are
//! ([`EguiHandOver::frame`]) and the render thread tessellates them
//! ([`EguiFrame::tessellate`]) while the next frame starts. The triangles
//! are those of `egui::Context::tessellate`, which would hold the context's
//! lock for the whole of it.
//!
//! The tessellator reads two facts of egui's font atlas: its size (the UVs
//! of the glyphs are in texels) and where its pre-rasterized discs are (a
//! small filled circle is one textured quad). egui does not expose the
//! discs of the context's atlas (nor the name of their type), but
//! `TextureAtlas::new` allocates them before anything else: an atlas made
//! the same way, of the same width and options, has them on the same
//! texels. [`EguiHandOver`] keeps such a twin, a few rows high.

use egui::epaint::{ClippedPrimitive, ClippedShape, Primitive, Tessellator, TextureAtlas};
use std::borrow::Cow;

/// Height egui gives a new font atlas (`FontsImpl::new`); it only grows.
const NEW_ATLAS_HEIGHT: usize = 32;

/// Whether `egui_wgpu::Renderer::update_buffers` gives `primitives` slices
/// of its buffers: it refreshes its index slices only when the frame has
/// indices, its vertex slices only when it has vertices, and otherwise
/// keeps those of an earlier frame.
pub(crate) fn meshes_uploaded(primitives: &[ClippedPrimitive]) -> bool {
    let (mut vertices, mut indices) = (false, false);
    for p in primitives {
        if let Primitive::Mesh(mesh) = &p.primitive {
            vertices |= !mesh.vertices.is_empty();
            indices |= !mesh.indices.is_empty();
        }
    }
    vertices && indices
}

/// What to hand `egui_wgpu::Renderer::render` of `primitives`, part of a
/// frame whose buffers were updated (`uploaded`: [`meshes_uploaded`]).
///
/// `render` takes one slice per mesh, even an empty one under an empty
/// clip. The labels, the glass and the previews leave such empty meshes in
/// the frame; when nothing else is left (mouselook hides the windows, the
/// labels go to `label_depth`), the slices are an earlier frame's and
/// `render` panics as soon as there are more meshes than slices (#1235).
/// Such a frame has no triangle to draw: only its paint callbacks remain.
pub(crate) fn paint_jobs(primitives: &[ClippedPrimitive], uploaded: bool) -> Cow<'_, [ClippedPrimitive]> {
    if uploaded {
        return Cow::Borrowed(primitives);
    }
    let callbacks = primitives.iter().filter(|p| matches!(p.primitive, Primitive::Callback(_)));
    Cow::Owned(callbacks.cloned().collect())
}

/// The interface of a frame as egui left it: shapes still to tessellate
/// and texture changes.
pub struct EguiFrame {
    shapes: Vec<ClippedShape>,
    pub textures_delta: egui::TexturesDelta,
    pub pixels_per_point: f32,
    /// Ready for this frame: options, font atlas size and discs.
    tessellator: Tessellator,
}

/// The interface of a frame, tessellated.
pub(crate) struct UiFrame {
    pub primitives: Vec<ClippedPrimitive>,
    pub pixels_per_point: f32,
    /// Shapes the primitives come from, and their vertices (AURORA_PROFILE).
    pub shapes: usize,
    pub vertices: usize,
}

impl EguiFrame {
    /// Same result as `egui::Context::tessellate` on the frame's shapes.
    pub(crate) fn tessellate(self) -> UiFrame {
        let EguiFrame {
            shapes, mut tessellator, ..
        } = self;
        let shape_count = shapes.len();
        let primitives = tessellator.tessellate_shapes(shapes);
        let vertices = primitives.iter().map(|p| match &p.primitive {
            Primitive::Mesh(mesh) => mesh.vertices.len(),
            Primitive::Callback(_) => 0,
        });
        UiFrame {
            vertices: vertices.sum(),
            primitives,
            pixels_per_point: self.pixels_per_point,
            shapes: shape_count,
        }
    }
}

/// What the main thread keeps from frame to frame to hand the interface
/// over: the twin of the context's font atlas, made again when that atlas
/// is, with another width or other options.
#[derive(Default)]
pub struct EguiHandOver {
    twin: Option<TextureAtlas>,
}

impl EguiHandOver {
    /// The frame to give `Renderer::render`, from the output of the pass
    /// that just ended on `ctx`.
    pub fn frame(
        &mut self,
        ctx: &egui::Context,
        shapes: Vec<ClippedShape>,
        textures_delta: egui::TexturesDelta,
        pixels_per_point: f32,
    ) -> EguiFrame {
        let (atlas_size, text) = ctx.fonts(|f| (f.font_image_size(), *f.options()));
        let twin = match &mut self.twin {
            Some(twin) if twin.size()[0] == atlas_size[0] && *twin.options() == text => twin,
            twin => twin.insert(TextureAtlas::new([atlas_size[0], NEW_ATLAS_HEIGHT], text)),
        };
        // the twin's discs are normalized by its own size: back to whole
        // texels, then the arithmetic of `TextureAtlas::prepared_discs` on
        // the size the context's atlas has now
        let [twin_w, twin_h] = twin.size().map(|side| side as f32);
        let (inv_w, inv_h) = (1.0 / atlas_size[0] as f32, 1.0 / atlas_size[1] as f32);
        let mut discs = twin.prepared_discs();
        for disc in &mut discs {
            for corner in [&mut disc.uv.min, &mut disc.uv.max] {
                corner.x = (corner.x * twin_w).round() * inv_w;
                corner.y = (corner.y * twin_h).round() * inv_h;
            }
        }
        EguiFrame {
            shapes,
            textures_delta,
            pixels_per_point,
            tessellator: Tessellator::new(pixels_per_point, ctx.tessellation_options(|o| *o), atlas_size, discs),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pass with what the interface draws: text, rounded and stroked
    /// rectangles, circles small enough for the atlas discs and larger
    /// ones, lines, a clipped area; from 12 rows on, a title whose glyphs
    /// do not fit in the rows of a new atlas. No renderer applies the
    /// texture changes here: they are dropped on purpose.
    fn pass(ctx: &egui::Context, rows: usize, max_texture_side: usize) -> egui::FullOutput {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 700.0))),
            max_texture_side: Some(max_texture_side),
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| {
            let painter = ui.painter().clone();
            for i in 0..24 {
                let c = egui::pos2(20.0 + 30.0 * i as f32, 600.0);
                painter.circle_filled(c, 0.5 + i as f32 * 0.75, egui::Color32::from_rgb(200, 40 + i * 8, 90));
                painter.circle_stroke(
                    c + egui::vec2(0.0, 40.0),
                    2.0 + i as f32,
                    egui::Stroke::new(1.5, egui::Color32::WHITE),
                );
            }
            painter.line_segment(
                [egui::pos2(5.0, 5.0), egui::pos2(300.5, 90.25)],
                egui::Stroke::new(2.0, egui::Color32::YELLOW),
            );
            egui::Window::new("Window").show(ui.ctx(), |ui| {
                if rows >= 12 {
                    ui.label(egui::RichText::new("Aurora").size(rows as f32 * 3.0));
                }
                egui::ScrollArea::vertical().max_height(120.0).show(ui, |ui| {
                    for row in 0..rows {
                        ui.horizontal(|ui| {
                            ui.label(format!("Row {row}: Aurora Viewer éàü"));
                            let _ = ui.button("Open");
                            let _ = ui.radio(row % 2 == 0, "Radio");
                        });
                    }
                });
            });
        });
        out.textures_delta.clear();
        out
    }

    fn assert_same(ours: &[ClippedPrimitive], egui: &[ClippedPrimitive]) {
        assert_eq!(ours.len(), egui.len());
        for (a, b) in ours.iter().zip(egui) {
            assert_eq!(a.clip_rect, b.clip_rect);
            match (&a.primitive, &b.primitive) {
                (Primitive::Mesh(a), Primitive::Mesh(b)) => {
                    assert_eq!(a.texture_id, b.texture_id);
                    assert_eq!(a.indices, b.indices);
                    assert_eq!(a.vertices, b.vertices);
                }
                _ => panic!("only meshes are drawn here"),
            }
        }
    }

    #[test]
    fn handed_over_frames_tessellate_as_the_context_does() {
        let ctx = egui::Context::default();
        let mut hand_over = EguiHandOver::default();
        // larger glyphs at each pass: the atlas grows under the discs
        let mut atlas_heights = Vec::new();
        for rows in [2, 12, 40] {
            let out = pass(&ctx, rows, 2048);
            atlas_heights.push(ctx.fonts(|f| f.font_image_size())[1]);
            let frame = hand_over.frame(&ctx, out.shapes.clone(), out.textures_delta, out.pixels_per_point);
            let ui = frame.tessellate();
            assert_eq!(ui.shapes, out.shapes.len());
            assert!(ui.vertices > 0);
            assert_same(&ui.primitives, &ctx.tessellate(out.shapes, out.pixels_per_point));
        }
        assert_eq!(atlas_heights[0], NEW_ATLAS_HEIGHT);
        assert!(atlas_heights[2] > NEW_ATLAS_HEIGHT, "{atlas_heights:?}");
    }

    #[test]
    fn small_circles_use_the_discs_of_the_atlas() {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.painter().circle_filled(egui::pos2(50.0, 50.0), 3.0, egui::Color32::WHITE);
        });
        out.textures_delta.clear();
        let circle: Vec<ClippedShape> = out
            .shapes
            .iter()
            .filter(|s| matches!(s.shape, egui::Shape::Circle(_)))
            .cloned()
            .collect();
        assert_eq!(circle.len(), 1);
        let ui = EguiHandOver::default()
            .frame(&ctx, circle.clone(), out.textures_delta, out.pixels_per_point)
            .tessellate();
        // one textured quad, not a fan of triangles
        assert_eq!(ui.vertices, 4);
        assert_same(&ui.primitives, &ctx.tessellate(circle, out.pixels_per_point));
    }

    /// A frame left with empty meshes only, after one with fewer meshes:
    /// egui-wgpu keeps the slices of the first and `render` runs out of them.
    #[test]
    fn a_frame_of_empty_meshes_renders_without_slices() {
        let _gpu = crate::test_gpu::lock();
        let instance = wgpu::Instance::default();
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
            return;
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).expect("GPU device");
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut egui = egui_wgpu::Renderer::new(&device, format, Default::default());
        let sd = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [64, 64],
            pixels_per_point: 1.0,
        };
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Empty egui frame test"),
            size: wgpu::Extent3d {
                width: 64,
                height: 64,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());
        let mut quad = egui::Mesh::default();
        quad.add_colored_rect(egui::Rect::from_min_size(egui::pos2(4.0, 4.0), egui::vec2(8.0, 8.0)), egui::Color32::RED);
        let drawn = [ClippedPrimitive {
            clip_rect: egui::Rect::EVERYTHING,
            primitive: Primitive::Mesh(quad),
        }];
        // what label_depth, the glass and the previews leave behind
        let emptied = vec![
            ClippedPrimitive {
                clip_rect: egui::Rect::NOTHING,
                primitive: Primitive::Mesh(egui::Mesh::default()),
            };
            3
        ];
        assert!(meshes_uploaded(&drawn));
        assert!(!meshes_uploaded(&emptied));
        let frame = |primitives: &[ClippedPrimitive], egui: &mut egui_wgpu::Renderer, filter: bool| {
            let mut encoder = device.create_command_encoder(&Default::default());
            let mut cmds = egui.update_buffers(&device, &queue, &mut encoder, primitives, &sd);
            let uploaded = meshes_uploaded(primitives);
            let rendered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut pass = encoder
                    .begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("egui"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &view,
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations::default(),
                        })],
                        ..Default::default()
                    })
                    .forget_lifetime();
                let jobs = if filter {
                    paint_jobs(primitives, uploaded)
                } else {
                    Cow::Borrowed(primitives)
                };
                egui.render(&mut pass, &jobs, &sd);
            }));
            cmds.push(encoder.finish());
            queue.submit(cmds);
            rendered.is_ok()
        };
        assert!(frame(&drawn, &mut egui, true));
        // the crash of #1235, then the same frames through paint_jobs
        assert!(!frame(&emptied, &mut egui, false));
        assert!(frame(&drawn, &mut egui, true));
        assert!(frame(&emptied, &mut egui, true));
        assert!(frame(&drawn, &mut egui, true));
    }

    #[test]
    fn discs_follow_an_atlas_made_anew() {
        let ctx = egui::Context::default();
        let mut hand_over = EguiHandOver::default();
        let out = pass(&ctx, 3, 2048);
        let _ = hand_over.frame(&ctx, out.shapes, out.textures_delta, out.pixels_per_point);
        assert_eq!(hand_over.twin.as_ref().map(|t| t.size()[0]), Some(2048));
        // another atlas width: the font atlas is made again, its discs with it
        let _ = pass(&ctx, 3, 4096);
        let out = pass(&ctx, 3, 4096);
        assert_eq!(ctx.fonts(|f| f.font_image_size())[0], 4096);
        let frame = hand_over.frame(&ctx, out.shapes.clone(), out.textures_delta, out.pixels_per_point);
        assert_eq!(hand_over.twin.as_ref().map(|t| t.size()[0]), Some(4096));
        assert_same(&frame.tessellate().primitives, &ctx.tessellate(out.shapes, out.pixels_per_point));
    }
}
