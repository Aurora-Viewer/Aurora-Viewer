//! Five independent rights per import type; payment remains a common setting.
use super::*;

pub(super) fn show(ui: &mut egui::Ui, p: &Palette, prefs: &mut Preferences) {
    defaults(ui, p, prefs);
    ui.add_space(14.0);
    widgets::section_header(ui, p, "coin-filled", aurora_i18n::tr!("import-permissions-payment"));
    checkbox(ui, p, &mut prefs.show_payment, aurora_i18n::tr!("import-show-payment"));
}

pub(crate) fn defaults(ui: &mut egui::Ui, p: &Palette, prefs: &mut Preferences) {
    widgets::section_header(ui, p, "shield-lock-filled", aurora_i18n::tr!("import-permissions"));
    ui.label(
        RichText::new(aurora_i18n::tr!("import-permissions-basic-help"))
            .small()
            .color(p.muted),
    );
    ui.add_space(10.0);
    let columns = [
        (64.0, aurora_i18n::tr!("common-edit"), aurora_i18n::tr!("common-edit")),
        (64.0, aurora_i18n::tr!("common-copy-2"), aurora_i18n::tr!("common-copy-2")),
        (70.0, aurora_i18n::tr!("common-transfer"), aurora_i18n::tr!("common-transfer")),
        (
            72.0,
            aurora_i18n::tr!("import-permissions-group"),
            aurora_i18n::tr!("import-share-group"),
        ),
        (
            80.0,
            aurora_i18n::tr!("import-permissions-everyone"),
            aurora_i18n::tr!("import-everyone-copy"),
        ),
    ];
    egui::Grid::new("import_permissions_by_type")
        .num_columns(6)
        .spacing(Vec2::new(10.0, 6.0))
        .show(ui, |ui| {
            ui.label(RichText::new(aurora_i18n::tr!("import-permissions-type")).strong());
            for (width, label, help) in columns {
                cell(ui, width, |ui| {
                    ui.label(RichText::new(label).strong()).on_hover_text(help);
                });
            }
            ui.end_row();
            for (kind, choice, icon) in [
                (Kind::Texture, Choice::Texture, "photo-filled"),
                (Kind::Sound, Choice::Sound, "file-music-filled"),
                (Kind::Animation, Choice::Animation, "run"),
                (Kind::Mesh, Choice::Mesh, "cube-3d-sphere"),
                (Kind::Material, Choice::Material, "droplet-filled"),
            ] {
                ui.allocate_ui_with_layout(Vec2::new(150.0, 32.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.set_width(150.0);
                    widgets::glyph(ui, icon, p.violet_light, 18.0);
                    ui.label(choice.label());
                });
                let mut row = prefs.next_owner_for(kind);
                let before = row;
                let mut group = row.share_group.unwrap_or(prefs.share_group);
                let mut everyone = row.everyone_copy.unwrap_or(prefs.everyone_copy);
                for (index, value) in [
                    (0, &mut row.modify),
                    (1, &mut row.copy),
                    (2, &mut row.transfer),
                    (3, &mut group),
                    (4, &mut everyone),
                ] {
                    let (width, _, help) = columns[index];
                    cell(ui, width, |ui| {
                        let response = ui
                            .add_enabled_ui(index != 2 || before.copy, |ui| checkbox(ui, p, value, "").on_hover_text(help))
                            .inner;
                        ui.ctx()
                            .data_mut(|data| data.insert_temp(egui::Id::unique(("import_permission", kind, index)), response.rect));
                    });
                }
                if !row.copy {
                    row.transfer = true;
                }
                if group != row.share_group.unwrap_or(prefs.share_group) {
                    row.share_group = Some(group);
                }
                if everyone != row.everyone_copy.unwrap_or(prefs.everyone_copy) {
                    row.everyone_copy = Some(everyone);
                }
                if row != before {
                    prefs.permissions_by_type.set(kind, row);
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

fn cell(ui: &mut egui::Ui, width: f32, body: impl FnOnce(&mut egui::Ui)) {
    ui.allocate_ui_with_layout(Vec2::new(width, 32.0), egui::Layout::top_down(egui::Align::Center), |ui| {
        ui.set_width(width);
        ui.add_space(4.0);
        body(ui);
    });
}
