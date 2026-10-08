//! Préférences › Caméra: Firestorm's « Déplacements et vue › Vue » and
//! « Vue subjective » camera settings (panel_preferences_move.xml), the
//! camera presets and the phototools offsets.

use super::{SLIDER_W, group, row, slider, toggle};
use crate::camera::{CameraPreset, CameraSettings, DEFAULT_FOV, FOV_RANGE};
use crate::settings::Settings;
use crate::theme::Palette;
use egui::RichText;

/// Three fields x / y / z (x forward, y left, z up).
fn offset(ui: &mut egui::Ui, v: &mut [f32; 3]) -> bool {
    let mut changed = false;
    for (axis, value) in ["X", "Y", "Z"].iter().zip(v.iter_mut()) {
        ui.label(RichText::new(*axis).size(12.0));
        changed |= ui
            .add(egui::DragValue::new(value).range(-10.0..=10.0).speed(0.02).max_decimals(2))
            .changed();
    }
    changed
}

pub(super) fn page(ui: &mut egui::Ui, p: &Palette, s: &mut Settings) -> bool {
    let c = &mut s.camera;
    let mut changed = false;
    ui.label(
        RichText::new(
            "Alt + clic : viser un point · Alt + glisser : zoomer · Ctrl + Alt + glisser : tourner autour · \
             Ctrl + Alt + Maj + glisser : décaler · Échap : revenir derrière l'avatar",
        )
        .size(12.0)
        .color(p.muted),
    );
    ui.add_space(8.0);
    group(ui, p, "Point de vue", |ui| {
        changed |= row(ui, p, "Préréglage", "Les vues de Firestorm (Caméra › Préréglages)", |ui| {
            let mut ch = false;
            let mut preset = c.preset;
            egui::ComboBox::from_id_salt("camera_preset")
                .width(SLIDER_W)
                .selected_text(preset.label())
                .show_ui(ui, |ui| {
                    for v in CameraPreset::ALL {
                        ch |= ui.selectable_value(&mut preset, v, v.label()).changed();
                    }
                });
            if ch {
                c.apply_preset(preset);
            }
            ch
        });
        let custom = |ch: bool, c: &mut CameraSettings| {
            if ch {
                c.preset = CameraPreset::Custom;
            }
            ch
        };
        changed |= row(
            ui,
            p,
            "Position de la caméra",
            "Décalage depuis l'avatar, en mètres : X devant, Y à gauche, Z en haut (Ctrl + molette monte la caméra)",
            |ui| {
                let ch = offset(ui, &mut c.camera_offset);
                custom(ch, c)
            },
        );
        changed |= row(
            ui,
            p,
            "Point visé",
            "Point regardé, depuis l'avatar (Maj + molette le monte ou le descend)",
            |ui| {
                let ch = offset(ui, &mut c.focus_offset);
                custom(ch, c)
            },
        );
        changed |= row(ui, p, "", "", |ui| {
            let reset = crate::ui::widgets::flat_button(ui, p, "Réinitialiser les angles")
                .on_hover_text("Remet la position de la caméra et le point visé du préréglage")
                .clicked();
            if reset {
                let preset = if c.preset == CameraPreset::Custom {
                    CameraPreset::Rear
                } else {
                    c.preset
                };
                c.apply_preset(preset);
            }
            reset
        });
    });
    group(ui, p, "Mouvements de la caméra", |ui| {
        changed |= row(
            ui,
            p,
            "Angle de vue",
            "Champ de vision vertical (60° par défaut), comme Ctrl+0 / Ctrl+8 dans Firestorm",
            |ui| {
                let mut deg = c.fov.to_degrees();
                let ch = slider(ui, &mut deg, FOV_RANGE.start().to_degrees()..=FOV_RANGE.end().to_degrees(), "°");
                if ch {
                    c.fov = deg.to_radians();
                }
                if ui.small_button("D").on_hover_text("Valeur par défaut").clicked() {
                    c.fov = DEFAULT_FOV;
                    return true;
                }
                ch
            },
        );
        changed |= row(
            ui,
            p,
            "Distance",
            "Éloignement de la caméra derrière l'avatar (1 par défaut)",
            |ui| slider(ui, &mut c.offset_scale, 0.5..=3.0, "×"),
        );
        changed |= row(
            ui,
            p,
            "Durée des transitions",
            "Temps du passage d'une vue à l'autre : vue subjective, retour derrière l'avatar, point visé (0,4 s par défaut)",
            |ui| slider(ui, &mut c.zoom_time, 0.0..=4.0, " s"),
        );
        changed |= row(
            ui,
            p,
            "Lissage",
            "La caméra suit l'avatar avec un léger retard (1 par défaut, 0 = aucun)",
            |ui| slider(ui, &mut c.smoothing, 0.0..=9.0, ""),
        );
        changed |= row(
            ui,
            p,
            "Recul en vol",
            "En l'air, la caméra prend du retard selon la vitesse (2 par défaut, 0 = aucun, 30 = vitesse de l'avatar)",
            |ui| slider(ui, &mut c.dynamic_strength, 0.0..=30.0, ""),
        );
    });
    group(ui, p, "Comportement", |ui| {
        changed |= row(
            ui,
            p,
            "Revenir en bougeant",
            "Bouger l'avatar ramène la caméra derrière lui après l'avoir déplacée",
            |ui| toggle(ui, p, &mut c.reset_on_movement),
        );
        changed |= row(
            ui,
            p,
            "Revenir après une téléportation",
            "La caméra se replace derrière l'avatar à l'arrivée",
            |ui| toggle(ui, p, &mut c.reset_on_teleport),
        );
        changed |= row(
            ui,
            p,
            "Échap tourne l'avatar",
            "Échap garde la direction de la caméra et tourne l'avatar vers elle",
            |ui| toggle(ui, p, &mut c.reset_view_turns_avatar),
        );
        changed |= row(
            ui,
            p,
            "Clic sur mon avatar : garder la caméra",
            "Cliquer sur son avatar ne ramène pas la caméra derrière lui",
            |ui| toggle(ui, p, &mut c.click_avatar_keeps_camera),
        );
        changed |= row(
            ui,
            p,
            "Suivre l'objet visé",
            "La caméra suit l'objet sur lequel elle est fixée quand il bouge",
            |ui| toggle(ui, p, &mut c.track_focus_object),
        );
        changed |= row(
            ui,
            p,
            "Suivre la hauteur de survol",
            "La hauteur de survol de l'avatar déplace aussi la caméra",
            |ui| toggle(ui, p, &mut c.hover_affects_camera),
        );
        changed |= row(
            ui,
            p,
            "Pas de zoom à la molette",
            "La molette ne rapproche ni n'éloigne la caméra",
            |ui| toggle(ui, p, &mut c.disable_wheel_zoom),
        );
    });
    group(ui, p, "Limites", |ui| {
        changed |= row(
            ui,
            p,
            "Caméra sans limites",
            "La caméra peut aller au-delà de la distance d'affichage, sous le sol et plus près des objets",
            |ui| toggle(ui, p, &mut c.disable_constraints),
        );
        changed |= row(
            ui,
            p,
            "Traverser les objets",
            "Ignorer le simulateur qui repousse la caméra devant les murs et les objets",
            |ui| toggle(ui, p, &mut c.ignore_sim_constraints),
        );
    });
    group(ui, p, "Vue subjective", |ui| {
        changed |= row(
            ui,
            p,
            "Activer la vue subjective",
            "Touche M, ou zoomer jusque dans la tête de l'avatar",
            |ui| toggle(ui, p, &mut c.enable_mouselook),
        );
        changed |= row(
            ui,
            p,
            "Quitter avec la molette",
            "Tourner la molette vers soi quitte la vue subjective",
            |ui| toggle(ui, p, &mut c.wheel_exits_mouselook),
        );
    });
    changed
}
