//! The Avatar tab's Fit lab: which mannequins stand in front of us and what
//! they play (`world::mannequins` makes and dresses them). Session state
//! only, like the rest of LiveMesh: nothing is saved.

use crate::theme::Palette;
use crate::ui::widgets::{self, ActionKind};
use crate::world::fit_poses::Pose;
use crate::world::mannequins::{Body, Motion};
use egui::Vec2;

/// The clipping map's reach: the body up to 3 cm in front of the garment's
/// surface is through it; farther, it is another part of the body in front
/// of it (an arm before the chest).
pub const CLIP_REACH: f32 = 0.03;
/// The band either side of the garment's surface where body and garment
/// meet (3 mm): amber rather than red.
pub const CLIP_TOUCH: f32 = 0.003;

#[derive(Debug, Clone, PartialEq)]
pub struct FitLab {
    /// Whether the mannequins stand in the world.
    pub shown: bool,
    /// The bodies chosen, in `Body::ALL`'s order.
    pub bodies: [bool; Body::ALL.len()],
    pub motion: Motion,
    /// The clipping map, on our avatar and the mannequins.
    pub clip: bool,
    /// Bring the row in front of us again (taken by the app).
    pub place: bool,
    /// The Fit lab's own clips are in the scene's animations.
    pub poses_registered: bool,
}

impl Default for FitLab {
    fn default() -> Self {
        Self {
            shown: false,
            bodies: Body::ALL.map(|body| matches!(body, Body::Slim | Body::Curvy | Body::Full)),
            motion: Motion::Stand,
            clip: false,
            place: false,
            poses_registered: false,
        }
    }
}

impl FitLab {
    /// The row to stand: empty while the Fit lab is off.
    pub fn wanted(&self) -> Vec<Body> {
        if !self.shown {
            return Vec::new();
        }
        Body::ALL
            .into_iter()
            .zip(self.bodies)
            .filter_map(|(body, on)| on.then_some(body))
            .collect()
    }
}

/// The Fit lab's section of the Avatar tab.
pub(super) fn section(fit: &mut FitLab, ui: &mut egui::Ui, p: &Palette) {
    super::ui::note(ui, p, aurora_i18n::tr!("fit-lab-help"));
    ui.horizontal_wrapped(|ui| {
        for (body, on) in Body::ALL.into_iter().zip(fit.bodies.iter_mut()) {
            widgets::checkbox(ui, p, on, body.label());
        }
    });
    widgets::form_row(ui, p, aurora_i18n::tr!("fit-lab-motion"), |ui| {
        ui.horizontal_wrapped(|ui| {
            for motion in Motion::ALL {
                let same = std::mem::discriminant(&fit.motion) == std::mem::discriminant(&motion);
                if widgets::radio(ui, p, same, motion.label()).clicked() && !same {
                    fit.motion = motion;
                }
            }
        });
    });
    if let Motion::Range(pose) = &mut fit.motion {
        widgets::form_row(ui, p, aurora_i18n::tr!("fit-lab-pose"), |ui| {
            let all = aurora_i18n::tr!("fit-lab-pose-all");
            egui::ComboBox::from_id_salt("fit_lab_pose")
                .selected_text((*pose).map_or(all, Pose::label))
                .width(ui.available_width().min(220.0))
                .show_ui(ui, |ui| {
                    ui.selectable_value(pose, None, all);
                    for each in Pose::ALL {
                        ui.selectable_value(pose, Some(each), each.label());
                    }
                });
        });
    }
    widgets::switch_labeled(ui, p, &mut fit.clip, aurora_i18n::tr!("fit-lab-clip"));
    if fit.clip {
        super::ui::note(ui, p, aurora_i18n::tr!("fit-lab-clip-note"));
    }
    let any = fit.bodies.iter().any(|on| *on);
    ui.horizontal(|ui| {
        let height = ui.spacing().interact_size.y.max(26.0);
        let button = |ui: &mut egui::Ui, icon: &str, label: &str, kind: ActionKind, enabled: bool| {
            let size = Vec2::new(widgets::action_button_fit(ui, icon, label), height);
            widgets::action_button_if(ui, p, icon, label, kind, size, enabled).clicked()
        };
        if !fit.shown {
            let label = aurora_i18n::tr!("fit-lab-show");
            if button(ui, "hanger-2-filled", label, ActionKind::Primary, any) {
                fit.shown = true;
            }
        } else {
            if button(
                ui,
                "current-location-filled",
                aurora_i18n::tr!("fit-lab-place"),
                ActionKind::Flat,
                true,
            ) {
                fit.place = true;
            }
            if button(ui, "trash-filled", aurora_i18n::tr!("fit-lab-remove"), ActionKind::Flat, true) {
                fit.shown = false;
            }
        }
    });
    if fit.shown && !any {
        super::ui::note(ui, p, aurora_i18n::tr!("fit-lab-choose"));
    }
}
