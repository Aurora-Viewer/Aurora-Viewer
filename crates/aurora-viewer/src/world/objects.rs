//! Object store: every prim, mesh and avatar known to the viewer.

use aurora_net::RegionHandle;
use aurora_net::objects::{ObjectUpdate, ParticleUpdate, TerseUpdate, TextureAnim};
use aurora_prim::{ExtraParams, TextureEntry, VolumeParams};
use glam::{Quat, Vec3};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ObjKey {
    pub region: RegionHandle,
    pub local_id: u32,
}

/// GPU-side state owned by the scene; kept here so it moves with the object.
#[derive(Debug, Default)]
#[allow(dead_code)] // geometry keys kept for diagnostics
pub struct RenderState {
    /// Geometry cache key currently bound (and its LOD).
    pub geom: Option<crate::scene::GeomKey>,
    /// Geometry requested but not ready yet.
    pub pending_geom: Option<crate::scene::GeomKey>,
    /// One record slot per face.
    pub records: Vec<u32>,
    /// Texture handles held by this object (for refcounting).
    pub textures: Vec<Uuid>,
    pub needs_records: bool,
    pub lod: u8,
    pub last_lod_check: u32,
    /// World bounds (render space): center, radius.
    pub center: Vec3,
    pub radius: f32,
    pub visible: bool,
}

/// Clock of an object's texture animation (LLViewerTextureAnim mTimer /
/// mLastTime): its raw frame counter at time `t` is
/// `phase + (t - start) * rate`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TexAnimClock {
    pub start: Instant,
    pub phase: f32,
}

impl TexAnimClock {
    /// Clock after an update carrying a texture animation block
    /// (LLVOVolume::processUpdateMessage, LLViewerTextureAnim::animateTextures):
    /// a new animation starts now; an existing non-smooth one restarts (its
    /// timer is reset on every such update, even unchanged: a known SL
    /// behaviour, e.g. llSetText restarts frame animations); a smooth one
    /// carries on from its accumulated counter, which an animation that is
    /// off keeps at 0.
    pub fn after_update(old: Option<(&TextureAnim, &TexAnimClock)>, new: &TextureAnim, now: Instant) -> TexAnimClock {
        let accumulated = match old {
            Some((a, c)) if a.mode & TextureAnim::ON != 0 => {
                let rate = if a.rate.is_finite() { a.rate } else { 0.0 };
                c.phase + now.saturating_duration_since(c.start).as_secs_f32() * rate
            }
            _ => 0.0,
        };
        TexAnimClock {
            start: now,
            phase: if new.mode & TextureAnim::SMOOTH != 0 && accumulated.is_finite() {
                accumulated
            } else {
                0.0
            },
        }
    }
}

#[derive(Debug)]
pub struct Object {
    pub key: ObjKey,
    pub full_id: Uuid,
    pub parent_id: u32,
    pub pcode: u8,
    pub state: u8,
    pub click_action: u8,
    /// Prim physical material (LL_MCODE_*); 7 = light, drawn fullbright.
    pub prim_material: u8,
    pub position: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
    pub velocity: Vec3,
    pub acceleration: Vec3,
    pub angular_velocity: Vec3,
    pub update_time: Instant,
    pub volume: VolumeParams,
    pub te: Option<Arc<TextureEntry>>,
    pub extra: ExtraParams,
    pub name_values: String,
    pub text: String,
    pub text_color: [u8; 4],
    pub tex_anim: Option<TextureAnim>,
    pub tex_anim_clock: TexAnimClock,
    pub update_flags: u32,
    pub owner_id: Uuid,
    pub tree_species: Option<u8>,
    pub render: RenderState,
    /// Shape / texture changed since last scene sync.
    pub shape_dirty: bool,
    pub material_dirty: bool,
    /// Particle system change not yet seen by the scene (Set or Clear).
    pub particle_update: Option<ParticleUpdate>,
    /// Attached sound of the last update, and whether the sound manager has
    /// not seen it yet (every update re-applies it, as LL does).
    pub sound: aurora_net::objects::AttachedSound,
    pub sound_update: bool,
    /// MediaURL of the last update: "x-mv:<version>/<agent>" for media on
    /// a prim (the media data is fetched when the version grows).
    pub media_url: String,
}

impl Object {
    pub fn from_update(region: RegionHandle, u: ObjectUpdate) -> Object {
        Object {
            key: ObjKey {
                region,
                local_id: u.local_id,
            },
            full_id: u.full_id,
            parent_id: u.parent_id,
            pcode: u.pcode,
            state: u.state,
            click_action: u.click_action,
            prim_material: u.material,
            position: u.position,
            rotation: u.rotation,
            scale: u.scale,
            velocity: u.velocity,
            acceleration: u.acceleration,
            angular_velocity: u.angular_velocity,
            update_time: Instant::now(),
            volume: u.volume,
            te: u.texture_entry,
            extra: u.extra,
            name_values: u.name_values,
            text: u.text,
            text_color: u.text_color,
            tex_anim: u.texture_anim,
            tex_anim_clock: TexAnimClock {
                start: Instant::now(),
                phase: 0.0,
            },
            update_flags: u.update_flags,
            owner_id: u.owner_id,
            tree_species: u.tree_species,
            render: RenderState {
                needs_records: true,
                ..Default::default()
            },
            shape_dirty: true,
            material_dirty: true,
            particle_update: match u.particles {
                ParticleUpdate::Keep => None,
                p => Some(p),
            },
            sound: u.sound,
            sound_update: true,
            media_url: u.media_url,
        }
    }

    /// Apply a full update; flags what changed.
    pub fn apply_full(&mut self, u: ObjectUpdate) {
        match &u.particles {
            ParticleUpdate::Keep => {}
            p => self.particle_update = Some(p.clone()),
        }
        self.sound = u.sound;
        self.sound_update = true;
        self.media_url = u.media_url.clone();
        if u.volume != self.volume || u.pcode != self.pcode {
            self.shape_dirty = true;
        }
        let te_changed = match (&self.te, &u.texture_entry) {
            (Some(a), Some(b)) => a != b,
            (None, None) => false,
            _ => true,
        };
        if te_changed || u.extra != self.extra || u.material != self.prim_material {
            self.material_dirty = true;
        }
        self.prim_material = u.material;
        if u.extra.sculpt != self.extra.sculpt {
            self.shape_dirty = true;
        }
        self.full_id = u.full_id;
        self.parent_id = u.parent_id;
        self.pcode = u.pcode;
        self.state = u.state;
        self.click_action = u.click_action;
        self.position = u.position;
        self.rotation = u.rotation;
        self.scale = u.scale;
        self.velocity = u.velocity;
        self.acceleration = u.acceleration;
        self.angular_velocity = u.angular_velocity;
        self.update_time = Instant::now();
        self.volume = u.volume;
        if u.texture_entry.is_some() {
            self.te = u.texture_entry;
        }
        self.extra = u.extra;
        self.name_values = u.name_values;
        self.text = u.text;
        self.text_color = u.text_color;
        if self.set_tex_anim(u.texture_anim, Instant::now()) {
            self.material_dirty = true;
        }
        self.update_flags = u.update_flags;
        self.owner_id = u.owner_id;
        self.tree_species = u.tree_species;
        self.render.needs_records = true;
    }

    /// Texture animation of a full update (None: the block is absent, the
    /// animation is removed). True when the face records must be rebuilt:
    /// the animation or its clock changed visibly.
    pub fn set_tex_anim(&mut self, new: Option<TextureAnim>, now: Instant) -> bool {
        let Some(new) = new else {
            return self.tex_anim.take().is_some();
        };
        let old = self.tex_anim;
        self.tex_anim_clock = TexAnimClock::after_update(old.as_ref().map(|a| (a, &self.tex_anim_clock)), &new, now);
        self.tex_anim = Some(new);
        let on = new.mode & TextureAnim::ON != 0;
        // an unchanged smooth animation keeps its time origin (its phase moved
        // with the start); a non-smooth one restarted
        old != Some(new) || (on && new.mode & TextureAnim::SMOOTH == 0)
    }

    pub fn apply_terse(&mut self, t: &TerseUpdate) {
        self.state = t.state;
        self.position = t.position;
        self.rotation = t.rotation;
        self.velocity = t.velocity;
        self.acceleration = t.acceleration;
        self.angular_velocity = t.angular_velocity;
        self.update_time = Instant::now();
        if let Some(te) = &t.texture_entry
            && self.te.as_ref() != Some(te)
        {
            self.te = Some(te.clone());
            self.material_dirty = true;
        }
        self.render.needs_records = true;
    }

    pub fn is_avatar(&self) -> bool {
        self.pcode == aurora_prim::params::LL_PCODE_LEGACY_AVATAR
    }

    pub fn is_tree(&self) -> bool {
        matches!(self.pcode, 0xFF | 0x6F | 0x5F)
    }

    pub fn attachment_point(&self) -> u8 {
        ((self.state & 0xF0) >> 4) | ((self.state & 0x0F) << 4)
    }

    pub fn display_name(&self) -> Option<String> {
        let mut first = None;
        let mut last = None;
        for line in self.name_values.lines() {
            let mut it = line.splitn(5, ' ');
            let k = it.next().unwrap_or("");
            let v = it.nth(3).map(|s| s.trim().to_owned());
            match k {
                "FirstName" => first = v,
                "LastName" => last = v,
                _ => {}
            }
        }
        let f = first?;
        Some(match last.as_deref() {
            Some("Resident") | None | Some("") => f,
            Some(l) => format!("{f} {l}"),
        })
    }

    /// Inventory item an attachment was worn from ("AttachItemID" name-value).
    pub fn attachment_item_id(&self) -> Option<Uuid> {
        self.name_values.lines().find_map(|line| {
            let mut it = line.splitn(5, ' ');
            (it.next() == Some("AttachItemID"))
                .then(|| it.nth(3))
                .flatten()
                .and_then(|v| Uuid::parse_str(v.trim()).ok())
        })
    }

    pub fn group_title(&self) -> Option<String> {
        for line in self.name_values.lines() {
            let mut it = line.splitn(5, ' ');
            if it.next() == Some("Title") {
                return it.nth(3).map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());
            }
        }
        None
    }

    /// Dead-reckoned local position/rotation at `now`.
    pub fn predicted(&self, now: Instant) -> (Vec3, Quat) {
        let dt = now.duration_since(self.update_time).as_secs_f32().min(0.6);
        if !self.has_motion() {
            return (self.position, self.rotation);
        }
        let p = self.position + self.velocity * dt + self.acceleration * (0.5 * dt * dt);
        let r = if self.angular_velocity.length_squared() > 1e-8 {
            let angle = self.angular_velocity.length() * dt;
            (Quat::from_axis_angle(self.angular_velocity.normalize(), angle) * self.rotation).normalize()
        } else {
            self.rotation
        };
        (p, r)
    }

    /// Whether `predicted` moves it (any velocity, acceleration or spin).
    pub fn has_motion(&self) -> bool {
        self.velocity != Vec3::ZERO || self.acceleration != Vec3::ZERO || self.angular_velocity != Vec3::ZERO
    }
}

/// Object indices noted once each until taken: a change feed.
#[derive(Default)]
struct ChangeFeed {
    list: Vec<usize>,
    mark: Vec<bool>,
}

impl ChangeFeed {
    fn note(&mut self, idx: usize) {
        if self.mark.len() <= idx {
            self.mark.resize(idx + 1, false);
        }
        if !self.mark[idx] {
            self.mark[idx] = true;
            self.list.push(idx);
        }
    }

    fn take(&mut self, out: &mut Vec<usize>) {
        for &i in &self.list {
            self.mark[i] = false;
        }
        out.append(&mut self.list);
    }
}

/// Slab of objects with lookup maps.
///
/// Changes go through `upsert` and `get_mut`, which note the object for the
/// scene sync (`take_touched`): the sync visits the changed objects instead
/// of every object each frame. Code changing sync state (flags, motion,
/// links) through `slots` directly is only seen by the periodic LOD pass.
///
/// The media manager runs after the sync has taken those changes, so it has
/// its own feed (`take_media_changes`): the same objects, plus the slots of
/// removed objects.
#[derive(Default)]
pub struct ObjectStore {
    pub slots: Vec<Option<Object>>,
    free: Vec<usize>,
    by_key: HashMap<ObjKey, usize>,
    by_uuid: HashMap<Uuid, usize>,
    /// parent key -> child indices
    children: HashMap<ObjKey, Vec<usize>>,
    count: usize,
    /// Removed objects whose GPU resources must be released by the scene.
    pub graveyard: Vec<Object>,
    /// Their slot indices, in removal order (a slot may be reused since).
    pub removed_slots: Vec<usize>,
    /// Objects changed since the scene last took them, each once.
    touched: ChangeFeed,
    /// Objects changed, added or removed since the media manager last took
    /// them (a slot may hold another object since, or none).
    media_changes: ChangeFeed,
}

impl ObjectStore {
    pub fn len(&self) -> usize {
        self.count
    }

    pub fn get(&self, idx: usize) -> Option<&Object> {
        self.slots.get(idx).and_then(|o| o.as_ref())
    }

    /// Mutable access, noted for the scene sync.
    pub fn get_mut(&mut self, idx: usize) -> Option<&mut Object> {
        self.touch(idx);
        self.get_mut_untracked(idx)
    }

    /// Mutable access the scene sync does not hear of: for the sync itself,
    /// which clears the flags it has handled.
    pub fn get_mut_untracked(&mut self, idx: usize) -> Option<&mut Object> {
        self.slots.get_mut(idx).and_then(|o| o.as_mut())
    }

    /// Note an object as changed for the scene sync.
    pub fn touch(&mut self, idx: usize) {
        if self.slots.get(idx).is_none_or(|o| o.is_none()) {
            return;
        }
        self.touched.note(idx);
        self.media_changes.note(idx);
    }

    /// The objects changed since the scene sync last took them.
    pub fn touched(&self) -> &[usize] {
        &self.touched.list
    }

    /// The objects changed since the last call (appended to `out`).
    pub fn take_touched(&mut self, out: &mut Vec<usize>) {
        self.touched.take(out);
    }

    /// The slots changed, filled or emptied since the last call (appended to
    /// `out`), for the media manager.
    pub fn take_media_changes(&mut self, out: &mut Vec<usize>) {
        self.media_changes.take(out);
    }

    pub fn index_of(&self, key: &ObjKey) -> Option<usize> {
        self.by_key.get(key).copied()
    }

    pub fn index_of_uuid(&self, id: &Uuid) -> Option<usize> {
        self.by_uuid.get(id).copied()
    }

    pub fn children_of(&self, key: &ObjKey) -> &[usize] {
        self.children.get(key).map(|v| v.as_slice()).unwrap_or(&[])
    }

    fn link(&mut self, idx: usize) {
        let Some(o) = self.get(idx) else {
            return;
        };
        if o.parent_id != 0 {
            let pk = ObjKey {
                region: o.key.region,
                local_id: o.parent_id,
            };
            let v = self.children.entry(pk).or_default();
            if !v.contains(&idx) {
                v.push(idx);
            }
        }
    }

    fn unlink(&mut self, idx: usize) {
        let Some(o) = self.get(idx) else {
            return;
        };
        if o.parent_id != 0 {
            let pk = ObjKey {
                region: o.key.region,
                local_id: o.parent_id,
            };
            if let Some(v) = self.children.get_mut(&pk) {
                v.retain(|&c| c != idx);
                if v.is_empty() {
                    self.children.remove(&pk);
                }
            }
        }
    }

    /// Insert or update from a full update. Returns the slot index.
    pub fn upsert(&mut self, region: RegionHandle, u: ObjectUpdate) -> usize {
        let key = ObjKey {
            region,
            local_id: u.local_id,
        };
        if let Some(&idx) = self.by_key.get(&key) {
            let parent_changed = self.get(idx).is_some_and(|o| o.parent_id != u.parent_id);
            let old_uuid = self.get(idx).map(|o| o.full_id);
            if parent_changed {
                self.unlink(idx);
            }
            if let Some(o) = self.get_mut(idx) {
                o.apply_full(u);
            }
            if parent_changed {
                self.link(idx);
            }
            let new_uuid = self.get(idx).map(|o| o.full_id);
            if let (Some(old), Some(new)) = (old_uuid, new_uuid)
                && old != new
            {
                self.by_uuid.remove(&old);
                self.by_uuid.insert(new, idx);
            }
            self.touch(idx);
            return idx;
        }
        // An object with the same UUID may exist under another region (crossing).
        if let Some(&old) = self.by_uuid.get(&u.full_id) {
            if self.get(old).is_some_and(|o| o.key.region != region) {
                // caller handles GPU cleanup through `take_removed`
                self.remove_idx(old);
            } else if let Some(old_key) = self.get(old).map(|o| o.key) {
                // same region, new local id: the simulator renumbered it; keep
                // the object (its state, animations, attachments) under the new
                // id, as LLViewerObjectList::processObjectUpdate does
                log::info!("object {} renumbered: local id {} -> {}", u.full_id, old_key.local_id, u.local_id);
                self.by_key.remove(&old_key);
                self.by_key.insert(key, old);
                if let Some(kids) = self.children.remove(&old_key) {
                    self.children.insert(key, kids);
                }
                let parent_changed = self.get(old).is_some_and(|o| o.parent_id != u.parent_id);
                if parent_changed {
                    self.unlink(old);
                }
                if let Some(o) = self.get_mut(old) {
                    o.key = key;
                    o.apply_full(u);
                }
                if parent_changed {
                    self.link(old);
                }
                self.touch(old);
                return old;
            }
        }
        let obj = Object::from_update(region, u);
        let idx = match self.free.pop() {
            Some(i) => i,
            None => {
                self.slots.push(None);
                self.slots.len() - 1
            }
        };
        self.by_uuid.insert(obj.full_id, idx);
        self.slots[idx] = Some(obj);
        self.by_key.insert(key, idx);
        self.count += 1;
        self.link(idx);
        self.touch(idx);
        idx
    }

    pub fn remove_idx(&mut self, idx: usize) {
        self.unlink(idx);
        let Some(obj) = self.slots.get_mut(idx).and_then(|s| s.take()) else {
            return;
        };
        self.by_key.remove(&obj.key);
        if self.by_uuid.get(&obj.full_id) == Some(&idx) {
            self.by_uuid.remove(&obj.full_id);
        }
        self.free.push(idx);
        self.removed_slots.push(idx);
        self.media_changes.note(idx);
        self.count -= 1;
        // Children of a removed object are removed too (linkset / attachments).
        let kids = self.children.remove(&obj.key).unwrap_or_default();
        self.graveyard.push(obj);
        for k in kids {
            self.remove_idx(k);
        }
    }

    pub fn remove(&mut self, key: &ObjKey) {
        if let Some(idx) = self.by_key.get(key).copied() {
            self.remove_idx(idx);
        }
    }

    /// Remove every object of a region.
    pub fn remove_region(&mut self, region: RegionHandle) {
        let idxs: Vec<usize> = self.by_key.iter().filter(|(k, _)| k.region == region).map(|(_, &i)| i).collect();
        for i in idxs {
            self.remove_idx(i);
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (usize, &Object)> {
        self.slots.iter().enumerate().filter_map(|(i, o)| o.as_ref().map(|o| (i, o)))
    }

    /// Parent index of an object, if its parent is known.
    pub fn parent_of(&self, o: &Object) -> Option<usize> {
        if o.parent_id == 0 {
            return None;
        }
        self.index_of(&ObjKey {
            region: o.key.region,
            local_id: o.parent_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn anim(mode: u8, rate: f32) -> TextureAnim {
        TextureAnim {
            mode,
            face: -1,
            size_x: 1,
            size_y: 1,
            start: 0.0,
            length: 1.0,
            rate,
        }
    }

    const SMOOTH_ON: u8 = TextureAnim::ON | TextureAnim::SMOOTH | TextureAnim::LOOP;

    #[test]
    fn tex_anim_clock_follows_firestorm() {
        let t0 = Instant::now();
        let t1 = t0 + Duration::from_secs(4);
        let clock = TexAnimClock { start: t0, phase: 0.5 };
        // a smooth animation carries on: 0.5 + 4 s * 0.25
        let c = TexAnimClock::after_update(Some((&anim(SMOOTH_ON, 0.25), &clock)), &anim(SMOOTH_ON, 1.0), t1);
        assert_eq!(c.start, t1);
        assert!((c.phase - 1.5).abs() < 1e-5);
        // a non-smooth one restarts
        let c = TexAnimClock::after_update(Some((&anim(SMOOTH_ON, 0.25), &clock)), &anim(TextureAnim::ON, 1.0), t1);
        assert_eq!(c.phase, 0.0);
        // smooth after an animation that was off: from 0
        let c = TexAnimClock::after_update(Some((&anim(TextureAnim::SMOOTH, 0.25), &clock)), &anim(SMOOTH_ON, 1.0), t1);
        assert_eq!(c.phase, 0.0);
        // smooth after a non-smooth one: its elapsed counter (mLastTime)
        let frames = TexAnimClock { start: t0, phase: 0.0 };
        let c = TexAnimClock::after_update(Some((&anim(TextureAnim::ON, 2.0), &frames)), &anim(SMOOTH_ON, 1.0), t1);
        assert!((c.phase - 8.0).abs() < 1e-5);
        // new animation
        let c = TexAnimClock::after_update(None, &anim(SMOOTH_ON, 1.0), t1);
        assert_eq!((c.start, c.phase), (t1, 0.0));
    }
}
