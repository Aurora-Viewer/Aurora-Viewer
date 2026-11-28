//! What the AO window shows while the account has no set: what an AO does,
//! three steps to start, and the ways to get a first set (a new one, one
//! from another viewer, or a notecard).
use super::*;

pub(super) fn show(ui: &mut egui::Ui, p: &Palette, inv: &Inventory, catalog: &inventory::Catalog, state: &mut AoUi, out: &mut Vec<Action>) {
    if catalog.loading || state.startup_loading {
        return;
    }
    let width = ui.available_width().min(560.0);
    ui.add_space((ui.available_height() * 0.12).min(70.0));
    ui.vertical_centered(|ui| {
        ui.set_max_width(width);
        ui.label(
            RichText::new(aurora_i18n::tr!("ao-welcome").to_uppercase())
                .small()
                .color(p.violet_pale),
        );
        ui.label(RichText::new(aurora_i18n::tr!("ao-welcome-title")).size(24.0).strong());
        ui.add(egui::Label::new(RichText::new(aurora_i18n::tr!("ao-welcome-body")).color(p.muted)).wrap());
    });
    ui.add_space(18.0);
    ui.vertical_centered(|ui| {
        ui.set_max_width(width);
        for (number, title, line) in [
            (1, "ao-welcome-step-1", "ao-welcome-step-1-line"),
            (2, "ao-welcome-step-2", "ao-welcome-step-2-line"),
            (3, "ao-welcome-step-3", "ao-welcome-step-3-line"),
        ] {
            ui.horizontal(|ui| {
                let (disc, _) = ui.allocate_exact_size(Vec2::splat(26.0), egui::Sense::hover());
                ui.painter().circle_filled(disc.center(), 12.0, p.violet.gamma_multiply(0.35));
                let digit = ui
                    .painter()
                    .layout_no_wrap(number.to_string(), egui::FontId::proportional(12.5), p.ink);
                widgets::paint_ink_centered(ui.painter(), digit, disc.center(), p.ink);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.label(RichText::new(aurora_i18n::text(title)).strong());
                    ui.label(RichText::new(aurora_i18n::text(line)).small().color(p.muted));
                });
            });
            ui.add_space(4.0);
        }
    });
    ui.add_space(18.0);
    ui.vertical_centered(|ui| {
        ui.set_max_width(width);
        ui.label(RichText::new(aurora_i18n::tr!("ao-welcome-name")).strong());
        if state.name.is_empty() && state.new_dialog.is_none() {
            state.name = aurora_i18n::tr!("ao-welcome-default-name").into();
        }
        let busy = state.busy();
        ui.add_enabled_ui(!busy, |ui| {
            ui.horizontal(|ui| {
                let button = 150.0;
                let field = ui.available_width() - button - ui.spacing().item_spacing.x;
                let response = widgets::restricted_text_field(ui, true, &mut state.name, widgets::TextPolicy::AoName, |b, _| {
                    egui::TextEdit::singleline(b).desired_width(field)
                });
                let enter = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let create = widgets::primary_button_sized(ui, p, aurora_i18n::tr!("ao-welcome-create"), Vec2::new(button, 28.0)).clicked();
                if create || enter {
                    match inventory::valid_name(&state.name) {
                        Ok(name) => {
                            let mut set = Set::new(name);
                            set.default = true;
                            out.push(Action::Create(set));
                        }
                        Err(reason) => state.message = reason,
                    }
                }
            });
            ui.add_enabled_ui(!state.startup_loading, |ui| {
                if ui
                    .add(egui::Button::new(RichText::new(aurora_i18n::tr!("ao-welcome-migrate")).color(p.violet_pale)).frame(false))
                    .clicked()
                {
                    state.migration = Some(Default::default());
                }
            });
            if ui
                .add(egui::Button::new(RichText::new(aurora_i18n::tr!("ao-import")).color(p.violet_pale)).frame(false))
                .clicked()
            {
                state.picker = Some(true);
                state.folder = inv.root;
            }
        });
        ui.add_space(10.0);
        ui.add(egui::Label::new(RichText::new(aurora_i18n::tr!("ao-welcome-library")).small().color(p.muted)).wrap());
    });
}
