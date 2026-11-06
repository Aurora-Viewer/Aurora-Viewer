//! Persistent compact AO controls, modelled on the Legacy Viewer's small AO panel.

use super::{Action, Floater, inventory, quick};
use crate::{
    ao::{Account, runtime::Runtime},
    movement::Preferences,
    theme::Palette,
    world::inventory::Inventory,
};
#[allow(clippy::too_many_arguments)]
pub fn show(
    ctx: &egui::Context,
    p: &Palette,
    catalog: &inventory::Catalog,
    inv: &Inventory,
    runtime: &mut Runtime,
    account: &mut Account,
    prefs: &mut Preferences,
    manager_open: &mut bool,
    busy: bool,
) -> Vec<Action> {
    let mut out = Vec::new();
    if !account.mini_open {
        return out;
    }
    let before = prefs.clone();
    let mut open = account.mini_open;
    let mut manager_requested = false;
    Floater::new(
        "ao_mini",
        aurora_i18n::tr!("ao-mini-title"),
        egui::pos2(260.0, 420.0),
        egui::vec2(280.0, 290.0),
    )
    .fixed()
    .show(ctx, p, &mut open, |ui| {
        let mut list_open = false;
        quick::body(
            ui,
            p,
            catalog,
            inv,
            runtime,
            account,
            prefs,
            busy,
            "ao_mini_set",
            &mut list_open,
            &mut out,
        );
        ui.horizontal(|ui| {
            if ui.button(aurora_i18n::tr!("ao-quick-manager")).clicked() {
                crate::ui::widgets::open_floater(ctx, "ao", manager_open);
                manager_requested = true;
            }
        });
    });
    if manager_requested {
        open = false;
    }
    account.mini_open = open;
    prefs.sanitize();
    if *prefs != before {
        out.push(Action::MovementPreferences);
    }
    out
}
