//! Outfit selection and COF changes, port of LLAppearanceMgr::updateCOF,
//! updateIsDirty and makeNewOutfitLinks (indra/newview/llappearancemgr.cpp,
//! originally LGPL 2.1). Required body parts survive an outfit replacement.

use super::inventory::{FetchState, Inventory};
use aurora_net::inventory::{InvFolder, InvItem};
use aurora_net::outfits::{OutfitLink, OutfitMutation};
use aurora_net::{AttachRequest, NetCommand};
use std::collections::HashSet;
use uuid::Uuid;

pub const FT_OUTFIT: i32 = 47;
pub const FT_MY_OUTFITS: i32 = 48;
pub const WEARABLES: &[&str] = &[
    "Silhouette",
    "Peau",
    "Cheveux",
    "Yeux",
    "Chemise",
    "Pantalon",
    "Chaussures",
    "Chaussettes",
    "Veste",
    "Gants",
    "Débardeur",
    "Caleçon",
    "Jupe",
    "Alpha",
    "Tatouage",
    "Propriétés physiques",
    "Universel",
];

#[derive(Clone, Debug)]
pub enum Action {
    Wear(Uuid, bool),
    Save(Option<String>),
    Remove(Uuid),
    Add(Uuid),
    MoveLayer(Uuid, bool),
    Revert,
}

pub fn system_folder(inv: &Inventory, kind: i32) -> Option<Uuid> {
    inv.folders
        .values()
        .find(|f| f.info.type_default == kind && !f.library)
        .map(|f| f.info.id)
}

pub fn cof(inv: &Inventory) -> Option<Uuid> {
    system_folder(inv, super::outfit::FT_CURRENT_OUTFIT)
}

pub fn base(inv: &Inventory) -> Option<Uuid> {
    let folder = inv.folders.get(&cof(inv)?)?;
    folder.items.iter().filter_map(|id| inv.items.get(id)).find_map(|it| {
        (it.asset_type == 25 && inv.folders.get(&it.asset_id).is_some_and(|f| f.info.type_default == FT_OUTFIT)).then_some(it.asset_id)
    })
}

pub fn folder_links(inv: &Inventory, folder: Uuid) -> Vec<OutfitLink> {
    inv.folders
        .get(&folder)
        .into_iter()
        .flat_map(|f| &f.items)
        .filter_map(|id| inv.items.get(id))
        .filter_map(|it| {
            if matches!(it.asset_type, 24 | 25) {
                Some(OutfitLink {
                    target: it.asset_id,
                    name: it.name.clone(),
                    desc: it.desc.clone(),
                    folder: it.asset_type == 25,
                    inv_type: it.inv_type,
                })
            } else if matches!(it.asset_type, 5 | 6 | 13 | 21) {
                Some(item_link(it))
            } else {
                None
            }
        })
        .collect()
}

fn item_link(it: &InvItem) -> OutfitLink {
    OutfitLink {
        target: it.id,
        name: it.name.clone(),
        desc: it.desc.clone(),
        folder: false,
        inv_type: it.inv_type,
    }
}

pub fn dirty(inv: &Inventory) -> bool {
    let Some((cof, base)) = cof(inv).zip(base(inv)) else {
        return false;
    };
    let keys = |id| {
        let mut keys: Vec<_> = folder_links(inv, id)
            .into_iter()
            .filter(|l| !l.folder && inv.items.get(&l.target).is_some_and(|it| matches!(it.asset_type, 5 | 6 | 13 | 21)))
            .map(|l| (l.target, l.name, l.desc))
            .collect();
        keys.sort();
        keys
    };
    loaded(inv, base) && keys(cof) != keys(base)
}

pub fn loaded(inv: &Inventory, folder: Uuid) -> bool {
    inv.folders.get(&folder).is_some_and(|f| f.state == FetchState::Fetched)
}

pub fn complete(inv: &Inventory, folder: Uuid) -> bool {
    loaded(inv, folder)
        && folder_links(inv, folder)
            .iter()
            .all(|l| l.folder || inv.items.contains_key(&l.target))
}

/// Only the relevant folders and link targets are fetched, including retry.
pub fn prepare(inv: &mut Inventory, agent: Uuid, worn: &HashSet<Uuid>) -> Vec<NetCommand> {
    let mut folders: Vec<_> = [cof(inv), system_folder(inv, FT_MY_OUTFITS), base(inv)]
        .into_iter()
        .flatten()
        .collect();
    if let Some(my) = system_folder(inv, FT_MY_OUTFITS) {
        folders.extend(inv.folders.get(&my).into_iter().flat_map(|f| &f.children).copied());
    }
    let mut missing: HashSet<_> = worn.iter().filter(|id| !inv.items.contains_key(id)).copied().collect();
    for id in &folders {
        inv.request(*id);
        for l in folder_links(inv, *id) {
            if !l.folder && !l.target.is_nil() && !inv.items.contains_key(&l.target) {
                missing.insert(l.target);
            }
        }
    }
    if missing.is_empty() {
        Vec::new()
    } else {
        vec![NetCommand::FetchItems {
            items: missing.into_iter().collect(),
            owner: agent,
        }]
    }
}

fn require_complete(inv: &Inventory, folder: Uuid) -> Result<(), String> {
    if complete(inv, folder) {
        Ok(())
    } else {
        Err("L’inventaire de cette tenue est encore en cours de chargement.".into())
    }
}

fn base_link(inv: &Inventory, id: Uuid) -> OutfitLink {
    OutfitLink {
        target: id,
        name: inv.folders.get(&id).map(|f| f.info.name.clone()).unwrap_or_default(),
        desc: String::new(),
        folder: true,
        inv_type: 8,
    }
}

/// Produces one acknowledged AIS operation. No optimistic inventory mutation.
pub fn plan(inv: &Inventory, action: Action, worn: &HashSet<Uuid>) -> Result<(OutfitMutation, bool), String> {
    let cof = cof(inv).ok_or("Le dossier Tenue actuelle n’est pas disponible.")?;
    require_complete(inv, cof)?;
    let mut current = folder_links(inv, cof);
    if matches!(action, Action::Add(_) | Action::Remove(_) | Action::MoveLayer(..)) {
        for id in worn {
            if !current.iter().any(|l| l.target == *id) {
                let it = inv.items.get(id).ok_or("Un objet porté est encore en cours de chargement.")?;
                current.push(item_link(it));
            }
        }
    }
    let mut change = OutfitMutation {
        folder: cof,
        create: None,
        links: Vec::new(),
        cof: None,
    };
    let original_current = current.clone();
    let mut sync = true;
    match action {
        Action::Save(name) => {
            // Include attachments that arrived before their COF link, as Firestorm does.
            for id in worn {
                if !current.iter().any(|l| l.target == *id) {
                    let it = inv
                        .items
                        .get(id)
                        .ok_or("Un objet porté n’est pas encore chargé dans l’inventaire.")?;
                    current.push(item_link(it));
                }
            }
            let links: Vec<_> = current.iter().filter(|l| !l.folder).cloned().collect();
            if let Some(name) = name {
                let name = name.trim();
                if name.is_empty() || name.chars().count() > 63 {
                    return Err("Choisissez un nom de 1 à 63 caractères.".into());
                }
                let parent = system_folder(inv, FT_MY_OUTFITS).ok_or("Le dossier Mes tenues n’est pas disponible.")?;
                let id = Uuid::new_v4();
                change.folder = id;
                change.create = Some(InvFolder {
                    id,
                    parent,
                    name: name.into(),
                    type_default: FT_OUTFIT,
                    version: 1,
                });
                change.links = links.clone();
                let mut cof_links = links;
                cof_links.push(OutfitLink {
                    target: id,
                    name: name.into(),
                    desc: String::new(),
                    folder: true,
                    inv_type: 8,
                });
                change.cof = Some((cof, cof_links));
            } else {
                change.folder = base(inv).ok_or("Enregistrez d’abord la tenue avec « Enregistrer sous… ».")?;
                require_complete(inv, change.folder)?;
                change.links = links;
                change.links.extend(
                    folder_links(inv, change.folder)
                        .into_iter()
                        .filter(|l| !l.folder && inv.items.get(&l.target).is_some_and(|it| it.asset_type == 0)),
                );
                change.cof = Some((cof, current.clone()));
                sync = false;
            }
        }
        Action::Wear(id, append) => {
            require_complete(inv, id)?;
            let mut next: Vec<_> = folder_links(inv, id)
                .into_iter()
                .filter(|l| !l.folder && inv.items.get(&l.target).is_none_or(|it| it.asset_type != 0))
                .collect();
            if next
                .iter()
                .any(|l| inv.items.get(&l.target).is_none_or(|it| !matches!(it.asset_type, 5 | 6 | 13 | 21)))
            {
                return Err("Cette tenue contient un lien qui ne peut pas être porté.".into());
            }
            for old in current.iter().filter(|l| !l.folder) {
                let preserve = append
                    || inv.items.get(&old.target).is_some_and(|it| {
                        it.asset_type == 13
                            && !next.iter().any(|l| {
                                inv.items
                                    .get(&l.target)
                                    .is_some_and(|n| n.asset_type == 13 && n.flags & 0xff == it.flags & 0xff)
                            })
                    });
                if preserve && !next.iter().any(|l| l.target == old.target) {
                    next.push(old.clone());
                }
            }
            // One body part per type: new body parts replace existing ones even when adding.
            let mut body = HashSet::new();
            let mut targets = HashSet::new();
            next.retain(|l| {
                targets.insert(l.target)
                    && inv
                        .items
                        .get(&l.target)
                        .is_none_or(|it| it.asset_type != 13 || body.insert(it.flags & 0xff))
            });
            if let Some(base) = if append { base(inv) } else { Some(id) } {
                next.push(base_link(inv, base));
            }
            change.links = next;
        }
        Action::Revert => {
            return plan(
                inv,
                Action::Wear(base(inv).ok_or("Aucune tenue enregistrée à rétablir.")?, false),
                worn,
            );
        }
        Action::Remove(id) => {
            let it = inv.items.get(&id).ok_or("Cet élément n’est pas encore chargé.")?;
            if it.asset_type == 13 {
                return Err("Une partie du corps doit être remplacée, elle ne peut pas être enlevée.".into());
            }
            current.retain(|l| l.target != id);
            change.links = current;
        }
        Action::Add(id) => {
            let it = inv.items.get(&id).ok_or("Cet élément n’est pas encore chargé.")?;
            if !matches!(it.asset_type, 5 | 6 | 13) {
                return Err("Cet élément ne peut pas être porté.".into());
            }
            if it.asset_type == 13 {
                current.retain(|l| {
                    l.folder
                        || inv
                            .items
                            .get(&l.target)
                            .is_none_or(|old| old.asset_type != 13 || old.flags & 0xff != it.flags & 0xff)
                });
            }
            if !current.iter().any(|l| l.target == id) {
                current.push(item_link(it));
            }
            change.links = current;
        }
        Action::MoveLayer(id, up) => {
            let it = inv.items.get(&id).ok_or("Cet élément n’est pas encore chargé.")?;
            if it.asset_type != 5 {
                return Err("Seuls les calques de vêtements peuvent être réordonnés.".into());
            }
            let mut layers: Vec<_> = current
                .iter()
                .enumerate()
                .filter(|(_, l)| {
                    inv.items
                        .get(&l.target)
                        .is_some_and(|n| n.asset_type == 5 && n.flags & 0xff == it.flags & 0xff)
                })
                .map(|(i, l)| (i, l.desc.clone(), l.target))
                .collect();
            layers.sort_by(|a, b| a.1.cmp(&b.1));
            if let Some(pos) = layers.iter().position(|l| l.2 == id) {
                let other = if up {
                    pos.checked_sub(1)
                } else {
                    (pos + 1 < layers.len()).then_some(pos + 1)
                };
                if let Some(other) = other {
                    layers.swap(pos, other);
                }
                for (order, (i, _, _)) in layers.iter().enumerate() {
                    current[*i].desc = format!("@{}", (it.flags & 0xff) * 100 + order as u32);
                }
            }
            change.links = current;
        }
    }
    normalize_layers(inv, &mut change.links);
    if let Some((_, links)) = &mut change.cof {
        normalize_layers(inv, links);
    }
    if change.create.is_none()
        && change.folder != cof
        && let Some((_, links)) = &change.cof
    {
        if *links == original_current {
            change.cof = None;
        } else {
            sync = true;
        }
    }
    let count = change
        .links
        .iter()
        .filter(|l| !l.folder && inv.items.get(&l.target).is_some_and(|it| it.asset_type == 6))
        .count();
    if sync && count > 38 {
        return Err("La tenue dépasse la limite de 38 attachements.".into());
    }
    for kind in 4..=16 {
        if sync
            && change
                .links
                .iter()
                .filter(|l| {
                    !l.folder
                        && inv
                            .items
                            .get(&l.target)
                            .is_some_and(|it| it.asset_type == 5 && it.flags & 0xff == kind)
                })
                .count()
                > 60
        {
            return Err("La tenue dépasse la limite de 60 calques par type de vêtement.".into());
        }
    }
    Ok((change, sync))
}

fn normalize_layers(inv: &Inventory, links: &mut [OutfitLink]) {
    for kind in 4..=16 {
        let mut indices: Vec<_> = links
            .iter()
            .enumerate()
            .filter(|(_, l)| {
                !l.folder
                    && inv
                        .items
                        .get(&l.target)
                        .is_some_and(|it| it.asset_type == 5 && it.flags & 0xff == kind)
            })
            .map(|(i, l)| {
                let order = l
                    .desc
                    .strip_prefix('@')
                    .and_then(|s| s.parse::<u32>().ok())
                    .filter(|order| order / 100 == kind)
                    .unwrap_or(u32::MAX);
                (i, order)
            })
            .collect();
        indices.sort_by_key(|(_, order)| *order);
        for (order, (i, _)) in indices.iter().enumerate() {
            links[*i].desc = format!("@{}", kind * 100 + order as u32);
        }
    }
}

pub fn sync_commands(inv: &Inventory, agent: Uuid, worn: &HashSet<Uuid>) -> Vec<NetCommand> {
    let Some(cof) = cof(inv) else {
        return Vec::new();
    };
    let objects: HashSet<_> = folder_links(inv, cof)
        .into_iter()
        .filter(|l| !l.folder && inv.items.get(&l.target).is_some_and(|it| it.asset_type == 6))
        .map(|l| l.target)
        .collect();
    let detach: Vec<_> = worn.difference(&objects).copied().collect();
    let attach: Vec<_> = objects
        .difference(worn)
        .filter_map(|id| inv.items.get(id))
        .map(|it| AttachRequest {
            item_id: it.id,
            owner_id: if it.owner.is_nil() { agent } else { it.owner },
            point: 0,
            add: true,
            flags: it.flags,
            group_mask: it.group_mask,
            everyone_mask: it.everyone_mask,
            next_owner_mask: it.next_owner_mask,
            name: it.name.clone(),
            desc: it.desc.clone(),
        })
        .collect();
    let mut commands = Vec::new();
    if !detach.is_empty() {
        commands.push(NetCommand::DetachAttachments(detach));
    }
    if !attach.is_empty() {
        commands.push(NetCommand::RezAttachments(attach));
    }
    if let Some(folder) = inv.folders.get(&cof) {
        commands.push(NetCommand::RequestServerAppearance {
            cof_version: folder.info.version,
        });
    }
    commands
}

/// Offline fixture; no grid, assets or real inventory are read.
pub fn seed_demo(inv: &mut Inventory, agent: Uuid) {
    use super::inventory::Folder;
    let my = system_folder(inv, FT_MY_OUTFITS).unwrap_or_else(|| Uuid::from_u128(700));
    let cof = Uuid::from_u128(701);
    let outfits: Vec<_> = (702..706).map(Uuid::from_u128).collect();
    let folder = |id, parent, name: &str, kind| Folder {
        info: InvFolder {
            id,
            parent,
            name: name.into(),
            type_default: kind,
            version: 1,
        },
        children: Vec::new(),
        items: Vec::new(),
        state: FetchState::Fetched,
        library: false,
    };
    inv.folders.insert(my, folder(my, inv.root, "Mes tenues", FT_MY_OUTFITS));
    inv.folders.insert(cof, folder(cof, inv.root, "Tenue actuelle", 46));
    for (id, name) in outfits.iter().zip(["Empty", "Flic", "Neurolab", "Veste Cuir"]) {
        inv.folders.insert(*id, folder(*id, my, name, FT_OUTFIT));
    }
    if let Some(f) = inv.folders.get_mut(&my) {
        f.children = outfits.clone();
    }
    let mut items = Vec::new();
    for (i, (name, kind, flags)) in [
        ("Silhouette Aurora", 13, 0),
        ("Peau Aurora", 13, 1),
        ("Cheveux Aurora", 13, 2),
        ("Yeux Aurora", 13, 3),
        ("Tatouage des étoiles", 5, 14),
        ("Veste en cuir", 5, 8),
        ("Pantalon noir", 5, 5),
        ("Cheveux mesh", 6, 0),
        ("Boucles d’oreilles", 6, 0),
        ("Bague argentée", 6, 0),
        ("Bottes", 6, 0),
        ("HUD de démo", 6, 0),
        ("Chemise de rechange", 5, 4),
    ]
    .into_iter()
    .enumerate()
    {
        let id = Uuid::from_u128(710 + i as u128);
        items.push(InvItem {
            id,
            parent: inv.root,
            name: name.into(),
            desc: String::new(),
            asset_type: kind,
            inv_type: if kind == 6 { 6 } else { 18 },
            asset_id: Uuid::nil(),
            flags,
            creator: agent,
            created_at: 0,
            owner: agent,
            group_mask: 0,
            everyone_mask: 0,
            next_owner_mask: 0,
        });
    }
    inv.add_items(items.clone());
    let all: Vec<_> = items.iter().take(12).map(item_link).collect();
    for (i, id) in outfits.iter().enumerate() {
        let selected = match i {
            0 => Vec::new(),
            1 => all.iter().take(7).cloned().collect(),
            2 => all.iter().take(9).cloned().collect(),
            _ => all.iter().take(11).cloned().collect(),
        };
        demo_replace(inv, agent, *id, &selected);
    }
    let mut current = all;
    current.push(base_link(inv, outfits[3]));
    demo_replace(inv, agent, cof, &current);
}

fn demo_replace(inv: &mut Inventory, agent: Uuid, id: Uuid, links: &[OutfitLink]) {
    let old = inv.folders.get(&id).map(|f| f.items.clone()).unwrap_or_default();
    for id in old {
        if inv.items.get(&id).is_some_and(|it| matches!(it.asset_type, 24 | 25)) {
            inv.items.remove(&id);
        }
    }
    let items: Vec<_> = links
        .iter()
        .map(|l| InvItem {
            id: Uuid::new_v4(),
            parent: id,
            name: l.name.clone(),
            desc: l.desc.clone(),
            asset_type: if l.folder { 25 } else { 24 },
            inv_type: l.inv_type,
            asset_id: l.target,
            flags: 0,
            creator: agent,
            created_at: 0,
            owner: agent,
            group_mask: 0,
            everyone_mask: 0,
            next_owner_mask: 0,
        })
        .collect();
    inv.apply(vec![aurora_net::inventory::FolderContents {
        folder_id: id,
        owner_id: agent,
        version: inv.folders.get(&id).map(|f| f.info.version + 1).unwrap_or(1),
        folders: Vec::new(),
        items,
    }]);
}

pub fn demo_mutate(
    inv: &mut Inventory,
    agent: Uuid,
    change: &OutfitMutation,
) -> Result<Vec<aurora_net::inventory::FolderContents>, String> {
    if let Some(cat) = &change.create {
        inv.folders.insert(
            cat.id,
            super::inventory::Folder {
                info: cat.clone(),
                children: Vec::new(),
                items: Vec::new(),
                state: FetchState::Fetched,
                library: false,
            },
        );
        if let Some(parent) = inv.folders.get_mut(&cat.parent) {
            parent.children.push(cat.id);
        }
    }
    demo_replace(inv, agent, change.folder, &change.links);
    if let Some((cof, links)) = &change.cof {
        demo_replace(inv, agent, *cof, links);
    }
    Ok(Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Inventory {
        let mut inv = Inventory::default();
        seed_demo(&mut inv, Uuid::from_u128(1));
        inv
    }
    #[test]
    fn replace_empty_outfit_preserves_required_body_parts() {
        let inv = fixture();
        let (change, sync) = plan(&inv, Action::Wear(Uuid::from_u128(702), false), &HashSet::new()).expect("plan");
        assert!(sync);
        assert_eq!(change.links.iter().filter(|l| !l.folder).count(), 4);
        assert!(
            change
                .links
                .iter()
                .filter(|l| !l.folder)
                .all(|l| inv.items[&l.target].asset_type == 13)
        );
        assert_eq!(change.links.iter().find(|l| l.folder).map(|l| l.target), Some(Uuid::from_u128(702)));
    }
    #[test]
    fn save_as_uses_links_and_replaces_the_base_outfit() {
        let mut inv = fixture();
        let (change, _) = plan(&inv, Action::Save(Some("  Nouvelle tenue  ".into())), &HashSet::new()).expect("save");
        assert_eq!(change.create.as_ref().expect("category").name, "Nouvelle tenue");
        assert!(change.links.iter().all(|l| !l.folder));
        demo_mutate(&mut inv, Uuid::from_u128(1), &change).expect("mutation");
        assert_eq!(base(&inv), Some(change.folder));
        assert!(!dirty(&inv));
    }
    #[test]
    fn body_parts_cannot_be_removed_and_unloaded_outfits_cannot_be_worn() {
        let mut inv = fixture();
        assert!(plan(&inv, Action::Remove(Uuid::from_u128(710)), &HashSet::new()).is_err());
        inv.folders.get_mut(&Uuid::from_u128(702)).expect("outfit").state = FetchState::Unknown;
        assert!(plan(&inv, Action::Wear(Uuid::from_u128(702), false), &HashSet::new()).is_err());
    }
    #[test]
    fn adding_a_body_part_replaces_the_existing_type() {
        let mut inv = fixture();
        let mut shape = inv.items[&Uuid::from_u128(710)].clone();
        shape.id = Uuid::from_u128(999);
        inv.items.insert(shape.id, shape);
        let (change, _) = plan(&inv, Action::Add(Uuid::from_u128(999)), &HashSet::new()).expect("add");
        assert!(change.links.iter().any(|l| l.target == Uuid::from_u128(999)));
        assert!(!change.links.iter().any(|l| l.target == Uuid::from_u128(710)));
    }

    #[test]
    fn saving_updates_clothing_order_in_both_cof_and_base() {
        let mut inv = fixture();
        let (change, sync) = plan(&inv, Action::Save(None), &HashSet::new()).expect("save");
        assert!(sync, "normalizing COF layer order requires a new bake");
        demo_mutate(&mut inv, Uuid::from_u128(1), &change).expect("mutation");
        assert!(!dirty(&inv), "Save must clear the unsaved-changes indicator");
        let links = folder_links(&inv, cof(&inv).expect("COF"));
        assert_eq!(
            links.iter().find(|l| l.target == Uuid::from_u128(715)).expect("jacket").desc,
            "@800"
        );
    }

    #[test]
    fn removing_one_clothing_layer_keeps_unlinked_worn_attachments() {
        let mut inv = fixture();
        let mut extra = inv.items[&Uuid::from_u128(717)].clone();
        extra.id = Uuid::from_u128(1000);
        inv.items.insert(extra.id, extra);
        let (change, _) = plan(&inv, Action::Remove(Uuid::from_u128(715)), &HashSet::from([Uuid::from_u128(1000)])).expect("remove");
        assert!(change.links.iter().any(|l| l.target == Uuid::from_u128(1000)));
        assert!(!change.links.iter().any(|l| l.target == Uuid::from_u128(715)));
    }
}
