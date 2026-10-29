//! Aurora's per-HUD visual zoom. Simulator transforms remain untouched.
//! Inventory item UUIDs survive reattachment; object name + creator identify copies
//! and temporary experience attachments even when there is no inventory item.
//! The profile file is per account and still accepts the original name/creator format.
use super::World;
use glam::Vec3;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use uuid::Uuid;

pub const FILE: &str = "hud_scales.json";
pub const MIN: f32 = 0.1;
pub const MAX: f32 = 8.0;

pub fn wheel_scale(scale: f32, steps: f32, fine: bool) -> f32 {
    if !steps.is_finite() || !scale.is_finite() {
        return scale;
    }
    (scale * (if fine { 1.02_f32 } else { 1.1_f32 }).powf(steps.clamp(-100.0, 100.0))).clamp(MIN, MAX)
}

fn request_due(last: &mut Option<Instant>, now: Instant) -> bool {
    if last.is_some_and(|last| now.saturating_duration_since(last) < Duration::from_secs(10)) {
        return false;
    }
    *last = Some(now);
    true
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    name: String,
    creator: Uuid,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Profile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    item: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    identity: Option<Identity>,
    scale: f32,
    /// An inherited inventory fallback may be superseded by object metadata.
    #[serde(default)]
    inherited: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    aliases: Vec<Identity>,
}

#[derive(Default)]
struct Local {
    scale: f32,
    item: Option<Uuid>,
    identity: Option<Identity>,
    object_identity: Option<Identity>,
    aliases: Vec<Identity>,
    edited: bool,
    forgotten: bool,
    offset: Vec3,
    requested_item: Option<Instant>,
    requested_properties: Option<Instant>,
    needs_clamp: bool,
}

#[derive(Default)]
pub struct HudScales {
    profiles: Vec<Profile>,
    local: HashMap<Uuid, Local>,
    writer: Option<Sender<Vec<Profile>>>,
    worker: Option<JoinHandle<()>>,
    viewport: Option<[f32; 2]>,
}

impl Drop for HudScales {
    fn drop(&mut self) {
        self.writer.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl HudScales {
    pub fn request_properties(&mut self, id: Uuid, now: Instant) -> bool {
        let Some(s) = self.local.get_mut(&id) else { return false };
        if s.object_identity.is_some() {
            return false;
        }
        request_due(&mut s.requested_properties, now)
    }

    fn on_properties(&mut self, props: &[aurora_net::build::ObjectProps]) {
        for p in props {
            if p.full
                && !p.creator_id.is_nil()
                && !p.name.trim().is_empty()
                && let Some(s) = self.local.get_mut(&p.object_id)
            {
                s.object_identity = Some(Identity {
                    name: p.name.trim().to_owned(),
                    creator: p.creator_id,
                });
            }
        }
    }

    pub fn request_item(&mut self, id: Uuid, now: Instant) -> bool {
        let Some(s) = self.local.get_mut(&id) else { return false };
        s.object_identity.is_none() && request_due(&mut s.requested_item, now)
    }
    pub fn load(path: PathBuf) -> Self {
        let profiles: Vec<Profile> = match std::fs::read(&path) {
            Ok(bytes) => match crate::tolerant::from_slice::<Vec<Profile>>(&bytes, "hud_scales.json") {
                Some(read) => {
                    if read.repaired() {
                        // rewritten without the sizes this build cannot read
                        crate::tolerant::backup(&path);
                    }
                    read.value
                }
                None => {
                    crate::tolerant::backup(&path);
                    Vec::new()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => {
                log::warn!("cannot read HUD scales: {e}");
                Vec::new()
            }
        };
        let profiles = profiles
            .into_iter()
            .filter(|p| p.scale.is_finite())
            .map(|mut p| {
                p.item = p.item.filter(|id| !id.is_nil());
                p.identity = p.identity.filter(|i| !i.name.is_empty() && !i.creator.is_nil());
                p.aliases.retain(|i| !i.name.is_empty() && !i.creator.is_nil());
                p.scale = p.scale.clamp(MIN, MAX);
                p
            })
            .filter(|p| p.item.is_some() || p.identity.is_some())
            .collect();
        let (tx, rx) = mpsc::channel::<Vec<Profile>>();
        let worker = std::thread::Builder::new().name("aurora-hud-scales".into()).spawn(move || {
            while let Ok(mut profiles) = rx.recv() {
                while let Ok(newer) = rx.try_recv() {
                    profiles = newer;
                }
                let save = || -> Result<(), Box<dyn std::error::Error>> {
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    let tmp = path.with_extension("json.tmp");
                    std::fs::write(&tmp, serde_json::to_vec_pretty(&profiles)?)?;
                    std::fs::rename(tmp, &path)?;
                    Ok(())
                };
                if let Err(e) = save() {
                    log::warn!("cannot save HUD scales: {e}");
                }
            }
        });
        match worker {
            Ok(worker) => Self {
                profiles,
                writer: Some(tx),
                worker: Some(worker),
                local: HashMap::new(),
                viewport: None,
            },
            Err(e) => {
                log::warn!("cannot start HUD scale writer: {e}");
                Self {
                    profiles,
                    local: HashMap::new(),
                    writer: None,
                    worker: None,
                    viewport: None,
                }
            }
        }
    }

    fn save(&self) {
        if let Some(writer) = &self.writer {
            let _ = writer.send(self.profiles.clone());
        }
    }

    fn saved_scale(&self, item: Option<Uuid>, identity: Option<&Identity>) -> Option<f32> {
        let exact = item.and_then(|item| self.profiles.iter().rev().find(|p| p.item == Some(item)));
        let shared = || {
            identity.and_then(|identity| {
                self.profiles
                    .iter()
                    .rev()
                    .find(|p| p.item.is_none() && p.identity.as_ref() == Some(identity))
            })
        };
        // Inventory labels can differ between copies. A provisional inheritance
        // must yield to the common object identity when its metadata arrives.
        if exact.is_some_and(|p| p.inherited && p.identity.as_ref() != identity) {
            shared().or(exact)
        } else {
            exact.or_else(shared)
        }
        .map(|p| p.scale)
    }

    fn remember(&mut self, item: Option<Uuid>, identity: Option<Identity>, scale: f32, aliases: &[Identity]) {
        if item.is_none() && identity.is_none() {
            return;
        }
        // The item profile keeps independently resized copies and survives a
        // rename or missing creator metadata. The generic profile retains
        // Aurora's name + creator fallback for genuinely new inventory copies.
        let mut changed = item.is_some_and(|item| self.remember_item(item, identity.clone(), scale, false, aliases));
        let primary = identity.clone();
        for identity in identity.into_iter().chain(aliases.iter().cloned()) {
            let position = self
                .profiles
                .iter()
                .position(|p| p.item.is_none() && p.identity.as_ref() == Some(&identity));
            let index = if let Some(index) = position {
                index
            } else {
                self.profiles.push(Profile {
                    item: None,
                    identity: Some(identity.clone()),
                    scale,
                    inherited: false,
                    aliases: Vec::new(),
                });
                changed = true;
                self.profiles.len() - 1
            };
            let p = &mut self.profiles[index];
            changed |= p.scale != scale;
            p.scale = scale;
            // The common object profile links its legacy inventory labels; each
            // label links back. Store the family once, not on every worn copy.
            let related: &[Identity] = if primary.as_ref() == Some(&identity) {
                aliases
            } else {
                primary.as_slice()
            };
            for alias in related {
                if alias != &identity && !p.aliases.contains(alias) {
                    p.aliases.push(alias.clone());
                    changed = true;
                }
            }
        }
        if changed {
            self.save();
        }
    }

    fn remember_item(&mut self, item: Uuid, identity: Option<Identity>, scale: f32, inherited: bool, aliases: &[Identity]) -> bool {
        if let Some(p) = self.profiles.iter_mut().find(|p| p.item == Some(item)) {
            let changed = p.scale != scale || p.identity != identity || p.inherited != inherited || p.aliases != aliases;
            p.identity = identity;
            p.scale = scale;
            p.inherited = inherited;
            p.aliases = aliases.to_vec();
            changed
        } else {
            self.profiles.push(Profile {
                item: Some(item),
                identity,
                scale,
                inherited,
                aliases: aliases.to_vec(),
            });
            true
        }
    }

    pub fn scale(&self, id: Uuid) -> f32 {
        self.local.get(&id).map_or(1.0, |s| s.scale)
    }

    pub fn offset(&self, id: Uuid) -> Vec3 {
        self.local.get(&id).map_or(Vec3::ZERO, |s| s.offset)
    }

    pub fn set_offset(&mut self, id: Uuid, offset: Vec3) {
        if let Some(s) = self.local.get_mut(&id) {
            s.offset = offset;
        }
    }

    pub fn prepare_clamp(&mut self, viewport: [f32; 2]) {
        if self.viewport != Some(viewport) {
            self.viewport = Some(viewport);
            for s in self.local.values_mut() {
                s.needs_clamp = true;
            }
        }
    }

    pub fn needs_clamp(&self, id: Uuid) -> bool {
        self.local.get(&id).is_some_and(|s| s.needs_clamp && s.scale != 1.0)
    }

    pub fn clamped(&mut self, id: Uuid) {
        if let Some(s) = self.local.get_mut(&id) {
            s.needs_clamp = false;
        }
    }

    pub fn set(&mut self, id: Uuid, scale: f32) {
        if !scale.is_finite() {
            return;
        }
        let s = self.local.entry(id).or_insert_with(|| Local {
            scale: 1.0,
            ..Default::default()
        });
        s.scale = scale.clamp(MIN, MAX);
        s.needs_clamp = true;
        s.edited = true;
        s.forgotten = false;
        let (item, identity, scale, aliases) = (s.item, s.identity.clone(), s.scale, s.aliases.clone());
        self.remember(item, identity, scale, &aliases);
    }

    pub fn has_profile(&self, id: Uuid) -> bool {
        self.local
            .get(&id)
            .is_some_and(|s| self.saved_scale(s.item, s.identity.as_ref()).is_some())
    }

    /// The current copy keeps its zoom; subsequent copies start at 100%.
    pub fn forget(&mut self, id: Uuid) {
        if let Some(s) = self.local.get_mut(&id) {
            s.edited = false;
            s.forgotten = true;
            let (item, identity, aliases) = (s.item, s.identity.clone(), s.aliases.clone());
            self.forget_saved(item, identity.as_ref(), &aliases);
        }
    }

    fn forget_saved(&mut self, item: Option<Uuid>, identity: Option<&Identity>, aliases: &[Identity]) {
        let before = self.profiles.len();
        let mut identities = aliases.to_vec();
        identities.extend(identity.cloned());
        let mut index = 0;
        while index < identities.len() {
            if let Some(p) = self
                .profiles
                .iter()
                .find(|p| p.item.is_none() && p.identity.as_ref() == Some(&identities[index]))
            {
                for alias in &p.aliases {
                    if !identities.contains(alias) {
                        identities.push(alias.clone());
                    }
                }
            }
            index += 1;
        }
        self.profiles.retain(|p| {
            !(item.is_some() && p.item == item || p.item.is_none() && p.identity.as_ref().is_some_and(|i| identities.contains(i)))
        });
        if self.profiles.len() != before {
            self.save();
        }
    }

    fn refresh(&mut self, roots: &[(Uuid, Option<Uuid>, Option<Identity>)]) {
        self.local.retain(|id, _| roots.iter().any(|(root, _, _)| root == id));
        for (id, item, identity) in roots {
            let s = self.local.entry(*id).or_insert_with(|| Local {
                scale: 1.0,
                ..Default::default()
            });
            // An ObjectUpdate or a delayed inventory snapshot may temporarily
            // omit metadata. Keep what this worn instance already told us.
            let identity = identity
                .clone()
                .or_else(|| (item.is_none() || *item == s.item).then(|| s.identity.clone()).flatten());
            let item = item.or(s.item);
            if s.item == item && s.identity == identity {
                continue;
            }
            // Keep the inventory labels associated with this object identity.
            // They bridge old profiles and must be forgotten with the shared size.
            let previous = s.identity.clone();
            let profile = item.and_then(|item| self.profiles.iter().find(|p| p.item == Some(item)));
            let known = profile
                .into_iter()
                .flat_map(|p| p.aliases.iter().cloned().chain(p.identity.clone()));
            for alias in previous.into_iter().chain(known) {
                if Some(&alias) != identity.as_ref() && !s.aliases.contains(&alias) {
                    s.aliases.push(alias);
                }
            }
            let aliases = s.aliases.clone();
            let object_identity = s.object_identity.is_some();
            s.item = item;
            s.identity = identity.clone();
            if s.forgotten {
                self.forget_saved(item, identity.as_ref(), &aliases);
            } else if s.edited {
                let scale = s.scale;
                self.remember(item, identity, scale, &aliases);
            } else {
                let saved = self.saved_scale(item, identity.as_ref());
                if let Some(s) = self.local.get_mut(id) {
                    s.scale = saved.unwrap_or(1.0);
                    s.needs_clamp = true;
                }
                // Bind inherited legacy/copy profiles once. Merely restoring
                // an exact item must not change the default for future copies.
                if let (Some(item), Some(scale)) = (item, saved) {
                    let explicit = self.profiles.iter().any(|p| p.item == Some(item) && !p.inherited);
                    let new_identity = identity.is_some() && self.profiles.iter().any(|p| p.item == Some(item) && p.identity != identity);
                    if explicit && new_identity {
                        self.remember(Some(item), identity, scale, &aliases);
                    } else {
                        // Migrate a legacy inventory preference to the common
                        // object identity without marking an inherited copy edited.
                        let shared = self.profiles.iter().any(|p| p.item.is_none() && p.identity == identity);
                        if object_identity && !shared {
                            self.remember(None, identity.clone(), scale, &aliases);
                        }
                        if self.remember_item(item, identity, scale, !explicit, &aliases) {
                            self.save();
                        }
                    }
                }
            }
        }
    }
}

impl World {
    pub fn on_hud_properties(&mut self, props: &[aurora_net::build::ObjectProps]) {
        self.refresh_hud_scales();
        self.hud_scales.on_properties(props);
        self.refresh_hud_scales();
    }

    /// Only attachments of the local avatar can carry a local zoom.
    pub fn hud_root(&self, mut idx: usize) -> Option<usize> {
        for _ in 0..16 {
            let o = self.objects.get(idx)?;
            let parent = self.objects.parent_of(o)?;
            let p = self.objects.get(parent)?;
            if p.is_avatar() {
                return (p.full_id == self.agent_id && (31..=38).contains(&o.attachment_point())).then_some(idx);
            }
            idx = parent;
        }
        None
    }

    pub fn hud_scale(&self, idx: usize) -> f32 {
        if self.hud_scales.local.is_empty() {
            return 1.0;
        }
        self.hud_root(idx)
            .and_then(|root| self.objects.get(root))
            .map_or(1.0, |o| self.hud_scales.scale(o.full_id))
    }

    pub fn hud_move_locked(&self, idx: usize) -> bool {
        self.hud_root(idx)
            .and_then(|root| self.objects.get(root))
            .is_some_and(|o| self.hud_locks.locked(o.full_id))
    }

    pub fn visual_scale(&self, idx: usize) -> Vec3 {
        self.objects.get(idx).map_or(Vec3::ONE, |o| o.damped_scale() * self.hud_scale(idx))
    }

    pub fn visual_scale_at(&self, idx: usize, now: Instant) -> Vec3 {
        self.objects
            .get(idx)
            .map_or(Vec3::ONE, |o| o.damped_scale_at(now) * self.hud_scale(idx))
    }

    /// Root translation stays unchanged; linked offsets follow the zoom.
    pub fn hud_position_scale(&self, idx: usize) -> f32 {
        self.hud_root(idx).filter(|root| *root != idx).map_or(1.0, |_| self.hud_scale(idx))
    }

    pub fn dirty_hud(&mut self, root: usize) {
        for idx in crate::build::family(self, root) {
            self.objects.touch(idx);
        }
    }

    pub fn refresh_hud_scales(&mut self) {
        let roots: Vec<_> = self
            .objects
            .index_of_uuid(&self.agent_id)
            .and_then(|idx| self.objects.get(idx))
            .map(|me| {
                self.objects
                    .children_of(&me.key)
                    .iter()
                    .filter_map(|idx| {
                        let o = self.objects.get(*idx)?;
                        if !(31..=38).contains(&o.attachment_point()) {
                            return None;
                        }
                        // LLViewerObject::extractAttachmentItemID (Firestorm
                        // llviewerobject.cpp, originally LGPL 2.1): this is the
                        // stable inventory UUID, unlike the rezzed object UUID.
                        // LLViewerObject::isTempAttachment: experiences use the
                        // current rezzed UUID as AttachItemID, not an inventory key.
                        let item = o
                            .attachment_item_id()
                            .filter(|id| !id.is_nil() && (*id != o.full_id || self.inventory.items.contains_key(id)));
                        let inventory_identity = item
                            .and_then(|id| self.inventory.items.get(&id))
                            .filter(|item| !item.creator.is_nil() && !item.name.trim().is_empty())
                            .map(|item| Identity {
                                name: item.name.trim().to_owned(),
                                creator: item.creator,
                            });
                        let identity = self
                            .hud_scales
                            .local
                            .get(&o.full_id)
                            .and_then(|s| s.object_identity.clone())
                            .or(inventory_identity);
                        Some((o.full_id, item, identity))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let before: Vec<_> = roots.iter().map(|(id, _, _)| self.hud_scales.scale(*id)).collect();
        self.hud_scales.refresh(&roots);
        let lock_roots: Vec<_> = roots.iter().map(|(id, item, _)| (*id, *item)).collect();
        self.hud_locks.refresh(&lock_roots);
        for ((id, _, _), before) in roots.iter().zip(before) {
            if before != self.hud_scales.scale(*id)
                && let Some(root) = self.objects.index_of_uuid(id)
            {
                self.dirty_hud(root);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_helpers_ignore_local_previews_but_keep_network_damping() {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let now = Instant::now();
        let mut prim = crate::demo::hud::object(31, false);
        prim.parent_id = 0;
        prim.state = 0;
        prim.scale = Vec3::ONE;
        let idx = world.objects.upsert_at(1, prim.clone(), Some(now));
        world.objects.get_mut(idx).expect("prim").render.local_scale = Some(Vec3::splat(9.0));
        prim.scale = Vec3::splat(2.0);
        world.objects.upsert_at(1, prim, Some(now));
        let half = now + Duration::from_millis(60);
        world.objects.expire_corrections(half);
        // The existing world helpers retain network dimensions, while the
        // renderer's object overlay keeps its previous preview behavior.
        assert_eq!(world.visual_scale(idx), Vec3::splat(1.5));
        assert_eq!(world.visual_scale_at(idx, half), Vec3::splat(1.5));
        assert_eq!(world.objects.get(idx).expect("prim").visual_scale_at(half), Vec3::splat(9.0));
        assert_eq!(world.objects.get(idx).expect("prim").scale, Vec3::splat(2.0));

        let mut world = hud_world();
        let root = world
            .objects
            .index_of_uuid(&crate::demo::hud::object(31, false).full_id)
            .expect("HUD");
        let id = world.objects.get(root).expect("HUD").full_id;
        let reported = world.objects.get(root).expect("HUD").scale;
        world.hud_scales.set(id, 2.0);
        world.objects.get_mut(root).expect("HUD").render.local_scale = Some(Vec3::splat(9.0));
        assert_eq!(world.visual_scale(root), reported * 2.0);
        assert_eq!(world.visual_scale_at(root, half), reported * 2.0);
    }

    fn props(id: Uuid, name: &str, creator: u128) -> aurora_net::build::ObjectProps {
        aurora_net::build::ObjectProps {
            object_id: id,
            name: name.into(),
            creator_id: Uuid::from_u128(creator),
            full: true,
            ..Default::default()
        }
    }

    fn hud_world() -> World {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for event in crate::demo::events().into_iter().chain(crate::demo::hud::events()) {
            world.apply(event);
        }
        world.hud_scales = HudScales::default();
        crate::demo::hud::seed_inventory(&mut world);
        // Ordinary inventory UUIDs differ from rezzed UUIDs. Matching UUIDs
        // identify temporary attachments in LLViewerObject::isTempAttachment.
        for point in 31..=38 {
            let id = crate::demo::hud::object(point, false).full_id;
            let mut item = world.inventory.items.remove(&id).unwrap();
            item.id = inventory_id(id);
            let root = world.objects.index_of_uuid(&id).unwrap();
            world.objects.get_mut(root).unwrap().name_values = format!("AttachItemID STRING RW SV {}", item.id);
            world.inventory.add_items(vec![item]);
        }
        world.refresh_hud_scales();
        world
    }

    fn inventory_id(object: Uuid) -> Uuid {
        Uuid::from_u128(object.as_u128() ^ (1 << 100))
    }

    fn reattach(world: &mut World, point: u8, instance: u128) -> usize {
        let mut update = crate::demo::hud::object(point, false);
        let old = world.objects.index_of_uuid(&update.full_id).unwrap();
        let handle = world.objects.get(old).unwrap().key.region;
        update.name_values = world.objects.get(old).unwrap().name_values.clone();
        world.objects.remove_idx(old);
        world.refresh_hud_scales();
        update.full_id = Uuid::from_u128(instance);
        world.apply(aurora_net::NetEvent::ObjectUpdates {
            handle,
            objects: vec![update],
        });
        world.refresh_hud_scales();
        world.objects.index_of_uuid(&Uuid::from_u128(instance)).unwrap()
    }

    #[test]
    fn copied_hud_with_a_different_inventory_label_restores_by_object_name_and_creator() {
        let mut world = hud_world();
        let a = crate::demo::hud::object(31, false).full_id;
        let b = crate::demo::hud::object(35, false).full_id;
        world.on_hud_properties(&[props(a, "Shared HUD", 10)]);
        world.hud_scales.set(a, 1.7);
        world.inventory.items.get_mut(&inventory_id(b)).unwrap().name = "My renamed inventory copy".into();
        world.inventory.items.get_mut(&inventory_id(b)).unwrap().creator = Uuid::nil();
        world.refresh_hud_scales();
        assert_eq!(world.hud_scales.scale(b), 1.0);
        world.on_hud_properties(&[props(b, "Shared HUD", 10)]);
        assert_eq!(world.hud_scales.scale(b), 1.7);
        world.hud_scales.set(b, 0.6);
        let root = reattach(&mut world, 35, 900_010);
        world.on_hud_properties(&[props(Uuid::from_u128(900_010), "Shared HUD", 10)]);
        assert_eq!(world.hud_scale(root), 0.6);
        assert_eq!(world.hud_scales.scale(a), 1.7);
    }

    #[test]
    fn object_metadata_supersedes_an_inherited_inventory_profile_but_not_an_explicit_choice() {
        let mut world = hud_world();
        let a = crate::demo::hud::object(31, false).full_id;
        let b = crate::demo::hud::object(35, false).full_id;
        let item = world.inventory.items[&inventory_id(b)].clone();
        world
            .hud_scales
            .remember(None, Some(identity(&item.name, item.creator.as_u128())), 1.2, &[]);
        world.hud_scales.local.remove(&b);
        world.refresh_hud_scales();
        assert_eq!(world.hud_scales.scale(b), 1.2);
        world.on_hud_properties(&[props(a, "Shared HUD", 10)]);
        world.hud_scales.set(a, 1.7);
        world.on_hud_properties(&[props(b, "Shared HUD", 10)]);
        assert_eq!(world.hud_scales.scale(b), 1.7);
        world.hud_scales.set(b, 0.6);
        world.on_hud_properties(&[props(b, "Shared HUD", 11)]);
        assert_eq!(world.hud_scales.scale(b), 0.6);
    }

    #[test]
    fn legacy_inventory_sizes_migrate_to_copies_and_forgetting_removes_their_aliases() {
        let mut world = hud_world();
        let a = crate::demo::hud::object(31, false).full_id;
        let b = crate::demo::hud::object(35, false).full_id;
        let item = world.inventory.items[&inventory_id(a)].clone();
        world
            .hud_scales
            .remember(None, Some(identity(&item.name, item.creator.as_u128())), 1.7, &[]);
        world.hud_scales.local.remove(&a);
        world.refresh_hud_scales();
        world.on_hud_properties(&[props(a, "Shared HUD", 10), props(b, "Shared HUD", 10)]);
        assert_eq!(world.hud_scales.scale(a), 1.7);
        assert_eq!(world.hud_scales.scale(b), 1.7);
        let saved = serde_json::to_vec(&world.hud_scales.profiles).unwrap();
        let mut restored = HudScales::default();
        restored.profiles = serde_json::from_slice(&saved).unwrap();
        restored.refresh(&[(Uuid::from_u128(900_020), Some(inventory_id(b)), Some(identity("Shared HUD", 10)))]);
        restored.forget(Uuid::from_u128(900_020));
        restored.refresh(&[]);
        restored.refresh(&[(Uuid::from_u128(900_021), None, Some(identity(&item.name, item.creator.as_u128())))]);
        assert_eq!(restored.scale(Uuid::from_u128(900_021)), 1.0);
        world.hud_scales.forget(a);
        assert_eq!(world.hud_scales.scale(a), 1.7);
        assert_eq!(world.hud_scales.scale(b), 1.7);
        let root = reattach(&mut world, 31, 900_022);
        world.on_hud_properties(&[props(Uuid::from_u128(900_022), "Shared HUD", 10)]);
        assert_eq!(world.hud_scale(root), 1.0);
    }

    #[test]
    fn temporary_experience_huds_restore_with_no_inventory_item_and_with_foreign_item_ids() {
        for item_kind in 0..3 {
            let mut world = hud_world();
            let a = crate::demo::hud::object(31, false).full_id;
            let root = world.objects.index_of_uuid(&a).unwrap();
            let handle = world.objects.get(root).unwrap().key.region;
            world.inventory.items.remove(&inventory_id(a));
            world.objects.get_mut(root).unwrap().name_values = match item_kind {
                1 => format!("AttachItemID STRING RW SV {}", Uuid::from_u128(999_001)),
                2 => format!("AttachItemID STRING RW SV {a}"),
                _ => String::new(),
            };
            world.hud_scales = HudScales::default();
            world.on_hud_properties(&[props(a, "Experience HUD", 10)]);
            world.hud_scales.set(a, 1.7);
            world.objects.remove_idx(root);
            world.refresh_hud_scales();
            let mut update = crate::demo::hud::object(31, false);
            update.full_id = Uuid::from_u128(900_011);
            update.name_values = match item_kind {
                1 => format!("AttachItemID STRING RW SV {}", Uuid::from_u128(999_002)),
                2 => format!("AttachItemID STRING RW SV {}", update.full_id),
                _ => String::new(),
            };
            world.apply(aurora_net::NetEvent::ObjectUpdates {
                handle,
                objects: vec![update],
            });
            world.on_hud_properties(&[props(Uuid::from_u128(900_011), "Experience HUD", 10)]);
            assert_eq!(world.hud_scales.scale(Uuid::from_u128(900_011)), 1.7);
            if item_kind == 2 {
                assert!(
                    world
                        .hud_scales
                        .profiles
                        .iter()
                        .all(|p| p.item != Some(a) && p.item != Some(Uuid::from_u128(900_011)))
                );
            }
            let other = crate::demo::hud::object(35, false).full_id;
            world.on_hud_properties(&[props(other, "Experience HUD", 11)]);
            assert_eq!(world.hud_scales.scale(other), 1.0);
        }
    }

    #[test]
    fn resizing_before_temporary_hud_properties_arrive_is_saved_and_partial_replies_are_ignored() {
        let mut world = hud_world();
        let id = crate::demo::hud::object(31, false).full_id;
        let root = world.objects.index_of_uuid(&id).unwrap();
        world.objects.get_mut(root).unwrap().name_values.clear();
        world.hud_scales = HudScales::default();
        world.refresh_hud_scales();
        world.hud_scales.set(id, 1.7);
        let mut partial = props(id, "Experience HUD", 10);
        partial.full = false;
        world.on_hud_properties(&[partial]);
        assert!(!world.hud_scales.has_profile(id));
        world.on_hud_properties(&[props(id, "Experience HUD", 10)]);
        assert!(world.hud_scales.has_profile(id));
        world.on_hud_properties(&[props(Uuid::from_u128(8241), "Experience HUD", 10)]);
        assert!(!world.hud_scales.local.contains_key(&Uuid::from_u128(8241)));
        world.hud_scales.refresh(&[]);
        world
            .hud_scales
            .refresh(&[(Uuid::from_u128(900_012), None, Some(identity("Experience HUD", 10)))]);
        assert_eq!(world.hud_scales.scale(Uuid::from_u128(900_012)), 1.7);
    }

    #[test]
    fn temporary_hud_shared_profile_survives_restart_without_any_inventory_uuid() {
        let folder = std::env::temp_dir().join(format!("aurora-hud-temp-{}", Uuid::new_v4()));
        let path = folder.join(FILE);
        let mut world = hud_world();
        let id = crate::demo::hud::object(31, false).full_id;
        let root = world.objects.index_of_uuid(&id).unwrap();
        world.objects.get_mut(root).unwrap().name_values.clear();
        world.hud_scales = HudScales::load(path.clone());
        world.on_hud_properties(&[props(id, "Experience HUD", 10)]);
        world.hud_scales.set(id, 1.7);
        drop(world);
        let mut restored = HudScales::load(path.clone());
        restored.refresh(&[(Uuid::from_u128(900_014), None, Some(identity("Experience HUD", 10)))]);
        assert_eq!(restored.scale(Uuid::from_u128(900_014)), 1.7);
        drop(restored);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(folder).unwrap();
    }

    #[test]
    fn reattached_inventory_item_restores_scale_without_creator_metadata() {
        let mut world = hud_world();
        let id = crate::demo::hud::object(31, false).full_id;
        world.inventory.items.get_mut(&inventory_id(id)).unwrap().creator = Uuid::nil();
        world.hud_scales = HudScales::default();
        world.refresh_hud_scales();
        world.hud_scales.set(id, 1.7);
        let root = reattach(&mut world, 31, 900_001);
        assert_eq!(world.hud_scale(root), 1.7);
    }

    #[test]
    fn reattached_inventory_item_restores_scale_after_rename() {
        let mut world = hud_world();
        let id = crate::demo::hud::object(31, false).full_id;
        world.hud_scales.set(id, 1.7);
        world.inventory.items.get_mut(&inventory_id(id)).unwrap().name = "Renamed HUD".into();
        let root = reattach(&mut world, 31, 900_002);
        assert_eq!(world.hud_scale(root), 1.7);
    }

    #[test]
    fn same_name_inventory_items_restore_their_independent_sizes() {
        let mut world = hud_world();
        let a = crate::demo::hud::object(31, false).full_id;
        let b = crate::demo::hud::object(35, false).full_id;
        let name = world.inventory.items[&inventory_id(a)].name.clone();
        world.inventory.items.get_mut(&inventory_id(b)).unwrap().name = name;
        world.refresh_hud_scales();
        world.hud_scales.set(a, 1.7);
        world.hud_scales.set(b, 0.6);
        let root = reattach(&mut world, 31, 900_003);
        assert_eq!(world.hud_scale(root), 1.7);
        assert_eq!(world.hud_scales.scale(b), 0.6);
    }

    #[test]
    fn reattached_item_restores_before_inventory_arrives() {
        let mut world = hud_world();
        let id = crate::demo::hud::object(31, false).full_id;
        world.inventory.items.remove(&inventory_id(id));
        world.hud_scales = HudScales::default();
        world.refresh_hud_scales();
        world.hud_scales.set(id, 1.7);
        assert!(world.hud_scales.has_profile(id));
        let root = reattach(&mut world, 31, 900_004);
        assert_eq!(world.hud_scale(root), 1.7);
    }

    #[test]
    fn delayed_profile_resolution_refreshes_the_whole_linkset_without_server_changes() {
        let mut world = hud_world();
        let id = crate::demo::hud::object(31, false).full_id;
        let item = world.inventory.items.remove(&inventory_id(id)).unwrap();
        world.hud_scales = HudScales::default();
        world
            .hud_scales
            .remember(None, Some(identity(&item.name, item.creator.as_u128())), 1.7, &[]);
        world.refresh_hud_scales();
        let root = world.objects.index_of_uuid(&id).unwrap();
        let family = crate::build::family(&world, root);
        assert_eq!(family.len(), 2);
        let server_scales: Vec<_> = family.iter().map(|idx| world.objects.get(*idx).unwrap().scale).collect();
        world.objects.take_touched(&mut Vec::new());
        world.inventory.add_items(vec![item]);
        world.refresh_hud_scales();
        for (idx, server_scale) in family.into_iter().zip(server_scales) {
            let object = world.objects.get(idx).unwrap();
            assert!(world.objects.touched().contains(&idx));
            assert_eq!(object.scale, server_scale);
            assert_eq!(world.hud_scale(idx), 1.7);
        }
    }

    #[test]
    fn delayed_metadata_keeps_an_edited_size_and_missing_updates_keep_known_identity() {
        let [a, b, item] = [1, 2, 101].map(Uuid::from_u128);
        let identity = identity("HUD", 10);
        let mut scales = HudScales::default();
        scales.remember(None, Some(identity.clone()), 2.0, &[]);
        scales.refresh(&[(a, Some(item), None)]);
        scales.set(a, 1.7);
        scales.refresh(&[(a, Some(item), Some(identity.clone()))]);
        assert_eq!(scales.scale(a), 1.7);
        scales.refresh(&[(a, None, None)]);
        scales.set(a, 0.6);
        scales.refresh(&[]);
        scales.refresh(&[(b, Some(item), Some(identity))]);
        assert_eq!(scales.scale(b), 0.6);
    }

    #[test]
    fn forgetting_one_item_keeps_other_saved_copies_and_the_current_size() {
        let [a, b, c, item_a, item_b] = [1, 2, 3, 101, 102].map(Uuid::from_u128);
        let identity = identity("HUD", 10);
        let mut scales = HudScales::default();
        scales.refresh(&[(a, Some(item_a), Some(identity.clone())), (b, Some(item_b), Some(identity.clone()))]);
        scales.set(a, 1.7);
        scales.set(b, 0.6);
        scales.forget(a);
        assert_eq!(scales.scale(a), 1.7);
        assert!(!scales.has_profile(a));
        scales.refresh(&[]);
        scales.refresh(&[
            (a, Some(item_a), Some(identity.clone())),
            (b, Some(item_b), Some(identity.clone())),
            (c, Some(Uuid::from_u128(103)), Some(identity)),
        ]);
        assert_eq!(scales.scale(a), 1.0);
        assert_eq!(scales.scale(b), 0.6);
        assert_eq!(scales.scale(c), 1.0);
    }

    #[test]
    fn forgetting_before_metadata_arrives_does_not_restore_or_resave_the_current_copy() {
        let [a, b, item] = [1, 2, 101].map(Uuid::from_u128);
        let identity = identity("HUD", 10);
        let mut scales = HudScales::default();
        scales.remember(None, Some(identity.clone()), 2.0, &[]);
        scales.refresh(&[(a, Some(item), None)]);
        scales.set(a, 1.7);
        scales.forget(a);
        scales.refresh(&[(a, Some(item), Some(identity.clone()))]);
        assert_eq!(scales.scale(a), 1.7);
        assert!(!scales.has_profile(a));
        scales.refresh(&[]);
        scales.refresh(&[(b, Some(item), Some(identity))]);
        assert_eq!(scales.scale(b), 1.0);
    }

    #[test]
    fn legacy_name_profiles_bind_to_items_before_the_generic_size_changes() {
        let [a, b, item_a, item_b] = [1, 2, 101, 102].map(Uuid::from_u128);
        let identity = identity("HUD", 10);
        let legacy = serde_json::json!([{ "identity": identity, "scale": 1.7 }]);
        let mut scales = HudScales::default();
        scales.profiles = serde_json::from_value(legacy).unwrap();
        scales.refresh(&[(a, Some(item_a), Some(identity.clone())), (b, Some(item_b), Some(identity.clone()))]);
        assert_eq!(scales.scale(a), 1.7);
        assert_eq!(scales.scale(b), 1.7);
        scales.set(b, 0.6);
        scales.refresh(&[]);
        scales.refresh(&[(a, Some(item_a), Some(identity))]);
        assert_eq!(scales.scale(a), 1.7);
    }

    fn identity(name: &str, creator: u128) -> Identity {
        Identity {
            name: name.into(),
            creator: Uuid::from_u128(creator),
        }
    }

    #[test]
    fn wheel_is_reversible_fine_and_bounded() {
        let enlarged = wheel_scale(1.0, 5.0, false);
        assert!(enlarged > 1.5);
        assert!((wheel_scale(enlarged, -5.0, false) - 1.0).abs() < 1e-5);
        assert!(wheel_scale(1.0, 1.0, true) < wheel_scale(1.0, 1.0, false));
        assert_eq!(wheel_scale(MAX, 1000.0, false), MAX);
        assert_eq!(wheel_scale(MIN, -1000.0, false), MIN);
        assert_eq!(wheel_scale(1.0, f32::NAN, false), 1.0);
    }

    #[test]
    fn replacement_copies_restore_name_and_creator_without_changing_other_worn_copies() {
        let [a, b, c, d] = [1, 2, 3, 4].map(Uuid::from_u128);
        let profile = identity("HUD", 10);
        let mut scales = HudScales::default();
        scales.refresh(&[(a, None, Some(profile.clone())), (b, None, Some(profile.clone()))]);
        scales.set(a, 1.7);
        assert_eq!(scales.scale(b), 1.0);
        let bytes = serde_json::to_vec(&scales.profiles).unwrap();
        let mut restored = HudScales::default();
        restored.profiles = serde_json::from_slice(&bytes).unwrap();
        restored.refresh(&[(c, None, Some(profile.clone())), (d, None, Some(identity("HUD", 11)))]);
        assert_eq!(restored.scale(c), 1.7);
        assert_eq!(restored.scale(d), 1.0);
        restored.forget(c);
        assert_eq!(restored.scale(c), 1.7);
        restored.refresh(&[]);
        restored.refresh(&[(a, None, Some(profile))]);
        assert_eq!(restored.scale(a), 1.0);
    }

    #[test]
    fn late_inventory_metadata_saves_the_current_choice_and_reset_persists() {
        let id = Uuid::from_u128(1);
        let profile = identity("HUD", 10);
        let mut scales = HudScales::default();
        scales.refresh(&[(id, None, None)]);
        scales.set(id, 2.0);
        scales.refresh(&[(id, None, Some(profile.clone()))]);
        assert!(scales.has_profile(id));
        assert_eq!(scales.profiles[0].scale, 2.0);
        scales.set(id, 1.0);
        scales.refresh(&[]);
        scales.refresh(&[(Uuid::from_u128(2), None, Some(profile))]);
        assert_eq!(scales.scale(Uuid::from_u128(2)), 1.0);
    }

    #[test]
    fn a_size_from_a_newer_build_costs_only_that_size() {
        let folder = std::env::temp_dir().join(format!("aurora-hud-scales-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join(FILE);
        let kept = Uuid::from_u128(7);
        let file = serde_json::json!([
            { "item": kept.to_string(), "scale": 1.5, "a_newer_field": [1, 2] },
            { "item": Uuid::from_u128(8).to_string(), "scale": "a newer kind of size" },
        ]);
        std::fs::write(&path, serde_json::to_vec(&file).unwrap()).unwrap();
        let mut scales = HudScales::load(path.clone());
        scales.refresh(&[(kept, Some(kept), None)]);
        assert_eq!(scales.scale(kept), 1.5);
        assert!(crate::tolerant::backup_path(&path).exists(), "the newer file is kept aside");
        drop(scales);
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn asynchronous_save_is_flushed_and_account_files_are_independent() {
        let folder = std::env::temp_dir().join(format!("aurora-hud-scales-{}", Uuid::new_v4()));
        let path = folder.join(FILE);
        let id = Uuid::from_u128(1);
        let profile = identity("HUD", 10);
        let mut scales = HudScales::load(path.clone());
        scales.refresh(&[(id, Some(id), None)]);
        scales.set(id, 2.0);
        scales.set(id, 1.8);
        drop(scales);
        let saved: Vec<Profile> = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert!(saved.iter().any(|p| p.item == Some(id) && p.scale == 1.8));
        let mut restored = HudScales::load(path.clone());
        let instance = Uuid::from_u128(2);
        restored.refresh(&[(instance, Some(id), None)]);
        assert_eq!(restored.scale(instance), 1.8);
        restored.refresh(&[(instance, Some(id), Some(profile.clone()))]);
        let copy = Uuid::from_u128(3);
        restored.refresh(&[
            (instance, Some(id), Some(profile.clone())),
            (copy, Some(copy), Some(profile.clone())),
        ]);
        assert_eq!(restored.scale(copy), 1.8);
        let other_path = folder.join("other-account.json");
        let mut other = HudScales::load(other_path);
        other.refresh(&[(id, Some(id), Some(profile))]);
        assert_eq!(other.scale(id), 1.0);
        drop(other);
        drop(restored);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(folder).unwrap();
    }
}
