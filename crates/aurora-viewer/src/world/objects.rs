//! Object store: every prim, mesh and avatar known to the viewer.

use super::edit_echo::{SentEdits, Transform};
use super::motion::{self, Correction};
use crate::fast_hash::FastMap;
use aurora_net::RegionHandle;
use aurora_net::build::TransformUpdate;
use aurora_net::objects::{ObjectUpdate, ParticleUpdate, TerseUpdate, TextureAnim};
use aurora_prim::{ExtraParams, TextureEntry, VolumeParams};
use glam::{Quat, Vec3, Vec4};
use std::collections::{BTreeSet, HashMap, HashSet};
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

/// Private namespace for render-only Blender attachments, never a grid handle.
pub const PREVIEW_REGION: RegionHandle = u64::MAX;

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
    /// Temporary material-inspector display mode; no simulator state changes.
    pub blinn_phong_preview: bool,
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

/// A root an avatar sits on: the pose drawn at the last frame, and whether
/// it is damped there or drawn where its reports send it (settled, or
/// still: a seat at rest costs nothing a frame).
#[derive(Debug, Clone, Copy)]
struct Seat {
    drawn: (Vec3, Quat),
    damping: bool,
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
    /// Simulator contact plane in region coordinates (zero means no contact).
    /// Keep it through updates without that block, as LLVOAvatar::mFootPlane.
    pub foot_plane: Option<Vec4>,
    /// Clock of its region when the simulator last reported its motion
    /// (`ObjectStore::motion_time`); NaN until the store has stamped it.
    motion_time: f64,
    /// The last report put it elsewhere than where it was drawn: the gap
    /// fades instead of showing at once (motion.rs).
    correction: Option<Correction>,
    /// A root an avatar sits on (`ObjectStore::damp_seats`).
    seat: Option<Seat>,
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
    pub fn is_client_preview(&self) -> bool {
        self.key.region == PREVIEW_REGION
    }
    pub fn is_editable_preview(&self) -> bool {
        self.is_client_preview() && self.update_flags & crate::ui::context::flags::OBJECT_MOVE != 0
    }
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
        if self.render.blinn_phong_preview {
            return None;
        }
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
            foot_plane: u.foot_plane,
            motion_time: f64::NAN,
            correction: None,
            seat: None,
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
            self.render.blinn_phong_preview = false;
            self.foot_plane = None;
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
        if let Some(plane) = u.foot_plane {
            self.foot_plane = Some(plane);
        }
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

    /// Apply a terse update (through `ObjectStore::apply_terse`, which also
    /// stamps the motion).
    fn apply_terse(&mut self, t: &TerseUpdate) {
        self.state = t.state;
        self.position = t.position;
        self.rotation = t.rotation;
        self.velocity = t.velocity;
        self.acceleration = t.acceleration;
        self.angular_velocity = t.angular_velocity;
        if let Some(plane) = t.foot_plane {
            self.foot_plane = Some(plane);
        }
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

    /// Local position and rotation drawn at `now`, `motion_time` being the
    /// clock of its region (`ObjectStore::motion_time`): where its last
    /// report sends it (motion.rs), and what is left of the gap that report
    /// opened with where it was drawn; a seat, where its damping has it.
    pub fn predicted(&self, now: Instant, motion_time: f64) -> (Vec3, Quat) {
        match self.seat {
            Some(Seat { drawn, damping: true }) => drawn,
            _ => self.undamped(now, motion_time),
        }
    }

    /// `predicted` without the damping of a seat.
    fn undamped(&self, now: Instant, motion_time: f64) -> (Vec3, Quat) {
        let pose = self.reported_pose(motion_time);
        match &self.correction {
            Some(c) if c.leads_to(self.position, self.rotation) => c.apply(pose, now),
            _ => pose,
        }
    }

    /// Where its last report sends it at `motion_time`, nothing smoothed.
    fn reported_pose(&self, motion_time: f64) -> (Vec3, Quat) {
        if self.extrapolates() && self.motion_time.is_finite() {
            motion::extrapolate(&self.reported(), motion_time - self.motion_time)
        } else {
            (self.position, self.rotation)
        }
    }

    fn reported(&self) -> motion::Reported {
        let (velocity, acceleration) = self.linear_motion();
        motion::Reported {
            position: self.position,
            rotation: self.rotation,
            velocity,
            acceleration,
            angular_velocity: self.angular_velocity,
        }
    }

    /// LLVOVolume::isAttachment: a prim worn by an avatar (trees, grass and
    /// avatars use the state for something else).
    pub fn is_attachment(&self) -> bool {
        self.pcode == aurora_prim::params::LL_PCODE_VOLUME && self.state != 0
    }

    /// Velocity and acceleration the object moves with. An attachment only
    /// spins (LLViewerObject::idleUpdate returns before
    /// interpolateLinearMotion): a velocity in the update of a worn prim
    /// would carry it away from its point, in the point's frame, until the
    /// next update.
    fn linear_motion(&self) -> (Vec3, Vec3) {
        if self.is_attachment() {
            (Vec3::ZERO, Vec3::ZERO)
        } else {
            (self.velocity, self.acceleration)
        }
    }

    /// The simulator reported a motion the object follows (velocity,
    /// acceleration or spin).
    fn extrapolates(&self) -> bool {
        let (velocity, acceleration) = self.linear_motion();
        velocity != Vec3::ZERO || acceleration != Vec3::ZERO || self.angular_velocity != Vec3::ZERO
    }

    /// Whether `predicted` changes from a frame to the next: a reported
    /// motion, a correction still fading (`ObjectStore::expire_corrections`
    /// ends it) or a seat still damped.
    pub fn has_motion(&self) -> bool {
        self.extrapolates() || self.correction.is_some() || self.seat_damping()
    }

    fn seat_damping(&self) -> bool {
        self.seat.is_some_and(|s| s.damping)
    }

    /// Put back the values of our own edit over a report of an earlier one
    /// (edit_echo.rs), or show the simulator's once the wait ends.
    fn keep_sent(&mut self, t: Transform) {
        if let Some(p) = t.position {
            self.position = p;
        }
        if let Some(r) = t.rotation {
            self.rotation = r;
        }
        if let Some(s) = t.scale {
            self.scale = s;
        }
        self.render.needs_records = true;
    }

    /// A report was applied at `now`: stamp it with its region's clock and,
    /// when it left the object where `drawn` says it was not, let the gap
    /// fade. True when a correction started. A damped seat needs none: its
    /// damping takes it there.
    fn stamp_motion(&mut self, motion_time: f64, now: Instant, drawn: Option<(Vec3, Quat)>) -> bool {
        self.motion_time = motion_time;
        let drawn = if self.seat_damping() { None } else { drawn };
        self.correction = drawn.and_then(|drawn| Correction::between(drawn, (self.position, self.rotation), now));
        self.correction.is_some()
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
    by_key: FastMap<ObjKey, usize>,
    by_uuid: FastMap<Uuid, usize>,
    /// Local BP display choices for this region visit; not saved in caches.
    blinn_phong_previews: HashSet<Uuid>,
    /// parent key -> child indices
    children: FastMap<ObjKey, Vec<usize>>,
    count: usize,
    /// Removed objects whose GPU resources must be released by the scene.
    pub graveyard: Vec<Object>,
    /// Their slot indices, in removal order (a slot may be reused since).
    pub removed_slots: Vec<usize>,
    /// What the simulator killed (KillObject) since the scene last took
    /// them, not what left with a region: each an object and what hung
    /// from it, its slots before the removal (the rez effect's derez).
    pub killed: Vec<Vec<usize>>,
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
    /// Clock of each region, in simulator seconds (motion.rs): what the
    /// objects of the region extrapolate along. Advanced by the world.
    motion_clocks: HashMap<RegionHandle, f64>,
    /// When the clocks were last advanced, and the rate each region ran at:
    /// what a report received since then is stamped from (`report_clock`).
    motion_tick: Option<Instant>,
    motion_rates: HashMap<RegionHandle, f32>,
    /// Slots whose object has a correction fading (`expire_corrections`).
    correcting: Vec<usize>,
    /// Slots of the roots avatars sat on at the last frame (`damp_seats`).
    seats: Vec<usize>,
    /// The photo mode's frozen world (`hold_poses`): where the objects that
    /// move are kept drawn.
    held: Option<HeldPoses>,
    /// Our transform edits waiting for the report that answers them: the
    /// reports of earlier sends do not show (edit_echo.rs).
    sent_edits: SentEdits,
}

/// The world frozen at an instant (issue #1154; LLPipeline::updateMove
/// returns at once under FreezeTime, indra/newview/pipeline.cpp, originally
/// LGPL 2.1): the objects keep the place they were drawn at, whatever the
/// simulator reports meanwhile. Reports are still applied, so the world
/// is current again as soon as it thaws.
struct HeldPoses {
    at: Instant,
    poses: HashMap<ObjKey, HeldPose>,
}

/// An object's place when the world froze, in its parent's coordinates.
#[derive(Debug, Clone, Copy)]
struct HeldPose {
    /// The parent it was drawn in: seated or unseated since, the pose
    /// means nothing in the new frame and the object goes where it is.
    parent_id: u32,
    pose: (Vec3, Quat),
}

impl ObjectStore {
    pub fn set_blinn_phong_preview(&mut self, idx: usize, preview: bool) {
        let Some(o) = self.get(idx) else { return };
        if o.render.blinn_phong_preview == preview {
            return;
        }
        let id = o.full_id;
        if preview {
            self.blinn_phong_previews.insert(id);
        } else {
            self.blinn_phong_previews.remove(&id);
        }
        if let Some(o) = self.get_mut(idx) {
            o.render.blinn_phong_preview = preview;
            o.material_dirty = true;
        }
    }

    /// End the region visit without scanning every world object. UUIDs follow
    /// simulator renumbering; removed/parked objects discard their choice too.
    pub fn clear_material_previews(&mut self) {
        for id in std::mem::take(&mut self.blinn_phong_previews) {
            if let Some(idx) = self.index_of_uuid(&id)
                && let Some(o) = self.get_mut(idx)
            {
                o.render.blinn_phong_preview = false;
                o.material_dirty = true;
            }
        }
    }

    /// Clock of a region, for `Object::predicted`.
    pub fn motion_time(&self, region: RegionHandle) -> f64 {
        self.motion_clocks.get(&region).copied().unwrap_or(0.0)
    }

    /// Let the clock of a region run for `seconds` of simulator time.
    pub fn advance_motion(&mut self, region: RegionHandle, seconds: f64) {
        if seconds > 0.0 {
            *self.motion_clocks.entry(region).or_insert(0.0) += seconds;
        }
    }

    /// The clocks were advanced up to `now`, `region`'s at `rate`.
    pub fn note_motion_tick(&mut self, now: Instant, region: RegionHandle, rate: f32) {
        self.motion_tick = Some(now);
        self.motion_rates.insert(region, rate);
    }

    /// The clock of a region at the moment a report was received: a report
    /// describes the object when the datagram was read, not when the main
    /// thread got to it (item 6 of the network audit). Without a time, or
    /// before the clocks ever ran, the clock as it is.
    pub fn report_clock(&self, region: RegionHandle, received: Option<Instant>) -> f64 {
        let clock = self.motion_time(region);
        let (Some(received), Some(tick)) = (received, self.motion_tick) else {
            return clock;
        };
        let rate = f64::from(self.motion_rates.get(&region).copied().unwrap_or(1.0));
        let offset = if received >= tick {
            received.duration_since(tick).as_secs_f64()
        } else {
            -tick.duration_since(received).as_secs_f64()
        };
        clock + rate * offset
    }

    /// Where an object is drawn at `now`, in its parent's (or its region's)
    /// coordinates.
    pub fn predicted(&self, o: &Object, now: Instant) -> (Vec3, Quat) {
        self.held_pose(o)
            .unwrap_or_else(|| o.predicted(now, self.motion_time(o.key.region)))
    }

    /// Freeze the objects at `at` (the photo mode's frozen world): those
    /// that move now are kept where they are drawn, the others as soon as a
    /// report would move them; `extra` adds poses known elsewhere (our own
    /// avatar, which the agent places). HUDs are the interface and go on.
    pub fn hold_poses(&mut self, at: Instant, extra: impl IntoIterator<Item = (ObjKey, (Vec3, Quat))>) {
        let mut poses = HashMap::new();
        for (idx, o) in self.iter() {
            if o.has_motion() && !self.worn_on_hud(idx) {
                let pose = o.predicted(at, self.motion_time(o.key.region));
                poses.insert(
                    o.key,
                    HeldPose {
                        parent_id: o.parent_id,
                        pose,
                    },
                );
            }
        }
        for (key, pose) in extra {
            if let Some(o) = self.index_of(&key).and_then(|idx| self.get(idx)) {
                let parent_id = o.parent_id;
                poses.insert(key, HeldPose { parent_id, pose });
            }
        }
        log::info!("world frozen: {} moving objects held", poses.len());
        self.held = Some(HeldPoses { at, poses });
    }

    /// Thaw the world: every object held is drawn where it is now (the
    /// Legacy Viewer's drawables move to their objects at once).
    pub fn release_poses(&mut self) {
        let Some(held) = self.held.take() else {
            return;
        };
        for key in held.poses.keys() {
            if let Some(&idx) = self.by_key.get(key) {
                self.touch(idx);
            }
        }
        log::info!("world thawed: {} objects released", held.poses.len());
    }

    /// The place `o` is held at while the world is frozen.
    pub fn held_pose(&self, o: &Object) -> Option<(Vec3, Quat)> {
        let held = self.held.as_ref()?.poses.get(&o.key)?;
        (held.parent_id == o.parent_id).then_some(held.pose)
    }

    /// A report is about to change `idx` while the world is frozen: the
    /// place it was drawn at is kept first.
    fn hold_before_report(&mut self, idx: usize) {
        let Some(at) = self.held.as_ref().map(|h| h.at) else {
            return;
        };
        let Some(o) = self.get(idx) else {
            return;
        };
        if self.held.as_ref().is_some_and(|h| h.poses.contains_key(&o.key)) || self.worn_on_hud(idx) {
            return;
        }
        let (key, parent_id) = (o.key, o.parent_id);
        let pose = o.predicted(at, self.motion_time(o.key.region));
        if let Some(held) = &mut self.held {
            held.poses.insert(key, HeldPose { parent_id, pose });
        }
    }

    /// Our transform edits sent to `region` (MultipleObjectUpdate): their
    /// values wait for the report that answers them (edit_echo.rs). The
    /// prims of a stretched linkset are moved and scaled by the simulator
    /// too: the values the build tools gave them locally wait as well.
    pub fn note_sent_transforms(&mut self, region: RegionHandle, updates: &[TransformUpdate], now: Instant) {
        for u in updates {
            let key = ObjKey {
                region,
                local_id: u.local_id,
            };
            let sent = Transform {
                position: u.position,
                rotation: u.rotation,
                scale: u.scale,
            };
            self.sent_edits.note(key, sent, now);
            if !(u.linked && u.scale.is_some()) {
                continue;
            }
            let children = self.children.get(&key).cloned().unwrap_or_default();
            for c in children {
                let Some(o) = self.get(c) else { continue };
                let child = Transform {
                    position: Some(o.position),
                    rotation: None,
                    scale: Some(o.scale),
                };
                self.sent_edits.note(o.key, child, now);
            }
        }
    }

    /// End the waits of our transform edits that got no answer: the
    /// simulator refused or limited them, its last report shows now.
    pub fn expire_sent_edits(&mut self, now: Instant) {
        for (key, late) in self.sent_edits.expire(now) {
            let Some(&idx) = self.by_key.get(&key) else { continue };
            if let Some(o) = self.get_mut(idx) {
                o.keep_sent(late);
            }
        }
    }

    /// End the corrections that have faded: the objects that do not move
    /// otherwise are still again, at the place of their last report, and
    /// the scene hears of it.
    pub fn expire_corrections(&mut self, now: Instant) {
        if self.correcting.is_empty() {
            return;
        }
        let mut listed = std::mem::take(&mut self.correcting);
        listed.retain(|&idx| {
            let Some(o) = self.slots.get_mut(idx).and_then(Option::as_mut) else {
                return false;
            };
            match &o.correction {
                Some(c) if c.active(now) => true,
                Some(_) => {
                    o.correction = None;
                    self.touched.note(idx);
                    false
                }
                None => false,
            }
        });
        self.correcting = listed;
    }

    /// One frame of the damping of the roots avatars sit on (motion::damp),
    /// `dt` seconds after the previous one. LLVOAvatar::idleUpdate damps the
    /// root of each seated avatar's linkset every frame, moving or not,
    /// except a root `selected` in the build tools, drawn where it is. A
    /// root its last rider left fades on from where it was drawn.
    pub fn damp_seats(&mut self, now: Instant, dt: f32, selected: impl Fn(&ObjKey) -> bool) {
        let mut ridden = Vec::new();
        for (_, avatar) in self.avatars() {
            if avatar.parent_id != 0
                && let Some(root) = self.seat_root(avatar)
                && !ridden.contains(&root)
            {
                ridden.push(root);
            }
        }
        let mut seats = std::mem::take(&mut self.seats);
        for &idx in &ridden {
            if !seats.contains(&idx) {
                seats.push(idx);
            }
        }
        for idx in seats {
            let Some(o) = self.get(idx) else { continue };
            let clock = self.motion_time(o.key.region);
            let target = o.reported_pose(clock);
            let rider = ridden.contains(&idx) && o.parent_id == 0;
            let seat = (rider && !selected(&o.key)).then(|| {
                // from where it was drawn at the last frame (its first: now)
                let undamped = o.undamped(now, clock);
                let drawn = o.seat.map_or(undamped, |s| s.drawn);
                match motion::damp(drawn, target, dt) {
                    Some(drawn) => Seat { drawn, damping: true },
                    None => Seat {
                        drawn: undamped,
                        damping: false,
                    },
                }
            });
            // (after this frame's step, which it still takes)
            let handover = match o.seat {
                Some(Seat { drawn, damping: true }) if !rider && o.parent_id == 0 => {
                    let drawn = motion::damp(drawn, target, dt).unwrap_or(target);
                    Correction::toward(drawn, target, (o.position, o.rotation), now)
                }
                _ => None,
            };
            // the scene hears when it starts or stops moving by itself
            let changed = o.seat_damping() != seat.is_some_and(|s| s.damping);
            let o = if changed { self.get_mut(idx) } else { self.get_mut_untracked(idx) };
            let Some(o) = o else { continue };
            o.seat = seat;
            if seat.is_some_and(|s| s.damping) {
                o.correction = None;
            } else if let Some(c) = handover {
                o.correction = Some(c);
                if !self.correcting.contains(&idx) {
                    self.correcting.push(idx);
                }
            }
            if seat.is_some() {
                self.seats.push(idx);
            }
        }
    }

    /// The root of the linkset `avatar` sits on, unless it sits on another
    /// avatar or a link of the chain is unknown.
    fn seat_root(&self, avatar: &Object) -> Option<usize> {
        let mut idx = self.parent_of(avatar)?;
        for _ in 0..16 {
            let o = self.get(idx)?;
            if o.is_avatar() {
                return None;
            }
            if o.parent_id == 0 {
                return Some(idx);
            }
            idx = self.parent_of(o)?;
        }
        None
    }

    /// Apply a terse update to the object in `idx`.
    #[cfg(test)]
    pub fn apply_terse(&mut self, idx: usize, t: &TerseUpdate) {
        self.apply_terse_at(idx, t, None);
    }

    /// Apply a terse update the network `received` at: its motion starts
    /// from that moment, and the gap it opens is measured with where the
    /// object was drawn for that same moment.
    pub fn apply_terse_at(&mut self, idx: usize, t: &TerseUpdate, received: Option<Instant>) {
        self.hold_before_report(idx);
        let now = received.unwrap_or_else(Instant::now);
        let Some(key) = self.get(idx).map(|o| o.key) else {
            return;
        };
        let clock = self.report_clock(key.region, received);
        let hud = self.worn_on_hud(idx);
        let reported = Transform {
            position: Some(t.position),
            rotation: Some(t.rotation),
            scale: None,
        };
        let sent = self.sent_edits.filter(key, reported, now);
        let Some(o) = self.get_mut(idx) else {
            return;
        };
        // A HUD shows a report at once: pages on the faces of a turning
        // cube flip instantly in the Legacy Viewer, with no fade.
        let drawn = (!hud).then(|| o.predicted(now, clock));
        o.apply_terse(t);
        if let Some(sent) = sent {
            o.keep_sent(sent);
        }
        if o.stamp_motion(clock, now, drawn) && !self.correcting.contains(&idx) {
            self.correcting.push(idx);
        }
    }

    /// Apply a full update to an object already known. `moved_frame`: its
    /// parent changed, so its coordinates are not those it was drawn in.
    fn apply_full(&mut self, idx: usize, region: RegionHandle, u: ObjectUpdate, moved_frame: bool, received: Option<Instant>) {
        self.hold_before_report(idx);
        let now = received.unwrap_or_else(Instant::now);
        let clock = self.report_clock(region, received);
        let hud = self.worn_on_hud(idx);
        let key = ObjKey {
            region,
            local_id: u.local_id,
        };
        let sent = if moved_frame {
            self.sent_edits.forget(&key);
            None
        } else {
            let reported = Transform {
                position: Some(u.position),
                rotation: Some(u.rotation),
                scale: Some(u.scale),
            };
            self.sent_edits.filter(key, reported, now)
        };
        let Some(o) = self.get_mut(idx) else {
            return;
        };
        let drawn = (!moved_frame && !hud).then(|| o.predicted(now, clock));
        o.apply_full(u);
        o.parent_region = region;
        if let Some(sent) = sent {
            o.keep_sent(sent);
        }
        if o.stamp_motion(clock, now, drawn) && !self.correcting.contains(&idx) {
            self.correcting.push(idx);
        }
    }

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
        // (the same age of its motion on the clock of its new region)
        let clocks = (self.motion_time(old.region), self.motion_time(key.region));
        if let Some(o) = self.get_mut(idx) {
            o.key = key;
            if old.region != key.region {
                o.foot_plane = None; // region-relative contact cannot follow a crossing
            }
            o.motion_time += clocks.1 - clocks.0;
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
    pub(crate) fn detach(&mut self, idx: usize) {
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
    #[cfg(test)]
    pub fn upsert(&mut self, region: RegionHandle, u: ObjectUpdate) -> usize {
        self.upsert_at(region, u, None)
    }

    /// `upsert` of a report the network `received` at (`apply_terse_at`).
    pub fn upsert_at(&mut self, region: RegionHandle, u: ObjectUpdate, received: Option<Instant>) -> usize {
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
            self.apply_full(idx, region, u, parent_changed, received);
            if parent_changed {
                self.link(idx);
            }
            let new_uuid = self.get(idx).map(|o| o.full_id);
            if let (Some(old), Some(new)) = (old_uuid, new_uuid)
                && old != new
            {
                self.by_uuid.remove(&old);
                self.blinn_phong_previews.remove(&old);
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
            // (it came under another key: its place is not corrected from
            // where it was drawn in the other region's coordinates)
            self.apply_full(old, region, u, true, received);
            if parent_changed {
                self.link(old);
            }
            self.touch(old);
            return old;
        }
        let mut obj = Object::from_update(region, u);
        if received.is_some() {
            obj.motion_time = self.report_clock(region, received);
        }
        self.insert(obj)
    }

    /// Insert a render-only attachment with an explicit real avatar parent.
    /// Its key cannot collide with a simulator object, even after crossings.
    #[cfg(test)]
    pub fn insert_preview_attachment(&mut self, parent: ObjKey, update: ObjectUpdate) -> usize {
        self.upsert_preview(Some(parent), update)
    }

    /// Update a private draw object without losing its stable UUID/slot. An
    /// explicit parent may be another preview batch or the real avatar.
    pub fn upsert_preview(&mut self, parent: Option<ObjKey>, update: ObjectUpdate) -> usize {
        if let Some(idx) = self
            .index_of_uuid(&update.full_id)
            .filter(|&idx| self.get(idx).is_some_and(Object::is_client_preview))
        {
            self.unlink(idx);
            if let Some(object) = self.get_mut(idx) {
                object.apply_full(update);
                object.parent_id = parent.map_or(0, |key| key.local_id);
                object.parent_region = parent.map_or(PREVIEW_REGION, |key| key.region);
            }
            self.link(idx);
            self.touch(idx);
            return idx;
        }
        let mut object = Object::from_update(PREVIEW_REGION, update);
        object.parent_id = parent.map_or(0, |key| key.local_id);
        object.parent_region = parent.map_or(PREVIEW_REGION, |key| key.region);
        self.insert(object)
    }

    /// Put a new object in a slot, under its key and its UUID.
    fn insert(&mut self, mut obj: Object) -> usize {
        let key = obj.key;
        if obj.motion_time.is_nan() {
            obj.motion_time = self.motion_time(key.region);
        }
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
        self.blinn_phong_previews.remove(&obj.full_id);
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

    /// A prim of a linkset worn on a HUD point (31–38). The simulator only
    /// sends the HUDs of our own avatar.
    fn worn_on_hud(&self, mut idx: usize) -> bool {
        for _ in 0..16 {
            let Some(o) = self.get(idx) else {
                return false;
            };
            let Some(parent) = self.parent_of(o) else {
                return false;
            };
            if self.get(parent).is_some_and(Object::is_avatar) {
                return (31..=38).contains(&o.attachment_point());
            }
            idx = parent;
        }
        false
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
    fn avatar_contact_plane_survives_absent_blocks_and_accepts_explicit_clears() {
        let plane = Vec4::new(0.0, 0.0, 1.0, 40.05);
        let mut full = update(1, AVATAR, "");
        full.foot_plane = Some(plane);
        let mut avatar = Object::from_update(1, full);
        assert_eq!(avatar.foot_plane, Some(plane));
        avatar.apply_full(update(1, AVATAR, ""));
        assert_eq!(avatar.foot_plane, Some(plane));
        let mut motion = terse(1, Vec3::ZERO, Vec3::ZERO);
        motion.foot_plane = Some(Vec4::ZERO);
        avatar.apply_terse(&motion);
        assert_eq!(avatar.foot_plane, Some(Vec4::ZERO));
        motion.foot_plane = None;
        avatar.apply_terse(&motion);
        assert_eq!(avatar.foot_plane, Some(Vec4::ZERO));
        motion.foot_plane = Some(plane * 2.0);
        avatar.apply_terse(&motion);
        assert_eq!(avatar.foot_plane, Some(plane * 2.0));
    }

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

    #[test]
    fn a_ridden_root_is_damped_every_frame_like_the_legacy_viewer() {
        let mut s = ObjectStore::default();
        let mut car = update(5, PRIM, "");
        car.velocity = Vec3::X * 10.0;
        let car = s.upsert(1, car);
        // the rider sits on a child prim: the linkset's root is damped
        let mut door = update(6, PRIM, "");
        door.parent_id = 5;
        s.upsert(1, door);
        let mut rider = update(10, AVATAR, "");
        rider.parent_id = 6;
        let rider = s.upsert(1, rider);
        let mut cart = update(20, PRIM, "");
        cart.velocity = Vec3::X * 10.0;
        let cart = s.upsert(1, cart);
        let mut chair = update(30, PRIM, "");
        chair.position = Vec3::Y * 3.0;
        let chair = s.upsert(1, chair);
        let mut sitter = update(31, AVATAR, "");
        sitter.parent_id = 30;
        s.upsert(1, sitter);
        let mut now = Instant::now();
        let dt = 1.0 / 60.0;
        let frames = |s: &mut ObjectStore, now: &mut Instant, n: u32, selected: bool| {
            for _ in 0..n {
                *now += Duration::from_secs_f32(dt);
                s.advance_motion(1, f64::from(dt));
                s.damp_seats(*now, dt, |k| selected && k.local_id == 5);
            }
        };
        let drawn = |s: &ObjectStore, idx: usize, now: Instant| s.predicted(s.get(idx).expect("object"), now);
        frames(&mut s, &mut now, 60, false);
        // nobody rides the cart: where its motion takes it; the car follows
        // its own a constant 0.78 m behind at 10 m/s (0.06 s half-life)
        near(drawn(&s, cart, now).0, Vec3::X * 10.0);
        let car_x = drawn(&s, car, now).0.x;
        assert!((car_x - 9.215).abs() < 0.02, "{car_x}");
        // the chair at rest is not moving: the scene does not place it again
        assert!(!s.get(chair).expect("chair").has_motion());
        near(drawn(&s, chair, now).0, Vec3::Y * 3.0);
        // a collision: thrown back and turned, then still; no jump on screen
        let mut hit = terse(5, Vec3::X * 9.0, Vec3::ZERO);
        (hit.is_avatar, hit.rotation) = (false, Quat::from_rotation_z(0.5));
        s.apply_terse_at(car, &hit, Some(now));
        near(drawn(&s, car, now).0, Vec3::X * car_x);
        frames(&mut s, &mut now, 1, false);
        let (position, rotation) = drawn(&s, car, now);
        assert!(position.distance(Vec3::X * car_x) < 0.05, "{position}");
        assert!(rotation.angle_between(Quat::IDENTITY) < 0.1, "{rotation}");
        frames(&mut s, &mut now, 60, false);
        near(drawn(&s, car, now).0, Vec3::X * 9.0);
        assert!(!s.get(car).expect("car").has_motion(), "settled");
        // selected in the build tools: drawn where it is
        let mut push = terse(5, Vec3::X * 9.0, Vec3::X * 10.0);
        push.is_avatar = false;
        s.apply_terse_at(car, &push, Some(now));
        frames(&mut s, &mut now, 30, true);
        near(drawn(&s, car, now).0, Vec3::X * 14.0);
        frames(&mut s, &mut now, 30, false);
        assert!(drawn(&s, car, now).0.x < 18.9, "damped again");
        // the rider stands up: the car fades on from where it is drawn
        let before = drawn(&s, car, now).0;
        let mut standing = update(10, AVATAR, "");
        standing.position = Vec3::new(19.0, 2.0, 0.0);
        s.upsert(1, standing);
        assert_eq!(s.get(rider).expect("rider").parent_id, 0);
        frames(&mut s, &mut now, 1, false);
        let after = drawn(&s, car, now).0;
        assert!(after.x > before.x && after.x - before.x < 0.4, "{before} -> {after}");
        frames(&mut s, &mut now, 60, false);
        // 121 frames at 10 m/s since the push from 9 m
        let x = drawn(&s, car, now).0.x;
        assert!((x - (9.0 + 10.0 * 121.0 * dt)).abs() < 0.01, "{x}");
    }

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

    fn terse(local_id: u32, position: Vec3, velocity: Vec3) -> TerseUpdate {
        TerseUpdate {
            local_id,
            state: 0,
            is_avatar: true,
            foot_plane: None,
            position,
            velocity,
            acceleration: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            angular_velocity: Vec3::ZERO,
            texture_entry: None,
        }
    }

    fn near(a: Vec3, b: Vec3) {
        assert!(a.distance(b) < 1e-3, "{a:?} != {b:?}");
    }

    #[test]
    fn reports_of_earlier_stretches_do_not_shake_the_stretched_linkset() {
        let mut s = ObjectStore::default();
        let root = s.upsert(1, update(5, PRIM, ""));
        let mut child = update(6, PRIM, "");
        child.parent_id = 5;
        child.position = Vec3::X * 0.5;
        child.scale = Vec3::splat(0.5);
        let child_idx = s.upsert(1, child.clone());
        // the build tools stretch the linkset twice as big, locally
        s.get_mut(root).expect("root").scale = Vec3::splat(2.0);
        let c = s.get_mut(child_idx).expect("child");
        c.position = Vec3::X;
        c.scale = Vec3::ONE;
        let stretch = |size: f32| TransformUpdate {
            local_id: 5,
            position: Some(Vec3::ZERO),
            rotation: None,
            scale: Some(Vec3::splat(size)),
            linked: true,
            uniform: true,
        };
        let now = Instant::now();
        s.note_sent_transforms(1, &[stretch(2.0)], now);
        let shown = |s: &ObjectStore, idx: usize| {
            let o = s.get(idx).expect("object");
            (s.predicted(o, Instant::now()).0, o.scale)
        };
        // the simulator's report of an earlier send, for the root and its prim
        let mut late = update(5, PRIM, "");
        late.scale = Vec3::splat(1.5);
        s.upsert(1, late);
        let mut late_child = child.clone();
        late_child.position = Vec3::X * 0.75;
        late_child.scale = Vec3::splat(0.75);
        s.upsert(1, late_child);
        assert_eq!(shown(&s, root), (Vec3::ZERO, Vec3::splat(2.0)));
        assert_eq!(shown(&s, child_idx), (Vec3::X, Vec3::ONE));
        assert!(s.get(child_idx).expect("child").correction.is_none());
        // the answers to the last send: nothing waits any more
        let mut answer = update(5, PRIM, "");
        answer.scale = Vec3::splat(2.0);
        s.upsert(1, answer);
        let mut answer_child = child.clone();
        answer_child.position = Vec3::X * 1.0001;
        answer_child.scale = Vec3::ONE;
        s.upsert(1, answer_child);
        s.upsert(1, child);
        assert_eq!(shown(&s, child_idx).1, Vec3::splat(0.5));
        // a refused stretch: the simulator's size shows once the wait ends
        s.note_sent_transforms(1, &[stretch(3.0)], now);
        let mut refused = update(5, PRIM, "");
        refused.scale = Vec3::splat(2.0);
        s.upsert(1, refused);
        assert_eq!(shown(&s, root).1, Vec3::splat(3.0));
        s.expire_sent_edits(now + std::time::Duration::from_secs(3));
        assert_eq!(shown(&s, root).1, Vec3::splat(2.0));
    }

    #[test]
    fn a_walker_goes_on_along_the_clock_of_its_own_region() {
        let mut s = ObjectStore::default();
        let now = Instant::now();
        // the region has been running for a while when the walker arrives
        s.advance_motion(1, 100.0);
        let mut walker = update(10, AVATAR, "");
        walker.velocity = Vec3::X * 3.2;
        let idx = s.upsert(1, walker);
        let drawn = |s: &ObjectStore| s.predicted(s.get(idx).expect("walker"), now).0;
        near(drawn(&s), Vec3::ZERO);
        // 5 s without a report: still walking (0.6 s was the old limit)
        s.advance_motion(1, 5.0);
        near(drawn(&s), Vec3::X * 16.0);
        // the clock of another region is not its own
        s.advance_motion(2, 50.0);
        near(drawn(&s), Vec3::X * 16.0);
        // a silent circuit stops the clock: it stays where it was
        s.advance_motion(1, 0.0);
        near(drawn(&s), Vec3::X * 16.0);
        // carried to region 2 (a border crossing): the same age of motion
        s.carry(idx, 1, 2);
        near(drawn(&s), Vec3::X * 16.0);
        s.advance_motion(2, 1.0);
        near(drawn(&s), Vec3::X * 19.2);
    }

    #[test]
    fn a_worn_prim_spins_but_never_drifts_with_a_reported_velocity() {
        let mut s = ObjectStore::default();
        let now = Instant::now();
        s.upsert(1, update(1, AVATAR, ""));
        // worn on the mouth (point 11), its update carrying a velocity
        let mut worn = update(10, PRIM, "");
        worn.parent_id = 1;
        worn.state = 0xB0;
        worn.position = Vec3::X * 0.2;
        worn.velocity = Vec3::Y * 0.5;
        worn.acceleration = Vec3::Z * -9.8;
        let idx = s.upsert(1, worn.clone());
        let o = s.get(idx).expect("worn prim");
        assert_eq!(o.attachment_point(), 11);
        assert!(o.is_attachment());
        // nothing moves it: placed from its point's anchor
        assert!(!o.has_motion());
        s.advance_motion(1, 2.0);
        near(s.predicted(s.get(idx).expect("worn prim"), now).0, Vec3::X * 0.2);
        // llTargetOmega still turns it, on the spot
        worn.angular_velocity = Vec3::Z * std::f32::consts::FRAC_PI_2;
        s.upsert(1, worn.clone());
        assert!(s.get(idx).expect("worn prim").has_motion());
        s.advance_motion(1, 1.0);
        let (position, rotation) = s.predicted(s.get(idx).expect("worn prim"), now);
        near(position, Vec3::X * 0.2);
        assert!(
            rotation.abs_diff_eq(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2), 1e-4),
            "{rotation:?}"
        );
        // the same prim on the ground goes its way
        worn.parent_id = 0;
        worn.state = 0;
        worn.acceleration = Vec3::ZERO;
        worn.angular_velocity = Vec3::ZERO;
        let ground = s.upsert(1, worn);
        assert!(!s.get(ground).expect("prim").is_attachment());
        s.advance_motion(1, 2.0);
        near(s.predicted(s.get(ground).expect("prim"), now).0, Vec3::X * 0.2 + Vec3::Y);
    }

    #[test]
    fn a_report_elsewhere_than_drawn_fades_in_instead_of_jumping() {
        let mut s = ObjectStore::default();
        let mut walker = update(10, AVATAR, "");
        walker.velocity = Vec3::X * 3.0;
        let idx = s.upsert(1, walker);
        // new: where its first report puts it, nothing to fade
        assert!(s.correcting.is_empty());
        s.advance_motion(1, 1.0);
        let mut touched = Vec::new();
        s.take_touched(&mut touched);
        // drawn at x = 3; the simulator says it stopped at x = 2
        s.apply_terse(idx, &terse(10, Vec3::X * 2.0, Vec3::ZERO));
        let applied = Instant::now();
        let at = |ms: u64| applied + Duration::from_millis(ms);
        let drawn = |s: &ObjectStore, t: Instant| s.predicted(s.get(idx).expect("walker"), t).0;
        assert!(drawn(&s, at(0)).x > 2.9, "drawn where it was");
        let half_way = drawn(&s, at(60)).x;
        assert!((2.4..2.6).contains(&half_way), "{half_way}");
        near(drawn(&s, at(700)), Vec3::X * 2.0);
        // at rest, yet placed every frame while it fades
        assert!(s.get(idx).expect("walker").has_motion());
        assert_eq!(s.correcting, [idx]);
        s.expire_corrections(at(100));
        assert_eq!(s.correcting, [idx]);
        touched.clear();
        s.take_touched(&mut touched);
        // faded: still again, and the scene is told to place it for good
        s.expire_corrections(at(700));
        assert!(s.correcting.is_empty());
        assert!(!s.get(idx).expect("walker").has_motion());
        touched.clear();
        s.take_touched(&mut touched);
        assert_eq!(touched, [idx]);
        near(drawn(&s, at(701)), Vec3::X * 2.0);
    }

    #[test]
    fn a_frozen_world_keeps_every_object_where_it_was_drawn() {
        let mut s = ObjectStore::default();
        let mut walker = update(10, AVATAR, "");
        walker.velocity = Vec3::X * 3.0;
        let walker = s.upsert(1, walker);
        let door = s.upsert(1, update(20, PRIM, ""));
        let mut chair = update(30, PRIM, "");
        chair.position = Vec3::Y * 4.0;
        s.upsert(1, chair);
        s.advance_motion(1, 1.0);
        let at = Instant::now();
        let own = (ObjKey { region: 1, local_id: 40 }, (Vec3::ONE, Quat::IDENTITY));
        s.upsert(1, update(40, AVATAR, ""));
        s.hold_poses(at, [own]);
        let drawn = |s: &ObjectStore, idx: usize| s.predicted(s.get(idx).expect("object"), at).0;
        let own_idx = s.index_of(&own.0).expect("own avatar");
        // the walker stops where it was drawn, its clock running on
        near(drawn(&s, walker), Vec3::X * 3.0);
        s.advance_motion(1, 2.0);
        near(drawn(&s, walker), Vec3::X * 3.0);
        // our avatar is held where the agent placed it
        near(drawn(&s, own_idx), Vec3::ONE);
        // reports still arrive: the door opens, the walker turns back, and
        // neither moves on screen
        s.apply_terse(door, &terse(20, Vec3::Z * 2.0, Vec3::ZERO));
        s.apply_terse(walker, &terse(10, Vec3::X * 9.0, Vec3::X * -3.0));
        near(drawn(&s, door), Vec3::ZERO);
        near(drawn(&s, walker), Vec3::X * 3.0);
        assert_eq!(s.get(door).expect("door").position, Vec3::Z * 2.0);
        // sitting meanwhile: the held place means nothing on the seat
        let mut seated = update(10, AVATAR, "");
        seated.parent_id = 30;
        seated.position = Vec3::Z * 0.5;
        s.upsert(1, seated);
        near(drawn(&s, walker), Vec3::Z * 0.5);
        // thawed: everything is where the simulator says, and the scene
        // places the objects that were held again
        let mut touched = Vec::new();
        s.take_touched(&mut touched);
        s.release_poses();
        touched.clear();
        s.take_touched(&mut touched);
        touched.sort_unstable();
        assert_eq!(touched, [walker, door, own_idx]);
        let later = at + Duration::from_secs(1);
        near(s.predicted(s.get(door).expect("door"), later).0, Vec3::Z * 2.0);
    }

    #[test]
    fn a_local_write_or_a_change_of_parent_shows_at_once() {
        let mut s = ObjectStore::default();
        let seat = s.upsert(1, update(5, PRIM, ""));
        let idx = s.upsert(1, update(10, AVATAR, ""));
        let now = Instant::now();
        let drawn = |s: &ObjectStore| s.predicted(s.get(idx).expect("object"), now).0;
        // the build tools drag it after a report that started a correction
        s.apply_terse(idx, &terse(10, Vec3::X * 2.0, Vec3::ZERO));
        assert!(drawn(&s).x < 0.1);
        s.get_mut(idx).expect("object").position = Vec3::X * 5.0;
        near(drawn(&s), Vec3::X * 5.0);
        // sitting: the coordinates become those of the seat, nothing to fade
        let mut seated = update(10, AVATAR, "");
        seated.parent_id = 5;
        seated.position = Vec3::Z * 0.5;
        s.upsert(1, seated);
        near(drawn(&s), Vec3::Z * 0.5);
        assert!(s.get(idx).expect("object").correction.is_none());
        // a full update that leaves it where it is starts nothing
        let mut same = update(5, PRIM, "");
        same.text = "open".into();
        s.upsert(1, same);
        assert!(!s.get(seat).expect("seat").has_motion());
        // a move of more than 10 m is a move, not a correction
        s.apply_terse(seat, &terse(5, Vec3::X * 40.0, Vec3::ZERO));
        assert!(!s.get(seat).expect("seat").has_motion());
    }

    #[test]
    fn a_hud_page_turns_at_once_while_a_worn_object_fades() {
        let mut s = ObjectStore::default();
        s.upsert(1, update(10, AVATAR, ""));
        let now = Instant::now();
        let quarter = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        for (point, root_id, fades) in [(31u8, 20, false), (2u8, 40, true)] {
            let mut root = update(root_id, PRIM, "");
            root.parent_id = 10;
            root.state = point.rotate_left(4);
            s.upsert(1, root);
            let mut cube = update(root_id + 1, PRIM, "");
            cube.parent_id = root_id;
            let cube_idx = s.upsert(1, cube.clone());
            // the page cube turns a quarter (llSetLinkPrimitiveParamsFast)
            cube.rotation = quarter;
            s.upsert(1, cube);
            let o = s.get(cube_idx).expect("cube");
            let drawn = s.predicted(o, now).1;
            assert_eq!(o.correction.is_some(), fades, "point {point}");
            assert_eq!(drawn.abs_diff_eq(quarter, 1e-4), !fades, "point {point}: {drawn:?}");
            // the HUD root itself turns at once too
            let root_idx = s
                .index_of(&ObjKey {
                    region: 1,
                    local_id: root_id,
                })
                .expect("root");
            s.apply_terse(
                root_idx,
                &TerseUpdate {
                    rotation: quarter,
                    is_avatar: false,
                    ..terse(root_id, Vec3::ZERO, Vec3::ZERO)
                },
            );
            let root = s.get(root_idx).expect("root");
            assert_eq!(root.correction.is_some(), fades, "root at point {point}");
        }
    }

    /// A report describes the object when the datagram was read: the wait
    /// before the main thread applied it is extrapolated, not lost (item 6
    /// of the network audit).
    #[test]
    fn a_report_moves_from_the_moment_it_was_received() {
        let mut s = ObjectStore::default();
        let tick = Instant::now();
        s.advance_motion(1, 10.0);
        s.note_motion_tick(tick, 1, 1.0);
        // read 50 ms before the clocks' last tick
        let mut walker = update(10, AVATAR, "");
        walker.velocity = Vec3::X * 4.0;
        let idx = s.upsert_at(1, walker, Some(tick - Duration::from_millis(50)));
        let drawn = |s: &ObjectStore| s.predicted(s.get(idx).expect("walker"), tick).0;
        near(drawn(&s), Vec3::X * 0.2);
        // read 20 ms after it: there 20 ms later on the clock
        s.apply_terse_at(
            idx,
            &terse(10, Vec3::X * 30.0, Vec3::X * 4.0),
            Some(tick + Duration::from_millis(20)),
        );
        near(drawn(&s), Vec3::X * 30.0);
        s.advance_motion(1, 0.12);
        near(drawn(&s), Vec3::X * 30.4);
        // a time dilation of one half: half as far on the clock
        s.note_motion_tick(tick, 1, 0.5);
        assert!((s.report_clock(1, Some(tick - Duration::from_millis(100))) - (10.12 - 0.05)).abs() < 1e-9);
        // without a time, the clock as it is
        assert_eq!(s.report_clock(1, None), s.motion_time(1));
    }
}
