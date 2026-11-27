//! The AO window's preview column: the open slot's entries one at a time,
//! with the layers that play on top, and a bar of segments to go through
//! them. The avatar itself is drawn on the stage above the caption.
use super::*;

/// Height of the caption under the stage.
const CAPTION: f32 = 136.0;

/// Which entry of the open slot the preview shows, and whether it plays.
#[derive(Default)]
pub(super) struct Preview {
    index: usize,
    paused: bool,
}

impl Preview {
    /// Shows that entry of the main list, playing.
    pub(super) fn show_entry(&mut self, index: usize) {
        self.index = index;
        self.paused = false;
    }
}

/// What the preview shows of an entry: its name, a group's members joined.
fn entry_names(set: &Set, state: State) -> Vec<String> {
    set.states.get(&state).map_or_else(Vec::new, |c| {
        c.steps
            .iter()
            .map(|s| {
                if s.group {
                    s.members.iter().map(|m| m.name.as_str()).collect::<Vec<_>>().join(" + ")
                } else {
                    s.name.clone()
                }
            })
            .collect()
    })
}

pub(super) fn show(ui: &mut egui::Ui, p: &Palette, set: Option<&Set>, runtime: &Runtime, state: &mut AoUi) {
    let rect = ui.max_rect();
    let names = set.map_or_else(Vec::new, |s| entry_names(s, state.state));
    if !names.is_empty() {
        state.preview.index = state.preview.index.min(names.len() - 1);
    }
    let header = egui::Rect::from_min_size(rect.min, Vec2::new(rect.width(), 40.0));
    widgets::paint_glyph(
        ui.painter(),
        "eye-filled",
        egui::pos2(header.left() + 22.0, header.center().y),
        15.0,
        p.muted,
    );
    widgets::paint_elided(
        ui,
        aurora_i18n::tr!("ao-preview"),
        egui::pos2(header.left() + 36.0, header.center().y),
        header.width() - 50.0,
        egui::FontId::new(13.0, super::super::fonts::semibold(ui.ctx())),
        p.ink,
    );
    let caption = egui::Rect::from_min_max(egui::pos2(rect.left(), rect.bottom() - CAPTION), rect.max);
    let stage = egui::Rect::from_min_max(egui::pos2(rect.left(), header.bottom()), egui::pos2(rect.right(), caption.top()));
    stage_placeholder(ui, p, stage);
    if names.len() > 1 {
        for (forward, icon, x) in [
            (false, "caret-left-filled", stage.left() + 24.0),
            (true, "caret-right-filled", stage.right() - 24.0),
        ] {
            let edge = egui::Rect::from_center_size(egui::pos2(x, stage.center().y), Vec2::new(36.0, 64.0));
            let tip = aurora_i18n::text(if forward { "ao-preview-next" } else { "ao-preview-previous" });
            let response = ui
                .interact(edge, ui.scope_id().with(("ao_preview_edge", forward)), egui::Sense::click())
                .on_hover_text(tip);
            let ink = if response.hovered() { p.ink } else { p.muted.gamma_multiply(0.6) };
            widgets::paint_glyph(ui.painter(), icon, edge.center(), 22.0, ink);
            if response.clicked() {
                let n = names.len();
                state.preview.index = if forward {
                    (state.preview.index + 1) % n
                } else {
                    (state.preview.index + n - 1) % n
                };
            }
        }
    }
    caption_text(ui, p, set, runtime, state, &names, caption.shrink2(Vec2::new(20.0, 16.0)));
}

/// The stage's floor until the avatar is drawn on it.
fn stage_placeholder(ui: &egui::Ui, p: &Palette, stage: egui::Rect) {
    let floor = egui::pos2(stage.center().x, stage.bottom() - 18.0);
    ui.painter().add(egui::Shape::ellipse_filled(
        floor,
        Vec2::new(110.0, 16.0),
        p.violet.gamma_multiply(0.18),
    ));
    let galley = widgets::elided_galley(
        ui,
        RichText::new(aurora_i18n::tr!("ao-preview-stage")).size(12.0).color(p.muted),
        stage.width() - 60.0,
    );
    widgets::paint_ink_centered(ui.painter(), galley, stage.center(), p.muted);
}

/// The slot in small capitals with the entry's place, the entry's name in
/// large type with its play button, the layers on top, and the segments.
fn caption_text(
    ui: &mut egui::Ui,
    p: &Palette,
    set: Option<&Set>,
    runtime: &Runtime,
    state: &mut AoUi,
    names: &[String],
    area: egui::Rect,
) {
    let mut y = area.top() + 8.0;
    let eyebrow = egui::FontId::new(11.0, super::super::fonts::semibold(ui.ctx()));
    let slot = ui
        .painter()
        .layout_no_wrap(state.state.label().to_uppercase(), eyebrow, p.violet_pale);
    let slot_w = slot.size().x;
    widgets::paint_row_text(ui.painter(), slot, area.left(), y, p.violet_pale);
    let place = if names.is_empty() {
        aurora_i18n::tr!("ao-slot-default").to_owned()
    } else {
        aurora_i18n::tr!("ao-preview-place", index = state.preview.index + 1, count = names.len())
    };
    widgets::paint_elided(
        ui,
        &place,
        egui::pos2(area.left() + slot_w + 10.0, y),
        area.width() - slot_w - 10.0,
        egui::FontId::proportional(11.0),
        p.muted,
    );
    y += 26.0;
    let hero = names
        .get(state.preview.index)
        .cloned()
        .unwrap_or_else(|| aurora_i18n::tr!("ao-preview-none").to_owned());
    let button = 26.0;
    widgets::paint_elided(
        ui,
        &hero,
        egui::pos2(area.left(), y),
        area.width() - button - 8.0,
        egui::FontId::new(20.0, super::super::fonts::semibold(ui.ctx())),
        p.ink,
    );
    if !names.is_empty() {
        let toggle = egui::Rect::from_center_size(egui::pos2(area.right() - button / 2.0, y), Vec2::splat(button));
        let (icon, tip) = if state.preview.paused {
            ("player-play-filled", aurora_i18n::tr!("ao-preview-play"))
        } else {
            ("player-pause-filled", aurora_i18n::tr!("ao-preview-pause"))
        };
        let response = ui
            .interact(toggle, ui.scope_id().with("ao_preview_toggle"), egui::Sense::click())
            .on_hover_text(tip);
        if response.hovered() {
            ui.painter()
                .rect_filled(toggle, ui.visuals().widgets.inactive.corner_radius, p.raised);
        }
        widgets::paint_glyph(ui.painter(), icon, toggle.center(), 14.0, p.ink);
        if response.clicked() {
            state.preview.paused = !state.preview.paused;
        }
    }
    y += 22.0;
    if let Some(config) = set.and_then(|s| s.states.get(&state.state)) {
        for track in config.tracks.iter().take(2) {
            let index = runtime.index(state.state, track.id).min(track.members.len().saturating_sub(1));
            let member = track.members.get(index).map_or("", |m| m.name.as_str());
            let line = format!("+ {}: {member}", track.name);
            widgets::paint_elided(
                ui,
                &line,
                egui::pos2(area.left(), y),
                area.width(),
                egui::FontId::proportional(12.0),
                p.muted,
            );
            y += 18.0;
        }
    }
    if names.len() > 1 {
        let n = names.len() as f32;
        let gap = 4.0;
        let width = (area.width() - gap * (n - 1.0)) / n;
        let top = area.bottom() - 14.0;
        for (i, name) in names.iter().enumerate() {
            let hit = egui::Rect::from_min_size(egui::pos2(area.left() + i as f32 * (width + gap), top), Vec2::new(width, 14.0));
            let response = ui
                .interact(hit, ui.scope_id().with(("ao_preview_segment", i)), egui::Sense::click())
                .on_hover_text(name);
            let height = if response.hovered() { 5.0 } else { 3.0 };
            let bar = egui::Rect::from_center_size(hit.center(), Vec2::new(width, height));
            let fill = match i.cmp(&state.preview.index) {
                std::cmp::Ordering::Equal => p.violet_pale,
                std::cmp::Ordering::Less => p.violet_pale.gamma_multiply(0.42),
                std::cmp::Ordering::Greater => p.ink.gamma_multiply(0.14),
            };
            ui.painter().rect_filled(bar, ui.visuals().widgets.inactive.corner_radius, fill);
            if response.clicked() {
                state.preview.show_entry(i);
            }
        }
    }
}
