//! Small building blocks of the build floater: labels, Firestorm-style
//! spinners (LLSpinCtrl: drag or type, commit on release / Enter), three-
//! state check boxes, combo boxes, texture and color swatches.

use crate::theme::Palette;
use egui::{RichText, Vec2};
use std::collections::HashMap;
use uuid::Uuid;

// Same hierarchy as the resource monitor: body/controls 12, secondary 11,
// section titles 13. Keep the theme's font families and global font scaling.
pub(super) const BODY: f32 = 12.0;
pub(super) const SMALL: f32 = 11.0;
pub(super) const TITLE: f32 = 13.0;

pub(super) fn typography(ui: &mut egui::Ui) {
    crate::ui::widgets::organization_typography(ui);
}

/// How a build field's keyboard entry stands this pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FieldEnd {
    /// Still typing, or not being typed in.
    Open,
    /// Focus moved away or Enter: send the typed value.
    Commit,
    /// Escape: drop the typed text and send nothing, as LLLineEditor and
    /// LLSpinCtrl::handleKeyHere (indra/llui/llspinctrl.cpp) revert it.
    Revert,
    /// The pass after the entry ended: egui reports lost_focus again and
    /// DragValue parses the abandoned text once more. Nothing to send.
    Repeat,
}

/// Escape pressed this pass, read from the raw input: a window drawn before
/// the build inspector may consume the key, but the field still has to know
/// that its entry was abandoned rather than committed.
pub(crate) fn escape_pressed(ui: &egui::Ui) -> bool {
    ui.input(|i| {
        i.raw
            .events
            .iter()
            .any(|e| matches!(e, egui::Event::Key { key: egui::Key::Escape, pressed: true, modifiers, .. } if !modifiers.any()))
    })
}

/// The shared end rule of every build field that commits typed text on its
/// own (spinners, name, description, sale price): one decision per entry.
pub(crate) fn field_end(ui: &egui::Ui, r: &egui::Response) -> FieldEnd {
    if !r.lost_focus() {
        return FieldEnd::Open;
    }
    let ended_id = r.id.with("build_field_ended");
    let pass = ui.ctx().cumulative_pass_nr();
    if ui.data(|d| d.get_temp::<u64>(ended_id)).is_some_and(|ended| ended + 1 == pass) {
        return FieldEnd::Repeat;
    }
    ui.data_mut(|d| d.insert_temp(ended_id, pass));
    if escape_pressed(ui) { FieldEnd::Revert } else { FieldEnd::Commit }
}

/// What a value control did this frame: changed (live) and / or must be
/// sent (typed value, end of a drag).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Edit {
    pub changed: bool,
    pub commit: bool,
    /// A number typed over a mixed ("—") field: committed even when it
    /// equals the first selected value the field holds underneath.
    pub typed_over_mixed: bool,
}

impl Edit {
    pub fn or(self, o: Edit) -> Edit {
        Edit {
            changed: self.changed || o.changed,
            commit: self.commit || o.commit,
            typed_over_mixed: self.typed_over_mixed || o.typed_over_mixed,
        }
    }
}

#[derive(Clone, Copy)]
struct SpinnerDrag {
    widget: egui::Id,
    value: f32,
}

pub fn label(ui: &mut egui::Ui, p: &Palette, t: &str) -> egui::Response {
    ui.label(RichText::new(t).size(12.0).color(p.muted))
}

pub fn text(ui: &mut egui::Ui, p: &Palette, t: &str) -> egui::Response {
    ui.label(RichText::new(t).size(12.0).color(p.ink))
}

pub fn small(ui: &mut egui::Ui, p: &Palette, t: &str) -> egui::Response {
    ui.label(RichText::new(t).size(11.0).color(p.muted))
}

/// Section title, in the violet of the brand.
pub fn section(ui: &mut egui::Ui, p: &Palette, title: &str) {
    crate::ui::widgets::organization_section(ui, p, title);
}

/// A label of fixed width then the content (Firestorm's two-column rows).
pub fn row<R>(ui: &mut egui::Ui, p: &Palette, name: &str, width: f32, body: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.horizontal(|ui| {
        if super::legacy::active(ui) {
            let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 20.0), egui::Sense::hover());
            let mut caption = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(rect)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            caption.set_clip_rect(ui.clip_rect().intersect(rect));
            caption
                .add(egui::Label::new(RichText::new(name).size(BODY).color(p.muted)).truncate())
                .on_hover_text(name);
        } else {
            ui.allocate_ui_with_layout(Vec2::new(width, 20.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.set_min_width(width);
                label(ui, p, name)
            });
        }
        body(ui)
    })
    .inner
}

/// LLSpinCtrl: a number to drag or type. `tentative` shows it as mixed
/// (several values selected); a number typed over the mixed value is
/// committed even when it equals the hidden first value.
pub fn spin(
    ui: &mut egui::Ui,
    v: &mut f32,
    increment: f32,
    range: std::ops::RangeInclusive<f32>,
    decimals: usize,
    width: f32,
    tentative: bool,
) -> Edit {
    spin_with_min_decimals(ui, v, increment, range, decimals, width, tentative, decimals.min(5))
}

#[allow(clippy::too_many_arguments)]
pub fn spin_with_min_decimals(
    ui: &mut egui::Ui,
    v: &mut f32,
    increment: f32,
    range: std::ops::RangeInclusive<f32>,
    decimals: usize,
    width: f32,
    tentative: bool,
    min_decimals: usize,
) -> Edit {
    spinner(ui, v, increment, range, decimals, width, tentative, min_decimals, true)
}

/// A position, size or rotation field: LLPanelObject::sendPosition /
/// sendScale / sendRotation (indra/newview/llpanelobject.cpp) send only a
/// value that differs from the object's, so a number typed over a mixed
/// axis that equals the primary object's sends nothing.
#[allow(clippy::too_many_arguments)]
pub fn transform_spin(
    ui: &mut egui::Ui,
    v: &mut f32,
    increment: f32,
    range: std::ops::RangeInclusive<f32>,
    decimals: usize,
    width: f32,
    tentative: bool,
    min_decimals: usize,
) -> Edit {
    spinner(ui, v, increment, range, decimals, width, tentative, min_decimals, false)
}

/// Points of a trackpad flick that make one notch. Same grain as the shape
/// editor's category wheel.
const WHEEL_POINTS_PER_NOTCH: f32 = 55.0;

#[derive(Clone, Copy)]
struct WheelRemainder(f32);

struct WheelSteps {
    /// Modifier-scaled notches. Positive raises.
    scaled: f32,
    /// The same notches, each rounded to an integer. Ctrl and Shift become
    /// no step on an integer field.
    integer: i32,
}

impl WheelSteps {
    fn none() -> Self {
        Self { scaled: 0.0, integer: 0 }
    }
}

/// Notches of the wheel over `rect`. Positive raises.
///
/// LLSpinCtrl::handleScrollWheel (indra/llui/llspinctrl.cpp) calls onDownBtn
/// for a positive click count and onUpBtn for a negative one, and returns
/// true so the panel does not also scroll. The Legacy Viewer negates the
/// platform wheel (llwindowsdl2.cpp), so rolling it up raises the value.
/// winit reports that roll as a positive delta.y. Each whole notch is scaled
/// like onUpBtn. While the pointer stays on an enabled field the smoothed
/// scroll is eaten, including the frames after the notch, so the build panel
/// does not creep. A disabled field leaves the wheel to the panel. `apply`
/// is false while a typed draft or a drag owns the value: the wheel is still
/// eaten, and it does not change the number.
fn spinner_wheel(ui: &mut egui::Ui, rect: egui::Rect, id: egui::Id, apply: bool) -> WheelSteps {
    let key = id.with("spinner_wheel_remainder");
    let hovered = ui.is_enabled() && ui.rect_contains_pointer(rect);
    if !hovered {
        ui.data_mut(|d| d.remove::<WheelRemainder>(key));
        return WheelSteps::none();
    }
    ui.input_mut(|input| input.smooth_scroll_delta = egui::Vec2::ZERO);
    if !apply {
        ui.data_mut(|d| d.remove::<WheelRemainder>(key));
        return WheelSteps::none();
    }
    let mut remainder = ui.data(|d| d.get_temp::<WheelRemainder>(key)).map(|kept| kept.0).unwrap_or(0.0);
    let mut scaled = 0.0f32;
    let mut integer = 0i32;
    ui.input(|input| {
        for event in &input.events {
            let egui::Event::MouseWheel {
                unit, delta, modifiers, ..
            } = event
            else {
                continue;
            };
            if delta.y == 0.0 || !delta.y.is_finite() {
                continue;
            }
            let notches = delta.y
                / match unit {
                    egui::MouseWheelUnit::Point => WHEEL_POINTS_PER_NOTCH,
                    egui::MouseWheelUnit::Line | egui::MouseWheelUnit::Page => 1.0,
                };
            if !notches.is_finite() {
                continue;
            }
            remainder += notches;
            let whole = remainder.trunc().clamp(-100.0, 100.0);
            remainder -= whole;
            if whole != 0.0 {
                let scale = crate::ui::widgets::spin_key_scale(*modifiers);
                scaled += whole * scale;
                integer = integer.saturating_add((whole as i32).saturating_mul(scale.round() as i32));
            }
        }
    });
    if remainder == 0.0 {
        ui.data_mut(|d| d.remove::<WheelRemainder>(key));
    } else {
        ui.data_mut(|d| d.insert_temp(key, WheelRemainder(remainder)));
    }
    WheelSteps { scaled, integer }
}

#[allow(clippy::too_many_arguments)]
fn spinner(
    ui: &mut egui::Ui,
    v: &mut f32,
    increment: f32,
    range: std::ops::RangeInclusive<f32>,
    decimals: usize,
    width: f32,
    tentative: bool,
    min_decimals: usize,
    commit_typed_mixed: bool,
) -> Edit {
    // Commit-only fields are read from the object again each frame. Keep the
    // gesture's value until release, without sending intermediate server edits.
    let draft_id = ui.next_auto_id().with("build_spinner_drag");
    let draft = ui
        .data_mut(|data| {
            let draft = data.get_temp::<SpinnerDrag>(draft_id);
            data.remove::<SpinnerDrag>(draft_id);
            draft
        })
        .filter(|draft| ui.is_enabled() && (ui.ctx().is_being_dragged(draft.widget) || ui.ctx().drag_stopped_id() == Some(draft.widget)));
    if let Some(draft) = draft {
        *v = draft.value;
    }
    // LLSpinCtrl::onUpBtn (indra/llui/llspinctrl.cpp): Alt ×10, else Ctrl
    // ×0.1, else Shift ×0.01. The drag keeps its speed (increment × 0.25)
    // and only borrows the scale; arrows and the wheel scale the increment.
    let scale = crate::ui::widgets::spin_key_scale(ui.input(|i| i.modifiers));
    let speed = increment * 0.25 * scale;
    let before = *v;
    // Whether the field wrote a value this frame: a typed entry writes its
    // parsed number even when it equals the one shown underneath.
    let written = std::cell::Cell::new(false);
    // LLSpinCtrl::setValue displays server values (including megaprims)
    // unchanged; LLPanelObject::sendScale bounds only deliberate edits.
    let mut dv = egui::DragValue::from_get_set(|new| {
        if let Some(new) = new {
            *v = new as f32;
            written.set(true);
        }
        f64::from(*v)
    })
    .speed(speed)
    .update_while_editing(false)
    .range(range.clone())
    .clamp_existing_to_range(false)
    .custom_parser(aurora_i18n::parse_number)
    .max_decimals(decimals)
    .min_decimals(min_decimals.min(decimals));
    if tentative {
        dv = dv.custom_formatter(|_, _| "—".into());
    }
    let height = if super::legacy::active(ui) { 20.0 } else { 26.0 };
    let row = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let r = ui.add_sized([(width - 13.0).max(16.0), height], dv);
        let step = crate::ui::widgets::step_buttons(ui, height);
        (r, step)
    });
    let row_rect = row.response.rect;
    let (r, step) = row.inner;
    // A typed draft and a drag keep the number they already own. The wheel
    // over the row is still eaten, so the panel does not jump underneath.
    let protect = r.has_focus() || r.dragged() || r.drag_stopped() || draft.is_some();
    let wheel = spinner_wheel(ui, row_rect, r.id, !protect);
    // A spin click or a drag is not a typed entry; otherwise an abandoned
    // or already ended entry writes nothing, even if DragValue parsed its
    // text again.
    if step == 0
        && wheel.scaled == 0.0
        && !r.dragged()
        && !r.drag_stopped()
        && matches!(field_end(ui, &r), FieldEnd::Revert | FieldEnd::Repeat)
    {
        *v = before;
        return Edit::default();
    }
    let stepped = step != 0 || wheel.scaled != 0.0;
    if stepped {
        *v = (*v + increment * (step as f32 * scale + wheel.scaled)).clamp(*range.start(), *range.end());
    }
    if r.dragged() {
        ui.data_mut(|data| data.insert_temp(draft_id, SpinnerDrag { widget: r.id, value: *v }));
    }
    let mut e = Edit {
        changed: stepped,
        commit: stepped,
        ..Edit::default()
    };
    // TextEdit also reports changed while only its uncommitted text changed.
    if r.changed() && *v != before {
        e.changed = true;
        if !r.dragged() {
            e.commit = true;
        }
    }
    if r.drag_stopped() {
        e.changed |= draft.is_some();
        e.commit = true;
    }
    // LLSpinCtrl::onEditorCommit (indra/llui/llspinctrl.cpp) commits every
    // typed entry. A mixed field shows "—" over the first selected value, so
    // typing that same value (0 % over a linkset part with one transparent
    // face) must still reach every selected face. Escape and an untouched
    // "—" write nothing.
    if commit_typed_mixed && tentative && written.get() && r.lost_focus() {
        e.changed = true;
        e.commit = true;
        e.typed_over_mixed = true;
    }
    e
}

/// Integer spinner.
pub fn spin_i(ui: &mut egui::Ui, v: &mut i32, range: std::ops::RangeInclusive<i32>, width: f32) -> Edit {
    let before = *v;
    let height = if super::legacy::active(ui) { 20.0 } else { 26.0 };
    // The drag stays at speed 0.25. Modifiers scale arrows and the wheel
    // only; a fractional scale rounds away, so Ctrl and Shift do not step.
    let scale = crate::ui::widgets::spin_key_scale(ui.input(|i| i.modifiers)).round() as i32;
    let row = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let r = ui.add_sized(
            [(width - 13.0).max(16.0), height],
            egui::DragValue::new(&mut *v)
                .update_while_editing(false)
                .custom_parser(aurora_i18n::parse_number)
                .speed(0.25)
                .range(range.clone()),
        );
        (r, crate::ui::widgets::step_buttons(ui, height))
    });
    let row_rect = row.response.rect;
    let (r, step) = row.inner;
    let protect = r.has_focus() || r.dragged() || r.drag_stopped();
    let wheel = spinner_wheel(ui, row_rect, r.id, !protect);
    let delta = scale.saturating_mul(step) + wheel.integer;
    if delta == 0 && !r.dragged() && !r.drag_stopped() && matches!(field_end(ui, &r), FieldEnd::Revert | FieldEnd::Repeat) {
        *v = before;
        return Edit::default();
    }
    if delta != 0 {
        *v = v.saturating_add(delta).clamp(*range.start(), *range.end());
    }
    Edit {
        changed: (r.changed() && *v != before) || delta != 0,
        commit: delta != 0 || (r.changed() && *v != before && !r.dragged()) || r.drag_stopped(),
        ..Edit::default()
    }
}

/// Check box that can show a mixed state; returns the new value when clicked.
pub fn check(ui: &mut egui::Ui, p: &Palette, value: bool, tentative: bool, label: &str, enabled: bool) -> Option<bool> {
    let mut v = value && !tentative;
    let label = RichText::new(label).size(12.0);
    let r = ui
        .add_enabled_ui(enabled, |ui| crate::ui::widgets::checkbox_mixed(ui, p, &mut v, label, tentative))
        .inner;
    r.clicked().then_some(v)
}

/// Combo box over a list of (label, value); returns the chosen value.
pub fn combo<T: PartialEq + Copy>(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    current: Option<T>,
    items: &[(&str, T)],
    width: f32,
    enabled: bool,
) -> Option<T> {
    let shown = current
        .and_then(|c| items.iter().find(|(_, v)| *v == c).map(|(l, _)| *l))
        .unwrap_or("—");
    let mut chosen = None;
    ui.add_enabled_ui(enabled, |ui| {
        egui::ComboBox::from_id_salt(id)
            .width(width)
            .selected_text(RichText::new(shown).size(12.0))
            .show_ui(ui, |ui| {
                for (l, v) in items {
                    if ui.selectable_label(current == Some(*v), RichText::new(*l).size(12.0)).clicked() {
                        chosen = Some(*v);
                    }
                }
            });
    });
    chosen.filter(|c| current != Some(*c))
}

/// A texture swatch (LLTextureCtrl): the image, its caption below, a
/// "Multiple" overlay when the selected faces differ. Clicked: open the picker.
#[allow(clippy::too_many_arguments)]
pub fn swatch(
    ui: &mut egui::Ui,
    p: &Palette,
    images: &HashMap<Uuid, egui::TextureHandle>,
    wanted: &mut std::collections::HashSet<Uuid>,
    id: Option<Uuid>,
    caption: &str,
    size: f32,
    multiple: bool,
    enabled: bool,
) -> egui::Response {
    ui.vertical(|ui| {
        let sense = if enabled { egui::Sense::click() } else { egui::Sense::hover() };
        let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), sense);
        let painter = ui.painter();
        painter.rect_filled(rect, ui.visuals().window_corner_radius, p.field);
        match id.filter(|i| !i.is_nil()) {
            Some(tex) => {
                wanted.insert(tex);
                if crate::scene::textures::is_builtin_transparent(&tex) {
                    let cell = 10.0;
                    let pixels = rect.shrink(1.0);
                    for y in 0..(pixels.height() / cell).ceil() as usize {
                        for x in 0..(pixels.width() / cell).ceil() as usize {
                            let tile =
                                egui::Rect::from_min_size(pixels.min + Vec2::new(x as f32 * cell, y as f32 * cell), Vec2::splat(cell))
                                    .intersect(pixels);
                            // Checkerboard represents transparent image pixels.
                            painter.rect_filled(tile, 0.0, if (x + y) % 2 == 0 { p.field } else { p.raised });
                        }
                    }
                } else if tex == aurora_prim::te::BLANK_TEXTURE {
                    // IMG_WHITE is an opaque white texture.
                    painter.rect_filled(
                        rect.shrink(1.0),
                        ui.visuals().window_corner_radius,
                        crate::ui::widgets::PICTURE_TINT,
                    );
                }
                match images.get(&tex) {
                    Some(t) => {
                        painter.image(
                            t.id(),
                            rect.shrink(1.0),
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                            if enabled {
                                crate::ui::widgets::PICTURE_TINT
                            } else {
                                crate::ui::widgets::PICTURE_TINT_DISABLED
                            },
                        );
                    }
                    None if tex != aurora_prim::te::BLANK_TEXTURE && !crate::scene::textures::is_builtin_transparent(&tex) => {
                        painter.text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            "…",
                            egui::FontId::proportional(12.0),
                            p.muted,
                        );
                    }
                    None => {}
                }
            }
            None => {
                // no texture: a cross like LLTextureCtrl
                let s = egui::Stroke::new(1.0, p.muted_dim);
                painter.line_segment([rect.left_top(), rect.right_bottom()], s);
                painter.line_segment([rect.right_top(), rect.left_bottom()], s);
            }
        }
        if multiple {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                aurora_i18n::tr!("build-ui-common-multiple"),
                egui::FontId::proportional(11.0),
                p.ink,
            );
        }
        let stroke = if resp.hovered() && enabled { p.violet_light } else { p.raised };
        painter.rect_stroke(
            rect,
            ui.visuals().window_corner_radius,
            egui::Stroke::new(1.0, stroke),
            egui::StrokeKind::Inside,
        );
        crate::ui::widgets::local_texture::asset(ui, p, rect, id);
        if !caption.is_empty() {
            ui.add_sized(
                [size, 14.0],
                egui::Label::new(RichText::new(caption).size(11.0).color(if enabled { p.muted } else { p.muted_dim })),
            );
        }
        resp
    })
    .inner
}

/// LLColorSwatchCtrl: clicking opens the shared color dialog. The stored
/// value is sRGB, not egui's linear-RGB color button input.
pub fn color_swatch(ui: &mut egui::Ui, p: &Palette, rgb: [f32; 3], caption: &str, size: f32, enabled: bool) -> egui::Response {
    color_swatch_sized(ui, p, rgb, caption, Vec2::splat(size), enabled)
}

/// Rectangular variant for a labelled material row; it keeps the shared picker transaction.
pub fn color_swatch_sized(ui: &mut egui::Ui, p: &Palette, rgb: [f32; 3], caption: &str, size: Vec2, enabled: bool) -> egui::Response {
    ui.vertical(|ui| {
        let response = ui
            .add_enabled_ui(enabled, |ui| {
                let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
                let fill = if ui.is_enabled() {
                    crate::ui::color_picker::color(rgb)
                } else {
                    p.field
                };
                ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, fill);
                ui.painter().rect_stroke(
                    rect,
                    ui.visuals().window_corner_radius,
                    egui::Stroke::new(1.0, if response.hovered() { p.violet } else { p.raised }),
                    egui::StrokeKind::Inside,
                );
                if !ui.is_enabled() {
                    crate::ui::sheet::icon_at(ui, "lock-filled", rect.center(), 14.0, p.muted);
                }
                response.on_hover_text(aurora_i18n::tr!("color-picker-open"))
            })
            .inner;
        if !caption.is_empty() {
            ui.add_sized(
                [size.x, 14.0],
                egui::Label::new(RichText::new(caption).size(11.0).color(if enabled { p.muted } else { p.muted_dim })),
            );
        }
        response
    })
    .inner
}

pub use crate::ui::widgets::icon_button;

/// sRGB <-> linear (llmath.h linearTosRGB / sRGBtoLinear).
pub fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

pub fn linear_to_srgb(v: f32) -> f32 {
    if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

/// The same value for every item, or None (mixed / empty).
pub fn same<T: PartialEq + Copy>(mut it: impl Iterator<Item = T>) -> (Option<T>, bool) {
    let Some(first) = it.next() else {
        return (None, false);
    };
    let mixed = it.any(|v| v != first);
    (Some(first), mixed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn megaprim_spinner_displays_existing_size_but_clamps_typed_edits() {
        for (typed, expected) in [("32", 32.0), ("128", 64.0), ("0", 0.01)] {
            let ctx = egui::Context::default();
            let mut value = 128.125;
            let mut point = egui::Pos2::ZERO;
            let mut show = |events: Vec<egui::Event>| {
                let mut edit = Edit::default();
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(200.0, 100.0))),
                        events,
                        focused: true,
                        ..Default::default()
                    },
                    |ui| {
                        point = ui.cursor().min + egui::vec2(25.0, 13.0);
                        edit = spin(ui, &mut value, 0.01, 0.01..=64.0, 3, 90.0, false);
                    },
                );
                output.textures_delta.clear();
                (value, edit, point)
            };
            let (_, _, point) = show(vec![]);
            for _ in 0..3 {
                let (value, edit, _) = show(vec![]);
                assert_eq!(value, 128.125);
                assert_eq!(edit, Edit::default());
            }
            show(vec![egui::Event::PointerMoved(point)]);
            for pressed in [true, false] {
                show(vec![egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                }]);
            }
            assert!(ctx.memory(|m| m.focused()).is_some());
            let (value, edit, _) = show(vec![egui::Event::Text(typed.into())]);
            assert_eq!(value, 128.125, "Typing must remain a draft");
            assert_eq!(edit, Edit::default());
            let (value, edit, _) = show(vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }]);
            assert_eq!(value, expected);
            assert!(edit.changed && edit.commit);
            let (value, edit, _) = show(vec![]);
            assert_eq!(value, expected);
            assert_eq!(edit, Edit::default(), "An edit must commit only once");
        }
    }

    #[test]
    fn increment_buttons_commit_clamp_and_inherit_disabled_state() {
        let ctx = egui::Context::default();
        let mut value = 0.95;
        let mut step_at = egui::Pos2::ZERO;
        let mut show = |events: Vec<egui::Event>, enabled| {
            let mut result = Edit::default();
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(200.0, 100.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    ui.add_enabled_ui(enabled, |ui| {
                        step_at = ui.cursor().min + egui::vec2(63.5, 6.5);
                        result = spin(ui, &mut value, 0.1, 0.0..=1.0, 3, 70.0, false);
                    });
                },
            );
            output.textures_delta.clear();
            (value, result, step_at)
        };
        let (_, _, point) = show(vec![], true);
        show(vec![egui::Event::PointerMoved(point)], true);
        for pressed in [true, false] {
            let (value, result, _) = show(
                vec![egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                }],
                true,
            );
            if !pressed {
                assert_eq!(value, 1.0);
                assert!(result.changed && result.commit);
            }
        }
        for pressed in [true, false] {
            let (value, result, _) = show(
                vec![egui::Event::PointerButton {
                    pos: point + egui::vec2(0.0, 13.0),
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                }],
                false,
            );
            assert_eq!(value, 1.0);
            assert!(!result.changed && !result.commit);
        }
    }

    fn input(time: f64, events: Vec<egui::Event>) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 400.0))),
            time: Some(time),
            events,
            focused: true,
            ..Default::default()
        }
    }

    fn wheel(y: f32, unit: egui::MouseWheelUnit, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::MouseWheel {
            unit,
            delta: egui::vec2(0.0, y),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Wheel,
            modifiers,
        }
    }

    fn button(pos: egui::Pos2, pressed: bool, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers,
        }
    }

    struct Shown {
        edit: Edit,
        field: egui::Pos2,
        gap: egui::Pos2,
        arrow: egui::Pos2,
        offset: f32,
    }

    /// One notch up is a positive delta.y: X11 button 4, Wayland's negated
    /// discrete axis and WM_MOUSEWHEEL all report a roll away from the user
    /// that way, and that roll raises a Legacy spinner.
    #[test]
    fn wheel_over_a_spinner_steps_clamps_and_leaves_the_panel_still() {
        let ctx = egui::Context::default();
        let value = std::cell::Cell::new(0.5f32);
        let run = |time: f64, events: Vec<egui::Event>, enabled: bool| {
            let mut shown = Shown {
                edit: Edit::default(),
                field: egui::Pos2::ZERO,
                gap: egui::Pos2::ZERO,
                arrow: egui::Pos2::ZERO,
                offset: 0.0,
            };
            let mut current = value.get();
            let mut output = ctx.run_ui(input(time, events), |ui| {
                let scrolled = egui::ScrollArea::vertical().max_height(90.0).show(ui, |ui| {
                    ui.add_space(30.0);
                    shown.gap = ui.min_rect().min + egui::vec2(12.0, 8.0);
                    shown.field = ui.cursor().min + egui::vec2(20.0, 10.0);
                    ui.add_enabled_ui(enabled, |ui| {
                        shown.edit = spin(ui, &mut current, 0.1, 0.0..=1.0, 2, 90.0, false);
                    });
                    ui.add_space(400.0);
                });
                shown.offset = scrolled.state.clamped_offset().y;
            });
            output.textures_delta.clear();
            value.set(current);
            shown
        };
        let laid = run(0.0, vec![], true);
        let field = laid.field;
        let gap = laid.gap;
        let up = run(
            0.1,
            vec![
                egui::Event::PointerMoved(field),
                wheel(1.0, egui::MouseWheelUnit::Line, Default::default()),
            ],
            true,
        );
        assert!(
            (value.get() - 0.6).abs() < 1e-4,
            "a notch up raises by one increment, got {}",
            value.get()
        );
        assert!(up.edit.changed && up.edit.commit);
        assert_eq!(up.offset, 0.0, "the build panel must not scroll while the field is hovered");
        let mut stayed = up.offset;
        for step in 1..6 {
            stayed = run(0.1 + step as f64 * 0.05, vec![egui::Event::PointerMoved(field)], true).offset;
        }
        assert!((value.get() - 0.6).abs() < 1e-4, "smoothing after the notch must not step again");
        assert_eq!(stayed, 0.0, "queued scroll must not move the panel while the field stays hovered");

        let several = run(
            0.5,
            vec![
                egui::Event::PointerMoved(field),
                wheel(3.0, egui::MouseWheelUnit::Line, Default::default()),
            ],
            true,
        );
        assert!(
            (value.get() - 0.9).abs() < 1e-4,
            "one event can carry several notches, got {}",
            value.get()
        );
        assert!(several.edit.commit);
        let clamped = run(
            0.6,
            vec![
                egui::Event::PointerMoved(field),
                wheel(1.0, egui::MouseWheelUnit::Line, Default::default()),
            ],
            true,
        );
        assert!((value.get() - 1.0).abs() < 1e-4, "the step clamps, got {}", value.get());
        assert!(clamped.edit.commit);
        run(
            0.7,
            vec![
                egui::Event::PointerMoved(field),
                wheel(-1.0, egui::MouseWheelUnit::Line, Default::default()),
            ],
            true,
        );
        assert!((value.get() - 0.9).abs() < 1e-4, "a notch down lowers, got {}", value.get());

        value.set(0.5);
        run(
            0.8,
            vec![
                egui::Event::PointerMoved(field),
                wheel(30.0, egui::MouseWheelUnit::Point, Default::default()),
            ],
            true,
        );
        assert!((value.get() - 0.5).abs() < 1e-4, "a short trackpad flick is not a whole notch yet");
        run(
            0.9,
            vec![
                egui::Event::PointerMoved(field),
                wheel(30.0, egui::MouseWheelUnit::Point, Default::default()),
            ],
            true,
        );
        assert!(
            (value.get() - 0.6).abs() < 1e-4,
            "points accumulate to one notch (55 points), got {}",
            value.get()
        );

        let parked = value.get();
        let mut scrolled = run(
            1.0,
            vec![
                egui::Event::PointerMoved(gap),
                wheel(-1.0, egui::MouseWheelUnit::Line, Default::default()),
            ],
            true,
        );
        assert_eq!(value.get(), parked, "a wheel that is not over the field does not edit it");
        let mut moved = scrolled.offset != 0.0;
        for step in 1..8 {
            scrolled = run(1.0 + step as f64 * 0.05, vec![egui::Event::PointerMoved(gap)], true);
            moved |= scrolled.offset != 0.0;
        }
        assert!(moved, "the same wheel over the panel still scrolls it");

        let parked = value.get();
        let parked_offset = scrolled.offset;
        let mut disabled = run(
            1.5,
            vec![
                egui::Event::PointerMoved(field),
                wheel(1.0, egui::MouseWheelUnit::Line, Default::default()),
            ],
            false,
        );
        assert_eq!(value.get(), parked, "a disabled field does not step");
        let mut disabled_scrolls = disabled.offset != parked_offset;
        for step in 1..8 {
            disabled = run(1.5 + step as f64 * 0.05, vec![egui::Event::PointerMoved(field)], false);
            disabled_scrolls |= disabled.offset != parked_offset;
        }
        assert!(disabled_scrolls, "a disabled field must not eat the panel's wheel");
    }

    #[test]
    fn arrow_and_wheel_modifiers_scale_the_increment() {
        let ctx = egui::Context::default();
        let value = std::cell::Cell::new(1.0f32);
        let run = |events: Vec<egui::Event>| {
            let mut shown = Shown {
                edit: Edit::default(),
                field: egui::Pos2::ZERO,
                gap: egui::Pos2::ZERO,
                arrow: egui::Pos2::ZERO,
                offset: 0.0,
            };
            let mut current = value.get();
            let mut output = ctx.run_ui(input(0.0, events), |ui| {
                shown.field = ui.cursor().min + egui::vec2(20.0, 10.0);
                shown.arrow = ui.cursor().min + egui::vec2(83.5, 6.5);
                shown.edit = spin(ui, &mut current, 0.1, 0.0..=100.0, 3, 90.0, false);
            });
            output.textures_delta.clear();
            value.set(current);
            shown
        };
        let laid = run(vec![]);
        let arrow = laid.arrow;
        let field = laid.field;
        let alt = egui::Modifiers {
            alt: true,
            ..Default::default()
        };
        let ctrl = egui::Modifiers {
            ctrl: true,
            ..Default::default()
        };
        let shift = egui::Modifiers {
            shift: true,
            ..Default::default()
        };
        run(vec![egui::Event::ModifiersChanged(alt), egui::Event::PointerMoved(arrow)]);
        run(vec![button(arrow, true, alt)]);
        let edit = run(vec![button(arrow, false, alt)]);
        assert!((value.get() - 2.0).abs() < 1e-3, "Alt on an arrow is ×10, got {}", value.get());
        assert!(edit.edit.changed && edit.edit.commit);
        run(vec![egui::Event::ModifiersChanged(ctrl), egui::Event::PointerMoved(arrow)]);
        run(vec![button(arrow, true, ctrl)]);
        run(vec![button(arrow, false, ctrl)]);
        assert!((value.get() - 2.01).abs() < 1e-3, "Ctrl on an arrow is ×0.1, got {}", value.get());
        run(vec![egui::Event::ModifiersChanged(shift), egui::Event::PointerMoved(arrow)]);
        run(vec![button(arrow, true, shift)]);
        run(vec![button(arrow, false, shift)]);
        assert!(
            (value.get() - 2.011).abs() < 1e-3,
            "Shift on an arrow is ×0.01, got {}",
            value.get()
        );
        run(vec![egui::Event::ModifiersChanged(Default::default())]);
        let edit = run(vec![egui::Event::PointerMoved(field), wheel(1.0, egui::MouseWheelUnit::Line, alt)]);
        assert!((value.get() - 3.011).abs() < 1e-3, "Alt on the wheel is ×10, got {}", value.get());
        assert!(edit.edit.commit);
        run(vec![egui::Event::PointerMoved(field), wheel(1.0, egui::MouseWheelUnit::Line, ctrl)]);
        assert!((value.get() - 3.021).abs() < 1e-3, "Ctrl on the wheel is ×0.1, got {}", value.get());
    }

    #[test]
    fn held_arrow_waits_then_repeats_until_release() {
        let ctx = egui::Context::default();
        let value = std::cell::Cell::new(0.0f32);
        let run = |time: f64, events: Vec<egui::Event>| {
            let mut arrow = egui::Pos2::ZERO;
            let mut current = value.get();
            let mut output = ctx.run_ui(input(time, events), |ui| {
                arrow = ui.cursor().min + egui::vec2(83.5, 6.5);
                let _ = spin(ui, &mut current, 0.1, 0.0..=100.0, 2, 90.0, false);
            });
            output.textures_delta.clear();
            value.set(current);
            arrow
        };
        let arrow = run(0.0, vec![]);
        run(0.0, vec![egui::Event::PointerMoved(arrow), button(arrow, true, Default::default())]);
        assert_eq!(value.get(), 0.0, "the press itself does not step; the release does");
        run(0.49, vec![egui::Event::PointerMoved(arrow)]);
        assert_eq!(value.get(), 0.0, "nothing repeats before the 0.5 s hold");
        run(0.50, vec![egui::Event::PointerMoved(arrow)]);
        assert!(
            (value.get() - 0.1).abs() < 1e-4,
            "the first repeat is at 0.5 s, got {}",
            value.get()
        );
        run(0.54, vec![egui::Event::PointerMoved(arrow)]);
        assert!((value.get() - 0.1).abs() < 1e-4, "repeats are 50 ms apart, not every frame");
        run(0.55, vec![egui::Event::PointerMoved(arrow)]);
        assert!(
            (value.get() - 0.2).abs() < 1e-4,
            "the next repeat is 50 ms later, got {}",
            value.get()
        );
        run(
            0.60,
            vec![egui::Event::PointerMoved(arrow), button(arrow, false, Default::default())],
        );
        assert!(
            (value.get() - 0.4).abs() < 1e-4,
            "release steps once more, and the repeat that came due, got {}",
            value.get()
        );
        run(1.0, vec![]);
        assert!((value.get() - 0.4).abs() < 1e-4, "it stops when the button is up");

        value.set(0.0);
        let arrow = run(2.0, vec![egui::Event::PointerMoved(arrow), button(arrow, true, Default::default())]);
        run(
            2.2,
            vec![egui::Event::PointerMoved(arrow), button(arrow, false, Default::default())],
        );
        assert!((value.get() - 0.1).abs() < 1e-4, "a short click is one step, got {}", value.get());

        // egui stops calling a press a click after 0.8 s. The release on the
        // arrow is still one step, and the repeats that came due still apply.
        value.set(0.0);
        let arrow = run(3.0, vec![egui::Event::PointerMoved(arrow), button(arrow, true, Default::default())]);
        assert_eq!(value.get(), 0.0);
        run(3.85, vec![egui::Event::PointerMoved(arrow)]);
        assert!(
            (value.get() - 0.8).abs() < 1e-4,
            "a jump past the click window still pays the repeats that came due, got {}",
            value.get()
        );
        run(
            3.90,
            vec![egui::Event::PointerMoved(arrow), button(arrow, false, Default::default())],
        );
        assert!(
            (value.get() - 1.0).abs() < 1e-4,
            "releasing after 0.8 s still steps once more, got {}",
            value.get()
        );
    }

    #[test]
    fn tab_from_one_spinner_lands_on_the_next() {
        let ctx = egui::Context::default();
        let first = std::cell::Cell::new(1.0f32);
        let second = std::cell::Cell::new(2.0f32);
        let show = |events: Vec<egui::Event>| {
            let mut field = egui::Pos2::ZERO;
            let mut a = first.get();
            let mut b = second.get();
            let mut output = ctx.run_ui(input(0.0, events), |ui| {
                field = ui.cursor().min + egui::vec2(20.0, 10.0);
                let _ = spin(ui, &mut a, 0.1, 0.0..=10.0, 2, 90.0, false);
                let _ = spin(ui, &mut b, 0.1, 0.0..=10.0, 2, 90.0, false);
            });
            output.textures_delta.clear();
            first.set(a);
            second.set(b);
            field
        };
        let field = show(vec![]);
        show(vec![egui::Event::PointerMoved(field), button(field, true, Default::default())]);
        show(vec![button(field, false, Default::default())]);
        assert!(ctx.memory(|m| m.focused()).is_some(), "clicking the field focuses it");
        show(vec![egui::Event::Key {
            key: egui::Key::Tab,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }]);
        // The next field is what receives the typed number. An arrow that
        // still takes Tab focus drops the keystroke and both values stay.
        show(vec![egui::Event::Text("9".into())]);
        show(vec![egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }]);
        assert_eq!(first.get(), 1.0, "Tab must leave the field that was just edited");
        assert_eq!(second.get(), 9.0, "Tab must skip both arrows and focus the next field");
    }

    #[test]
    fn drag_and_typed_text_ignore_the_wheel() {
        let drag_to = |with_wheel: bool| {
            let ctx = egui::Context::default();
            let value = std::cell::Cell::new(10.0f32);
            let run = |events: Vec<egui::Event>| {
                let mut field = egui::Pos2::ZERO;
                let mut current = value.get();
                let mut output = ctx.run_ui(input(0.0, events), |ui| {
                    field = ui.cursor().min + egui::vec2(20.0, 10.0);
                    let _ = spin(ui, &mut current, 1.0, 0.0..=100.0, 2, 90.0, false);
                });
                output.textures_delta.clear();
                value.set(current);
                field
            };
            let field = run(vec![]);
            run(vec![egui::Event::PointerMoved(field), button(field, true, Default::default())]);
            let mut events = vec![egui::Event::PointerMoved(field + egui::vec2(40.0, 0.0))];
            if with_wheel {
                events.push(wheel(1.0, egui::MouseWheelUnit::Line, Default::default()));
            }
            run(events);
            run(vec![button(field + egui::vec2(40.0, 0.0), false, Default::default())]);
            value.get()
        };
        let plain = drag_to(false);
        let wheeled = drag_to(true);
        assert!(plain > 10.0, "the drag still edits, got {plain}");
        assert_eq!(plain, wheeled, "a wheel during the drag must not add a step");

        let ctx = egui::Context::default();
        let value = std::cell::Cell::new(10.0f32);
        let run = |events: Vec<egui::Event>| {
            let mut field = egui::Pos2::ZERO;
            let mut edit = Edit::default();
            let mut current = value.get();
            let mut output = ctx.run_ui(input(0.0, events), |ui| {
                field = ui.cursor().min + egui::vec2(20.0, 10.0);
                edit = spin(ui, &mut current, 1.0, 0.0..=100.0, 2, 90.0, false);
            });
            output.textures_delta.clear();
            value.set(current);
            (field, edit)
        };
        let (field, _) = run(vec![]);
        run(vec![egui::Event::PointerMoved(field), button(field, true, Default::default())]);
        run(vec![button(field, false, Default::default())]);
        let (_, edit) = run(vec![egui::Event::Text("4".into())]);
        assert_eq!(value.get(), 10.0);
        assert_eq!(edit, Edit::default());
        let (_, edit) = run(vec![
            egui::Event::PointerMoved(field),
            wheel(1.0, egui::MouseWheelUnit::Line, Default::default()),
        ]);
        assert_eq!(value.get(), 10.0, "the wheel must not replace a typed draft");
        assert!(!edit.commit);
        let (_, edit) = run(vec![egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }]);
        assert_eq!(value.get(), 4.0, "Enter commits the typed number alone");
        assert!(edit.changed && edit.commit);
    }

    #[test]
    fn integer_spinner_rounds_modifier_steps_and_keeps_its_drag() {
        let ctx = egui::Context::default();
        let value = std::cell::Cell::new(5i32);
        let run = |events: Vec<egui::Event>| {
            let mut field = egui::Pos2::ZERO;
            let mut arrow = egui::Pos2::ZERO;
            let mut edit = Edit::default();
            let mut current = value.get();
            let mut output = ctx.run_ui(input(0.0, events), |ui| {
                field = ui.cursor().min + egui::vec2(20.0, 10.0);
                arrow = ui.cursor().min + egui::vec2(83.5, 6.5);
                edit = spin_i(ui, &mut current, 0..=100, 90.0);
            });
            output.textures_delta.clear();
            value.set(current);
            (field, arrow, edit)
        };
        let (_, arrow, _) = run(vec![]);
        let alt = egui::Modifiers {
            alt: true,
            ..Default::default()
        };
        let ctrl = egui::Modifiers {
            ctrl: true,
            ..Default::default()
        };
        run(vec![egui::Event::ModifiersChanged(alt), egui::Event::PointerMoved(arrow)]);
        run(vec![button(arrow, true, alt)]);
        let (_, _, edit) = run(vec![button(arrow, false, alt)]);
        assert_eq!(value.get(), 15, "Alt steps an integer by 10");
        assert!(edit.commit);
        run(vec![egui::Event::ModifiersChanged(ctrl), egui::Event::PointerMoved(arrow)]);
        run(vec![button(arrow, true, ctrl)]);
        let (_, _, edit) = run(vec![button(arrow, false, ctrl)]);
        assert_eq!(value.get(), 15, "Ctrl ×0.1 rounds to no integer step");
        assert!(!edit.changed && !edit.commit);
        run(vec![egui::Event::ModifiersChanged(Default::default())]);
        let (field, _, _) = run(vec![
            egui::Event::PointerMoved(arrow),
            wheel(1.0, egui::MouseWheelUnit::Line, Default::default()),
        ]);
        let _ = field;
        assert_eq!(value.get(), 16);
        run(vec![wheel(1.0, egui::MouseWheelUnit::Line, ctrl)]);
        assert_eq!(value.get(), 16, "Ctrl on the wheel rounds to no integer step");

        let drag = |alt_drag: bool| {
            let ctx = egui::Context::default();
            let value = std::cell::Cell::new(20i32);
            let modifiers = if alt_drag { alt } else { egui::Modifiers::NONE };
            let run = |events: Vec<egui::Event>| {
                let mut field = egui::Pos2::ZERO;
                let mut current = value.get();
                let mut output = ctx.run_ui(input(0.0, events), |ui| {
                    field = ui.cursor().min + egui::vec2(20.0, 10.0);
                    let _ = spin_i(ui, &mut current, 0..=100, 90.0);
                });
                output.textures_delta.clear();
                value.set(current);
                field
            };
            let field = run(vec![]);
            run(vec![
                egui::Event::ModifiersChanged(modifiers),
                egui::Event::PointerMoved(field),
                button(field, true, modifiers),
            ]);
            run(vec![egui::Event::PointerMoved(field + egui::vec2(40.0, 0.0))]);
            run(vec![button(field + egui::vec2(40.0, 0.0), false, modifiers)]);
            value.get()
        };
        assert_eq!(drag(false), drag(true), "an integer drag stays at speed 0.25, with or without Alt");
    }

    #[test]
    fn srgb_round_trip() {
        for v in [0.0, 0.01, 0.2, 0.5, 1.0] {
            assert!((linear_to_srgb(srgb_to_linear(v)) - v).abs() < 1e-5);
        }
        assert_eq!(same([1, 1, 1].into_iter()), (Some(1), false));
        assert_eq!(same([1, 2].into_iter()), (Some(1), true));
    }
}
