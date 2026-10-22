//! Shared wear traversal follows RlvWearableItemCollector (rlvinventory.cpp,
//! originally LGPL 2.1), including folded folders and single-level folder links.
use super::{naming, preferences::Preferences};
use crate::world::inventory::{Inventory, actions};
use std::collections::HashSet;
use uuid::Uuid;

pub(super) struct Entry {
    pub item: Uuid,
    /// Logical parent after point-folder and folder-link folding.
    pub folder: Uuid,
    pub point: u8,
    pub replace: bool,
}

#[derive(Default)]
pub(super) struct Collection {
    pub items: Vec<Entry>,
    pub missing: Vec<Uuid>,
    pub missing_items: Vec<Uuid>,
}

pub(super) fn attachment_point(inv: &Inventory, id: Uuid, legacy: bool) -> Option<u8> {
    let item = inv.items.get(&id)?;
    if item.asset_type == 24 {
        let target = inv.items.get(&item.asset_id)?;
        if target.asset_type != 6 {
            return None;
        }
        return attachment_point(inv, target.id, legacy).or_else(|| {
            inv.folders
                .get(&item.parent)
                .and_then(|f| naming::folder_point(&f.info.name, legacy))
        });
    }
    if item.asset_type != 6 {
        return None;
    }
    let folder = inv
        .folders
        .get(&item.parent)
        .and_then(|f| naming::folder_point(&f.info.name, legacy));
    let name = naming::item_point(&item.name);
    if item.owner_mask & actions::MODIFY != 0 {
        name.or(folder)
    } else {
        folder.or(name)
    }
}

pub(super) fn strippable(inv: &Inventory, id: Uuid, legacy: bool) -> bool {
    // isStrippable follows link targets, ignores a link's own name, and only
    // climbs above the containing folder while that folder is folded.
    let mut pending = vec![id];
    let mut seen = HashSet::new();
    while let Some(id) = pending.pop() {
        if !seen.insert(id) {
            continue;
        }
        let Some(item) = inv.items.get(&id) else {
            continue;
        };
        if item.asset_type == 24 {
            pending.push(item.asset_id);
        } else if item.name.contains("nostrip") {
            return false;
        }
        let mut parent = item.parent;
        while seen.insert(parent) {
            let Some(folder) = inv.folders.get(&parent) else {
                break;
            };
            if folder.info.name.contains("nostrip") {
                return false;
            }
            if folder.info.parent == inv.root || !naming::folded(&folder.info.name, legacy) {
                break;
            }
            parent = folder.info.parent;
        }
    }
    true
}

pub(super) fn collect(inv: &Inventory, base: Uuid, all: bool, attach: bool, replace: bool, prefs: &Preferences) -> Collection {
    struct Frame {
        id: Uuid,
        parent: Uuid,
        linked: bool,
        folded: bool,
        replace: bool,
    }
    let mut result = Collection::default();
    let mode = inv.folders.get(&base).map_or(replace, |f| prefs.replace(&f.info.name, replace));
    let mut stack = vec![Frame {
        id: base,
        parent: base,
        linked: false,
        folded: false,
        replace: mode,
    }];
    let mut seen = HashSet::new();
    let mut items = HashSet::new();
    while let Some(frame) = stack.pop() {
        if !seen.insert(frame.id) {
            continue;
        }
        let Some(folder) = inv.folders.get(&frame.id) else {
            result.missing.push(frame.id);
            continue;
        };
        if !crate::world::appearance::loaded(inv, frame.id) {
            result.missing.push(frame.id);
            continue;
        }
        for child in folder
            .children
            .iter()
            .rev()
            .filter(|_| !frame.folded)
            .filter_map(|id| inv.folders.get(id))
        {
            let folded = naming::folded(&child.info.name, prefs.legacy_naming);
            if folded {
                if !crate::world::appearance::loaded(inv, child.info.id) {
                    result.missing.push(child.info.id);
                    continue;
                }
                let objects = child
                    .items
                    .iter()
                    .filter_map(|id| actions::original(inv, *id))
                    .filter(|id| inv.items.get(id).is_some_and(|it| it.asset_type == 6))
                    .count();
                if attach && objects != 1 {
                    continue;
                }
            } else if !all || frame.linked || child.info.name.is_empty() || child.info.name.starts_with('.') {
                continue;
            }
            stack.push(Frame {
                id: child.info.id,
                parent: if folded { frame.parent } else { child.info.id },
                linked: frame.linked,
                folded,
                replace: if folded {
                    frame.replace
                } else {
                    prefs.replace(&child.info.name, mode)
                },
            });
        }
        for offered in folder.items.iter().filter_map(|id| inv.items.get(id)) {
            if offered.asset_type == 25 {
                // Never descend through a second link, an outfit, or back into
                // the operation's own subtree.
                if frame.linked || actions::under(inv, offered.asset_id, base) {
                    continue;
                }
                if inv
                    .folders
                    .get(&offered.asset_id)
                    .is_some_and(|f| f.info.type_default == 47 || f.info.name.is_empty() || f.info.name.starts_with('.'))
                {
                    continue;
                }
                stack.push(Frame {
                    id: offered.asset_id,
                    parent: frame.parent,
                    linked: true,
                    folded: false,
                    replace: frame.replace,
                });
                continue;
            }
            let Some(id) = actions::original(inv, offered.id) else {
                if offered.asset_type == 24 {
                    result.missing_items.push(offered.asset_id);
                }
                continue;
            };
            let Some(item) = inv.items.get(&id) else {
                continue;
            };
            let point = attachment_point(inv, offered.id, prefs.legacy_naming);
            if !attach && !strippable(inv, offered.id, prefs.legacy_naming) {
                continue;
            }
            let eligible = match item.asset_type {
                13 if !attach => false,
                5 | 13 => !frame.folded || (attach && strippable(inv, offered.id, prefs.legacy_naming)),
                6 => !attach || point.is_some() || prefs.shared_wear,
                21 => !frame.folded,
                _ => false,
            };
            if eligible && items.insert(id) {
                result.items.push(Entry {
                    item: id,
                    folder: frame.parent,
                    point: if item.asset_type == 6 {
                        point.unwrap_or((item.flags & 0xff) as u8)
                    } else {
                        0
                    },
                    replace: frame.replace,
                });
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::inventory::{FetchState, Folder};
    use aurora_net::inventory::{InvFolder, InvItem};

    fn folder(inv: &mut Inventory, number: u128, parent: Uuid, name: &str) -> Uuid {
        let id = Uuid::from_u128(number);
        inv.folders.insert(
            id,
            Folder {
                info: InvFolder {
                    id,
                    parent,
                    name: name.into(),
                    type_default: -1,
                    ..Default::default()
                },
                children: vec![],
                items: vec![],
                state: FetchState::Fetched,
                library: false,
            },
        );
        if let Some(parent) = inv.folders.get_mut(&parent) {
            parent.children.push(id);
        }
        id
    }

    fn item(inv: &mut Inventory, number: u128, parent: Uuid, name: &str, kind: i32) -> Uuid {
        let id = Uuid::from_u128(number);
        inv.items.insert(
            id,
            InvItem {
                id,
                parent,
                name: name.into(),
                asset_type: kind,
                owner_mask: actions::MODIFY,
                ..Default::default()
            },
        );
        inv.folders.get_mut(&parent).unwrap().items.push(id);
        id
    }

    fn fixture() -> (Inventory, Uuid) {
        let mut inv = Inventory::default();
        inv.root = folder(&mut inv, 1, Uuid::nil(), "Inventory");
        let root = inv.root;
        let shared = folder(&mut inv, 2, root, "#RLV");
        let base = folder(&mut inv, 3, shared, "Outfit");
        (inv, base)
    }

    #[test]
    fn rlva_nostrip_checks_originals_and_folded_parents_without_locking_unrelated_ancestors() {
        let (mut inv, base) = fixture();
        inv.folders.get_mut(&base).unwrap().info.name = "Outfit nostrip".into();
        let ordinary = folder(&mut inv, 4, base, "Ordinary child");
        let object = item(&mut inv, 100, ordinary, "Object", 6);
        assert!(strippable(&inv, object, true), "a normal containing folder ends the ascent");
        inv.folders.get_mut(&ordinary).unwrap().info.name = ".(chest)".into();
        assert!(!strippable(&inv, object, true));
        inv.folders.get_mut(&base).unwrap().info.name = "Outfit".into();
        inv.items.get_mut(&object).unwrap().name = "NOSTRIP".into();
        assert!(strippable(&inv, object, true), "the native nostrip marker is case-sensitive");
        let link = item(&mut inv, 101, base, "Link nostrip", 24);
        inv.items.get_mut(&link).unwrap().asset_id = object;
        assert!(strippable(&inv, link, true), "link labels are ignored");
        inv.items.get_mut(&object).unwrap().name = "Object nostrip".into();
        assert!(!strippable(&inv, link, true), "the original item's marker still applies");
    }

    #[test]
    fn rlva_attachment_point_lookup_follows_originals_and_modify_precedence() {
        let (mut inv, base) = fixture();
        let chest = folder(&mut inv, 4, base, ".(chest)");
        let object = item(&mut inv, 100, chest, "Object (old) (skull)", 6);
        assert_eq!(attachment_point(&inv, object, true), Some(2));
        inv.items.get_mut(&object).unwrap().owner_mask = 0;
        assert_eq!(attachment_point(&inv, object, true), Some(1));
        let hand = folder(&mut inv, 5, base, ".(right hand)");
        let link = item(&mut inv, 101, hand, "Link (left foot)", 24);
        inv.items.get_mut(&link).unwrap().asset_id = object;
        assert_eq!(
            attachment_point(&inv, link, true),
            Some(1),
            "the original's point wins over the link's folder"
        );
        inv.items.get_mut(&object).unwrap().parent = base;
        inv.items.get_mut(&object).unwrap().name = "Object".into();
        assert_eq!(attachment_point(&inv, link, true), Some(6), "the link's own point name is ignored");
    }

    #[test]
    fn rlva_shared_wear_collects_gestures_prefixes_and_only_valid_folded_contents() {
        let (mut inv, base) = fixture();
        let gesture = item(&mut inv, 100, base, "Gesture", 21);
        let object = item(&mut inv, 101, base, "Unnamed point", 6);
        let add = folder(&mut inv, 4, base, "+Extra");
        let extra = item(&mut inv, 102, add, "Extra shirt", 5);
        let fold = folder(&mut inv, 5, base, ".(chest)");
        let folded_object = item(&mut inv, 103, fold, "Attachment", 6);
        let folded_shirt = item(&mut inv, 104, fold, "Shirt", 5);
        item(&mut inv, 105, fold, "Gesture", 21);
        item(&mut inv, 106, fold, "Shirt nostrip", 5);
        let nested = folder(&mut inv, 20, fold, "Nested");
        item(&mut inv, 110, nested, "Nested shirt", 5);
        let nested_fold = folder(&mut inv, 21, fold, ".(skull)");
        item(&mut inv, 111, nested_fold, "Nested object", 6);
        let prefs = Preferences::default();
        let result = collect(&inv, base, true, true, true, &prefs);
        assert_eq!(result.items.len(), 5);
        assert!(result.items.iter().any(|entry| entry.item == gesture));
        assert!(result.items.iter().any(|entry| entry.item == folded_shirt && entry.folder == base));
        assert!(
            result
                .items
                .iter()
                .any(|entry| entry.item == folded_object && entry.point == 1 && entry.folder == base)
        );
        assert!(result.items.iter().any(|entry| entry.item == extra && !entry.replace));
        let prefs = Preferences {
            shared_wear: false,
            ..prefs
        };
        assert!(
            !collect(&inv, base, false, true, true, &prefs)
                .items
                .iter()
                .any(|entry| entry.item == object)
        );
        item(&mut inv, 107, fold, "Second attachment", 6);
        assert!(
            !collect(&inv, base, false, true, true, &prefs)
                .items
                .iter()
                .any(|entry| entry.item == folded_object)
        );
        assert!(
            collect(&inv, base, false, false, true, &prefs)
                .items
                .iter()
                .any(|entry| entry.item == folded_object)
        );
    }

    #[test]
    fn rlva_folder_links_fold_direct_contents_without_second_links_outfits_or_recursion() {
        let (mut inv, base) = fixture();
        let root = inv.root;
        let target = folder(&mut inv, 4, root, "External");
        let direct = item(&mut inv, 100, target, "External gesture", 21);
        let child = folder(&mut inv, 5, target, "Child");
        item(&mut inv, 101, child, "Do not include", 5);
        let deeper = folder(&mut inv, 6, root, "Deeper");
        item(&mut inv, 102, deeper, "Second indirection", 5);
        let nested = item(&mut inv, 103, target, "Nested link", 25);
        inv.items.get_mut(&nested).unwrap().asset_id = deeper;
        let link = item(&mut inv, 104, base, "Link", 25);
        inv.items.get_mut(&link).unwrap().asset_id = target;
        let back = item(&mut inv, 105, target, "Back", 25);
        inv.items.get_mut(&back).unwrap().asset_id = base;
        let result = collect(&inv, base, true, true, true, &Preferences::default());
        assert_eq!(result.items.len(), 1);
        assert_eq!((result.items[0].item, result.items[0].folder), (direct, base));
        inv.folders.get_mut(&target).unwrap().info.type_default = 47;
        assert!(collect(&inv, base, true, true, true, &Preferences::default()).items.is_empty());
        inv.folders.get_mut(&target).unwrap().info.type_default = -1;
        inv.folders.get_mut(&target).unwrap().info.name = ".hidden".into();
        assert!(collect(&inv, base, true, true, true, &Preferences::default()).items.is_empty());
        inv.folders.get_mut(&target).unwrap().info.name = "External".into();
        inv.folders.get_mut(&target).unwrap().state = FetchState::Unknown;
        assert_eq!(collect(&inv, base, true, true, true, &Preferences::default()).missing, [target]);
    }

    #[test]
    fn rlva_inventory_queries_share_collection_and_find_the_deepest_matching_folder() {
        use super::super::Command;
        use crate::world::World;
        let (mut inv, base) = fixture();
        let root = inv.root;
        let target = folder(&mut inv, 4, root, "External");
        let gesture = item(&mut inv, 100, target, "Gesture", 21);
        let link = item(&mut inv, 101, base, "Link", 25);
        inv.items.get_mut(&link).unwrap().asset_id = target;
        folder(&mut inv, 5, base, "chest");
        let visible = folder(&mut inv, 6, base, "Match");
        folder(&mut inv, 7, visible, "Match deep");
        folder(&mut inv, 8, base, "~Match ignored");
        let hidden = folder(&mut inv, 9, base, ".hidden");
        folder(&mut inv, 10, hidden, "Match visible");
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.inventory = inv;
        world.rlva.configure(true);
        world.active_gestures.insert(gesture);
        let mut query = |text| world.rlva_inventory_query(&Command::parse(text).unwrap(), Uuid::nil()).unwrap();
        assert_eq!(query("getinv:Outfit=123"), "Match,~Match ignored");
        assert_eq!(query("findfolder:Match=123"), "Outfit/Match/Match deep");
        let matches = query("findfolders:&&Match&&=123");
        assert!(matches.contains("Outfit/Match/Match deep") && !matches.contains("~Match"));
        assert!(query("findfolder=123").is_empty());
        assert!(query("findfolder:&&=123").is_empty());
        assert!(query("getinvworn:Outfit=123").starts_with("|30"));
        folder(&mut world.inventory, 12, base, "Slash/name");
        let dot = folder(&mut world.inventory, 13, base, ".");
        folder(&mut world.inventory, 14, dot, "..");
        let mut query = |text| world.rlva_inventory_query(&Command::parse(text).unwrap(), Uuid::nil()).unwrap();
        assert!(!query("getinv:Outfit=123").contains("Slash/name"));
        assert!(query("getinvworn:Outfit=123").contains("Slash/name|00"));
        assert_eq!(query("getinv:Outfit/.=123"), "");
        assert_eq!(
            query("getinvworn:Outfit/./..=123"),
            "|00",
            "dot names are literal inventory children"
        );
        let plan = crate::world::appearance::Action::WearItem {
            item: gesture,
            replace: false,
            point: 0,
        };
        let cof = folder(&mut world.inventory, 11, root, "Current Outfit");
        world.inventory.folders.get_mut(&cof).unwrap().info.type_default = 46;
        assert!(crate::world::appearance::plan(&world.inventory, plan, &Default::default()).is_ok());
    }

    #[test]
    fn rlva_getpath_returns_one_original_path_and_getpathnew_preserves_cof_order_and_duplicates() {
        use super::super::Command;
        use crate::world::World;
        let (mut inv, base) = fixture();
        let root = inv.root;
        let shared = inv.folders[&base].info.parent;
        let second = folder(&mut inv, 8, shared, "Second");
        let external = folder(&mut inv, 9, root, "External");
        let fold = folder(&mut inv, 10, base, ".(chest)");
        let nested = folder(&mut inv, 11, fold, ".(skull)");
        let cof = folder(&mut inv, 12, root, "Current Outfit");
        inv.folders.get_mut(&cof).unwrap().info.type_default = 46;
        for (number, parent) in [(100, second), (101, base), (102, base), (103, external), (104, nested)] {
            let original = item(&mut inv, number, parent, "Shirt", 5);
            inv.items.get_mut(&original).unwrap().flags = 4;
            let link = item(&mut inv, number + 100, cof, "COF link", 24);
            inv.items.get_mut(&link).unwrap().asset_id = original;
        }
        let link = item(&mut inv, 301, base, "Shared link to an external original", 24);
        inv.items.get_mut(&link).unwrap().asset_id = Uuid::from_u128(103);
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for event in crate::demo::events() {
            world.apply(event);
        }
        world.inventory = inv;
        world.rlva_configure(true);
        let mut query = |text| world.rlva_inventory_query(&Command::parse(text).unwrap(), Uuid::nil()).unwrap();
        assert_eq!(query("getpath:shirt=123"), "Second");
        assert_eq!(query("getpathnew:shirt=123"), "Second,Outfit,Outfit,Outfit/.(chest)");
        assert_eq!(query("getpath:Outfit=123"), "", "a folder name is not a getpath context option");
        assert_eq!(query("getpath=123"), "", "an unknown controller has no query path");
        assert!(world.rlva_inventory_force(Uuid::nil(), &Command::parse("attachthis:Outfit=force").unwrap()));
        assert!(
            world.rlva.appearance.is_empty(),
            "relative force options use the same native context parser"
        );
        let object = item(&mut world.inventory, 105, base, "Object", 6);
        let update = crate::demo::outfit_attachment(&aurora_net::AttachRequest {
            item_id: object,
            owner_id: world.agent_id,
            point: 1,
            add: true,
            flags: 0,
            group_mask: 0,
            everyone_mask: 0,
            next_owner_mask: 0,
            name: "Object".into(),
            desc: String::new(),
        });
        world.objects.upsert(world.main_region.unwrap(), update);
        assert_eq!(
            world.rlva_inventory_query(&Command::parse("getpath=123").unwrap(), object),
            Some("Outfit".into())
        );
    }
}
