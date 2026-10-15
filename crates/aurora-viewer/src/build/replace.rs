//! Guided replacement using LLSelectMgr::sendLink / sendDelink and
//! LLViewerObject::moveInventory / saveScript (Firestorm, LGPL 2.1).
//! Object contents go through agent inventory: direct world-to-world drops
//! are explicitly rejected by LLToolDragAndDrop (SL-22177).

pub mod demo;
#[cfg(test)]
mod tests;
mod transaction;

use super::{BuildSettings, BuildTool, Tool, family, root_of};
use crate::world::World;
use crate::world::objects::{ObjKey, Object};
use aurora_net::build::{BuildCmd, PhysicsParams, TaskItem, TransformUpdate};
use aurora_net::inventory::{InvItem, operations::Reply};
use glam::{Quat, Vec3};
use std::time::Instant;
use uuid::Uuid;

pub use transaction::{Job, Stage};

#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub position: bool,
    pub rotation: bool,
    pub size: bool,
    pub contents: bool,
    pub pause_scripts: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            position: true,
            rotation: true,
            size: false,
            contents: false,
            pause_scripts: true,
        }
    }
}

#[derive(Debug, Default)]
pub struct Replacement {
    pub options: Options,
    pub draft: Option<Draft>,
    pub job: Option<Job>,
    pub message: String,
}

#[derive(Debug)]
pub struct Draft {
    pub target: Uuid,
    pub number: usize,
    pub source: Option<Uuid>,
    pub picking: bool,
}

#[derive(Debug, Clone)]
pub struct Prim {
    pub id: Uuid,
    pub key: ObjKey,
    pub parent: u32,
    /// Region coordinates, including for children: the simulator preserves
    /// these when delinking and converts them to root-local on linking.
    pub position: Vec3,
    pub rotation: Quat,
    pub size: Vec3,
    pub flags: u32,
    pub physics: Option<PhysicsParams>,
}

impl Prim {
    fn capture(tool: &BuildTool, world: &World, idx: usize) -> Option<Self> {
        let o = world.objects.get(idx)?;
        let (position, rotation) = region_transform(world, idx)?;
        Some(Self {
            id: o.full_id,
            key: o.key,
            parent: o.parent_id,
            position,
            rotation,
            size: o.scale,
            flags: o.update_flags,
            physics: tool.physics.get(&o.key.local_id).copied(),
        })
    }

    pub fn update(&self) -> TransformUpdate {
        TransformUpdate {
            local_id: self.key.local_id,
            position: Some(self.position),
            rotation: Some(self.rotation),
            scale: Some(self.size),
            linked: false,
            uniform: false,
        }
    }

    pub fn matches(&self, world: &World) -> bool {
        let Some(idx) = world.objects.index_of_uuid(&self.id) else {
            return false;
        };
        let Some(o) = world.objects.get(idx) else { return false };
        let Some((position, rotation)) = region_transform(world, idx) else {
            return false;
        };
        position.abs_diff_eq(self.position, 0.002) && rotation.dot(self.rotation).abs() > 0.99999 && o.scale.abs_diff_eq(self.size, 0.002)
    }
}

/// Build from the actual parent transforms, without animation/extrapolation
/// or global-region offsets. Attachments and seated avatars are excluded.
pub fn region_transform(world: &World, idx: usize) -> Option<(Vec3, Quat)> {
    let mut chain = Vec::new();
    let mut current = idx;
    for _ in 0..64 {
        let o = world.objects.get(current)?;
        if o.is_avatar() {
            return None;
        }
        chain.push((o.position, o.rotation));
        if o.parent_id == 0 {
            let (mut pos, mut rot) = chain.pop()?;
            while let Some((p, r)) = chain.pop() {
                pos += rot * p;
                rot *= r;
            }
            return Some((pos, rot));
        }
        current = world.objects.parent_of(o)?;
    }
    None
}

pub fn object_name(tool: &BuildTool, world: &World, id: Uuid) -> String {
    tool.props
        .get(&id)
        .map(|p| p.name.clone())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            world
                .objects
                .index_of_uuid(&id)
                .and_then(|i| world.objects.get(i))
                .map(|o| o.text.lines().next().unwrap_or("").to_owned())
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| aurora_i18n::tr!("replace-unnamed").into())
}

pub fn task_from_item(item: &InvItem, object: Uuid) -> TaskItem {
    TaskItem {
        item_id: item.id,
        parent_id: object,
        creator_id: item.creator,
        owner_id: item.owner,
        last_owner_id: item.last_owner,
        group_id: item.group_id,
        group_owned: item.group_owned,
        base_mask: item.base_mask,
        owner_mask: item.owner_mask,
        group_mask: item.group_mask,
        everyone_mask: item.everyone_mask,
        next_owner_mask: item.next_owner_mask,
        asset_id: item.asset_id,
        asset_type: item.asset_type as i8,
        inv_type: item.inv_type as i8,
        flags: item.flags,
        sale_type: item.sale_type,
        sale_price: item.sale_price,
        name: item.name.clone(),
        description: item.desc.clone(),
        creation_date: item.created_at as i32,
        ..Default::default()
    }
}

impl BuildTool {
    /// ObjectUpdate.OwnerID is only reliable for sound/particles
    /// (LLViewerObject::processUpdateMessage). Use full ObjectProperties,
    /// otherwise the root's permission flags as permYouOwner/permModify do.
    /// The transaction reads fresh full properties again before any mutation.
    fn replacement_can_modify(&self, world: &World, idx: usize) -> bool {
        use crate::ui::context::flags::{OBJECT_MODIFY, OBJECT_YOU_OWNER};
        let Some(o) = world.objects.get(idx) else { return false };
        if let Some(props) = self.props.get(&o.full_id).filter(|props| props.full) {
            return props.owner_id == world.agent_id && props.owner_mask & aurora_net::build::perm::MODIFY != 0;
        }
        let Some(root) = world.objects.get(root_of(world, idx)) else {
            return false;
        };
        let required = OBJECT_MODIFY | OBJECT_YOU_OWNER;
        root.update_flags & required == required
    }

    pub fn replacement_visible(&self, world: &World, s: &BuildSettings) -> bool {
        self.tool == Tool::Edit
            && s.edit_linked
            && self.selection.len() == 1
            && self
                .selection
                .first()
                .and_then(|k| world.objects.index_of(k))
                .is_some_and(|i| family(world, root_of(world, i)).len() > 1)
    }

    pub fn replacement_active(&self) -> bool {
        self.replacement.draft.is_some() || self.replacement_busy()
    }

    pub fn replacement_busy(&self) -> bool {
        self.replacement.job.as_ref().is_some_and(|job| !job.stage.terminal())
    }

    pub fn begin_replacement(&mut self, world: &World, s: &BuildSettings) {
        if !self.replacement_visible(world, s) || self.replacement_busy() {
            return;
        }
        let Some(idx) = self.selection.first().and_then(|k| world.objects.index_of(k)) else {
            return;
        };
        let Some(o) = world.objects.get(idx) else { return };
        let members = family(world, root_of(world, idx));
        let Some(number) = members.iter().position(|i| *i == idx).map(|n| n + 1) else {
            return;
        };
        self.manip.cancel();
        self.rect = None;
        self.replacement.job = None;
        self.replacement.message.clear();
        self.replacement.draft = Some(Draft {
            target: o.full_id,
            number,
            source: None,
            picking: true,
        });
    }

    /// Capture the candidate without changing the target selection or
    /// letting the ordinary manipulator consume this click.
    pub fn pick_replacement(&mut self, world: &World, idx: Option<usize>) {
        let Some(draft) = self.replacement.draft.as_ref() else { return };
        let target = draft.target;
        let Some(i) = idx else { return };
        let Some(o) = world.objects.get(i) else { return };
        let error = if o.full_id == target {
            Some(aurora_i18n::tr!("replace-same-prim"))
        } else if o.is_avatar() || o.pcode != aurora_prim::params::LL_PCODE_VOLUME {
            Some(aurora_i18n::tr!("replace-volume-only"))
        } else if o.parent_id != 0 || !world.objects.children_of(&o.key).is_empty() {
            Some(aurora_i18n::tr!("replace-independent"))
        } else if !self.replacement_can_modify(world, i) {
            Some(aurora_i18n::tr!("replace-modify"))
        } else if world
            .objects
            .index_of_uuid(&target)
            .and_then(|t| world.objects.get(t))
            .is_none_or(|t| t.key.region != o.key.region)
        {
            Some(aurora_i18n::tr!("replace-same-region"))
        } else {
            None
        };
        if let Some(error) = error {
            self.replacement.message = error.into();
            return;
        }
        let (id, key) = (o.full_id, o.key);
        if let Some(old) = self.replacement.draft.as_ref().and_then(|d| d.source).filter(|old| *old != id)
            && let Some(o) = world.objects.index_of_uuid(&old).and_then(|i| world.objects.get(i))
        {
            self.send(BuildCmd::Deselect {
                handle: o.key.region,
                local_ids: vec![o.key.local_id],
            });
        }
        if let Some(draft) = self.replacement.draft.as_mut() {
            draft.source = Some(id);
            draft.picking = false;
        }
        self.replacement.message.clear();
        self.send(BuildCmd::Select {
            handle: key.region,
            local_ids: vec![key.local_id],
        });
    }

    pub fn cancel_replacement(&mut self, world: &World) {
        if self.replacement_busy() {
            return;
        }
        if let Some(source) = self.replacement.draft.as_ref().and_then(|d| d.source)
            && let Some(o) = world.objects.index_of_uuid(&source).and_then(|i| world.objects.get(i))
        {
            self.send(BuildCmd::Deselect {
                handle: o.key.region,
                local_ids: vec![o.key.local_id],
            });
        }
        self.replacement.draft = None;
        self.replacement.job = None;
        self.replacement.message.clear();
    }

    pub fn apply_replacement(&mut self, world: &World) {
        let Some(draft) = self.replacement.draft.as_ref() else { return };
        match Job::new(self, world, draft, self.replacement.options) {
            Ok(job) => {
                self.replacement.draft = None;
                self.replacement.message.clear();
                self.replacement.job = Some(job);
            }
            Err(error) => self.replacement.message = error,
        }
    }

    pub fn replacement_inventory_result(&mut self, request: Uuid, result: &Result<Reply, String>) -> bool {
        let Some(job) = self.replacement.job.as_mut() else { return false };
        if job.inventory_request != Some(request) {
            return false;
        }
        job.inventory_result = Some(result.clone());
        true
    }

    pub fn replacement_task_inventory_error(&mut self, object: Uuid, reason: String) {
        self.bulk_inventory_error(object);
        if let Some(inv) = self.contents.objects.get_mut(&object) {
            inv.loading = false;
        }
        if let Some(job) = self.replacement.job.as_mut()
            && !job.stage.terminal()
            && match job.stage {
                Stage::Read => job.inventory_ids().contains(&object),
                Stage::CopyIn(_) | Stage::RecoverContents => object == job.source,
                _ => false,
            }
        {
            job.inventory_error = Some((
                object,
                aurora_i18n::tr!("replace-inventory-read-failed", stage = job.stage.label(), reason = reason),
            ));
        }
    }

    pub fn update_replacement(&mut self, world: &mut World, now: Instant) {
        let Some(mut job) = self.replacement.job.take() else { return };
        if !job.stage.terminal() {
            job.tick(self, world, now);
        }
        self.replacement.job = Some(job);
    }
}

/// A demo simulator needs full updates to exercise the same parent-change
/// path as network traffic, rather than editing the viewer's links locally.
pub(crate) fn demo_update(o: &Object) -> aurora_net::objects::ObjectUpdate {
    aurora_net::objects::ObjectUpdate {
        local_id: o.key.local_id,
        full_id: o.full_id,
        parent_id: o.parent_id,
        pcode: o.pcode,
        state: o.state,
        crc: 0,
        material: o.prim_material,
        click_action: o.click_action,
        scale: o.scale,
        position: o.position,
        rotation: o.rotation,
        velocity: o.velocity,
        acceleration: o.acceleration,
        angular_velocity: o.angular_velocity,
        update_flags: o.update_flags,
        owner_id: o.owner_id,
        volume: o.volume,
        texture_entry: o.te.clone(),
        extra: o.extra.clone(),
        name_values: o.name_values.clone(),
        text: o.text.clone(),
        text_color: o.text_color,
        media_url: o.media_url.clone(),
        tree_species: o.tree_species,
        texture_anim: o.tex_anim,
        foot_plane: None,
        particles: Default::default(),
        sound: o.sound,
    }
}
