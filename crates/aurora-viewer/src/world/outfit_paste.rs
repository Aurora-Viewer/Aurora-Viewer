//! What a paste does in the outfit folders: port of the outfit branches of
//! LLFolderBridge::pasteFromClipboard, with can_move_to_outfit,
//! can_move_to_my_outfits_as_outfit, can_move_to_my_outfits_as_subfolder,
//! dropToOutfit and dropToMyOutfits (indra/newview/llinventorybridge.cpp and
//! llinventoryfunctions.cpp, originally LGPL 2.1). The inventory and
//! Appearance share these rules, and the clipboard they apply to.
//!
//! - Into an outfit: items become links to their originals; a cut link or
//!   item that sits in another outfit moves instead (the Legacy Viewer links
//!   it and leaves the first outfit unchanged, so cut and paste could not
//!   move an item between outfits).
//! - Into the Current Outfit: the items are worn, each replacing what is on
//!   its layer or point (wearItemOnAvatar with replace).
//! - Into My Outfits or one of its groups: a folder of wearables becomes a
//!   new outfit linking its items; a folder of folders comes as a group; a
//!   cut outfit or group moves there (the Legacy Viewer makes a new outfit
//!   of a cut outfit too, and leaves the old one).

use super::appearance::{FT_MY_OUTFITS, FT_OUTFIT, coalesced, cof, folder_links};
use super::inventory::{Inventory, actions};
use super::outfit_folders;
use aurora_net::inventory::operations::{Mutation, Operation};
use aurora_net::outfits::OutfitLink;
use std::collections::HashSet;
use uuid::Uuid;

/// WearFolderLimit (app_settings/settings.xml): the most items a folder may
/// hold to become an outfit.
const WEAR_FOLDER_LIMIT: usize = 125;

/// The most folders a group may hold (can_move_to_my_outfits_as_subfolder).
const GROUP_CONTENT_LIMIT: usize = 255;

#[derive(Debug)]
pub enum Paste {
    /// Not an outfit folder: the inventory's own rules apply.
    Inventory,
    Edit(Mutation),
    /// The originals to wear, in clipboard order.
    Wear(Vec<Uuid>),
}

/// Where `to` stands among the outfit folders.
enum Target {
    Current,
    Outfit,
    Container,
}

fn target(inv: &Inventory, to: Uuid) -> Option<Target> {
    if Some(to) == cof(inv) {
        return Some(Target::Current);
    }
    let folder = inv.folders.get(&to)?;
    if folder.library {
        return None;
    }
    if folder.info.type_default == FT_OUTFIT {
        Some(Target::Outfit)
    } else if outfit_folders::container(inv, to) {
        Some(Target::Container)
    } else {
        None
    }
}

/// can_move_to_outfit: what an outfit or the Current Outfit may hold.
fn wearable(inv: &Inventory, id: Uuid, current: bool) -> bool {
    inv.items.get(&id).is_some_and(|it| {
        // IT_OBJECT, IT_ATTACHMENT, IT_WEARABLE, IT_GESTURE; the Legacy
        // Viewer lets textures and snapshots through only to refuse them in
        // dropToOutfit ("ThumbnailOutfitPhoto").
        matches!(it.inv_type, 6 | 17 | 18 | 20) && !matches!(it.asset_type, 24 | 25) && !coalesced(it)
    }) && !(current && worn_in(inv, id))
}

fn worn_in(inv: &Inventory, id: Uuid) -> bool {
    cof(inv).is_some_and(|cof| folder_links(inv, cof).iter().any(|l| !l.folder && l.target == id))
}

fn link_to(inv: &Inventory, id: Uuid) -> Option<OutfitLink> {
    let it = inv.items.get(&id)?;
    Some(OutfitLink {
        target: id,
        name: it.name.clone(),
        desc: it.desc.clone(),
        folder: false,
        inv_type: it.inv_type,
    })
}

/// The plan of a paste of `ids` (cut or copied) into `to`.
pub fn plan(inv: &Inventory, ids: &[Uuid], to: Uuid, cut: bool, protected: &HashSet<Uuid>) -> Result<Paste, String> {
    let refused = || aurora_i18n::tr!("world-outfit-paste-cannot-be-used-in-outfits").to_string();
    let Some(target) = target(inv, to) else {
        // A folder inside an outfit, or one My Outfits does not show as a
        // group (myoutfit_object_subfolder_type): nothing pastes there.
        return if actions::in_type(inv, to, FT_MY_OUTFITS) {
            Err(refused())
        } else {
            Ok(Paste::Inventory)
        };
    };
    if ids.is_empty() {
        return Err(aurora_i18n::tr!("world-inventory-actions-the-clipboard-is-empty-or-the-folder-is-protected").into());
    }
    let ids = actions::roots(inv, ids);
    match target {
        Target::Current => {
            let mut wear = Vec::new();
            for id in ids {
                let original = actions::original(inv, id).ok_or(aurora_i18n::tr!("common-the-link-is-broken"))?;
                if !wearable(inv, original, false) {
                    return Err(refused());
                }
                // Already worn: skipped, where the Legacy Viewer warns.
                if !worn_in(inv, original) && !wear.contains(&original) {
                    wear.push(original);
                }
            }
            if wear.is_empty() {
                return Err(aurora_i18n::tr!("world-outfit-paste-already-worn").into());
            }
            Ok(Paste::Wear(wear))
        }
        Target::Outfit => into_outfit(inv, &ids, to, cut, protected).map(Paste::Edit),
        Target::Container => into_container(inv, &ids, to, cut, protected).map(Paste::Edit),
    }
}

fn outfit_folder(inv: &Inventory, id: Uuid) -> bool {
    inv.folders.get(&id).is_some_and(|f| f.info.type_default == FT_OUTFIT && !f.library)
}

fn into_outfit(inv: &Inventory, ids: &[Uuid], to: Uuid, cut: bool, protected: &HashSet<Uuid>) -> Result<Mutation, String> {
    let mut held: HashSet<_> = folder_links(inv, to).into_iter().filter(|l| !l.folder).map(|l| l.target).collect();
    let mut change = Mutation::default();
    let mut links = Vec::new();
    for &id in ids {
        if inv.folders.contains_key(&id) {
            return Err(aurora_i18n::tr!("world-outfit-paste-cannot-be-used-in-outfits").into());
        }
        let original = actions::original(inv, id).ok_or(aurora_i18n::tr!("common-the-link-is-broken"))?;
        if !wearable(inv, original, false) {
            return Err(aurora_i18n::tr!("world-outfit-paste-cannot-be-used-in-outfits").into());
        }
        if !held.insert(original) {
            continue;
        }
        let from = actions::parent(inv, id);
        if cut && from.is_some_and(|from| from != to && outfit_folder(inv, from)) && actions::relocatable(inv, id, protected) {
            change.operations.push(Operation::Patch {
                id,
                folder: false,
                body: aurora_llsd::llsd_map! { "parent_id" => to },
            });
            change.refresh.extend(from);
        } else if let Some(link) = link_to(inv, original) {
            links.push(link);
        }
    }
    if !links.is_empty() {
        change.operations.push(Operation::Links { parent: to, links });
    }
    if change.operations.is_empty() {
        return Err(aurora_i18n::tr!("world-outfit-paste-already-in-outfit").into());
    }
    change.refresh.push(to);
    change.refresh.sort();
    change.refresh.dedup();
    Ok(change)
}

/// can_move_to_my_outfits_as_outfit: the links of the outfit a folder
/// makes, or `None` when it cannot be one.
fn as_outfit(inv: &Inventory, id: Uuid) -> Option<Vec<OutfitLink>> {
    let f = inv.folders.get(&id)?;
    if f.state != super::inventory::FetchState::Fetched
        || !f.children.is_empty()
        || f.items.len() > WEAR_FOLDER_LIMIT
        || (f.items.is_empty() && f.info.type_default != FT_OUTFIT)
    {
        return None;
    }
    let mut links = Vec::new();
    for item in &f.items {
        let original = actions::original(inv, *item)?;
        if !wearable(inv, original, false) {
            return None;
        }
        if !links.iter().any(|l: &OutfitLink| l.target == original) {
            links.push(link_to(inv, original)?);
        }
    }
    Some(links)
}

/// can_move_to_my_outfits_as_subfolder: the levels of groups a plain folder
/// of folders brings (1 when it holds outfits only), or `None` when it
/// cannot be a group.
fn as_group(inv: &Inventory, id: Uuid, level: usize) -> Option<usize> {
    let f = inv.folders.get(&id)?;
    if f.state != super::inventory::FetchState::Fetched
        || !f.items.is_empty()
        || f.info.type_default != -1
        || f.children.len() > GROUP_CONTENT_LIMIT
        || level >= outfit_folders::MAX_DEPTH
    {
        return None;
    }
    let mut height = 1;
    for child in &f.children {
        if !outfit_folder(inv, *child) {
            height = height.max(1 + as_group(inv, *child, level + 1)?);
        }
    }
    Some(height)
}

fn into_container(inv: &Inventory, ids: &[Uuid], to: Uuid, cut: bool, protected: &HashSet<Uuid>) -> Result<Mutation, String> {
    let depth = outfit_folders::depth(inv, to).ok_or(aurora_i18n::tr!("world-outfit-folders-cannot-move-here"))?;
    let mut change = Mutation {
        defer_folder_contents: true,
        ..Default::default()
    };
    for &id in ids {
        let Some(f) = inv.folders.get(&id) else {
            return Err(aurora_i18n::tr!("world-outfit-paste-cannot-be-used-in-outfits").into());
        };
        if id == to || actions::under(inv, to, id) {
            return Err(aurora_i18n::tr!("world-inventory-actions-a-folder-cannot-be-copied-into-itself").into());
        }
        let group = as_group(inv, id, 0);
        let fits = |height: usize| depth + height <= outfit_folders::MAX_DEPTH;
        let movable = cut && actions::relocatable(inv, id, protected);
        if movable && (outfit_folder(inv, id) || group.is_some_and(fits)) {
            if f.info.parent != to {
                change.operations.push(Operation::Patch {
                    id,
                    folder: true,
                    body: aurora_llsd::llsd_map! { "parent_id" => to },
                });
                change.refresh.extend([f.info.parent, to]);
            }
        } else if let Some(links) = as_outfit(inv, id) {
            // dropToMyOutfits: a new outfit of the folder's name, linking
            // what it holds; the folder itself stays.
            let new = Uuid::new_v4();
            change.operations.push(Operation::Folder {
                id: new,
                parent: to,
                name: f.info.name.clone(),
                type_default: FT_OUTFIT,
            });
            if !links.is_empty() {
                change.operations.push(Operation::Links { parent: new, links });
            }
            change.refresh.extend([to, new]);
        } else if let Some(height) = group {
            if !fits(height) {
                return Err(aurora_i18n::tr!("world-outfit-folders-too-deep").into());
            }
            // copy_inventory_category: its outfits stay outfits.
            let copy = actions::copy_tree(inv, id, to)?;
            change.operations.extend(copy.operations);
            change.copies.extend(copy.copies);
            change.refresh.extend(copy.refresh);
        } else {
            return Err(aurora_i18n::tr!("world-outfit-paste-cannot-be-used-in-outfits").into());
        }
    }
    if change.operations.is_empty() && change.copies.is_empty() {
        return Err(aurora_i18n::tr!("world-outfit-folders-already-there").into());
    }
    change.refresh.sort();
    change.refresh.dedup();
    Ok(change)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_net::inventory::InvItem;

    const COP: u128 = 703;
    const NEUROLAB: u128 = 704;
    const SPARE_SHIRT: u128 = 722;
    const SPARE_ATTACHMENT: u128 = 723;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    /// The demo wardrobe: outfits 702 to 705 in My Outfits (700), the
    /// Current Outfit (701) wearing 710 to 721, the groups of `outfit_folders`.
    fn demo() -> Inventory {
        let mut inv = Inventory::default();
        super::super::appearance::seed_demo(&mut inv, id(1));
        outfit_folders::seed_demo(&mut inv, id(1));
        inv
    }

    fn plan(inv: &Inventory, ids: &[u128], to: u128, cut: bool) -> Result<Paste, String> {
        super::plan(inv, &ids.iter().map(|n| id(*n)).collect::<Vec<_>>(), id(to), cut, &HashSet::new())
    }

    fn edit(paste: Result<Paste, String>) -> Mutation {
        match paste {
            Ok(Paste::Edit(change)) => change,
            other => panic!("expected an edit, got {other:?}"),
        }
    }

    fn link_in(inv: &Inventory, folder: u128, target: u128) -> Uuid {
        inv.folders[&id(folder)]
            .items
            .iter()
            .copied()
            .find(|item| inv.items[item].asset_id == id(target))
            .expect("link to the target")
    }

    fn links(change: &Mutation) -> Vec<(Uuid, Vec<Uuid>)> {
        change
            .operations
            .iter()
            .filter_map(|op| match op {
                Operation::Links { parent, links } => Some((*parent, links.iter().map(|l| l.target).collect())),
                _ => None,
            })
            .collect()
    }

    fn moves(change: &Mutation) -> Vec<Uuid> {
        change
            .operations
            .iter()
            .filter_map(|op| match op {
                Operation::Patch { id, .. } => Some(*id),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn items_pasted_into_an_outfit_become_links_and_a_cut_link_moves() {
        let inv = demo();
        // dropToOutfit: a copied original, and a copied link, link the original.
        let change = edit(plan(&inv, &[SPARE_SHIRT], COP, false));
        assert_eq!(links(&change), vec![(id(COP), vec![id(SPARE_SHIRT)])]);
        let neurolab_link = link_in(&inv, NEUROLAB, SPARE_ATTACHMENT);
        let change = edit(super::plan(&inv, &[neurolab_link], id(COP), false, &HashSet::new()));
        assert_eq!(links(&change), vec![(id(COP), vec![id(SPARE_ATTACHMENT)])]);
        // A cut original stays where it is, and is linked.
        let change = edit(plan(&inv, &[SPARE_SHIRT], COP, true));
        assert!(moves(&change).is_empty());
        assert_eq!(links(&change).len(), 1);
        // A cut link of another outfit moves to this one.
        let change = edit(super::plan(&inv, &[neurolab_link], id(COP), true, &HashSet::new()));
        assert_eq!(moves(&change), vec![neurolab_link]);
        assert!(links(&change).is_empty());
        assert!(change.refresh.contains(&id(NEUROLAB)) && change.refresh.contains(&id(COP)));
        // What the outfit holds already is not linked twice.
        assert_eq!(
            plan(&inv, &[710], COP, false).unwrap_err(),
            aurora_i18n::tr!("world-outfit-paste-already-in-outfit")
        );
        let change = edit(plan(&inv, &[710, SPARE_SHIRT], COP, false));
        assert_eq!(links(&change), vec![(id(COP), vec![id(SPARE_SHIRT)])]);
    }

    #[test]
    fn an_outfit_refuses_folders_and_what_cannot_be_worn() {
        let mut inv = demo();
        inv.add_items(vec![InvItem {
            id: id(800),
            parent: inv.root,
            name: "Snapshot".into(),
            asset_type: 0,
            inv_type: 0,
            ..Default::default()
        }]);
        let refused = aurora_i18n::tr!("world-outfit-paste-cannot-be-used-in-outfits");
        assert_eq!(plan(&inv, &[800], COP, false).unwrap_err(), refused);
        assert_eq!(plan(&inv, &[NEUROLAB], COP, false).unwrap_err(), refused);
        // Nor does an item go into My Outfits or a group itself.
        assert_eq!(plan(&inv, &[SPARE_SHIRT], 700, false).unwrap_err(), refused);
        assert_eq!(plan(&inv, &[SPARE_SHIRT], 740, false).unwrap_err(), refused);
    }

    #[test]
    fn a_paste_into_the_current_outfit_wears_what_is_not_worn() {
        let inv = demo();
        match plan(&inv, &[SPARE_ATTACHMENT, 710, SPARE_SHIRT], 701, true) {
            Ok(Paste::Wear(items)) => assert_eq!(items, vec![id(SPARE_ATTACHMENT), id(SPARE_SHIRT)]),
            other => panic!("expected a wear, got {other:?}"),
        }
        // A link wears its original.
        let link = link_in(&inv, NEUROLAB, SPARE_ATTACHMENT);
        assert!(matches!(
            super::plan(&inv, &[link], id(701), false, &HashSet::new()),
            Ok(Paste::Wear(items)) if items == vec![id(SPARE_ATTACHMENT)]
        ));
        assert_eq!(
            plan(&inv, &[710], 701, false).unwrap_err(),
            aurora_i18n::tr!("world-outfit-paste-already-worn")
        );
        assert!(plan(&inv, &[COP], 701, false).is_err());
    }

    #[test]
    fn my_outfits_takes_outfits_as_new_outfits_and_moves_what_is_cut() {
        let inv = demo();
        // A copied outfit comes as a new outfit linking the same originals.
        let change = edit(plan(&inv, &[COP], 740, false));
        let Some(Operation::Folder {
            id: new,
            parent,
            name,
            type_default,
        }) = change.operations.first().cloned()
        else {
            panic!("a new outfit first");
        };
        assert_eq!((parent, type_default), (id(740), FT_OUTFIT));
        assert_eq!(name, inv.folders[&id(COP)].info.name);
        let targets: Vec<_> = folder_links(&inv, id(COP)).into_iter().map(|l| l.target).collect();
        assert_eq!(links(&change), vec![(new, targets)]);
        // Cut outfits and groups move, deeper levels included.
        assert_eq!(moves(&edit(plan(&inv, &[744], 700, true))), vec![id(744)]);
        assert_eq!(moves(&edit(plan(&inv, &[742], 740, true))), vec![id(742)]);
        assert_eq!(
            plan(&inv, &[COP], 700, true).unwrap_err(),
            aurora_i18n::tr!("world-outfit-folders-already-there")
        );
        // A group cannot go into itself.
        assert!(plan(&inv, &[741], 742, true).is_err());
    }

    #[test]
    fn a_copied_group_keeps_its_outfits_and_a_plain_folder_of_wearables_becomes_one() {
        let mut inv = demo();
        let change = edit(plan(&inv, &[742], 743, false));
        let folders: Vec<_> = change
            .operations
            .iter()
            .filter_map(|op| match op {
                Operation::Folder { name, type_default, .. } => Some((name.clone(), *type_default)),
                _ => None,
            })
            .collect();
        assert_eq!(folders, vec![("Night".to_string(), -1), ("Beach Night".to_string(), FT_OUTFIT)]);

        // can_move_to_my_outfits_as_outfit: a folder of wearables.
        let folder = id(810);
        inv.created_folders(vec![aurora_net::inventory::InvFolder {
            id: folder,
            parent: inv.root,
            name: "Party".into(),
            ..Default::default()
        }]);
        if let Some(f) = inv.folders.get_mut(&folder) {
            f.state = super::super::inventory::FetchState::Fetched;
        }
        let mut moved = Vec::new();
        for item in [SPARE_SHIRT, SPARE_ATTACHMENT] {
            let mut it = inv.items[&id(item)].clone();
            it.parent = folder;
            moved.push(it);
        }
        inv.add_items(moved);
        for cut in [false, true] {
            let change = edit(plan(&inv, &[810], 700, cut));
            assert!(moves(&change).is_empty(), "the folder itself stays");
            assert!(matches!(
                change.operations.first(),
                Some(Operation::Folder { type_default: FT_OUTFIT, name, .. }) if name == "Party"
            ));
            assert_eq!(links(&change)[0].1.len(), 2);
        }
    }

    #[test]
    fn other_folders_keep_the_inventory_rules() {
        let inv = demo();
        assert!(matches!(
            super::plan(&inv, &[id(SPARE_SHIRT)], inv.root, false, &HashSet::new()),
            Ok(Paste::Inventory)
        ));
    }
}
