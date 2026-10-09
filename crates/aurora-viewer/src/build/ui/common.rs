//! Small building blocks of the build floater: labels, Firestorm-style
//! spinners (LLSpinCtrl: drag or type, commit on release / Enter), three-
//! state check boxes, combo boxes, texture and color swatches.

use crate::theme::Palette;
use egui::{Color32, RichText, Vec2};
use std::collections::HashMap;
use uuid::Uuid;

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
    ui.add_space(4.0);
    ui.label(RichText::new(title).size(12.0).strong().color(p.violet_light));
    ui.add_space(1.0);
}

/// A label of fixed width then the content (Firestorm's two-column rows).
pub fn row<R>(ui: &mut egui::Ui, p: &Palette, name: &str, width: f32, body: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(Vec2::new(width, 20.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.set_min_width(width);
            label(ui, p, name)
        });
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
    let mut dv = egui::DragValue::new(v)
        .speed(speed)
        .range(range)
        .max_decimals(decimals)
        .min_decimals(decimals.min(5));
    if tentative {
        dv = dv.custom_formatter(|_, _| "—".into());
    }
    let r = ui.add_sized([width, 19.0], dv);
    let mut e = Edit::default();
    if r.changed() {
        e.changed = true;
        if !r.dragged() {
            e.commit = true;
        }
    }
    if r.drag_stopped() {
        e.commit = true;
    }
    e
}

/// Integer spinner.
pub fn spin_i(ui: &mut egui::Ui, v: &mut i32, range: std::ops::RangeInclusive<i32>, width: f32) -> Edit {
    let r = ui.add_sized([width, 19.0], egui::DragValue::new(v).speed(0.25).range(range));
    Edit {
        changed: r.changed(),
        commit: (r.changed() && !r.dragged()) || r.drag_stopped(),
    }
}

/// Check box that can show a mixed state; returns the new value when clicked.
pub fn check(ui: &mut egui::Ui, value: bool, tentative: bool, label: &str, enabled: bool) -> Option<bool> {
    let mut v = value && !tentative;
    let r = ui.add_enabled(
        enabled,
        egui::Checkbox::new(&mut v, RichText::new(label).size(12.0)).indeterminate(tentative),
    );
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
/// « Multiples » overlay when the selected faces differ. Clicked: open the picker.
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
        painter.rect_filled(rect, 2.0, p.field);
        match id.filter(|i| !i.is_nil()) {
            Some(tex) => {
                wanted.insert(tex);
                match images.get(&tex) {
                    Some(t) => {
                        painter.image(
                            t.id(),
                            rect.shrink(1.0),
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                            if enabled { Color32::WHITE } else { Color32::from_gray(120) },
                        );
                    }
                    None => {
                        painter.text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            "…",
                            egui::FontId::proportional(12.0),
                            p.muted,
                        );
                    }
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
                "Multiples",
                egui::FontId::proportional(11.0),
                p.ink,
            );
        }
        let stroke = if resp.hovered() && enabled { p.violet_light } else { p.raised };
        painter.rect_stroke(rect, 2.0, egui::Stroke::new(1.0, stroke), egui::StrokeKind::Inside);
        ui.add_sized(
            [size, 14.0],
            egui::Label::new(RichText::new(caption).size(11.0).color(if enabled { p.muted } else { p.muted_dim })),
        );
        resp
    })
    .inner
}

/// A color swatch (LLColorSwatchCtrl) with egui's color picker; returns
/// the new color (sRGB 0..1) when changed, and whether the picker closed.
pub fn color_swatch(ui: &mut egui::Ui, p: &Palette, rgb: [f32; 3], caption: &str, size: f32, enabled: bool) -> Option<[f32; 3]> {
    let mut out = None;
    ui.vertical(|ui| {
        let mut c = rgb;
        ui.add_enabled_ui(enabled, |ui| {
            ui.spacing_mut().interact_size = Vec2::new(size, size);
            if egui::color_picker::color_edit_button_rgb(ui, &mut c).changed() {
                out = Some(c);
            }
        });
        ui.add_sized(
            [size, 14.0],
            egui::Label::new(RichText::new(caption).size(11.0).color(if enabled { p.muted } else { p.muted_dim })),
        );
    });
    out
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
    fn srgb_round_trip() {
        for v in [0.0, 0.01, 0.2, 0.5, 1.0] {
            assert!((linear_to_srgb(srgb_to_linear(v)) - v).abs() < 1e-5);
        }
        assert_eq!(same([1, 1, 1].into_iter()), (Some(1), false));
        assert_eq!(same([1, 2].into_iter()), (Some(1), true));
    }
}
