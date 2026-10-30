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

/// What a value control did this frame: changed (live) and / or must be
/// sent (typed value, end of a drag).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Edit {
    pub changed: bool,
    pub commit: bool,
}

impl Edit {
    pub fn or(self, o: Edit) -> Edit {
        Edit {
            changed: self.changed || o.changed,
            commit: self.commit || o.commit,
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
/// (several values selected).
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
    // Firestorm's spinner modifiers: Alt ×10, Ctrl ×0.1, Shift ×0.01
    let m = ui.input(|i| i.modifiers);
    let speed = increment
        * 0.25
        * if m.alt {
            10.0
        } else if m.ctrl {
            0.1
        } else if m.shift {
            0.01
        } else {
            1.0
        };
    let before = *v;
    let mut dv = egui::DragValue::new(v)
        .speed(speed)
        .update_while_editing(false)
        .range(range.clone())
        .custom_parser(aurora_i18n::parse_number)
        .max_decimals(decimals)
        .min_decimals(min_decimals.min(decimals));
    if tentative {
        dv = dv.custom_formatter(|_, _| "—".into());
    }
    let height = if super::legacy::active(ui) { 20.0 } else { 26.0 };
    let (r, step) = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let r = ui.add_sized([(width - 13.0).max(16.0), height], dv);
            let step = crate::ui::widgets::step_buttons(ui, height);
            (r, step)
        })
        .inner;
    if step != 0 {
        *v = (*v + increment * step as f32).clamp(*range.start(), *range.end());
    }
    if r.dragged() {
        ui.data_mut(|data| data.insert_temp(draft_id, SpinnerDrag { widget: r.id, value: *v }));
    }
    let mut e = Edit {
        changed: step != 0,
        commit: step != 0,
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
    e
}

/// Integer spinner.
pub fn spin_i(ui: &mut egui::Ui, v: &mut i32, range: std::ops::RangeInclusive<i32>, width: f32) -> Edit {
    let before = *v;
    let height = if super::legacy::active(ui) { 20.0 } else { 26.0 };
    let (r, step) = ui
        .horizontal(|ui| {
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
        })
        .inner;
    if step != 0 {
        *v = v.saturating_add(step as i32).clamp(*range.start(), *range.end());
    }
    Edit {
        changed: (r.changed() && *v != before) || step != 0,
        commit: step != 0 || (r.changed() && *v != before && !r.dragged()) || r.drag_stopped(),
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

    #[test]
    fn srgb_round_trip() {
        for v in [0.0, 0.01, 0.2, 0.5, 1.0] {
            assert!((linear_to_srgb(srgb_to_linear(v)) - v).abs() < 1e-5);
        }
        assert_eq!(same([1, 1, 1].into_iter()), (Some(1), false));
        assert_eq!(same([1, 2].into_iter()), (Some(1), true));
    }
}
