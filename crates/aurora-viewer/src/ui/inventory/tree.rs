//! The pieces of the inventory tree that other trees of inventory reuse (the
//! Marketplace listings window): the caret and the type icon of a row, its
//! label, the list frame and its scrolling, the filter field and the outline
//! of a drop target. One look for every inventory tree.

use super::super::icons::Icons;
use crate::theme::Palette;
use egui::Vec2;

/// Row spacing of the tree (compact rows of 20 points).
pub fn spacing(ui: &mut egui::Ui) {
    super::super::widgets::inventory_spacing(ui);
}

/// The open / closed triangle before a folder; clicked to toggle it.
pub fn caret(ui: &mut egui::Ui, p: &Palette, open: bool) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(Vec2::splat(12.0), egui::Sense::click());
    let c = r.center();
    let pts = if open {
        vec![c + egui::vec2(-4.0, -2.0), c + egui::vec2(4.0, -2.0), c + egui::vec2(0.0, 3.0)]
    } else {
        vec![c + egui::vec2(-2.0, -4.0), c + egui::vec2(3.0, 0.0), c + egui::vec2(-2.0, 4.0)]
    };
    ui.painter().add(egui::Shape::convex_polygon(pts, p.muted, egui::Stroke::NONE));
    resp
}

/// The icon of a folder by its preferred type.
pub fn folder_icon(type_default: i32, open: bool) -> &'static str {
    match type_default {
        14 => "Inv_TrashClosed",
        // FT_RLV: Legacy Inv_RLVOpen / Inv_RLVClosed; Aurora has no RLV glyph
        // in the inventory atlas, so the shared root keeps a folder icon.
        60 if open => "Inv_FolderOpen",
        60 => "Inv_FolderClosed",
        _ if open => "Inv_FolderOpen",
        _ => "Inv_FolderClosed",
    }
}

/// The type icon of a row: folders in the light accent, items in ink, or
/// `tint` when given (a state the row shows by its icon).
pub fn icon(ui: &mut egui::Ui, icons: &Icons, name: &str, p: &Palette, tint: Option<egui::Color32>) {
    if let Some(t) = icons.get(name) {
        let tint = tint.unwrap_or(if name.starts_with("Inv_Folder") { p.violet_light } else { p.ink });
        ui.add(egui::Image::new(t).fit_to_exact_size(Vec2::splat(16.0)).tint(tint));
    } else {
        ui.add_space(16.0);
    }
}

/// The label of a row: the whole width, selected or not, in the row font.
pub fn label(
    ui: &mut egui::Ui,
    label: egui::text::LayoutJob,
    selected: bool,
    sense: egui::Sense,
    bold_name: Option<std::sync::Arc<egui::Galley>>,
) -> egui::Response {
    super::super::widgets::inventory_item_label(ui, label, selected, sense, bold_name)
}

/// A name in the row font, for `label`.
pub fn name(name: &str, p: &Palette) -> egui::text::LayoutJob {
    super::super::widgets::name_with_suffix(name, "", font(), p.ink)
}

pub fn font() -> egui::FontId {
    egui::FontId::proportional(13.0)
}

/// The frame the tree scrolls in.
pub fn frame<R>(ui: &mut egui::Ui, p: &Palette, body: impl FnOnce(&mut egui::Ui) -> R) -> egui::InnerResponse<R> {
    egui::Frame::new()
        .fill(p.field.gamma_multiply(0.72))
        .corner_radius(ui.visuals().window_corner_radius)
        .inner_margin(4)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            body(ui)
        })
}

pub fn scroll(ui: &mut egui::Ui, height: f32) -> egui::ScrollArea {
    // (egui's own salt: the area keeps its saved offset)
    crate::ui::widgets::scroll_offset::show(ui, egui::ScrollArea::both(), "scroll_area")
        .auto_shrink([false, false])
        .max_width(ui.available_width())
        .max_height(height)
        .min_scrolled_height((height - ui.spacing().scroll.allocated_width()).max(40.0))
}

/// The height left to the list above the one-line footer.
pub fn height(ui: &egui::Ui) -> f32 {
    // One fixed-height footer, including the frame margins and layout spacing.
    // Async messages share this row so fetching never enlarges the floater.
    (ui.available_height() - 58.0).max(40.0)
}

/// The filter field (LLSearchEditor, indra/llui/llsearcheditor.cpp): the
/// clear action inside the field, its text padding kept even before typing.
/// Returns the field and whether the clear action was clicked.
pub fn search_field(
    ui: &mut egui::Ui,
    p: &Palette,
    text: &mut String,
    hint: &str,
    width: f32,
    clear_shown: bool,
    clear_tip: &str,
) -> (egui::Response, bool) {
    let centre = crate::ui::widgets::field_text_margin(ui.ctx(), &egui::TextStyle::Body.resolve(ui.style()));
    let field = ui.add_sized(
        [width, ui.spacing().interact_size.y],
        egui::TextEdit::singleline(text)
            .hint_text(hint)
            .align(egui::Align2::LEFT_CENTER)
            .margin(egui::Margin {
                left: 5,
                right: 28,
                ..centre
            }),
    );
    if !clear_shown {
        return (field, false);
    }
    let rect = egui::Rect::from_center_size(field.rect.right_center() - Vec2::new(14.0, 0.0), Vec2::splat(20.0));
    let clear = ui
        .interact(rect, field.id.with("clear"), egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(clear_tip);
    clear.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), clear_tip));
    let color = if clear.hovered() { p.ink } else { p.muted };
    if let Some(icon) = crate::ui::icons::global("x") {
        ui.painter().image(
            icon.id(),
            egui::Rect::from_center_size(rect.center(), Vec2::splat(12.0)),
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            color,
        );
    } else {
        ui.painter().line_segment(
            [rect.center() - Vec2::splat(3.0), rect.center() + Vec2::splat(3.0)],
            egui::Stroke::new(1.0, color),
        );
        ui.painter().line_segment(
            [rect.center() + Vec2::new(-3.0, 3.0), rect.center() + Vec2::new(3.0, -3.0)],
            egui::Stroke::new(1.0, color),
        );
    }
    let clicked = clear.clicked();
    if clicked {
        field.request_focus();
    }
    (field, clicked)
}

/// The outline of a row or a list that cargo hovers: the light accent when
/// it takes the drop, the danger colour when it refuses it.
pub fn drop_outline(ui: &egui::Ui, p: &Palette, rect: egui::Rect, allowed: bool) {
    let color = if allowed { p.violet_light } else { p.danger };
    ui.painter().rect_stroke(
        rect,
        ui.visuals().window_corner_radius,
        egui::Stroke::new(1.0, color),
        egui::StrokeKind::Inside,
    );
}

/// The footer line of a tree window: small text, a message colour.
pub fn status_text(ui: &mut egui::Ui, text: &str, color: egui::Color32) -> egui::Response {
    ui.add(egui::Label::new(egui::RichText::new(text).size(11.0).color(color)).truncate())
}
