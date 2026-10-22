//! Compact color picker of the interface: a saturation / value field, a hue
//! bar, the new color over the one the picker opened with, the R G B and
//! hexadecimal fields, an eyedropper. `color_button` is the swatch that opens
//! it as a pop-up; `color_panel` is its body, for any window. The palette
//! dialog of the construction tools (`ui/color_picker.rs`) stays apart.

use crate::theme::Palette;
use egui::{Color32, Rect, RichText, Sense, Stroke, Vec2, pos2};

/// An sRGB color with a straight (not premultiplied) alpha, as the settings
/// store it.
pub type Rgba = [u8; 4];

/// Hue, saturation and value, each in 0..=1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hsv {
    pub h: f32,
    pub s: f32,
    pub v: f32,
}

impl Hsv {
    /// The HSV of `rgb`, keeping from `self` what `rgb` cannot tell: the hue
    /// of a grey and the saturation of black. The handles then stay where
    /// the user left them when S or V reach 0.
    pub fn tracking(self, rgb: [u8; 3]) -> Hsv {
        if hsv_to_rgb(self) == rgb {
            return self;
        }
        let mut next = rgb_to_hsv(rgb);
        if next.s == 0.0 || (next.h == 0.0 && self.h == 1.0) {
            next.h = self.h;
        }
        if next.v == 0.0 {
            next.s = self.s;
        }
        next
    }
}

pub fn rgb_to_hsv(rgb: [u8; 3]) -> Hsv {
    let [r, g, b] = rgb.map(|c| f32::from(c) / 255.0);
    let max = r.max(g).max(b);
    let delta = max - r.min(g).min(b);
    let sector = if delta == 0.0 {
        0.0
    } else if max == r {
        ((g - b) / delta).rem_euclid(6.0)
    } else if max == g {
        (b - r) / delta + 2.0
    } else {
        (r - g) / delta + 4.0
    };
    Hsv {
        h: sector / 6.0,
        s: if max == 0.0 { 0.0 } else { delta / max },
        v: max,
    }
}

pub fn hsv_to_rgb(hsv: Hsv) -> [u8; 3] {
    let (s, v) = (hsv.s.clamp(0.0, 1.0), hsv.v.clamp(0.0, 1.0));
    let sector = hsv.h.rem_euclid(1.0) * 6.0;
    let f = sector.fract();
    let (low, down, up) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
    let rgb = match sector as u8 {
        0 => [v, up, low],
        1 => [down, v, low],
        2 => [low, v, up],
        3 => [low, down, v],
        4 => [up, low, v],
        _ => [v, low, down],
    };
    rgb.map(|c| (c * 255.0).round() as u8)
}

/// `rrggbb`, or `rrggbbaa` when the alpha is part of the color.
pub fn format_hex(color: Rgba, alpha: bool) -> String {
    let [r, g, b, a] = color;
    if alpha {
        format!("{r:02x}{g:02x}{b:02x}{a:02x}")
    } else {
        format!("{r:02x}{g:02x}{b:02x}")
    }
}

/// A typed color: `#rrggbb` or `rrggbb`, and `rrggbbaa` when `alpha` is on
/// (the alpha it gives, if any). Anything else is no color.
pub fn parse_hex(text: &str, alpha: bool) -> Option<([u8; 3], Option<u8>)> {
    let text = text.trim();
    let digits = text.strip_prefix('#').unwrap_or(text);
    // ASCII only: the slices below then fall on character boundaries
    if !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |at: usize| u8::from_str_radix(&digits[at..at + 2], 16).ok();
    match digits.len() {
        6 => Some(([byte(0)?, byte(2)?, byte(4)?], None)),
        8 if alpha => Some(([byte(0)?, byte(2)?, byte(4)?], Some(byte(6)?))),
        _ => None,
    }
}

/// The eyedropper shared by the compact pickers. The application owns the
/// mouse in the world: while a picker has armed the eyedropper, it samples
/// the face under the pointer (as it does for the palette dialog of the
/// construction tools) and hands the color back here.
pub mod pipette {
    #[derive(Clone, Copy, Default)]
    struct Shared {
        /// Picker the eyedropper works for, and the last frame it asked.
        armed: Option<(egui::Id, u64)>,
        /// Pointer position already sampled.
        cursor: Option<(f32, f32)>,
        sample: Option<(egui::Id, [u8; 3])>,
    }

    fn key() -> egui::Id {
        egui::Id::unique("aurora_color_pipette")
    }

    fn load(ctx: &egui::Context) -> Shared {
        ctx.data(|d| d.get_temp(key())).unwrap_or_default()
    }

    fn store(ctx: &egui::Context, shared: Shared) {
        ctx.data_mut(|d| d.insert_temp(key(), shared));
    }

    /// The picker that armed the eyedropper. A picker that has not been
    /// drawn for a few frames (its window was closed) no longer holds it.
    pub fn owner(ctx: &egui::Context) -> Option<egui::Id> {
        let (owner, seen) = load(ctx).armed?;
        (ctx.cumulative_frame_nr() <= seen + 2).then_some(owner)
    }

    pub fn armed(ctx: &egui::Context) -> bool {
        owner(ctx).is_some()
    }

    /// Picker: take the eyedropper, or keep it for one more frame.
    pub fn arm(ctx: &egui::Context, owner: egui::Id) {
        let mut shared = load(ctx);
        if shared.armed.map(|(id, _)| id) != Some(owner) {
            shared.cursor = None;
            shared.sample = None;
        }
        shared.armed = Some((owner, ctx.cumulative_frame_nr()));
        store(ctx, shared);
    }

    /// Put the eyedropper down: the picker's button or Escape, and the
    /// application when the mouse button that sampled is released.
    pub fn release(ctx: &egui::Context) {
        let mut shared = load(ctx);
        shared.armed = None;
        shared.cursor = None;
        store(ctx, shared);
    }

    /// Application: whether the pointer at `cursor` is still to be sampled
    /// (once per position, like `ColorPicker::needs_sample`).
    pub fn needs_sample(ctx: &egui::Context, cursor: (f32, f32)) -> bool {
        let mut shared = load(ctx);
        if owner(ctx).is_none() || shared.cursor == Some(cursor) {
            return false;
        }
        shared.cursor = Some(cursor);
        store(ctx, shared);
        true
    }

    /// Application: the sRGB color of the face under the pointer, if any.
    pub fn sample(ctx: &egui::Context, rgb: Option<[f32; 3]>) {
        let (Some(owner), Some(rgb)) = (owner(ctx), rgb) else {
            return;
        };
        let mut shared = load(ctx);
        shared.sample = Some((owner, rgb.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8)));
        store(ctx, shared);
    }

    /// Picker: the color sampled for it since its last frame.
    pub fn take(ctx: &egui::Context, owner: egui::Id) -> Option<[u8; 3]> {
        let mut shared = load(ctx);
        let (id, rgb) = shared.sample?;
        if id != owner {
            return None;
        }
        shared.sample = None;
        store(ctx, shared);
        Some(rgb)
    }
}

/// What a picker keeps between two frames.
#[derive(Clone, Debug)]
pub struct ColorPickState {
    /// Name of this picker for the shared eyedropper.
    id: egui::Id,
    hsv: Hsv,
    /// The color when the picker opened, under the new one.
    original: Rgba,
    /// Text of the hexadecimal field: the user's while it is edited.
    hex: String,
    hex_edited: bool,
    /// The eyedropper was armed on the previous frame: the click that
    /// samples is not a click away from the pop-up.
    was_armed: bool,
    /// Where the saturation / value field and the hue bar were drawn.
    pub(crate) field: Rect,
    pub(crate) hue_bar: Rect,
}

impl ColorPickState {
    pub fn new(id: egui::Id, color: Rgba) -> Self {
        Self {
            id,
            hsv: rgb_to_hsv([color[0], color[1], color[2]]),
            original: color,
            hex: String::new(),
            hex_edited: false,
            was_armed: false,
            field: Rect::NOTHING,
            hue_bar: Rect::NOTHING,
        }
    }
}

const FIELD: f32 = 140.0;
const HUE_BAR: f32 = 14.0;
const COLUMN: f32 = 80.0;
/// Space between the field, the bar and the column.
const GAP: f32 = 8.0;

/// Body of the picker. `alpha` adds the alpha to the fields; without it the
/// alpha of `color` is left as it is. True when `color` changed.
pub fn color_panel(ui: &mut egui::Ui, p: &Palette, state: &mut ColorPickState, color: &mut Rgba, alpha: bool) -> bool {
    let before = *color;
    if let Some(rgb) = pipette::take(ui.ctx(), state.id) {
        color[..3].copy_from_slice(&rgb);
    }
    state.hsv = state.hsv.tracking([color[0], color[1], color[2]]);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(GAP, 4.0);
        // rows of the height of their fields
        ui.spacing_mut().interact_size.y = 20.0;
        let moved = sv_field(ui, p, state) | hue_bar(ui, p, state);
        if moved {
            color[..3].copy_from_slice(&hsv_to_rgb(state.hsv));
        }
        ui.vertical(|ui| {
            ui.set_width(COLUMN);
            ui.spacing_mut().button_padding = Vec2::new(4.0, 1.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if swatches(ui, p, *color, state.original, alpha) {
                    *color = state.original;
                }
                eyedropper(ui, p, state.id);
            });
            ui.add_space(2.0);
            let letters: Vec<char> = aurora_i18n::tr!("color-picker-rgb").chars().collect();
            let letters: [char; 3] = letters.try_into().unwrap_or(['R', 'G', 'B']);
            let alpha_letter = aurora_i18n::tr!("common-alpha").chars().next().unwrap_or('A');
            let channels = [
                (letters[0], aurora_i18n::tr!("color-picker-red")),
                (letters[1], aurora_i18n::tr!("color-picker-green")),
                (letters[2], aurora_i18n::tr!("color-picker-blue")),
                (alpha_letter, aurora_i18n::tr!("common-alpha")),
            ];
            for (at, (letter, name)) in channels.into_iter().enumerate().take(if alpha { 4 } else { 3 }) {
                field_row(ui, p, &letter.to_string(), name, |ui| {
                    ui.add_sized(
                        [ui.available_width(), 20.0],
                        egui::DragValue::new(&mut color[at]).range(0..=255).speed(1.0),
                    );
                });
            }
            field_row(ui, p, "#", aurora_i18n::tr!("color-picker-hex-label"), |ui| {
                if !state.hex_edited {
                    state.hex = format_hex(*color, alpha);
                }
                let field = ui.add(
                    egui::TextEdit::singleline(&mut state.hex)
                        .desired_width(ui.available_width())
                        .char_limit(9)
                        .font(egui::FontId::monospace(11.5)),
                );
                // a text that is not a color leaves the color as it is
                if field.changed()
                    && let Some((rgb, typed_alpha)) = parse_hex(&state.hex, alpha)
                {
                    color[..3].copy_from_slice(&rgb);
                    if let Some(a) = typed_alpha {
                        color[3] = a;
                    }
                }
                state.hex_edited = field.has_focus();
            });
        });
    });
    *color != before
}

/// A letter and its field, on one line of the right column.
fn field_row(ui: &mut egui::Ui, p: &Palette, letter: &str, name: &str, field: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(12.0, 20.0), Sense::hover());
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            letter,
            egui::FontId::proportional(11.5),
            p.muted,
        );
        resp.on_hover_text(name);
        field(ui);
    });
}

/// Two rings, light in dark: the cursor reads on any color.
fn ring(painter: &egui::Painter, p: &Palette, center: egui::Pos2) {
    painter.circle_stroke(center, 5.0, Stroke::new(3.0, p.navy));
    painter.circle_stroke(center, 5.0, Stroke::new(1.4, p.ink));
}

/// The saturation (left to right) and value (bottom to top) of the hue.
fn sv_field(ui: &mut egui::Ui, p: &Palette, state: &mut ColorPickState) -> bool {
    const CELLS: u32 = 16;
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(FIELD), Sense::click_and_drag());
    state.field = rect;
    let hsv = &mut state.hsv;
    let mut changed = false;
    if let Some(pos) = resp.interact_pointer_pos() {
        let s = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
        let v = 1.0 - ((pos.y - rect.top()) / rect.height()).clamp(0.0, 1.0);
        changed = s != hsv.s || v != hsv.v;
        (hsv.s, hsv.v) = (s, v);
    }
    if ui.is_rect_visible(rect) {
        // a grid of exact colors: two triangles alone would bend the gradient
        let mut mesh = egui::Mesh::default();
        for y in 0..=CELLS {
            for x in 0..=CELLS {
                let (s, down) = (x as f32 / CELLS as f32, y as f32 / CELLS as f32);
                let [r, g, b] = hsv_to_rgb(Hsv {
                    h: hsv.h,
                    s,
                    v: 1.0 - down,
                });
                mesh.colored_vertex(rect.lerp_inside(Vec2::new(s, down)), Color32::from_rgb(r, g, b));
            }
        }
        for y in 0..CELLS {
            for x in 0..CELLS {
                let at = y * (CELLS + 1) + x;
                mesh.add_triangle(at, at + 1, at + CELLS + 1);
                mesh.add_triangle(at + 1, at + CELLS + 2, at + CELLS + 1);
            }
        }
        ui.painter().add(egui::Shape::mesh(mesh));
        ui.painter()
            .rect_stroke(rect, 0.0, Stroke::new(1.0, p.raised), egui::StrokeKind::Inside);
        let cursor = rect.lerp_inside(Vec2::new(hsv.s, 1.0 - hsv.v));
        ring(&ui.painter().with_clip_rect(rect.expand(6.0)), p, cursor);
    }
    changed
}

/// The hue, from red at the top round to red at the bottom.
fn hue_bar(ui: &mut egui::Ui, p: &Palette, state: &mut ColorPickState) -> bool {
    const STEPS: u32 = 36;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(HUE_BAR, FIELD), Sense::click_and_drag());
    state.hue_bar = rect;
    let hsv = &mut state.hsv;
    let mut changed = false;
    if let Some(pos) = resp.interact_pointer_pos() {
        let h = ((pos.y - rect.top()) / rect.height()).clamp(0.0, 1.0);
        changed = h != hsv.h;
        hsv.h = h;
    }
    if ui.is_rect_visible(rect) {
        let mut mesh = egui::Mesh::default();
        for step in 0..=STEPS {
            let h = step as f32 / STEPS as f32;
            let [r, g, b] = hsv_to_rgb(Hsv { h, s: 1.0, v: 1.0 });
            let y = egui::lerp(rect.top()..=rect.bottom(), h);
            mesh.colored_vertex(pos2(rect.left(), y), Color32::from_rgb(r, g, b));
            mesh.colored_vertex(pos2(rect.right(), y), Color32::from_rgb(r, g, b));
        }
        for step in 0..STEPS {
            let at = step * 2;
            mesh.add_triangle(at, at + 1, at + 2);
            mesh.add_triangle(at + 1, at + 3, at + 2);
        }
        ui.painter().add(egui::Shape::mesh(mesh));
        ui.painter()
            .rect_stroke(rect, 0.0, Stroke::new(1.0, p.raised), egui::StrokeKind::Inside);
        let y = egui::lerp(rect.top()..=rect.bottom(), hsv.h);
        let handle = Rect::from_center_size(pos2(rect.center().x, y), Vec2::new(rect.width() + 4.0, 6.0));
        let radius = ui.visuals().window_corner_radius;
        ui.painter()
            .rect_stroke(handle, radius, Stroke::new(3.0, p.navy), egui::StrokeKind::Middle);
        ui.painter()
            .rect_stroke(handle, radius, Stroke::new(1.4, p.ink), egui::StrokeKind::Middle);
    }
    changed
}

/// Grey squares under a color that lets them show through.
fn checkerboard(painter: &egui::Painter, p: &Palette, rect: Rect) {
    const CELL: f32 = 5.0;
    painter.rect_filled(rect, 0.0, p.muted);
    let painter = painter.with_clip_rect(rect);
    let (columns, rows) = ((rect.width() / CELL).ceil() as u32, (rect.height() / CELL).ceil() as u32);
    for row in 0..rows {
        for column in (row % 2..columns).step_by(2) {
            let min = rect.min + Vec2::new(column as f32 * CELL, row as f32 * CELL);
            painter.rect_filled(Rect::from_min_size(min, Vec2::splat(CELL)), 0.0, p.raised);
        }
    }
}

/// A flat patch of `color` with the given corners, over a checkerboard
/// (square: its cells are not rounded) when it is translucent.
fn patch(painter: &egui::Painter, p: &Palette, rect: Rect, color: Rgba, alpha: bool, corners: egui::CornerRadius) {
    let [r, g, b, a] = color;
    if alpha && a < 255 {
        checkerboard(painter, p, rect.shrink(1.0));
        painter.rect_filled(rect, corners, Color32::from_rgba_unmultiplied(r, g, b, a));
    } else {
        painter.rect_filled(rect, corners, Color32::from_rgb(r, g, b));
    }
}

/// The new color over the one the picker opened with; true when the latter
/// is clicked, to take it back.
fn swatches(ui: &mut egui::Ui, p: &Palette, color: Rgba, original: Rgba, alpha: bool) -> bool {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(COLUMN - 26.0, 40.0), Sense::hover());
    let (top, bottom) = rect.split_top_bottom_at_fraction(0.5);
    let new = ui.interact(top, ui.scope_id().with("color_new"), Sense::hover());
    let old = ui.interact(bottom, ui.scope_id().with("color_original"), Sense::click());
    let radius = ui.visuals().window_corner_radius;
    let painter = ui.painter().with_clip_rect(rect);
    let (upper, lower) = (
        egui::CornerRadius { sw: 0, se: 0, ..radius },
        egui::CornerRadius { nw: 0, ne: 0, ..radius },
    );
    patch(&painter, p, top, color, alpha, upper);
    patch(&painter, p, bottom, original, alpha, lower);
    let edge = if old.hovered() { p.violet_light } else { p.raised };
    painter.rect_stroke(rect, radius, Stroke::new(1.0, edge), egui::StrokeKind::Inside);
    new.on_hover_text(aurora_i18n::tr!("color-picker-current"));
    old.on_hover_text(aurora_i18n::tr!("color-picker-original")).clicked()
}

/// The eyedropper: armed, the next click on an object's face in the world
/// gives its color to this picker.
fn eyedropper(ui: &mut egui::Ui, p: &Palette, id: egui::Id) {
    let armed = pipette::owner(ui.ctx()) == Some(id);
    let tip = if armed {
        aurora_i18n::tr!("color-picker-pipette-hint")
    } else {
        aurora_i18n::tr!("color-picker-pipette")
    };
    if super::icon_toggle(ui, p, "color-picker", tip, armed) {
        if armed {
            pipette::release(ui.ctx());
        } else {
            pipette::arm(ui.ctx(), id);
        }
    } else if armed {
        // still wanted: a picker that is no longer drawn lets it go
        pipette::arm(ui.ctx(), id);
    }
}

/// Swatch of `color` that opens the compact picker under it. The response
/// is the swatch's, `changed()` when the color changed this frame. The
/// swatch takes the size of a stock control (`spacing.interact_size`).
pub fn color_button(ui: &mut egui::Ui, p: &Palette, id_salt: impl egui::AsIdSalt, color: &mut Rgba, alpha: bool) -> egui::Response {
    let id = ui.make_persistent_id(id_salt);
    let (rect, _) = ui.allocate_exact_size(ui.spacing().interact_size, Sense::hover());
    let mut resp = ui.interact(rect, id, Sense::click());
    let ctx = ui.ctx().clone();
    // The pop-up keeps its open state itself (the state exists while it is
    // open): egui's memory holds a single open pop-up, so a swatch inside a
    // menu or another pop-up could not open its own there.
    let mut state: Option<ColorPickState> = ctx.data(|d| d.get_temp(id));
    if ui.is_rect_visible(rect) {
        let radius = ui.visuals().window_corner_radius;
        let painter = ui.painter().with_clip_rect(rect);
        patch(&painter, p, rect, *color, alpha, radius);
        let lit = resp.hovered() || state.is_some();
        let edge = if lit { p.violet_light } else { p.raised };
        painter.rect_stroke(rect, radius, Stroke::new(1.0, edge), egui::StrokeKind::Inside);
    }
    if !ui.is_enabled() {
        // a greyed swatch neither opens its picker nor keeps it open
        state = None;
    } else if resp.clicked() || demo_opens(&ctx) {
        state = match state {
            Some(_) => None,
            None => Some(ColorPickState::new(id, *color)),
        };
    }
    let Some(mut state) = state else {
        close(&ctx, id);
        return resp.on_hover_text(aurora_i18n::tr!("color-picker-open"));
    };
    // Escape puts an armed eyedropper down before it closes the picker
    if pipette::owner(&ctx) == Some(id) && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        pipette::release(&ctx);
    }
    let before = *color;
    let mut open = true;
    let shown = egui::Popup::from_response(&resp)
        .open_bool(&mut open)
        .gap(4.0)
        .frame(egui::Frame::NONE)
        .close_behavior(egui::PopupCloseBehavior::IgnoreClicks)
        .show(|ui| {
            super::overlay(ui, p, egui::Margin::same(8), |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                ui.set_width(FIELD + HUE_BAR + COLUMN + 2.0 * GAP);
                let mut closed = false;
                ui.horizontal(|ui| {
                    let title = aurora_i18n::tr!("color-picker-title");
                    ui.label(RichText::new(title).size(11.0).strong().color(p.muted));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        closed = super::title_button(ui, p, "x")
                            .on_hover_text(aurora_i18n::tr!("common-close"))
                            .clicked();
                    });
                });
                color_panel(ui, p, &mut state, color, alpha);
                closed
            })
            .inner
        });
    // a click away closes the pop-up, but not the one that samples a color
    // for the eyedropper
    let armed = pipette::owner(&ctx) == Some(id);
    if let Some(shown) = shown {
        let away = shown.response.clicked_elsewhere() && resp.clicked_elsewhere();
        if shown.inner || (away && !armed && !state.was_armed) {
            open = false;
        }
    }
    state.was_armed = armed;
    if open {
        ctx.data_mut(|d| d.insert_temp(id, state));
    } else {
        close(&ctx, id);
    }
    if *color != before {
        resp.mark_changed();
    }
    resp
}

/// Forget the state of a closed picker, and its eyedropper with it.
fn close(ctx: &egui::Context, id: egui::Id) {
    if pipette::owner(ctx) == Some(id) {
        pipette::release(ctx);
    }
    ctx.data_mut(|d| d.remove::<ColorPickState>(id));
}

/// Demo captures: `AURORA_DEMO_COLORPICK=1` opens the first swatch drawn at
/// frame 300.
fn demo_opens(ctx: &egui::Context) -> bool {
    static DEMO: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if !*DEMO.get_or_init(|| std::env::var_os("AURORA_DEMO_COLORPICK").is_some()) || ctx.cumulative_frame_nr() != 300 {
        return false;
    }
    let done = egui::Id::unique("aurora_color_pick_demo");
    let first = !ctx.data(|d| d.get_temp::<bool>(done)).unwrap_or(false);
    ctx.data_mut(|d| d.insert_temp(done, true));
    first
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb_and_hsv_round_trip() {
        for rgb in [
            [56, 226, 28],
            [255, 0, 0],
            [0, 255, 0],
            [0, 0, 255],
            [255, 255, 0],
            [12, 34, 56],
            [200, 100, 250],
            [139, 92, 246],
            [1, 2, 3],
            [254, 255, 253],
        ] {
            assert_eq!(hsv_to_rgb(rgb_to_hsv(rgb)), rgb, "{rgb:?}");
        }
        let green = rgb_to_hsv([56, 226, 28]);
        assert!((green.h - 0.3098).abs() < 0.001 && (green.s - 0.876).abs() < 0.001 && (green.v - 0.886).abs() < 0.001);
        // both ends of the hue bar are red
        assert_eq!(hsv_to_rgb(Hsv { h: 0.0, s: 1.0, v: 1.0 }), [255, 0, 0]);
        assert_eq!(hsv_to_rgb(Hsv { h: 1.0, s: 1.0, v: 1.0 }), [255, 0, 0]);
    }

    #[test]
    fn black_white_and_grey_keep_the_stored_hue() {
        let stored = Hsv { h: 0.6, s: 0.8, v: 0.5 };
        let black = stored.tracking([0, 0, 0]);
        assert_eq!((black.h, black.s, black.v), (0.6, 0.8, 0.0), "black keeps hue and saturation");
        let white = stored.tracking([255, 255, 255]);
        assert_eq!((white.h, white.s, white.v), (0.6, 0.0, 1.0));
        let grey = stored.tracking([128, 128, 128]);
        assert_eq!((grey.h, grey.s), (0.6, 0.0));
        // a color the stored value already gives is not derived again
        let exact = Hsv { h: 0.123, s: 0.5, v: 0.5 };
        assert_eq!(exact.tracking(hsv_to_rgb(exact)), exact);
        // the bottom of the hue bar stays the bottom
        let end = Hsv { h: 1.0, s: 1.0, v: 1.0 };
        assert_eq!(end.tracking([200, 0, 0]).h, 1.0);
    }

    #[test]
    fn hexadecimal_text_with_and_without_alpha() {
        assert_eq!(format_hex([0x38, 0xe2, 0x1c, 0x80], false), "38e21c");
        assert_eq!(format_hex([0x38, 0xe2, 0x1c, 0x80], true), "38e21c80");
        assert_eq!(parse_hex("38e21c", false), Some(([0x38, 0xe2, 0x1c], None)));
        assert_eq!(parse_hex(" #38E21C ", false), Some(([0x38, 0xe2, 0x1c], None)));
        assert_eq!(parse_hex("#38e21c80", true), Some(([0x38, 0xe2, 0x1c], Some(0x80))));
        assert_eq!(parse_hex("38e21c", true), Some(([0x38, 0xe2, 0x1c], None)));
        // not colors: the alpha when it is off, a wrong length, other characters
        assert_eq!(parse_hex("38e21c80", false), None);
        assert_eq!(parse_hex("38e21", false), None);
        assert_eq!(parse_hex("#38e2zz", false), None);
        assert_eq!(parse_hex("ééé", false), None);
        assert_eq!(parse_hex("", true), None);
        for color in [[0, 0, 0, 0], [255, 255, 255, 255], [18, 52, 86, 120]] {
            let (rgb, alpha) = parse_hex(&format_hex(color, true), true).unwrap();
            assert_eq!([rgb[0], rgb[1], rgb[2], alpha.unwrap()], color);
        }
    }

    /// A swatch alone in a window.
    struct Bench {
        ctx: egui::Context,
        color: Rgba,
        alpha: bool,
        id: egui::Id,
        changed: bool,
        /// False: the swatch is in a greyed part of its window.
        enabled: bool,
    }

    const SWATCH: egui::Pos2 = pos2(40.0, 30.0);

    impl Bench {
        fn new(color: Rgba, alpha: bool) -> Self {
            let mut bench = Self {
                ctx: egui::Context::default(),
                color,
                alpha,
                id: egui::Id::NULL,
                changed: false,
                enabled: true,
            };
            bench.frame(vec![]);
            bench
        }

        fn frame(&mut self, events: Vec<egui::Event>) {
            let p = crate::theme::Theme::default().palette();
            let mut output = self.ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(700.0, 500.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let slot = Rect::from_min_size(SWATCH, Vec2::new(40.0, 20.0));
                    ui.scope_builder(egui::UiBuilder::new().max_rect(slot), |ui| {
                        ui.add_enabled_ui(self.enabled, |ui| {
                            self.id = ui.make_persistent_id("swatch");
                            self.changed = color_button(ui, &p, "swatch", &mut self.color, self.alpha).changed();
                        });
                    });
                },
            );
            output.textures_delta.clear();
        }

        fn pointer(&mut self, pos: egui::Pos2, pressed: bool) {
            self.frame(vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ]);
        }

        fn click(&mut self, pos: egui::Pos2) {
            self.pointer(pos, true);
            self.pointer(pos, false);
        }

        fn key(&mut self, key: egui::Key) {
            self.frame(vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }]);
        }

        fn state(&self) -> Option<ColorPickState> {
            self.ctx.data(|d| d.get_temp(self.id))
        }

        fn open(&mut self) -> ColorPickState {
            self.click(SWATCH + Vec2::new(8.0, 8.0));
            self.frame(vec![]);
            self.state().expect("a click on the swatch opens the picker")
        }
    }

    #[test]
    fn the_pop_up_stays_open_while_its_field_and_bar_are_dragged() {
        let mut bench = Bench::new([255, 0, 0, 255], false);
        assert!(bench.state().is_none());
        let state = bench.open();
        // drag across the field: to its middle, then out of it by its top right
        let field = state.field;
        assert!(field.width() > 100.0, "the field is drawn in the pop-up");
        bench.pointer(field.center(), true);
        assert!(bench.changed);
        assert_eq!(bench.color, [128, 64, 64, 255], "half saturation, half value of red");
        bench.frame(vec![egui::Event::PointerMoved(field.right_top() + Vec2::new(40.0, -40.0))]);
        assert_eq!(bench.color, [255, 0, 0, 255], "the drag goes on outside the field, clamped");
        bench.pointer(field.right_top() + Vec2::new(40.0, -40.0), false);
        assert!(bench.state().is_some(), "releasing a drag outside the pop-up is not a click away");
        // the hue bar: a third of the way down is green
        let bar = state.hue_bar;
        bench.click(bar.lerp_inside(Vec2::new(0.5, 1.0 / 3.0)));
        assert_eq!(bench.color, [0, 255, 0, 255]);
        // down to black, then back up: the hue was kept
        bench.click(field.lerp_inside(Vec2::new(1.0, 1.0)));
        assert_eq!(bench.color, [0, 0, 0, 255]);
        bench.click(field.lerp_inside(Vec2::new(1.0, 0.0)));
        assert_eq!(bench.color, [0, 255, 0, 255]);
        assert!(bench.state().is_some());
        // a click away closes it, and so does Escape
        bench.click(pos2(650.0, 450.0));
        assert!(bench.state().is_none());
        bench.open();
        bench.key(egui::Key::Escape);
        bench.frame(vec![]);
        assert!(bench.state().is_none());
        // the swatch closes the pop-up it opened
        bench.open();
        bench.click(SWATCH + Vec2::new(8.0, 8.0));
        assert!(bench.state().is_none());
    }

    #[test]
    fn a_greyed_swatch_neither_opens_nor_keeps_its_picker() {
        let mut bench = Bench::new([255, 0, 0, 255], false);
        let enable = |bench: &mut Bench, on: bool| {
            bench.enabled = on;
            bench.frame(vec![]);
        };
        enable(&mut bench, false);
        bench.click(SWATCH + Vec2::new(8.0, 8.0));
        bench.frame(vec![]);
        assert!(bench.state().is_none(), "a click on a greyed swatch opens nothing");
        assert_eq!(bench.color, [255, 0, 0, 255]);
        // greyed while its picker is open: the picker goes with it
        enable(&mut bench, true);
        bench.open();
        enable(&mut bench, false);
        assert!(bench.state().is_none());
        // and it opens again once it is enabled
        enable(&mut bench, true);
        bench.open();
    }

    #[test]
    fn the_alpha_is_left_alone_unless_it_is_edited() {
        let mut bench = Bench::new([10, 20, 30, 77], false);
        let state = bench.open();
        bench.click(state.field.center());
        assert_eq!(bench.color[3], 77);
        assert_ne!(&bench.color[..3], &[10, 20, 30]);
    }

    #[test]
    fn the_eyedropper_gives_the_sampled_color_and_escape_puts_it_down_first() {
        let mut bench = Bench::new([10, 20, 30, 77], true);
        bench.open();
        assert!(!pipette::armed(&bench.ctx));
        pipette::arm(&bench.ctx, bench.id);
        bench.frame(vec![]);
        assert!(pipette::armed(&bench.ctx), "the open picker keeps its eyedropper");
        // the application samples the face under the pointer on the press,
        // and puts the eyedropper down on the release
        let world = pos2(600.0, 400.0);
        assert!(pipette::needs_sample(&bench.ctx, (world.x, world.y)));
        assert!(!pipette::needs_sample(&bench.ctx, (world.x, world.y)), "once per position");
        pipette::sample(&bench.ctx, Some([1.0, 0.5, 0.0]));
        bench.pointer(world, true);
        assert!(bench.changed);
        assert_eq!(bench.color, [255, 128, 0, 77], "the sampled color, the alpha unchanged");
        pipette::release(&bench.ctx);
        bench.pointer(world, false);
        bench.frame(vec![]);
        assert!(bench.state().is_some(), "the click that samples does not close the pop-up");
        // nothing under the pointer: the color stays
        pipette::arm(&bench.ctx, bench.id);
        pipette::sample(&bench.ctx, None);
        bench.frame(vec![]);
        assert_eq!(bench.color, [255, 128, 0, 77]);
        // Escape: the eyedropper first, the pop-up next
        bench.key(egui::Key::Escape);
        assert!(!pipette::armed(&bench.ctx));
        assert!(
            bench.state().is_some(),
            "Escape puts the eyedropper down and leaves the picker open"
        );
        bench.key(egui::Key::Escape);
        bench.frame(vec![]);
        assert!(bench.state().is_none());
        // a picker that is closed, or no longer drawn, does not hold it
        bench.open();
        pipette::arm(&bench.ctx, bench.id);
        bench.click(SWATCH + Vec2::new(8.0, 8.0));
        assert!(!pipette::armed(&bench.ctx));
        pipette::arm(&bench.ctx, egui::Id::unique("gone"));
        for _ in 0..4 {
            bench.frame(vec![]);
        }
        assert!(!pipette::armed(&bench.ctx));
    }
}
