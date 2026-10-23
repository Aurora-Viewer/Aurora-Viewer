//! Replacement stays inside the build floater; the world click picker
//! retains the old selection as the locked destination.

use super::{
    common::{small, text},
    compact,
};
use crate::build::{
    BuildSettings, BuildTool,
    replace::{Stage, object_name},
};
use crate::theme::Palette;
use crate::ui::widgets::{flat_button_sized, primary_button_sized, switch};
use crate::world::World;
use egui::{RichText, Vec2};

pub fn button(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &World, s: &BuildSettings) {
    if !tool.replacement_visible(world, s) {
        return;
    }
    ui.add_space(3.0);
    if flat_button_sized(ui, p, aurora_i18n::tr!("replace-button"), Vec2::new(ui.available_width(), 24.0))
        .on_hover_text(aurora_i18n::tr!("replace-button-help"))
        .clicked()
    {
        tool.begin_replacement(world, s);
    }
}

fn toggle(ui: &mut egui::Ui, p: &Palette, value: &mut bool, label: &str, help: &str) {
    ui.horizontal(|ui| {
        switch(ui, p, value).on_hover_text(help);
        if ui
            .add(egui::Label::new(RichText::new(label).size(12.0).color(p.ink)).sense(egui::Sense::click()))
            .on_hover_text(help)
            .clicked()
        {
            *value = !*value;
        }
    });
}

pub fn panel(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &mut World) {
    compact::heading(ui, p, "replace-filled", aurora_i18n::tr!("replace-button"), |_| {});
    // Leave the confirmation/cancel row in reach when the inspector is short.
    let height = (ui.available_height() - 96.0).clamp(64.0, 380.0);
    egui::ScrollArea::vertical()
        .id_salt("replace_prim")
        .max_height(height)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            if let Some(draft) = tool.replacement.draft.as_ref() {
                let (target, number, source, picking) = (draft.target, draft.number, draft.source, draft.picking);
                text(
                    ui,
                    p,
                    &aurora_i18n::tr!(
                        "replace-target",
                        number = number.to_string(),
                        name = object_name(tool, world, target)
                    ),
                );
                if number == 1 {
                    ui.label(RichText::new(aurora_i18n::tr!("replace-root-help")).size(11.0).color(p.warn));
                }
                ui.add_space(5.0);
                if let Some(source) = source {
                    text(ui, p, &aurora_i18n::tr!("replace-source", name = object_name(tool, world, source)));
                }
                if flat_button_sized(
                    ui,
                    p,
                    if source.is_some() {
                        aurora_i18n::tr!("replace-change-source")
                    } else {
                        aurora_i18n::tr!("replace-choose-source")
                    },
                    Vec2::new(ui.available_width(), 26.0),
                )
                .clicked()
                    && let Some(draft) = tool.replacement.draft.as_mut()
                {
                    draft.picking = true;
                }
                if picking {
                    ui.label(RichText::new(aurora_i18n::tr!("replace-picking")).size(12.0).color(p.violet_light));
                }
                crate::ui::widgets::divider(ui, p);
                let o = &mut tool.replacement.options;
                toggle(
                    ui,
                    p,
                    &mut o.position,
                    aurora_i18n::tr!("replace-copy-position"),
                    aurora_i18n::tr!("replace-position-help"),
                );
                toggle(
                    ui,
                    p,
                    &mut o.rotation,
                    aurora_i18n::tr!("replace-copy-rotation"),
                    aurora_i18n::tr!("replace-rotation-help"),
                );
                toggle(
                    ui,
                    p,
                    &mut o.size,
                    aurora_i18n::tr!("replace-copy-size"),
                    aurora_i18n::tr!("replace-size-help"),
                );
                toggle(
                    ui,
                    p,
                    &mut o.contents,
                    aurora_i18n::tr!("replace-copy-contents"),
                    aurora_i18n::tr!("replace-contents-help"),
                );
                toggle(
                    ui,
                    p,
                    &mut o.pause_scripts,
                    aurora_i18n::tr!("replace-pause-scripts"),
                    aurora_i18n::tr!("replace-scripts-help"),
                );
                ui.add_space(5.0);
                small(ui, p, aurora_i18n::tr!("replace-old-kept"));
                small(ui, p, &aurora_i18n::tr!("replace-number-kept", number = number.to_string()));
                if !tool.replacement.message.is_empty() {
                    ui.label(RichText::new(&tool.replacement.message).size(11.0).color(p.warn));
                }
            } else if let Some(job) = &tool.replacement.job {
                let terminal = job.stage.terminal();
                text(
                    ui,
                    p,
                    &aurora_i18n::tr!(
                        "replace-target",
                        number = job.number.to_string(),
                        name = object_name(tool, world, job.target)
                    ),
                );
                text(
                    ui,
                    p,
                    &aurora_i18n::tr!("replace-source", name = object_name(tool, world, job.source)),
                );
                ui.add_space(8.0);
                let width = ui.available_width();
                crate::ui::widgets::progress(ui, p, "replace_progress", job.progress(), width, &job.progress_text(), None);
                if !terminal {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        small(ui, p, aurora_i18n::tr!("replace-waiting-server"));
                    });
                }
                if job.stage == Stage::Done {
                    ui.label(
                        RichText::new(aurora_i18n::tr!("replace-success", number = job.number.to_string()))
                            .size(12.0)
                            .color(p.success),
                    );
                    small(ui, p, aurora_i18n::tr!("replace-old-kept"));
                }
                if !job.message.is_empty() {
                    ui.label(RichText::new(&job.message).size(11.0).color(p.warn));
                }
            }
        });
    ui.add_space(8.0);
    crate::ui::widgets::divider(ui, p);
    ui.add_space(3.0);
    if let Some(draft) = &tool.replacement.draft {
        let ready = draft.source.is_some() && !draft.picking;
        ui.horizontal(|ui| {
            if flat_button_sized(ui, p, aurora_i18n::tr!("common-cancel"), Vec2::new(80.0, 26.0)).clicked() {
                tool.cancel_replacement(world);
            }
            let width = ui.available_width();
            if ui
                .add_enabled_ui(ready, |ui| {
                    primary_button_sized(ui, p, aurora_i18n::tr!("replace-validate"), Vec2::new(width, 26.0))
                })
                .inner
                .clicked()
            {
                tool.apply_replacement(world);
            }
        });
    } else if tool.replacement.job.as_ref().is_some_and(|job| job.stage.terminal())
        && flat_button_sized(ui, p, aurora_i18n::tr!("common-close"), Vec2::new(ui.available_width(), 26.0)).clicked()
    {
        tool.cancel_replacement(world);
    }
}
