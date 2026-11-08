//! One texture copy rule for faces, sculpt, projector and material slots:
//! which texture or material ids a copy may carry from one prim to another,
//! and how an item with limited rights follows the id into the target's
//! contents. Port of get_is_predefined_texture, get_copy_free_item_by_asset_id
//! and get_can_copy_texture (indra/newview/lltexturectrl.cpp),
//! set_item_availability (indra/newview/llpanelface.cpp),
//! LLMaterialEditor::canClipboardObjectsMaterial (indra/newview/llmaterialeditor.cpp)
//! and LLToolDragAndDrop::handleDropMaterialProtections
//! (indra/newview/lltooldraganddrop.cpp), originally LGPL 2.1.

use crate::build::{BuildTool, contents_drop, task_inventory};
use crate::ui::context::flags::{OBJECT_COPY, OBJECT_MODIFY, OBJECT_TRANSFER};
use crate::world::World;
use crate::world::inventory::{Inventory, actions as rules};
use aurora_net::build::BuildCmd;
use aurora_net::inventory::InvItem;
use uuid::Uuid;

/// The built-in ids every agent may use (get_is_predefined_texture):
/// the default object texture, specular and normal maps, the blank normal,
/// white, the default sculpt map and the blank material.
const PREDEFINED: [Uuid; 7] = [
    aurora_prim::te::DEFAULT_TEXTURE,
    Uuid::from_u128(0x87e0e8f7_8729_1ea8_cfc9_8915773009db),
    Uuid::from_u128(0x85f28839_7a1c_b4e3_d71d_967792970a7b),
    Uuid::from_u128(0x5b53359e_59dd_d8a2_04c3_9e65134da47a),
    aurora_prim::te::BLANK_TEXTURE,
    crate::build::edits::SCULPT_DEFAULT.texture,
    crate::scene::meshes::BLANK_MATERIAL,
];

pub fn predefined(asset: Uuid) -> bool {
    PREDEFINED.contains(&asset)
}

fn library(inv: &Inventory, id: Uuid) -> bool {
    rules::library(inv, id) || !inv.lib_root.is_nil() && rules::under(inv, id, inv.lib_root)
}

/// get_copy_free_item_by_asset_id: an inventory item (Trash and Library
/// included) holding `asset` that the agent may copy. One whose transfer
/// right differs from `no_transfer` wins; otherwise any copyable one. Among
/// equals the smallest id, so the answer does not depend on the map order.
pub fn copy_item(inv: &Inventory, agent: Uuid, asset: Uuid, no_transfer: bool) -> Option<Uuid> {
    let mut fallback: Option<Uuid> = None;
    let mut preferred: Option<Uuid> = None;
    for item in inv.items.values() {
        if item.asset_id != asset || item.asset_id.is_nil() || !contents_drop::copyable(item, agent) {
            continue;
        }
        let transfer = crate::world::inventory::actions::agent_mask(item, agent) & rules::TRANSFER != 0;
        let slot = if transfer != no_transfer { &mut preferred } else { &mut fallback };
        if slot.is_none_or(|id| item.id < id) {
            *slot = Some(item.id);
        }
    }
    preferred.or(fallback)
}

/// get_can_copy_texture: a built-in id, or one the agent holds a copyable
/// item of. A nil id carries nothing.
pub fn can_copy(inv: &Inventory, agent: Uuid, asset: Uuid) -> bool {
    asset.is_nil() || predefined(asset) || copy_item(inv, agent, asset, false).is_some()
}

/// The PBR map slots only take an item the agent may copy and transfer
/// (FSPanelFace::postBuild's setFilterPermissionMasks on those swatches).
pub fn map_slot_takes(item: &InvItem, agent: Uuid) -> bool {
    let both = rules::COPY | rules::TRANSFER;
    crate::world::inventory::actions::agent_mask(item, agent) & both == both
}

/// What a copied id may count on when it is pasted (set_item_availability).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Availability {
    /// The id is free to carry: built-in, from the Library, from an object
    /// its owner created with every right, or from a full-rights item.
    pub full: bool,
    /// The copyable inventory item the paste goes through when not `full`.
    pub item: Option<Uuid>,
    pub library: bool,
}

impl Availability {
    /// Usable by a paste now: free, or its item is still in the inventory
    /// (LLPanelFace::validateInventoryItem).
    pub fn usable(&self, inv: &Inventory) -> bool {
        self.full || self.item.is_some_and(|id| inv.items.contains_key(&id))
    }
}

/// set_item_availability for `asset` copied from an object: `creator` when
/// the source object has every right and its owner created it; `held` is the
/// source contents' own item of that asset, when there is one, and whether
/// it has full rights.
pub fn availability(inv: &Inventory, agent: Uuid, asset: Uuid, creator: bool, held: Option<bool>) -> Availability {
    // nil, or an override's "no texture here": nothing is carried
    if asset.is_nil() || asset == aurora_assets::material::OVERRIDE_NULL_UUID {
        return Availability {
            full: true,
            ..Default::default()
        };
    }
    let mut out = Availability {
        full: predefined(asset) || creator,
        library: predefined(asset),
        item: None,
    };
    let item = if out.full {
        None
    } else {
        copy_item(inv, agent, asset, held == Some(false))
    };
    if item.is_some_and(|id| library(inv, id)) {
        out.full = true;
        out.library = true;
    }
    if !out.full
        && let Some(it) = item.and_then(|id| inv.items.get(&id))
        && contents_drop::copyable(it, agent)
    {
        out.item = Some(it.id);
        out.full = rules::agent_mask(it, agent) & rules::FULL == rules::FULL;
    }
    out
}

/// The source object of a copy has every right and its owner created it:
/// its textures count as free (the `is_creator` of LLPanelFace::onCopyTexture).
pub fn creator_owned(tool: &BuildTool, world: &World, idx: usize) -> bool {
    let all = OBJECT_COPY | OBJECT_MODIFY | OBJECT_TRANSFER;
    let Some(o) = world.objects.get(idx) else { return false };
    if o.update_flags & all != all {
        return false;
    }
    let root = world.objects.get(crate::build::root_of(world, idx));
    let props = tool.props.get(&o.full_id).or_else(|| root.and_then(|r| tool.props.get(&r.full_id)));
    props.is_some_and(|p| !p.creator_id.is_nil() && p.creator_id == p.owner_id)
}

/// The source contents' own item of `asset`, and whether it has full rights
/// (getInventoryItemByAsset + getIsFullPerm).
pub fn held(tool: &BuildTool, world: &World, idx: usize, asset: Uuid) -> Option<bool> {
    let id = world.objects.get(idx)?.full_id;
    let inv = tool.contents.objects.get(&id)?;
    inv.items
        .iter()
        .find(|i| i.asset_id == asset)
        .map(|i| rules::task_agent_mask(i, world.agent_id) & rules::FULL == rules::FULL)
}

/// LLMaterialEditor::canClipboardObjectsMaterial for the faces `faces` of
/// the prim `idx`: free when no face has a material; otherwise the first
/// material needs the object's copy, modify and transfer rights, its contents
/// listing not on its way, and its contents' own item of that material (when
/// there is one) with the same three rights.
pub fn material_copy_allowed(tool: &BuildTool, world: &World, idx: usize, faces: &[u8]) -> bool {
    let Some(material) = faces.iter().find_map(|f| BuildTool::gltf_asset(world, idx, *f)) else {
        return true;
    };
    let Some(o) = world.objects.get(idx) else { return false };
    if tool.contents.objects.get(&o.full_id).is_some_and(|inv| inv.loading) {
        return false;
    }
    if material != crate::scene::meshes::BLANK_MATERIAL && held(tool, world, idx, material) == Some(false) {
        return false;
    }
    let all = OBJECT_COPY | OBJECT_MODIFY | OBJECT_TRANSFER;
    world
        .objects
        .get(crate::build::root_of(world, idx))
        .is_some_and(|root| root.update_flags & all == all)
}

/// Whether `item` cannot simply be painted and must sit in the target's
/// contents first: no copy, no transfer, or a material without modify
/// (handleDropMaterialProtections, LLSelectMgr::selectionSetGLTFMaterial).
pub fn restricted(item: &InvItem, agent: Uuid) -> bool {
    let mask = crate::world::inventory::actions::agent_mask(item, agent);
    mask & rules::COPY == 0 || mask & rules::TRANSFER == 0 || item.asset_type == AT_MATERIAL && mask & rules::MODIFY == 0
}

/// `restricted`, outside the Library (whose items always just work).
pub fn requires_contents(inv: &Inventory, item: &InvItem, agent: Uuid) -> bool {
    !library(inv, item.id) && restricted(item, agent)
}

/// LLSelectMgr::selectionSetImage / selectionSetGLTFMaterial: a no-copy item
/// cannot go on several objects. Counted by prim, not by face: it may cover
/// several faces of one object.
pub fn selection_copy_denied(item: &InvItem, agent: Uuid, objects: usize) -> bool {
    objects > 1 && !contents_drop::copyable(item, agent)
}

const AT_MATERIAL: i32 = 57;

impl BuildTool {
    /// LLMaterialEditor::canModifyObjectsMaterial for the faces `faces`: the
    /// first face with a material, on an object the agent may modify whose
    /// contents are listed, and whose contents' own item of that material,
    /// when there is one, allows modify. Asked each frame the PBR controls
    /// show: one object's listing (asked for once), no inventory scan.
    pub fn material_edit_allowed(&mut self, world: &World, faces: &[(usize, u8)]) -> bool {
        let Some((idx, material)) = faces
            .iter()
            .find_map(|&(idx, face)| BuildTool::gltf_asset(world, idx, face).map(|m| (idx, m)))
        else {
            return true;
        };
        if !self.can_modify(world, idx) || !self.contents_ready(world, idx) {
            return false;
        }
        let Some(o) = world.objects.get(idx) else { return false };
        material == crate::scene::meshes::BLANK_MATERIAL
            || self.contents.objects.get(&o.full_id).is_none_or(|inv| {
                inv.items
                    .iter()
                    .find(|i| i.asset_id == material)
                    .is_none_or(|i| rules::task_agent_mask(i, world.agent_id) & rules::MODIFY != 0)
            })
    }
}

/// Why an item could not follow its id into a target's contents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Retain {
    /// Nothing to do, or the item went into the contents.
    Done,
    /// The target's contents are not listed yet.
    Wait,
    /// The target does not take the item: its id stays off the face.
    Refused,
}

impl BuildTool {
    /// The target `idx`'s contents are listed and current enough to tell
    /// whether they hold an asset; asks for them otherwise.
    pub fn contents_ready(&mut self, world: &World, idx: usize) -> bool {
        let Some(id) = world.objects.get(idx).map(|o| o.full_id) else {
            return false;
        };
        self.request_contents(world, idx, false);
        self.contents
            .objects
            .get(&id)
            .is_some_and(|inv| !inv.loading && inv.requested.is_some())
    }

    /// The listed contents of `idx` hold `item`'s asset already.
    fn holds(&self, world: &World, idx: usize, item: &InvItem) -> bool {
        let Some(o) = world.objects.get(idx) else { return false };
        self.contents.objects.get(&o.full_id).is_some_and(|inv| {
            inv.items
                .iter()
                .any(|i| i.asset_id == item.asset_id && i.asset_type as i32 == item.asset_type)
        })
    }

    /// UpdateTaskInventory of `item` into `idx`, counted at once in the local
    /// listing so a second face of the same action does not send it again.
    fn put_in_contents(&mut self, world: &World, idx: usize, item: &InvItem) {
        let Some(o) = world.objects.get(idx) else { return };
        let object = o.full_id;
        let task = task_inventory::from_inventory(object, item);
        self.send(BuildCmd::UpdateTaskInventory {
            handle: o.key.region,
            local_id: o.key.local_id,
            item: Box::new(task.clone()),
        });
        if let Some(inv) = self.contents.objects.get_mut(&object) {
            inv.items.push(task);
        }
        self.request_contents(world, idx, true);
    }

    /// handleDropMaterialProtections for a paste: the item of a limited id
    /// goes into the target's contents once (the object then carries the
    /// item's rights), unless they hold that asset already.
    pub fn retain_for_paste(&mut self, world: &World, idx: usize, item_id: Uuid) -> Retain {
        let Some(item) = world.inventory.items.get(&item_id) else {
            return Retain::Refused;
        };
        if !requires_contents(&world.inventory, item, world.agent_id) {
            return Retain::Done;
        }
        if !self.contents_ready(world, idx) {
            return Retain::Wait;
        }
        if self.holds(world, idx, item) {
            return Retain::Done;
        }
        if contents_drop::accepts(world, idx, item).is_err() {
            return Retain::Refused;
        }
        let item = item.clone();
        self.put_in_contents(world, idx, &item);
        Retain::Done
    }

    /// An inventory item chosen in a build control (picker OK, a slot drop)
    /// before its id goes on the prims `objects`: LLSelectMgr::selectionSetImage,
    /// selectionSetGLTFMaterial and LLPanelVolume::setLightTextureID. A no-copy
    /// item goes on one object only; a worn object takes only what
    /// `contents_drop::worn_rights` allows, a Library item included, like
    /// TextureApplyFunctor; a restricted item goes into each object's
    /// contents (moved when it cannot be copied). Without an item (a typed id,
    /// a built-in button) the id is applied as it is, like the Legacy Viewer.
    /// Runs once per action. The error is the text to show.
    pub fn admit_item(&mut self, world: &mut World, item_id: Uuid, objects: &[usize]) -> Result<(), &'static str> {
        let inv = &world.inventory;
        let Some(item) = rules::original(inv, item_id).and_then(|id| inv.items.get(&id)).cloned() else {
            return Ok(());
        };
        let agent = world.agent_id;
        let mut objects = objects.to_vec();
        objects.sort_unstable();
        objects.dedup();
        if selection_copy_denied(&item, agent, objects.len()) {
            return Err(aurora_i18n::tr!("texture-library-no-copy-multiple"));
        }
        if objects.iter().any(|&idx| contents_drop::worn_refused(world, idx, &item)) {
            return Err(contents_drop::worn_refusal_text(&item, agent));
        }
        if !requires_contents(inv, &item, agent) {
            return Ok(());
        }
        let mut waiting = false;
        for &idx in &objects {
            waiting |= !self.contents_ready(world, idx);
        }
        if waiting {
            return Err(aurora_i18n::tr!("build-ui-texture-paste-wait-contents"));
        }
        let missing: Vec<usize> = objects.into_iter().filter(|&idx| !self.holds(world, idx, &item)).collect();
        if missing.iter().any(|&idx| contents_drop::accepts(world, idx, &item).is_err()) {
            return Err(aurora_i18n::tr!("texture-library-locked"));
        }
        for &idx in &missing {
            self.put_in_contents(world, idx, &item);
        }
        if !missing.is_empty() && !contents_drop::copyable(&item, agent) {
            world.inventory.consume_item(item.id);
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "texture_copy_tests.rs"]
mod tests;
