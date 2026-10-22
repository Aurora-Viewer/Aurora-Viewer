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
// RlvHandler::onGetOutfit uses the historical protocol order, independent
// of LLWearableType's numeric order used by flags and the name queries.
const OUTFIT_ORDER: &[usize] = &[9, 8, 5, 4, 6, 12, 7, 11, 10, 1, 3, 2, 0, 13, 14, 15, 16];
const GROUPS: &[&str] = &["arms", "head", "legs", "torso", "hud"];

/// rlvAttachGroupFromIndex; body groups in avatar_lad.xml are zero-based.
/// Deliberate correction: HUDs use the native HUD flag below. The reference's
/// pre-Bento group 8 is now ordinary Bento points; actual HUDs use group 9.
fn group_name(group: u8) -> Option<&'static str> {
    Some(match group {
        0 | 1 | 3 | 4 => "arms",
        2 => "head",
        5 | 7 => "legs",
        6 => "torso",
        _ => return None,
    })
}
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
    if name.eq_ignore_ascii_case("root") {
        return Some(40);
    }
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

fn folded(name: &str) -> bool {
    name == ".(nostrip)" || name.strip_prefix(".(").and_then(|n| n.strip_suffix(')')).and_then(point).is_some()
}

fn folded_parent(inv: &Inventory, mut id: Uuid) -> Uuid {
    let mut seen = HashSet::new();
    while seen.insert(id) {
        let Some(f) = inv.folders.get(&id).filter(|f| folded(&f.info.name)) else {
            break;
        };
        id = f.info.parent;
    }
    id
}

fn named_point(name: &str) -> Option<u8> {
    let (_, tail) = name.split_once('(')?;
    point(tail.split_once(')')?.0)
}

pub fn force_kind(name: &str) -> Option<&'static str> {
    if !super::FORCES.contains(&name) {
        return None;
    }
    if matches!(name, "detachme" | "remattach" | "remoutfit") {
        return Some(match name {
            "detachme" => "detachme",
            "remattach" => "remattach",
            _ => "remoutfit",
        });
    }
    if name.starts_with("detach") {
        return Some(if name.contains("all") { "detachall" } else { "detach" });
    }
    if !(name.starts_with("attach") || name.starts_with("addoutfit")) {
        return None;
    }
    Some(match (name.contains("all"), name.ends_with("over")) {
        (true, true) => "attachallover",
        (true, false) => "attachall",
        (false, true) => "attachover",
        _ => "attach",
    })
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
    for component in name.split('/').filter(|v| !v.is_empty()) {
        if component == "." || component == ".." {
            return None;
        }
        let f = inv.folders.get(&id)?;
        id = *f
            .children
            .iter()
            .find(|child| inv.folders.get(child).is_some_and(|f| f.info.name.eq_ignore_ascii_case(component)))
            .or_else(|| {
                f.children.iter().find(|child| {
                    inv.folders
                        .get(child)
                        .is_some_and(|f| visible(&f.info.name) && f.info.name.to_lowercase().contains(&component.to_lowercase()))
                })
            })?;
    }
    Some(id)
}

impl World {
    fn rlva_attachment_group(&self, point: u8) -> Option<&'static str> {
        let point = self.avatar_lib.attach_points.get(&point)?;
        if point.hud { Some("hud") } else { group_name(point.group) }
    }
    /// Shared originals can live elsewhere; locks and getpath follow every
    /// shared link, then fold attachment-point folders into their parent.
    fn rlva_item_folders(&self, item: Uuid) -> Vec<Uuid> {
        self.inventory
            .items
            .values()
            .filter(|it| it.id == item || (it.asset_type == 24 && it.asset_id == item))
            .map(|it| folded_parent(&self.inventory, it.parent))
            .filter(|id| path(&self.inventory, *id).is_some())
            .collect()
    }

    fn rlva_context_folders(&self, source: Uuid, option: &str) -> Vec<Uuid> {
        let worn = self.rlva_worn_attachment_points();
        let items: Vec<_> = if option.is_empty() {
            self.rlva_attachment_item(source).into_iter().collect()
        } else if let Some(p) = point(option) {
            worn.iter().filter(|(_, at)| **at == p).map(|(id, _)| *id).collect()
        } else if let Some(kind) = layer(option) {
            appearance::cof(&self.inventory)
                .map(|id| appearance::folder_links(&self.inventory, id))
                .unwrap_or_default()
                .iter()
                .filter_map(|l| self.inventory.items.get(&l.target))
                .filter(|it| matches!(it.asset_type, 5 | 13) && it.flags & 0xff == kind as u32)
                .map(|it| it.id)
                .collect()
        } else if let Ok(id) = Uuid::parse_str(option) {
            self.rlva_attachment_item(id).into_iter().collect()
        } else {
            return resolve(&self.inventory, option).into_iter().collect();
        };
        let mut folders: Vec<_> = items.into_iter().flat_map(|id| self.rlva_item_folders(id)).collect();
        folders.sort();
        folders.dedup();
        folders
    }

    pub(super) fn rlva_folder_allowed(&self, folder: Uuid, add: bool, actor: Uuid) -> bool {
        self.rlva_folder_signature(folder, add, actor).is_empty()
    }

    fn rlva_folder_signature(&self, folder: Uuid, add: bool, _actor: Uuid) -> std::collections::BTreeSet<(Uuid, String, String)> {
        let folder = folded_parent(&self.inventory, folder);
        let mut ancestors = Vec::new();
        let mut id = folder;
        while !ancestors.contains(&id) {
            ancestors.push(id);
            let Some(f) = self.inventory.folders.get(&id) else { break };
            id = f.info.parent;
        }
        let shared = path(&self.inventory, folder).is_some();
        // The nearest rule of each issuer wins; an exception granted by one
        // issuer cannot release another issuer's subtree lock.
        let mut nearest = std::collections::BTreeMap::<Uuid, (usize, bool, Vec<(String, String)>)>::new();
        for (source, rules) in &self.rlva.rules {
            for rule in rules {
                let name = rule.behaviour.as_str();
                let global = name
                    == match (shared, add) {
                        (true, true) => "sharedwear",
                        (true, false) => "sharedunwear",
                        (false, true) => "unsharedwear",
                        _ => "unsharedunwear",
                    };
                if !global
                    && !(if add {
                        name.starts_with("attach")
                    } else {
                        name.starts_with("detach")
                    })
                {
                    continue;
                }
                if !global && !name.contains("this") {
                    continue;
                }
                let allow = name.ends_with("_except");
                let distance = if global {
                    Some(usize::MAX)
                } else {
                    self.rlva_context_folders(*source, &rule.option)
                        .into_iter()
                        .filter_map(|root| ancestors.iter().position(|id| *id == root))
                        .filter(|d| *d == 0 || name.contains("all"))
                        .min()
                };
                if let Some(distance) = distance {
                    let entry = nearest.entry(*source).or_insert((usize::MAX, true, Vec::new()));
                    if distance < entry.0 {
                        *entry = (distance, allow, Vec::new());
                    } else if distance == entry.0 {
                        entry.1 &= allow;
                    }
                    if distance == entry.0 && !allow {
                        entry.2.push((rule.behaviour.clone(), rule.option.clone()));
                    }
                }
            }
        }
        nearest
            .into_iter()
            .filter(|(_, (_, allow, _))| !allow)
            .flat_map(|(source, (_, _, rules))| rules.into_iter().map(move |(name, option)| (source, name, option)))
            .collect()
    }

    pub fn rlva_inventory_move_allowed(&self, item: Uuid, destination: Uuid, actor: Uuid) -> bool {
        if !self.rlva.enabled() || !self.rlva.inventory_locked() {
            return true;
        }
        let Some(parent) = self
            .inventory
            .items
            .get(&item)
            .map(|it| it.parent)
            .or_else(|| self.inventory.folders.get(&item).map(|f| f.info.parent))
        else {
            return false;
        };
        let folder = self.inventory.folders.contains_key(&item);
        let signature = |id, add| {
            self.rlva_folder_signature(id, add, actor)
                .into_iter()
                .filter(|(_, name, _)| folder || name.contains("this"))
                .collect::<std::collections::BTreeSet<_>>()
        };
        [false, true]
            .into_iter()
            .all(|add| signature(if folder { item } else { parent }, add) == signature(destination, add))
            && (!folder || !self.rlva_explicit_folder_descendant(item, actor))
    }

    fn rlva_explicit_folder_descendant(&self, folder: Uuid, actor: Uuid) -> bool {
        self.rlva
            .rules
            .iter()
            .filter(|(source, _)| **source != actor)
            .any(|(source, rules)| {
                rules.iter().filter(|r| r.behaviour.contains("this")).any(|rule| {
                    self.rlva_context_folders(*source, &rule.option).into_iter().any(|mut root| {
                        let mut seen = HashSet::new();
                        while seen.insert(root) {
                            if root == folder {
                                return true;
                            }
                            let Some(f) = self.inventory.folders.get(&root) else { break };
                            root = f.info.parent;
                        }
                        false
                    })
                })
            })
    }

    fn rlva_inventory_tree_unlocked(&self, folder: Uuid, actor: Uuid) -> bool {
        if self.rlva_explicit_folder_descendant(folder, actor) {
            return false;
        }
        let mut stack = vec![folder];
        let mut seen = HashSet::new();
        let worn = self.worn_attachment_items();
        let cof = appearance::cof(&self.inventory)
            .map(|id| appearance::folder_links(&self.inventory, id))
            .unwrap_or_default();
        while let Some(id) = stack.pop() {
            if !seen.insert(id) {
                continue;
            }
            let Some(f) = self.inventory.folders.get(&id) else {
                return false;
            };
            if !appearance::loaded(&self.inventory, id)
                || [false, true].into_iter().any(|add| {
                    self.rlva_folder_signature(id, add, actor)
                        .iter()
                        .any(|(_, name, _)| name.contains("this"))
                })
            {
                return false;
            }
            for id in &f.items {
                let target = super::super::inventory::actions::original(&self.inventory, *id).unwrap_or(*id);
                if (worn.contains(&target) || cof.iter().any(|l| l.target == target)) && !self.rlva_can_remove(target, actor) {
                    return false;
                }
            }
            stack.extend(f.children.iter().copied());
        }
        true
    }

    pub fn rlva_inventory_mutation_allowed(&self, change: &aurora_net::inventory::operations::Mutation, actor: Uuid) -> bool {
        use aurora_net::inventory::operations::Operation;
        if !self.rlva.enabled() || !self.rlva.inventory_locked() {
            return true;
        }
        change
            .copies
            .iter()
            .all(|copy| self.rlva_inventory_move_allowed(copy.item, copy.parent, actor))
            && change.operations.iter().all(|operation| match operation {
                Operation::Patch { id, folder, body } => {
                    let moved = body.as_map().is_some_and(|m| m.contains_key("parent_id"));
                    let renamed = body.as_map().is_some_and(|m| m.contains_key("name"));
                    (!moved || self.rlva_inventory_move_allowed(*id, body["parent_id"].as_uuid(), actor))
                        && (!renamed || !*folder || !self.rlva_explicit_folder_descendant(*id, actor))
                }
                Operation::Delete { id, folder: true } | Operation::Purge(id) => self.rlva_inventory_tree_unlocked(*id, actor),
                Operation::Delete { id, folder: false } => {
                    let target = super::super::inventory::actions::original(&self.inventory, *id).unwrap_or(*id);
                    let worn = self.worn_attachment_items().contains(&target)
                        || appearance::cof(&self.inventory)
                            .is_some_and(|id| appearance::folder_links(&self.inventory, id).iter().any(|l| l.target == target));
                    self.inventory.items.get(id).is_some_and(|it| {
                        [false, true].into_iter().all(|add| {
                            self.rlva_folder_signature(it.parent, add, actor)
                                .iter()
                                .all(|(_, name, _)| !name.contains("this"))
                        })
                    }) && (!worn || self.rlva_can_remove(target, actor))
                }
                Operation::Links { parent, links } if Some(*parent) == appearance::cof(&self.inventory) => {
                    let before = appearance::folder_links(&self.inventory, *parent);
                    let after: Vec<_> = before.iter().chain(links).cloned().collect();
                    self.rlva_outfit_allowed(&before, &after, actor)
                }
                _ => true,
            })
    }

    fn rlva_shared_ready(&mut self) -> bool {
        let Some(root) = shared_root(&self.inventory) else {
            let loaded = appearance::loaded(&self.inventory, self.inventory.root);
            if !loaded {
                self.inventory.request(self.inventory.root);
            }
            return loaded;
        };
        let mut stack = vec![root];
        let mut seen = HashSet::new();
        let mut ready = true;
        let mut missing = HashSet::new();
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
                        .filter(|id| {
                            self.inventory
                                .folders
                                .get(id)
                                .is_some_and(|f| visible(&f.info.name) || folded(&f.info.name))
                        })
                        .copied(),
                );
                for id in &f.items {
                    if let Some(it) = self.inventory.items.get(id)
                        && it.asset_type == 24
                        && !self.inventory.items.contains_key(&it.asset_id)
                    {
                        missing.insert(it.asset_id);
                    }
                }
            }
        }
        if !missing.is_empty() {
            let now = std::time::Instant::now();
            let mut request = Vec::new();
            for id in missing {
                let entry = self.rlva.inventory_fetches.entry(id).or_insert((now, 0));
                if entry.1 < 3 {
                    ready = false;
                    if entry.1 == 0 || now.saturating_duration_since(entry.0).as_secs() >= 10 {
                        request.push(id);
                        *entry = (now, entry.1 + 1);
                    }
                }
            }
            if !request.is_empty() {
                self.rlva.out.push(aurora_net::NetCommand::FetchItems {
                    items: request,
                    owner: self.agent_id,
                });
            }
        }
        ready
    }
    /// Resolve forced wear through the same COF planner as the native UI.
    /// Requests remain queued while folders or originals are still loading.
    pub fn rlva_inventory_force(&mut self, source: Uuid, c: &Command) -> bool {
        let Some(name) = force_kind(&c.behaviour) else { return false };
        let context = c.behaviour.contains("this");
        if !context && c.option.is_empty() && matches!(name, "attach" | "attachover" | "attachall" | "attachallover" | "detachall") {
            return true;
        }
        if self.rlva.appearance.len() >= 128 {
            return true;
        }
        if appearance::cof(&self.inventory).is_none_or(|cof| !appearance::complete(&self.inventory, cof)) {
            let worn = self.worn_attachment_items();
            self.rlva.out.extend(appearance::prepare(&mut self.inventory, self.agent_id, &worn));
            return false;
        }
        let worn = self.rlva_worn_attachment_points();
        let group = GROUPS.contains(&c.option.as_str()) && resolve(&self.inventory, &c.option).is_none();
        let action = if name == "detachme" {
            self.rlva_attachment_item(source).map(|item| {
                self.rlva.self_detach = Some((source, item));
                appearance::Action::Remove(item)
            })
        } else if !context
            && matches!(name, "detach" | "remattach")
            && (c.option.is_empty() || point(&c.option).is_some() || Uuid::parse_str(&c.option).is_ok() || group)
        {
            let target = Uuid::parse_str(&c.option).ok().and_then(|id| self.rlva_attachment_item(id));
            let ids = worn
                .iter()
                .filter(|(id, at)| {
                    c.option.is_empty()
                        || point(&c.option) == Some(**at)
                        || target == Some(**id)
                        || (group && self.rlva_attachment_group(**at) == Some(c.option.as_str()))
                })
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
        } else {
            if !self.rlva_shared_ready() {
                return false;
            }
            let mut stack = if context {
                self.rlva_context_folders(source, &c.option)
            } else {
                resolve(&self.inventory, &c.option)
                    .filter(|id| Some(*id) != shared_root(&self.inventory))
                    .into_iter()
                    .collect()
            };
            let mut seen = HashSet::new();
            let mut ids = Vec::new();
            let mut points = std::collections::HashMap::new();
            while let Some(id) = stack.pop() {
                if !seen.insert(id) {
                    continue;
                }
                if !appearance::loaded(&self.inventory, id) {
                    self.inventory.request(id);
                    return false;
                }
                if let Some(f) = self.inventory.folders.get(&id) {
                    stack.extend(
                        f.children
                            .iter()
                            .filter(|id| {
                                self.inventory.folders.get(id).is_some_and(|f| {
                                    folded(&f.info.name)
                                        || (matches!(name, "attachall" | "attachallover" | "detachall") && visible(&f.info.name))
                                })
                            })
                            .copied(),
                    );
                    for item in f
                        .items
                        .iter()
                        .filter_map(|id| super::super::inventory::actions::original(&self.inventory, *id))
                    {
                        let Some(it) = self.inventory.items.get(&item).filter(|it| matches!(it.asset_type, 5 | 6 | 13)) else {
                            continue;
                        };
                        if it.asset_type == 6 {
                            points.insert(
                                item,
                                named_point(&f.info.name)
                                    .or_else(|| named_point(&it.name))
                                    .unwrap_or((it.flags & 0xff) as u8),
                            );
                        }
                        ids.push(item);
                    }
                }
            }
            ids.sort();
            ids.dedup();
            if matches!(name, "detach" | "detachall" | "remattach" | "remoutfit") {
                ids.retain(|id| self.inventory.items.get(id).is_some_and(|it| it.asset_type != 13) && self.rlva_can_remove(*id, source));
                Some(appearance::Action::RemoveItems(ids))
            } else {
                ids.sort_by(|a, b| {
                    self.inventory.items[a]
                        .name
                        .to_lowercase()
                        .cmp(&self.inventory.items[b].name.to_lowercase())
                        .then_with(|| a.cmp(b))
                });
                let replace = !matches!(name, "attachover" | "attachallover");
                let mut replaced = HashSet::new();
                let current = appearance::cof(&self.inventory)
                    .map(|id| appearance::folder_links(&self.inventory, id))
                    .unwrap_or_default();
                for item in ids.into_iter().take(128) {
                    let it = &self.inventory.items[&item];
                    let point = points.get(&item).copied().unwrap_or(0);
                    let key = (it.asset_type, if it.asset_type == 6 { point as u32 } else { it.flags & 0xff });
                    let replace_item = (replace || it.asset_type == 13) && !replaced.contains(&key);
                    if !self.rlva_can_add(item, point, source) {
                        continue;
                    }
                    if replace_item
                        && it.asset_type != 6
                        && current
                            .iter()
                            .filter(|l| l.target != item)
                            .filter_map(|l| self.inventory.items.get(&l.target))
                            .any(|old| {
                                let same_slot = old.asset_type == it.asset_type
                                    && if it.asset_type == 6 {
                                        point != 0 && worn.get(&old.id).copied().unwrap_or((old.flags & 0xff) as u8) == point
                                    } else {
                                        old.flags & 0xff == it.flags & 0xff
                                    };
                                same_slot && !self.rlva_can_remove(old.id, source)
                            })
                    {
                        continue;
                    }
                    replaced.insert(key);
                    self.rlva.appearance.push((
                        source,
                        appearance::Action::WearItem {
                            item,
                            replace: replace_item,
                            point,
                        },
                    ));
                }
                None
            }
        };
        if let Some(action) = action {
            self.rlva.appearance.push((source, action));
        }
        true
    }

    pub fn rlva_can_remove(&self, item: Uuid, actor: Uuid) -> bool {
        let r = &self.rlva;
        if r.enabled() && actor == super::watchdog::ACTOR {
            return r.watchdog.rejected.contains(&item);
        }
        if !r.enabled() || (actor.is_nil() && !r.inventory_locked()) {
            return true;
        }
        let Some(it) = self.inventory.items.get(&item) else {
            return self.rlva_temporary_removable(item, actor);
        };
        // Nostrip is the user's override of scripted undressing. It does not
        // prevent the user manually removing an item.
        if !actor.is_nil() && self.rlva.self_detach != Some((actor, item)) {
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
            let p = self
                .worn_attachment_points()
                .get(&item)
                .copied()
                .or_else(|| r.watchdog.point(item))
                .unwrap_or((it.flags & 0xff) as u8);
            if !self.rlva_attachment_remove_at(item, p, actor) {
                return false;
            }
        } else if matches!(it.asset_type, 5 | 13) && self.rlva_layer_locked("remoutfit", (it.flags & 0xff) as u8, actor) {
            return false;
        }
        self.rlva_folder_allowed(it.parent, false, actor)
    }

    pub fn rlva_can_add(&self, item: Uuid, at: u8, actor: Uuid) -> bool {
        let r = &self.rlva;
        if r.enabled() && actor == super::watchdog::ACTOR {
            return r.watchdog.restore_allowed(item, at);
        }
        if r.enabled() && at != 0 && r.watchdog.pending.iter().any(|(id, p)| *id != item && p.attachment.point == at) {
            return false;
        }
        if !r.enabled() || !r.inventory_locked() {
            return true;
        }
        let Some(it) = self.inventory.items.get(&item) else {
            return self.rlva_temporary_addable(item, at, actor);
        };
        if it.asset_type == 6 {
            let at = if at == 0 { (it.flags & 0xff) as u8 } else { at };
            if !self.rlva_point_addable(at) {
                return false;
            }
        }
        if matches!(it.asset_type, 5 | 13) && self.rlva_layer_locked("addoutfit", (it.flags & 0xff) as u8, actor) {
            return false;
        }
        self.rlva_folder_allowed(it.parent, true, actor)
    }

    pub fn rlva_attachment_item(&self, object: Uuid) -> Option<Uuid> {
        if let Some(mut index) = self.objects.index_of_uuid(&object) {
            for _ in 0..64 {
                let Some(o) = self.objects.get(index) else {
                    break;
                };
                if o.attachment_point() != 0
                    && self
                        .objects
                        .parent_of(o)
                        .and_then(|i| self.objects.get(i))
                        .is_some_and(|p| p.is_avatar())
                {
                    return Some(o.attachment_item_id().filter(|item| !item.is_nil()).unwrap_or(o.full_id));
                }
                if let Some(item) = o.attachment_item_id().filter(|item| !item.is_nil()) {
                    return Some(item);
                }
                let Some(parent) = self.objects.parent_of(o) else {
                    break;
                };
                index = parent;
            }
        }
        self.rlva.watchdog.sources.get(&object).copied()
    }

    pub(super) fn rlva_point_locked(&self, name: &str, p: u8, _actor: Uuid) -> bool {
        self.rlva
            .rules_for(name, Uuid::nil())
            .any(|(_, option)| option.is_empty() || point(option) == Some(p))
    }
    pub(super) fn rlva_point_addable(&self, p: u8) -> bool {
        !self.rlva_point_locked("addattach", p, Uuid::nil())
            && !self
                .rlva
                .rules_for("detach", Uuid::nil())
                .any(|(_, option)| !option.is_empty() && point(option) == Some(p))
    }
    fn rlva_layer_locked(&self, name: &str, kind: u8, _actor: Uuid) -> bool {
        self.rlva
            .rules_for(name, Uuid::nil())
            .any(|(_, option)| option.is_empty() || layer(option) == Some(kind))
    }

    /// RlvAttachmentLocks::canAttach and RLVaWearReplaceUnlocked (default true):
    /// replacement keeps locked attachments and removes unlocked predecessors.
    /// Clothing and explicit removal still use the ordinary lock checks.
    pub fn rlva_prepare_outfit(&self, command: &mut aurora_net::NetCommand, actor: Uuid) {
        if !self.rlva.enabled() || actor == super::watchdog::ACTOR {
            return;
        }
        let aurora_net::NetCommand::UpdateOutfit { change, .. } = command else {
            return;
        };
        let Some(cof) = appearance::cof(&self.inventory) else {
            return;
        };
        let links = if change.folder == cof {
            &mut change.links
        } else if let Some((id, links)) = &mut change.cof {
            if *id != cof {
                return;
            }
            links
        } else {
            return;
        };
        // Only a transaction adding an attachment is a replacement. Removing
        // an outfit must fail when it contains an attachment locked in place.
        let before = appearance::folder_links(&self.inventory, cof);
        if !links.iter().any(|link| {
            !before.iter().any(|old| old.target == link.target)
                && self.inventory.items.get(&link.target).is_some_and(|it| it.asset_type == 6)
        }) {
            return;
        }
        for old in before {
            if !old.folder
                && !links.iter().any(|link| link.target == old.target)
                && self.inventory.items.get(&old.target).is_some_and(|it| it.asset_type == 6)
                && !self.rlva_can_remove(old.target, actor)
            {
                links.push(old);
            }
        }
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

    pub fn rlva_inventory_query(&mut self, c: &Command, source: Uuid) -> Option<String> {
        let worn_points = self.rlva_worn_attachment_points();
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
                    "getaddoutfitnames" => {
                        !self.rlva_layer_locked("addoutfit", *i as u8, Uuid::nil())
                            && !(*i < 4 && worn(*i) && self.rlva_layer_locked("remoutfit", *i as u8, Uuid::nil()))
                    }
                    _ => cof
                        .iter()
                        .filter_map(|l| self.inventory.items.get(&l.target))
                        .any(|it| it.asset_type == 5 && it.flags & 0xff == *i as u32 && self.rlva_can_remove(it.id, source)),
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
                    OUTFIT_ORDER.iter().map(|i| if worn(*i) { '1' } else { '0' }).collect()
                } else {
                    layer(&c.option).map_or(String::new(), |i| if worn(i as usize) { "1" } else { "0" }.to_owned())
                });
            }
            "getoutfitnames" | "getaddoutfitnames" | "getremoutfitnames" => {
                return Some(if c.option.is_empty() {
                    list_layers(&c.behaviour)
                } else {
                    String::new()
                });
            }
            "getattachnames" | "getaddattachnames" | "getremattachnames" => {
                return Some(
                    POINTS
                        .iter()
                        .enumerate()
                        .skip(1)
                        .filter(|(i, _)| {
                            let group = self.rlva_attachment_group(*i as u8);
                            !GROUPS.contains(&c.option.as_str()) || group == Some(c.option.as_str())
                        })
                        .filter(|(i, _)| match c.behaviour.as_str() {
                            "getattachnames" => worn_points.values().any(|p| *p == *i as u8),
                            "getaddattachnames" => self.rlva_point_addable(*i as u8),
                            _ => worn_points
                                .iter()
                                .any(|(id, at)| *at == *i as u8 && self.rlva_can_remove(*id, source)),
                        })
                        .map(|(_, n)| *n)
                        .collect::<Vec<_>>()
                        .join(","),
                );
            }
            "getpath" | "getpathnew" => {
                if !self.rlva_shared_ready() {
                    return None;
                }
                let mut paths: Vec<_> = self
                    .rlva_context_folders(source, &c.option)
                    .into_iter()
                    .filter_map(|id| path(&self.inventory, id))
                    .collect();
                paths.sort();
                paths.dedup();
                return Some(paths.join(","));
            }
            _ => {}
        }
        if !self.rlva_shared_ready() {
            return None;
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
                .filter(|p| !p.is_empty() && p.split('/').all(visible))
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
                self.inventory.folders.get(&id).map_or(Vec::new(), |f| {
                    f.children
                        .iter()
                        .filter(|id| self.inventory.folders.get(id).is_some_and(|f| visible(&f.info.name)))
                        .copied()
                        .collect()
                })
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
                    stack.extend(
                        f.children
                            .iter()
                            .filter(|id| {
                                self.inventory
                                    .folders
                                    .get(id)
                                    .is_some_and(|f| folded(&f.info.name) || (descendants && visible(&f.info.name)))
                            })
                            .copied(),
                    );
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
