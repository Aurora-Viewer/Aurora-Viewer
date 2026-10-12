//! Shared hierarchy for floaters: navigation, inset surfaces and summaries.

use super::*;
use egui::AtomExt;

/// The user tree and object contents share row density and interaction styling.
pub(crate) fn inventory_spacing(ui: &mut egui::Ui) {
    ui.spacing_mut().item_spacing.y = 1.0;
    ui.spacing_mut().button_padding = egui::vec2(2.0, 1.0);
    ui.spacing_mut().interact_size.y = 20.0;
}

pub(crate) fn inventory_item_label(
    ui: &mut egui::Ui,
    label: egui::text::LayoutJob,
    selected: bool,
    sense: Sense,
    bold_name: Option<std::sync::Arc<egui::Galley>>,
) -> egui::Response {
    inventory_spacing(ui);
    let text_id = ui.make_persistent_id("label");
    ui.add_sized([ui.available_width(), 20.0], |ui: &mut egui::Ui| {
        let response = egui::Button::selectable(selected, "")
            .left_text(egui::Atom::from(label).atom_id(text_id))
            .wrap_mode(egui::TextWrapMode::Extend)
            .sense(sense)
            .atom_ui(ui);
        // LLItemBridge::getLabelStyle: only the worn name is bold.
        if let Some((name, rect)) = bold_name.zip(response.rect(text_id)) {
            let color = ui.visuals().text_color();
            ui.painter().galley(rect.min + bold_text_offset(ui), name, color);
        }
        response.response
    })
}

pub fn glyph(ui: &mut egui::Ui, name: &str, color: Color32, size: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    if let Some(icon) = crate::ui::icons::global(name) {
        ui.painter().image(
            icon.id(),
            rect,
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            color,
        );
    }
}

pub fn navigation(ui: &mut egui::Ui, p: &Palette, icon: &str, label: &str, selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 34.0), Sense::click());
    if selected || response.hovered() {
        ui.painter()
            .rect_filled(rect, 5.0, if selected { p.violet.gamma_multiply(0.24) } else { p.raised });
    }
    if selected {
        ui.painter().vline(
            rect.left() + 1.0,
            rect.top() + 6.0..=rect.bottom() - 6.0,
            Stroke::new(2.0, p.violet),
        );
    }
    if let Some(icon) = crate::ui::icons::global(icon) {
        ui.painter().image(
            icon.id(),
            egui::Rect::from_center_size(rect.left_center() + egui::vec2(17.0, 0.0), Vec2::splat(17.0)),
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            if selected { p.violet_pale } else { p.muted },
        );
    }
    let galley = egui::WidgetText::from(label).into_galley(
        ui,
        Some(egui::TextWrapMode::Truncate),
        (rect.width() - 42.0).max(1.0),
        egui::FontId::proportional(12.0),
    );
    ui.painter().galley(
        rect.left_center() + egui::vec2(34.0, -galley.size().y / 2.0),
        galley,
        if selected { p.ink } else { p.muted },
    );
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, label));
    response.on_hover_text(label)
}

pub fn surface<R>(ui: &mut egui::Ui, p: &Palette, body: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::new()
        .fill(p.field.gamma_multiply(0.8))
        .stroke(Stroke::new(1.0, p.raised))
        .corner_radius(3)
        .inner_margin(10)
        .show(ui, body)
        .inner
}

pub fn section_header(ui: &mut egui::Ui, p: &Palette, icon: &str, title: &str) {
    ui.horizontal(|ui| {
        glyph(ui, icon, p.violet_light, 17.0);
        ui.label(RichText::new(title).size(13.0).strong().color(p.ink));
    });
    ui.add_space(4.0);
}

pub fn stat(ui: &mut egui::Ui, p: &Palette, label: &str, value: &str) {
    ui.vertical(|ui| {
        ui.label(RichText::new(label).size(11.0).color(p.muted));
        ui.add(egui::Label::new(RichText::new(value).size(15.0).strong().color(p.ink)).truncate())
            .on_hover_text(value);
    });
}

pub(super) fn window_icon(id: &str) -> &'static str {
    if id.starts_with("inventory") {
        return "folder";
    }
    if id.starts_with("profile") {
        return "user";
    }
    if id.contains("texture") || id.contains("thumbnail") {
        return "image";
    }
    if id.contains("script") {
        return "code";
    }
    if id.contains("place") || id.contains("land") {
        return "map-pin";
    }
    match id {
        "appearance" => "t-shirt",
        "conversations" | "contacts" => "chats-circle",
        "people" | "avatar_picker" => "users-three",
        "options" => "gear-six",
        "worldmap" | "minimap" => "map-trifold",
        "environment_selector" | "personal_lighting" => "sun",
        "performance" => "chart-bar",
        "ao" => "person",
        "asset_imports_v3" => "upload-simple",
        _ => "squares-four",
    }
}

/// Reserve a drawing slot before the content, then resolve its exact bounds
/// after layout. The opaque skin remains available with panel_alpha = 255.
pub(crate) fn finish_glass(painter: &egui::Painter, slot: egui::layers::ShapeIdx, rect: egui::Rect, p: &Palette, radius: f32) {
    if p.panel_alpha < 255 {
        painter.set(
            slot,
            aurora_render::glass_panel(
                rect.shrink(1.0),
                radius,
                Color32::from_rgba_unmultiplied(p.panel.r(), p.panel.g(), p.panel.b(), p.panel_alpha),
            ),
        );
    }
}

/// Modal dialogs use the same surface and spacing as regular windows.
pub struct Modal(egui::Modal);

impl Modal {
    pub fn new(id: egui::Id) -> Self {
        Self(egui::Modal::new(id))
    }
    pub fn show<T>(self, ctx: &egui::Context, body: impl FnOnce(&mut egui::Ui) -> T) -> egui::ModalResponse<T> {
        // Theme supplies skin colors and radii; per-modal frames remain local.
        let mut glass_slot = None;
        let response = self
            .0
            .frame(
                egui::Frame::popup(&ctx.global_style())
                    .inner_margin(18)
                    .corner_radius(ctx.global_style().visuals.window_corner_radius),
            )
            .show(ctx, |ui| {
                glass_slot = Some((ui.painter().clone(), ui.painter().add(egui::Shape::Noop)));
                body(ui)
            });
        if let Some((painter, slot)) = glass_slot {
            let style = ctx.global_style();
            if style.visuals.window_fill.a() < 255 {
                painter.set(
                    slot,
                    aurora_render::glass_panel(
                        response.response.rect.shrink(1.0),
                        f32::from(style.visuals.window_corner_radius.nw),
                        style.visuals.window_fill,
                    ),
                );
            }
        }
        response
    }
}
