//! Compact import entry point. Typed pickers follow llviewermenufile.cpp;
//! the movable icon strip and shared upload queue are Aurora additions.

use super::*;

pub(crate) const WINDOW: &str = "import_launcher";

/// Icon-only actions retain the shared button's styling and accessible name.
fn button(ui: &mut egui::Ui, p: &Palette, icon: &str, label: &str, kind: ActionKind, enabled: bool) -> egui::Response {
    let response = action_button_if(ui, p, icon, "", kind, Vec2::splat(32.0), enabled);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
    response
}

pub(super) fn show(ctx: &egui::Context, p: &Palette, imports: &mut Imports, actions: &mut Vec<Action>) {
    if !imports.launcher_open || widgets::interface_hidden(ctx) {
        return;
    }
    if !imports.any_window_open() && ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        imports.launcher_open = false;
        return;
    }
    let mut open = imports.launcher_open;
    let mut picked = None;
    let mut settings = false;
    let work = crate::ui::layout::work_area(ctx);
    let room = crate::ui::layout::move_area(ctx);
    let top = egui::pos2(room.center().x, work.top() + crate::ui::layout::GAP);
    let visible = imports.open && !widgets::floater_minimized(ctx, super::WINDOW);
    widgets::toolstrip(ctx, WINDOW, egui::Align2::CENTER_TOP, top, |ui| {
        widgets::overlay(ui, p, egui::Margin::same(6), |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let (grip, response) = ui.allocate_exact_size(Vec2::new(16.0, 32.0), egui::Sense::hover());
                widgets::paint_glyph(ui.painter(), "grip-vertical", grip.center(), 14.0, p.muted_dim);
                response
                    .on_hover_cursor(egui::CursorIcon::Grab)
                    .on_hover_text(aurora_i18n::tr!("import-icon-bar"));
                for (choice, icon, formats) in [
                    (Choice::Texture, "photo-filled", "PNG · JPG · TGA · BMP · JPEG 2000"),
                    (Choice::Sound, "file-music-filled", "WAV"),
                    (Choice::Animation, "run", "BVH · ANIM"),
                    (Choice::Material, "droplet-filled", "glTF · GLB"),
                    (Choice::Mesh, "cube-3d-sphere", "DAE · glTF · GLB"),
                    (Choice::Bulk, "folder-open-filled", ""),
                    (Choice::Local, "eye-filled", ""),
                ] {
                    if choice == Choice::Bulk {
                        ui.add_space(6.0);
                    }
                    let enabled = choice != Choice::Bulk || (!imports.busy() && imports.pending_files.is_none() && !imports.confirm);
                    let label = if choice == Choice::Bulk {
                        aurora_i18n::tr!("import-launcher-folder")
                    } else {
                        choice.label()
                    };
                    let shown = imports.window_open(choice) && !widgets::floater_minimized(ctx, choice.window_id());
                    let kind = if shown && (choice != Choice::Bulk || imports.tab != Tab::Permissions) {
                        ActionKind::Primary
                    } else {
                        ActionKind::Flat
                    };
                    let response = button(ui, p, icon, label, kind, enabled);
                    #[cfg(test)]
                    ctx.data_mut(|data| data.insert_temp(egui::Id::unique(("import_launcher_choice", choice as usize)), response.rect));
                    let tip = if formats.is_empty() {
                        label.to_owned()
                    } else {
                        format!("{label}\n{formats}")
                    };
                    if response.on_hover_text(tip).clicked() {
                        picked = Some(choice);
                    }
                }
                ui.add_space(6.0);
                let kind = if visible && imports.tab == Tab::Permissions {
                    ActionKind::Primary
                } else {
                    ActionKind::Flat
                };
                settings = button(ui, p, "settings-filled", aurora_i18n::tr!("import-launcher-settings"), kind, true)
                    .on_hover_text(aurora_i18n::tr!("import-settings-title"))
                    .clicked();
                if button(ui, p, "x", aurora_i18n::tr!("common-close"), ActionKind::FlatDanger, true)
                    .on_hover_text(aurora_i18n::tr!("common-close"))
                    .clicked()
                {
                    open = false;
                }
            });
        });
    });
    imports.launcher_open = open;
    if let Some(choice) = picked {
        if choice == Choice::Bulk {
            imports.select_type(Choice::Bulk);
            imports.type_filters = [true; 5];
            imports.tab = Tab::Batch;
            imports.open_window(ctx);
            actions.push(Action::Folder);
        } else {
            imports.open_type_window(ctx, choice);
            if !imports.busy()
                && imports.pending_files.is_none()
                && !imports.confirm
                && !imports.entries.iter().any(|entry| choice.accepts(entry))
            {
                actions.push(Action::Choose(choice));
            }
        }
    }
    if settings {
        imports.tab = Tab::Permissions;
        imports.open_window(ctx);
    }
}

pub(super) fn keep_accessible(ctx: &egui::Context, imports: &Imports) {
    if imports.launcher_open
        && ctx.memory(|memory| {
            memory.area_rect(egui::Id::unique(WINDOW)).is_some_and(|bar| {
                std::iter::once(Choice::Bulk).chain(windows::TYPES).any(|choice| {
                    imports.window_open(choice)
                        && memory
                            .area_rect(egui::Id::unique(choice.window_id()))
                            .is_some_and(|window| bar.intersects(window))
                })
            })
        })
    {
        ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, egui::Id::unique(WINDOW)));
    }
}
