//! Baseline placement of llSetText, for world objects and HUD attachments.
//! Behaviour of LLHUDText::addLine/updateSize/renderText and hud_render_text
//! (indra/newview/llhudtext.cpp, llhudrender.cpp, originally LGPL 2.1).

use egui::emath::GuiRounding as _;
use egui::epaint::text::LayoutJob;
use egui::{Color32, FontId, Pos2, Rect};
use std::sync::Arc;

const MAX_WIDTH: f32 = 1000.0;
const VERTICAL_PADDING: f32 = 12.0;

fn normalize(text: String) -> String {
    if text.contains('\r') || text.contains("\n\n") || text.starts_with('\n') || text.ends_with('\n') {
        // boost::char_separator in LLHUDText drops empty CR/LF tokens, but
        // a line containing spaces is a real segment and must keep its row.
        text.split(['\r', '\n'])
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        text
    }
}

/// `projection_scale` affects the positions of HUD text, not its glyphs:
/// get_hud_matrices scales the model, then hud_render_text paints in 2D.
/// `background`: a card of that color behind the lines
/// (FSHudTextShowBackground, `hud::floating_text_background`).
pub(super) fn paint(
    painter: &egui::Painter,
    text: String,
    anchor: Pos2,
    size: f32,
    color: Color32,
    projection_scale: f32,
    background: Option<Color32>,
) {
    let text = normalize(text);
    if text.is_empty() || color.a() < 3 {
        return;
    }
    let metrics = super::fonts::baseline_metrics(painter.ctx(), size);
    let mut job = LayoutJob::simple(text, FontId::proportional(size), color, MAX_WIDTH);
    job.halign = egui::Align::Center;
    job.sections[0].format.line_height = Some(metrics.advance * projection_scale);
    let mut galley = super::bidi::galley(painter.ctx(), job, super::bidi::Base::Auto);
    // Only row placements are copied. The cached row meshes/glyphs stay shared.
    let layout = Arc::make_mut(&mut galley);
    layout.rect = Rect::NOTHING;
    layout.mesh_bounds = Rect::NOTHING;
    for (i, row) in layout.rows.iter_mut().enumerate() {
        // LLHUDText centers each row, then hud_render_text floors its offset
        // before projecting it. This also preserves HUD edit-zoom behaviour.
        row.pos.x = row.pos.x.floor() * projection_scale;
        // egui rounds row heights to physical pixels. Set each type
        // baseline explicitly: at fractional DPI that rounding would add
        // a cumulative spacing error (e.g. 29 -> 29.333 UI units at 150%).
        row.pos.y = i as f32 * metrics.advance * projection_scale - row.glyphs.first().map_or(0.0, |glyph| glyph.pos.y) - metrics.offset;
    }
    let Some(last) = galley.rows.last() else { return };
    let baseline = last.pos.y + last.glyphs.first().map_or(0.0, |glyph| glyph.pos.y) + metrics.offset;
    // updateSize adds 12 to the segment heights; ALIGN_VERT_TOP starts at
    // that height and renderText subtracts every segment, leaving 12 on the
    // last baseline. hud_render_text then calls font.render at baseline y=1.
    let ideal_origin = Pos2::new(anchor.x, anchor.y - VERTICAL_PADDING * projection_scale - 1.0 - baseline);
    let ppp = painter.ctx().pixels_per_point();
    let origin = ideal_origin.round_to_pixels(ppp);
    let layout = Arc::make_mut(&mut galley);
    for row in &mut layout.rows {
        // egui only snaps the galley's origin at tessellation; cached glyph
        // quads are already pixel-aligned. Snap each absolute row placement
        // once, then express it relative to that snapped origin. Otherwise
        // removing the optical font offset blurs the atlas between pixels.
        // Rounding absolute positions keeps error below half a pixel per
        // row without accumulating it in the reference line advance.
        row.pos = ((ideal_origin + row.pos.to_vec2()).round_to_pixels(ppp) - origin).to_pos2();
        layout.rect = layout.rect.union(row.rect());
        layout.mesh_bounds = layout.mesh_bounds.union(row.visuals.mesh_bounds.translate(row.pos.to_vec2()));
    }
    if let Some(fill) = background.filter(|c| c.a() > 0) {
        // the lines' own box, HORIZONTAL_PADDING (16) wider, as
        // LLHUDText::updateSize measures it; the theme's card radius
        let rect = layout
            .rect
            .translate(origin.to_vec2())
            .expand2(egui::vec2(8.0, 2.0) * projection_scale);
        painter.rect_filled(rect, painter.ctx().global_style().visuals.window_corner_radius, fill);
    }
    let [r, g, b, a] = color.to_srgba_unmultiplied();
    let luminance = (r.max(g).max(b) as f32 + r.min(g).min(b) as f32) / 510.0;
    let shadow = ((luminance - 0.35) / 0.25).clamp(0.0, 1.0);
    if shadow > 0.0 {
        let pixel = 1.0 / ppp;
        painter.galley_with_override_text_color(
            origin + egui::vec2(pixel, pixel),
            galley.clone(),
            Color32::from_black_alpha((a as f32 * shadow) as u8),
        );
    }
    painter.galley(origin, galley, color);
}

#[cfg(test)]
#[path = "floating_text_tests.rs"]
mod tests;
