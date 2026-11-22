//! "Experiences" (LLPanelLandExperiences with two LLPanelExperienceListEditor:
//! allowed and blocked experiences, sent as access lists
//! AL_ALLOW_EXPERIENCE / AL_BLOCK_EXPERIENCE).

use super::{LandUi, View, not_yet};
use crate::theme::Palette;
use crate::world::World;
use crate::world::land::powers;
use aurora_net::land::{AL_ALLOW_EXPERIENCE, AL_BLOCK_EXPERIENCE, PARCEL_MAX_EXPERIENCE_LIST};
use egui::{FontId, RichText, Vec2};
use uuid::Uuid;

pub(super) fn show(ui: &mut egui::Ui, p: &Palette, s: &mut LandUi, v: &View, world: &mut World) {
    if v.parcel.is_none() {
        super::no_selection(ui, p);
        return;
    }
    let editable = v.can(powers::LAND_OPTIONS);
    let sel = world.land.sel.clone();
    let (allowed, blocked) = sel.as_ref().map_or((Vec::new(), Vec::new()), |s| {
        (s.allowed_experiences.clone(), s.blocked_experiences.clone())
    });
    let half_h = (ui.available_height() - 10.0) / 2.0;
    for (kind, list, title, help) in [
        (
            AL_ALLOW_EXPERIENCE,
            &allowed,
            aurora_i18n::tr!("ui-land-experiences-allowed-experiences"),
            aurora_i18n::tr!("ui-land-experiences-only-experiences-defined-for-the-parcel-are-allowed-allowed"),
        ),
        (
            AL_BLOCK_EXPERIENCE,
            &blocked,
            aurora_i18n::tr!("ui-land-experiences-blocked-experiences"),
            aurora_i18n::tr!("ui-land-experiences-any-resident-experience-can-be-blocked-blocked-experiences-cannot"),
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
        let rows: Vec<(Uuid, String, Option<String>)> = list
            .iter()
            .map(|id| {
                let (name, creator) = describe(world, *id);
                (*id, name, creator)
            })
            .collect();
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
                            RichText::new(aurora_i18n::tr!(
                                "status-parcel-max-experience-list-max",
                                value0 = format!("{}", list.len()),
                                PARCEL_MAX_EXPERIENCE_LIST = format!("{}", PARCEL_MAX_EXPERIENCE_LIST)
                            ))
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
                                        if rows.is_empty() {
                                            ui.label(RichText::new(aurora_i18n::tr!("status-empty")).size(12.0).color(p.muted));
                                        }
                                        for (id, name, creator) in &rows {
                                            let (rect, r) = ui.allocate_exact_size(Vec2::new(list_w - 12.0, 32.0), egui::Sense::click());
                                            let fill = if *selected == Some(*id) {
                                                p.violet.gamma_multiply(0.6)
                                            } else if r.hovered() {
                                                p.raised
                                            } else {
                                                p.field
                                            };
                                            ui.painter().rect_filled(rect, 0.0, fill);
                                            let name_font = FontId::proportional(12.0);
                                            let note_font = FontId::proportional(10.5);
                                            let gap = crate::ui::widgets::two_lines_gap(ui, &name_font, &note_font, 14.0);
                                            let text_w = rect.width() - 12.0;
                                            let note = creator
                                                .as_ref()
                                                .map(|c| aurora_i18n::tr!("ui-experiences-by", creator = c.as_str()));
                                            let cut_name = crate::ui::widgets::paint_elided(
                                                ui,
                                                name,
                                                rect.left_center() + Vec2::new(6.0, -gap / 2.0),
                                                text_w,
                                                name_font,
                                                p.ink,
                                            );
                                            let cut_note = note.as_deref().is_some_and(|note| {
                                                crate::ui::widgets::paint_elided(
                                                    ui,
                                                    note,
                                                    rect.left_center() + Vec2::new(6.0, gap / 2.0),
                                                    text_w,
                                                    note_font,
                                                    p.muted,
                                                )
                                            });
                                            if cut_name || cut_note {
                                                let mut hover = name.clone();
                                                if let Some(note) = &note {
                                                    hover.push('\n');
                                                    hover.push_str(note);
                                                }
                                                r.clone().on_hover_text(hover);
                                            }
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
                            super::button(ui, p, aurora_i18n::tr!("common-add-2"), false).on_disabled_hover_text(not_yet());
                            if super::button(ui, p, aurora_i18n::tr!("common-delete"), editable && selected.is_some()).clicked() {
                                remove = *selected;
                            }
                            super::button(ui, p, aurora_i18n::tr!("common-profile-2"), false).on_disabled_hover_text(not_yet());
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

/// The two lines of an experience's row: its name and its creator's display
/// name. The details are asked once when unknown. An empty name, and one
/// whose details could not be read, shows untitled, as
/// LLPanelExperienceListEditor::onExperienceDetails does
/// (ExperienceNameUntitled, indra/newview/llpanelexperiencelisteditor.cpp).
///
/// The creator line is Aurora's: the Legacy Viewer's list shows the name
/// only, and two experiences with the same name cannot be told apart
/// without their creator.
pub(super) fn describe(world: &mut World, id: Uuid) -> (String, Option<String>) {
    let Some(owner) = world
        .land
        .experience(&id)
        .map(|d| (!d.invalid()).then_some(d.owner).filter(|o| !o.is_nil()))
    else {
        return (aurora_i18n::tr!("common-loading").into(), None);
    };
    let name = world
        .land
        .experience_name(&id)
        .unwrap_or_else(|| aurora_i18n::tr!("common-loading").into());
    (name, owner.and_then(|owner| super::creator_name(world, owner)))
}

/// experienceRemoved: EXPERIENCE_KEY_TYPE_NONE, then the list goes out.
pub(super) fn remove_experience(world: &mut World, kind: u32, id: Uuid) {
    if let Some(sel) = world.land.sel.as_mut() {
        sel.allowed_experiences.retain(|x| *x != id);
        sel.blocked_experiences.retain(|x| *x != id);
        sel.revision += 1;
        world.land.send_lists(kind);
    }
}
