//! Explicit, one-time preparation of copyable inventory models (#1283).
//! RezObject / ObjectProperties.ItemID follow the Legacy Viewer's
//! LLToolDragAndDrop::dropObject and LLSelectMgr::processObjectProperties.
//! Nothing is worn and the original inventory item is never removed.
pub mod geometry;
pub mod snapshot;
#[cfg(test)]
mod tests;

use super::{
    animation_model::{Model, Saved},
    *,
};
use crate::world::{
    World,
    inventory::{Inventory, actions},
    objects::{ObjKey, Object},
};
use aurora_net::{
    build::{BuildCmd, ObjectProps},
    inventory::{InvItem, rez::RezInventory},
};
use std::time::Instant;

fn tr(key: &'static str) -> String {
    aurora_i18n::text(key).into()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Origin {
    pub profile: String,
    pub item: Uuid,
}

#[derive(Default)]
pub struct State {
    pub profile: String,
    pub entry: Option<Uuid>,
    pub selected: Option<Uuid>,
    pub query: String,
    pub error: Option<String>,
    pub pending: Option<Pending>,
    pub picker: super::ui::animation_inventory::Picker,
    pub demo_mesh: Option<super::local::Mesh>,
    pub demo_seeded: bool,
}

pub const DEMO_ITEM: Uuid = Uuid::from_u128(0xA0E0_1283_0000_0000_0000_0000_0000_0001);
pub const DEMO_MESH: Uuid = Uuid::from_u128(0xA0E0_1283_0000_0000_0000_0000_0000_0002);

pub(super) fn demo_asset() -> Result<super::local::Mesh, String> {
    let skeleton =
        aurora_assets::Skeleton::parse(include_bytes!("../../../assets/character/avatar_skeleton.xml")).map_err(|e| e.to_string())?;
    let source = aurora_assets::import_mesh::collada::parse(super::demo::tail_dae().as_bytes(), &skeleton).map_err(|e| e.to_string())?;
    let mut draft = mesh::Draft::new(source, HashMap::new());
    draft.options.skin = true;
    let mut prepared = super::local::prepare(DEMO_ITEM, 0, &Source::Mesh(Box::new(draft)), Default::default(), None)?;
    let mut mesh = prepared.meshes.pop().ok_or_else(|| tr("import-animation-model-rigged"))?;
    mesh.id = DEMO_MESH;
    Ok(mesh)
}

pub struct Pending {
    pub entry: Uuid,
    pub generation: u64,
    pub saved: Saved,
    pub handle: u64,
    pub agent: Uuid,
    pub known: HashSet<Uuid>,
    pub requested: HashSet<Uuid>,
    pub textures: HashSet<Uuid>,
    pub root: Option<(Uuid, ObjKey)>,
    pub started: Instant,
    pub checked: Instant,
    pub stable: Instant,
    pub members: Vec<Uuid>,
    pub cancelled: bool,
    pub worker: Option<Receiver<Result<Arc<Model>, String>>>,
    pub result: Option<Result<Arc<Model>, String>>,
    pub cleaning: Option<Instant>,
}

pub fn available(inv: &Inventory, id: Uuid, agent: Uuid) -> Result<&InvItem, &'static str> {
    let original = actions::original(inv, id).ok_or("import-animation-inventory-item-error")?;
    let item = inv.items.get(&original).ok_or("import-animation-inventory-item-error")?;
    if item.asset_type != 6
        || item.inv_type != 6
        || item.owner != agent
        || item.group_owned
        || actions::library(inv, id)
        || [14, 46, 48, 50, 53]
            .iter()
            .any(|kind| inv.in_folder_type(id, *kind) || inv.in_folder_type(original, *kind))
    {
        return Err("import-animation-inventory-item-error");
    }
    if !crate::build::contents_drop::copyable(item, agent) {
        return Err("import-animation-inventory-copy-error");
    }
    if item.flags & crate::ui::inventory::OBJECT_HAS_MULTIPLE_ITEMS != 0 {
        return Err("import-animation-inventory-coalesced-error");
    }
    Ok(item)
}

impl Pending {
    /// Never guess by name, distance or selection alone. Nil ItemID and
    /// family properties are deliberately insufficient to delete an object.
    pub fn matches(&self, object: &Object, properties: Option<&ObjectProps>) -> bool {
        object.key.region == self.handle
            && object.parent_id == 0
            && !object.is_avatar()
            && object.update_flags & crate::build::FLAGS_CREATE_SELECTED != 0
            && !self.known.contains(&object.full_id)
            && properties.is_some_and(|p| {
                p.full
                    && p.object_id == object.full_id
                    && p.owner_id == self.agent
                    && Some(p.item_id) == self.saved.inventory.as_ref().map(|origin| origin.item)
            })
    }

    pub fn cleanup(&self, world: &World) -> Option<BuildCmd> {
        let (id, key) = self.root?;
        let object = world.objects.index_of_uuid(&id).and_then(|i| world.objects.get(i))?;
        // Local IDs are recycled; also reject a root that scripts attached or
        // moved into another linkset. Leave the resident a cleanup warning.
        if object.key != key || object.parent_id != 0 || !object.owner_id.is_nil() && object.owner_id != self.agent {
            return None;
        }
        let mut stack = vec![world.objects.index_of_uuid(&id)?];
        let mut visited = HashSet::new();
        while let Some(index) = stack.pop() {
            let object = world.objects.get(index)?;
            if !visited.insert(index) || visited.len() > 256 || self.known.contains(&object.full_id) {
                return None;
            }
            stack.extend(world.objects.children_of(&object.key));
        }
        Some(BuildCmd::DeRez {
            handle: key.region,
            local_ids: vec![key.local_id],
            destination: aurora_net::build::derez::TRASH,
            destination_id: actions::system(&world.inventory, 14).unwrap_or_default(),
            group_id: world.groups.active,
        })
    }
}

pub fn plan(
    world: &World,
    entry: Uuid,
    generation: u64,
    item: Uuid,
    profile: String,
    directory: PathBuf,
) -> Result<(Pending, BuildCmd), String> {
    let item = available(&world.inventory, item, world.agent_id).map_err(tr)?;
    if !world.movement_complete || world.teleporting || world.frozen {
        return Err(tr("rez-incomplete"));
    }
    let region = world.main().ok_or_else(|| tr("rez-incomplete"))?;
    let parcel = world.parcel.as_deref().ok_or_else(|| tr("import-animation-inventory-land-error"))?;
    let info = region.info.as_ref().ok_or_else(|| tr("import-animation-inventory-land-error"))?;
    if !crate::ui::parcel_icons::agent_can_build(&crate::ui::parcel_icons::ParcelState {
        region_flags: info.region_flags,
        parcel,
        agent_id: world.agent_id,
        group: world.groups.group(&parcel.group_id),
        health: world.health,
        frozen: world.frozen,
    }) {
        return Err(tr("import-animation-inventory-land-error"));
    }
    let offset = world.region_offset(region.handle).ok_or_else(|| tr("rez-no-surface"))?;
    let point = world.agent.position - offset;
    if !point.is_finite() {
        return Err(tr("rez-no-surface"));
    }
    let group = if world.groups.group(&parcel.group_id).is_some() {
        parcel.group_id
    } else {
        world.groups.active
    };
    let saved = Saved {
        id: Uuid::new_v4(),
        name: item.name.clone(),
        path: directory.join(profile.as_str()).join(format!("{}.apmodel", Uuid::new_v4())),
        inventory: Some(Origin { profile, item: item.id }),
    };
    let command = BuildCmd::RezInventory(Box::new(RezInventory {
        handle: region.handle,
        group,
        item: item.clone(),
        ray_start: point + glam::Vec3::Z * 2.0,
        ray_end: point,
        target: Uuid::nil(),
        selected: true,
        remove: false,
        source_folder: item.parent,
    }));
    let pending = Pending {
        entry,
        generation,
        saved,
        handle: region.handle,
        agent: world.agent_id,
        known: world.objects.iter().map(|(_, object)| object.full_id).collect(),
        requested: HashSet::new(),
        textures: HashSet::new(),
        root: None,
        started: Instant::now(),
        checked: Instant::now(),
        stable: Instant::now(),
        members: Vec::new(),
        cancelled: false,
        worker: None,
        result: None,
        cleaning: None,
    };
    Ok((pending, command))
}

impl Imports {
    pub fn complete_inventory_model(&mut self, id: Uuid, generation: u64, saved: Saved, result: Result<Arc<Model>, String>) {
        if !self.entries.iter().any(|entry| {
            entry.id == id && entry.animation_model.generation == generation && matches!(entry.source, Some(Source::Animation { .. }))
        }) {
            return;
        }
        if result.is_ok() {
            self.load_preferences.animation_models.push(saved.clone());
            self.animation_models_dirty = true;
        }
        self.complete_animation_model(id, generation, Some(result));
        if let Some(entry) = self.entries.iter_mut().find(|entry| entry.id == id)
            && entry.animation_model.current.as_ref().is_some_and(|m| m.path == saved.path)
        {
            // complete_animation_model deduplicates this already registered path.
            entry.animation_model.saved = Some(saved.id);
        }
    }
}
