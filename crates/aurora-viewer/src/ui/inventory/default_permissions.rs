//! Shared access to creation and import defaults. The importer retains its
//! existing Permissions tab and edits the same saved import preferences.
use crate::{creation_permissions::Defaults, imports, theme::Palette, ui::widgets};
use egui::{RichText, Vec2};

pub(crate) fn show(ctx: &egui::Context, p: &Palette, open: &mut bool, creation: &mut Defaults, imports: &mut imports::Preferences) {
    let mut close = false;
    widgets::Floater::new(
        "inventory_default_permissions",
        aurora_i18n::tr!("inventory-default-permissions"),
        ctx.content_rect().center() - Vec2::new(350.0, 380.0),
        Vec2::new(700.0, 760.0),
    )
    .show(ctx, p, open, |ui| {
        ui.label(
            RichText::new(aurora_i18n::tr!("inventory-default-permissions-help"))
                .small()
                .color(p.muted),
        );
        ui.add_space(10.0);
        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .max_height((ui.available_height() - 44.0).max(240.0))
            .show(ui, |ui| {
                widgets::section_header(ui, p, "folder-filled", aurora_i18n::tr!("inventory-default-permissions-creations"));
                ui.add_space(8.0);
                creation_rows(ui, p, creation);
                ui.add_space(18.0);
                ui.push_id("inventory_import_defaults", |ui| {
                    imports::ui::permission_panel::defaults(ui, p, imports)
                });
            });
        ui.add_space(8.0);
        close = ui.button(aurora_i18n::tr!("common-close")).clicked();
    });
    if close {
        *open = false;
    }
}

fn creation_rows(ui: &mut egui::Ui, p: &Palette, defaults: &mut Defaults) {
    let columns = [
        (64.0, aurora_i18n::tr!("common-edit")),
        (64.0, aurora_i18n::tr!("common-copy-2")),
        (70.0, aurora_i18n::tr!("common-transfer")),
        (72.0, aurora_i18n::tr!("import-permissions-group")),
        (80.0, aurora_i18n::tr!("import-permissions-everyone")),
    ];
    egui::Grid::new("inventory_creation_defaults")
        .num_columns(6)
        .spacing(Vec2::new(10.0, 6.0))
        .show(ui, |ui| {
            ui.label(RichText::new(aurora_i18n::tr!("inventory-default-permissions-type")).strong());
            for (width, label) in columns {
                cell(ui, width, |ui| {
                    ui.label(RichText::new(label).strong());
                });
            }
            ui.end_row();
            for (label, icon, settings, rights) in [
                (aurora_i18n::tr!("common-scripts"), "code", false, &mut defaults.scripts),
                (aurora_i18n::tr!("common-notes"), "file-text-filled", false, &mut defaults.notecards),
                (
                    aurora_i18n::tr!("ui-inventory-view-gestures"),
                    "hand-move",
                    false,
                    &mut defaults.gestures,
                ),
                (
                    aurora_i18n::tr!("ui-inventory-view-materials"),
                    "droplet-filled",
                    false,
                    &mut defaults.materials,
                ),
                (
                    aurora_i18n::tr!("ui-inventory-view-clothing-and-body"),
                    "shirt-filled",
                    false,
                    &mut defaults.wearables,
                ),
                (
                    aurora_i18n::tr!("ui-inventory-view-eep-settings"),
                    "sun-filled",
                    true,
                    &mut defaults.settings,
                ),
            ] {
                if settings {
                    rights.copy = true;
                    rights.share_group = false;
                    rights.everyone_copy = false;
                }
                if !rights.copy {
                    rights.transfer = true;
                }
                ui.allocate_ui_with_layout(Vec2::new(150.0, 32.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.set_width(150.0);
                    widgets::glyph(ui, icon, p.violet_light, 18.0);
                    ui.label(label);
                });
                let copy = rights.copy;
                for (index, value) in [
                    (0, &mut rights.modify),
                    (1, &mut rights.copy),
                    (2, &mut rights.transfer),
                    (3, &mut rights.share_group),
                    (4, &mut rights.everyone_copy),
                ] {
                    let enabled = if settings { index == 0 || index == 2 } else { index != 2 || copy };
                    cell(ui, columns[index].0, |ui| {
                        ui.add_enabled_ui(enabled, |ui| {
                            let response = widgets::checkbox(ui, p, value, "");
                            if settings {
                                response.on_hover_text(aurora_i18n::tr!("inventory-default-permissions-settings-help"));
                            }
                        });
                    });
                }
                if !rights.copy {
                    rights.transfer = true;
                }
                ui.end_row();
            }
        });
    ui.add_space(8.0);
    ui.label(
        RichText::new(aurora_i18n::tr!("import-permissions-fair-use"))
            .small()
            .color(p.muted),
    );
}

fn cell(ui: &mut egui::Ui, width: f32, content: impl FnOnce(&mut egui::Ui)) {
    ui.allocate_ui_with_layout(Vec2::new(width, 32.0), egui::Layout::top_down(egui::Align::Center), |ui| {
        ui.set_width(width);
        ui.add_space(4.0);
        content(ui);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(events: Vec<egui::Event>) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1280.0, 1000.0))),
            events,
            ..Default::default()
        }
    }

    #[test]
    fn default_permissions_entry_is_last_and_opens_even_when_creation_is_disabled() {
        let ctx = egui::Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, 1.0);
        let mut st = super::super::InventoryUi::default();
        let mut prefs = super::super::InventoryPreferences::default();
        let mut actions = Vec::new();
        let mut frame = |events| {
            let mut output = ctx.run_ui(input(events), |ui| {
                super::super::context::new_menu(ui, &theme.palette(), uuid::Uuid::nil(), false, &mut st, &mut prefs, &mut actions);
            });
            output.textures_delta.clear();
            output
        };
        let mut output = frame(Vec::new());
        for _ in 0..4 {
            output = frame(Vec::new());
        }
        let labels: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some((text.pos, text.galley.text())),
                _ => None,
            })
            .collect();
        let (position, _) = labels
            .iter()
            .find(|(_, label)| *label == aurora_i18n::tr!("inventory-default-permissions"))
            .unwrap();
        assert!(labels.iter().all(|(pos, _)| pos.y <= position.y));
        let position = *position + egui::vec2(24.0, 8.0);
        for pressed in [true, false] {
            drop(frame(vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]));
        }
        assert!(matches!(actions.as_slice(), [super::super::InvAction::DefaultPermissions]));
    }

    #[test]
    fn shared_default_permissions_edits_import_preferences_without_touching_creation_or_payment() {
        use aurora_net::inventory::upload::Kind;
        let ctx = egui::Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, 1.0);
        let mut creation = Defaults::default();
        let original = creation;
        let mut imports = imports::Preferences::default();
        let mut open = true;
        let mut frame = |events| {
            let mut output = ctx.run_ui(input(events), |ui| {
                show(ui.ctx(), &theme.palette(), &mut open, &mut creation, &mut imports)
            });
            output.textures_delta.clear();
            output
        };
        for _ in 0..8 {
            drop(frame(Vec::new()));
        }
        let position = ctx
            .data(|data| data.get_temp::<egui::Rect>(egui::Id::unique(("import_permission", Kind::Sound, 0))))
            .unwrap()
            .center();
        for pressed in [true, false] {
            drop(frame(vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]));
        }
        assert!(imports.next_owner_for(Kind::Sound).modify);
        assert!(!imports.next_owner_for(Kind::Texture).modify);
        assert!(imports.show_payment);
        assert_eq!(creation, original);
    }
}
