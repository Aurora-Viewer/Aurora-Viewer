//! Shared inventory paths and wear locks, following RlvInventory,
//! RlvAttachmentLocks and RlvWearableLocks (rlvinventory.cpp, rlvlocks.cpp).

use super::super::{World, appearance, inventory::Inventory};
use super::Command;
use aurora_net::outfits::OutfitLink;
use std::collections::HashSet;
use uuid::Uuid;

pub const LAYERS: &[&str] = &[
    "shape",
    "skin",
    "hair",
    "eyes",
    "shirt",
    "pants",
    "shoes",
    "socks",
    "jacket",
    "gloves",
    "undershirt",
    "underpants",
    "skirt",
    "alpha",
    "tattoo",
    "physics",
    "universal",
];
pub const POINTS: &[&str] = &[
    "",
    "chest",
    "skull",
    "left shoulder",
    "right shoulder",
    "left hand",
    "right hand",
    "left foot",
    "right foot",
    "spine",
    "pelvis",
    "mouth",
    "chin",
    "left ear",
    "right ear",
    "left eyeball",
    "right eyeball",
    "nose",
    "r upper arm",
    "r forearm",
    "l upper arm",
    "l forearm",
    "right hip",
    "r upper leg",
    "r lower leg",
    "left hip",
    "l upper leg",
    "l lower leg",
    "stomach",
    "left pec",
    "right pec",
    "center 2",
    "top right",
    "top",
    "top left",
    "center",
    "bottom left",
    "bottom",
    "bottom right",
    "neck",
    "avatar center",
    "left ring finger",
    "right ring finger",
    "tail base",
    "tail tip",
    "left wing",
    "right wing",
    "jaw",
    "alt left ear",
    "alt right ear",
    "alt left eye",
    "alt right eye",
    "tongue",
    "groin",
    "hind left foot",
    "hind right foot",
];

pub fn point(name: &str) -> Option<u8> {
    POINTS
        .iter()
        .position(|p| !p.is_empty() && p.eq_ignore_ascii_case(name))
        .map(|p| p as u8)
}
pub fn layer(name: &str) -> Option<u8> {
    LAYERS.iter().position(|l| l.eq_ignore_ascii_case(name)).map(|l| l as u8)
}

fn shared_root(inv: &Inventory) -> Option<Uuid> {
    inv.folders
        .values()
        .find(|f| !f.library && f.info.parent == inv.root && f.info.name == "#RLV")
        .map(|f| f.info.id)
}

fn visible(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('.') && !name.contains('/')
}

fn path(inv: &Inventory, mut id: Uuid) -> Option<String> {
    let root = shared_root(inv)?;
    let mut names = Vec::new();
    let mut seen = HashSet::new();
    while id != root && seen.insert(id) {
        let f = inv.folders.get(&id)?;
        names.push(f.info.name.clone());
        id = f.info.parent;
    }
    if id != root {
        return None;
    }
    names.reverse();
    Some(names.join("/"))
}

fn resolve(inv: &Inventory, name: &str) -> Option<Uuid> {
    let mut id = shared_root(inv)?;
    if name.is_empty() {
        return Some(id);
    }
    for component in name.split('/') {
        if component.is_empty() || component == "." || component == ".." {
            return None;
        }
        let f = inv.folders.get(&id)?;
        id = *f
            .children
            .iter()
            .find(|child| inv.folders.get(child).is_some_and(|f| f.info.name.eq_ignore_ascii_case(component)))?;
    }
    Some(id)
}

impl World {
    /// Resolve forced wear through the same COF planner as the native UI.
    /// Requests remain queued while folders or originals are still loading.
    pub fn rlva_inventory_force(&mut self, source: Uuid, c: &Command) -> bool {
        let name = c.behaviour.as_str();
        if !matches!(
            name,
            "attach" | "attachover" | "attachall" | "attachallover" | "detach" | "detachall" | "detachme" | "remattach" | "remoutfit"
        ) {
            return false;
        }
        if self.rlva.appearance.len() >= 128 {
            return true;
        }
        if appearance::cof(&self.inventory).is_none_or(|cof| !appearance::complete(&self.inventory, cof)) {
            let worn = self.worn_attachment_items();
            self.rlva.out.extend(appearance::prepare(&mut self.inventory, self.agent_id, &worn));
            return false;
        }
        let worn = self.worn_attachment_points();
        let action = if name == "detachme" {
            self.rlva_attachment_item(source).map(appearance::Action::Remove)
        } else if matches!(name, "detach" | "remattach")
            && (c.option.is_empty() || point(&c.option).is_some() || Uuid::parse_str(&c.option).is_ok())
        {
            let target = Uuid::parse_str(&c.option).ok().and_then(|id| self.rlva_attachment_item(id));
            let ids = worn
                .iter()
                .filter(|(id, at)| c.option.is_empty() || point(&c.option) == Some(**at) || target == Some(**id))
                .map(|(id, _)| *id)
                .filter(|id| self.rlva_can_remove(*id, source))
                .collect();
            Some(appearance::Action::RemoveItems(ids))
        } else if name == "remoutfit" && (c.option.is_empty() || layer(&c.option).is_some()) {
            let ids = appearance::cof(&self.inventory)
                .map(|cof| appearance::folder_links(&self.inventory, cof))
                .unwrap_or_default()
                .iter()
                .filter_map(|l| self.inventory.items.get(&l.target))
                .filter(|it| it.asset_type == 5 && (c.option.is_empty() || layer(&c.option) == Some((it.flags & 0xff) as u8)))
                .filter(|it| self.rlva_can_remove(it.id, source))
                .map(|it| it.id)
                .collect();
            Some(appearance::Action::RemoveItems(ids))
        } else if let Some(folder) = resolve(&self.inventory, &c.option) {
            let mut stack = vec![folder];
            let mut seen = HashSet::new();
            let mut ids = Vec::new();
            while let Some(id) = stack.pop() {
                if !seen.insert(id) {
                    continue;
                }
                if !appearance::loaded(&self.inventory, id) {
                    self.inventory.request(id);
                    return false;
                }
                if let Some(f) = self.inventory.folders.get(&id) {
                    if matches!(name, "attachall" | "attachallover" | "detachall") {
                        stack.extend(f.children.iter().copied());
                    }
                    ids.extend(
                        f.items
                            .iter()
                            .filter_map(|id| super::super::inventory::actions::original(&self.inventory, *id))
                            .filter(|id| self.inventory.items.get(id).is_some_and(|it| matches!(it.asset_type, 5 | 6 | 13))),
                    );
                }
            }
            ids.sort();
            ids.dedup();
            if matches!(name, "detach" | "detachall" | "remattach" | "remoutfit") {
                ids.retain(|id| self.rlva_can_remove(*id, source));
                Some(appearance::Action::RemoveItems(ids))
            } else {
                ids.retain(|id| self.rlva_can_add(*id, 0, source));
                Some(appearance::Action::WearItems {
                    items: ids,
                    replace: !matches!(name, "attachover" | "attachallover"),
                    point: 0,
                })
            }
        } else {
            None
        };
        if let Some(action) = action {
            self.rlva.appearance.push((source, action));
        }
        true
    }

    pub fn rlva_can_remove(&self, item: Uuid, actor: Uuid) -> bool {
        let r = &self.rlva;
        if !r.enabled() || (actor.is_nil() && !r.inventory_locked()) {
            return true;
        }
        let Some(it) = self.inventory.items.get(&item) else {
            return false;
        };
        // Nostrip is the user's override of scripted undressing. It does not
        // prevent the user manually removing an item.
        if !actor.is_nil() {
            let mut id = item;
            let mut seen = HashSet::new();
            while seen.insert(id) {
                if let Some(it) = self.inventory.items.get(&id) {
                    if it.name.to_lowercase().contains("nostrip") {
                        return false;
                    }
                    id = it.parent;
                } else if let Some(f) = self.inventory.folders.get(&id) {
                    if f.info.name.to_lowercase().contains("nostrip") {
                        return false;
                    }
                    id = f.info.parent;
                } else {
                    break;
                }
            }
        }
        if it.asset_type == 6 {
            let p = self.worn_attachment_points().get(&item).copied().unwrap_or((it.flags & 0xff) as u8);
            if self.rlva_point_locked("remattach", p, actor) {
                return false;
            }
            for (source, option) in r.rules_for("detach", actor) {
                if option.is_empty() {
                    if self.rlva_attachment_item(source) == Some(item) {
                        return false;
                    }
                } else if point(option) == Some(p) {
                    return false;
                }
            }
        } else if matches!(it.asset_type, 5 | 13) && self.rlva_layer_locked("remoutfit", (it.flags & 0xff) as u8, actor) {
            return false;
        }
        let shared = path(&self.inventory, it.parent).is_some();
        !r.has_except(if shared { "sharedunwear" } else { "unsharedunwear" }, actor)
    }

    pub fn rlva_can_add(&self, item: Uuid, at: u8, actor: Uuid) -> bool {
        let r = &self.rlva;
        if !r.enabled() || !r.inventory_locked() {
            return true;
        }
        let Some(it) = self.inventory.items.get(&item) else {
            return false;
        };
        if it.asset_type == 6 && self.rlva_point_locked("addattach", if at == 0 { (it.flags & 0xff) as u8 } else { at }, actor) {
            return false;
        }
        if matches!(it.asset_type, 5 | 13) && self.rlva_layer_locked("addoutfit", (it.flags & 0xff) as u8, actor) {
            return false;
        }
        let shared = path(&self.inventory, it.parent).is_some();
        !r.has_except(if shared { "sharedwear" } else { "unsharedwear" }, actor)
    }

    pub fn rlva_attachment_item(&self, object: Uuid) -> Option<Uuid> {
        let mut index = self.objects.index_of_uuid(&object)?;
        for _ in 0..64 {
            let o = self.objects.get(index)?;
            if let Some(item) = o.attachment_item_id() {
                return Some(item);
            }
            index = self.objects.parent_of(o)?;
        }
        None
    }

    fn rlva_point_locked(&self, name: &str, p: u8, actor: Uuid) -> bool {
        self.rlva
            .rules_for(name, actor)
            .any(|(_, option)| option.is_empty() || point(option) == Some(p))
    }
    fn rlva_layer_locked(&self, name: &str, kind: u8, actor: Uuid) -> bool {
        self.rlva
            .rules_for(name, actor)
            .any(|(_, option)| option.is_empty() || layer(option) == Some(kind))
    }

    pub fn rlva_outfit_allowed(&self, before: &[OutfitLink], after: &[OutfitLink], actor: Uuid) -> bool {
        before
            .iter()
            .filter(|l| !l.folder && !after.iter().any(|n| n.target == l.target))
            .all(|l| self.rlva_can_remove(l.target, actor))
            && after
                .iter()
                .filter(|l| !l.folder && !before.iter().any(|n| n.target == l.target))
                .all(|l| self.rlva_can_add(l.target, 0, actor))
    }

    pub fn rlva_inventory_query(&mut self, c: &Command) -> Option<String> {
        let worn_points = self.worn_attachment_points();
        let cof = appearance::cof(&self.inventory)
            .map(|id| appearance::folder_links(&self.inventory, id))
            .unwrap_or_default();
        let worn = |kind: usize| {
            cof.iter().any(|l| {
                self.inventory
                    .items
                    .get(&l.target)
                    .is_some_and(|it| matches!(it.asset_type, 5 | 13) && it.flags & 0xff == kind as u32)
            })
        };
        let list_layers = |mode: &str| {
            LAYERS
                .iter()
                .enumerate()
                .filter(|(i, _)| match mode {
                    "getoutfitnames" => worn(*i),
                    "getaddoutfitnames" => !self.rlva_layer_locked("addoutfit", *i as u8, Uuid::nil()),
                    _ => !self.rlva_layer_locked("remoutfit", *i as u8, Uuid::nil()),
                })
                .map(|(_, n)| *n)
                .collect::<Vec<_>>()
                .join(",")
        };
        match c.behaviour.as_str() {
            "getattach" => {
                return Some(if c.option.is_empty() {
                    (0..POINTS.len())
                        .map(|p| {
                            if p != 0 && worn_points.values().any(|v| *v == p as u8) {
                                '1'
                            } else {
                                '0'
                            }
                        })
                        .collect()
                } else {
                    point(&c.option).map_or(String::new(), |p| {
                        if worn_points.values().any(|v| *v == p) { "1" } else { "0" }.to_owned()
                    })
                });
            }
            "getoutfit" => {
                return Some(if c.option.is_empty() {
                    (0..LAYERS.len()).map(|i| if worn(i) { '1' } else { '0' }).collect()
                } else {
                    layer(&c.option).map_or(String::new(), |i| if worn(i as usize) { "1" } else { "0" }.to_owned())
                });
            }
            "getoutfitnames" | "getaddoutfitnames" | "getremoutfitnames" => return Some(list_layers(&c.behaviour)),
            "getattachnames" | "getaddattachnames" | "getremattachnames" => {
                return Some(
                    POINTS
                        .iter()
                        .enumerate()
                        .skip(1)
                        .filter(|(i, _)| match c.behaviour.as_str() {
                            "getattachnames" => worn_points.values().any(|p| *p == *i as u8),
                            "getaddattachnames" => !self.rlva_point_locked("addattach", *i as u8, Uuid::nil()),
                            _ => !self.rlva_point_locked("remattach", *i as u8, Uuid::nil()),
                        })
                        .map(|(_, n)| *n)
                        .collect::<Vec<_>>()
                        .join(","),
                );
            }
            "getpath" | "getpathnew" => {
                let items: Vec<_> = if let Some(p) = point(&c.option) {
                    worn_points.iter().filter(|(_, at)| **at == p).map(|(id, _)| *id).collect()
                } else if let Some(kind) = layer(&c.option) {
                    cof.iter()
                        .filter_map(|l| self.inventory.items.get(&l.target))
                        .filter(|it| matches!(it.asset_type, 5 | 13) && it.flags & 0xff == kind as u32)
                        .map(|it| it.id)
                        .collect()
                } else {
                    Vec::new()
                };
                let mut paths: Vec<_> = items
                    .iter()
                    .filter_map(|id| self.inventory.items.get(id))
                    .filter_map(|it| path(&self.inventory, it.parent))
                    .collect();
                paths.sort();
                paths.dedup();
                return Some(paths.join(","));
            }
            _ => {}
        }
        let Some(root) = shared_root(&self.inventory) else {
            let loaded = appearance::loaded(&self.inventory, self.inventory.root);
            self.inventory.request(self.inventory.root);
            return loaded.then(String::new);
        };
        // A shared-tree query cannot treat unloaded descendants as empty.
        // Request only #RLV, never unrelated inventory or the Library.
        let mut stack = vec![root];
        let mut seen = HashSet::new();
        let mut ready = true;
        while let Some(id) = stack.pop() {
            if !seen.insert(id) {
                continue;
            }
            if !appearance::loaded(&self.inventory, id) {
                self.inventory.request(id);
                ready = false;
            }
            if let Some(f) = self.inventory.folders.get(&id) {
                stack.extend(
                    f.children
                        .iter()
                        .copied()
                        .filter(|id| self.inventory.folders.get(id).is_some_and(|f| visible(&f.info.name))),
                );
            }
        }
        if !ready {
            return None;
        }
        if matches!(c.behaviour.as_str(), "findfolder" | "findfolders") {
            let terms: Vec<_> = c.option.split("&&").map(str::to_lowercase).collect();
            let mut matches: Vec<_> = self
                .inventory
                .folders
                .values()
                .filter(|f| visible(&f.info.name) && terms.iter().all(|term| f.info.name.to_lowercase().contains(term)))
                .filter_map(|f| path(&self.inventory, f.info.id))
                .filter(|p| !p.is_empty())
                .collect();
            matches.sort();
            if c.behaviour == "findfolder" {
                matches.truncate(1);
            }
            return Some(matches.join(","));
        }
        let Some(folder) = resolve(&self.inventory, &c.option) else {
            return Some(String::new());
        };
        if !appearance::loaded(&self.inventory, folder) {
            self.inventory.request(folder);
            return None;
        }
        let Some(folder) = self.inventory.folders.get(&folder) else {
            return Some(String::new());
        };
        let children: Vec<_> = folder
            .children
            .iter()
            .filter_map(|id| self.inventory.folders.get(id))
            .filter(|f| visible(&f.info.name))
            .collect();
        if c.behaviour == "getinv" {
            return Some(children.iter().map(|f| f.info.name.as_str()).collect::<Vec<_>>().join(","));
        }
        let worn: HashSet<_> = cof.iter().map(|l| l.target).chain(worn_points.keys().copied()).collect();
        let status = |id, descendants| {
            let mut stack = if descendants {
                self.inventory.folders.get(&id).map_or(Vec::new(), |f| f.children.clone())
            } else {
                vec![id]
            };
            let mut seen = HashSet::new();
            let mut ids = HashSet::new();
            while let Some(id) = stack.pop() {
                if !seen.insert(id) {
                    continue;
                }
                if let Some(f) = self.inventory.folders.get(&id) {
                    ids.extend(
                        f.items
                            .iter()
                            .filter_map(|id| super::super::inventory::actions::original(&self.inventory, *id))
                            .filter(|id| self.inventory.items.get(id).is_some_and(|it| matches!(it.asset_type, 5 | 6 | 13))),
                    );
                    if descendants {
                        stack.extend(f.children.iter().copied());
                    }
                }
            }
            let count = ids.iter().filter(|id| worn.contains(id)).count();
            if ids.is_empty() {
                0
            } else if count == 0 {
                1
            } else if count == ids.len() {
                3
            } else {
                2
            }
        };
        Some(
            std::iter::once(format!("|{}{}", status(folder.info.id, false), status(folder.info.id, true)))
                .chain(
                    children
                        .iter()
                        .map(|f| format!("{}|{}{}", f.info.name, status(f.info.id, false), status(f.info.id, true))),
                )
                .collect::<Vec<_>>()
                .join(","),
        )
    }
}
