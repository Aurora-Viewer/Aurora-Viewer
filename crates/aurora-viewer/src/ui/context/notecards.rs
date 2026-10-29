//! Versioned scene presentation only. Native transfers and the inventory's
//! COPY-aware picker follow LLPreviewNotecard's load/save cycle; no picked
//! identities, coordinates or executable commands belong in the document.
use super::Preferences;
use crate::{
    theme::Palette,
    ui::{
        inventory::context_notecards::{self, Mode, picker::Picker},
        widgets,
    },
    world::inventory::Inventory,
};
use aurora_net::{
    NetCommand,
    inventory::{InvItem, notecard::Notecard},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const SCOPES: [&str; 7] = [
    "scene:land",
    "scene:object",
    "scene:self",
    "scene:avatar",
    "scene:attachment",
    "scene:other-attachment",
    "scene:hud",
];

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    format: String,
    version: u8,
    menus: Preferences,
}

fn invalid() -> String {
    aurora_i18n::tr!("scene-menu-invalid-notecard").into()
}

fn read(text: &str) -> Result<Preferences, String> {
    if text.len() > aurora_net::inventory::notecard::MAX_TEXT_BYTES {
        return Err(invalid());
    }
    let mut doc: Document = serde_json::from_str(text).map_err(|_| invalid())?;
    let id = |s: &str| !s.is_empty() && s.len() <= 512 && s.bytes().all(|c| c.is_ascii_alphanumeric() || b":_-/".contains(&c));
    if doc.format != "aurora-scene-menus"
        || doc.version != 1
        || doc.menus.profiles.len() > SCOPES.len()
        || !doc
            .menus
            .profiles
            .iter()
            .all(|(scope, profile)| SCOPES.contains(&scope.as_str()) && context_notecards::valid_with(profile, id))
    {
        return Err(invalid());
    }
    for profile in doc.menus.profiles.values_mut() {
        profile.locked = true;
    }
    Ok(doc.menus)
}

fn write(menus: &Preferences) -> Result<String, String> {
    let text = serde_json::to_string_pretty(&Document {
        format: "aurora-scene-menus".into(),
        version: 1,
        menus: menus.clone(),
    })
    .map_err(|_| invalid())?;
    let locked = read(&text)?;
    serde_json::to_string_pretty(&Document {
        format: "aurora-scene-menus".into(),
        version: 1,
        menus: locked,
    })
    .map_err(|_| invalid())
}

#[derive(Default)]
pub(crate) struct State {
    mode: Option<Mode>,
    name: String,
    picker: Picker,
    selected: Option<Uuid>,
    filter: String,
    saving: Option<Uuid>,
    loading: Option<Uuid>,
    message: Option<String>,
    status_dismissed: bool,
}

impl State {
    pub(super) fn open(&mut self, mode: Mode) {
        if self.saving.is_some() || self.loading.is_some() {
            return;
        }
        self.mode = Some(mode);
        self.name = aurora_i18n::tr!("scene-menu-notecard-name").into();
        self.picker = Default::default();
        self.selected = None;
        self.filter.clear();
        self.message = None;
        self.status_dismissed = false;
    }

    fn export(&mut self, inv: &Inventory, prefs: &Preferences) -> Result<NetCommand, String> {
        let text = write(prefs)?;
        let data = Notecard::decode(text.as_bytes()).and_then(|note| note.encode(&text))?;
        let parent = inv
            .folders
            .values()
            .find(|f| !f.library && f.info.type_default == 7)
            .map_or(inv.root, |f| f.info.id);
        if parent.is_nil() || self.name.trim().is_empty() || self.saving.is_some() || self.loading.is_some() {
            return Err(aurora_i18n::tr!("session-saving-is-unavailable-in-this-region").into());
        }
        let request = Uuid::new_v4();
        self.saving = Some(request);
        self.status_dismissed = false;
        Ok(NetCommand::CreateNotecardItem {
            request,
            parent,
            name: self.name.trim().chars().take(63).collect(),
            data,
            permissions: None,
        })
    }

    fn load(&mut self, inv: &Inventory, item: Uuid) -> Option<NetCommand> {
        if self.saving.is_some() || self.loading.is_some() || !inv.can_preview(7) {
            return None;
        }
        let item = context_notecards::available(inv, item)?.clone();
        self.loading = Some(item.id);
        self.status_dismissed = false;
        Some(NetCommand::PreviewInventoryItem(item))
    }

    pub(crate) fn saved(&mut self, request: Uuid, result: &Result<InvItem, String>) -> bool {
        if self.saving != Some(request) {
            return false;
        }
        self.saving = None;
        self.status_dismissed = false;
        self.message = Some(
            result
                .as_ref()
                .map(|_| aurora_i18n::tr!("scene-menu-saved").into())
                .unwrap_or_else(Clone::clone),
        );
        true
    }

    pub(crate) fn loaded(&mut self, item: Uuid, result: &Result<Vec<u8>, String>) -> Option<Preferences> {
        if self.loading != Some(item) {
            return None;
        }
        self.loading = None;
        self.status_dismissed = false;
        match result
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|bytes| Notecard::decode(bytes))
            .and_then(|note| read(&note.text))
        {
            Ok(menus) => {
                self.message = Some(aurora_i18n::tr!("scene-menu-loaded").into());
                Some(menus)
            }
            Err(reason) => {
                self.message = Some(reason);
                None
            }
        }
    }

    pub(crate) fn show(&mut self, ctx: &egui::Context, p: &Palette, inv: &mut Inventory, prefs: &Preferences) -> Option<NetCommand> {
        let mut command = None;
        if self.mode == Some(Mode::Load) {
            let (close, chosen) = self
                .picker
                .show(ctx, p, inv, &mut self.filter, &mut self.selected, inv.can_preview(7));
            if let Some(item) = chosen {
                command = self.load(inv, item);
            }
            if close || command.is_some() {
                self.mode = None;
            }
        } else if self.mode == Some(Mode::Export) {
            let mut close = false;
            let mut save = false;
            let modal = widgets::Modal::new(egui::Id::unique("scene-menu-notecard-export")).show(ctx, |ui| {
                ui.set_width(380.0);
                ui.label(egui::RichText::new(aurora_i18n::tr!("inventory-menu-export-notecard")).color(p.violet_light));
                ui.label(aurora_i18n::tr!("scene-menu-notecard-help"));
                ui.label(aurora_i18n::tr!("common-name-2"));
                ui.add(
                    egui::TextEdit::singleline(&mut self.name)
                        .char_limit(63)
                        .desired_width(ui.available_width()),
                );
                ui.horizontal(|ui| {
                    close |= ui.button(aurora_i18n::tr!("common-cancel")).clicked();
                    save = widgets::default_button(
                        ui.add_enabled(!self.name.trim().is_empty(), egui::Button::new(aurora_i18n::tr!("common-save"))),
                    )
                    .clicked();
                });
            });
            if save {
                match self.export(inv, prefs) {
                    Ok(cmd) => command = Some(cmd),
                    Err(reason) => self.message = Some(reason),
                }
                close = true;
            }
            if close || modal.should_close() {
                self.mode = None;
            }
        }
        if self.mode.is_none() && !self.status_dismissed && (self.message.is_some() || self.saving.is_some() || self.loading.is_some()) {
            let mut close = false;
            let pending = self.saving.is_some() || self.loading.is_some();
            let modal = widgets::Modal::new(egui::Id::unique("scene-menu-notecard-result")).show(ctx, |ui| {
                ui.set_width(380.0);
                ui.label(self.message.clone().unwrap_or_else(|| aurora_i18n::tr!("common-loading").into()));
                if pending {
                    ctx.request_repaint_after(std::time::Duration::from_millis(100));
                }
                close = widgets::default_button(ui.button(aurora_i18n::tr!("common-close"))).clicked();
            });
            if close || modal.should_close() {
                self.message = None;
                self.status_dismissed = true;
            }
        }
        command
    }

    pub(super) fn demo(&mut self, mode: Mode) {
        self.open(mode);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::inventory::menu_layout::{Profile, TEXT};

    #[test]
    fn seven_independent_profiles_roundtrip_relock_without_commands_or_picks() {
        let mut prefs = Preferences::default();
        for scope in SCOPES {
            let mut profile = Profile {
                locked: false,
                ..Default::default()
            };
            profile
                .layout
                .placements
                .insert("/group:cube-filled:0/ObjectProfile:0".into(), TEXT.into());
            prefs.profiles.insert(scope.into(), profile);
        }
        let text = write(&prefs).unwrap();
        let imported = read(&text).unwrap();
        assert_eq!(imported.profiles.len(), 7);
        for (scope, profile) in &mut prefs.profiles {
            profile.locked = true;
            assert_eq!(imported.profiles.get(scope), Some(&*profile));
        }
        assert!(read(&text.replace("scene:land", "inventory:shared")).is_err());
        assert!(read(&text.replace("aurora-scene-menus", "aurora-inventory-menu")).is_err());
        assert!(read(&text.replace("\"version\": 1", "\"version\": 255")).is_err());
        assert!(read(&"x".repeat(aurora_net::inventory::notecard::MAX_TEXT_BYTES + 1)).is_err());
    }

    #[test]
    fn native_transfer_checks_copy_and_correlates_callbacks_without_partial_import() {
        let mut inv = Inventory {
            root: Uuid::from_u128(1),
            ..Default::default()
        };
        crate::world::inventory::demo::seed(&mut inv, Uuid::from_u128(2));
        let item = inv.items.values().find(|item| item.asset_type == 7).unwrap().clone();
        let mut state = State {
            name: "Scene menus".into(),
            ..Default::default()
        };
        let prefs = Preferences::default();
        let NetCommand::CreateNotecardItem { request, data, .. } = state.export(&inv, &prefs).unwrap() else {
            panic!("native notecard upload");
        };
        assert!(!state.saved(Uuid::new_v4(), &Ok(item.clone())));
        assert!(state.load(&inv, item.id).is_none());
        assert!(state.saved(request, &Ok(item.clone())));
        inv.items.get_mut(&item.id).unwrap().owner_mask &= !crate::world::inventory::actions::COPY;
        assert!(state.load(&inv, item.id).is_none());
        inv.items.insert(item.id, item.clone());
        inv.rlva_preview_mask = 2;
        assert!(state.load(&inv, item.id).is_none());
        inv.rlva_preview_mask = 0;
        assert!(state.load(&inv, item.id).is_some());
        assert!(state.loaded(Uuid::new_v4(), &Ok(data.clone())).is_none());
        assert_eq!(state.loading, Some(item.id));
        assert_eq!(state.loaded(item.id, &Ok(data)), Some(prefs));
        assert!(state.loaded(item.id, &Err("late".into())).is_none());
        assert!(state.load(&inv, item.id).is_some());
        assert!(state.loaded(item.id, &Ok(b"{}".to_vec())).is_none());
        assert!(state.loading.is_none());
        assert!(!state.message.as_ref().unwrap().is_empty());
    }
}
