//! LLInventoryPanel::handleKeyHere and LLPanelMainInventory::onTrashButtonClick
//! (Firestorm indra/newview, originally LGPL 2.1): delete uses the same selected
//! inventory action as the context menu, with no keyboard modifiers.

use super::*;

pub(super) fn allowed(inv: &Inventory, ids: &[Uuid], prefs: &InventoryPreferences, facts: &Facts) -> bool {
    if ids.is_empty() {
        return false;
    }
    let holds_worn = rules::worn_ancestors(inv, &facts.worn);
    ids.iter()
        .all(|id| rules::removable(inv, *id, &prefs.protected) && !holds_worn.contains(id))
}

pub(super) fn request(inv: &Inventory, st: &mut InventoryUi, prefs: &InventoryPreferences, facts: &Facts) {
    if st.pending.is_some() || st.dialog.is_some() {
        return;
    }
    let mut ids: Vec<_> = st.selection.iter().copied().collect();
    ids.sort();
    if allowed(inv, &ids, prefs, facts) {
        // A selection spanning trash and another folder first moves to trash.
        // Permanent deletion is offered only when every selected entry is there.
        let purge = ids.iter().all(|id| rules::in_type(inv, *id, 14));
        st.dialog = Some(EditDialog::Delete(ids, purge));
    }
}

pub(super) fn shortcut(ui: &mut egui::Ui, inv: &Inventory, st: &mut InventoryUi, prefs: &InventoryPreferences, facts: &Facts) {
    // Called only by the focused list's shortcut dispatcher. Text editors,
    // popups and other inventory windows retain their own Delete handling.
    let pressed = ui.input_mut(|input| {
        let mut pressed = false;
        input.events.retain(|event| {
            if matches!(event, egui::Event::Key { key: egui::Key::Delete, pressed: true, repeat: false, modifiers, .. } if modifiers.is_none()) {
                pressed = true;
                false
            } else {
                true
            }
        });
        pressed
    });
    if pressed {
        request(inv, st, prefs, facts);
    }
}

pub(super) fn button(ui: &mut egui::Ui, p: &Palette, inv: &Inventory, st: &mut InventoryUi, prefs: &InventoryPreferences, facts: &Facts) {
    let ids: Vec<_> = st.selection.iter().copied().collect();
    let enabled = st.pending.is_none() && st.dialog.is_none() && allowed(inv, &ids, prefs, facts);
    let button =
        ui.scope(|ui| super::super::widgets::icon_button(ui, p, "trash-filled", aurora_i18n::tr!("inventory-delete-shortcut"), enabled));
    if cfg!(test) || demo_mode().is_some() {
        ui.ctx()
            .data_mut(|data| data.insert_temp(egui::Id::unique(("inventory_delete_button", st.ui_id)), button.response.rect));
    }
    if button.inner {
        request(inv, st, prefs, facts);
    }
}

fn demo_mode() -> Option<&'static str> {
    static MODE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    MODE.get_or_init(|| {
        std::env::var("AURORA_DEMO_INVENTORY")
            .ok()
            .filter(|mode| mode.starts_with("delete-"))
    })
    .as_deref()
}

pub(super) fn demo_enabled() -> bool {
    demo_mode().is_some()
}

/// Actual list selection, followed by a native Delete event or footer click.
pub fn demo_events(ctx: &egui::Context, frame: u64, st: &InventoryUi) -> Vec<egui::Event> {
    let Some(mode) = demo_mode() else { return Vec::new() };
    if frame == 640 {
        super::super::widgets::place_floater(ctx, "inventory", egui::pos2(20.0, 56.0), egui::vec2(680.0, 850.0));
    }
    if mode == "delete-key" && frame == 680 {
        return vec![egui::Event::Key {
            key: egui::Key::Delete,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }];
    }
    let pos = match frame {
        650 | 651 => {
            ctx.data(|data| data.get_temp::<egui::Pos2>(egui::Id::unique(("clipboard_demo_row", st.ui_id, Uuid::from_u128(8001)))))
        }
        680 | 681 if mode == "delete-button" => ctx
            .data(|data| data.get_temp::<egui::Rect>(egui::Id::unique(("inventory_delete_button", st.ui_id))))
            .map(|rect| rect.center()),
        _ => None,
    };
    pos.map_or_else(Vec::new, |pos| {
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: matches!(frame, 650 | 680),
                modifiers: Default::default(),
            },
        ]
    })
}
