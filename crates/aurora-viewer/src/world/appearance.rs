//! Outfit selection and COF changes, port of LLAppearanceMgr::updateCOF,
//! updateIsDirty and makeNewOutfitLinks (indra/newview/llappearancemgr.cpp,
//! originally LGPL 2.1). Required body parts survive an outfit replacement.

use super::inventory::{FetchState, Inventory};
use aurora_net::inventory::{InvFolder, InvItem};
use aurora_net::outfits::{OutfitLink, OutfitMutation};
use aurora_net::{AttachRequest, NetCommand};
use std::collections::{HashMap, HashSet};
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
    WearItem { item: Uuid, replace: bool, point: u8 },
    DeleteFromOutfit { folder: Uuid, item: Uuid },
    ShowOriginal(Uuid),
    Favorite(Uuid),
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
    let mut links: Vec<_> = inv
        .folders
        .get(&folder)
        .into_iter()
        .flat_map(|f| &f.items)
        .filter_map(|id| inv.items.get(id))
        .filter(|it| it.parent == folder)
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
        .collect();
    // LLAppearanceMgr::removeDuplicateItems keeps the last link to each
    // original, preserving its layer order. Separate inventory copies stay.
    let mut seen = HashSet::new();
    links.reverse();
    links.retain(|l| !l.target.is_nil() && seen.insert((l.folder, l.target)));
    links.reverse();
    links
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
pub fn plan(inv: &Inventory, action: Action, worn: &HashMap<Uuid, u8>) -> Result<(OutfitMutation, bool), String> {
    // DeleteFromOutfit removes the saved link, never the original or the worn item.
    if let Action::DeleteFromOutfit { folder, item } = action {
        if !inv
            .folders
            .get(&folder)
            .is_some_and(|f| f.info.type_default == FT_OUTFIT && !f.library)
            || !loaded(inv, folder)
        {
            return Err("Le dossier de tenue n’est pas disponible.".into());
        }
        return Ok((
            OutfitMutation {
                folder,
                create: None,
                links: folder_links(inv, folder).into_iter().filter(|l| l.target != item).collect(),
                cof: None,
            },
            false,
        ));
    }
    let cof = cof(inv).ok_or("Le dossier Tenue actuelle n’est pas disponible.")?;
    require_complete(inv, cof)?;
    let mut current = folder_links(inv, cof);
    if matches!(
        action,
        Action::Add(_) | Action::WearItem { .. } | Action::Remove(_) | Action::MoveLayer(..)
    ) {
        for id in worn.keys() {
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
            for id in worn.keys() {
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
        Action::Add(id) | Action::WearItem { item: id, .. } => {
            let it = inv.items.get(&id).ok_or("Cet élément n’est pas encore chargé.")?;
            if !matches!(it.asset_type, 5 | 6 | 13) {
                return Err("Cet élément ne peut pas être porté.".into());
            }
            let (replace, point) = match action {
                Action::WearItem { replace, point, .. } => (replace, point),
                _ => (false, 0),
            };
            if it.asset_type == 13 || (replace && it.asset_type == 5) {
                current.retain(|l| {
                    l.folder
                        || inv
                            .items
                            .get(&l.target)
                            .is_none_or(|old| old.asset_type != it.asset_type || old.flags & 0xff != it.flags & 0xff)
                });
            }
            if replace && it.asset_type == 6 {
                // Point zero uses the last saved point (LLObjectBridge::mAttachPt).
                let point = if point == 0 { (it.flags & 0xff) as u8 } else { point };
                if point != 0 {
                    current.retain(|l| {
                        l.folder
                            || l.target == id
                            || inv.items.get(&l.target).is_none_or(|old| {
                                old.asset_type != 6 || worn.get(&l.target).copied().unwrap_or((old.flags & 0xff) as u8) != point
                            })
                    });
                }
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
        Action::ShowOriginal(_) | Action::Favorite(_) | Action::DeleteFromOutfit { .. } => {
            return Err("Cette action ne modifie pas la tenue actuelle.".into());
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

/// Explicit attachment requests retain the chosen point and replace/add mode.
/// They are sent only after the COF update has been acknowledged.
pub fn attachment_for_action(inv: &Inventory, agent: Uuid, action: &Action) -> Option<AttachRequest> {
    let Action::WearItem { item, point, replace } = action else {
        return None;
    };
    let it = inv.items.get(item).filter(|it| it.asset_type == 6)?;
    Some(AttachRequest {
        item_id: it.id,
        owner_id: if it.owner.is_nil() { agent } else { it.owner },
        point: *point,
        add: !replace,
        flags: it.flags,
        group_mask: it.group_mask,
        everyone_mask: it.everyone_mask,
        next_owner_mask: it.next_owner_mask,
        name: it.name.clone(),
        desc: it.desc.clone(),
    })
}

pub fn apply_attachment_override(commands: &mut Vec<NetCommand>, attachment: AttachRequest) {
    let mut attachment = Some(attachment);
    for cmd in commands.iter_mut() {
        if let NetCommand::RezAttachments(items) = cmd
            && let Some(attachment) = attachment.take()
        {
            items.retain(|it| it.item_id != attachment.item_id);
            items.push(attachment);
        }
    }
    if let Some(attachment) = attachment {
        let pos = commands
            .iter()
            .position(|cmd| matches!(cmd, NetCommand::RequestServerAppearance { .. }))
            .unwrap_or(commands.len());
        commands.insert(pos, NetCommand::RezAttachments(vec![attachment]));
    }
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
        ("Accessoire de rechange", 6, 5),
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
            favorite: false,
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
        let mut selected = match i {
            0 => Vec::new(),
            1 => all.iter().take(7).cloned().collect(),
            2 => all.iter().take(9).cloned().collect(),
            _ => all.iter().take(11).cloned().collect(),
        };
        if i == 2
            && let Some(item) = items.last()
        {
            selected.push(item_link(item));
        }
        demo_replace(inv, agent, *id, &selected);
    }
    let mut current = all;
    current.push(base_link(inv, outfits[3]));
    demo_replace(inv, agent, cof, &current);
}

/// Replay a folder response with repeated links and originals from other parents.
pub fn seed_duplicate_response(inv: &mut Inventory, agent: Uuid) {
    for folder in [Uuid::from_u128(704), Uuid::from_u128(701)] {
        let links: Vec<_> = inv
            .folders
            .get(&folder)
            .into_iter()
            .flat_map(|f| &f.items)
            .filter_map(|id| inv.items.get(id))
            .cloned()
            .collect();
        let mut items = links.clone();
        for mut link in links {
            if let Some(original) = inv.items.get(&link.asset_id) {
                items.push(original.clone());
            }
            link.id = Uuid::new_v4();
            items.push(link);
        }
        inv.apply(vec![aurora_net::inventory::FolderContents {
            folder_id: folder,
            owner_id: agent,
            version: 3,
            folders: Vec::new(),
            items,
        }]);
    }
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
            favorite: false,
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
    fn adding_objects_or_clothes_counts_each_original_once() {
        let mut inv = fixture();
        let cof = cof(&inv).expect("COF");
        let mut links = folder_links(&inv, cof);
        for i in 0..20 {
            let mut item = inv.items[&Uuid::from_u128(717)].clone();
            item.id = Uuid::from_u128(1000 + i);
            inv.items.insert(item.id, item.clone());
            links.extend([item_link(&item), item_link(&item)]);
        }
        demo_replace(&mut inv, Uuid::from_u128(1), cof, &links);
        for item in [Uuid::from_u128(722), Uuid::from_u128(723)] {
            let (change, sync) = plan(
                &inv,
                Action::WearItem {
                    item,
                    replace: false,
                    point: 0,
                },
                &HashMap::new(),
            )
            .expect("duplicate links must not exhaust the attachment limit");
            assert!(sync);
            assert_eq!(change.links.iter().filter(|l| l.target == item).count(), 1);
            let mut seen = HashSet::new();
            assert!(change.links.iter().all(|l| seen.insert((l.folder, l.target))));
            for i in 0..20 {
                assert_eq!(
                    change.links.iter().filter(|l| l.target == Uuid::from_u128(1000 + i)).count(),
                    1,
                    "distinct inventory copies must remain"
                );
            }
        }
    }

    #[test]
    fn duplicate_links_keep_the_last_layer_and_ignore_foreign_originals() {
        let mut inv = fixture();
        let folder = Uuid::from_u128(704);
        let original = inv.items[&Uuid::from_u128(715)].clone();
        let mut children = vec![original.id]; // index left over from an older cache
        for (id, desc) in [(990, "@800"), (991, "@801")] {
            let mut link = original.clone();
            link.id = Uuid::from_u128(id);
            link.parent = folder;
            link.asset_type = 24;
            link.asset_id = original.id;
            link.desc = desc.into();
            children.push(link.id);
            inv.items.insert(link.id, link);
        }
        let mut copy = original.clone();
        copy.id = Uuid::from_u128(992);
        copy.parent = folder;
        children.push(copy.id);
        inv.items.insert(copy.id, copy);
        inv.folders.get_mut(&folder).expect("outfit").items = children;
        let links = folder_links(&inv, folder);
        assert_eq!(links.len(), 2, "a separate inventory copy stays");
        assert_eq!(links.iter().find(|l| l.target == original.id).expect("link").desc, "@801");
    }
    #[test]
    fn replace_empty_outfit_preserves_required_body_parts() {
        let inv = fixture();
        let (change, sync) = plan(&inv, Action::Wear(Uuid::from_u128(702), false), &HashMap::new()).expect("plan");
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
        let (change, _) = plan(&inv, Action::Save(Some("  Nouvelle tenue  ".into())), &HashMap::new()).expect("save");
        assert_eq!(change.create.as_ref().expect("category").name, "Nouvelle tenue");
        assert!(change.links.iter().all(|l| !l.folder));
        demo_mutate(&mut inv, Uuid::from_u128(1), &change).expect("mutation");
        assert_eq!(base(&inv), Some(change.folder));
        assert!(!dirty(&inv));
    }
    #[test]
    fn body_parts_cannot_be_removed_and_unloaded_outfits_cannot_be_worn() {
        let mut inv = fixture();
        assert!(plan(&inv, Action::Remove(Uuid::from_u128(710)), &HashMap::new()).is_err());
        inv.folders.get_mut(&Uuid::from_u128(702)).expect("outfit").state = FetchState::Unknown;
        assert!(plan(&inv, Action::Wear(Uuid::from_u128(702), false), &HashMap::new()).is_err());
    }
    #[test]
    fn adding_a_body_part_replaces_the_existing_type() {
        let mut inv = fixture();
        let mut shape = inv.items[&Uuid::from_u128(710)].clone();
        shape.id = Uuid::from_u128(999);
        inv.items.insert(shape.id, shape);
        let (change, _) = plan(&inv, Action::Add(Uuid::from_u128(999)), &HashMap::new()).expect("add");
        assert!(change.links.iter().any(|l| l.target == Uuid::from_u128(999)));
        assert!(!change.links.iter().any(|l| l.target == Uuid::from_u128(710)));
    }

    #[test]
    fn saving_updates_clothing_order_in_both_cof_and_base() {
        let mut inv = fixture();
        let (change, sync) = plan(&inv, Action::Save(None), &HashMap::new()).expect("save");
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
        let (change, _) = plan(
            &inv,
            Action::Remove(Uuid::from_u128(715)),
            &HashMap::from([(Uuid::from_u128(1000), 5)]),
        )
        .expect("remove");
        assert!(change.links.iter().any(|l| l.target == Uuid::from_u128(1000)));
        assert!(!change.links.iter().any(|l| l.target == Uuid::from_u128(715)));
    }

    #[test]
    fn wear_replaces_same_clothing_type_while_add_preserves_layers() {
        let mut inv = fixture();
        let mut jacket = inv.items[&Uuid::from_u128(715)].clone();
        jacket.id = Uuid::from_u128(999);
        inv.items.insert(jacket.id, jacket);
        for replace in [false, true] {
            let (change, _) = plan(
                &inv,
                Action::WearItem {
                    item: Uuid::from_u128(999),
                    replace,
                    point: 0,
                },
                &HashMap::new(),
            )
            .expect("wear");
            assert_eq!(change.links.iter().any(|l| l.target == Uuid::from_u128(715)), !replace);
            assert!(change.links.iter().any(|l| l.target == Uuid::from_u128(999)));
            assert!(change.links.iter().any(|l| l.target == Uuid::from_u128(710)), "body preserved");
        }
    }

    #[test]
    fn deleting_saved_link_preserves_original_and_current_outfit() {
        let mut inv = fixture();
        let current = folder_links(&inv, cof(&inv).expect("COF"));
        let item = Uuid::from_u128(717);
        let folder = Uuid::from_u128(704);
        let (change, sync) = plan(&inv, Action::DeleteFromOutfit { folder, item }, &HashMap::new()).expect("delete link");
        assert!(!sync, "saved-outfit removal must not detach worn objects");
        demo_mutate(&mut inv, Uuid::from_u128(1), &change).expect("mutation");
        assert!(!folder_links(&inv, folder).iter().any(|l| l.target == item));
        assert!(inv.items.contains_key(&item), "original retained");
        assert_eq!(folder_links(&inv, cof(&inv).expect("COF")), current);
    }

    #[test]
    fn attach_to_hud_keeps_other_objects_and_passes_the_chosen_point() {
        let inv = fixture();
        let agent = Uuid::from_u128(1);
        let item = Uuid::from_u128(723);
        let action = Action::WearItem {
            item,
            replace: false,
            point: 35,
        };
        let (change, _) = plan(&inv, action.clone(), &HashMap::new()).expect("add to HUD");
        assert!(change.links.iter().any(|l| l.target == Uuid::from_u128(717)));
        let attachment = attachment_for_action(&inv, agent, &action).expect("attachment");
        assert_eq!(attachment.point, 35);
        assert!(attachment.add);
        let duplicate = attachment.clone();
        let mut commands = vec![
            NetCommand::DetachAttachments(vec![Uuid::from_u128(999)]),
            NetCommand::RezAttachments(vec![duplicate]),
            NetCommand::RequestServerAppearance { cof_version: 1 },
        ];
        apply_attachment_override(&mut commands, attachment);
        assert!(matches!(&commands[0], NetCommand::DetachAttachments(_)));
        assert!(
            matches!(&commands[1], NetCommand::RezAttachments(items) if items.len() == 1 && items[0].item_id == item && items[0].point == 35)
        );
        assert!(matches!(commands.last(), Some(NetCommand::RequestServerAppearance { .. })));
    }

    #[test]
    fn replace_attachment_uses_actual_point_when_inventory_flags_are_stale() {
        let inv = fixture();
        let worn = HashMap::from([(Uuid::from_u128(717), 5), (Uuid::from_u128(718), 6)]);
        let (change, _) = plan(
            &inv,
            Action::WearItem {
                item: Uuid::from_u128(723),
                replace: true,
                point: 0,
            },
            &worn,
        )
        .expect("replace point 5");
        assert!(!change.links.iter().any(|l| l.target == Uuid::from_u128(717)));
        assert!(change.links.iter().any(|l| l.target == Uuid::from_u128(718)));
        assert!(change.links.iter().any(|l| l.target == Uuid::from_u128(723)));
    }
}
