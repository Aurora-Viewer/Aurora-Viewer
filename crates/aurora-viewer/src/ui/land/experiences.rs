//! "Expériences" (LLPanelLandExperiences with two LLPanelExperienceListEditor:
//! allowed and blocked experiences, sent as access lists
//! AL_ALLOW_EXPERIENCE / AL_BLOCK_EXPERIENCE).

use super::{LandUi, NOT_YET, View};
use crate::theme::Palette;
use crate::world::World;
use crate::world::land::powers;
use aurora_net::land::{AL_ALLOW_EXPERIENCE, AL_BLOCK_EXPERIENCE, PARCEL_MAX_EXPERIENCE_LIST};
use egui::{RichText, Vec2};
use uuid::Uuid;

pub(super) fn show(ui: &mut egui::Ui, p: &Palette, s: &mut LandUi, v: &View, world: &mut World) {
    if v.parcel.is_none() {
        super::no_selection(ui, p);
        return;
    }
    let editable = v.can(powers::LAND_OPTIONS);
    let sel = world.land.sel.clone();
    let (allowed, blocked) = sel
        .as_ref()
        .map_or((Vec::new(), Vec::new()), |s| (s.allowed_experiences.clone(), s.blocked_experiences.clone()));
    let half_h = (ui.available_height() - 10.0) / 2.0;
    for (kind, list, title, help) in [
        (
            AL_ALLOW_EXPERIENCE,
            &allowed,
            "Exp. autorisées :",
            "Seules les expériences définies par terrain sont autorisées. Les expériences autorisées ont la permission de s'exécuter sur ce terrain sauf si elles ne sont pas bloquées par le domaine.",
        ),
        (
            AL_BLOCK_EXPERIENCE,
            &blocked,
            "Exp. bloquées :",
            "N'importe quelle expérience de résident peut être bloquée. Les expériences bloquées ne peuvent pas s'exécuter sur ce terrain.",
        ),
    ] {
        let selected = if kind == AL_ALLOW_EXPERIENCE {
            &mut s.allowed_exp_sel
        } else {
            &mut s.blocked_exp_sel
        };
        if selected.is_some_and(|id| !list.contains(&id)) {
            *selected = None;
        }
        let mut remove = None;
        ui.allocate_ui_with_layout(
            Vec2::new(ui.available_width(), half_h),
            egui::Layout::left_to_right(egui::Align::Min),
            |ui| {
                let list_w = 300.0;
                ui.allocate_ui_with_layout(Vec2::new(list_w + 90.0, half_h), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(title).size(12.0).color(p.ink));
                        // ExperiencesCounter
                        ui.label(
                            RichText::new(format!("({}, {PARCEL_MAX_EXPERIENCE_LIST} max.)", list.len()))
                                .size(12.0)
                                .color(p.muted),
                        );
                    });
                    ui.horizontal_top(|ui| {
                        let list_h = half_h - 30.0;
                        egui::Frame::new().fill(p.field).show(ui, |ui| {
                            ui.set_width(list_w);
                            ui.set_max_width(list_w);
                            ui.set_min_height(list_h);
                            egui::ScrollArea::vertical()
                                .id_salt(("land_xp", kind))
                                .max_height(list_h)
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    ui.vertical(|ui| {
                                        ui.set_width(list_w - 12.0);
                                        if list.is_empty() {
                                            ui.label(RichText::new("(vide)").size(12.0).color(p.muted));
                                        }
                                        for id in list {
                                            let name = world.land.experience_name(id).unwrap_or_else(|| "Chargement...".into());
                                            let (rect, r) = ui.allocate_exact_size(Vec2::new(list_w - 12.0, 20.0), egui::Sense::click());
                                            let fill = if *selected == Some(*id) {
                                                p.violet.gamma_multiply(0.6)
                                            } else if r.hovered() {
                                                p.raised
                                            } else {
                                                p.field
                                            };
                                            ui.painter().rect_filled(rect, 0.0, fill);
                                            ui.painter().text(
                                                rect.left_center() + Vec2::new(6.0, 0.0),
                                                egui::Align2::LEFT_CENTER,
                                                name,
                                                egui::FontId::proportional(12.0),
                                                p.ink,
                                            );
                                            if r.clicked() {
                                                *selected = Some(*id);
                                            }
                                        }
                                    });
                                });
                        });
                        ui.vertical(|ui| {
                            ui.set_min_width(90.0);
                            // the experience picker and profile are not ported yet
                            super::button(ui, p, "Ajouter...", false).on_disabled_hover_text(NOT_YET);
                            if super::button(ui, p, "Supprimer", editable && selected.is_some()).clicked() {
                                remove = *selected;
                            }
                            super::button(ui, p, "Profil...", false).on_disabled_hover_text(NOT_YET);
                        });
                    });
                });
                ui.add_space(8.0);
                ui.add(egui::Label::new(RichText::new(help).size(12.0).color(p.muted)).wrap());
            },
        );
        if let Some(id) = remove {
            remove_experience(world, kind, id);
            *selected = None;
        }
    }
}

/// experienceRemoved: EXPERIENCE_KEY_TYPE_NONE, then the list goes out.
fn remove_experience(world: &mut World, kind: u32, id: Uuid) {
    if let Some(sel) = world.land.sel.as_mut() {
        sel.allowed_experiences.retain(|x| *x != id);
        sel.blocked_experiences.retain(|x| *x != id);
        sel.revision += 1;
        world.land.send_lists(kind);
    }
}
