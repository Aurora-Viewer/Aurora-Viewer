//! The AO window's preview column: the open slot's entries one at a time,
//! with the layers that play on top, and a bar of segments to go through
//! them. Above the caption, our outfit on the 3D stage of Inspect
//! (`ui::object_stage`), posed by the preview's skeleton playing the entry
//! (world/ao_preview.rs, app/ao_preview.rs): turned by a drag, paused and
//! played from its own bar.
use super::*;

/// The room the stage keeps at its bottom for its own bar of buttons
/// (`ui::object_stage`), under the caption laid over it.
const STAGE_BAR: f32 = 44.0;
/// The caption's lines: the slot, the entry's name, each layer, the
/// segments.
const EYEBROW: f32 = 15.0;
const HERO: f32 = 24.0;
const LAYER: f32 = 16.0;
const SEGMENTS: f32 = 12.0;
/// How far above the caption the fade to the stage's ground starts.
const FADE: f32 = 36.0;
/// How blurred the picture's bottom row is under the fade: each step halves
/// it (`aurora_render::preview::Veil`).
const VEIL_DEPTH: f32 = 4.0;

/// Which entry of the open slot the preview shows.
#[derive(Default)]
pub(super) struct Preview {
    index: usize,
    /// A layer member previewed over the main entry (the layer, its index),
    /// in place of the one the layer plays in the world.
    layer: Option<(Uuid, usize)>,
    /// The stage was drawn this frame (taken by the app, `AoUi::take_stage`).
    shown: bool,
}

impl Preview {
    /// Shows that entry of the main list, each layer as it plays.
    pub(super) fn show_entry(&mut self, index: usize) {
        self.index = index;
        self.layer = None;
    }

    /// Shows that member of a layer over the main entry.
    pub(super) fn show_layer(&mut self, layer: Uuid, index: usize) {
        self.layer = Some((layer, index));
    }

    /// Whether that entry is previewed: of the main list (`layer` None), or
    /// of a layer.
    pub(super) fn previews(&self, layer: Option<Uuid>, index: usize) -> bool {
        match layer {
            None => self.index == index,
            Some(id) => self.layer == Some((id, index)),
        }
    }
}

impl AoUi {
    /// Whether the window drew its stage this frame: the app keeps the
    /// skeleton and the stage's model only then.
    pub fn take_stage(&mut self) -> bool {
        std::mem::take(&mut self.preview.shown)
    }

    /// The animations the skeleton plays: the previewed entry's (all members
    /// of a stack) with the entry each layer plays now; the slot's stock
    /// animation while it is empty.
    pub fn preview_assets(&self, set: Option<&Set>, runtime: &Runtime) -> Vec<Uuid> {
        let state = self.state;
        let config = set.and_then(|s| s.states.get(&state));
        let (Some(config), Some(entry)) = (config, config.and_then(|c| c.steps.get(self.preview.index))) else {
            return vec![state.motion()];
        };
        let mut assets: Vec<Uuid> = entry.members.iter().map(|m| m.asset).collect();
        for track in &config.tracks {
            let index = match self.preview.layer {
                Some((id, picked)) if id == track.id => picked,
                _ => runtime.index(state, track.id),
            };
            let index = index.min(track.members.len().saturating_sub(1));
            assets.extend(track.members.get(index).map(|m| m.asset));
        }
        assets.retain(|a| !a.is_nil());
        assets
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
    // the stage fills the column edge to edge; the title and the caption lie
    // over it, the caption on a fade to its ground
    let rect = ui.max_rect();
    let names = set.map_or_else(Vec::new, |s| entry_names(s, state.state));
    if !names.is_empty() {
        state.preview.index = state.preview.index.min(names.len() - 1);
    }
    state.preview.shown = true;
    let ground = state.stage.ground(p);
    let (ink, muted) = (state.stage.ink(p), state.stage.muted(p));
    ui.painter().rect_filled(rect, egui::CornerRadius::ZERO, ground);
    let title_room = 40.0;
    let layers = set.and_then(|s| s.states.get(&state.state)).map_or(0, |c| c.tracks.len().min(2));
    let segments = if names.len() > 1 { SEGMENTS + 6.0 } else { 0.0 };
    let height = EYEBROW + HERO + layers as f32 * LAYER + segments;
    let bottom = rect.bottom() - STAGE_BAR;
    let caption = egui::Rect::from_min_max(
        egui::pos2(rect.left() + 18.0, bottom - height),
        egui::pos2(rect.right() - 18.0, bottom),
    );
    // the picture blurred under the fade, more and more toward the bottom
    let from = (caption.top() - FADE - rect.top()) / rect.height().max(1.0);
    state.stage.veil = Some(aurora_render::preview::Veil { from, depth: VEIL_DEPTH });
    let spoken = aurora_i18n::tr!("ao-preview");
    let _ = state.stage.show(ui, p, rect, title_room, Default::default(), spoken, &|_| None);
    let y = rect.top() + title_room / 2.0;
    widgets::paint_glyph(ui.painter(), "eye-filled", egui::pos2(rect.left() + 22.0, y), 15.0, muted);
    widgets::paint_elided(
        ui,
        aurora_i18n::tr!("ao-preview"),
        egui::pos2(rect.left() + 36.0, y),
        rect.width() - 50.0,
        egui::FontId::new(13.0, super::super::fonts::semibold(ui.ctx())),
        ink,
    );
    fade(
        ui,
        egui::Rect::from_min_max(
            egui::pos2(rect.left(), caption.top() - FADE),
            // (to the panel's bottom edge, under the stage's buttons too)
            rect.right_bottom(),
        ),
        ground,
    );
    if names.len() > 1 {
        let middle = (rect.top() + title_room + caption.top()) / 2.0;
        for (forward, icon, x) in [
            (false, "caret-left-filled", rect.left() + 20.0),
            (true, "caret-right-filled", rect.right() - 20.0),
        ] {
            let edge = egui::Rect::from_center_size(egui::pos2(x, middle), Vec2::new(36.0, 64.0));
            let tip = aurora_i18n::text(if forward { "ao-preview-next" } else { "ao-preview-previous" });
            let response = ui
                .interact(edge, ui.scope_id().with(("ao_preview_edge", forward)), egui::Sense::click())
                .on_hover_text(tip);
            let tint = if response.hovered() { ink } else { muted.gamma_multiply(0.7) };
            widgets::paint_glyph(ui.painter(), icon, edge.center(), 22.0, tint);
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
    caption_text(ui, p, set, runtime, state, &names, caption, (ink, muted));
    // the stage's buttons over the fade
    state.stage.show_bar(ui, p, rect);
}

/// How long "Set saved" stays, and how long it takes to come and go.
const SAVED_FOR: f32 = 2.4;
const SAVED_FADE: f32 = 0.35;

/// "Set saved" for a moment after an edit is saved, right-aligned at
/// `right` on the entry name's line; the room it takes from that line.
fn saved_notice(ui: &egui::Ui, p: &Palette, state: &mut AoUi, right: egui::Pos2) -> f32 {
    let Some(at) = state.saved_at else { return 0.0 };
    let age = at.elapsed().as_secs_f32();
    if age >= SAVED_FOR {
        state.saved_at = None;
        return 0.0;
    }
    ui.ctx().request_repaint();
    let alpha = (age / SAVED_FADE).min((SAVED_FOR - age) / SAVED_FADE).clamp(0.0, 1.0);
    let ink = p.success.gamma_multiply(alpha);
    let galley = ui.painter().layout_no_wrap(
        aurora_i18n::tr!("ao-saved").to_owned(),
        egui::FontId::new(12.0, super::super::fonts::semibold(ui.ctx())),
        ink,
    );
    let text_w = galley.size().x;
    widgets::paint_row_text(ui.painter(), galley, right.x - text_w, right.y, ink);
    widgets::paint_glyph(
        ui.painter(),
        "circle-check-filled",
        egui::pos2(right.x - text_w - 10.0, right.y),
        14.0,
        ink,
    );
    text_w + 28.0
}

/// A fade from nothing at the top of `rect` to the stage's ground at its
/// bottom, under the caption.
fn fade(ui: &egui::Ui, rect: egui::Rect, ground: egui::Color32) {
    let mut mesh = egui::Mesh::default();
    let clear = ground.gamma_multiply(0.0);
    let solid = ground.gamma_multiply(0.92);
    let base = mesh.vertices.len() as u32;
    for (pos, color) in [
        (rect.left_top(), clear),
        (rect.right_top(), clear),
        (rect.right_bottom(), solid),
        (rect.left_bottom(), solid),
    ] {
        mesh.colored_vertex(pos, color);
    }
    mesh.add_triangle(base, base + 1, base + 2);
    mesh.add_triangle(base, base + 2, base + 3);
    ui.painter().add(egui::Shape::mesh(mesh));
}

/// The slot in small capitals with the entry's place, the entry's name in
/// large type, the layers on top, and the segments, close together.
#[allow(clippy::too_many_arguments)]
fn caption_text(
    ui: &mut egui::Ui,
    p: &Palette,
    set: Option<&Set>,
    runtime: &Runtime,
    state: &mut AoUi,
    names: &[String],
    area: egui::Rect,
    (ink, muted): (egui::Color32, egui::Color32),
) {
    let mut y = area.top() + EYEBROW / 2.0;
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
        muted,
    );
    y += (EYEBROW + HERO) / 2.0;
    let notice = saved_notice(ui, p, state, egui::pos2(area.right(), y));
    let hero = names
        .get(state.preview.index)
        .cloned()
        .unwrap_or_else(|| aurora_i18n::tr!("ao-preview-none").to_owned());
    widgets::paint_elided(
        ui,
        &hero,
        egui::pos2(area.left(), y),
        area.width() - notice,
        egui::FontId::new(20.0, super::super::fonts::semibold(ui.ctx())),
        ink,
    );
    y += (HERO + LAYER) / 2.0;
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
                muted,
            );
            y += LAYER;
        }
    }
    if names.len() > 1 {
        let n = names.len() as f32;
        let gap = 4.0;
        let width = (area.width() - gap * (n - 1.0)) / n;
        let top = area.bottom() - SEGMENTS;
        for (i, name) in names.iter().enumerate() {
            let hit = egui::Rect::from_min_size(egui::pos2(area.left() + i as f32 * (width + gap), top), Vec2::new(width, SEGMENTS));
            let response = ui
                .interact(hit, ui.scope_id().with(("ao_preview_segment", i)), egui::Sense::click())
                .on_hover_text(name);
            let height = if response.hovered() { 5.0 } else { 3.0 };
            let bar = egui::Rect::from_center_size(hit.center(), Vec2::new(width, height));
            let fill = match i.cmp(&state.preview.index) {
                std::cmp::Ordering::Equal => p.violet_pale,
                std::cmp::Ordering::Less => p.violet_pale.gamma_multiply(0.42),
                std::cmp::Ordering::Greater => ink.gamma_multiply(0.18),
            };
            ui.painter().rect_filled(bar, ui.visuals().widgets.inactive.corner_radius, fill);
            if response.clicked() {
                state.preview.show_entry(i);
            }
        }
    }
}
