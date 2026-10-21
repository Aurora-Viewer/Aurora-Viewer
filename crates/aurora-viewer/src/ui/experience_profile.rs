//! Experience profile: what the experience is and the agent's permission
//! for it, with Firestorm's Allow / Forget / Block buttons
//! (LLFloaterExperienceProfile, indra/newview/llfloaterexperienceprofile.cpp,
//! originally LGPL 2.1). The minimal part of it: the name and description,
//! rating, scope, location and owner, and "Report Abuse", without editing.

use super::widgets::{ActionKind, Floater, action_button_if};
use crate::theme::Palette;
use crate::world::World;
use aurora_net::experience::{ExperiencePermission, ExperiencePermissionChange};
use egui::{RichText, Vec2};

/// Shows the profile of `world.experiences.profile`; returns the change asked for.
pub fn show(ctx: &egui::Context, p: &Palette, world: &mut World) -> Option<(uuid::Uuid, ExperiencePermissionChange)> {
    let id = world.experiences.profile?;
    if std::mem::take(&mut world.experiences.profile_raise) {
        super::widgets::expand_floater(ctx, "experience_profile");
    }
    let screen = ctx.content_rect();
    let mut open = true;
    let mut change = None;
    let details = world.experiences.details.get(&id).cloned();
    let permission = world.experiences.permissions.get(&id).copied();
    let busy = world.experiences.busy.contains(&id);
    let error = world.experiences.errors.get(&id).cloned();
    let owner = details.as_ref().map(|d| d.owner).filter(|o| !o.is_nil());
    let owner_name = owner.map(|o| {
        let names = &world.social.avatar_names;
        names.want(&o);
        names.get(&o).map(|n| n.display_name.clone()).unwrap_or_else(|| o.to_string())
    });
    Floater::new(
        "experience_profile",
        aurora_i18n::tr!("ui-experience-profile-title"),
        egui::pos2(screen.center().x - 190.0, screen.center().y - 170.0),
        Vec2::new(380.0, 340.0),
    )
    .show(ctx, p, &mut open, |ui| {
        let Some(d) = details.as_ref() else {
            ui.label(
                RichText::new(aurora_i18n::tr!("ui-experience-profile-loading"))
                    .size(12.0)
                    .color(p.muted),
            );
            return;
        };
        let name = if d.name.trim().is_empty() { id.to_string() } else { d.name.clone() };
        ui.label(RichText::new(name).size(15.0).strong().color(p.ink));
        if d.invalid() {
            ui.add(
                egui::Label::new(
                    RichText::new(aurora_i18n::tr!("ui-experience-profile-unknown"))
                        .size(12.0)
                        .color(p.muted),
                )
                .wrap(),
            );
        }
        if !d.description.trim().is_empty() {
            ui.add_space(4.0);
            ui.add(
                egui::Label::new(RichText::new(&d.description).size(12.0).color(p.ink))
                    .wrap()
                    .selectable(true),
            );
        }
        ui.add_space(8.0);
        let row = |ui: &mut egui::Ui, label: &str, value: &str| {
            ui.horizontal(|ui| {
                ui.allocate_ui_with_layout(Vec2::new(110.0, 18.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.set_min_width(110.0);
                    ui.label(RichText::new(label).size(12.0).color(p.muted));
                });
                ui.add(
                    egui::Label::new(RichText::new(value).size(12.0).color(p.ink))
                        .wrap()
                        .selectable(true),
                );
            });
        };
        if !d.invalid() {
            row(
                ui,
                aurora_i18n::tr!("ui-experience-profile-rating"),
                crate::world::worldmap::access_label(d.maturity),
            );
            let scope = if d.grid_wide() {
                aurora_i18n::tr!("world-experiences-grid-scope")
            } else {
                aurora_i18n::tr!("world-experiences-land-scope")
            };
            row(ui, aurora_i18n::tr!("ui-experience-profile-scope"), scope);
        }
        if !d.slurl.trim().is_empty() {
            // "Region (x,y,z)" when the SLURL is a place (LLSLURL)
            let place = crate::slurl::place_link(&d.slurl)
                .map(|l| l.label)
                .unwrap_or_else(|| d.slurl.clone());
            row(ui, aurora_i18n::tr!("ui-experience-profile-location"), &place);
        }
        if let Some(owner) = &owner_name {
            row(ui, aurora_i18n::tr!("ui-experience-profile-owner"), owner);
        }
        ui.add_space(10.0);
        let state = match (busy, permission) {
            (true, _) | (false, None) => aurora_i18n::tr!("ui-experience-profile-checking"),
            (false, Some(ExperiencePermission::Allowed)) => aurora_i18n::tr!("ui-experience-profile-allowed"),
            (false, Some(ExperiencePermission::Blocked)) => aurora_i18n::tr!("ui-experience-profile-blocked"),
            (false, Some(ExperiencePermission::Unset)) => aurora_i18n::tr!("ui-experience-profile-unset"),
        };
        row(ui, aurora_i18n::tr!("ui-experience-profile-permission"), state);
        if let Some(e) = &error {
            ui.add(
                egui::Label::new(
                    RichText::new(aurora_i18n::tr!("ui-experience-profile-error", error = e.clone()))
                        .size(12.0)
                        .color(p.danger),
                )
                .wrap(),
            );
        }
        ui.add_space(8.0);
        // experiencePermissionResults: the button of the current permission
        // is the one that cannot be pressed
        ui.horizontal(|ui| {
            let size = Vec2::new(0.0, 26.0);
            let ready = !busy && permission.is_some();
            if action_button_if(
                ui,
                p,
                "circle-check-filled",
                aurora_i18n::tr!("ui-experience-profile-allow"),
                ActionKind::Primary,
                size,
                ready && permission != Some(ExperiencePermission::Allowed),
            )
            .clicked()
            {
                change = Some(ExperiencePermissionChange::Allow);
            }
            if action_button_if(
                ui,
                p,
                "restore",
                aurora_i18n::tr!("ui-experience-profile-forget"),
                ActionKind::Flat,
                size,
                ready && permission != Some(ExperiencePermission::Unset),
            )
            .on_hover_text(aurora_i18n::tr!("ui-experience-profile-forget-tip"))
            .clicked()
            {
                change = Some(ExperiencePermissionChange::Forget);
            }
            if action_button_if(
                ui,
                p,
                "ban",
                aurora_i18n::tr!("ui-experience-profile-block"),
                ActionKind::FlatDanger,
                size,
                ready && permission != Some(ExperiencePermission::Blocked),
            )
            .clicked()
            {
                change = Some(ExperiencePermissionChange::Block);
            }
        });
        // LLFloaterExperienceProfile::onReportExperience: Report Abuse on
        // the experience (LLFloaterReporter::showFromExperience); on its own
        // row, the window is too narrow for a fourth button
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if action_button_if(
                ui,
                p,
                "flag-filled",
                aurora_i18n::tr!("report-abuse-title"),
                ActionKind::Flat,
                Vec2::new(0.0, 26.0),
                true,
            )
            .clicked()
            {
                crate::ui::context::request(
                    ui.ctx(),
                    crate::ui::context::CtxAction::ReportAbuse(crate::report_abuse::ReportTarget::Experience(id)),
                );
            }
        });
    });
    if !open {
        world.experiences.profile = None;
    }
    change.map(|c| (id, c))
}
