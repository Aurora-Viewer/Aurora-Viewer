//! Compact type chooser requested for the configurable import shortcut.
//! File filters and uploads keep the Firestorm llviewermenufile.cpp paths
//! already implemented by Imports; the chooser is an Aurora UI extension.

use super::*;

#[cfg(test)]
mod tests;

pub(crate) const WINDOW: &str = "import_launcher";
const SETTINGS: &str = "import_settings";

pub(super) fn show(ctx: &egui::Context, p: &Palette, imports: &mut Imports, prefs: &mut Preferences) {
    if !imports.launcher_open && !imports.settings_open {
        return;
    }
    if (imports.settings_open || imports.launcher_open)
        && ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
    {
        if imports.settings_open {
            imports.settings_open = false;
        } else {
            imports.launcher_open = false;
        }
    }
    let mut open = imports.launcher_open;
    let mut picked = None;
    let mut settings = false;
    // the hidden interface hides the strip too (it stays open)
    if open && !widgets::interface_hidden(ctx) {
        let work = crate::ui::layout::work_area(ctx);
        let room = crate::ui::layout::move_area(ctx);
        let top = egui::pos2(room.center().x, work.top() + crate::ui::layout::GAP);
        widgets::toolstrip(ctx, WINDOW, egui::Align2::CENTER_TOP, top, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                for (choice, icon, formats) in [
                    (Choice::Texture, "photo-filled", "PNG · JPG · TGA · BMP · JPEG 2000"),
                    (Choice::Sound, "file-music-filled", "WAV"),
                    (Choice::Animation, "run", "BVH · ANIM"),
                    (Choice::Material, "droplet-filled", "glTF · GLB"),
                    (Choice::Mesh, "cube-3d-sphere", "DAE · glTF · GLB"),
                    (Choice::Bulk, "files-filled", ""),
                    (Choice::Local, "eye-filled", ""),
                ] {
                    let response = widgets::glass_action_icon(ui, p, icon, choice.label(), Vec2::splat(36.0), p.violet_light);
                    let tip = if formats.is_empty() {
                        choice.label().to_owned()
                    } else {
                        format!("{}\n{formats}", choice.label())
                    };
                    if response.on_hover_text(tip).clicked() {
                        picked = Some(choice);
                    }
                }
                settings = widgets::glass_action_icon(
                    ui,
                    p,
                    "settings-filled",
                    aurora_i18n::tr!("import-launcher-settings"),
                    Vec2::splat(36.0),
                    p.muted,
                )
                .on_hover_text(aurora_i18n::tr!("import-settings-title"))
                .clicked();
                if widgets::close_button(ui, p).clicked() {
                    open = false;
                }
            });
        });
    }
    imports.launcher_open = open;
    if let Some(choice) = picked {
        if imports.open && imports.window_choice() == choice {
            imports.open = false;
            imports.choose_files_on_open = false;
        } else {
            let has_files = imports.entries.iter().any(|entry| choice.accepts(entry));
            if has_files {
                imports.open(choice);
                imports.choose_files_on_open = false;
            } else {
                imports.open_file_picker(choice);
            }
            widgets::open_floater(ctx, choice.window_id(), &mut imports.open);
            ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, egui::Id::unique(choice.window_id())));
        }
    }
    if settings {
        if imports.settings_open {
            imports.settings_open = false;
        } else {
            widgets::open_floater(ctx, SETTINGS, &mut imports.settings_open);
        }
    }
    show_settings(ctx, p, imports, prefs);
    if imports.launcher_open && imports.settings_open {
        keep_accessible(ctx, SETTINGS);
    }
}

pub(super) fn keep_accessible(ctx: &egui::Context, window: &str) {
    if bounds(ctx, WINDOW)
        .zip(bounds(ctx, window))
        .is_some_and(|(bar, other)| bar.intersects(other))
    {
        // On a narrow viewport there may be no room beside the bar: its
        // type buttons, toggle and close action must remain reachable.
        ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, egui::Id::unique(WINDOW)));
    }
}

fn bounds(ctx: &egui::Context, id: &str) -> Option<egui::Rect> {
    let state = egui::AreaState::load(ctx, egui::Id::unique(id))?;
    Some(egui::Rect::from_min_size(state.left_top_pos(), state.size?))
}

fn show_settings(ctx: &egui::Context, p: &Palette, imports: &mut Imports, prefs: &mut Preferences) {
    if !imports.settings_open {
        return;
    }
    let mut open = imports.settings_open;
    let mut floater = Floater::new(
        SETTINGS,
        aurora_i18n::tr!("import-settings-title"),
        IMPORT_POSITION,
        Vec2::new(620.0, 470.0),
    );
    floater.min_size = Vec2::new(600.0, 430.0);
    floater.show(ctx, p, &mut open, |ui| {
        egui::ScrollArea::vertical()
            .id_salt("import_defaults")
            .max_height(ui.available_height())
            .auto_shrink([false, false])
            .show(ui, |ui| {
                permission_panel::show(ui, p, prefs);
            });
    });
    imports.settings_open = open;
}
