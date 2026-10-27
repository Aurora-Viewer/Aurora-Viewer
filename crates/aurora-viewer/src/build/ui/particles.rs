//! Build â€º Edit particles: the particle editor window (ParticleEditor,
//! floater_particle_editor.xml, indra/newview/particleeditor.cpp, originally
//! LGPL 2.1) in the Aurora skin. The emitter, the particles and their motion
//! in three sections, then the flags; Copy puts the script on the clipboard,
//! Inject writes it into the prim (`app/particle_editor.rs`). Every change
//! shows at once on the prim (`ParticleManager::set_preview`).

use super::common::{self, combo, label, section, spin, spin_i};
use crate::build::particle_editor::{Blend, FLAGS, Params, Pattern, range};
use crate::theme::Palette;
use crate::ui::texture_picker::TexturePicker;
use crate::ui::widgets::{self, ActionKind, Floater};
use aurora_prim::particles::{self as ps, PartSysData};
use egui::{RichText, Vec2};
use glam::Vec3;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// Width of a number field.
const W: f32 = 76.0;
/// Width of a field of a vector or a scale (two or three in a row).
const WV: f32 = 66.0;
/// Width of the labels' column.
const LABEL: f32 = 104.0;

/// What the window asks of the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Copy,
    Inject,
    /// Arm the object picker for the target (LLToolObjPicker).
    Pick,
    CancelPick,
}

/// A line under the buttons: what Copy or Inject did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub text: String,
    pub error: bool,
}

#[derive(Default)]
pub struct ParticleEditor {
    pub open: bool,
    /// The prim the system is tuned on (the primary selected object).
    pub object: Uuid,
    pub name: String,
    pub params: Params,
    /// The target key as typed (an invalid key is no target).
    target_text: String,
    /// The object picker is armed for the target.
    pub picking: bool,
    /// Inject is in progress: the window waits (setCanClose(false)).
    pub busy: bool,
    pub notice: Option<Notice>,
    pub picker: TexturePicker,
    pub wanted_images: HashSet<Uuid>,
}

impl ParticleEditor {
    /// Open on the prim `object`. Another prim starts from the window's
    /// initial values, as a new LLFloaterReg instance does; the same prim
    /// keeps what was being tuned.
    pub fn open_on(&mut self, object: Uuid, name: String) {
        if object != self.object {
            self.params = Params::default();
            self.target_text.clear();
            self.notice = None;
        }
        self.object = object;
        self.name = name;
        self.picking = false;
        self.open = true;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.picking = false;
        self.picker.open = false;
    }

    /// The object picker's choice (onTargetPicked).
    pub fn set_target(&mut self, id: Uuid) {
        self.params.target = id;
        self.target_text = id.to_string();
    }

    /// The system to show on the prim while the window is open.
    pub fn preview(&self) -> Option<(Uuid, PartSysData)> {
        (self.open && !self.object.is_nil()).then(|| (self.object, self.params.system()))
    }

    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, images: &HashMap<Uuid, egui::TextureHandle>) -> Option<Action> {
        if !self.open {
            return None;
        }
        let mut open = true;
        let mut action = None;
        let title = aurora_i18n::tr!("particle-editor-title");
        let mut floater = Floater::new("particle_editor", title, egui::pos2(90.0, 110.0), Vec2::new(468.0, 820.0));
        floater.min_size = Vec2::new(468.0, 320.0);
        floater
            .title_note(self.name.clone())
            .show(ctx, p, &mut open, |ui| action = self.body(ui, p, images));
        // the window waits for Inject (setCanClose(false))
        if !open && !self.busy {
            self.close();
            if action == Some(Action::Pick) {
                action = None;
            }
        }
        action
    }

    fn body(&mut self, ui: &mut egui::Ui, p: &Palette, images: &HashMap<Uuid, egui::TextureHandle>) -> Option<Action> {
        common::typography(ui);
        let mut action = None;
        let busy = self.busy;
        egui::ScrollArea::vertical()
            .auto_shrink([false, true])
            .max_height((ui.available_height() - 64.0).max(80.0))
            .show(ui, |ui| {
                ui.add_enabled_ui(!busy, |ui| {
                    self.emitter(ui, p, images);
                    ui.add_space(8.0);
                    self.particles(ui, p);
                    ui.add_space(8.0);
                    if let Some(a) = self.motion(ui, p) {
                        action = Some(a);
                    }
                    ui.add_space(8.0);
                    self.flags(ui, p);
                });
            });
        widgets::divider(ui, p);
        ui.horizontal(|ui| {
            if let Some(n) = &self.notice {
                let color = if n.error { p.danger } else { p.muted };
                ui.add(egui::Label::new(RichText::new(&n.text).size(11.0).color(color)).wrap());
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let inject = widgets::action_button_if(
                    ui,
                    p,
                    "code",
                    aurora_i18n::tr!("particle-editor-inject"),
                    ActionKind::Primary,
                    Vec2::new(0.0, 28.0),
                    !busy,
                )
                .on_hover_text(aurora_i18n::tr!("particle-editor-inject-tip"));
                if inject.clicked() {
                    action = Some(Action::Inject);
                }
                let copy = widgets::action_button(
                    ui,
                    p,
                    "copy-filled",
                    aurora_i18n::tr!("common-copy"),
                    ActionKind::Flat,
                    Vec2::new(0.0, 28.0),
                )
                .on_hover_text(aurora_i18n::tr!("particle-editor-copy-tip"));
                if copy.clicked() {
                    action = Some(Action::Copy);
                }
            });
        });
        action
    }

    /// Pattern, texture and the bursts.
    fn emitter(&mut self, ui: &mut egui::Ui, p: &Palette, images: &HashMap<Uuid, egui::TextureHandle>) {
        section(ui, p, aurora_i18n::tr!("particle-editor-emitter"));
        let v = &mut self.params;
        let on = v.enabled();
        ui.horizontal(|ui| {
            let texture = common::swatch(
                ui,
                p,
                images,
                &mut self.wanted_images,
                Some(v.texture),
                aurora_i18n::tr!("particle-editor-texture"),
                56.0,
                false,
                true,
            )
            .on_hover_text(aurora_i18n::tr!("particle-editor-texture-tip"));
            if texture.clicked() {
                self.picker
                    .open_with_defaults(aurora_i18n::tr!("particle-editor-texture"), v.texture, None, true);
            }
            ui.add_space(8.0);
            ui.vertical(|ui| {
                label(ui, p, aurora_i18n::tr!("particle-editor-pattern"));
                let patterns: Vec<_> = Pattern::ALL.iter().map(|x| (x.label(), *x)).collect();
                if let Some(x) = combo(ui, "particle_pattern", Some(v.pattern), &patterns, 180.0, true) {
                    v.pattern = x;
                }
            });
        });
        ui.add_space(4.0);
        egui::Grid::new("particle_emitter")
            .num_columns(4)
            .spacing([8.0, 4.0])
            .show(ui, |ui| {
                field(ui, p, aurora_i18n::tr!("particle-editor-burst-rate"), true, |ui| {
                    spin(ui, &mut v.burst_rate, 0.1, range::BURST_RATE.0..=range::BURST_RATE.1, 3, W, false);
                });
                field(ui, p, aurora_i18n::tr!("particle-editor-burst-count"), true, |ui| {
                    let mut count = v.burst_count as i32;
                    let (lo, hi) = range::BURST_COUNT;
                    if spin_i(ui, &mut count, lo as i32..=hi as i32, W).changed {
                        v.burst_count = count.max(1) as u32;
                    }
                });
                ui.end_row();
                field(ui, p, aurora_i18n::tr!("particle-editor-burst-radius"), on.burst_radius, |ui| {
                    spin(
                        ui,
                        &mut v.burst_radius,
                        0.1,
                        range::BURST_RADIUS.0..=range::BURST_RADIUS.1,
                        3,
                        W,
                        false,
                    );
                });
                field(ui, p, aurora_i18n::tr!("particle-editor-source-max-age"), true, |ui| {
                    spin(
                        ui,
                        &mut v.source_max_age,
                        1.0,
                        range::SOURCE_MAX_AGE.0..=range::SOURCE_MAX_AGE.1,
                        3,
                        W,
                        false,
                    );
                });
                ui.end_row();
                let speed = range::SPEED.0..=range::SPEED.1;
                field(ui, p, aurora_i18n::tr!("particle-editor-speed-min"), on.speed, |ui| {
                    spin(ui, &mut v.speed_min, 0.1, speed.clone(), 3, W, false);
                });
                field(ui, p, aurora_i18n::tr!("particle-editor-speed-max"), on.speed, |ui| {
                    spin(ui, &mut v.speed_max, 0.1, speed.clone(), 3, W, false);
                });
                ui.end_row();
                let angle = range::ANGLE.0..=range::ANGLE.1;
                field(ui, p, aurora_i18n::tr!("particle-editor-angle-begin"), on.angles, |ui| {
                    spin(ui, &mut v.angle_begin, 0.1, angle.clone(), 3, W, false);
                });
                field(ui, p, aurora_i18n::tr!("particle-editor-angle-end"), on.angles, |ui| {
                    spin(ui, &mut v.angle_end, 0.1, angle.clone(), 3, W, false);
                });
                ui.end_row();
            });
    }

    /// Life, colours, alpha, glow, scale and blending of the particles.
    fn particles(&mut self, ui: &mut egui::Ui, p: &Palette) {
        section(ui, p, aurora_i18n::tr!("particle-editor-particles"));
        let v = &mut self.params;
        let on = v.enabled();
        let unit = range::UNIT.0..=range::UNIT.1;
        let scale = range::SCALE.0..=range::SCALE.1;
        egui::Grid::new("particle_particles")
            .num_columns(4)
            .spacing([8.0, 4.0])
            .show(ui, |ui| {
                field(ui, p, aurora_i18n::tr!("particle-editor-part-max-age"), true, |ui| {
                    spin(
                        ui,
                        &mut v.part_max_age,
                        0.1,
                        range::PART_MAX_AGE.0..=range::PART_MAX_AGE.1,
                        3,
                        W,
                        false,
                    );
                });
                ui.end_row();
                field(ui, p, aurora_i18n::tr!("particle-editor-start-color"), true, |ui| {
                    color(ui, p, "particle_start_color", &mut v.start_color);
                });
                field(ui, p, aurora_i18n::tr!("particle-editor-end-color"), on.end_color, |ui| {
                    color(ui, p, "particle_end_color", &mut v.end_color);
                });
                ui.end_row();
                field(ui, p, aurora_i18n::tr!("particle-editor-start-alpha"), true, |ui| {
                    spin(ui, &mut v.start_alpha, 0.1, unit.clone(), 3, W, false);
                });
                field(ui, p, aurora_i18n::tr!("particle-editor-end-alpha"), on.end_color, |ui| {
                    spin(ui, &mut v.end_alpha, 0.1, unit.clone(), 3, W, false);
                });
                ui.end_row();
                field(ui, p, aurora_i18n::tr!("particle-editor-start-glow"), true, |ui| {
                    spin(ui, &mut v.start_glow, 0.1, unit.clone(), 3, W, false);
                });
                field(ui, p, aurora_i18n::tr!("particle-editor-end-glow"), true, |ui| {
                    spin(ui, &mut v.end_glow, 0.1, unit.clone(), 3, W, false);
                });
                ui.end_row();
            });
        egui::Grid::new("particle_scale").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
            field(ui, p, aurora_i18n::tr!("particle-editor-start-scale"), true, |ui| {
                spin(ui, &mut v.start_scale.x, 0.1, scale.clone(), 3, WV, false);
                spin(ui, &mut v.start_scale.y, 0.1, scale.clone(), 3, WV, false);
            });
            ui.end_row();
            field(ui, p, aurora_i18n::tr!("particle-editor-end-scale"), on.end_scale, |ui| {
                spin(ui, &mut v.end_scale.x, 0.1, scale.clone(), 3, WV, false);
                spin(ui, &mut v.end_scale.y, 0.1, scale.clone(), 3, WV, false);
            });
            ui.end_row();
            let blends: Vec<_> = Blend::ALL.iter().map(|b| (b.label(), *b)).collect();
            field(ui, p, aurora_i18n::tr!("particle-editor-blend-source"), true, |ui| {
                if let Some(b) = combo(ui, "particle_blend_source", Some(v.blend_source), &blends, 180.0, true) {
                    v.blend_source = b;
                }
            });
            ui.end_row();
            field(ui, p, aurora_i18n::tr!("particle-editor-blend-dest"), true, |ui| {
                if let Some(b) = combo(ui, "particle_blend_dest", Some(v.blend_dest), &blends, 180.0, true) {
                    v.blend_dest = b;
                }
            });
            ui.end_row();
        });
    }

    /// Acceleration, omega and the target.
    fn motion(&mut self, ui: &mut egui::Ui, p: &Palette) -> Option<Action> {
        section(ui, p, aurora_i18n::tr!("particle-editor-motion"));
        let mut action = None;
        let on = self.params.enabled();
        egui::Grid::new("particle_motion")
            .num_columns(2)
            .spacing([8.0, 4.0])
            .show(ui, |ui| {
                field(ui, p, aurora_i18n::tr!("particle-editor-acceleration"), on.motion, |ui| {
                    vector(ui, &mut self.params.acceleration);
                });
                ui.end_row();
                field(ui, p, aurora_i18n::tr!("particle-editor-omega"), on.motion, |ui| {
                    vector(ui, &mut self.params.omega);
                });
                ui.end_row();
                field(ui, p, aurora_i18n::tr!("particle-editor-target"), on.target, |ui| {
                    let edit = egui::TextEdit::singleline(&mut self.target_text)
                        .desired_width(WV * 3.0 - 52.0)
                        .font(egui::TextStyle::Small)
                        .hint_text(aurora_i18n::tr!("particle-editor-target-hint"));
                    if ui.add(edit).changed() {
                        self.params.target = Uuid::parse_str(self.target_text.trim()).unwrap_or_default();
                    }
                    let tip = aurora_i18n::tr!("particle-editor-pick-tip");
                    if widgets::icon_toggle(ui, p, "color-picker", tip, self.picking) {
                        action = Some(if self.picking { Action::CancelPick } else { Action::Pick });
                    }
                    if widgets::icon_button(
                        ui,
                        p,
                        "x",
                        aurora_i18n::tr!("particle-editor-clear-target"),
                        !self.target_text.is_empty(),
                    ) {
                        self.target_text.clear();
                        self.params.target = Uuid::nil();
                    }
                });
                ui.end_row();
            });
        if !on.target && self.picking {
            action = Some(Action::CancelPick);
        }
        action
    }

    /// The PSYS_PART_FLAGS switches, two columns.
    fn flags(&mut self, ui: &mut egui::Ui, p: &Palette) {
        section(ui, p, aurora_i18n::tr!("particle-editor-flags"));
        let on = self.params.enabled();
        egui::Grid::new("particle_flags")
            .num_columns(2)
            .spacing([24.0, 2.0])
            .show(ui, |ui| {
                for (i, flag) in FLAGS.iter().enumerate() {
                    let enabled = flag.mask != ps::LL_PART_TARGET_POS_MASK || on.target_position;
                    let value = self.params.has(flag.mask);
                    if let Some(v) = common::check(ui, p, value, false, flag.label(), enabled) {
                        self.params.set(flag.mask, v);
                    }
                    if i % 2 == 1 {
                        ui.end_row();
                    }
                }
            });
    }
}

/// A label then its control, greyed out together when it does not apply.
fn field(ui: &mut egui::Ui, p: &Palette, name: &str, enabled: bool, body: impl FnOnce(&mut egui::Ui)) {
    ui.add_enabled_ui(enabled, |ui| {
        ui.allocate_ui_with_layout(Vec2::new(LABEL, 20.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.set_min_width(LABEL);
            label(ui, p, name);
        });
    });
    ui.add_enabled_ui(enabled, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            body(ui);
        });
    });
}

fn vector(ui: &mut egui::Ui, v: &mut Vec3) {
    let r = range::VECTOR.0..=range::VECTOR.1;
    spin(ui, &mut v.x, 0.1, r.clone(), 3, WV, false);
    spin(ui, &mut v.y, 0.1, r.clone(), 3, WV, false);
    spin(ui, &mut v.z, 0.1, r, 3, WV, false);
}

/// An sRGB colour of the system through the shared colour button.
fn color(ui: &mut egui::Ui, p: &Palette, id: &str, rgb: &mut [f32; 3]) {
    let byte = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
    let mut c = [byte(rgb[0]), byte(rgb[1]), byte(rgb[2]), 255];
    if widgets::color_button(ui, p, id, &mut c, false).changed() {
        *rgb = [f32::from(c[0]) / 255.0, f32::from(c[1]) / 255.0, f32::from(c[2]) / 255.0];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn another_prim_starts_from_the_initial_values() {
        let mut e = ParticleEditor::default();
        e.open_on(Uuid::from_u128(1), "Box".into());
        e.params.burst_count = 40;
        e.set_target(Uuid::from_u128(3));
        e.open_on(Uuid::from_u128(1), "Box".into());
        assert_eq!(e.params.burst_count, 40);
        assert_eq!(e.params.target, Uuid::from_u128(3));
        e.open_on(Uuid::from_u128(2), "Ball".into());
        assert_eq!(e.params, Params::default());
        assert_eq!(e.preview().map(|(o, _)| o), Some(Uuid::from_u128(2)));
        e.close();
        assert!(e.preview().is_none());
    }
}
