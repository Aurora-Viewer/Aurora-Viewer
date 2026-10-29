//! Inventory menu presentation only. Native notecard transfers follow
//! LLPreviewNotecard::loadAsset/saveIfNeeded; callbacks must match their request.
use super::{
    InvAction, Inventory, InventoryPreferences, InventoryUi,
    menu_layout::{Preferences, Profile, SHARED},
};
use crate::{theme::Palette, ui::widgets, world::inventory::actions};
use aurora_net::{
    NetCommand,
    inventory::{InvItem, notecard::Notecard},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub(crate) mod picker;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Export,
    Load,
}

#[derive(Default)]
pub(super) struct State {
    pub open: Option<Mode>,
    name: String,
    selected: Option<Uuid>,
    filter: String,
    picker: picker::Picker,
    pending_save: Option<Uuid>,
    pending_load: Option<Uuid>,
    message: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    format: String,
    version: u8,
    menus: Preferences,
}

fn invalid() -> String {
    aurora_i18n::tr!("inventory-menu-invalid-notecard").into()
}

fn valid(profile: &Profile) -> bool {
    let id = |text: &str| !text.is_empty() && text.len() <= 128 && text.bytes().all(|c| c.is_ascii_alphanumeric() || b":_-".contains(&c));
    valid_with(profile, id)
}

pub(crate) fn valid_with(profile: &Profile, id: impl Fn(&str) -> bool) -> bool {
    let l = &profile.layout;
    l.groups.len() <= 64
        && l.icon_bars.len() <= 16
        && l.separators.len() <= 128
        && l.placements.len() <= 2048
        && l.orders.len() <= 256
        && l.slots.len() <= 256
        && l.filters.len() <= 256
        && l.directions.len() <= 256
        && l.removed.len() <= 2048
        && profile.open.len() <= 256
        && l.groups
            .iter()
            .all(|(key, g)| id(key) && !g.name.trim().is_empty() && g.name.chars().count() <= 80)
        && l.placements.iter().all(|(key, zone)| id(key) && id(zone))
        && l.orders.iter().all(|(zone, entries)| {
            id(zone)
                && entries.len() <= 2048
                && entries.iter().all(|e| id(e))
                && entries.iter().collect::<std::collections::BTreeSet<_>>().len() == entries.len()
        })
        && l.slots.iter().all(|(zone, slots)| {
            id(zone) && slots.len() <= 2048 && slots.iter().all(|(key, slot)| id(key) && *slot < super::menu_layout::ICON_SLOTS)
        })
        && l.separators.iter().all(|(key, zone)| id(key) && id(zone))
        && l.filters.iter().chain(&l.icon_bars).chain(&l.removed).all(|key| id(key))
        && l.directions.keys().chain(profile.open.keys()).all(|key| id(key))
}

fn read(text: &str) -> Result<Preferences, String> {
    if text.len() > aurora_net::inventory::notecard::MAX_TEXT_BYTES {
        return Err(invalid());
    }
    let mut note: Document = serde_json::from_str(text).map_err(|_| invalid())?;
    if note.format != "aurora-inventory-menu"
        || note.version != 1
        || note.menus.profiles.len() != 1
        || !note.menus.profiles.get(SHARED).is_some_and(valid)
    {
        return Err(invalid());
    }
    for profile in note.menus.profiles.values_mut() {
        profile.locked = true;
    }
    Ok(note.menus)
}

fn write(prefs: &Preferences) -> Result<String, String> {
    let mut menus = prefs.clone();
    menus.unify();
    for profile in menus.profiles.values_mut() {
        profile.locked = true;
    }
    let text = serde_json::to_string_pretty(&Document {
        format: "aurora-inventory-menu".into(),
        version: 1,
        menus,
    })
    .map_err(|_| invalid())?;
    read(&text)?;
    Ok(text)
}

pub(crate) fn available(inv: &Inventory, id: Uuid) -> Option<&InvItem> {
    inv.items
        .get(&id)
        .filter(|item| item.asset_type == 7 && item.owner_mask & actions::COPY != 0 && !inv.in_folder_type(id, 14))
}

pub(crate) fn export(st: &mut InventoryUi, inv: &Inventory, name: String, text: String) -> Option<NetCommand> {
    let state = &mut st.menu_notecards;
    if state.pending_save.is_some() || state.pending_load.is_some() {
        return None;
    }
    let result = read(&text)
        .and_then(|_| Notecard::decode(text.as_bytes()))
        .and_then(|note| note.encode(&text));
    let parent = inv
        .folders
        .values()
        .find(|f| !f.library && f.info.type_default == 7)
        .map_or(inv.root, |f| f.info.id);
    match result {
        Ok(data) if !parent.is_nil() && !name.trim().is_empty() => {
            let request = Uuid::new_v4();
            state.pending_save = Some(request);
            state.message = aurora_i18n::tr!("notecard-saving").into();
            Some(NetCommand::CreateNotecardItem {
                request,
                parent,
                name: name.trim().chars().take(63).collect(),
                data,
                permissions: None,
            })
        }
        result => {
            state.message = result
                .err()
                .unwrap_or_else(|| aurora_i18n::tr!("session-saving-is-unavailable-in-this-region").into());
            st.message.clone_from(&state.message);
            None
        }
    }
}

pub(crate) fn load(st: &mut InventoryUi, inv: &Inventory, item: Uuid) -> Option<NetCommand> {
    let state = &mut st.menu_notecards;
    if state.pending_load.is_some() || state.pending_save.is_some() || !inv.can_preview(7) {
        return None;
    }
    let item = available(inv, item)?.clone();
    state.pending_load = Some(item.id);
    state.message = aurora_i18n::tr!("inventory-menu-loading").into();
    Some(NetCommand::PreviewInventoryItem(item))
}

fn feedback(st: &mut InventoryUi, result: Result<String, String>) {
    match result {
        Ok(message) => {
            st.menu_notecards.message.clear();
            st.message.clear();
            st.notice = Some((message, std::time::Instant::now()));
        }
        Err(reason) => {
            st.notice = None;
            st.menu_notecards.message = reason;
            st.message.clone_from(&st.menu_notecards.message);
        }
    }
}

pub(crate) fn saved(st: &mut InventoryUi, request: Uuid, result: &Result<InvItem, String>) -> bool {
    if st.menu_notecards.pending_save != Some(request) {
        return false;
    }
    st.menu_notecards.pending_save = None;
    feedback(
        st,
        result
            .as_ref()
            .map(|_| aurora_i18n::tr!("inventory-menu-saved").into())
            .map_err(Clone::clone),
    );
    true
}

pub(crate) fn loaded(st: &mut InventoryUi, prefs: &mut InventoryPreferences, item: Uuid, result: &Result<Vec<u8>, String>) -> bool {
    if st.menu_notecards.pending_load != Some(item) {
        return false;
    }
    st.menu_notecards.pending_load = None;
    let result = match result
        .as_ref()
        .map_err(Clone::clone)
        .and_then(|bytes| Notecard::decode(bytes))
        .and_then(|note| read(&note.text))
    {
        Ok(menus) => {
            prefs.context_menus = menus;
            st.menu_editor = Default::default();
            for window in &mut st.windows {
                window.state.menu_editor = Default::default();
            }
            Ok(aurora_i18n::tr!("inventory-menu-loaded").into())
        }
        Err(reason) => Err(reason),
    };
    feedback(st, result);
    true
}

/// Offline native callback; no grid item or transfer is created.
pub(super) fn demo_restore(st: &mut InventoryUi, prefs: &mut InventoryPreferences) {
    let item = Uuid::from_u128(8300);
    let data = write(&prefs.context_menus).and_then(|text| Notecard::decode(text.as_bytes()).and_then(|note| note.encode(&text)));
    st.menu_notecards.pending_load = Some(item);
    loaded(st, prefs, item, &data);
}

pub(super) fn demo_picker(st: &mut InventoryUi, search: bool) {
    st.menu_editor.notecard = None;
    st.menu_notecards.open = Some(Mode::Load);
    st.menu_notecards.picker.reveal(Uuid::from_u128(8103));
    if search {
        st.menu_notecards.filter = aurora_i18n::tr!("world-inventory-demo-instructions").into();
    }
}

pub(super) fn show(
    ctx: &egui::Context,
    p: &Palette,
    inv: &mut Inventory,
    st: &mut InventoryUi,
    prefs: &InventoryPreferences,
    actions: &mut Vec<InvAction>,
) {
    let requested = st
        .menu_editor
        .notecard
        .take()
        .or_else(|| st.windows.iter_mut().find_map(|w| w.state.menu_editor.notecard.take()));
    if let Some(mode) = requested {
        st.menu_notecards.open = Some(mode);
        st.menu_notecards.name = aurora_i18n::tr!("inventory-menu-notecard-name").into();
        st.menu_notecards.message.clear();
        st.menu_notecards.selected = None;
        st.menu_notecards.filter.clear();
        st.menu_notecards.picker = Default::default();
    }
    let state = &mut st.menu_notecards;
    let Some(mode) = state.open else { return };
    if mode == Mode::Load {
        st.demo_menu = None;
        let enabled = state.pending_save.is_none() && state.pending_load.is_none() && inv.can_preview(7);
        let (close, chosen) = state.picker.show(ctx, p, inv, &mut state.filter, &mut state.selected, enabled);
        if let Some(item) = chosen {
            actions.push(InvAction::LoadMenuNotecard(item));
        }
        if close {
            state.open = None;
        }
        return;
    }
    let mut close = false;
    let modal = widgets::Modal::new(egui::Id::unique("inventory-menu-notecard")).show(ctx, |ui| {
        ui.set_width(350.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(aurora_i18n::tr!("inventory-menu-export-notecard")).color(p.violet_light));
            close |= widgets::action_button(ui, p, "x", "", widgets::ActionKind::Flat, egui::vec2(24.0, 24.0))
                .on_hover_text(aurora_i18n::tr!("common-close"))
                .clicked();
        });
        ui.label(aurora_i18n::tr!("inventory-menu-notecard-help"));
        ui.label(aurora_i18n::tr!("common-name-2"));
        ui.add(
            egui::TextEdit::singleline(&mut state.name)
                .desired_width(ui.available_width())
                .char_limit(63),
        );
        if !state.message.is_empty() {
            ui.label(&state.message);
        }
        ui.horizontal(|ui| {
            close |= ui.button(aurora_i18n::tr!("common-cancel")).clicked();
            let enabled = state.pending_save.is_none() && state.pending_load.is_none() && !state.name.trim().is_empty();
            if widgets::default_button(ui.add_enabled(enabled, egui::Button::new(aurora_i18n::tr!("common-save")))).clicked() {
                match write(&prefs.context_menus) {
                    Ok(text) => {
                        actions.push(InvAction::ExportMenuNotecard {
                            name: state.name.clone(),
                            text,
                        });
                        close = true;
                    }
                    Err(reason) => state.message = reason,
                }
            }
        });
    });
    if close || modal.should_close() {
        state.open = None;
    }
}

#[cfg(test)]
mod tests {
    use super::super::menu_layout::{Direction, HIDDEN, LOCK, TEXT};
    use super::*;

    fn settings() -> InventoryPreferences {
        let mut prefs = InventoryPreferences::default();
        prefs.context_menus.unify();
        let profile = prefs.context_menus.profiles.get_mut(SHARED).unwrap();
        profile.locked = false;
        profile.layout.create_group(&[], "My actions", Direction::Horizontal, None);
        profile.layout.filters.insert("attach-body".into());
        profile.layout.placements.insert("copy".into(), HIDDEN.into());
        profile.layout.placements.insert(LOCK.into(), TEXT.into());
        profile
            .layout
            .slots
            .insert("top".into(), std::collections::BTreeMap::from([("paste".into(), 6)]));
        profile.open.insert("organize".into(), true);
        prefs
    }

    #[test]
    fn native_notecard_round_trip_preserves_shared_arrangement_filters_holes_and_groups_and_relocks() {
        let prefs = settings();
        let text = write(&prefs.context_menus).unwrap();
        let mut inv = Inventory {
            root: Uuid::from_u128(1),
            ..Default::default()
        };
        crate::world::inventory::demo::seed(&mut inv, Uuid::from_u128(2));
        let mut st = InventoryUi::default();
        let Some(NetCommand::CreateNotecardItem { request, parent, data, .. }) = export(&mut st, &inv, "Shared menu".into(), text) else {
            panic!("missing native upload: {}", st.message);
        };
        assert!(!parent.is_nil());
        let decoded = Notecard::decode(&data).unwrap();
        let mut expected = prefs.context_menus.clone();
        expected.profiles.get_mut(SHARED).unwrap().locked = true;
        assert_eq!(read(&decoded.text).unwrap(), expected);
        assert!(export(&mut st, &inv, "Duplicate".into(), write(&prefs.context_menus).unwrap()).is_none());
        let item = inv.items.values().find(|item| item.asset_type == 7).unwrap().clone();
        assert!(!saved(&mut st, Uuid::new_v4(), &Ok(item.clone())));
        assert_eq!(st.menu_notecards.pending_save, Some(request));
        assert!(saved(&mut st, request, &Ok(item.clone())));
        assert!(st.message.is_empty() && st.menu_notecards.message.is_empty());
        assert_eq!(
            st.success_notice(std::time::Instant::now()),
            Some(aurora_i18n::tr!("inventory-menu-saved"))
        );
        assert!(!saved(&mut st, request, &Err("late response".into())));
        assert!(matches!(load(&mut st, &inv, item.id), Some(NetCommand::PreviewInventoryItem(_))));
        let mut restored = InventoryPreferences::default();
        assert!(!loaded(&mut st, &mut restored, Uuid::new_v4(), &Ok(data.clone())));
        assert!(loaded(&mut st, &mut restored, item.id, &Ok(data)));
        assert!(st.message.is_empty() && st.menu_notecards.message.is_empty());
        assert_eq!(
            st.success_notice(std::time::Instant::now()),
            Some(aurora_i18n::tr!("inventory-menu-loaded"))
        );
        assert_eq!(restored.context_menus, expected);
        assert!(!loaded(&mut st, &mut restored, item.id, &Err("late response".into())));
    }

    #[test]
    fn invalid_or_failed_notecard_loads_are_atomic_and_permissions_are_rechecked() {
        let prefs = settings();
        let text = write(&prefs.context_menus).unwrap();
        let mut invalid_slot: serde_json::Value = serde_json::from_str(&text).unwrap();
        invalid_slot["menus"]["profiles"][SHARED]["layout"]["slots"]["top"]["paste"] = 8.into();
        for text in [
            "{}".into(),
            text.replace("aurora-inventory-menu", "aurora-construction-settings"),
            text.replace("\"version\": 1", "\"version\": 255"),
            invalid_slot.to_string(),
            "x".repeat(aurora_net::inventory::notecard::MAX_TEXT_BYTES + 1),
        ] {
            assert!(read(&text).is_err());
            let mut st = InventoryUi::default();
            let mut unchanged = prefs.clone();
            let item = Uuid::new_v4();
            st.menu_notecards.pending_load = Some(item);
            st.notice = Some(("Previous success".into(), std::time::Instant::now()));
            assert!(loaded(&mut st, &mut unchanged, item, &Ok(text.into_bytes())));
            assert_eq!(unchanged.context_menus, prefs.context_menus);
            assert!(!st.message.is_empty());
            assert!(st.notice.is_none());
        }
        let mut inv = Inventory::default();
        crate::world::inventory::demo::seed(&mut inv, Uuid::from_u128(2));
        let item = inv.items.values().find(|item| item.asset_type == 7).unwrap().id;
        let mut st = InventoryUi::default();
        inv.rlva_preview_mask = 2;
        assert!(load(&mut st, &inv, item).is_none());
        inv.rlva_preview_mask = 0;
        inv.items.get_mut(&item).unwrap().owner_mask &= !actions::COPY;
        assert!(load(&mut st, &inv, item).is_none());
        assert!(st.menu_notecards.pending_load.is_none());
    }
}
