//! Tracks: a few choices or buttons in one sunken bar, the chosen cell lit
//! and sliding between cells (the poser's compact window: its groups,
//! transforms, sides and button clusters), with Apple-style help tags.
//! `segmented` is the plain-text form of `icon_track`.

use crate::theme::Palette;
use egui::Sense;

/// Room between a track's bar and its cells.
pub const TRACK_INSET: f32 = 2.0;

/// What a cell of a track shows.
pub enum TrackFace<'a> {
    Icon(&'a str),
    Letter(&'a str),
}

/// How a track shows its chosen cell: `Accent` fills it in the accent (the
/// groups, the window's one strong choice), `Quiet` raises it and lights
/// its glyph in the accent (the modes and sides), so that one violet block
/// leads the eye rather than four.
#[derive(Clone, Copy, PartialEq)]
pub enum TrackLit {
    Accent,
    Quiet,
}

/// A choice among cells in one track, painted as the theme's `segmented`;
/// `labels` puts a short word under each icon. The highlight slides to the
/// chosen cell and the hover fades in and out, both with egui's animation
/// time (none under Reduce animations). Returns the cell clicked, if it
/// was not the chosen one.
pub fn icon_track(
    ui: &mut egui::Ui,
    p: &Palette,
    id: &str,
    cell: egui::Vec2,
    chosen: usize,
    lit: TrackLit,
    cells: &[(TrackFace, &str, &str)],
) -> Option<usize> {
    let n = cells.len() as f32;
    let size = egui::vec2(cell.x * n + TRACK_INSET * 2.0, cell.y + TRACK_INSET * 2.0);
    let (bar, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(bar, ui.visuals().widgets.inactive.corner_radius, p.bar);
    let id = ui.scope_id().with(("poser-track", id));
    let cell_at = |i: f32| egui::Rect::from_min_size(egui::pos2(bar.left() + TRACK_INSET + cell.x * i, bar.top() + TRACK_INSET), cell);
    // the highlight, sliding from the cell chosen before
    let time = ui.style().animation_time;
    let slide = ui.ctx().animate_value_with_time(id, chosen as f32, time);
    let (face, chosen_ink) = match lit {
        TrackLit::Accent => (p.violet, p.on_accent),
        TrackLit::Quiet => (p.raised, p.violet_light),
    };
    ui.painter().rect_filled(cell_at(slide), super::inner_radius(ui.visuals()), face);
    let mut picked = None;
    for (i, (glyph_face, label, tip)) in cells.iter().enumerate() {
        let rect = cell_at(i as f32);
        let response = ui.interact(rect, id.with(i), Sense::click());
        let on = chosen == i;
        let hover = ui.ctx().animate_bool(id.with(("hover", i)), response.hovered() && !on);
        if hover > 0.0 {
            ui.painter()
                .rect_filled(rect, super::inner_radius(ui.visuals()), p.raised.gamma_multiply(0.5 * hover));
        }
        let ink = if on { chosen_ink } else { p.muted.lerp_to_gamma(p.ink, hover) };
        let glyph = if label.is_empty() {
            rect.center()
        } else {
            egui::pos2(rect.center().x, rect.top() + 14.0)
        };
        match glyph_face {
            TrackFace::Icon(name) => crate::ui::viewfinder::icon(
                ui.painter(),
                name,
                egui::Rect::from_center_size(glyph, egui::Vec2::splat(16.0)),
                ink,
            ),
            TrackFace::Letter(letter) => {
                ui.painter().text(
                    glyph,
                    egui::Align2::CENTER_CENTER,
                    letter,
                    egui::FontId::new(12.5, crate::ui::fonts::semibold(ui.ctx())),
                    ink,
                );
            }
        }
        if !label.is_empty() {
            let galley = ui
                .painter()
                .layout_no_wrap((*label).to_owned(), egui::FontId::proportional(10.5), ink);
            let left = rect.center().x - galley.size().x.min(rect.width() - 4.0) * 0.5;
            ui.painter().with_clip_rect(rect.shrink(2.0)).galley(
                egui::pos2(left, rect.bottom() - 9.0 - galley.size().y * 0.5),
                galley,
                ink,
            );
        }
        let name = if label.is_empty() { *tip } else { *label };
        response.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, true, on, name));
        if response.on_hover_text(*tip).clicked() && !on {
            picked = Some(i);
        }
    }
    picked
}

/// A track holding a few square buttons side by side (`track_button`), so that a
/// set (the view toggles, the files) reads as one cluster.
pub fn button_track<R>(ui: &mut egui::Ui, p: &Palette, count: usize, button: f32, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let size = egui::vec2(
        count as f32 * button + (count.max(1) - 1) as f32 * 2.0 + TRACK_INSET * 2.0,
        button + TRACK_INSET * 2.0,
    );
    let (bar, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(bar, ui.visuals().widgets.inactive.corner_radius, p.bar);
    let mut inner = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(bar.shrink(TRACK_INSET))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    inner.spacing_mut().item_spacing.x = 2.0;
    add(&mut inner)
}

/// A help tag (Apple's): the control's name in bold, then one short line
/// saying what it does.
pub fn help_tag(response: egui::Response, p: &Palette, title: &str, detail: &str) -> egui::Response {
    response.on_hover_ui(|ui| {
        ui.set_max_width(240.0);
        ui.label(
            egui::RichText::new(title)
                .font(egui::FontId::new(12.5, crate::ui::fonts::semibold(ui.ctx())))
                .color(p.ink),
        );
        if !detail.is_empty() {
            ui.label(egui::RichText::new(detail).size(11.5).color(p.muted));
        }
    })
}

/// A square button of a `button_track`: raised under the pointer, and while `on` raised
/// with its icon lit in the accent (a quiet "on", as the modes); its help
/// tag names it (`help`); its response anchors a menu.
pub fn track_button(
    ui: &mut egui::Ui,
    p: &Palette,
    size: egui::Vec2,
    icon: &str,
    (title, detail): (&str, &str),
    on: bool,
    enabled: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&response));
    let hover = ui
        .ctx()
        .animate_bool(response.id.with("hover"), enabled && (open || response.hovered()));
    let lift = if on { 1.0 } else { 0.5 * hover };
    if lift > 0.0 {
        ui.painter()
            .rect_filled(rect, super::inner_radius(ui.visuals()), p.raised.gamma_multiply(lift));
    }
    let ink = match (enabled, on) {
        (false, _) => p.muted_dim,
        (true, true) => p.violet_light,
        (true, false) => p.muted.lerp_to_gamma(p.ink, hover),
    };
    crate::ui::viewfinder::icon(
        ui.painter(),
        icon,
        egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(15.0)),
        ink,
    );
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, enabled, on, title));
    help_tag(response, p, title, detail)
}

/// A track holding one button as wide as `width`.
pub fn wide_button_track(ui: &mut egui::Ui, p: &Palette, width: f32, button: f32, add: impl FnOnce(&mut egui::Ui, egui::Vec2)) {
    let (bar, _) = ui.allocate_exact_size(egui::vec2(width, button + TRACK_INSET * 2.0), Sense::hover());
    ui.painter().rect_filled(bar, ui.visuals().widgets.inactive.corner_radius, p.bar);
    let mut inner = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(bar.shrink(TRACK_INSET))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    add(&mut inner, bar.shrink(TRACK_INSET).size());
}

/// The height of a `labelled_choice` row.
const HEADER_H: f32 = 24.0;
/// The least width of a header row's cell: "Change", with room.
const HEADER_CELL: f32 = 68.0;

/// An inspector row: its name on the left, a small two-way
/// choice on the right, as an Apple inspector row; both choices as wide as
/// the wider word. Returns the cell clicked, if it was not the chosen one.
pub fn labelled_choice(ui: &mut egui::Ui, p: &Palette, id: &str, label: &str, chosen: usize, choices: &[(&str, &str); 2]) -> Option<usize> {
    let mut picked = None;
    ui.horizontal(|ui| {
        ui.set_min_height(HEADER_H);
        ui.label(egui::RichText::new(label).size(12.0).color(p.muted));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let font = egui::FontId::new(12.5, crate::ui::fonts::semibold(ui.ctx()));
            let widest = choices
                .iter()
                .map(|(word, _)| ui.painter().layout_no_wrap((*word).to_owned(), font.clone(), p.ink).size().x)
                .fold(0.0, f32::max);
            // one width for every header row's cells, so that they line up
            let cell = egui::vec2((widest + 18.0).max(HEADER_CELL), HEADER_H - 2.0 * TRACK_INSET);
            let cells = choices.map(|(word, tip)| (TrackFace::Letter(word), "", tip));
            picked = icon_track(ui, p, id, cell, chosen, TrackLit::Quiet, &cells);
        });
    });
    picked
}

/// The face of a square button with its icon over a short label, painted
/// in `rect` (a button inside a row, which owns its own response):
/// `accent` for the main answer, greyed when not `enabled`.
#[allow(clippy::too_many_arguments)]
pub fn paint_tile_button(
    ui: &egui::Ui,
    p: &Palette,
    rect: egui::Rect,
    icon: &str,
    label: &str,
    accent: bool,
    enabled: bool,
    hovered: bool,
) {
    let face = match (accent, enabled, hovered) {
        (true, _, false) => p.violet,
        (true, _, true) => p.violet_light,
        (false, true, true) => p.raised,
        (false, true, false) => p.raised.gamma_multiply(0.7),
        (false, false, _) => p.bar,
    };
    ui.painter().rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, face);
    let ink = match (accent, enabled) {
        (true, _) => p.on_accent,
        (false, true) => p.ink,
        (false, false) => p.muted,
    };
    crate::ui::viewfinder::icon(
        ui.painter(),
        icon,
        egui::Rect::from_center_size(egui::pos2(rect.center().x, rect.top() + 11.0), egui::Vec2::splat(14.0)),
        ink,
    );
    let galley = ui.painter().layout_no_wrap(label.to_owned(), egui::FontId::proportional(10.5), ink);
    let x = rect.center().x - galley.size().x.min(rect.width() - 4.0) * 0.5;
    ui.painter()
        .with_clip_rect(rect.shrink(2.0))
        .galley(egui::pos2(x, rect.bottom() - 10.0 - galley.size().y * 0.5), galley, ink);
}
