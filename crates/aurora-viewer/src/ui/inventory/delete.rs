//! LLInventoryPanel::handleKeyHere and LLPanelMainInventory::onTrashButtonClick
//! (Firestorm indra/newview, originally LGPL 2.1): delete uses the same selected
//! inventory action as the context menu, with no keyboard modifiers.

use super::*;
use aurora_net::inventory::operations::Operation;

pub(super) fn allowed(inv: &Inventory, ids: &[Uuid], prefs: &InventoryPreferences, facts: &Facts) -> bool {
    if ids.is_empty() {
        return false;
    }
    let holds_worn = rules::worn_ancestors(inv, &facts.worn);
    ids.iter()
        .all(|id| rules::removable(inv, *id, &prefs.protected) && !holds_worn.contains(id))
}

/// Build the mutation for a confirmed Delete: Trash move, or permanent purge
/// when every selected entry is already in the Trash.
pub(super) fn plan(inv: &Inventory, ids: &[Uuid], purge: bool, prefs: &InventoryPreferences, facts: &Facts) -> Result<Mutation, String> {
    if purge {
        let ids = rules::roots(inv, ids);
        let refresh = ids.iter().filter_map(|id| rules::parent(inv, *id)).collect();
        let operations = ids
            .iter()
            .map(|id| Operation::Delete {
                id: *id,
                folder: inv.folders.contains_key(id),
            })
            .collect();
        let holds_worn = rules::worn_ancestors(inv, &facts.worn);
        if ids
            .iter()
            .all(|id| rules::removable(inv, *id, &prefs.protected) && rules::in_type(inv, *id, 14) && !holds_worn.contains(id))
        {
            Ok(Mutation {
                operations,
                refresh,
                removed: ids,
                ..Default::default()
            })
        } else {
            Err(aurora_i18n::tr!("ui-inventory-dialogs-the-selection-can-no-longer-be-purged").into())
        }
    } else {
        rules::system(inv, 14)
            .ok_or_else(|| aurora_i18n::tr!("common-trash-folder-not-found").to_owned())
            .and_then(|to| rules::move_selection(inv, ids, to, &prefs.protected, &facts.worn))
    }
}

/// Open the Delete confirmation, or apply a Trash move immediately when the
/// preference is off. Purge from Trash always asks (PurgeSelectedItems has no
/// ignore box). `DeleteItems` notification, indra/newview/skins/.../notifications.xml.
pub(super) fn confirm_or_run(
    inv: &Inventory,
    st: &mut InventoryUi,
    prefs: &InventoryPreferences,
    facts: &Facts,
    actions: &mut Vec<InvAction>,
    mut ids: Vec<Uuid>,
) {
    if st.pending.is_some() || st.dialog.is_some() {
        return;
    }
    ids.sort();
    if !allowed(inv, &ids, prefs, facts) {
        return;
    }
    // A selection spanning trash and another folder first moves to trash.
    // Permanent deletion is offered only when every selected entry is there.
    let purge = ids.iter().all(|id| rules::in_type(inv, *id, 14));
    if purge || prefs.confirm_delete {
        st.dialog = Some(EditDialog::Delete(ids, purge));
        return;
    }
    match plan(inv, &ids, false, prefs, facts) {
        Ok(change) => actions.push(InvAction::Edit(change)),
        Err(reason) => st.message = reason,
    }
}

pub(super) fn request(inv: &Inventory, st: &mut InventoryUi, prefs: &InventoryPreferences, facts: &Facts, actions: &mut Vec<InvAction>) {
    let ids: Vec<_> = st.selection.iter().copied().collect();
    confirm_or_run(inv, st, prefs, facts, actions, ids);
}

/// LLInventoryPanel::handleKeyHere: on a Mac the key labelled Delete sends
/// Backspace, so both delete there (`#if LL_DARWIN`); elsewhere Backspace does
/// nothing in the list.
pub(super) fn deletes(key: egui::Key, mac: bool) -> bool {
    key == egui::Key::Delete || (mac && key == egui::Key::Backspace)
}

pub(super) fn shortcut(
    ui: &mut egui::Ui,
    inv: &Inventory,
    st: &mut InventoryUi,
    prefs: &InventoryPreferences,
    facts: &Facts,
    actions: &mut Vec<InvAction>,
) {
    // Called only by the focused list's shortcut dispatcher. Text editors,
    // popups and other inventory windows retain their own Delete handling.
    let pressed = ui.input_mut(|input| {
        let mut pressed = false;
        input.events.retain(|event| {
            if matches!(event, egui::Event::Key { key, pressed: true, repeat: false, modifiers, .. } if modifiers.is_none() && deletes(*key, cfg!(target_os = "macos")))
            {
                pressed = true;
                false
            } else {
                true
            }
        });
        pressed
    });
    if pressed {
        request(inv, st, prefs, facts, actions);
    }
}

pub(super) fn button(
    ui: &mut egui::Ui,
    p: &Palette,
    inv: &Inventory,
    st: &mut InventoryUi,
    prefs: &InventoryPreferences,
    facts: &Facts,
    actions: &mut Vec<InvAction>,
) {
    let ids: Vec<_> = st.selection.iter().copied().collect();
    let enabled = st.pending.is_none() && st.dialog.is_none() && allowed(inv, &ids, prefs, facts);
    let button =
        ui.scope(|ui| super::super::widgets::icon_button(ui, p, "trash-filled", aurora_i18n::tr!("inventory-delete-shortcut"), enabled));
    if cfg!(test) || demo_mode().is_some() {
        ui.ctx()
            .data_mut(|data| data.insert_temp(egui::Id::unique(("inventory_delete_button", st.ui_id)), button.response.rect));
    }
    if button.inner {
        request(inv, st, prefs, facts, actions);
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
    // `delete-enter`: the Delete key, then Enter answers the confirmation
    let key = match (mode, frame) {
        ("delete-key" | "delete-enter", 680) => Some(egui::Key::Delete),
        ("delete-enter", 740) => Some(egui::Key::Enter),
        _ => None,
    };
    if let Some(key) = key {
        return vec![egui::Event::Key {
            key,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    #[test]
    fn skipping_confirm_moves_to_trash_without_a_dialog() {
        let mut inv = Inventory {
            root: id(1),
            ..Default::default()
        };
        inv.folders.insert(
            id(1),
            crate::world::inventory::Folder {
                info: aurora_net::inventory::InvFolder {
                    id: id(1),
                    parent: Uuid::nil(),
                    type_default: 8,
                    name: "Root".into(),
                    ..Default::default()
                },
                children: vec![id(14)],
                items: vec![id(10)],
                state: crate::world::inventory::FetchState::Fetched,
                library: false,
            },
        );
        inv.folders.insert(
            id(14),
            crate::world::inventory::Folder {
                info: aurora_net::inventory::InvFolder {
                    id: id(14),
                    parent: id(1),
                    type_default: 14,
                    name: "Trash".into(),
                    ..Default::default()
                },
                children: Vec::new(),
                items: Vec::new(),
                state: crate::world::inventory::FetchState::Fetched,
                library: false,
            },
        );
        inv.items.insert(
            id(10),
            aurora_net::inventory::InvItem {
                id: id(10),
                parent: id(1),
                name: "Shirt".into(),
                asset_type: 6,
                owner_mask: 0x7fff_ffff,
                ..Default::default()
            },
        );
        let prefs = InventoryPreferences {
            confirm_delete: false,
            ..Default::default()
        };
        let facts = Facts::default();
        let mut st = InventoryUi {
            selection: HashSet::from([id(10)]),
            selected: Some(id(10)),
            ..Default::default()
        };
        let mut actions = Vec::new();
        request(&inv, &mut st, &prefs, &facts, &mut actions);
        assert!(st.dialog.is_none(), "no confirmation when the preference is off");
        assert!(matches!(actions.as_slice(), [InvAction::Edit(change)] if change.move_only()));
    }
}
