//! Additional inventory commands from LLInvFVBridge / LLFolderBridge and
//! menu_inventory.xml (indra/newview). Presentation remains in the shared menu.
use super::*;
use aurora_net::inventory::operations::Operation;

#[derive(Default)]
pub(in crate::ui::inventory) struct FolderCache {
    key: Option<(Uuid, u64)>,
    contacts: Vec<Uuid>,
    complete: bool,
}

impl FolderCache {
    fn contacts(&mut self, inv: &Inventory, folder: Uuid) -> &[Uuid] {
        let key = (folder, inv.generation);
        if self.key != Some(key) {
            self.key = Some(key);
            let items = descendant_items(inv, folder);
            self.complete = items.is_some();
            self.contacts = items
                .unwrap_or_default()
                .into_iter()
                .filter_map(|id| rules::original(inv, id))
                .filter_map(|id| inv.items.get(&id))
                .filter(|item| item.asset_type == 2)
                .map(|item| item.creator)
                .collect();
        }
        &self.contacts
    }
}

pub(in crate::ui::inventory) enum Command {
    CopyName(Uuid),
    ImAll(Vec<Uuid>),
    CopyFolderId(Uuid),
    CopyOutfit(Uuid),
    EmptyLost(Uuid),
    CleanLinks(Uuid),
    ShowMain(Uuid),
    OpenCurrent(Uuid),
    NewOutfit(Uuid),
    RecoverFolder(Uuid),
    FindSimilar(super::super::smart::SmartQuery),
}

/// "Find similar" for the one selected item: the submenu offers what the
/// item has in common with others, each as the rules of an advanced search
/// (Aurora's own; the Legacy Viewer has none). The creator goes by key, so
/// it needs no known name.
pub(super) fn similar(c: &mut Catalog, zone: &str, inv: &Inventory, id: Uuid, single: bool, all: bool) {
    use super::super::smart::{Field, Op, Rule, SmartQuery, name_stem};
    let item = rules::original(inv, id)
        .and_then(|id| inv.items.get(&id))
        .or_else(|| inv.items.get(&id));
    if item.is_none() && !all {
        return;
    }
    let one = |rule: Rule| {
        let mut query = SmartQuery::default();
        query.root.nodes.push(super::super::smart::Node::Rule(rule));
        query
    };
    let group = c.group(
        "find-similar",
        zone,
        "search-filled",
        aurora_i18n::tr!("ui-inventory-context-find-similar"),
        single && item.is_some(),
        Direction::Vertical,
        false,
    );
    let stem = item.map(|it| name_stem(&it.name)).filter(|stem| !stem.is_empty());
    let creator = item.map(|it| it.creator).filter(|creator| !creator.is_nil());
    let folder = item
        .and_then(|it| inv.folders.get(&it.parent))
        .map(|folder| folder.info.name.clone())
        .filter(|name| !name.is_empty());
    let entries = [
        (
            "find-similar-name",
            "typography",
            aurora_i18n::tr!("ui-inventory-context-find-similar-name"),
            stem.map(|stem| one(Rule::text(Field::Name, Op::StartsWith, &stem))),
        ),
        (
            "find-similar-creator",
            "user",
            aurora_i18n::tr!("ui-inventory-context-find-similar-creator"),
            creator.map(|key| one(Rule::text(Field::Creator, Op::Is, &key.to_string()))),
        ),
        (
            "find-similar-type",
            "category",
            aurora_i18n::tr!("ui-inventory-context-find-similar-type"),
            item.filter(|it| (0..32).contains(&it.inv_type)).map(|it| {
                let mut rule = Rule::of(Field::Type);
                // snapshots count as textures, as in the filter itself
                rule.bits = 1_u32 << if it.inv_type == 17 { 6 } else { it.inv_type };
                one(rule)
            }),
        ),
        (
            "find-similar-folder",
            "folder-filled",
            aurora_i18n::tr!("ui-inventory-context-find-similar-folder"),
            folder.map(|name| one(Rule::text(Field::Folder, Op::Is, &name))),
        ),
    ];
    for (key, icon, label, query) in entries {
        if query.is_some() || all {
            c.add(
                key,
                &group,
                icon,
                label,
                single && query.is_some(),
                super::Command::Extra(Command::FindSimilar(query.unwrap_or_default())),
            );
        }
    }
}

pub(super) fn build(c: &mut Catalog, inv: &Inventory, id: Uuid, st: &InventoryUi, prefs: &InventoryPreferences, facts: &Facts, all: bool) {
    let single = st.selected_ids(id).len() == 1;
    let folder = inv.folders.contains_key(&id);
    let ids = st.selected_ids(id);
    let selected: Vec<_> = ids
        .iter()
        .filter_map(|id| rules::original(inv, *id))
        .filter_map(|id| inv.items.get(&id))
        .collect();
    let complete_selection = selected.len() == ids.len() && !selected.is_empty();
    let trash = rules::in_type(inv, id, 14);
    let ready = st.pending.is_none();
    let listing_options = st.marketplace_commands.get(&id);
    if listing_options.is_some() || all {
        let zone = c.group(
            "listing",
            TEXT,
            "tag-filled",
            aurora_i18n::tr!("common-marketplace-listings"),
            true,
            Direction::Vertical,
            false,
        );
        for command in crate::ui::marketplace_listings::Command::ALL {
            if all || listing_options.is_some_and(|options| options.iter().any(|option| option.command == command)) {
                c.invoke(
                    command.id(),
                    &zone,
                    command.icon(),
                    command.label(),
                    single
                        && ready
                        && listing_options.is_some_and(|options| options.iter().any(|option| option.command == command && option.enabled)),
                    InvAction::Marketplace(id, command),
                );
            }
        }
    }
    let outfit_parent = folder && rules::in_type(inv, id, 48) && !rules::in_type(inv, id, 47) && rules::mutable(inv, id, &prefs.protected);
    if outfit_parent || all {
        c.add(
            "new-outfit",
            &children("new"),
            "folder-plus",
            aurora_i18n::tr!("inventory-new-outfit"),
            single && ready && outfit_parent,
            super::Command::Extra(Command::NewOutfit(id)),
        );
        if !all && outfit_parent {
            replace(c, "new-folder", single && ready, super::Command::Folder(id));
        }
        if let Some(group) = c.entries.iter_mut().find(|entry| entry.id == "new") {
            group.enabled |= single && ready && outfit_parent;
        }
    }
    if (folder && id != inv.root && !rules::under(inv, id, inv.root) && !rules::library(inv, id)) || all {
        c.add(
            "recover-folder",
            TEXT,
            "folder-filled",
            aurora_i18n::tr!("inventory-recover-folder"),
            single && ready && rules::system(inv, 16).is_some(),
            super::Command::Extra(Command::RecoverFolder(id)),
        );
    }
    if !rules::in_type(inv, id, 14) || all {
        c.add(
            "copy-name",
            &children("organize"),
            "copy-filled",
            aurora_i18n::tr!("inventory-copy-name"),
            single && (folder || inv.items.contains_key(&id)),
            super::Command::Extra(Command::CopyName(id)),
        );
    }
    if folder || all {
        // The maintainer requested the existing recoverable hidden area,
        // rather than a separate advanced setting (InventoryExposeFolderID).
        c.add(
            "folder-uuid",
            super::super::menu_layout::HIDDEN,
            "copy-filled",
            aurora_i18n::tr!("inventory-copy-folder-uuid"),
            single && folder,
            super::Command::Extra(Command::CopyFolderId(id)),
        );
    }
    let folder_window = st.ui_id != super::super::listings::PANEL && !st.scope.is_nil() && st.scope != inv.root;
    if (!trash && (st.result_view() || folder_window || st.ui_id == super::super::listings::PANEL)) || all {
        c.add(
            "show-main",
            TEXT,
            "folder-open-filled",
            aurora_i18n::tr!("inventory-show-main"),
            single,
            super::Command::Extra(Command::ShowMain(id)),
        );
    }
    if folder && folder_window || all {
        c.add(
            "open-current",
            TEXT,
            "folder-open-filled",
            aurora_i18n::tr!("inventory-open-current"),
            single,
            super::Command::Extra(Command::OpenCurrent(id)),
        );
    }
    if rules::in_type(inv, id, 46) && folder || all {
        c.add(
            "copy-outfit-list",
            TEXT,
            "copy-filled",
            aurora_i18n::tr!("inventory-copy-outfit-list"),
            single && inv.folders.get(&id).is_some_and(|folder| folder.state == FetchState::Fetched),
            super::Command::Extra(Command::CopyOutfit(id)),
        );
    }
    if inv.folders.get(&id).is_some_and(|folder| folder.info.type_default == 16) || all {
        c.add(
            "empty-lost-found",
            TEXT,
            "trash-filled",
            aurora_i18n::tr!("inventory-empty-lost-found"),
            ready
                && single
                && inv.folders.get(&id).is_some_and(|folder| {
                    folder.state == FetchState::Fetched
                        && super::super::delete::allowed(
                            inv,
                            &folder.items.iter().chain(&folder.children).copied().collect::<Vec<_>>(),
                            prefs,
                            facts,
                        )
                }),
            super::Command::Extra(Command::EmptyLost(id)),
        );
    }
    if inv
        .folders
        .get(&id)
        .is_some_and(|folder| matches!(folder.info.name.as_str(), "Aurora AO" | "#AO"))
        || all
    {
        c.add(
            "clean-broken-links",
            TEXT,
            "link-off",
            aurora_i18n::tr!("inventory-clean-broken-links"),
            ready && single && rules::mutable(inv, id, &prefs.protected),
            super::Command::Extra(Command::CleanLinks(id)),
        );
    }
    if selected.first().is_some_and(|item| item.asset_type == 56) || all {
        c.invoke(
            "apply-parcel",
            TEXT,
            "map-filled",
            aurora_i18n::tr!("ui-env-editor-apply-parcel"),
            single && facts.parcel_environment && selected.first().is_some_and(|item| !item.asset_id.is_nil()),
            InvAction::EnvironmentParcel(selected.first().map_or(id, |item| item.id)),
        );
    }
    if complete_selection && selected.iter().all(|item| matches!(item.asset_type, 0 | 15)) || all {
        let permitted = complete_selection
            && selected.iter().all(|item| {
                !item.asset_id.is_nil()
                    && item.owner_mask & (rules::COPY | rules::MODIFY | rules::TRANSFER) == rules::COPY | rules::MODIFY | rules::TRANSFER
            });
        let formats = c.group(
            "save-textures",
            TEXT,
            "device-floppy",
            if single {
                aurora_i18n::tr!("common-save-as")
            } else {
                aurora_i18n::tr!("inventory-save-selected-textures")
            },
            permitted && !trash,
            Direction::Vertical,
            false,
        );
        for (key, label, png) in [
            ("save-textures-png", aurora_i18n::tr!("inventory-export-png"), true),
            ("save-textures-tga", aurora_i18n::tr!("inventory-export-tga"), false),
        ] {
            c.invoke(
                key,
                &formats,
                "device-floppy",
                label,
                permitted && !trash,
                InvAction::ExportTextures {
                    items: selected.iter().map(|item| item.id).collect(),
                    png,
                },
            );
        }
    }
    if selected
        .first()
        .is_some_and(|item| item.asset_type == 6 && facts.worn.contains(&item.id))
        || all
    {
        c.invoke(
            "refresh-attachment-textures",
            TEXT,
            "refresh",
            aurora_i18n::tr!("ui-context-refresh-textures"),
            single && !trash,
            InvAction::RefreshAttachment(selected.first().map_or(id, |item| item.id)),
        );
    }
    let cards: Vec<_> = if folder {
        if inv
            .folders
            .get(&id)
            .is_some_and(|folder| !folder.library && folder.info.type_default == -1)
        {
            let mut cache = st.context_folders.borrow_mut();
            let contacts = cache.contacts(inv, id).to_vec();
            if !all && single && !cache.complete {
                c.prepare_outfit = Some(id);
            }
            contacts
        } else {
            Vec::new()
        }
    } else {
        selected
            .iter()
            .filter(|item| item.asset_type == 2)
            .map(|item| item.creator)
            .collect()
    };
    let contacts: Vec<_> = cards
        .into_iter()
        .filter(|who| !who.is_nil() && *who != facts.agent)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let cards_only = complete_selection && selected.iter().all(|item| item.asset_type == 2);
    if selected.first().is_some_and(|item| item.asset_type == 2) || all {
        let who = contacts.first().copied().unwrap_or_default();
        for (key, icon, label, action, enabled) in [
            (
                "card-im",
                "message-filled",
                aurora_i18n::tr!("inventory-card-im"),
                crate::ui::context::CtxAction::Im(who),
                single && !who.is_nil(),
            ),
            (
                "card-offer-teleport",
                "map-pin-filled",
                aurora_i18n::tr!("inventory-card-offer-teleport"),
                crate::ui::context::CtxAction::OfferTeleports(contacts.clone()),
                cards_only && !contacts.is_empty() && contacts.iter().all(|who| facts.online.contains(who)),
            ),
            (
                "card-request-teleport",
                "map-pin-filled",
                aurora_i18n::tr!("inventory-card-request-teleport"),
                crate::ui::context::CtxAction::RequestTeleport(who),
                single && facts.online.contains(&who),
            ),
        ] {
            c.add(key, TEXT, icon, label, enabled && !trash, super::Command::Worn(action));
        }
        replace(
            c,
            "card-im",
            cards_only && !contacts.is_empty() && !trash,
            super::Command::Extra(Command::ImAll(contacts.clone())),
        );
    }
    if !contacts.is_empty() || all {
        if folder || all {
            c.add(
                "folder-im-all",
                TEXT,
                "message-filled",
                aurora_i18n::tr!("inventory-folder-im-all"),
                single && !contacts.is_empty() && !trash,
                super::Command::Extra(Command::ImAll(contacts.clone())),
            );
        }
        c.add(
            "card-conference",
            TEXT,
            "messages-filled",
            aurora_i18n::tr!("inventory-card-conference"),
            !contacts.is_empty() && !trash && (folder && single || cards_only && contacts.iter().all(|who| facts.online.contains(who))),
            super::Command::Worn(crate::ui::context::CtxAction::Conference(contacts)),
        );
    }
    if !all {
        let wearable = ready
            && !facts.appearance_busy
            && complete_selection
            && selected
                .iter()
                .all(|item| matches!(item.asset_type, 5 | 6 | 13) && !facts.worn.contains(&item.id) && !rules::in_type(inv, item.id, 14));
        let objects = complete_selection && selected.iter().all(|item| item.asset_type == 6);
        let keys: Vec<_> = c
            .entries
            .iter()
            .filter(|entry| matches!(entry.id.as_str(), "wear" | "add") || entry.id.starts_with("attach:"))
            .map(|entry| entry.id.clone())
            .collect();
        for key in keys {
            let point = key.strip_prefix("attach:").and_then(|point| point.parse().ok()).unwrap_or(0);
            let replace_items = key == "wear";
            let mut batch = Vec::new();
            let ordinary: Vec<_> = selected
                .iter()
                .filter(|item| !rules::library(inv, item.id))
                .map(|item| item.id)
                .collect();
            if !ordinary.is_empty() {
                batch.push(InvAction::Appearance(Wear::WearItems {
                    items: ordinary,
                    replace: replace_items,
                    point,
                }));
            }
            for item in selected.iter().filter(|item| rules::library(inv, item.id)) {
                batch.push(InvAction::WearLibrary {
                    item: item.id,
                    replace: replace_items,
                    point,
                });
            }
            if selected.len() > 1 && selected.iter().any(|item| rules::library(inv, item.id)) {
                batch = vec![InvAction::WearSelection {
                    items: selected.iter().map(|item| item.id).collect(),
                    replace: replace_items,
                    point,
                }];
            }
            let enabled = wearable
                && if key.starts_with("attach:") {
                    objects
                } else if key == "add" {
                    selected.iter().all(|item| item.asset_type != 13 && item.flags & 0xff != 15)
                } else {
                    true
                };
            replace(c, &key, enabled, super::Command::Batch(batch));
        }
        for group in c
            .entries
            .iter_mut()
            .filter(|entry| matches!(entry.id.as_str(), "attach-body" | "attach-hud"))
        {
            group.enabled = wearable && objects;
        }
        // Preserve stable identities while allowing the native bulk actions.
        for (key, local) in [("play-world", false), ("play-local", true)] {
            let batch = selected
                .iter()
                .filter(|item| item.asset_type == 20 && !item.asset_id.is_nil())
                .map(|item| InvAction::Animation {
                    asset: item.asset_id,
                    local,
                    start: true,
                })
                .collect();
            replace(
                c,
                key,
                complete_selection && selected.iter().all(|item| item.asset_type == 20 && !item.asset_id.is_nil()),
                super::Command::Batch(batch),
            );
        }
        replace(
            c,
            "play-sound",
            complete_selection && selected.iter().all(|item| item.asset_type == 1 && !item.asset_id.is_nil()),
            super::Command::Batch(selected.iter().map(|item| InvAction::Sound(item.asset_id)).collect()),
        );
        replace(
            c,
            "rez-previous",
            complete_selection
                && selected
                    .iter()
                    .all(|item| item.asset_type == 6 && !facts.worn.contains(&item.id) && rules::mutable(inv, item.id, &prefs.protected)),
            super::Command::Batch(selected.iter().map(|item| InvAction::Restore(item.id)).collect()),
        );
    }
    if all
        || complete_selection
            && selected.iter().all(|item| item.asset_type == 21)
            && selected.iter().any(|item| facts.active_gestures.contains(&item.id))
            && selected.iter().any(|item| !facts.active_gestures.contains(&item.id))
    {
        c.invoke(
            "gesture-deactivate",
            TEXT,
            "circle-x-filled",
            aurora_i18n::tr!("gesture-deactivate"),
            ready && !trash && selected.iter().all(|item| !rules::library(inv, item.id)),
            InvAction::Gesture(crate::ui::gestures::Action::Active(
                selected.iter().map(|item| item.id).collect(),
                false,
            )),
        );
    }
    if trash {
        if let Some(target) = inv
            .items
            .get(&id)
            .filter(|item| matches!(item.asset_type, 24 | 25))
            .and_then(|_| rules::original(inv, id))
        {
            c.add(
                "original",
                TEXT,
                "external-link-filled",
                aurora_i18n::tr!("ui-inventory-context-find-original"),
                single,
                super::Command::Original(target),
            );
        }
        for (kind, key, label, local) in [
            (1, "play-sound", aurora_i18n::tr!("common-play"), true),
            (
                20,
                "play-world",
                aurora_i18n::tr!("ui-inventory-context-play-in-second-life"),
                false,
            ),
            (20, "play-local", aurora_i18n::tr!("common-play-locally"), true),
        ] {
            if complete_selection && selected.iter().all(|item| item.asset_type == kind) {
                let batch = selected
                    .iter()
                    .map(|item| {
                        if kind == 1 {
                            InvAction::Sound(item.asset_id)
                        } else {
                            InvAction::Animation {
                                asset: item.asset_id,
                                local,
                                start: true,
                            }
                        }
                    })
                    .collect();
                c.add(
                    key,
                    TEXT,
                    "player-play-filled",
                    label,
                    selected.iter().all(|item| !item.asset_id.is_nil()),
                    super::Command::Batch(batch),
                );
            }
        }
    }
}

fn replace(c: &mut Catalog, id: &str, enabled: bool, command: super::Command) {
    if c.editing {
        return;
    }
    if let Some(entry) = c.entries.iter_mut().find(|entry| entry.id == id) {
        entry.enabled = enabled;
        if let Kind::Action(index) = entry.kind {
            c.commands[index] = Some(command);
        }
    }
}

fn descendant_items(inv: &Inventory, root: Uuid) -> Option<Vec<Uuid>> {
    let mut pending = vec![root];
    let mut seen = HashSet::new();
    let mut items = Vec::new();
    while let Some(id) = pending.pop() {
        if !seen.insert(id) {
            return None;
        }
        let folder = inv.folders.get(&id)?;
        if folder.state != FetchState::Fetched {
            return None;
        }
        items.extend(folder.items.iter().copied());
        pending.extend(folder.children.iter().copied());
    }
    Some(items)
}

fn discover_originals(inv: &mut Inventory, root: Uuid) -> bool {
    let mut pending = vec![root];
    let mut seen = HashSet::new();
    let mut requests = Vec::new();
    let mut complete = true;
    while let Some(id) = pending.pop() {
        if !seen.insert(id) {
            continue;
        }
        let Some(folder) = inv.folders.get(&id) else { continue };
        complete &= folder.state == FetchState::Fetched;
        if folder.state == FetchState::Unknown && inv.queue.len() + requests.len() < 16 {
            requests.push(id);
        }
        pending.extend(folder.children.iter().copied());
    }
    for id in requests {
        inv.request(id);
    }
    complete
}

pub(in crate::ui::inventory) fn process_deferred(inv: &mut Inventory, st: &mut InventoryUi, prefs: &InventoryPreferences, facts: &Facts) {
    let Some(folder) = st.pending_cleanup else { return };
    if st.pending.is_some() || st.dialog.is_some() || st.cleanup_generation == Some(inv.generation) {
        return;
    }
    st.cleanup_generation = Some(inv.generation);
    if !inv.folders.contains_key(&folder) {
        st.pending_cleanup = None;
        st.message = aurora_i18n::tr!("common-folder-not-found").into();
        return;
    }
    // Complete discovery before deciding that an original is missing. Fetches
    // and the confirmation follow the pinned folder even if selection changes.
    if !discover_originals(inv, inv.root)
        || !discover_originals(inv, folder)
        || !inv.lib_root.is_nil() && !discover_originals(inv, inv.lib_root)
    {
        st.fetch_all = true;
        st.message = aurora_i18n::tr!("ui-inventory-dialogs-loading-all-folders-before-replacing").into();
        return;
    }
    let Some(items) = descendant_items(inv, folder) else { return };
    st.pending_cleanup = None;
    let broken: Vec<_> = items
        .into_iter()
        .filter(|id| {
            inv.items
                .get(id)
                .is_some_and(|item| matches!(item.asset_type, 24 | 25) && rules::original(inv, item.id).is_none())
        })
        .collect();
    if broken.is_empty() {
        st.message = aurora_i18n::tr!("inventory-no-broken-links").into();
    } else if super::super::delete::allowed(inv, &broken, prefs, facts) {
        st.message.clear();
        st.dialog = Some(EditDialog::Delete(broken, false));
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
    match command {
        Command::FindSimilar(query) => st.find_similar(query),
        Command::ImAll(contacts) => {
            for who in contacts {
                crate::ui::context::request(ui.ctx(), crate::ui::context::CtxAction::Im(who));
            }
        }
        Command::CopyName(id) => ui.ctx().copy_text(rules::name(inv, id)),
        Command::CopyFolderId(id) => {
            if inv.folders.contains_key(&id) {
                ui.ctx().copy_text(id.to_string());
            }
        }
        Command::CopyOutfit(id) => {
            if let Some(folder) = inv.folders.get(&id).filter(|folder| folder.state == FetchState::Fetched) {
                ui.ctx().copy_text(
                    folder
                        .items
                        .iter()
                        .filter_map(|id| inv.items.get(id))
                        .map(|item| item.name.as_str())
                        .collect::<Vec<_>>()
                        .join("\n"),
                );
            }
        }
        Command::EmptyLost(id) => {
            if let Some(folder) = inv
                .folders
                .get(&id)
                .filter(|folder| folder.info.type_default == 16 && folder.state == FetchState::Fetched)
            {
                let ids = folder.items.iter().chain(&folder.children).copied().collect::<Vec<_>>();
                if super::super::delete::allowed(inv, &ids, prefs, facts) {
                    st.dialog = Some(EditDialog::Delete(ids, false));
                }
            }
        }
        Command::CleanLinks(id) => {
            st.pending_cleanup = Some(id);
            st.cleanup_generation = None;
            st.fetch_all = true;
        }
        Command::ShowMain(id) => {
            actions.push(InvAction::ShowMain(id));
        }
        Command::OpenCurrent(id) => {
            if inv.folders.contains_key(&id) {
                st.navigate_to = Some(id);
                st.search.clear();
                st.view = Default::default();
            }
        }
        Command::NewOutfit(parent) => {
            if rules::in_type(inv, parent, 48) && !rules::in_type(inv, parent, 47) && rules::mutable(inv, parent, &prefs.protected) {
                let id = Uuid::new_v4();
                actions.push(InvAction::Edit(Mutation {
                    operations: vec![Operation::Folder {
                        id,
                        parent,
                        name: aurora_i18n::tr!("inventory-new-outfit").into(),
                        type_default: 47,
                    }],
                    refresh: vec![parent, id],
                    ..Default::default()
                }));
            }
        }
        Command::RecoverFolder(id) => {
            if let Some(to) = rules::system(inv, 16) {
                match rules::move_selection(inv, &[id], to, &prefs.protected, &facts.worn) {
                    Ok(change) => actions.push(InvAction::Edit(change)),
                    Err(reason) => st.message = reason,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed() -> Inventory {
        let mut inv = Inventory::default();
        crate::world::inventory::demo::seed(&mut inv, Uuid::from_u128(2));
        inv
    }

    #[test]
    fn texture_export_formats_keep_the_whole_selection_and_recheck_permissions() {
        let mut inv = seed();
        let texture = Uuid::from_u128(8104);
        let snapshot = Uuid::from_u128(8100);
        inv.items.get_mut(&snapshot).expect("snapshot").asset_type = 15;
        let link = Uuid::from_u128(8113);
        inv.items.get_mut(&link).expect("texture link").asset_id = texture;
        let mut state = InventoryUi::default();
        state.selection.extend([link, snapshot]);
        for permitted in [true, false] {
            if !permitted {
                inv.items.get_mut(&snapshot).expect("snapshot").owner_mask &= !rules::TRANSFER;
            }
            let menu = super::super::build(&inv, link, &state, &InventoryPreferences::default(), &Facts::default());
            for (key, expected_png) in [("save-textures-png", true), ("save-textures-tga", false)] {
                let entry = menu.entries.iter().find(|entry| entry.id == key).expect("export format");
                assert_eq!(entry.enabled, permitted);
                assert_eq!(entry.zone, children("save-textures"));
                let Kind::Action(index) = entry.kind else { panic!("export action") };
                let Some(super::super::Command::Invoke(InvAction::ExportTextures { items, png })) = &menu.commands[index] else {
                    panic!("texture export command")
                };
                assert_eq!(*png, expected_png);
                assert_eq!(items.len(), 2);
                assert!(items.contains(&texture) && items.contains(&snapshot));
            }
        }
    }

    #[test]
    fn every_wearable_edit_menu_resolves_links_and_obeys_modify_rights() {
        for kind in (1..=14).chain(std::iter::once(16)) {
            let mut inv = seed();
            let id = Uuid::from_u128(8103);
            let asset = inv.items.get_mut(&id).expect("original");
            asset.asset_type = if kind < 4 { 13 } else { 5 };
            asset.flags = kind;
            let link = Uuid::from_u128(8113);
            inv.items.get_mut(&link).expect("link").asset_id = id;
            let mut facts = Facts::default();
            facts.worn.insert(id);
            for target in [id, link] {
                for modify in [true, false] {
                    inv.items.get_mut(&id).expect("original").owner_mask = if modify { rules::MODIFY } else { 0 };
                    let menu = super::super::build(&inv, target, &InventoryUi::default(), &InventoryPreferences::default(), &facts);
                    let entry = menu
                        .entries
                        .iter()
                        .find(|entry| entry.id == "edit-worn")
                        .expect("edit entry for every worn type");
                    assert_eq!(entry.enabled, modify);
                    let Kind::Action(index) = entry.kind else { panic!("native action") };
                    assert!(
                        matches!(&menu.commands[index], Some(super::super::Command::Worn(crate::ui::context::CtxAction::EditWorn(item))) if *item == id)
                    );
                }
            }
        }
    }

    #[test]
    fn library_attachment_menu_keeps_the_explicit_hud_point_through_copy_then_wear() {
        let mut inv = seed();
        let id = Uuid::from_u128(8100);
        inv.items.get_mut(&id).expect("object").parent = inv.lib_root;
        let mut facts = Facts::default();
        facts.points.push((31, true, "HUD".into()));
        let menu = super::super::build(&inv, id, &InventoryUi::default(), &InventoryPreferences::default(), &facts);
        let entry = menu.entries.iter().find(|entry| entry.id == "attach:31").expect("HUD point");
        assert!(entry.enabled);
        let Kind::Action(index) = entry.kind else { panic!("action") };
        assert!(
            matches!(&menu.commands[index], Some(super::super::Command::Batch(batch)) if matches!(&batch[..], [InvAction::WearLibrary { item, replace: false, point: 31 }] if *item == id))
        );
    }

    #[test]
    fn mixed_gestures_offer_explicit_activation_and_deactivation() {
        let mut inv = seed();
        let ids = [Uuid::from_u128(8103), Uuid::from_u128(8100)];
        for id in ids {
            inv.items.get_mut(&id).expect("gesture").asset_type = 21;
        }
        let mut state = InventoryUi::default();
        state.selection.extend(ids);
        let mut facts = Facts::default();
        facts.active_gestures.insert(ids[0]);
        let menu = super::super::build(&inv, ids[0], &state, &InventoryPreferences::default(), &facts);
        for key in ["gesture-active", "gesture-deactivate"] {
            assert!(
                menu.entries
                    .iter()
                    .find(|entry| entry.id == key)
                    .expect("gesture state command")
                    .enabled
            );
        }
    }

    #[test]
    fn card_commands_reject_a_mixed_selection_and_enable_valid_online_contacts() {
        let mut inv = seed();
        let id = Uuid::from_u128(8103);
        let who = Uuid::from_u128(77);
        let card = inv.items.get_mut(&id).expect("card");
        card.asset_type = 2;
        card.creator = who;
        let mut facts = Facts::default();
        facts.online.insert(who);
        let mut state = InventoryUi::default();
        for mixed in [false, true] {
            if mixed {
                state.selection.extend([id, Uuid::from_u128(8100)]);
            }
            let menu = super::super::build(&inv, id, &state, &InventoryPreferences::default(), &facts);
            for key in ["card-im", "card-offer-teleport", "card-request-teleport", "card-conference"] {
                assert_eq!(
                    menu.entries.iter().find(|entry| entry.id == key).is_some_and(|entry| entry.enabled),
                    !mixed,
                    "{key}"
                );
            }
        }
    }

    #[test]
    fn empty_lost_and_found_obeys_the_same_worn_and_protection_rules_as_delete() {
        let mut inv = seed();
        let lost = Uuid::from_u128(9400);
        let item = Uuid::from_u128(8100);
        let mut folder = inv.folders[&Uuid::from_u128(8000)].clone();
        folder.info.id = lost;
        folder.info.type_default = 16;
        folder.children.clear();
        folder.items = vec![item];
        inv.folders.insert(lost, folder);
        inv.items.get_mut(&item).expect("lost item").parent = lost;
        let mut facts = Facts::default();
        let mut prefs = InventoryPreferences::default();
        for blocked in 0..3 {
            if blocked == 1 {
                facts.worn.insert(item);
            }
            if blocked == 2 {
                facts.worn.clear();
                prefs.protected.insert(lost);
            }
            let menu = super::super::build(&inv, lost, &InventoryUi::default(), &prefs, &facts);
            assert_eq!(
                menu.entries
                    .iter()
                    .find(|entry| entry.id == "empty-lost-found")
                    .expect("empty command")
                    .enabled,
                blocked == 0
            );
        }
    }

    #[test]
    fn cleanup_waits_for_the_library_and_keeps_its_original_folder_after_selection_changes() {
        let mut inv = seed();
        let folder = Uuid::from_u128(8000);
        let mut root = inv.folders[&folder].clone();
        root.info.id = inv.root;
        root.info.type_default = 8;
        root.items.clear();
        root.children = vec![folder];
        inv.folders.insert(inv.root, root);
        inv.folders.get_mut(&inv.lib_root).expect("library").state = FetchState::Unknown;
        let broken = Uuid::from_u128(9191);
        let mut link = inv.items[&Uuid::from_u128(8113)].clone();
        link.id = broken;
        link.parent = folder;
        link.asset_id = Uuid::from_u128(98989);
        inv.add_items(vec![link]);
        let mut state = InventoryUi {
            pending_cleanup: Some(folder),
            ..Default::default()
        };
        let prefs = InventoryPreferences::default();
        process_deferred(&mut inv, &mut state, &prefs, &Facts::default());
        assert_eq!(state.pending_cleanup, Some(folder));
        assert!(state.dialog.is_none());
        assert!(inv.queue.iter().any(|(id, _)| *id == inv.lib_root));
        state.selection.insert(Uuid::from_u128(8001));
        inv.folders.get_mut(&inv.lib_root).expect("library").state = FetchState::Fetched;
        inv.generation += 1;
        process_deferred(&mut inv, &mut state, &prefs, &Facts::default());
        assert!(state.pending_cleanup.is_none());
        assert!(matches!(&state.dialog,Some(EditDialog::Delete(ids,false)) if ids.contains(&broken)));
    }

    #[test]
    fn folder_uuid_copies_the_folder_key_without_modifying_the_inventory_clipboard() {
        let inv = seed();
        let ctx = egui::Context::default();
        let mut state = InventoryUi::default();
        state.clipboard.push(Uuid::from_u128(8103));
        let before = state.clipboard.clone();
        let mut prefs = InventoryPreferences::default();
        let mut actions = Vec::new();
        let mut output = ctx.run_ui(Default::default(), |ui| {
            execute(
                Command::CopyFolderId(Uuid::from_u128(8000)),
                ui,
                &inv,
                &mut state,
                &mut prefs,
                &Facts::default(),
                &mut actions,
            )
        });
        assert!(
            output
                .platform_output
                .commands
                .iter()
                .any(|command| matches!(command, egui::OutputCommand::CopyText(text) if text == &Uuid::from_u128(8000).to_string()))
        );
        output.textures_delta.clear();
        assert_eq!(state.clipboard, before);
        assert!(actions.is_empty());
    }

    #[test]
    fn folder_uuid_starts_hidden_in_new_and_saved_layouts_and_can_be_recovered() {
        let mut inv = Inventory::default();
        crate::world::inventory::demo::seed(&mut inv, Uuid::from_u128(2));
        let id = Uuid::from_u128(8000);
        let catalog = super::super::build(
            &inv,
            id,
            &InventoryUi::default(),
            &InventoryPreferences::default(),
            &Facts::default(),
        );
        let entry = catalog
            .entries
            .iter()
            .find(|entry| entry.id == "folder-uuid")
            .expect("folder command");
        assert!(entry.enabled);
        for mut layout in [
            super::super::super::menu_layout::Layout::default(),
            super::super::super::menu_layout::Profile::default().layout,
        ] {
            let resolved = layout.resolve(&catalog.entries);
            assert!(
                resolved[super::super::super::menu_layout::HIDDEN]
                    .iter()
                    .any(|entry| entry.id == "folder-uuid")
            );
            layout.placements.insert("folder-uuid".into(), TEXT.into());
            let resolved = layout.resolve(&catalog.entries);
            assert!(resolved[TEXT].iter().any(|entry| entry.id == "folder-uuid"));
        }
        let metadata = super::super::editing_entries(&[], &Facts::default());
        assert_eq!(metadata.iter().filter(|entry| entry.id == "folder-uuid").count(), 1);
    }
}
