//! The viewfinder's camera presets panel (issue #1164): the saved presets,
//! each applied with a click, with what it holds the camera by and what it
//! keeps; under them, the current shot saved by name with the parts chosen
//! and what holds the camera (the avatar, the place, or an object chosen in
//! the scene like the isolation's). The rule is `camera::presets`, the app
//! side `app::camera_presets`; the Controls window lists the same presets.

use super::controls::segmented;
use super::{Frame, Out, Viewfinder, bold, icon, keep_keys, shadow_text, widgets};
use crate::camera::CameraShot;
use crate::camera::presets::{Anchor, AnchorKind, Include};
use crate::theme::Palette;
use egui::{Align2, Id, Order, Pos2, Rect, Sense, Vec2};

/// What the panel asks of the app.
#[derive(Debug, Clone, PartialEq)]
pub enum Preset {
    Apply(usize),
    Delete(usize),
    Save {
        name: String,
        include: Include,
    },
    /// Choosing the anchor object in the scene: a click and the pointer
    /// (points).
    Click(Pos2),
    Hover(Pos2),
}

/// The saving form, kept while the photo mode is open.
#[derive(Debug, Clone, Default)]
pub struct PresetForm {
    pub name: String,
    pub include: Include,
    /// The object an object anchor holds (chosen in the scene, or the one
    /// the camera is on when the panel opens).
    pub object: Option<uuid::Uuid>,
    /// Choosing it in the scene.
    pub picking: bool,
}

const WIDTH: f32 = 312.0;
const ROW_H: f32 = 28.0;

/// The icon of what holds a preset's camera (here and in the Controls
/// window).
pub fn anchor_icon(anchor: Option<&Anchor>) -> &'static str {
    match anchor.map(Anchor::kind) {
        Some(AnchorKind::Avatar) => "user-filled",
        Some(AnchorKind::World) => "map-pin-filled",
        Some(AnchorKind::Object) => "cube",
        None => "camera-filled",
    }
}

/// A preset's tooltip: what holds its camera and what it keeps.
fn preset_tip(shot: &CameraShot) -> String {
    let anchor = match &shot.anchor {
        Some(Anchor::Avatar) => aurora_i18n::tr!("camera-preset-on-avatar").to_owned(),
        Some(Anchor::World { .. }) => aurora_i18n::tr!("camera-preset-on-place").to_owned(),
        Some(Anchor::Object { name, .. }) => aurora_i18n::tr!("camera-preset-on-object", name = name.as_str()),
        None => aurora_i18n::tr!("camera-preset-no-position").to_owned(),
    };
    let mut parts = vec![anchor];
    for (kept, label) in [
        (shot.lens.is_some(), aurora_i18n::tr!("camera-preset-lens")),
        (shot.looks.is_some(), aurora_i18n::tr!("camera-preset-looks")),
        (shot.light.is_some(), aurora_i18n::tr!("camera-preset-light")),
    ] {
        if kept {
            parts.push(label.to_owned());
        }
    }
    parts.join(" · ")
}

impl Viewfinder {
    /// The panel, left of the dock.
    pub(super) fn presets_panel(&mut self, ctx: &egui::Context, p: &Palette, screen: Rect, f: &Frame, out: &mut Vec<Out>) {
        egui::Area::new(Id::unique("vf_presets"))
            .order(Order::Foreground)
            .fade_in(false)
            .pivot(Align2::RIGHT_CENTER)
            .fixed_pos(Pos2::new(screen.right() - 84.0, screen.center().y))
            .show(ctx, |ui| {
                widgets::overlay(ui, p, egui::Margin::same(12), |ui| {
                    ui.set_width(WIDTH);
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(aurora_i18n::tr!("camera-presets"))
                                .font(bold(ctx, 14.0))
                                .color(p.ink),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if widgets::icon_button(ui, p, "x", aurora_i18n::tr!("common-close"), true) {
                                self.presets = false;
                                self.preset_form.picking = false;
                            }
                        });
                    });
                    ui.add_space(6.0);
                    self.presets_list(ui, p, f, out);
                    ui.add_space(8.0);
                    widgets::divider(ui, p);
                    ui.add_space(6.0);
                    self.preset_save(ui, p, f, out);
                });
            });
    }

    /// The presets, a row each: applied with a click, deleted with the bin.
    fn presets_list(&mut self, ui: &mut egui::Ui, p: &Palette, f: &Frame, out: &mut Vec<Out>) {
        if f.presets.is_empty() {
            ui.label(
                egui::RichText::new(aurora_i18n::tr!("camera-presets-none"))
                    .size(12.0)
                    .color(p.muted),
            );
            return;
        }
        egui::ScrollArea::vertical()
            .id_salt("vf_presets_list")
            .max_height(6.0 * (ROW_H + 2.0))
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                for (i, shot) in f.presets.iter().enumerate() {
                    ui.horizontal(|ui| {
                        let bin = 26.0;
                        let room = ui.available_width() - bin - ui.spacing().item_spacing.x;
                        let (rect, resp) = ui.allocate_exact_size(Vec2::new(room, ROW_H), Sense::click());
                        keep_keys(&resp);
                        if resp.hovered() {
                            ui.painter()
                                .rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, p.ink.gamma_multiply(0.08));
                        }
                        let painter = ui.painter();
                        let lead = Rect::from_center_size(Pos2::new(rect.left() + 14.0, rect.center().y), Vec2::splat(16.0));
                        icon(painter, anchor_icon(shot.anchor.as_ref()), lead, p.muted);
                        // what it keeps, right-aligned
                        let mut x = rect.right() - 6.0;
                        for (kept, glyph) in [
                            (shot.light.is_some(), "sun-filled"),
                            (shot.looks.is_some(), "color-filter"),
                            (shot.lens.is_some(), "aperture"),
                        ] {
                            if kept {
                                let r = Rect::from_center_size(Pos2::new(x - 7.0, rect.center().y), Vec2::splat(14.0));
                                icon(painter, glyph, r, p.muted);
                                x -= 18.0;
                            }
                        }
                        let name = Rect::from_x_y_ranges(rect.left() + 28.0..=x - 6.0, rect.y_range());
                        // one line: the end of a long name gives way
                        let mut job = egui::text::LayoutJob::simple_singleline(shot.name.clone(), egui::FontId::proportional(12.5), p.ink);
                        job.wrap = egui::text::TextWrapping::truncate_at_width(name.width().max(1.0));
                        let galley = painter.layout_job(job);
                        painter.galley(Pos2::new(name.left(), name.center().y - galley.size().y * 0.5), galley, p.ink);
                        if resp.on_hover_text(preset_tip(shot)).clicked() {
                            out.push(Out::Preset(Preset::Apply(i)));
                        }
                        if widgets::icon_button(ui, p, "trash-filled", aurora_i18n::tr!("camera-preset-delete"), true) {
                            out.push(Out::Preset(Preset::Delete(i)));
                        }
                    });
                }
            });
    }

    /// Saving the current shot: its name, what holds the camera, what it
    /// keeps.
    fn preset_save(&mut self, ui: &mut egui::Ui, p: &Palette, f: &Frame, out: &mut Vec<Out>) {
        ui.label(
            egui::RichText::new(aurora_i18n::tr!("camera-preset-save-title"))
                .font(bold(ui.ctx(), 12.5))
                .color(p.ink),
        );
        ui.add_space(4.0);
        let form = &mut self.preset_form;
        let field = ui.add_sized(
            Vec2::new(ui.available_width(), 26.0),
            egui::TextEdit::singleline(&mut form.name).hint_text(aurora_i18n::tr!("camera-preset-name")),
        );
        let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        ui.add_space(6.0);

        let mut position = form.include.anchor.is_some();
        if widgets::checkbox(ui, p, &mut position, aurora_i18n::tr!("camera-preset-position")).changed() {
            form.include.anchor = position.then_some(AnchorKind::Avatar);
            form.picking &= position;
        }
        if let Some(kind) = form.include.anchor {
            ui.horizontal(|ui| {
                ui.add_space(24.0);
                let labels = [
                    (
                        AnchorKind::Avatar,
                        aurora_i18n::tr!("camera-preset-anchor-avatar"),
                        aurora_i18n::tr!("camera-preset-anchor-avatar-tip"),
                    ),
                    (
                        AnchorKind::World,
                        aurora_i18n::tr!("camera-preset-anchor-place"),
                        aurora_i18n::tr!("camera-preset-anchor-place-tip"),
                    ),
                    (
                        AnchorKind::Object,
                        aurora_i18n::tr!("camera-preset-anchor-object"),
                        aurora_i18n::tr!("camera-preset-anchor-object-tip"),
                    ),
                ];
                let items: Vec<(&str, bool, &str)> = labels.iter().map(|(k, t, tip)| (*t, *k == kind, *tip)).collect();
                if let Some(i) = segmented(ui, p, &items, Some(ui.available_width())) {
                    form.include.anchor = Some(labels[i].0);
                    form.picking = labels[i].0 == AnchorKind::Object && form.object.is_none();
                }
            });
            if kind == AnchorKind::Object {
                ui.horizontal(|ui| {
                    ui.add_space(24.0);
                    let (glyph, label) = if form.picking {
                        ("check", aurora_i18n::tr!("camera-preset-pick-done").to_owned())
                    } else {
                        match &f.preset_object {
                            Some(name) => ("cube", name.clone()),
                            None => ("pointer-filled", aurora_i18n::tr!("camera-preset-pick").to_owned()),
                        }
                    };
                    let kind = if form.picking {
                        widgets::ActionKind::Primary
                    } else {
                        widgets::ActionKind::Flat
                    };
                    let resp = widgets::action_button(ui, p, glyph, &label, kind, Vec2::new(ui.available_width(), 28.0))
                        .on_hover_text(aurora_i18n::tr!("camera-preset-pick-tip"));
                    keep_keys(&resp);
                    if resp.clicked() {
                        form.picking = !form.picking;
                    }
                });
            }
        }
        widgets::checkbox(ui, p, &mut form.include.lens, aurora_i18n::tr!("camera-preset-lens"))
            .on_hover_text(aurora_i18n::tr!("camera-preset-lens-tip"));
        widgets::checkbox(ui, p, &mut form.include.looks, aurora_i18n::tr!("camera-preset-looks"))
            .on_hover_text(aurora_i18n::tr!("camera-preset-looks-tip"));
        widgets::checkbox(ui, p, &mut form.include.light, aurora_i18n::tr!("camera-preset-light"))
            .on_hover_text(aurora_i18n::tr!("camera-preset-light-tip"));
        ui.add_space(8.0);

        let name = form.name.trim().to_owned();
        let object_missing = form.include.anchor == Some(AnchorKind::Object) && form.object.is_none();
        let ready = !name.is_empty() && form.include.any() && !object_missing;
        let replaces = f.presets.iter().any(|s| s.name == name);
        let label = if replaces {
            aurora_i18n::tr!("camera-preset-replace")
        } else {
            aurora_i18n::tr!("camera-preset-save")
        };
        let resp = ui.add_enabled_ui(ready, |ui| {
            widgets::primary_button_sized(ui, p, label, Vec2::new(ui.available_width(), 30.0))
        });
        keep_keys(&resp.inner);
        if ready && (resp.inner.clicked() || enter) {
            out.push(Out::Preset(Preset::Save {
                name,
                include: form.include,
            }));
            form.name.clear();
            form.picking = false;
        }
        let hint = if form.picking {
            Some(aurora_i18n::tr!("camera-preset-pick-hint"))
        } else if object_missing {
            Some(aurora_i18n::tr!("camera-preset-choose-object"))
        } else if !form.include.any() {
            Some(aurora_i18n::tr!("camera-preset-nothing"))
        } else {
            None
        };
        if let Some(hint) = hint {
            ui.add_space(6.0);
            ui.label(egui::RichText::new(hint).size(11.5).color(p.muted));
        }
    }

    /// Choosing the anchor object over the photo's frame: the pointer
    /// names what is under it, a click takes it.
    pub(super) fn preset_picking(&mut self, ctx: &egui::Context, p: &Palette, frame: Rect, f: &Frame, out: &mut Vec<Out>) {
        if !self.preset_form.picking {
            return;
        }
        egui::Area::new(Id::unique("vf_preset_pick"))
            .order(Order::Middle)
            .fade_in(false)
            .fixed_pos(frame.min)
            .show(ctx, |ui| {
                let (rect, resp) = ui.allocate_exact_size(frame.size(), Sense::click());
                keep_keys(&resp);
                let Some(at) = resp.hover_pos() else {
                    return;
                };
                ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
                out.push(Out::Preset(Preset::Hover(at)));
                if resp.clicked() && rect.contains(at) {
                    out.push(Out::Preset(Preset::Click(at)));
                }
                if let Some((name, _)) = &f.isolate_hover {
                    shadow_text(
                        ui.painter(),
                        p,
                        at + Vec2::new(16.0, 14.0),
                        Align2::LEFT_TOP,
                        &aurora_i18n::tr!("camera-preset-hover", name = name.as_str()),
                        bold(ctx, 13.0),
                        p.ink,
                    );
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_preset_tells_what_it_holds_and_keeps() {
        let mut shot = CameraShot::at_avatar("Close".into(), [-1.0, 0.0, 0.5], [0.0; 3]);
        assert_eq!(anchor_icon(shot.anchor.as_ref()), "user-filled");
        assert_eq!(preset_tip(&shot), aurora_i18n::tr!("camera-preset-on-avatar"));
        shot.anchor = Some(Anchor::Object {
            id: uuid::Uuid::from_u128(1),
            name: "Chair".into(),
            camera: [0.0; 3],
            focus: [0.0; 3],
        });
        shot.light = Some(crate::camera::presets::Light {
            environment: None,
            time_of_day: 2,
        });
        assert_eq!(anchor_icon(shot.anchor.as_ref()), "cube");
        let tip = preset_tip(&shot);
        assert!(tip.contains("Chair") && tip.contains(aurora_i18n::tr!("camera-preset-light")));
        assert!(!tip.contains(aurora_i18n::tr!("camera-preset-lens")));
        shot.anchor = None;
        assert_eq!(anchor_icon(None), "camera-filled");
    }
}
