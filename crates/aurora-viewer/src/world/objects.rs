//! Object store: every prim, mesh and avatar known to the viewer.

use aurora_net::RegionHandle;
use aurora_net::objects::{ObjectUpdate, ParticleUpdate, TerseUpdate, TextureAnim};
use aurora_prim::{ExtraParams, TextureEntry, VolumeParams};
use glam::{Quat, Vec3};
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::time::Instant;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ObjKey {
    pub region: RegionHandle,
    pub local_id: u32,
}

/// Local id of our avatar between the removal of the region it was in and
/// the first update from the one it arrived in (`ObjectStore::carry`). No
/// simulator is expected to give it to an object.
pub const CARRIED_LOCAL_ID: u32 = u32::MAX;

/// Client-only appearance; never included in simulator texture/material edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LocalAppearance {
    Texture(Uuid),
    Material(Uuid),
}
impl LocalAppearance {
    pub fn entry(self) -> Uuid {
        match self {
            Self::Texture(id) | Self::Material(id) => id,
        }
    }
}

/// GPU-side state owned by the scene; kept here so it moves with the object.
#[derive(Debug, Default)]
#[allow(dead_code)] // geometry keys kept for diagnostics
pub struct RenderState {
    /// Client-only local mesh preview; server volume/scale remain authoritative.
    pub local_volume: Option<Box<VolumeParams>>,
    pub local_scale: Option<Vec3>,
    /// None applies to all faces, Some(face) overrides that face only.
    pub local_appearance: HashMap<Option<u8>, LocalAppearance>,
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
    /// Region of the parent's key: this object's own, except after its
    /// parent moved to another region and until its own update comes from
    /// there (LLViewerObject::setRegion takes the children along).
    pub parent_region: RegionHandle,
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
    /// Full updates carry parentage; terse motion packets do not confirm it.
    pub full_update_serial: u64,
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
    /// `PSBlock` bytes of its particle system, as last set: the simulator
    /// only sends them once, and an object back from a parked region
    /// (`ObjectStore::restore`) hands them to the scene again.
    pub particles: Option<Vec<u8>>,
    /// Attached sound of the last update, and whether the sound manager has
    /// not seen it yet (every update re-applies it, as LL does).
    pub sound: aurora_net::objects::AttachedSound,
    pub sound_update: bool,
    /// MediaURL of the last update: "x-mv:<version>/<agent>" for media on
    /// a prim (the media data is fetched when the version grows).
    pub media_url: String,
}

impl Object {
    pub fn local_appearance(&self, face: usize) -> Option<LocalAppearance> {
        self.render
            .local_appearance
            .get(&Some(face as u8))
            .or_else(|| self.render.local_appearance.get(&None))
            .copied()
    }
    pub fn visual_texture_face(&self, face: usize) -> aurora_prim::te::TextureFace {
        let mut value = self.te.as_ref().map(|te| *te.face(face)).unwrap_or_default();
        if let Some(LocalAppearance::Texture(id)) = self.local_appearance(face) {
            value.texture = id;
        }
        value
    }
    pub fn visual_material(&self, face: usize) -> Option<Uuid> {
        match self.local_appearance(face) {
            Some(LocalAppearance::Texture(_)) => None,
            Some(LocalAppearance::Material(id)) => Some(id),
            None => self
                .extra
                .render_materials
                .iter()
                .find(|(f, _)| *f as usize == face)
                .map(|(_, id)| *id)
                .filter(|id| !id.is_nil()),
        }
    }
    #[inline]
    pub fn visual_volume(&self) -> VolumeParams {
        self.render.local_volume.as_deref().copied().unwrap_or(self.volume)
    }
    #[inline]
    pub fn visual_scale(&self) -> Vec3 {
        self.render.local_scale.unwrap_or(self.scale)
    }
    /// Ordinary flexible volumes use a simulated path; sculpt/mesh assets
    /// retain their own geometry even if they carry stale flexible parameters.
    pub fn is_flexible(&self) -> bool {
        self.pcode == aurora_prim::params::LL_PCODE_VOLUME
            && self.volume.path.curve_type == aurora_prim::params::LL_PCODE_PATH_FLEXIBLE
            && self.extra.flexible.is_some()
            && !self.volume.is_sculpt()
            && !self.visual_volume().is_mesh()
    }

    pub fn from_update(region: RegionHandle, u: ObjectUpdate) -> Object {
        Object {
            key: ObjKey {
                region,
                local_id: u.local_id,
            },
            full_id: u.full_id,
            parent_id: u.parent_id,
            parent_region: region,
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
            full_update_serial: 1,
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
            particles: match &u.particles {
                ParticleUpdate::Set(bytes) => Some(bytes.clone()),
                _ => None,
            },
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
    /// Key of its parent (None for a root).
    pub fn parent_key(&self) -> Option<ObjKey> {
        (self.parent_id != 0).then_some(ObjKey {
            region: self.parent_region,
            local_id: self.parent_id,
        })
    }

    pub fn apply_full(&mut self, u: ObjectUpdate) {
        if !u.full_id.is_nil() && self.full_id != u.full_id {
            self.render.local_volume = None;
            self.render.local_scale = None;
            self.render.local_appearance.clear();
        }
        self.full_update_serial = self.full_update_serial.wrapping_add(1);
        match &u.particles {
            ParticleUpdate::Keep => {}
            p => {
                self.particles = match p {
                    ParticleUpdate::Set(bytes) => Some(bytes.clone()),
                    _ => None,
                };
                self.particle_update = Some(p.clone());
            }
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
///
/// A new object that is not an avatar is also marked as an arrival
/// (`is_arrival`) until the scene has synced it once (`settle`): the sync
/// tells the objects it never saw from the changed ones without reading
/// each of them, when they arrive by thousands.
///
/// The interface labels few of the objects (name tags of the avatars, hover
/// text of the prims, dots of the maps): `avatars` and `texted` list them,
/// kept from a third feed, so that no window walks every object each frame.
///
/// A full update may carry a particle system or an attached sound for their
/// managers (`particle_update`, `sound_update`): two more feeds list those
/// objects (`take_particle_changes`, `take_sound_changes`).
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
    /// Keys changed without the object being removed (`take_rekeyed`).
    rekeyed: Vec<(ObjKey, ObjKey)>,
    /// Objects changed since the scene last took them, each once.
    touched: ChangeFeed,
    /// By slot: a new object the scene has not synced yet.
    arrival: Vec<bool>,
    /// Objects changed, added or removed since the media manager last took
    /// them (a slot may hold another object since, or none).
    media_changes: ChangeFeed,
    /// The same, since `refresh_labelled`.
    label_changes: ChangeFeed,
    /// Objects whose `particle_update` / `sound_update` waits for its
    /// manager (a slot may hold another object since, or none).
    particle_changes: ChangeFeed,
    sound_changes: ChangeFeed,
    /// Slots of the avatars, and of the other objects with a hover text
    /// (llSetText), as of the last `refresh_labelled`.
    avatar_slots: BTreeSet<usize>,
    texted_slots: BTreeSet<usize>,
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
        self.label_changes.note(idx);
        self.note_carried(idx);
    }

    /// The objects changed since the scene sync last took them.
    pub fn touched(&self) -> &[usize] {
        &self.touched.list
    }

    /// The objects changed since the last call (appended to `out`).
    pub fn take_touched(&mut self, out: &mut Vec<usize>) {
        self.touched.take(out);
    }

    /// Whether the slot holds a new object, not an avatar, that the scene
    /// has not synced yet.
    pub fn is_arrival(&self, idx: usize) -> bool {
        self.arrival.get(idx).copied().unwrap_or(false)
    }

    /// The scene has synced the object once: no longer an arrival.
    pub fn settle(&mut self, idx: usize) {
        if let Some(a) = self.arrival.get_mut(idx) {
            *a = false;
        }
    }

    /// The slots changed, filled or emptied since the last call (appended to
    /// `out`), for the media manager.
    pub fn take_media_changes(&mut self, out: &mut Vec<usize>) {
        self.media_changes.take(out);
    }

    /// The slots whose object got a particle system change since the last
    /// call (appended to `out`): the only ones with a `particle_update`.
    pub fn take_particle_changes(&mut self, out: &mut Vec<usize>) {
        self.particle_changes.take(out);
    }

    /// The slots whose object got a full update since the last call
    /// (appended to `out`): the only ones with `sound_update` set.
    pub fn take_sound_changes(&mut self, out: &mut Vec<usize>) {
        self.sound_changes.take(out);
    }

    /// Note what a full update left on an object for the particle and
    /// sound managers.
    fn note_carried(&mut self, idx: usize) {
        let Some(o) = self.slots.get(idx).and_then(|o| o.as_ref()) else {
            return;
        };
        if o.particle_update.is_some() {
            self.particle_changes.note(idx);
        }
        if o.sound_update {
            self.sound_changes.note(idx);
        }
    }

    /// Bring the lists behind [`Self::avatars`] and [`Self::texted`] up to
    /// date with the objects changed, added or removed since the last call.
    /// Once a frame, before the interface.
    pub fn refresh_labelled(&mut self) {
        for i in 0..self.label_changes.list.len() {
            let idx = self.label_changes.list[i];
            self.label_changes.mark[idx] = false;
            let (avatar, texted) = match self.slots.get(idx).and_then(|o| o.as_ref()) {
                Some(o) => (o.is_avatar(), !o.is_avatar() && !o.text.is_empty()),
                None => (false, false),
            };
            for (set, member) in [(&mut self.avatar_slots, avatar), (&mut self.texted_slots, texted)] {
                if member {
                    set.insert(idx);
                } else {
                    set.remove(&idx);
                }
            }
        }
        self.label_changes.list.clear();
    }

    /// Listed slots that still satisfy `keep`, in slot order, then the
    /// slots changed since the last `refresh_labelled` that do: right at
    /// any time, in slot order once refreshed.
    fn labelled<'a>(&'a self, listed: &'a BTreeSet<usize>, keep: fn(&Object) -> bool) -> impl Iterator<Item = (usize, &'a Object)> {
        let changed = self.label_changes.list.iter().filter(move |idx| !listed.contains(idx));
        listed
            .iter()
            .chain(changed)
            .filter_map(move |&idx| self.get(idx).filter(|o| keep(o)).map(|o| (idx, o)))
    }

    /// The avatars, without a walk over every object.
    pub fn avatars(&self) -> impl Iterator<Item = (usize, &Object)> {
        self.labelled(&self.avatar_slots, Object::is_avatar)
    }

    /// The objects with a hover text (llSetText) that are not avatars.
    pub fn texted(&self) -> impl Iterator<Item = (usize, &Object)> {
        self.labelled(&self.texted_slots, |o| !o.is_avatar() && !o.text.is_empty())
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

    /// Object updates carry parent IDs, not link numbers. Once every parent
    /// change is confirmed, retain the ordered ObjectLink instruction issued
    /// by this viewer instead of using UDP arrival order as a link number.
    /// Never apply it to missing, extra, duplicated or differently parented prims.
    pub fn remember_link_order(&mut self, root: Uuid, children: &[Uuid]) -> bool {
        let Some(root_idx) = self.index_of_uuid(&root) else { return false };
        let Some(root_obj) = self.get(root_idx).filter(|o| o.parent_id == 0) else {
            return false;
        };
        let key = root_obj.key;
        let Some(indices) = children.iter().map(|id| self.index_of_uuid(id)).collect::<Option<Vec<_>>>() else {
            return false;
        };
        let existing = self.children_of(&key);
        if existing.len() != indices.len()
            || indices.iter().collect::<std::collections::HashSet<_>>().len() != indices.len()
            || indices
                .iter()
                .any(|idx| !existing.contains(idx) || self.get(*idx).is_none_or(|o| o.parent_key() != Some(key)))
        {
            return false;
        }
        self.children.insert(key, indices);
        self.touched.note(root_idx);
        true
    }

    fn link(&mut self, idx: usize) {
        let Some(pk) = self.get(idx).and_then(Object::parent_key) else {
            return;
        };
        let v = self.children.entry(pk).or_default();
        if !v.contains(&idx) {
            v.push(idx);
        }
    }

    fn unlink(&mut self, idx: usize) {
        let Some(pk) = self.get(idx).and_then(Object::parent_key) else {
            return;
        };
        if let Some(v) = self.children.get_mut(&pk) {
            v.retain(|&c| c != idx);
            if v.is_empty() {
                self.children.remove(&pk);
            }
        }
    }

    /// Give an object another key (new local id, or another region), keeping
    /// its slot and what hangs from it: LLViewerObjectList::processObjectUpdate
    /// finds an object by its UUID and only updates its local id and region,
    /// and LLViewerObject::setRegion takes its children along. The children
    /// keep their own key until their update comes from the new region.
    fn rekey(&mut self, idx: usize, key: ObjKey) {
        let Some(old) = self.get(idx).map(|o| o.key) else {
            return;
        };
        if old == key {
            return;
        }
        self.by_key.remove(&old);
        self.by_key.insert(key, idx);
        if let Some(o) = self.get_mut(idx) {
            o.key = key;
        }
        if let Some(mut kids) = self.children.remove(&old) {
            for &k in &kids {
                if let Some(c) = self.get_mut(k) {
                    c.parent_region = key.region;
                    c.parent_id = key.local_id;
                }
                self.touch(k);
            }
            // (those that came from the new region before their parent)
            for k in self.children.remove(&key).unwrap_or_default() {
                if !kids.contains(&k) {
                    kids.push(k);
                }
            }
            self.children.insert(key, kids);
        }
        self.rekeyed.push((old, key));
        self.touch(idx);
    }

    /// Make a root of an object whose parent goes away without it.
    fn detach(&mut self, idx: usize) {
        self.unlink(idx);
        if let Some(o) = self.get_mut(idx) {
            o.parent_id = 0;
        }
        self.touch(idx);
    }

    /// Keys changed by `rekey` since last taken (old, new), for what is
    /// stored by key outside the store.
    pub fn take_rekeyed(&mut self) -> Vec<(ObjKey, ObjKey)> {
        std::mem::take(&mut self.rekeyed)
    }

    /// An object and everything linked under it.
    pub fn subtree(&self, idx: usize) -> Vec<usize> {
        let mut out = vec![idx];
        let mut next = 0;
        while let Some(&i) = out.get(next) {
            next += 1;
            if let Some(o) = self.get(i) {
                for &k in self.children_of(&o.key) {
                    if !out.contains(&k) {
                        out.push(k);
                    }
                }
            }
        }
        out
    }

    /// Our avatar leaves `from` for `to` before `to` has sent it:
    /// LLViewerObjectList::killObject never destroys gAgentAvatarp, it moves
    /// it to the agent's region, with what it wears. A seat stays behind.
    /// The avatar waits under `CARRIED_LOCAL_ID` for its update from `to`.
    pub fn carry(&mut self, idx: usize, from: RegionHandle, to: RegionHandle) {
        if from == to || self.get(idx).is_none_or(|o| o.key.region != from) {
            return;
        }
        self.detach(idx);
        self.rekey(
            idx,
            ObjKey {
                region: to,
                local_id: CARRIED_LOCAL_ID,
            },
        );
    }

    /// Insert or update from a full update. Returns the slot index.
    pub fn upsert(&mut self, region: RegionHandle, u: ObjectUpdate) -> usize {
        let key = ObjKey {
            region,
            local_id: u.local_id,
        };
        if let Some(&idx) = self.by_key.get(&key) {
            let parent_changed = self
                .get(idx)
                .is_some_and(|o| o.parent_id != u.parent_id || o.parent_region != region);
            let old_uuid = self.get(idx).map(|o| o.full_id);
            if parent_changed {
                self.unlink(idx);
            }
            if let Some(o) = self.get_mut(idx) {
                o.apply_full(u);
                o.parent_region = region;
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
        // The same UUID under another key: the simulator renumbered it, or
        // it came from another region (border crossing, teleport of what we
        // wear). Keep the object (its state, animations, attachments) under
        // the new key, as LLViewerObjectList::processObjectUpdate does.
        if let Some(&old) = self.by_uuid.get(&u.full_id)
            && let Some(old_key) = self.get(old).map(|o| o.key)
        {
            if old_key.region == region {
                log::info!("object {} renumbered: local id {} -> {}", u.full_id, old_key.local_id, u.local_id);
            }
            self.rekey(old, key);
            let parent_changed = self
                .get(old)
                .is_some_and(|o| o.parent_id != u.parent_id || o.parent_region != region);
            if parent_changed {
                self.unlink(old);
            }
            if let Some(o) = self.get_mut(old) {
                o.apply_full(u);
                o.parent_region = region;
            }
            if parent_changed {
                self.link(old);
            }
            self.touch(old);
            return old;
        }
        self.insert(Object::from_update(region, u))
    }

    /// Put a new object in a slot, under its key and its UUID.
    fn insert(&mut self, obj: Object) -> usize {
        let key = obj.key;
        let idx = match self.free.pop() {
            Some(i) => i,
            None => {
                self.slots.push(None);
                self.slots.len() - 1
            }
        };
        if self.arrival.len() <= idx {
            self.arrival.resize(idx + 1, false);
        }
        self.arrival[idx] = !obj.is_avatar();
        self.by_uuid.insert(obj.full_id, idx);
        self.slots[idx] = Some(obj);
        self.by_key.insert(key, idx);
        self.count += 1;
        self.link(idx);
        self.touch(idx);
        idx
    }

    /// Empty a slot: the object leaves the maps and the scene and the feeds
    /// hear of it. What hung from it is returned for the caller to deal with.
    fn take_idx(&mut self, idx: usize) -> Option<(Object, Vec<usize>)> {
        self.unlink(idx);
        let obj = self.slots.get_mut(idx).and_then(|s| s.take())?;
        self.by_key.remove(&obj.key);
        if self.by_uuid.get(&obj.full_id) == Some(&idx) {
            self.by_uuid.remove(&obj.full_id);
        }
        self.free.push(idx);
        self.settle(idx);
        self.removed_slots.push(idx);
        self.media_changes.note(idx);
        self.label_changes.note(idx);
        self.count -= 1;
        let kids = self.children.remove(&obj.key).unwrap_or_default();
        Some((obj, kids))
    }

    pub fn remove_idx(&mut self, idx: usize) {
        let Some((obj, kids)) = self.take_idx(idx) else {
            return;
        };
        // Children of a removed object are removed too (linkset / attachments).
        self.graveyard.push(obj);
        for k in kids {
            self.remove_idx(k);
        }
    }

    /// Take every object of a region out of the store, except `kept`, and
    /// hand them back for [`Self::restore`]: a parent before what hangs
    /// from it, children in their link order. For the scene and the feeds
    /// they are removed, as by `remove_region`.
    pub fn park_region(&mut self, region: RegionHandle, kept: &[usize]) -> Vec<Object> {
        let mut parked = vec![false; self.slots.len()];
        let mut all: Vec<usize> = self.by_key.iter().filter(|(k, _)| k.region == region).map(|(_, &i)| i).collect();
        all.sort_unstable();
        for &i in kept {
            if let Some(k) = parked.get_mut(i) {
                *k = true;
            }
        }
        all.retain(|&i| !parked[i]);
        parked.fill(false);
        for &i in &all {
            parked[i] = true;
        }
        // each tree from its top, in link order
        let mut order = Vec::with_capacity(all.len());
        let mut listed = vec![false; self.slots.len()];
        let mut stack = Vec::new();
        for &top in &all {
            let under_parked = self.get(top).and_then(|o| self.parent_of(o)).is_some_and(|p| parked[p]);
            if under_parked {
                continue;
            }
            stack.push(top);
            while let Some(i) = stack.pop() {
                if std::mem::replace(&mut listed[i], true) {
                    continue;
                }
                order.push(i);
                if let Some(o) = self.get(i) {
                    // (reversed: the first child is the next one popped)
                    stack.extend(self.children_of(&o.key).iter().rev().filter(|&&k| parked[k]));
                }
            }
        }
        // (a parent chain that loops has no top)
        order.extend(all.iter().filter(|&&i| !listed[i]));
        let mut out = Vec::with_capacity(order.len());
        for i in order {
            // a kept object hanging from a parked one stays, as a root
            for k in self.get(i).map(|o| self.children_of(&o.key).to_vec()).unwrap_or_default() {
                if !parked[k] && kept.contains(&k) {
                    self.detach(k);
                }
            }
            let Some((obj, kids)) = self.take_idx(i) else {
                continue;
            };
            // what hung from it without being parked (an object of another
            // region) goes for good, as when its parent is removed
            for k in kids {
                if !parked[k] {
                    self.remove_idx(k);
                }
            }
            out.push(obj);
        }
        out
    }

    /// Put back an object taken by [`Self::park_region`], as a new arrival
    /// for the scene and every feed. None when an object came under its key
    /// or its UUID in the meantime: that one is newer.
    pub fn restore(&mut self, mut obj: Object) -> Option<usize> {
        if self.by_key.contains_key(&obj.key) || self.by_uuid.contains_key(&obj.full_id) {
            return None;
        }
        // nothing of it is on the GPU any more; local previews are kept
        obj.render = RenderState {
            local_volume: obj.render.local_volume.take(),
            local_scale: obj.render.local_scale,
            local_appearance: std::mem::take(&mut obj.render.local_appearance),
            needs_records: true,
            ..Default::default()
        };
        obj.shape_dirty = true;
        obj.material_dirty = true;
        obj.particle_update = obj.particles.clone().map(ParticleUpdate::Set);
        obj.sound_update = true;
        Some(self.insert(obj))
    }

    pub fn remove(&mut self, key: &ObjKey) {
        if let Some(idx) = self.by_key.get(key).copied() {
            self.remove_idx(idx);
        }
    }

    /// Remove every object of a region, except `kept` (our avatar and what
    /// it wears, still to come from the region it arrived in).
    pub fn remove_region(&mut self, region: RegionHandle, kept: &[usize]) {
        let idxs: Vec<usize> = self
            .by_key
            .iter()
            .filter(|(k, i)| k.region == region && !kept.contains(i))
            .map(|(_, &i)| i)
            .collect();
        for i in idxs {
            // (a kept object hanging from a removed one stays, as a root)
            for k in self.get(i).map(|o| self.children_of(&o.key).to_vec()).unwrap_or_default() {
                if kept.contains(&k) {
                    self.detach(k);
                }
            }
            self.remove_idx(i);
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (usize, &Object)> {
        self.slots.iter().enumerate().filter_map(|(i, o)| o.as_ref().map(|o| (i, o)))
    }

    /// Parent index of an object, if its parent is known.
    pub fn parent_of(&self, o: &Object) -> Option<usize> {
        self.index_of(&o.parent_key()?)
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

    fn update(local_id: u32, pcode: u8, text: &str) -> ObjectUpdate {
        ObjectUpdate {
            local_id,
            full_id: Uuid::from_u128(local_id as u128),
            parent_id: 0,
            pcode,
            state: 0,
            crc: 0,
            material: 3,
            click_action: 0,
            scale: Vec3::ONE,
            position: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            velocity: Vec3::ZERO,
            acceleration: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
            update_flags: 0,
            owner_id: Uuid::nil(),
            volume: VolumeParams::default(),
            texture_entry: None,
            extra: ExtraParams::default(),
            name_values: String::new(),
            text: text.to_owned(),
            text_color: [255; 4],
            media_url: String::new(),
            tree_species: None,
            texture_anim: None,
            foot_plane: None,
            particles: Default::default(),
            sound: Default::default(),
        }
    }

    #[test]
    fn avatars_and_texted_objects_are_listed_without_a_walk() {
        use aurora_prim::params::{LL_PCODE_LEGACY_AVATAR as AVATAR, LL_PCODE_VOLUME as PRIM};
        let ids = |it: &mut dyn Iterator<Item = (usize, &Object)>| it.map(|(_, o)| o.key.local_id).collect::<Vec<_>>();
        let mut s = ObjectStore::default();
        s.upsert(1, update(10, PRIM, ""));
        s.upsert(1, update(11, AVATAR, ""));
        s.upsert(1, update(12, PRIM, "Vendor"));
        // an avatar's own text is not a hover text
        s.upsert(1, update(13, AVATAR, "ignored"));
        // right before the first refresh, and the same after it
        for refreshed in [false, true] {
            if refreshed {
                s.refresh_labelled();
                assert!(s.label_changes.list.is_empty());
            }
            assert_eq!(ids(&mut s.avatars()), [11, 13], "refreshed: {refreshed}");
            assert_eq!(ids(&mut s.texted()), [12], "refreshed: {refreshed}");
        }
        // changes through `get_mut` and full updates are seen at once: a
        // text set, a text cleared, an avatar gone
        let plain = s.index_of(&ObjKey { region: 1, local_id: 10 }).unwrap();
        s.get_mut(plain).unwrap().text = "Open".into();
        s.upsert(1, update(12, PRIM, ""));
        s.remove(&ObjKey { region: 1, local_id: 11 });
        assert_eq!(ids(&mut s.avatars()), [13]);
        assert_eq!(ids(&mut s.texted()), [10]);
        // the freed slot goes to a prim: it is not taken for the avatar
        let reused = s.upsert(1, update(14, PRIM, ""));
        assert_eq!(s.get(reused).unwrap().key.local_id, 14);
        assert_eq!(ids(&mut s.avatars()), [13]);
        s.refresh_labelled();
        assert_eq!(ids(&mut s.avatars()), [13]);
        assert_eq!(ids(&mut s.texted()), [10]);
        assert_eq!((s.avatar_slots.len(), s.texted_slots.len()), (1, 1));
        // a region left takes its objects out of the lists
        s.remove_region(1, &[]);
        s.refresh_labelled();
        assert_eq!(s.avatars().count() + s.texted().count(), 0);
        assert!(s.avatar_slots.is_empty() && s.texted_slots.is_empty());
    }

    #[test]
    fn particle_and_sound_changes_are_listed_without_a_walk() {
        use aurora_prim::params::LL_PCODE_VOLUME as PRIM;
        let taken = |s: &mut ObjectStore, particles: bool| {
            let mut out = Vec::new();
            if particles {
                s.take_particle_changes(&mut out);
            } else {
                s.take_sound_changes(&mut out);
            }
            out
        };
        let with = |local_id: u32, particles: ParticleUpdate| ObjectUpdate {
            particles,
            ..update(local_id, PRIM, "")
        };
        let mut s = ObjectStore::default();
        let plain = s.upsert(1, with(10, ParticleUpdate::Keep));
        let emitter = s.upsert(1, with(11, ParticleUpdate::Set(vec![1, 2, 3])));
        // every full update carries the attached sound; only one a particle
        // system
        assert_eq!(taken(&mut s, true), [emitter]);
        assert_eq!(taken(&mut s, false), [plain, emitter]);
        // what the managers have not handled yet stays listed once, whatever
        // touches the object meanwhile
        s.touch(emitter);
        s.get_mut(emitter).unwrap().text = "x".into();
        assert_eq!(taken(&mut s, true), [emitter]);
        assert_eq!(taken(&mut s, false), [emitter]);
        // handled: no longer listed, until the next change
        for idx in [plain, emitter] {
            let o = s.get_mut_untracked(idx).unwrap();
            o.particle_update = None;
            o.sound_update = false;
        }
        s.touch(emitter);
        s.touch(plain);
        assert!(taken(&mut s, true).is_empty() && taken(&mut s, false).is_empty());
        // a system stopped by an update that keeps the object, a full update
        // without particles
        s.upsert(1, with(11, ParticleUpdate::Clear));
        s.upsert(1, with(10, ParticleUpdate::Keep));
        assert_eq!(taken(&mut s, true), [emitter]);
        assert_eq!(taken(&mut s, false), [emitter, plain]);
        // a slot freed and filled again before the managers ran: listed once
        s.remove(&ObjKey { region: 1, local_id: 11 });
        let reused = s.upsert(1, with(12, ParticleUpdate::Set(vec![4])));
        assert_eq!(reused, emitter);
        assert_eq!(taken(&mut s, true), [emitter]);
        assert!(s.get(reused).unwrap().particle_update.is_some());
    }

    #[test]
    fn new_objects_are_arrivals_until_settled() {
        use aurora_prim::params::{LL_PCODE_LEGACY_AVATAR as AVATAR, LL_PCODE_VOLUME as PRIM};
        let mut s = ObjectStore::default();
        let prim = s.upsert(1, update(10, PRIM, ""));
        let avatar = s.upsert(1, update(11, AVATAR, ""));
        // an avatar is never put off
        assert!(s.is_arrival(prim) && !s.is_arrival(avatar) && !s.is_arrival(999));
        // updated again before its first sync: still new
        s.upsert(1, update(10, PRIM, "text"));
        assert!(s.is_arrival(prim));
        s.settle(prim);
        s.settle(999);
        assert!(!s.is_arrival(prim));
        // an update of a known object does not make it new again
        s.upsert(1, update(10, PRIM, ""));
        assert!(!s.is_arrival(prim));
        // removed before its first sync: the slot is no arrival, until a
        // new object takes it
        let gone = s.upsert(1, update(12, PRIM, ""));
        s.remove(&ObjKey { region: 1, local_id: 12 });
        assert!(!s.is_arrival(gone));
        let again = s.upsert(1, update(13, AVATAR, ""));
        assert_eq!(again, gone);
        assert!(!s.is_arrival(again));
        s.remove(&ObjKey { region: 1, local_id: 13 });
        let again = s.upsert(1, update(14, PRIM, ""));
        assert!(again == gone && s.is_arrival(again));
    }

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

    use aurora_prim::params::{LL_PCODE_LEGACY_AVATAR as AVATAR, LL_PCODE_VOLUME as PRIM};

    /// `id` under another local id, as another region sends it.
    fn resent(id: u32, local_id: u32, parent_id: u32, pcode: u8) -> ObjectUpdate {
        let mut u = update(local_id, pcode, "");
        u.full_id = Uuid::from_u128(id as u128);
        u.parent_id = parent_id;
        u
    }

    /// An avatar (10) wearing a prim (11) with a linked prim (12), in region 1.
    fn dressed() -> (ObjectStore, usize, usize, usize) {
        let mut s = ObjectStore::default();
        let avatar = s.upsert(1, update(10, AVATAR, ""));
        let worn = s.upsert(1, resent(11, 11, 10, PRIM));
        let linked = s.upsert(1, resent(12, 12, 11, PRIM));
        s.take_rekeyed();
        (s, avatar, worn, linked)
    }

    fn key(region: RegionHandle, local_id: u32) -> ObjKey {
        ObjKey { region, local_id }
    }

    #[test]
    fn object_coming_from_another_region_keeps_its_slot_and_what_hangs_from_it() {
        let (mut s, avatar, worn, linked) = dressed();
        // the avatar crosses to region 2, which sends it first
        assert_eq!(s.upsert(2, resent(10, 50, 0, AVATAR)), avatar);
        assert_eq!(s.get(avatar).expect("avatar").key, key(2, 50));
        assert!(s.removed_slots.is_empty(), "nothing was destroyed");
        assert_eq!(s.index_of(&key(1, 10)), None);
        assert_eq!(s.take_rekeyed(), [(key(1, 10), key(2, 50))]);
        // what it wears follows it, under the key it had
        assert_eq!(s.children_of(&key(2, 50)), [worn]);
        let w = s.get(worn).expect("worn");
        assert_eq!((w.key, w.parent_key()), (key(1, 11), Some(key(2, 50))));
        assert_eq!(s.parent_of(w), Some(avatar));
        // the region left asks to remove the avatar: no longer its object
        s.remove(&key(1, 10));
        assert!(s.get(avatar).is_some());
        // region 2 sends the worn prim, then its linked prim
        assert_eq!(s.upsert(2, resent(11, 51, 50, PRIM)), worn);
        assert_eq!(s.children_of(&key(2, 50)), [worn]);
        assert_eq!(s.children_of(&key(2, 51)), [linked]);
        assert_eq!(s.get(linked).expect("linked").parent_key(), Some(key(2, 51)));
        assert_eq!(s.upsert(2, resent(12, 52, 51, PRIM)), linked);
        assert_eq!(s.children_of(&key(2, 51)), [linked]);
        assert_eq!(s.count, 3);
        assert!(s.removed_slots.is_empty());
    }

    #[test]
    fn worn_object_sent_before_its_avatar_joins_it_again() {
        let (mut s, avatar, worn, _) = dressed();
        // region 2 sends the worn prim before the avatar it hangs from
        assert_eq!(s.upsert(2, resent(11, 51, 50, PRIM)), worn);
        assert!(s.children_of(&key(1, 10)).is_empty());
        assert_eq!(s.parent_of(s.get(worn).expect("worn")), None);
        assert_eq!(s.upsert(2, resent(10, 50, 0, AVATAR)), avatar);
        assert_eq!(s.children_of(&key(2, 50)), [worn]);
        assert_eq!(s.parent_of(s.get(worn).expect("worn")), Some(avatar));
    }

    #[test]
    fn renumbered_object_takes_its_children_along() {
        let (mut s, avatar, worn, _) = dressed();
        assert_eq!(s.upsert(1, resent(10, 60, 0, AVATAR)), avatar);
        assert_eq!(s.children_of(&key(1, 60)), [worn]);
        assert_eq!(s.parent_of(s.get(worn).expect("worn")), Some(avatar));
    }

    #[test]
    fn our_avatar_outlives_the_region_it_left() {
        let (mut s, avatar, worn, linked) = dressed();
        // it sat on a seat of region 1, next to another object
        let seat = s.upsert(1, update(30, PRIM, ""));
        s.upsert(1, resent(10, 10, 30, AVATAR));
        let other = s.upsert(1, update(40, PRIM, ""));
        assert_eq!(s.children_of(&key(1, 30)), [avatar]);
        // region 1 goes before region 2 has sent anything
        s.carry(avatar, 1, 2);
        let kept = s.subtree(avatar);
        assert_eq!(kept, [avatar, worn, linked]);
        s.remove_region(1, &kept);
        assert!(s.get(seat).is_none() && s.get(other).is_none());
        let a = s.get(avatar).expect("avatar");
        assert_eq!((a.key, a.parent_id), (key(2, CARRIED_LOCAL_ID), 0));
        assert_eq!(s.parent_of(s.get(worn).expect("worn")), Some(avatar));
        assert_eq!(s.parent_of(s.get(linked).expect("linked")), Some(worn));
        assert_eq!(s.count, 3);
        // region 2 sends it under its own id
        assert_eq!(s.upsert(2, resent(10, 50, 0, AVATAR)), avatar);
        assert_eq!(s.index_of(&key(2, CARRIED_LOCAL_ID)), None);
        assert_eq!(s.children_of(&key(2, 50)), [worn]);
        // an avatar already sent by region 2 is not carried again
        s.carry(avatar, 1, 2);
        assert_eq!(s.get(avatar).expect("avatar").key, key(2, 50));
    }
}
