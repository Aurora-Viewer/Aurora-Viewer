//! What the interface paints for the world (avatar name tags, floating
//! texts, the tracking beacon) lies under everything else on screen: the
//! viewer paints it first and ends it with a marker; the renderer draws what
//! precedes the marker on the scene, then the HUD attachments, then the rest
//! of the interface.
//!
//! Firestorm draws the name tags over the HUD attachments (render_ui in
//! indra/newview/llviewerdisplay.cpp: render_hud_attachments, then
//! LLHUDObject::renderAll): a name can cover the button of a HUD. This order
//! is an explicit Aurora choice, requested by the user.

use std::sync::Arc;

struct End;

/// Place this after the last shape painted for the world, before anything
/// else is painted on the background layer. The marker survives
/// tessellation, so even meshes with the same texture cannot straddle it.
pub fn end_world_labels(ctx: &egui::Context) {
    ctx.layer_painter(egui::LayerId::background()).add(egui::PaintCallback {
        rect: ctx.content_rect(),
        callback: Arc::new(End),
    });
}

/// How many of the first `primitives` were painted for the world; 0 without
/// a marker, which is removed.
pub(crate) fn take_end(primitives: &mut Vec<egui::ClippedPrimitive>) -> usize {
    let at = primitives
        .iter()
        .position(|p| matches!(&p.primitive, egui::epaint::Primitive::Callback(c) if c.callback.is::<End>()));
    match at {
        Some(at) => {
            primitives.remove(at);
            at
        }
        None => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::Color32;

    fn colors(jobs: &[egui::ClippedPrimitive]) -> Vec<Color32> {
        jobs.iter()
            .flat_map(|p| match &p.primitive {
                egui::epaint::Primitive::Mesh(mesh) => mesh.vertices.iter().map(|v| v.color).collect::<Vec<_>>(),
                _ => Vec::new(),
            })
            .collect()
    }

    fn square(x: f32) -> egui::Rect {
        egui::Rect::from_min_size(egui::pos2(x, 0.0), egui::vec2(20.0, 20.0))
    }

    /// A square of `color` laid out in `ui`.
    fn fill(ui: &mut egui::Ui, color: Color32) {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::hover());
        ui.painter().rect_filled(rect, 0.0, color);
    }

    #[test]
    fn what_precedes_the_marker_is_under_the_bars_and_every_other_layer() {
        let ctx = egui::Context::default();
        let frame = || {
            let mut output = ctx.run_ui(Default::default(), |ui| {
                ctx.layer_painter(egui::LayerId::background())
                    .rect_filled(square(0.0), 0.0, Color32::RED);
                end_world_labels(&ctx);
                // a bar of the background layer, a panel laid on the world and a window
                egui::Panel::top("bar").show(ui, |ui| fill(ui, Color32::GREEN));
                egui::Area::new(egui::Id::unique("toast"))
                    .order(egui::Order::Background)
                    .fade_in(false)
                    .show(&ctx, |ui| fill(ui, Color32::BLUE));
                egui::Area::new(egui::Id::unique("window"))
                    .order(egui::Order::Middle)
                    .fade_in(false)
                    .show(&ctx, |ui| fill(ui, Color32::YELLOW));
            });
            // No GPU renderer in this test: discard texture uploads explicitly.
            output.textures_delta.clear();
            output
        };
        // an area only measures itself in its first frame
        frame();
        let output = frame();
        let mut primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
        let end = take_end(&mut primitives);
        let (world, over) = (colors(&primitives[..end]), colors(&primitives[end..]));
        assert!(world.contains(&Color32::RED));
        assert!(!over.contains(&Color32::RED));
        for color in [Color32::GREEN, Color32::BLUE, Color32::YELLOW] {
            assert!(over.contains(&color), "{color:?} is drawn after the world's labels");
            assert!(!world.contains(&color), "{color:?} is not drawn with the world's labels");
        }
        assert_eq!(take_end(&mut primitives), 0, "the marker is gone");
    }

    #[test]
    fn the_modal_blur_boundary_stays_after_the_marker() {
        let ctx = egui::Context::default();
        let modal = egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("modal"));
        let mut output = ctx.run_ui(Default::default(), |_| {
            crate::blurred_backdrop(&ctx, modal);
            ctx.layer_painter(modal).rect_filled(square(40.0), 0.0, Color32::GREEN);
            let world = ctx.layer_painter(egui::LayerId::background());
            world.rect_filled(square(0.0), 0.0, Color32::RED);
            end_world_labels(&ctx);
            world.rect_filled(square(20.0), 0.0, Color32::BLUE);
        });
        output.textures_delta.clear();
        let mut primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
        // as the renderer does: the end of the world's labels, then the boundary
        let end = take_end(&mut primitives);
        let boundary = crate::ui_blur::take_boundary(&mut primitives).expect("blur marker survives tessellation");
        assert!(end > 0 && end < boundary);
        let parts = [&primitives[..end], &primitives[end..boundary], &primitives[boundary..]];
        for (part, color) in parts.into_iter().zip([Color32::RED, Color32::BLUE, Color32::GREEN]) {
            let found = colors(part);
            for other in [Color32::RED, Color32::BLUE, Color32::GREEN] {
                assert_eq!(found.contains(&other), other == color, "{other:?} among the {color:?} part");
            }
        }
    }
}
