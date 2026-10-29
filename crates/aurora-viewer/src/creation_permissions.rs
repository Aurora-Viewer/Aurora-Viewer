//! Creation defaults follow LLFloaterPermsDefault and create_new_item
//! (indra/newview/llfloaterperms.cpp, llviewerinventory.cpp).
use aurora_net::{NetCommand, build::perm, inventory::operations::NewItem, inventory::upload::Permissions};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rights {
    pub modify: bool,
    pub copy: bool,
    pub transfer: bool,
    pub share_group: bool,
    pub everyone_copy: bool,
}

impl Default for Rights {
    fn default() -> Self {
        Self::new(false, false)
    }
}

impl Rights {
    const fn new(modify: bool, copy: bool) -> Self {
        Self {
            modify,
            copy,
            transfer: true,
            share_group: false,
            everyone_copy: false,
        }
    }

    pub fn permissions(self) -> Permissions {
        Permissions {
            next_owner: perm::MOVE
                | if self.modify { perm::MODIFY } else { 0 }
                | if self.copy { perm::COPY } else { 0 }
                | if self.transfer || !self.copy { perm::TRANSFER } else { 0 },
            group: if self.share_group {
                perm::COPY | perm::MOVE | perm::MODIFY
            } else {
                0
            },
            everyone: if self.everyone_copy { perm::COPY } else { 0 },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Defaults {
    pub scripts: Rights,
    pub notecards: Rights,
    pub gestures: Rights,
    pub materials: Rights,
    pub wearables: Rights,
    pub settings: Rights,
}

impl Default for Defaults {
    fn default() -> Self {
        Self {
            scripts: Rights::new(false, false),
            notecards: Rights::new(true, true),
            gestures: Rights::new(true, true),
            materials: Rights::new(true, false),
            wearables: Rights::new(false, false),
            settings: Rights::new(true, true),
        }
    }
}

impl Defaults {
    pub fn for_item(self, kind: NewItem) -> Option<Permissions> {
        Some(
            match kind {
                NewItem::Script => self.scripts,
                NewItem::Note => self.notecards,
                NewItem::Gesture => self.gestures,
                NewItem::Material => self.materials,
                NewItem::Wearable(_) => self.wearables,
                NewItem::Settings(_) => {
                    // Settings always allow copying; group/everyone controls are
                    // unavailable (floater_perms_default.xml, LLSettingsInventoryCB).
                    return Some(Permissions {
                        next_owner: Rights {
                            copy: true,
                            ..self.settings
                        }
                        .permissions()
                        .next_owner,
                        group: 0,
                        everyone: perm::COPY,
                    });
                }
                NewItem::Landmark => return None,
            }
            .permissions(),
        )
    }

    /// Resolve once, before the network's asynchronous upload or demo path.
    /// Explicit permissions (for example Save As) belong to the source item.
    pub fn prepare(self, command: &mut NetCommand) {
        match command {
            NetCommand::CreateInventoryItem { kind, permissions, .. } => {
                if permissions.is_none() {
                    *permissions = self.for_item(*kind);
                }
            }
            NetCommand::CreateInventoryWearable { kind, permissions, .. } => {
                if permissions.is_none() {
                    *permissions = self.for_item(NewItem::Wearable(*kind));
                }
            }
            NetCommand::CreateNotecardItem { permissions, .. } => {
                if permissions.is_none() {
                    *permissions = self.for_item(NewItem::Note);
                }
            }
            NetCommand::CreateSettingsItem { next_owner_mask, .. } => {
                if next_owner_mask.is_none() {
                    *next_owner_mask = self.for_item(NewItem::Settings(0)).map(|p| p.next_owner);
                }
            }
            NetCommand::Build(aurora_net::build::BuildCmd::RezScript { item, .. }) if item.item_id.is_nil() => {
                let rights = self.scripts.permissions();
                item.next_owner_mask = rights.next_owner;
                item.group_mask = rights.group;
                item.everyone_mask = rights.everyone;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creation_defaults_keep_legacy_categories_and_no_copy_transfer_rule() {
        let defaults = Defaults::default();
        assert_eq!(defaults.for_item(NewItem::Script), Some(Permissions::default()));
        assert_eq!(defaults.for_item(NewItem::Wearable(0)), defaults.for_item(NewItem::Wearable(16)));
        assert_eq!(
            defaults.notecards.permissions().next_owner,
            perm::MOVE | perm::MODIFY | perm::COPY | perm::TRANSFER
        );
        let rights = Rights {
            transfer: false,
            share_group: true,
            everyone_copy: true,
            ..Rights::default()
        }
        .permissions();
        assert_ne!(rights.next_owner & perm::TRANSFER, 0);
        assert_eq!(rights.group, perm::COPY | perm::MODIFY | perm::MOVE);
        assert_eq!(rights.everyone, perm::COPY);
        assert_eq!(defaults.for_item(NewItem::Landmark), None);
    }

    #[test]
    fn creation_defaults_migrate_old_settings_and_round_trip_independent_rows() {
        let mut prefs: crate::ui::inventory::InventoryPreferences = serde_json::from_str("{}").unwrap();
        assert_eq!(prefs.creation_permissions, Defaults::default());
        prefs.creation_permissions.scripts.copy = true;
        prefs.creation_permissions.scripts.transfer = false;
        let loaded: crate::ui::inventory::InventoryPreferences = serde_json::from_str(&serde_json::to_string(&prefs).unwrap()).unwrap();
        assert_eq!(loaded, prefs);
        assert_eq!(loaded.creation_permissions.notecards, Defaults::default().notecards);
    }

    #[test]
    fn creation_defaults_preserve_explicit_rights_and_settings_copy() {
        let defaults = Defaults::default();
        let settings = defaults.for_item(NewItem::Settings(0)).unwrap();
        assert_ne!(settings.next_owner & perm::COPY, 0);
        assert_eq!((settings.group, settings.everyone), (0, perm::COPY));
        let mut command = NetCommand::CreateInventoryItem {
            parent: uuid::Uuid::from_u128(1),
            kind: NewItem::Script,
            name: "Script".into(),
            permissions: None,
        };
        defaults.prepare(&mut command);
        let NetCommand::CreateInventoryItem { ref mut permissions, .. } = command else {
            panic!("creation")
        };
        assert_eq!(*permissions, Some(defaults.scripts.permissions()));
        *permissions = Some(settings);
        defaults.prepare(&mut command);
        assert!(matches!(command, NetCommand::CreateInventoryItem { permissions: Some(p), .. } if p == settings));
    }

    #[test]
    fn creation_defaults_capture_upload_rights_and_preserve_save_as_and_existing_scripts() {
        let mut defaults = Defaults::default();
        defaults.wearables.copy = true;
        defaults.wearables.transfer = false;
        let snapshot = defaults.wearables.permissions();
        let mut wearable = NetCommand::CreateInventoryWearable {
            parent: uuid::Uuid::from_u128(1),
            kind: 0,
            name: "Shape".into(),
            data: Vec::new(),
            permissions: None,
        };
        defaults.prepare(&mut wearable);
        defaults.wearables = Rights::default();
        defaults.prepare(&mut wearable);
        assert!(matches!(wearable, NetCommand::CreateInventoryWearable { permissions: Some(p), .. } if p == snapshot));
        let mut settings = NetCommand::CreateSettingsItem {
            request: uuid::Uuid::nil(),
            parent: uuid::Uuid::nil(),
            name: "Save As".into(),
            subtype: 0,
            next_owner_mask: Some(perm::COPY),
            data: Vec::new(),
        };
        defaults.prepare(&mut settings);
        assert!(matches!(
            settings,
            NetCommand::CreateSettingsItem {
                next_owner_mask: Some(perm::COPY),
                ..
            }
        ));
        defaults.scripts.share_group = true;
        let mut script = NetCommand::Build(aurora_net::build::BuildCmd::RezScript {
            handle: 0,
            local_id: 1,
            enabled: true,
            group_id: uuid::Uuid::nil(),
            item: Box::default(),
        });
        defaults.prepare(&mut script);
        let NetCommand::Build(aurora_net::build::BuildCmd::RezScript { ref mut item, .. }) = script else {
            panic!("script")
        };
        assert_eq!(
            (item.next_owner_mask, item.group_mask),
            (defaults.scripts.permissions().next_owner, defaults.scripts.permissions().group)
        );
        item.item_id = uuid::Uuid::from_u128(2);
        item.next_owner_mask = perm::COPY;
        defaults.prepare(&mut script);
        assert!(
            matches!(script, NetCommand::Build(aurora_net::build::BuildCmd::RezScript { item, .. }) if item.next_owner_mask == perm::COPY)
        );
    }
}
