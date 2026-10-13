//! The five importable types retain independent next-owner rights.
use super::*;

pub(super) fn show(ui: &mut egui::Ui, p: &Palette, prefs: &mut Preferences) {
    widgets::section_header(ui, p, "shield-lock-filled", aurora_i18n::tr!("import-permissions"));
    ui.label(RichText::new(aurora_i18n::tr!("import-permissions-basic-help")).color(p.muted));
    ui.add_space(14.0);
    egui::Grid::new("import_permissions_by_type")
        .num_columns(4)
        .spacing(Vec2::new(12.0, 6.0))
        .show(ui, |ui| {
            ui.label(RichText::new(aurora_i18n::tr!("import-permissions-type")).strong());
            for label in [
                aurora_i18n::tr!("common-edit"),
                aurora_i18n::tr!("common-copy-2"),
                aurora_i18n::tr!("common-transfer"),
            ] {
                cell(ui, |ui| {
                    ui.label(RichText::new(label).strong());
                });
            }
            ui.end_row();
            for (kind, choice, icon) in [
                (Kind::Texture, Choice::Texture, "photo-filled"),
                (Kind::Sound, Choice::Sound, "volume"),
                (Kind::Animation, Choice::Animation, "run"),
                (Kind::Mesh, Choice::Mesh, "cube-3d-sphere"),
                (Kind::Material, Choice::Material, "palette-filled"),
            ] {
                {
                    ui.allocate_ui_with_layout(Vec2::new(190.0, 32.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.set_width(190.0);
                        widgets::glyph(ui, icon, p.violet_light, 18.0);
                        ui.label(choice.label());
                    });
                    let mut row = prefs.next_owner_for(kind);
                    let old = row;
                    cell(ui, |ui| {
                        checkbox(ui, p, &mut row.modify, "").on_hover_text(aurora_i18n::tr!("common-edit"));
                    });
                    cell(ui, |ui| {
                        checkbox(ui, p, &mut row.copy, "").on_hover_text(aurora_i18n::tr!("common-copy-2"));
                    });
                    if !row.copy {
                        row.transfer = true;
                    }
                    cell(ui, |ui| {
                        ui.add_enabled_ui(row.copy, |ui| {
                            checkbox(ui, p, &mut row.transfer, "").on_hover_text(aurora_i18n::tr!("common-transfer"));
                        })
                        .response
                        .on_disabled_hover_text(aurora_i18n::tr!("import-permissions-fair-use"));
                    });
                    if row != old {
                        prefs.permissions_by_type.set(kind, row);
                    }
                    ui.end_row();
                }
            }
        });
    ui.add_space(12.0);
    ui.label(
        RichText::new(aurora_i18n::tr!("import-permissions-fair-use"))
            .small()
            .color(p.muted),
    );
    ui.add_space(18.0);
    widgets::section_header(ui, p, "coin-filled", aurora_i18n::tr!("import-permissions-payment"));
    checkbox(ui, p, &mut prefs.show_payment, aurora_i18n::tr!("import-show-payment"));
}

fn cell(ui: &mut egui::Ui, body: impl FnOnce(&mut egui::Ui)) {
    ui.allocate_ui_with_layout(Vec2::new(100.0, 32.0), egui::Layout::top_down(egui::Align::Center), |ui| {
        ui.set_width(100.0);
        ui.add_space(4.0);
        body(ui);
    });
}
