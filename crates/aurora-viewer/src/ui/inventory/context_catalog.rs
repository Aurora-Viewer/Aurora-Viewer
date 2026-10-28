//! Native inventory commands and their current availability, before presentation.
mod extra;
use super::menu_layout::{Direction, Entry, Kind, LOCK, TEXT, children};
use super::*;
use crate::world::appearance::Action as Wear;
pub(super) use extra::FolderCache;
pub(super) use extra::process_deferred;

pub(super) enum Command {
    Extra(extra::Command),
    Invoke(InvAction),
    Folder(Uuid),
    Upload(usize, Uuid),
    Empty(Uuid),
    DefaultFolder(Vec<Uuid>),
    Delete(Vec<Uuid>, bool),
    Share(Vec<Uuid>),
    Open(Uuid),
    OpenMany(Vec<Uuid>),
    PropertiesMany(Vec<Uuid>),
    Batch(Vec<InvAction>),
    Properties(Uuid),
    BulkPermissions(Vec<Uuid>),
    Rename(Uuid),
    Image(Uuid),
    Protect(Uuid),
    Reload(Uuid),
    Window(Uuid),
    Asset(Uuid),
    Copy(Vec<Uuid>, bool),
    Move(Uuid),
    Paste(Uuid, bool),
    Original(Uuid),
    Links(Uuid),
    Replace(Uuid),
    Group(Vec<Uuid>),
    Ungroup(Uuid),
    Favorite(Vec<Uuid>, bool),
    Worn(crate::ui::context::CtxAction),
    Marketplace(Vec<Uuid>, bool),
}

#[derive(Default)]
pub(super) struct Catalog {
    pub entries: Vec<Entry>,
    pub commands: Vec<Option<Command>>,
    pub prepare_outfit: Option<Uuid>,
    /// Complete presentation metadata; never retains executable commands.
    editing: bool,
}
impl Catalog {
    fn add(&mut self, id: impl Into<String>, zone: &str, icon: &str, label: &str, enabled: bool, command: Command) {
        let id = id.into();
        if self.editing && self.entries.iter().any(|e| e.id == id) {
            return;
        }
        let index = if self.editing { usize::MAX } else { self.commands.len() };
        if !self.editing {
            self.commands.push(Some(command));
        }
        self.entries.push(Entry {
            id,
            zone: zone.into(),
            label: label.into(),
            icon: icon.into(),
            enabled: enabled && !self.editing,
            checked: None,
            kind: Kind::Action(index),
            slot: None,
        });
    }
    fn group(&mut self, id: &str, zone: &str, icon: &str, label: &str, enabled: bool, direction: Direction, open: bool) -> String {
        if self.editing && self.entries.iter().any(|e| e.id == id) {
            return children(id);
        }
        self.entries.push(Entry {
            id: id.into(),
            zone: zone.into(),
            label: label.into(),
            icon: icon.into(),
            enabled: enabled && !self.editing,
            checked: None,
            kind: Kind::Group { direction, open },
            slot: None,
        });
        children(id)
    }
    fn divider(&mut self, id: &str, zone: &str) {
        if self.editing && self.entries.iter().any(|e| e.id == format!("separator:{id}")) {
            return;
        }
        self.entries.push(Entry {
            id: format!("separator:{id}"),
            zone: zone.into(),
            label: String::new(),
            icon: String::new(),
            enabled: true,
            checked: None,
            kind: Kind::Separator,
            slot: None,
        });
    }
    fn invoke(&mut self, id: &str, zone: &str, icon: &str, label: &str, enabled: bool, action: InvAction) {
        self.add(id, zone, icon, label, enabled, Command::Invoke(action));
    }
    fn slot(&mut self, id: &str, slot: usize) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.id == id) {
            e.slot = Some(slot);
        }
    }
}

pub(super) fn scope(_inv: &Inventory, _id: Uuid) -> String {
    super::menu_layout::SHARED.into()
}

fn creation(c: &mut Catalog, zone: &str, parent: Uuid, enabled: bool) {
    c.add(
        "new-folder",
        zone,
        "folder-filled",
        aurora_i18n::tr!("common-new-folder"),
        enabled,
        Command::Folder(parent),
    );
    for (key, kind, icon, label) in [
        ("new-script", NewItem::Script, "code", aurora_i18n::tr!("common-new-script")),
        (
            "new-note",
            NewItem::Note,
            "file-text-filled",
            aurora_i18n::tr!("common-new-notecard"),
        ),
        (
            "new-gesture",
            NewItem::Gesture,
            "hand-move",
            aurora_i18n::tr!("ui-inventory-context-new-gesture"),
        ),
        (
            "new-material",
            NewItem::Material,
            "droplet-filled",
            aurora_i18n::tr!("ui-inventory-context-new-material"),
        ),
    ] {
        c.invoke(
            key,
            zone,
            icon,
            label,
            enabled,
            InvAction::Create {
                parent,
                kind,
                name: label.into(),
            },
        );
    }
    let clothes = c.group(
        "new-clothes",
        zone,
        "shirt-filled",
        aurora_i18n::tr!("ui-inventory-context-new-clothes"),
        enabled,
        Direction::Horizontal,
        false,
    );
    for (kind, label) in [
        (4, aurora_i18n::tr!("common-new-shirt")),
        (5, aurora_i18n::tr!("ui-inventory-context-new-pants")),
        (6, aurora_i18n::tr!("common-new-shoes")),
        (7, aurora_i18n::tr!("common-new-socks")),
        (8, aurora_i18n::tr!("common-new-jacket")),
        (12, aurora_i18n::tr!("common-new-skirt")),
        (9, aurora_i18n::tr!("common-new-gloves")),
        (10, aurora_i18n::tr!("ui-inventory-context-new-tank-top")),
        (11, aurora_i18n::tr!("ui-inventory-context-new-underpants")),
        (13, aurora_i18n::tr!("ui-inventory-context-new-alpha-mask")),
        (14, aurora_i18n::tr!("common-new-tattoo")),
        (16, aurora_i18n::tr!("ui-inventory-context-new-universal")),
        (15, aurora_i18n::tr!("ui-inventory-context-new-physics-properties")),
    ] {
        c.add(
            format!("create-wearable:{kind}"),
            &clothes,
            "shirt-filled",
            label,
            enabled,
            Command::Invoke(InvAction::Create {
                parent,
                kind: NewItem::Wearable(kind),
                name: label.into(),
            }),
        );
    }
    let body = c.group(
        "new-body",
        zone,
        "man-filled",
        aurora_i18n::tr!("ui-inventory-context-new-body-parts"),
        enabled,
        Direction::Horizontal,
        false,
    );
    for (kind, label) in [
        (0, aurora_i18n::tr!("common-new-shape")),
        (1, aurora_i18n::tr!("common-new-skin")),
        (2, aurora_i18n::tr!("common-new-hair")),
        (3, aurora_i18n::tr!("common-new-eyes")),
    ] {
        c.add(
            format!("create-wearable:{kind}"),
            &body,
            "man-filled",
            label,
            enabled,
            Command::Invoke(InvAction::Create {
                parent,
                kind: NewItem::Wearable(kind),
                name: label.into(),
            }),
        );
    }
    let settings = c.group(
        "new-settings",
        zone,
        "sun-filled",
        aurora_i18n::tr!("ui-inventory-context-new-settings"),
        enabled,
        Direction::Horizontal,
        false,
    );
    for (kind, label) in [
        (0, aurora_i18n::tr!("ui-inventory-context-new-sky")),
        (1, aurora_i18n::tr!("ui-inventory-context-new-water")),
        (2, aurora_i18n::tr!("ui-inventory-context-new-day-cycle")),
    ] {
        c.add(
            format!("create-settings:{kind}"),
            &settings,
            "sun-filled",
            label,
            enabled,
            Command::Invoke(InvAction::Create {
                parent,
                kind: NewItem::Settings(kind),
                name: label.into(),
            }),
        );
    }
}

pub(super) fn lock_entry() -> Entry {
    Entry {
        id: LOCK.into(),
        zone: "bottom".into(),
        label: aurora_i18n::tr!("inventory-menu-lock-label").into(),
        icon: "lock-filled".into(),
        enabled: true,
        checked: None,
        kind: Kind::Lock,
        slot: Some(0),
    }
}

pub(super) fn build(inv: &Inventory, id: Uuid, st: &InventoryUi, prefs: &InventoryPreferences, facts: &Facts) -> Catalog {
    build_mode(inv, id, st, prefs, facts, false)
}

/// Reuse the native definitions, independent of inventory contents and selection.
/// The cached metadata has no command bindings; attachment names stay live.
pub(super) fn editing_entries(live: &[Entry], facts: &Facts) -> Vec<Entry> {
    static ALL: aurora_i18n::Localized<Vec<Entry>> = aurora_i18n::Localized::new();
    let mut entries = ALL
        .get(|| {
            build_mode(
                &Inventory::default(),
                Uuid::nil(),
                &InventoryUi::default(),
                &InventoryPreferences::default(),
                &Facts::default(),
                true,
            )
            .entries
        })
        .clone();
    for (point, hud, name) in &facts.points {
        entries.push(Entry {
            id: format!("attach:{point}"),
            zone: children(if *hud { "attach-hud" } else { "attach-body" }),
            label: if *hud { name.clone() } else { format!("{name} ({point})") },
            icon: "man-filled".into(),
            enabled: false,
            checked: None,
            kind: Kind::Action(usize::MAX),
            slot: None,
        });
    }
    for entry in &mut entries {
        if let Some(current) = live.iter().find(|current| current.id == entry.id) {
            entry.enabled = current.enabled;
            entry.checked = current.checked;
            if matches!(entry.kind, Kind::Action(_)) && matches!(current.kind, Kind::Action(_)) {
                entry.kind = current.kind.clone();
            }
        }
    }
    for entry in live {
        if !entries.iter().any(|known| known.id == entry.id) {
            entries.push(entry.clone());
        }
    }
    entries
}

fn build_mode(inv: &Inventory, id: Uuid, st: &InventoryUi, prefs: &InventoryPreferences, facts: &Facts, all: bool) -> Catalog {
    let mut c = Catalog {
        editing: all,
        ..Default::default()
    };
    c.entries.push(lock_entry());
    let folder = inv.folders.get(&id);
    let item = inv.items.get(&id);
    let target = rules::original(inv, id);
    let original = target.and_then(|id| inv.items.get(&id));
    let kind = original.or(item).map_or(-1, |it| it.asset_type);
    let ids = st.selected_ids(id);
    let single = ids.len() == 1;
    let ready = st.pending.is_none();
    let writable = rules::mutable(inv, id, &prefs.protected);
    let movable = ids.iter().all(|id| rules::movable(inv, *id, &prefs.protected));
    let worn = target.is_some_and(|id| facts.worn.contains(&id));
    let trash = rules::in_type(inv, id, 14);
    let linked = item.is_some_and(|it| matches!(it.asset_type, 24 | 25));
    let to = folder.map(|_| id).or_else(|| rules::parent(inv, id));
    let trash_root = folder.is_some_and(|f| f.info.type_default == 14);
    let can_create = ready
        && single
        && !trash
        && to.is_some_and(|to| {
            rules::destination(inv, to, &prefs.protected) && inv.folders.get(&to).is_some_and(|f| f.info.type_default != 16)
        });
    let new = c.group(
        "new",
        "top",
        "plus",
        aurora_i18n::tr!("inventory-menu-new"),
        can_create,
        Direction::Horizontal,
        false,
    );
    c.slot("new", 0);
    creation(&mut c, &new, to.unwrap_or_default(), can_create);
    if trash_root && !all {
        c.add(
            "empty-trash",
            TEXT,
            "trash-filled",
            aurora_i18n::tr!("common-empty-trash"),
            ready && folder.is_some_and(|f| f.state == FetchState::Fetched && (!f.items.is_empty() || !f.children.is_empty())),
            Command::Empty(id),
        );
        c.add(
            "image",
            TEXT,
            "photo-filled",
            aurora_i18n::tr!("common-image-2"),
            ready && single && writable,
            Command::Image(id),
        );
        c.divider("folder-navigation", TEXT);
        navigation(&mut c, inv, id, single);
        extra::build(&mut c, inv, id, st, prefs, facts, all);
        return c;
    }
    let copyable = clipboard::copy_allowed(inv, &ids);
    c.add(
        "copy",
        "top",
        "copy-filled",
        aurora_i18n::tr!("common-copy-2"),
        ready && copyable,
        Command::Copy(ids.clone(), false),
    );
    c.add(
        "cut",
        "top",
        "cut",
        aurora_i18n::tr!("common-cut"),
        ready && movable,
        Command::Copy(ids.clone(), true),
    );
    let paste =
        ready && single && to.is_some_and(|to| rules::paste_allowed(inv, &st.clipboard, to, st.cut, false, &prefs.protected, &facts.worn));
    c.add(
        "paste",
        "top",
        "clipboard-text-filled",
        aurora_i18n::tr!("common-paste"),
        paste,
        Command::Paste(to.unwrap_or_default(), false),
    );
    if !trash || all {
        let favorites_ready = ready
            && ids
                .iter()
                .all(|id| rules::original(inv, *id).is_some_and(|target| !rules::library(inv, target) && target != inv.root));
        let changes: Vec<_> = [true, false]
            .into_iter()
            .filter(|favorite| all || ids.iter().any(|id| rules::is_favorite(inv, *id) != *favorite))
            .collect();
        let favorite_zone = if changes.len() > 1 {
            c.group(
                "favorite",
                "top",
                "star-filled",
                aurora_i18n::tr!("inventory-menu-favorites"),
                favorites_ready,
                Direction::Horizontal,
                false,
            )
        } else {
            "top".into()
        };
        for favorite in changes {
            let key = if favorite_zone == "top" {
                "favorite"
            } else if favorite {
                "favorite-add"
            } else {
                "favorite-remove"
            };
            // The star shows the state the row leaves: empty to add, full to remove.
            c.add(
                key,
                &favorite_zone,
                if favorite { "star" } else { "star-filled" },
                if favorite {
                    aurora_i18n::tr!("common-add-to-favorites")
                } else {
                    aurora_i18n::tr!("common-remove-from-favorites")
                },
                favorites_ready,
                Command::Favorite(ids.clone(), favorite),
            );
            if let Some(last) = c.entries.last_mut() {
                last.checked = Some(!favorite);
            }
        }
    }
    if trash && !all {
        c.add(
            "favorite",
            "top",
            "star-filled",
            aurora_i18n::tr!("inventory-menu-favorites"),
            false,
            Command::Favorite(ids.clone(), true),
        );
    }
    for (key, slot) in [("copy", 4), ("cut", 5), ("paste", 6), ("favorite", 7)] {
        c.slot(key, slot);
    }
    if trash || all {
        c.add(
            "restore",
            TEXT,
            "restore",
            aurora_i18n::tr!("ui-inventory-context-restore-object"),
            ready && movable,
            Command::DefaultFolder(ids.clone()),
        );
    }
    if !trash || all {
        if linked || all {
            c.add(
                "original",
                TEXT,
                "external-link-filled",
                aurora_i18n::tr!("ui-inventory-context-find-original"),
                single && target.is_some(),
                Command::Original(target.unwrap_or(id)),
            );
        }
        if folder.is_none() || all {
            let opens = !ids.is_empty()
                && ids.iter().all(|id| {
                    rules::original(inv, *id)
                        .and_then(|id| inv.items.get(&id))
                        .is_some_and(|item| matches!(item.asset_type, 0 | 1 | 2 | 3 | 7 | 10 | 20 | 21 | 56 | 57))
                });
            if opens || !matches!(kind, 5 | 6 | 13) || all {
                c.add(
                    "open",
                    TEXT,
                    if kind == 3 { "location-filled" } else { "folder-open-filled" },
                    if all {
                        aurora_i18n::tr!("inventory-menu-editor-open")
                    } else if kind == 3 {
                        aurora_i18n::tr!("common-teleport")
                    } else {
                        aurora_i18n::tr!("common-open")
                    },
                    opens,
                    if single {
                        Command::Open(id)
                    } else {
                        Command::OpenMany(ids.clone())
                    },
                );
            }
            let placeholder = aurora_net::inventory::InvItem::default();
            if let Some(landmark) = original.filter(|it| it.asset_type == 3).or_else(|| all.then_some(&placeholder)) {
                c.invoke(
                    "about-landmark",
                    TEXT,
                    "info-circle-filled",
                    aurora_i18n::tr!("ui-inventory-context-about-landmark"),
                    single && !landmark.asset_id.is_nil(),
                    InvAction::AboutLandmark(landmark.id, landmark.asset_id),
                );
                c.invoke(
                    "copy-landmark-slurl",
                    TEXT,
                    "link-filled",
                    aurora_i18n::tr!("common-copy-slurl"),
                    single && !landmark.asset_id.is_nil(),
                    InvAction::CopyLandmarkSlurl(landmark.asset_id),
                );
                c.invoke(
                    "show-landmark-on-map",
                    TEXT,
                    "map",
                    aurora_i18n::tr!("common-show-on-map"),
                    single && !landmark.asset_id.is_nil(),
                    InvAction::ShowLandmarkOnMap(landmark.asset_id),
                );
            }
            if (kind == 6 && !worn) || all {
                c.invoke(
                    "rez-previous",
                    TEXT,
                    "restore",
                    aurora_i18n::tr!("ui-inventory-context-return-to-last-location"),
                    writable
                        && ids.iter().all(|id| {
                            rules::original(inv, *id)
                                .and_then(|id| inv.items.get(&id))
                                .is_some_and(|item| item.asset_type == 6 && !facts.worn.contains(&item.id))
                        }),
                    InvAction::Restore(target.unwrap_or(id)),
                );
            }
            if let Some(it) = original.or_else(|| all.then_some(&placeholder)) {
                if kind == 20 || all {
                    c.invoke(
                        "play-world",
                        TEXT,
                        "player-play-filled",
                        aurora_i18n::tr!("ui-inventory-context-play-in-second-life"),
                        single && !it.asset_id.is_nil(),
                        InvAction::Animation {
                            asset: it.asset_id,
                            local: false,
                            start: true,
                        },
                    );
                    c.invoke(
                        "play-local",
                        TEXT,
                        "player-play-filled",
                        aurora_i18n::tr!("common-play-locally"),
                        single && !it.asset_id.is_nil(),
                        InvAction::Animation {
                            asset: it.asset_id,
                            local: true,
                            start: true,
                        },
                    );
                }
                if kind == 1 || all {
                    c.invoke(
                        "play-sound",
                        TEXT,
                        "volume",
                        aurora_i18n::tr!("common-play"),
                        single && !it.asset_id.is_nil(),
                        InvAction::Sound(it.asset_id),
                    );
                }
                if kind == 56 || all {
                    c.invoke(
                        "apply-environment",
                        TEXT,
                        "sun-filled",
                        aurora_i18n::tr!("common-apply-to-myself-only"),
                        single && !it.asset_id.is_nil(),
                        InvAction::Environment(it.asset_id, it.flags),
                    );
                }
            }
            let selected: Vec<_> = ids.iter().filter_map(|id| rules::original(inv, *id)).collect();
            if kind == 21 || all {
                let active = selected.iter().all(|id| facts.active_gestures.contains(id));
                c.invoke(
                    "gesture-active",
                    TEXT,
                    "check",
                    if all {
                        aurora_i18n::tr!("inventory-menu-editor-gesture")
                    } else if active {
                        aurora_i18n::tr!("gesture-deactivate")
                    } else {
                        aurora_i18n::tr!("gesture-activate")
                    },
                    ready && !selected.is_empty() && selected.iter().all(|id| !rules::library(inv, *id)),
                    InvAction::Gesture(crate::ui::gestures::Action::Active(selected.clone(), !active)),
                );
                c.invoke(
                    "gesture-play",
                    TEXT,
                    "player-play-filled",
                    aurora_i18n::tr!("common-play"),
                    single && ready,
                    InvAction::Gesture(crate::ui::gestures::Action::Play(target.unwrap_or(id))),
                );
            }
            if all || (single && original.is_some_and(|it| (matches!(it.asset_type, 5 | 13)) && facts.worn.contains(&it.id))) {
                c.add(
                    "edit-worn",
                    TEXT,
                    "pencil-filled",
                    aurora_i18n::tr!("common-edit"),
                    original.is_some_and(|item| item.owner_mask & rules::MODIFY != 0),
                    Command::Worn(crate::ui::context::CtxAction::EditWorn(target.unwrap_or(id))),
                );
            }
            // Resource types and unresolved links have no attachment controls.
            if (target.is_some() && matches!(kind, 5 | 6 | 13)) || all {
                item_attachments(&mut c, inv, target.unwrap_or(id), &selected, &ids, kind, ready, single, facts);
            }
        }
        if folder.is_some() || all {
            let content = crate::world::appearance::wearable_contents(inv, id);
            if !all && single && content.is_err() && !rules::library(inv, id) {
                c.prepare_outfit = Some(id);
            }
            let content = content.unwrap_or_default();
            if !content.is_empty() || c.prepare_outfit.is_some() || all {
                let zone = c.group(
                    "attachments",
                    TEXT,
                    "hanger-2-filled",
                    aurora_i18n::tr!("inventory-menu-attachments"),
                    true,
                    Direction::Vertical,
                    true,
                );
                let can_wear = single && !facts.appearance_busy && !content.is_empty() && !rules::library(inv, id);
                for (key, icon, label, mode) in [
                    (
                        "replace-outfit",
                        "shirt-filled",
                        aurora_i18n::tr!("ui-inventory-context-replace-current-outfit"),
                        crate::world::appearance::WearMode::ReplaceOutfit,
                    ),
                    (
                        "append-outfit",
                        "plus",
                        aurora_i18n::tr!("ui-inventory-context-add-to-current-outfit"),
                        crate::world::appearance::WearMode::Append,
                    ),
                    (
                        "wear-contents",
                        "hanger-2-filled",
                        aurora_i18n::tr!("ui-inventory-context-wear-objects"),
                        crate::world::appearance::WearMode::ReplaceItems,
                    ),
                ] {
                    c.invoke(
                        key,
                        &zone,
                        icon,
                        label,
                        ready && can_wear,
                        InvAction::Appearance(Wear::WearContents { folder: id, mode }),
                    );
                }
                c.invoke(
                    "remove-outfit",
                    &zone,
                    "circle-x-filled",
                    aurora_i18n::tr!("ui-inventory-context-remove-from-current-outfit"),
                    ready && can_wear && content.iter().any(|l| facts.worn.contains(&l.target)),
                    InvAction::Appearance(Wear::RemoveOutfit(id)),
                );
            }
        }
        c.divider("metadata", TEXT);
        if folder.is_none() || all {
            c.add(
                "properties",
                TEXT,
                "info-circle-filled",
                aurora_i18n::tr!("common-properties"),
                !ids.is_empty(),
                if single {
                    Command::Properties(id)
                } else {
                    Command::PropertiesMany(ids.clone())
                },
            );
        }
        if all || !single || folder.is_some() {
            c.add(
                "bulk-permissions",
                TEXT,
                "lock-filled",
                aurora_i18n::tr!("inventory-bulk-permissions"),
                ready && ids.iter().any(|id| bulk_permissions::eligible(inv, *id, prefs, facts.agent)),
                Command::BulkPermissions(ids.clone()),
            );
        }
        c.add(
            "rename",
            TEXT,
            "pencil-filled",
            aurora_i18n::tr!("common-rename"),
            ready && single && rules::renameable(inv, id, &prefs.protected),
            Command::Rename(id),
        );
        let share = ready && writable && ids.iter().all(|id| rules::offer(inv, *id, &prefs.protected, &facts.worn).is_ok());
        c.add(
            "share",
            TEXT,
            "share",
            aurora_i18n::tr!("common-share"),
            share,
            Command::Share(ids.clone()),
        );
        if all || folder.is_some_and(|f| matches!(f.info.type_default, -1 | 47 | 54) && !f.library) {
            c.add(
                "protect-folder",
                TEXT,
                "lock-filled",
                if all {
                    aurora_i18n::tr!("inventory-menu-editor-protect")
                } else if prefs.protected.contains(&id) {
                    aurora_i18n::tr!("ui-inventory-context-unlock")
                } else {
                    aurora_i18n::tr!("ui-inventory-context-lock")
                },
                true,
                Command::Protect(id),
            );
        }
    }
    let organize = c.group(
        "organize",
        TEXT,
        "folder-filled",
        aurora_i18n::tr!("inventory-menu-organize"),
        true,
        Direction::Vertical,
        false,
    );
    c.add(
        "move",
        &organize,
        "folder-filled",
        aurora_i18n::tr!("inventory-move-to"),
        ready && movable,
        Command::Move(id),
    );
    if !trash || all {
        if folder.is_none() || all {
            c.add(
                "default-folder",
                &organize,
                "folder-filled",
                aurora_i18n::tr!("ui-inventory-context-move-to-default-folder"),
                ready && movable,
                Command::DefaultFolder(ids.clone()),
            );
        }
        c.add(
            "group-selection",
            &organize,
            "folder-filled",
            aurora_i18n::tr!("ui-inventory-context-create-folder-from-selection"),
            ready && movable,
            Command::Group(ids.clone()),
        );
        if folder.is_some() || all {
            c.add(
                "ungroup",
                &organize,
                "arrows-maximize",
                aurora_i18n::tr!("ui-inventory-context-ungroup-folder-items"),
                ready
                    && movable
                    && single
                    && folder.is_some_and(|f| {
                        f.state == FetchState::Fetched && (!f.items.is_empty() || !f.children.is_empty()) && f.info.type_default != 47
                    }),
                Command::Ungroup(id),
            );
        }
    }
    c.divider("links", &organize);
    let paste_links = ready
        && single
        && !st.cut
        && to.is_some_and(|to| rules::paste_allowed(inv, &st.clipboard, to, false, true, &prefs.protected, &facts.worn));
    c.add(
        "paste-link",
        &organize,
        "link-filled",
        aurora_i18n::tr!("ui-inventory-context-paste-as-link"),
        paste_links,
        Command::Paste(to.unwrap_or_default(), true),
    );
    if !linked || all {
        c.add(
            "find-links",
            &organize,
            "link-filled",
            aurora_i18n::tr!("ui-inventory-context-find-all-links"),
            single,
            Command::Links(id),
        );
    }
    if item.is_some() || all {
        c.add(
            "replace-links",
            &organize,
            "link-filled",
            aurora_i18n::tr!("common-replace-links"),
            single && ready && target.is_some() && !rules::library(inv, id),
            Command::Replace(target.unwrap_or(id)),
        );
    }
    let placeholder = aurora_net::inventory::InvItem::default();
    if let Some(it) = item
        .filter(|it| !trash && super::context::exposes_asset_uuid(it.asset_type))
        .or_else(|| all.then_some(&placeholder))
    {
        let asset = super::context::copyable_asset_uuid(it);
        c.add(
            "asset-uuid",
            &organize,
            "copy-filled",
            aurora_i18n::tr!("ui-inventory-context-copy-object-uuid"),
            single && asset.is_some(),
            Command::Asset(asset.unwrap_or_default()),
        );
    }
    if !trash || all {
        c.divider("marketplace", &organize);
        for (copy, key, label) in [
            (
                true,
                "marketplace-copy",
                aurora_i18n::tr!("ui-inventory-context-copy-to-marketplace-listings"),
            ),
            (
                false,
                "marketplace-move",
                aurora_i18n::tr!("ui-inventory-context-move-to-marketplace-listings"),
            ),
        ] {
            c.add(
                key,
                &organize,
                "tag-filled",
                label,
                ready && st.merchant && rules::marketplace(inv, &ids, copy, &prefs.protected, &facts.worn).is_ok(),
                Command::Marketplace(ids.clone(), copy),
            );
        }
        if folder.is_some() || all {
            c.divider("uploads", &organize);
            let upload = c.group(
                "upload-folder",
                &organize,
                "file-upload-filled",
                aurora_i18n::tr!("ui-inventory-context-use-as-default-folder-for"),
                can_create,
                Direction::Horizontal,
                false,
            );
            for (slot, label) in [
                aurora_i18n::tr!("ui-inventory-context-image-loads"),
                aurora_i18n::tr!("ui-inventory-context-sound-loads"),
                aurora_i18n::tr!("ui-inventory-context-animation-loads"),
                aurora_i18n::tr!("ui-inventory-context-model-loads"),
                aurora_i18n::tr!("ui-inventory-context-pbr-material-loads"),
            ]
            .iter()
            .enumerate()
            {
                c.add(
                    format!("upload:{slot}"),
                    &upload,
                    "file-upload-filled",
                    label,
                    can_create,
                    Command::Upload(slot, id),
                );
                if let Some(e) = c.entries.last_mut() {
                    e.checked = Some(prefs.upload_folders[slot] == id);
                }
            }
        }
        c.divider("image", TEXT);
        c.add(
            "image",
            TEXT,
            "photo-filled",
            aurora_i18n::tr!("common-image-2"),
            ready && single && writable && !linked,
            Command::Image(id),
        );
    }
    if folder.is_some() || all {
        c.divider("folder-navigation", TEXT);
        navigation(&mut c, inv, id, single);
    }
    if trash || all {
        c.divider("purge", TEXT);
        c.add(
            "purge",
            TEXT,
            "trash-filled",
            aurora_i18n::tr!("ui-inventory-context-purge-object"),
            ready && super::delete::allowed(inv, &ids, prefs, facts),
            Command::Delete(ids.clone(), true),
        );
    }
    if all {
        c.add(
            "empty-trash",
            TEXT,
            "trash-filled",
            aurora_i18n::tr!("common-empty-trash"),
            false,
            Command::Empty(id),
        );
    }
    if !trash || all {
        c.add(
            "delete",
            "bottom",
            "trash-filled",
            aurora_i18n::tr!("common-delete"),
            ready && super::delete::allowed(inv, &ids, prefs, facts),
            Command::Delete(ids, false),
        );
        c.slot("delete", 7);
    }
    extra::build(&mut c, inv, id, st, prefs, facts, all);
    if !all {
        super::listings::restrict(&mut c, inv, id, st);
    }
    c
}

fn navigation(c: &mut Catalog, inv: &Inventory, id: Uuid, single: bool) {
    c.add(
        "reload",
        TEXT,
        "refresh",
        aurora_i18n::tr!("ui-inventory-context-reload-folder"),
        single && id != inv.root && id != inv.lib_root,
        Command::Reload(id),
    );
    c.add(
        "show-window",
        TEXT,
        "external-link-filled",
        aurora_i18n::tr!("ui-inventory-context-show-in-a-new-window"),
        single,
        Command::Window(id),
    );
    c.add(
        "open-window",
        TEXT,
        "folder-open-filled",
        aurora_i18n::tr!("ui-inventory-context-open-in-a-new-window"),
        single,
        Command::Window(id),
    );
}

#[allow(clippy::too_many_arguments)]
fn item_attachments(
    c: &mut Catalog,
    inv: &Inventory,
    target: Uuid,
    selected: &[Uuid],
    ids: &[Uuid],
    kind: i32,
    ready: bool,
    single: bool,
    facts: &Facts,
) {
    let objects = !selected.is_empty() && selected.iter().all(|id| inv.items.get(id).is_some_and(|it| it.asset_type == 6));
    let worn_objects = objects && selected.iter().all(|id| facts.worn.contains(id));
    if worn_objects || c.editing {
        c.add(
            "touch-worn",
            TEXT,
            "click-filled",
            aurora_i18n::tr!("ui-context-touch"),
            single && facts.touchable.contains(&target),
            Command::Worn(crate::ui::context::CtxAction::TouchWorn(target)),
        );
        c.add(
            "edit-worn",
            TEXT,
            "pencil-filled",
            aurora_i18n::tr!("common-edit"),
            single,
            Command::Worn(crate::ui::context::CtxAction::EditWorn(target)),
        );
    }
    let zone = c.group(
        "attachments",
        TEXT,
        "hanger-2-filled",
        aurora_i18n::tr!("inventory-menu-attachments"),
        true,
        Direction::Vertical,
        true,
    );
    let wear = ready
        && !facts.appearance_busy
        && selected.len() == ids.len()
        && selected.iter().all(|id| {
            !facts.worn.contains(id)
                && !rules::library(inv, *id)
                && !rules::in_type(inv, *id, 14)
                && inv.items.get(id).is_some_and(|it| matches!(it.asset_type, 5 | 6 | 13))
        });
    if !worn_objects || c.editing {
        c.invoke(
            "wear",
            &zone,
            "shirt-filled",
            aurora_i18n::tr!("common-wear"),
            wear && single,
            InvAction::Appearance(Wear::WearItem {
                item: target,
                replace: true,
                point: 0,
            }),
        );
        c.invoke(
            "add",
            &zone,
            "plus",
            aurora_i18n::tr!("common-add"),
            wear && selected.iter().all(|id| inv.items.get(id).is_some_and(|it| it.asset_type != 13)),
            InvAction::Appearance(Wear::WearItems {
                items: selected.to_vec(),
                replace: false,
                point: 0,
            }),
        );
        if kind == 6 || c.editing {
            for hud in [false, true] {
                let attach = c.group(
                    if hud { "attach-hud" } else { "attach-body" },
                    &zone,
                    if hud { "device-desktop-filled" } else { "man-filled" },
                    if hud {
                        aurora_i18n::tr!("common-attach-to-hud")
                    } else {
                        aurora_i18n::tr!("common-attach-to")
                    },
                    wear && objects,
                    Direction::Horizontal,
                    false,
                );
                for (point, is_hud, name) in facts.points.iter().filter(|(_, is_hud, _)| *is_hud == hud) {
                    let label = if *is_hud { name.clone() } else { format!("{name} ({point})") };
                    c.add(
                        format!("attach:{point}"),
                        &attach,
                        "man-filled",
                        &label,
                        wear && objects,
                        Command::Invoke(InvAction::Appearance(Wear::WearItems {
                            items: selected.to_vec(),
                            replace: false,
                            point: *point,
                        })),
                    );
                }
            }
        }
    }
    if selected.iter().any(|id| facts.worn.contains(id)) || c.editing {
        c.invoke(
            "detach",
            &zone,
            "circle-x-filled",
            if c.editing {
                aurora_i18n::tr!("inventory-menu-editor-detach")
            } else if kind == 6 {
                aurora_i18n::tr!("ui-inventory-context-detach-from-yourself")
            } else {
                aurora_i18n::tr!("common-remove")
            },
            ready
                && !facts.appearance_busy
                && selected.len() == ids.len()
                && selected
                    .iter()
                    .all(|id| facts.worn.contains(id) && inv.items.get(id).is_some_and(|it| matches!(it.asset_type, 5 | 6))),
            InvAction::Appearance(Wear::RemoveItems(selected.to_vec())),
        );
    }
}

pub(super) fn execute(
    command: Command,
    ui: &egui::Ui,
    inv: &Inventory,
    st: &mut InventoryUi,
    prefs: &mut InventoryPreferences,
    facts: &Facts,
    actions: &mut Vec<InvAction>,
) {
    let mut submit = |plan: Result<Mutation, String>| match plan {
        Ok(m) => actions.push(InvAction::Edit(m)),
        Err(reason) => {
            st.message = reason;
            st.consume_clipboard = false;
        }
    };
    match command {
        Command::Extra(command) => extra::execute(command, ui, inv, st, prefs, facts, actions),
        Command::Invoke(action) => actions.push(action),
        Command::Folder(id) => submit(rules::create_folder(id, aurora_i18n::tr!("common-new-folder"))),
        Command::Upload(slot, id) => prefs.upload_folders[slot] = id,
        Command::Empty(id) => st.dialog = Some(EditDialog::Empty(id)),
        Command::DefaultFolder(ids) => {
            let plan = ids.iter().try_fold(Mutation::default(), |mut plan, id| {
                let to = rules::default_folder(inv, *id).ok_or_else(|| aurora_i18n::tr!("common-folder-not-found").to_owned())?;
                let change = rules::move_selection(inv, &[*id], to, &prefs.protected, &facts.worn)?;
                plan.operations.extend(change.operations);
                plan.refresh.extend(change.refresh);
                Ok::<_, String>(plan)
            });
            submit(plan);
        }
        Command::Delete(ids, purge) => st.dialog = Some(EditDialog::Delete(ids, purge)),
        Command::Share(ids) => {
            st.share_items = ids;
            st.share_picker.open(&format!("inventory_share_{}", st.ui_id), false);
        }
        Command::Open(id) => super::context::open(inv, id, st, actions),
        Command::OpenMany(ids) => {
            for id in ids {
                super::context::open(inv, id, st, actions);
            }
        }
        Command::Batch(batch) => actions.extend(batch),
        Command::PropertiesMany(ids) => {
            for id in ids {
                if !st
                    .windows
                    .iter()
                    .any(|window| matches!(window.state.dialog, Some(EditDialog::Properties(item)) if item == id))
                {
                    let state = InventoryUi {
                        ui_id: Uuid::new_v4(),
                        dialog: Some(EditDialog::Properties(id)),
                        ..Default::default()
                    };
                    st.windows.push(InventoryWindow {
                        id: state.ui_id,
                        root: Uuid::nil(),
                        additional: false,
                        state: Box::new(state),
                        open: false,
                    });
                }
            }
        }
        Command::Properties(id) => {
            st.property_edit = None;
            st.dialog = Some(EditDialog::Properties(id));
        }
        Command::BulkPermissions(ids) => st.open_bulk_permissions(ids),
        Command::Rename(id) => st.begin_rename(inv, id),
        Command::Image(id) => st.thumbnail.open(id),
        Command::Protect(id) => {
            if !prefs.protected.remove(&id) {
                prefs.protected.insert(id);
            }
        }
        Command::Reload(id) => {
            st.message.clear();
            actions.push(InvAction::Preview(id));
        }
        Command::Window(id) => st.open_folder_window(id),
        Command::Asset(id) => ui.ctx().copy_text(id.to_string()),
        Command::Copy(ids, cut) => clipboard::store(ui.ctx(), st, ids, cut),
        Command::Move(id) => st.begin_move_to(id),
        Command::Paste(id, link) => clipboard::paste(inv, id, link, st, prefs, facts, actions),
        Command::Original(id) => st.show_original(inv, id),
        Command::Links(id) if st.ui_id == super::listings::PANEL => actions.push(InvAction::FindLinks(id)),
        Command::Links(id) => {
            st.find_links(id);
        }
        Command::Replace(id) => st.begin_replace_links(id),
        Command::Group(ids) => submit(rules::group(inv, &ids, &prefs.protected, &facts.worn)),
        Command::Ungroup(id) => st.dialog = Some(EditDialog::Ungroup(id)),
        Command::Favorite(ids, favorite) => submit(if ids.len() == 1 {
            rules::toggle_favorite(inv, ids[0])
        } else {
            rules::set_favorites(inv, &ids, favorite)
        }),
        Command::Worn(action) => crate::ui::context::request(ui.ctx(), action),
        Command::Marketplace(ids, copy) => submit(rules::marketplace(inv, &ids, copy, &prefs.protected, &facts.worn)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    pub(super) fn facts() -> Facts {
        Facts {
            agent: Uuid::nil(),
            worn: HashSet::new(),
            worn_labels: Default::default(),
            worn_at: Default::default(),
            points: vec![(1, false, "Chest".into()), (31, true, "Center".into())],
            appearance_busy: false,
            active_gestures: Default::default(),
            touchable: Default::default(),
            names: Default::default(),
            ..Default::default()
        }
    }
    fn inventory() -> Inventory {
        let mut inv = Inventory::default();
        crate::world::inventory::demo::seed(&mut inv, Uuid::from_u128(2));
        inv
    }
    #[test]
    fn landmark_destination_actions_use_link_targets_and_require_one_available_asset() {
        let mut inv = inventory();
        let item = Uuid::from_u128(8103);
        let asset = Uuid::from_u128(4242);
        let landmark = inv.items.get_mut(&item).expect("demo item");
        landmark.asset_type = 3;
        landmark.inv_type = 3;
        landmark.asset_id = asset;
        landmark.owner_mask = 0;
        let link = Uuid::from_u128(8113);
        inv.items.get_mut(&link).expect("demo link").asset_id = item;
        for target in [item, link] {
            for multiple in [false, true] {
                let mut st = InventoryUi::default();
                if multiple {
                    st.selection.extend([target, Uuid::from_u128(8100)]);
                }
                let c = build(&inv, target, &st, &InventoryPreferences::default(), &facts());
                for (id, copy) in [("copy-landmark-slurl", true), ("show-landmark-on-map", false)] {
                    let entry = c.entries.iter().find(|e| e.id == id).expect("destination action");
                    assert_eq!(entry.enabled, !multiple);
                    let Kind::Action(index) = entry.kind else { panic!("action") };
                    assert!(
                        matches!(&c.commands[index], Some(Command::Invoke(InvAction::CopyLandmarkSlurl(a))) if copy && *a == asset)
                            || matches!(&c.commands[index], Some(Command::Invoke(InvAction::ShowLandmarkOnMap(a))) if !copy && *a == asset)
                    );
                }
            }
        }
        inv.items.get_mut(&item).expect("landmark").asset_id = Uuid::nil();
        let c = build(&inv, item, &InventoryUi::default(), &InventoryPreferences::default(), &facts());
        assert!(
            c.entries
                .iter()
                .filter(|e| matches!(e.id.as_str(), "copy-landmark-slurl" | "show-landmark-on-map"))
                .all(|e| !e.enabled)
        );
        let c = build(
            &inv,
            Uuid::from_u128(8100),
            &InventoryUi::default(),
            &InventoryPreferences::default(),
            &facts(),
        );
        assert!(
            !c.entries
                .iter()
                .any(|e| matches!(e.id.as_str(), "copy-landmark-slurl" | "show-landmark-on-map"))
        );
    }
    #[test]
    fn incomplete_folder_menus_keep_native_descendant_preparation_out_of_the_inert_catalogue() {
        let mut inv = inventory();
        let folder = Uuid::from_u128(8000);
        let child = Uuid::from_u128(8001);
        inv.folders.get_mut(&child).unwrap().state = FetchState::Unknown;
        let prefs = InventoryPreferences::default();
        let state = InventoryUi::default();
        assert_eq!(build(&inv, folder, &state, &prefs, &facts()).prepare_outfit, Some(folder));
        assert!(build_mode(&inv, folder, &state, &prefs, &facts(), true).prepare_outfit.is_none());
        let mut multi = InventoryUi::default();
        multi.selection.extend([folder, Uuid::from_u128(8100)]);
        assert!(build(&inv, folder, &multi, &prefs, &facts()).prepare_outfit.is_none());
        inv.folders.get_mut(&child).unwrap().state = FetchState::Fetched;
        // A received folder response advances the generation used by caches.
        inv.generation += 1;
        assert!(build(&inv, folder, &state, &prefs, &facts()).prepare_outfit.is_none());
        inv.folders.get_mut(&child).unwrap().state = FetchState::Unknown;
        inv.folders.get_mut(&folder).unwrap().library = true;
        assert!(build(&inv, folder, &state, &prefs, &facts()).prepare_outfit.is_none());
    }

    #[test]
    fn complete_editor_metadata_is_inert_unique_and_covers_native_types_and_selection_states() {
        let mut inv = inventory();
        for (number, kind, flags) in [(8300, 3, 0), (8301, 2, 0), (8302, 13, 15)] {
            let mut item = inv.items[&Uuid::from_u128(8103)].clone();
            item.id = Uuid::from_u128(number);
            item.asset_type = kind;
            item.flags = flags;
            inv.add_items(vec![item]);
        }
        let prefs = InventoryPreferences::default();
        let metadata = build_mode(
            &Inventory::default(),
            Uuid::nil(),
            &InventoryUi::default(),
            &prefs,
            &Facts::default(),
            true,
        );
        assert!(metadata.commands.is_empty());
        assert!(
            metadata
                .entries
                .iter()
                .all(|e| !matches!(e.kind, Kind::Action(i) if i != usize::MAX))
        );
        let all = editing_entries(&[], &facts());
        let ids: BTreeSet<_> = all.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids.len(), all.len());
        for id in [
            "play-local",
            "play-sound",
            "apply-environment",
            "gesture-active",
            "about-landmark",
            "copy-landmark-slurl",
            "show-landmark-on-map",
            "detach",
            "attach:1",
            "attach:31",
            "replace-outfit",
            "upload:4",
            "restore",
            "purge",
            "empty-trash",
            "favorite-add",
            "favorite-remove",
        ] {
            assert!(ids.contains(id), "missing editor option {id}");
        }
        let targets: Vec<_> = inv.items.keys().chain(inv.folders.keys()).copied().collect();
        for worn in [false, true] {
            let mut live_facts = facts();
            if worn {
                live_facts.worn.extend(inv.items.keys().copied());
                live_facts.active_gestures.extend(inv.items.keys().copied());
            }
            for &target in &targets {
                for multi in [false, true] {
                    let mut st = InventoryUi::default();
                    if multi {
                        st.selection.extend([target, Uuid::from_u128(8100)]);
                    }
                    let live = build(&inv, target, &st, &prefs, &live_facts);
                    for entry in &live.entries {
                        assert!(
                            ids.contains(entry.id.as_str()),
                            "missing {} for {target}, worn={worn}, multi={multi}",
                            entry.id
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn unavailable_editor_options_keep_shared_placements_and_only_live_commands_get_bindings() {
        use super::super::menu_layout::{HIDDEN, Layout};
        let inv = inventory();
        let st = InventoryUi::default();
        let facts = facts();
        let prefs = InventoryPreferences::default();
        let texture = build(&inv, Uuid::from_u128(8104), &st, &prefs, &facts);
        let editing = editing_entries(&texture.entries, &facts);
        let disabled = editing.iter().find(|e| e.id == "play-local").unwrap();
        assert!(!disabled.enabled && matches!(disabled.kind, Kind::Action(usize::MAX)));
        for current in texture.entries.iter().filter(|e| matches!(e.kind, Kind::Action(_))) {
            let editor = editing.iter().find(|e| e.id == current.id).unwrap();
            // Favorite has both Add and Remove children in the complete editor.
            if current.id != "favorite" {
                assert!(matches!((&current.kind, &editor.kind), (Kind::Action(a), Kind::Action(b)) if a == b));
            }
        }
        let mut layout = Layout::default();
        assert!(layout.move_entry(&editing, "play-local", "top", None, Some(2)));
        assert!(layout.move_entry(&editing, "detach", HIDDEN, None, None));
        let saved = serde_json::to_string(&layout).unwrap();
        let layout: Layout = serde_json::from_str(&saved).unwrap();
        assert!(
            !layout
                .resolve(&texture.entries)
                .values()
                .flatten()
                .any(|e| e.id == "play-local" || e.id == "detach")
        );
        let animation = build(&inv, Uuid::from_u128(8101), &st, &prefs, &facts);
        let resolved = layout.resolve(&animation.entries);
        assert_eq!(layout.icon_slots("top", &resolved["top"])[2].as_deref(), Some("play-local"));
        let mut worn = facts;
        worn.worn.insert(Uuid::from_u128(8100));
        let object = build(&inv, Uuid::from_u128(8100), &st, &prefs, &worn);
        assert!(layout.resolve(&object.entries)[HIDDEN].iter().any(|e| e.id == "detach"));
    }
    #[test]
    fn every_type_has_consistent_icon_cells_but_only_wearables_have_attachment_controls() {
        let inv = inventory();
        let st = InventoryUi::default();
        let prefs = InventoryPreferences::default();
        let facts = facts();
        for (number, attaches, points) in [
            (8100, true, true),
            (8111, true, false),
            (8112, true, false),
            (8101, false, false),
            (8102, false, false),
            (8103, false, false),
            (8104, false, false),
            (8105, false, false),
            (8106, false, false),
            (8107, false, false),
            (8108, false, false),
            (8000, true, false),
            (8001, false, false),
        ] {
            let c = build(&inv, Uuid::from_u128(number), &st, &prefs, &facts);
            assert_eq!(c.entries.iter().any(|e| e.id == "attachments"), attaches, "{number}");
            assert_eq!(c.entries.iter().any(|e| e.id == "attach-hud"), points, "{number}");
            for (id, slot) in [
                ("new", 0),
                ("copy", 4),
                ("cut", 5),
                ("paste", 6),
                ("favorite", 7),
                (LOCK, 0),
                ("delete", 7),
            ] {
                assert_eq!(c.entries.iter().find(|e| e.id == id).unwrap().slot, Some(slot), "{number}: {id}");
            }
            assert_eq!(c.entries.iter().find(|e| e.id == LOCK).unwrap().zone, "bottom");
            assert_eq!(c.entries.iter().find(|e| e.id == "delete").unwrap().zone, "bottom");
            let cids: BTreeSet<_> = c.entries.iter().map(|e| e.id.as_str()).collect();
            assert_eq!(cids.len(), c.entries.len(), "duplicate identity {number}");
        }
    }
    #[test]
    fn creation_on_an_item_uses_its_actual_parent_and_library_commands_remain_disabled_after_moving() {
        let mut inv = inventory();
        let id = Uuid::from_u128(8104);
        let parent = inv.items[&id].parent;
        for target in [id, parent] {
            let c = build(&inv, target, &InventoryUi::default(), &InventoryPreferences::default(), &facts());
            let entry = c.entries.iter().find(|e| e.id == "new-folder").unwrap();
            let Kind::Action(index) = entry.kind else { panic!() };
            assert!(matches!(c.commands[index], Some(Command::Folder(to)) if to == parent));
            assert!(entry.enabled);
        }
        inv.items.get_mut(&id).unwrap().parent = inv.lib_root;
        let c = build(&inv, id, &InventoryUi::default(), &InventoryPreferences::default(), &facts());
        let new = c.entries.iter().find(|e| e.id == "new-folder").unwrap();
        assert!(!new.enabled);
        let mut layout = super::super::menu_layout::Layout::default();
        layout.move_entry(&c.entries, "new-folder", TEXT, None, None);
        assert!(
            !layout.resolve(&c.entries)[TEXT]
                .iter()
                .find(|e| e.id == "new-folder")
                .unwrap()
                .enabled
        );
    }
    #[test]
    fn trash_has_exactly_its_five_commands_and_the_separator_precedes_reload() {
        let mut inv = inventory();
        let trash = rules::system(&inv, 14).unwrap();
        inv.folders.get_mut(&trash).unwrap().state = FetchState::Fetched;
        let c = build(&inv, trash, &InventoryUi::default(), &InventoryPreferences::default(), &facts());
        let root: Vec<_> = c.entries.iter().filter(|e| e.zone == TEXT).map(|e| e.id.as_str()).collect();
        assert_eq!(
            root,
            [
                "empty-trash",
                "image",
                "separator:folder-navigation",
                "reload",
                "show-window",
                "open-window"
            ]
        );
        assert!(c.entries.iter().filter(|e| e.zone == "top").all(|e| e.id == "new" && !e.enabled));
        assert_eq!(
            c.entries
                .iter()
                .filter(|e| e.zone == "bottom")
                .map(|e| e.id.as_str())
                .collect::<Vec<_>>(),
            [LOCK]
        );
    }
    #[test]
    fn all_item_types_share_one_layout_while_live_commands_stay_specific_and_settings_survive_reload() {
        let mut inv = inventory();
        let texture = Uuid::from_u128(8104);
        let mut photo = inv.items[&texture].clone();
        photo.id = Uuid::new_v4();
        photo.inv_type = 15;
        let photo_id = photo.id;
        inv.add_items(vec![photo]);
        assert_eq!(scope(&inv, texture), "shared");
        assert_eq!(scope(&inv, photo_id), "shared");
        assert_eq!(scope(&inv, Uuid::from_u128(8000)), "shared");
        assert_eq!(scope(&inv, inv.lib_root), "shared");
        let mut link = inv.items[&texture].clone();
        link.id = Uuid::new_v4();
        link.asset_type = 24;
        link.asset_id = Uuid::nil();
        let link_id = link.id;
        inv.add_items(vec![link]);
        assert_eq!(scope(&inv, link_id), "shared");
        assert!(
            !build(&inv, link_id, &InventoryUi::default(), &InventoryPreferences::default(), &facts())
                .entries
                .iter()
                .any(|e| e.id == "attachments")
        );
        let mut settings = crate::settings::Settings::default();
        let profile = settings.inventory.context_menus.profiles.entry("shared".into()).or_default();
        profile.locked = true;
        profile.open.insert("organize".into(), true);
        profile.layout.create_group(&[], "Shared tools", Direction::Horizontal, None);
        let saved = serde_json::to_string(&settings).unwrap();
        let reopened: crate::settings::Settings = serde_json::from_str(&saved).unwrap();
        assert_eq!(reopened.inventory.context_menus, settings.inventory.context_menus);
        assert!(!reopened.inventory.context_menus.profiles.contains_key("type:photo"));
    }
}
