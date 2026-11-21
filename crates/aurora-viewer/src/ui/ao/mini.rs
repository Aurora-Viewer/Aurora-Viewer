//! Persistent compact AO controls, after the small mode of FloaterAO
//! (indra/newview/ao.cpp, `animation_overrider_panel_small` in
//! skins/default/xui/en/panel_ao.xml): the set list that switches the set at
//! once (`onSelectSetSmall`), Previous / Next and the Sits checkbox
//! (`onCheckOverrideSitsSmall`).
//!
//! Differences with the Legacy Viewer:
//! - its small panel has only the set list, Previous / Next, Sits and More;
//!   the auto-cycle button, the "now playing" line and the Enabled, Swimming
//!   and Water walk switches are Aurora's (shared with the toolbar popup,
//!   `quick::body`);
//! - it is one floater that switches between its small and full modes
//!   (`onClickMore` / `onClickLess`, remembered in `UseFullAOInterface`).
//!   Here the mini window is a window of its own, remembered per account
//!   (`Account::mini_open`): its Settings button opens the full AO window and
//!   the mini window stays open, so that both can be shown together.

use super::{Action, Floater, inventory, quick};
use crate::{
    ao::{Account, runtime::Runtime},
    movement::Preferences,
    theme::Palette,
    world::inventory::Inventory,
};

/// The floater id of the mini window.
pub const WINDOW: &str = "ao_mini";

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
    Floater::new(
        WINDOW,
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
            // Opens (or expands) the full AO window; the mini window stays
            // open, unlike FloaterAO::onClickMore (see the module comment).
            if ui.button(aurora_i18n::tr!("ao-quick-manager")).clicked() {
                crate::ui::widgets::open_floater(ctx, "ao", manager_open);
            }
        });
    });
    account.mini_open = open;
    prefs.sanitize();
    if *prefs != before {
        out.push(Action::MovementPreferences);
    }
    out
}
