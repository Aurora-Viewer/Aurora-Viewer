//! Which objects the scene sync visits each frame, kept up to date from
//! events instead of a pass over every object.
//!
//! - **Moving**: objects that move by themselves (velocity, acceleration or
//!   spin, avatars, our agent, rigged meshes) and everything that follows
//!   them (linkset children, attachments and their prims). Only the objects
//!   moving by themselves are stored; the followers are found each frame
//!   through the store's child lists, so links, unlinks, attachments and
//!   detachments need no bookkeeping. What the GPU places from its wearer's
//!   attachment point (anchors.rs) follows without a visit.
//! - **Dirty**: objects changed through the store (`ObjectStore::take_touched`),
//!   material users on a new material generation, and objects whose last
//!   sync could not finish. An object updated with `needs_records` brings
//!   its whole subtree along (its children are placed relative to it).
//! - **Pending**: objects whose wanted geometry is not bound yet; visited
//!   when that geometry can make progress (ready, or to be requested again).
//!   Beyond [`PENDING_PER_FRAME`] of them, a frame only looks at a part.
//!   Those waiting for a geometry still being built or downloaded are
//!   parked out of that round until it changes ([`SyncSets::geometry_changed`]),
//!   and put back in it every [`PARKED_SECONDS`] in case.
//! - **Detail level checks** ([`LodChecks::Events`]): an object at rest is
//!   visited when the camera has travelled far enough for its level to
//!   change (lod_watch.rs); what the GPU places from an avatar moves with
//!   it, and is visited in turn: every [`LOD_PERIOD`] frames while the
//!   camera moves, as by the slice below, every [`LOD_SECONDS`] while it
//!   rests; and every object every [`SWEEP_SECONDS`], a safety net for
//!   state changed behind the store's back. Never more per frame than the
//!   slice below.
//! - **LOD slice** ([`LodChecks::Slice`], `AURORA_SYNC_SLICE=1`): each
//!   object visited every [`LOD_PERIOD`] frames, as before the checks
//!   above. Only that frame's slice of the slab is walked.
//! - **Backlog**: objects whose full sync did not fit the frame's time
//!   budget; the sync takes them back, nearest first ([`Backlog`]).
//! - **Arrivals**: new objects, which wait for their first sync without
//!   being listed; the sync takes them in arrival order, within the same
//!   budget ([`Arrivals`]).

use super::lod_watch::LodWatch;
use super::sync_check::Sources;
use crate::world::objects::{Object, ObjectStore};
use glam::{Quat, Vec3};
use uuid::Uuid;

/// Frames between two detail-level checks of an object by the slice; with
/// the checks by events, the most a frame visits is still a slice of this.
pub const LOD_PERIOD: u32 = 24;

/// Time between two visits of an object the GPU places from an avatar
/// (worn, rigged) while the camera rests: the slice's period at 60 frames
/// per second, which a higher frame rate no longer shortens.
pub const LOD_SECONDS: f32 = LOD_PERIOD as f32 / 60.0;

/// Time between two visits of any object, whatever happened to it.
pub const SWEEP_SECONDS: f32 = 10.0;

/// How the sync comes back to the objects nothing brought to it.
pub enum LodChecks<'a> {
    /// Every object in turn, a slice of the slab per frame.
    Slice,
    /// When their detail level can have changed (see the module
    /// documentation). `anchored`: the objects the GPU places; `dt`: time
    /// since the last frame (s); `camera_moved`: since the last frame.
    Events {
        anchored: &'a [usize],
        dt: f32,
        camera_moved: bool,
    },
}

/// AURORA_SYNC_SLICE: the scene sync as before the checks by events (LOD
/// slice of every object, HUD attachments placed at each frame, whole
/// chunks of records uploaded), to compare. `1`: always; `alt`: one second
/// in three, the app switching at each line of the frame profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SliceEnv {
    Off,
    On,
    Alternate,
}

pub fn slice_env() -> SliceEnv {
    static ENV: std::sync::OnceLock<SliceEnv> = std::sync::OnceLock::new();
    *ENV.get_or_init(|| match std::env::var("AURORA_SYNC_SLICE").as_deref() {
        Ok("alt") => SliceEnv::Alternate,
        Ok("0") | Ok("") | Err(_) => SliceEnv::Off,
        Ok(_) => SliceEnv::On,
    })
}

static SLICE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static SLICE_SET: std::sync::Once = std::sync::Once::new();

/// Whether the sync works as before the checks by events, at the moment.
pub fn slice_mode() -> bool {
    SLICE_SET.call_once(|| set_slice_mode(slice_env() != SliceEnv::Off));
    SLICE.load(std::sync::atomic::Ordering::Relaxed)
}

/// Switch between the two ways; the renderer's uploads follow.
fn set_slice_mode(on: bool) {
    SLICE.store(on, std::sync::atomic::Ordering::Relaxed);
    aurora_render::set_whole_chunk_uploads(on);
}

/// `AURORA_SYNC_SLICE=alt`: switch to the way of second `n` of the run and
/// tell which it is (true: as before). The old way one second in three, so
/// that work done every other second falls on both. None without `alt`.
pub fn alternate_slice(n: u64) -> Option<bool> {
    (slice_env() == SliceEnv::Alternate).then(|| {
        let on = n.is_multiple_of(3);
        // (after the first read of the variable, which would undo it)
        slice_mode();
        set_slice_mode(on);
        on
    })
}

/// Objects a round visits in this frame, for a round of `period` seconds
/// over `len` objects: what `dt` owes, never more than a slice of them.
/// `credit` carries the fractions from frame to frame.
fn round_share(credit: &mut f32, len: usize, dt: f32, period: f32) -> usize {
    if len == 0 {
        *credit = 0.0;
        return 0;
    }
    let most = len.div_ceil(LOD_PERIOD as usize);
    *credit = (*credit + len as f32 * dt.clamp(0.0, 1.0) / period).min(most as f32);
    let n = *credit as usize;
    *credit -= n as f32;
    n
}

/// Objects waiting for their geometry that a frame asks about at most,
/// unless that would take more than [`PENDING_ROUND`] frames to see them
/// all: after a teleport, 12 000 objects waiting cost the frame list over
/// 1 ms in every frame until their geometry was there.
pub const PENDING_PER_FRAME: usize = 2048;
/// Frames in which every object waiting for its geometry is asked about,
/// however many there are.
pub const PENDING_ROUND: usize = 4;

/// How many of `waiting` objects a frame asks about.
fn pending_checks(waiting: usize) -> usize {
    waiting.min(waiting.div_ceil(PENDING_ROUND).max(PENDING_PER_FRAME))
}

/// Time between two returns of a parked object to the pending round,
/// whatever happened to its geometry: a safety net for a change nothing
/// announced.
pub const PARKED_SECONDS: f32 = 2.0;

/// What an object waiting for its geometry needs ([`SyncState::pending_due`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PendingGeom {
    /// Nothing pending any more.
    Done,
    /// A sync can make progress now.
    Due,
    /// Not yet, for a reason no event announces: asked again in the round.
    Later,
    /// Not before the geometry of this token (never 0) changes: parked.
    Waits(u64),
}

/// Whether objects waiting for a geometry are parked: not with
/// `AURORA_PENDING_PARK=0`, which asks about all of them in the round as
/// before, to compare.
fn parking() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("AURORA_PENDING_PARK").as_deref() != Ok("0"))
}

/// Parent chains longer than this are cut (as `Scene::object_transform`).
pub const MAX_DEPTH: u8 = 16;

/// World transform of an object: position, rotation, HUD attachment.
pub type Xf = (Vec3, Quat, bool);

/// Set of object indices: O(1) insert, remove and membership test.
#[derive(Default)]
pub struct IndexSet {
    items: Vec<usize>,
    /// Position in `items` + 1 (0: absent), by object index.
    pos: Vec<u32>,
}

impl IndexSet {
    pub fn insert(&mut self, idx: usize) -> bool {
        if self.contains(idx) {
            return false;
        }
        if self.pos.len() <= idx {
            self.pos.resize(idx + 1, 0);
        }
        self.items.push(idx);
        self.pos[idx] = self.items.len() as u32;
        true
    }

    pub fn remove(&mut self, idx: usize) -> bool {
        let Some(p) = self.pos.get(idx).copied().filter(|&p| p != 0) else {
            return false;
        };
        let at = p as usize - 1;
        self.items.swap_remove(at);
        if let Some(&moved) = self.items.get(at) {
            self.pos[moved] = p;
        }
        self.pos[idx] = 0;
        true
    }

    pub fn contains(&self, idx: usize) -> bool {
        self.pos.get(idx).is_some_and(|&p| p != 0)
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn as_slice(&self) -> &[usize] {
        &self.items
    }

    /// Every member, leaving the set empty.
    pub fn take(&mut self) -> Vec<usize> {
        for &i in &self.items {
            self.pos[i] = 0;
        }
        std::mem::take(&mut self.items)
    }
}

/// Per-frame marks by object index (cleared in O(1) by a new frame).
#[derive(Default)]
struct FrameMarks {
    stamp: Vec<u32>,
    frame: u32,
}

impl FrameMarks {
    fn next_frame(&mut self, n: usize) {
        if self.stamp.len() < n {
            self.stamp.resize(n, 0);
        }
        self.frame = self.frame.wrapping_add(1);
        if self.frame == 0 {
            self.stamp.iter_mut().for_each(|s| *s = 0);
            self.frame = 1;
        }
    }

    /// True the first time `idx` is marked this frame.
    fn mark(&mut self, idx: usize) -> bool {
        if self.stamp.len() <= idx {
            self.stamp.resize(idx + 1, 0);
        }
        let first = self.stamp[idx] != self.frame;
        self.stamp[idx] = self.frame;
        first
    }

    fn is_marked(&self, idx: usize) -> bool {
        self.stamp.get(idx) == Some(&self.frame)
    }
}

/// Transforms computed this frame, so the prims of an attachment or a
/// linkset reuse their parent's instead of walking up to the avatar again.
#[derive(Default)]
pub struct XfCache {
    stamp: Vec<u32>,
    xf: Vec<Xf>,
    frame: u32,
}

impl XfCache {
    pub fn next_frame(&mut self, n: usize) {
        if self.stamp.len() < n {
            self.stamp.resize(n, 0);
            self.xf.resize(n, (Vec3::ZERO, Quat::IDENTITY, false));
        }
        self.frame = self.frame.wrapping_add(1);
        if self.frame == 0 {
            self.stamp.iter_mut().for_each(|s| *s = 0);
            self.frame = 1;
        }
    }

    pub fn get(&self, idx: usize) -> Option<Xf> {
        (self.stamp.get(idx) == Some(&self.frame)).then(|| self.xf[idx])
    }

    pub fn set(&mut self, idx: usize, xf: Xf) {
        if let (Some(s), Some(x)) = (self.stamp.get_mut(idx), self.xf.get_mut(idx)) {
            *s = self.frame;
            *x = xf;
        }
    }
}

/// Whether an object moves by itself and so must be placed again every
/// frame, with everything that follows it.
pub fn moves_by_itself(o: &Object, agent: Uuid, rigged: bool) -> bool {
    o.has_motion() || o.is_avatar() || o.full_id == agent || rigged || o.is_flexible()
}

/// Ancestors of an object known to the store (0 for a root), at most
/// [`MAX_DEPTH`]: frame lists are placed level by level, parents first.
pub fn depth(objects: &ObjectStore, mut idx: usize) -> u8 {
    let mut d = 0;
    while d < MAX_DEPTH {
        match objects.get(idx).and_then(|o| objects.parent_of(o)) {
            Some(p) => {
                idx = p;
                d += 1;
            }
            None => break,
        }
    }
    d
}

/// Objects to sync this frame, with their depth in the parent chain.
#[derive(Default)]
pub struct FrameList {
    pub items: Vec<(usize, u8)>,
    /// How many of them each set listed (AURORA_SYNC_CHECK).
    pub sources: Sources,
    /// Objects the detail level check looked at.
    pub lod_checked: usize,
}

impl FrameList {
    /// Indices grouped by depth, parents' level first.
    pub fn levels(&self) -> Vec<Vec<usize>> {
        let mut levels: Vec<Vec<usize>> = Vec::new();
        for &(idx, d) in &self.items {
            let d = d as usize;
            if levels.len() <= d {
                levels.resize_with(d + 1, Vec::new);
            }
            levels[d].push(idx);
        }
        levels
    }
}

/// What the frame list needs to know about objects beyond the store.
pub trait SyncState {
    /// Rigged mesh the sync places again every frame with its skeleton
    /// (not one the GPU places).
    fn rigged(&self, idx: usize) -> bool;
    /// Attachment placed on a HUD point of its wearer (in the HUD's own
    /// space): it does not follow the wearer through the world.
    fn hud(&self, idx: usize) -> bool;
    /// Placed by the GPU from an anchor of its wearer's palette (a worn
    /// object from its attachment point, a rigged mesh from its skeleton):
    /// neither it nor what is linked below it follows a moving ancestor
    /// through the sync (what moves by itself below it is listed on its
    /// own).
    fn anchored(&self, idx: usize) -> bool;
    /// Must be synced: changed, or its last sync could not finish.
    fn needs_sync(&self, idx: usize, o: &Object) -> bool;
    /// Pending geometry: what the object waits for, if anything.
    fn pending_due(&self, idx: usize) -> PendingGeom;
}

/// Full syncs put off to later frames, nearest to the camera first.
///
/// A full sync (geometry request, faces built, textures acquired) costs a
/// few microseconds, but objects arrive by thousands: the region's object
/// cache answering, a teleport, a linkset of a few hundred prims. Done all
/// in the frame they arrive in, they made frames of 8 to 35 ms (Agni,
/// BackBone Main Store). The sync does what fits its time budget and
/// leaves the rest here; an object waits a few frames at most, the ones
/// near the camera less than the far ones.
#[derive(Default)]
pub struct Backlog {
    /// (distance key, object index), nearest first. An entry whose object
    /// is no longer queued (synced, removed) is skipped when it comes up.
    heap: std::collections::BinaryHeap<std::cmp::Reverse<(u32, u32)>>,
    queued: Vec<bool>,
    len: usize,
}

impl Backlog {
    /// Queue an object at `distance` (m) from the camera. False when it
    /// was already waiting (it keeps its place).
    pub fn push(&mut self, idx: usize, distance: f32) -> bool {
        if self.queued.len() <= idx {
            self.queued.resize(idx + 1, false);
        }
        if std::mem::replace(&mut self.queued[idx], true) {
            return false;
        }
        self.len += 1;
        // 1/16 m steps; `as` saturates (NaN: 0, treated as nearest)
        let key = (distance.max(0.0) * 16.0) as u32;
        self.heap.push(std::cmp::Reverse((key, idx as u32)));
        true
    }

    pub fn contains(&self, idx: usize) -> bool {
        self.queued.get(idx).copied().unwrap_or(false)
    }

    /// The object was synced another way, or removed.
    pub fn remove(&mut self, idx: usize) {
        if let Some(q) = self.queued.get_mut(idx)
            && std::mem::replace(q, false)
        {
            self.len -= 1;
            if self.len == 0 {
                // nothing but skipped entries left
                self.heap.clear();
            }
        }
    }

    /// The nearest waiting object.
    pub fn pop(&mut self) -> Option<usize> {
        while let Some(std::cmp::Reverse((_, idx))) = self.heap.pop() {
            let idx = idx as usize;
            if self.contains(idx) {
                self.remove(idx);
                return Some(idx);
            }
        }
        None
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// New objects waiting for their first sync, in arrival order.
///
/// An object that was never synced has nothing on screen to keep in place:
/// its first sync can wait a few frames. Listed like a changed object, it
/// went through the frame list and the parallel placement only for the
/// sync to find that out, and thousands arriving together (the region's
/// object cache answering, a region coming into view) made that frame
/// several milliseconds long whatever the budget of the full syncs. Here
/// an arrival costs a push; the frame list leaves these objects alone, and
/// the sync takes them within its time budget, as it does the [`Backlog`].
#[derive(Default)]
pub struct Arrivals {
    /// An entry whose object is no longer queued (removed) is skipped when
    /// it comes up.
    queue: std::collections::VecDeque<u32>,
    queued: Vec<bool>,
    len: usize,
}

impl Arrivals {
    /// Queue an object. False when it was already waiting (it keeps its
    /// place).
    pub fn push(&mut self, idx: usize) -> bool {
        if self.queued.len() <= idx {
            self.queued.resize(idx + 1, false);
        }
        if std::mem::replace(&mut self.queued[idx], true) {
            return false;
        }
        self.len += 1;
        self.queue.push_back(idx as u32);
        true
    }

    pub fn contains(&self, idx: usize) -> bool {
        self.queued.get(idx).copied().unwrap_or(false)
    }

    /// The object was removed.
    pub fn remove(&mut self, idx: usize) {
        if let Some(q) = self.queued.get_mut(idx)
            && std::mem::replace(q, false)
        {
            self.len -= 1;
            if self.len == 0 {
                // nothing but skipped entries left
                self.queue.clear();
            }
        }
    }

    /// The object that has waited longest.
    pub fn pop(&mut self) -> Option<usize> {
        while let Some(idx) = self.queue.pop_front() {
            let idx = idx as usize;
            if self.contains(idx) {
                self.remove(idx);
                return Some(idx);
            }
        }
        None
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// Main-thread time a frame still has for the full syncs that can wait.
/// Only their own time counts: what the frame must do anyway (moves,
/// avatars) does not eat it.
pub struct FullSyncBudget {
    left: std::time::Duration,
    done: u32,
}

impl FullSyncBudget {
    pub fn new(budget: std::time::Duration) -> Self {
        FullSyncBudget { left: budget, done: 0 }
    }

    /// Whether another full sync may start: while time is left, and
    /// always the first of the frame (the backlog advances whatever the
    /// budget).
    pub fn allows(&self) -> bool {
        self.allows_beyond(std::time::Duration::ZERO)
    }

    /// The same, for work that must leave `reserve` of the budget to what
    /// comes after it.
    pub fn allows_beyond(&self, reserve: std::time::Duration) -> bool {
        self.left > reserve || self.done == 0
    }

    pub fn left(&self) -> std::time::Duration {
        self.left
    }

    /// A full sync took `spent`.
    pub fn spend(&mut self, spent: std::time::Duration) {
        self.left = self.left.saturating_sub(spent);
        self.done += 1;
    }
}

/// The event-driven sets of the scene sync (see the module documentation).
#[derive(Default)]
pub struct SyncSets {
    /// Objects moving by themselves (validated each frame, removed lazily).
    pub own_moving: IndexSet,
    pub dirty: IndexSet,
    pub pending: IndexSet,
    /// Pending objects waiting for a geometry being built or downloaded,
    /// out of the pending round (never in `pending` at the same time).
    parked: IndexSet,
    /// The token whose list holds an object (0: none), by object index.
    parked_on: Vec<u64>,
    /// Objects to put back in the round when a geometry changes, by its
    /// token (stale entries are skipped: `parked_on` tells).
    parked_by: std::collections::HashMap<u64, Vec<usize>>,
    /// Objects whose faces use materials (glTF or legacy).
    pub material_users: IndexSet,
    /// Material generation the material users were last checked against.
    pub seen_mat_gen: u64,
    /// Objects whose full sync waits for a frame with time left.
    pub backlog: Backlog,
    /// New objects waiting for their first sync.
    pub arrivals: Arrivals,
    /// Objects at rest, due for a detail level check when the camera has
    /// travelled enough.
    pub lod: LodWatch,
    /// Where the next frame goes on in `pending`, in the objects the GPU
    /// places and in the slab (the sweep), with what the rounds are owed.
    pending_next: usize,
    parked_next: usize,
    parked_credit: f32,
    anchored_next: usize,
    anchored_credit: f32,
    sweep_next: usize,
    sweep_credit: f32,
    listed: FrameMarks,
    /// Listed this frame by a detail level check and nothing else.
    checked: FrameMarks,
    /// Subtrees listed this frame: but for what the GPU places, and whole.
    expanded: FrameMarks,
    expanded_whole: FrameMarks,
    /// Stack of `push_tree`, kept for its room.
    tree_stack: Vec<(usize, u8)>,
}

impl SyncSets {
    pub fn clear(&mut self) {
        *self = SyncSets::default();
    }

    /// An object freed from the store.
    pub fn forget(&mut self, idx: usize) {
        self.own_moving.remove(idx);
        self.dirty.remove(idx);
        self.pending.remove(idx);
        self.parked.remove(idx);
        if let Some(t) = self.parked_on.get_mut(idx) {
            *t = 0;
        }
        self.material_users.remove(idx);
        self.backlog.remove(idx);
        self.arrivals.remove(idx);
        self.lod.forget(idx);
    }

    /// After a full sync: whether the object still waits for its geometry
    /// (asked about in the round from the next frame on).
    pub fn geometry_wanted(&mut self, idx: usize, waiting: bool) {
        self.parked.remove(idx);
        if waiting {
            self.pending.insert(idx);
        } else {
            self.pending.remove(idx);
        }
    }

    /// The geometry of `token` is ready or gone: the objects parked for it
    /// go back to the pending round.
    pub fn geometry_changed(&mut self, token: u64) {
        let Some(list) = self.parked_by.remove(&token) else {
            return;
        };
        for idx in list {
            if self.parked_on.get(idx) == Some(&token) {
                self.parked_on[idx] = 0;
                if self.parked.remove(idx) {
                    self.pending.insert(idx);
                }
            }
        }
    }

    /// Out of the pending round until the geometry of `token` changes.
    fn park(&mut self, idx: usize, token: u64) {
        self.pending.remove(idx);
        self.parked.insert(idx);
        if self.parked_on.len() <= idx {
            self.parked_on.resize(idx + 1, 0);
        }
        // (already in its list: a full sync took it out of the round and
        // back, the geometry it waits for is the same)
        if self.parked_on[idx] != token {
            self.parked_on[idx] = token;
            self.parked_by.entry(token).or_default().push(idx);
        }
    }

    #[cfg(test)]
    fn parked_len(&self) -> usize {
        self.parked.len()
    }

    /// Listed in this frame because it follows something that moves: it
    /// will be again in the next one.
    pub fn follows_motion(&self, idx: usize) -> bool {
        self.expanded.is_marked(idx)
    }

    /// Listed in this frame only because its turn had come for a detail
    /// level check: nothing is known to have changed for it.
    pub fn only_checked(&self, idx: usize) -> bool {
        self.checked.is_marked(idx)
    }

    /// An object changed through the store: synced this frame, and moving
    /// from now on if it moves by itself (it stops being moving lazily).
    pub fn touched(&mut self, objects: &ObjectStore, agent: Uuid, idx: usize, rigged: bool) {
        let Some(o) = objects.get(idx) else {
            return;
        };
        self.dirty.insert(idx);
        if moves_by_itself(o, agent, rigged) {
            self.own_moving.insert(idx);
        }
    }

    /// After a full sync: a mesh found rigged starts moving.
    pub fn synced(&mut self, objects: &ObjectStore, agent: Uuid, idx: usize, rigged: bool) {
        if objects.get(idx).is_some_and(|o| moves_by_itself(o, agent, rigged)) {
            self.own_moving.insert(idx);
        }
    }

    /// Objects to sync this frame `frame` (slab of `n` slots): moving
    /// subtrees, dirty objects (subtrees of the updated ones), pending
    /// geometry that can progress and the frame's detail level checks.
    /// Each once.
    pub fn frame_list(
        &mut self,
        objects: &ObjectStore,
        agent: Uuid,
        frame: u32,
        n: usize,
        state: &impl SyncState,
        checks: LodChecks,
    ) -> FrameList {
        self.listed.next_frame(n);
        self.checked.next_frame(n);
        self.expanded.next_frame(n);
        self.expanded_whole.next_frame(n);
        let mut list = FrameList::default();
        let mut i = 0;
        while i < self.own_moving.len() {
            let idx = self.own_moving.items[i];
            match objects.get(idx) {
                Some(o) if moves_by_itself(o, agent, state.rigged(idx)) => {
                    self.push_followers(objects, idx, &mut list, state);
                    i += 1;
                }
                // stopped, or removed: swap-removed, the next one takes its place
                _ => {
                    self.own_moving.remove(idx);
                }
            }
        }
        list.sources.moving = list.items.len();
        for idx in self.dirty.take() {
            let Some(o) = objects.get(idx) else {
                continue;
            };
            if !state.needs_sync(idx, o) {
                continue;
            }
            if o.render.needs_records && o.is_avatar() {
                // what it wears is placed relative to its attachment
                // points, not to it: the anchored objects stay put
                self.push_followers(objects, idx, &mut list, state);
            } else if o.render.needs_records {
                // (anchored objects too: they are placed relative to it)
                self.push_whole(objects, idx, &mut list);
            } else {
                self.push_one(objects, idx, &mut list);
            }
        }
        list.sources.dirty = list.items.len() - list.sources.moving;
        let before_pending = list.items.len();
        // parked objects asked about again in turn, all of them every
        // PARKED_SECONDS (with the slice, back in the round at once: the
        // way before)
        let park = match checks {
            LodChecks::Events { dt, .. } if parking() => Some(dt),
            _ => None,
        };
        let again = match park {
            Some(dt) => round_share(&mut self.parked_credit, self.parked.len(), dt, PARKED_SECONDS),
            None => self.parked.len(),
        };
        for _ in 0..again {
            if self.parked_next >= self.parked.len() {
                self.parked_next = 0;
            }
            let Some(&idx) = self.parked.items.get(self.parked_next) else {
                break;
            };
            match park.map(|_| state.pending_due(idx)) {
                // still waiting for a geometry in progress (maybe another)
                Some(PendingGeom::Waits(token)) => {
                    self.park(idx, token);
                    self.parked_next += 1;
                }
                // back in the round (swap-removed: the last one takes its place)
                _ => {
                    self.parked.remove(idx);
                    self.pending.insert(idx);
                }
            }
        }
        // (all of them while they are few; else a part, the next frame
        // going on where this one stops)
        let mut i = self.pending_next;
        for _ in 0..pending_checks(self.pending.len()) {
            if i >= self.pending.len() {
                i = 0;
            }
            let Some(&idx) = self.pending.items.get(i) else {
                break;
            };
            // already waiting for its full sync: not planned again each frame
            if self.backlog.contains(idx) {
                i += 1;
                continue;
            }
            match state.pending_due(idx) {
                // (swap-removed: the last one takes its place)
                PendingGeom::Done => {
                    self.pending.remove(idx);
                    continue;
                }
                PendingGeom::Waits(token) if park.is_some() => {
                    self.park(idx, token);
                    continue;
                }
                PendingGeom::Due => self.push_one(objects, idx, &mut list),
                PendingGeom::Later | PendingGeom::Waits(_) => {}
            }
            i += 1;
        }
        self.pending_next = i;
        list.sources.pending = list.items.len() - before_pending;
        let before_lod = list.items.len();
        match checks {
            LodChecks::Slice => {
                // (frame + idx) is a multiple of LOD_PERIOD
                let start = ((LOD_PERIOD - frame % LOD_PERIOD) % LOD_PERIOD) as usize;
                for idx in (start..n).step_by(LOD_PERIOD as usize) {
                    self.push_check(objects, agent, idx, &mut list, state);
                }
            }
            LodChecks::Events {
                anchored,
                dt,
                camera_moved,
            } => {
                let most = n.div_ceil(LOD_PERIOD as usize);
                // at rest: those the camera has travelled enough for
                for _ in 0..most {
                    let Some(idx) = self.lod.pop_due() else {
                        break;
                    };
                    self.push_check(objects, agent, idx, &mut list, state);
                }
                // placed by the GPU from an avatar: in turn, at the slice's
                // pace while the camera moves
                let turn = if camera_moved {
                    anchored.len().div_ceil(LOD_PERIOD as usize)
                } else {
                    round_share(&mut self.anchored_credit, anchored.len(), dt, LOD_SECONDS)
                };
                for _ in 0..turn {
                    if self.anchored_next >= anchored.len() {
                        self.anchored_next = 0;
                    }
                    self.push_check(objects, agent, anchored[self.anchored_next], &mut list, state);
                    self.anchored_next += 1;
                }
                // everything, slowly
                for _ in 0..round_share(&mut self.sweep_credit, n, dt, SWEEP_SECONDS) {
                    if self.sweep_next >= n {
                        self.sweep_next = 0;
                    }
                    self.push_check(objects, agent, self.sweep_next, &mut list, state);
                    self.sweep_next += 1;
                }
            }
        }
        list.sources.lod = list.items.len() - before_lod;
        list
    }

    /// An object whose turn has come for a detail level check.
    fn push_check(&mut self, objects: &ObjectStore, agent: Uuid, idx: usize, list: &mut FrameList, state: &impl SyncState) {
        // (its first sync chooses its detail level)
        if self.arrivals.contains(idx) {
            return;
        }
        let Some(o) = objects.get(idx) else {
            return;
        };
        if moves_by_itself(o, agent, state.rigged(idx)) {
            self.own_moving.insert(idx);
        }
        list.lod_checked += 1;
        if self.listed.mark(idx) {
            self.checked.mark(idx);
            list.items.push((idx, depth(objects, idx)));
        }
    }

    fn push_one(&mut self, objects: &ObjectStore, idx: usize, list: &mut FrameList) {
        if self.listed.mark(idx) {
            list.items.push((idx, depth(objects, idx)));
        }
    }

    /// A moving object and what follows it through the sync: everything
    /// below it (children, attachments, their prims) but the subtrees of
    /// the objects the GPU places.
    fn push_followers(&mut self, objects: &ObjectStore, root: usize, list: &mut FrameList, state: &impl SyncState) {
        if self.expanded_whole.is_marked(root) || !self.expanded.mark(root) {
            return;
        }
        // (one allocation per call showed when thousands of objects arrive
        // in the same frame)
        let mut stack = std::mem::take(&mut self.tree_stack);
        stack.push((root, depth(objects, root)));
        while let Some((idx, d)) = stack.pop() {
            if self.listed.mark(idx) {
                list.items.push((idx, d));
            }
            let Some(o) = objects.get(idx) else {
                continue;
            };
            if d >= MAX_DEPTH {
                continue;
            }
            // (what is on a HUD point stays where it is on the screen)
            let wearer = o.is_avatar();
            for &c in objects.children_of(&o.key) {
                // a subtree already listed (another moving object)
                if !state.anchored(c) && !(wearer && state.hud(c)) && !self.expanded_whole.is_marked(c) && self.expanded.mark(c) {
                    stack.push((c, d + 1));
                }
            }
        }
        self.tree_stack = stack;
    }

    /// An object and everything below it, the objects the GPU places too.
    fn push_whole(&mut self, objects: &ObjectStore, root: usize, list: &mut FrameList) {
        if !self.expanded_whole.mark(root) {
            return;
        }
        let mut stack = std::mem::take(&mut self.tree_stack);
        stack.push((root, depth(objects, root)));
        while let Some((idx, d)) = stack.pop() {
            if self.listed.mark(idx) {
                list.items.push((idx, d));
            }
            let Some(o) = objects.get(idx) else {
                continue;
            };
            if d >= MAX_DEPTH {
                continue;
            }
            for &c in objects.children_of(&o.key) {
                // a subtree already listed whole (another updated object)
                if self.expanded_whole.mark(c) {
                    stack.push((c, d + 1));
                }
            }
        }
        self.tree_stack = stack;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::avatar::AvatarLibrary;
    use crate::world::World;
    use aurora_net::NetEvent;
    use aurora_net::objects::ObjectUpdate;
    use std::collections::HashSet;
    use std::sync::Arc;

    /// Test double of the scene state: rigged meshes, objects needing a
    /// sync beyond their flags, pending geometry.
    #[derive(Default)]
    struct State {
        rigged: HashSet<usize>,
        hud: HashSet<usize>,
        anchored: HashSet<usize>,
        extra: HashSet<usize>,
        pending: Vec<(usize, bool)>,
        /// Answer for the objects that are not in `pending`.
        pending_else: Option<bool>,
        /// Objects waiting for the geometry of a token (before `pending`).
        waits: Vec<(usize, u64)>,
        /// Calls of `pending_due`.
        asked: std::cell::Cell<usize>,
    }

    impl SyncState for State {
        fn rigged(&self, idx: usize) -> bool {
            self.rigged.contains(&idx)
        }
        fn hud(&self, idx: usize) -> bool {
            self.hud.contains(&idx)
        }
        fn anchored(&self, idx: usize) -> bool {
            self.anchored.contains(&idx)
        }
        fn needs_sync(&self, idx: usize, o: &Object) -> bool {
            o.render.needs_records || o.shape_dirty || o.material_dirty || self.extra.contains(&idx)
        }
        fn pending_due(&self, idx: usize) -> PendingGeom {
            self.asked.set(self.asked.get() + 1);
            if let Some(&(_, token)) = self.waits.iter().find(|w| w.0 == idx) {
                return PendingGeom::Waits(token);
            }
            match self.pending.iter().find(|p| p.0 == idx).map(|p| p.1).or(self.pending_else) {
                Some(true) => PendingGeom::Due,
                Some(false) => PendingGeom::Later,
                None => PendingGeom::Done,
            }
        }
    }

    struct Fixture {
        world: World,
        handle: u64,
        prim: ObjectUpdate,
        sets: SyncSets,
        frame: u32,
    }

    impl Fixture {
        fn new() -> Fixture {
            let world = World::new(Arc::new(AvatarLibrary::load()));
            let (handle, prim) = crate::demo::events()
                .into_iter()
                .find_map(|ev| match ev {
                    NetEvent::ObjectUpdates { handle, objects } => objects.into_iter().find(|o| !o.is_avatar()).map(|o| (handle, o)),
                    _ => None,
                })
                .expect("demo contains a prim");
            Fixture {
                world,
                handle,
                prim,
                sets: SyncSets::default(),
                frame: 0,
            }
        }

        /// A still prim (or an avatar) with this local id and parent.
        fn add(&mut self, local_id: u32, parent_id: u32, avatar: bool) -> usize {
            let mut u = self.prim.clone();
            u.local_id = local_id;
            u.full_id = Uuid::from_u128(local_id as u128);
            u.parent_id = parent_id;
            u.velocity = Vec3::ZERO;
            u.acceleration = Vec3::ZERO;
            u.angular_velocity = Vec3::ZERO;
            if avatar {
                u.pcode = aurora_prim::params::LL_PCODE_LEGACY_AVATAR;
            }
            self.world.objects.upsert(self.handle, u)
        }

        /// Update an object as the network would (full update).
        fn update(&mut self, idx: usize, f: impl FnOnce(&mut ObjectUpdate)) {
            let o = self.world.objects.get(idx).expect("object");
            let mut u = self.prim.clone();
            u.local_id = o.key.local_id;
            u.full_id = o.full_id;
            u.parent_id = o.parent_id;
            u.pcode = o.pcode;
            u.velocity = o.velocity;
            u.acceleration = Vec3::ZERO;
            u.angular_velocity = o.angular_velocity;
            f(&mut u);
            self.world.objects.upsert(self.handle, u);
        }

        /// One frame of the sync bookkeeping: touched objects, the frame
        /// list, then every listed object synced (flags cleared).
        fn frame(&mut self, state: &State) -> Vec<usize> {
            self.frame += 1;
            let agent = self.world.agent_id;
            let mut touched = Vec::new();
            self.world.objects.take_touched(&mut touched);
            for idx in touched {
                self.sets.touched(&self.world.objects, agent, idx, state.rigged(idx));
            }
            let n = self.world.objects.slots.len();
            let list = self
                .sets
                .frame_list(&self.world.objects, agent, self.frame, n, state, LodChecks::Slice);
            let mut out: Vec<usize> = list.items.iter().map(|i| i.0).collect();
            for &idx in &out {
                if let Some(o) = self.world.objects.get_mut_untracked(idx) {
                    o.render.needs_records = false;
                    o.shape_dirty = false;
                    o.material_dirty = false;
                }
            }
            out.sort_unstable();
            out
        }

        /// One frame with the detail level checks by events, `dt` after
        /// the last one; nothing is watched again by itself.
        fn frame_events(&mut self, state: &State, anchored: &[usize], dt: f32) -> Vec<usize> {
            self.frame_checks(state, anchored, dt, false)
        }

        fn frame_checks(&mut self, state: &State, anchored: &[usize], dt: f32, camera_moved: bool) -> Vec<usize> {
            self.frame += 1;
            let agent = self.world.agent_id;
            let mut touched = Vec::new();
            self.world.objects.take_touched(&mut touched);
            for idx in touched {
                self.sets.touched(&self.world.objects, agent, idx, state.rigged(idx));
            }
            let n = self.world.objects.slots.len();
            let checks = LodChecks::Events {
                anchored,
                dt,
                camera_moved,
            };
            let list = self.sets.frame_list(&self.world.objects, agent, self.frame, n, state, checks);
            let mut out: Vec<usize> = list.items.iter().map(|i| i.0).collect();
            for &idx in &out {
                if let Some(o) = self.world.objects.get_mut_untracked(idx) {
                    o.render.needs_records = false;
                    o.shape_dirty = false;
                    o.material_dirty = false;
                }
            }
            out.sort_unstable();
            out
        }

        /// Frames until the LOD slice would list `idx` are skipped: the
        /// next frame whose slice does not contain any of `objs`.
        fn quiet_frame(&mut self, objs: &[usize], state: &State) -> Vec<usize> {
            while objs.iter().any(|&i| (self.frame + 1 + i as u32).is_multiple_of(LOD_PERIOD)) {
                self.frame += 1;
            }
            self.frame(state)
        }
    }

    #[test]
    fn index_set_insert_remove() {
        let mut s = IndexSet::default();
        assert!(s.insert(5) && s.insert(2) && s.insert(9));
        assert!(!s.insert(2));
        assert!(s.remove(5));
        assert!(!s.remove(5));
        assert!(s.contains(2) && s.contains(9) && !s.contains(5));
        let mut v = s.take();
        v.sort_unstable();
        assert_eq!(v, vec![2, 9]);
        assert_eq!(s.len(), 0);
        assert!(!s.contains(2));
        assert!(s.insert(2));
    }

    #[test]
    fn new_objects_are_synced_once_then_left_alone() {
        let mut f = Fixture::new();
        let state = State::default();
        let root = f.add(9000, 0, false);
        let child = f.add(9001, 9000, false);
        assert_eq!(f.frame(&state), vec![root, child]);
        assert_eq!(f.quiet_frame(&[root, child], &state), Vec::<usize>::new());
    }

    #[test]
    fn motion_start_and_stop() {
        let mut f = Fixture::new();
        let state = State::default();
        let root = f.add(9000, 0, false);
        let child = f.add(9001, 9000, false);
        let other = f.add(9002, 0, false);
        f.frame(&state);
        // the root starts spinning: it and its child are placed every frame
        f.update(root, |u| u.angular_velocity = Vec3::Z);
        assert_eq!(f.frame(&state), vec![root, child]);
        assert_eq!(f.quiet_frame(&[root, child, other], &state), vec![root, child]);
        assert!(f.sets.own_moving.contains(root));
        // and stops: synced once more (the update), then left alone
        f.update(root, |u| u.angular_velocity = Vec3::ZERO);
        assert_eq!(f.frame(&state), vec![root, child]);
        assert_eq!(f.quiet_frame(&[root, child, other], &state), Vec::<usize>::new());
        assert!(!f.sets.own_moving.contains(root));
    }

    #[test]
    fn flexible_prims_update_without_network_motion_and_have_unique_geometry() {
        let mut f = Fixture::new();
        let state = State::default();
        let a = f.add(9000, 0, false);
        let b = f.add(9001, 0, false);
        let rigid = f.add(9002, 0, false);
        for idx in [a, b] {
            f.update(idx, |u| {
                u.volume.path.curve_type = aurora_prim::params::LL_PCODE_PATH_FLEXIBLE;
                u.extra.flexible = Some(crate::build::edits::FLEX_DEFAULT);
            });
        }
        f.frame(&state);
        assert_eq!(f.quiet_frame(&[a, b, rigid], &state), vec![a, b]);
        let ka = crate::scene::Scene::object_geom_key(f.world.objects.get(a).unwrap(), 3);
        let kb = crate::scene::Scene::object_geom_key(f.world.objects.get(b).unwrap(), 3);
        assert_ne!(ka, kb, "same shape never shares its simulated vertices");
        f.update(a, |u| {
            u.extra.flexible = None;
            u.volume.path.curve_type = aurora_prim::params::LL_PCODE_PATH_LINE;
        });
        f.frame(&state);
        assert_eq!(f.quiet_frame(&[a, b, rigid], &state), vec![b]);
    }

    #[test]
    fn a_moving_child_does_not_move_its_root() {
        let mut f = Fixture::new();
        let state = State::default();
        let root = f.add(9000, 0, false);
        let child = f.add(9001, 9000, false);
        f.frame(&state);
        f.update(child, |u| u.velocity = Vec3::X);
        f.frame(&state);
        assert_eq!(f.quiet_frame(&[root, child], &state), vec![child]);
    }

    #[test]
    fn updated_root_brings_its_linkset() {
        let mut f = Fixture::new();
        let state = State::default();
        let root = f.add(9000, 0, false);
        let child = f.add(9001, 9000, false);
        let grandchild = f.add(9002, 9001, false);
        f.frame(&state);
        // terse update of the root: every prim of the linkset is placed again
        if let Some(o) = f.world.objects.get_mut(root) {
            o.render.needs_records = true;
        }
        assert_eq!(f.quiet_frame(&[root, child, grandchild], &state), vec![root, child, grandchild]);
        // a texture change of the child alone: only the child
        if let Some(o) = f.world.objects.get_mut(child) {
            o.material_dirty = true;
        }
        assert_eq!(f.quiet_frame(&[root, child, grandchild], &state), vec![child]);
    }

    #[test]
    fn attachments_follow_their_avatar_and_stop_when_detached() {
        let mut f = Fixture::new();
        let state = State::default();
        let avatar = f.add(9000, 0, true);
        let still = f.add(9010, 0, false);
        f.frame(&state);
        // attach a linkset of two prims
        let att = f.add(9001, 9000, false);
        let att_child = f.add(9002, 9001, false);
        f.frame(&state);
        assert_eq!(
            f.quiet_frame(&[avatar, still, att, att_child], &state),
            vec![avatar, att, att_child]
        );
        // detached (dropped on the ground): it no longer follows the avatar
        f.update(att, |u| u.parent_id = 0);
        f.frame(&state);
        assert_eq!(f.quiet_frame(&[avatar, still, att, att_child], &state), vec![avatar]);
        // and attached again
        f.update(att, |u| u.parent_id = 9000);
        f.frame(&state);
        assert_eq!(
            f.quiet_frame(&[avatar, still, att, att_child], &state),
            vec![avatar, att, att_child]
        );
        // the avatar leaves: it and its attachments are removed
        f.world.objects.remove_idx(avatar);
        let mut removed = std::mem::take(&mut f.world.objects.removed_slots);
        removed.sort_unstable();
        assert_eq!(removed, vec![avatar, att, att_child]);
        for idx in removed {
            f.sets.forget(idx);
        }
        assert_eq!(f.quiet_frame(&[still], &state), Vec::<usize>::new());
    }

    /// What the GPU places from its wearer's attachment point is not
    /// visited while only the wearer moves; it is when it changes itself,
    /// or an object between it and the point.
    #[test]
    fn anchored_attachments_are_left_to_the_gpu() {
        let mut f = Fixture::new();
        let mut state = State::default();
        let avatar = f.add(9000, 0, true);
        let att = f.add(9001, 9000, false);
        let att_child = f.add(9002, 9001, false);
        let hud = f.add(9003, 9000, false);
        let all = [avatar, att, att_child, hud];
        f.frame(&state);
        assert_eq!(f.quiet_frame(&all, &state), vec![avatar, att, att_child, hud]);
        // the sync anchored the linkset (not the HUD)
        state.anchored.extend([att, att_child]);
        assert_eq!(f.quiet_frame(&all, &state), vec![avatar, hud]);
        // the avatar is updated (it walks): what it wears stays put
        f.update(avatar, |u| u.velocity = Vec3::X);
        assert_eq!(f.frame(&state), vec![avatar, hud]);
        // the attachment is moved on its point: its prims are placed again
        if let Some(o) = f.world.objects.get_mut(att) {
            o.render.needs_records = true;
        }
        assert_eq!(f.quiet_frame(&all, &state), vec![avatar, att, att_child, hud]);
        assert_eq!(f.quiet_frame(&all, &state), vec![avatar, hud]);
        // a prim alone changes
        if let Some(o) = f.world.objects.get_mut(att_child) {
            o.material_dirty = true;
        }
        assert_eq!(f.quiet_frame(&all, &state), vec![avatar, att_child, hud]);
        // the detail level check still comes round
        while !(f.frame + 1 + att_child as u32).is_multiple_of(LOD_PERIOD) {
            f.frame(&state);
        }
        assert!(f.frame(&state).contains(&att_child));
        // the attachment starts spinning: the sync takes the linkset back
        f.update(att, |u| u.angular_velocity = Vec3::Z);
        assert_eq!(f.frame(&state), vec![avatar, att, att_child, hud]);
        state.anchored.clear();
        assert_eq!(f.quiet_frame(&all, &state), vec![avatar, att, att_child, hud]);
    }

    /// A report that moves a prim of a worn linkset fades in, the prim
    /// placed by the sync at each frame while its root stays on its anchor;
    /// once faded, the prim is placed once more, back on the anchor. Nothing
    /// else visits it (its wearer's followers stop at the anchored root):
    /// left in the world where the last fading frame put it, it trailed
    /// behind its walking wearer until the next report.
    #[test]
    fn a_faded_correction_puts_a_worn_prim_back_on_its_anchor() {
        let mut f = Fixture::new();
        let mut state = State::default();
        let avatar = f.add(9000, 0, true);
        let att = f.add(9001, 9000, false);
        let att_child = f.add(9002, 9001, false);
        let all = [avatar, att, att_child];
        f.frame(&state);
        state.anchored.extend([att, att_child]);
        assert_eq!(f.quiet_frame(&all, &state), vec![avatar]);
        // a script moves the prim: the gap fades in, placed by the sync
        let at = f.world.objects.get(att_child).expect("worn prim").position;
        f.update(att_child, |u| u.position = at + Vec3::X * 0.05);
        assert!(f.world.objects.get(att_child).expect("worn prim").has_motion());
        state.anchored.remove(&att_child);
        assert_eq!(f.frame(&state), vec![avatar, att_child]);
        assert_eq!(f.quiet_frame(&all, &state), vec![avatar, att_child]);
        // faded: placed once more, back on its anchor, then left to the GPU
        f.world
            .objects
            .expire_corrections(std::time::Instant::now() + std::time::Duration::from_secs(1));
        assert!(!f.world.objects.get(att_child).expect("worn prim").has_motion());
        assert_eq!(f.quiet_frame(&all, &state), vec![avatar, att_child]);
        state.anchored.insert(att_child);
        assert_eq!(f.quiet_frame(&all, &state), vec![avatar]);
    }

    /// A rigged root moves by itself and its plain prims are anchored: an
    /// update of the root still reaches them, though the root was already
    /// listed with its followers.
    #[test]
    fn an_updated_moving_root_reaches_its_anchored_prims() {
        let mut f = Fixture::new();
        let mut state = State::default();
        let avatar = f.add(9000, 0, true);
        let mesh = f.add(9001, 9000, false);
        let prim = f.add(9002, 9001, false);
        let all = [avatar, mesh, prim];
        f.frame(&state);
        state.rigged.insert(mesh);
        f.sets.synced(&f.world.objects, f.world.agent_id, mesh, true);
        state.anchored.insert(prim);
        assert_eq!(f.quiet_frame(&all, &state), vec![avatar, mesh]);
        if let Some(o) = f.world.objects.get_mut(mesh) {
            o.render.needs_records = true;
        }
        assert_eq!(f.quiet_frame(&all, &state), vec![avatar, mesh, prim]);
        assert_eq!(f.quiet_frame(&all, &state), vec![avatar, mesh]);
    }

    /// A rigged mesh the GPU places stops being visited at each frame,
    /// with the prims anchored below it; one the sync places is back.
    #[test]
    fn rigged_meshes_placed_by_the_gpu_are_not_visited() {
        let mut f = Fixture::new();
        let mut state = State::default();
        let avatar = f.add(9000, 0, true);
        let mesh = f.add(9001, 9000, false);
        let prim = f.add(9002, 9001, false);
        let all = [avatar, mesh, prim];
        f.frame(&state);
        // found rigged, placed by the sync (no palette yet, a HUD...)
        state.rigged.insert(mesh);
        f.sets.synced(&f.world.objects, f.world.agent_id, mesh, true);
        assert_eq!(f.quiet_frame(&all, &state), vec![avatar, mesh, prim]);
        // then by the GPU, with its prim
        state.rigged.remove(&mesh);
        state.anchored.extend([mesh, prim]);
        assert_eq!(f.quiet_frame(&all, &state), vec![avatar]);
        assert!(!f.sets.own_moving.contains(mesh));
        // its texture changes: it alone
        if let Some(o) = f.world.objects.get_mut(mesh) {
            o.material_dirty = true;
        }
        assert_eq!(f.quiet_frame(&all, &state), vec![avatar, mesh]);
        // the sync takes it back (its palette changed place): the detail
        // level check finds it moving again
        state.anchored.clear();
        state.rigged.insert(mesh);
        while !(f.frame + 1 + mesh as u32).is_multiple_of(LOD_PERIOD) {
            f.frame(&state);
        }
        f.frame(&state);
        assert!(f.sets.own_moving.contains(mesh));
        assert_eq!(f.quiet_frame(&all, &state), vec![avatar, mesh, prim]);
    }

    #[test]
    fn link_and_unlink_follow_a_moving_root() {
        let mut f = Fixture::new();
        let state = State::default();
        let root = f.add(9000, 0, false);
        let loose = f.add(9001, 0, false);
        f.frame(&state);
        f.update(root, |u| u.velocity = Vec3::X);
        f.frame(&state);
        assert_eq!(f.quiet_frame(&[root, loose], &state), vec![root]);
        // linked to the moving root
        f.update(loose, |u| u.parent_id = 9000);
        f.frame(&state);
        assert_eq!(f.quiet_frame(&[root, loose], &state), vec![root, loose]);
        // unlinked again
        f.update(loose, |u| u.parent_id = 0);
        f.frame(&state);
        assert_eq!(f.quiet_frame(&[root, loose], &state), vec![root]);
    }

    #[test]
    fn rigged_meshes_move_and_unfinished_syncs_are_retried() {
        let mut f = Fixture::new();
        let mut state = State::default();
        let mesh = f.add(9000, 0, false);
        let orphan = f.add(9001, 9999, false);
        f.frame(&state);
        // found rigged by its sync: placed every frame
        state.rigged.insert(mesh);
        f.sets.synced(&f.world.objects, f.world.agent_id, mesh, true);
        assert_eq!(f.quiet_frame(&[mesh, orphan], &state), vec![mesh]);
        // a sync that could not finish (parent unknown) stays dirty
        state.extra.insert(orphan);
        f.sets.dirty.insert(orphan);
        assert_eq!(f.quiet_frame(&[mesh, orphan], &state), vec![mesh, orphan]);
    }

    #[test]
    fn pending_geometry_is_visited_when_it_can_progress() {
        let mut f = Fixture::new();
        let mut state = State::default();
        let a = f.add(9000, 0, false);
        let b = f.add(9001, 0, false);
        f.frame(&state);
        f.sets.pending.insert(a);
        f.sets.pending.insert(b);
        state.pending = vec![(a, false), (b, true)];
        assert_eq!(f.quiet_frame(&[a, b], &state), vec![b]);
        // a resolved: dropped from the set
        state.pending = vec![(b, false)];
        assert_eq!(f.quiet_frame(&[a, b], &state), Vec::<usize>::new());
        assert!(!f.sets.pending.contains(a) && f.sets.pending.contains(b));
    }

    #[test]
    fn many_pending_objects_are_asked_about_over_a_few_frames() {
        assert_eq!(pending_checks(0), 0);
        assert_eq!(pending_checks(100), 100);
        assert_eq!(pending_checks(PENDING_PER_FRAME), PENDING_PER_FRAME);
        assert_eq!(pending_checks(PENDING_PER_FRAME + 500), PENDING_PER_FRAME);
        assert_eq!(pending_checks(12_000), 3000);

        let mut f = Fixture::new();
        let n = PENDING_PER_FRAME * 2 + 500;
        let objs: Vec<usize> = (0..n).map(|i| f.add(10_000 + i as u32, 0, false)).collect();
        let mut state = State {
            pending_else: Some(false),
            ..State::default()
        };
        f.frame(&state);
        for &i in &objs {
            f.sets.pending.insert(i);
        }
        // nothing can progress: a part of them is asked about each frame
        state.asked.set(0);
        f.frame(&state);
        assert_eq!(state.asked.get(), PENDING_PER_FRAME);
        // one becomes ready, wherever the round is: listed within the round
        let ready = objs[n - 7];
        state.pending = vec![(ready, true)];
        let listed = (0..n.div_ceil(PENDING_PER_FRAME)).any(|_| f.quiet_frame(&[ready], &state).contains(&ready));
        assert!(listed);
        // resolved ones leave the set as the round meets them
        state.pending.clear();
        state.pending_else = None;
        for _ in 0..n.div_ceil(PENDING_PER_FRAME) {
            f.frame(&state);
        }
        assert_eq!(f.sets.pending.len(), 0);
    }

    #[test]
    fn objects_waiting_for_a_geometry_in_progress_are_parked_until_it_changes() {
        let mut f = Fixture::new();
        let mut state = State::default();
        let a = f.add(9000, 0, false);
        let b = f.add(9001, 0, false);
        let c = f.add(9002, 0, false);
        f.frame_events(&state, &[], 0.0);
        for i in [a, b, c] {
            f.sets.pending.insert(i);
        }
        // a and c wait for geometries being built, b for its sculpt map
        state.waits = vec![(a, 7), (c, 7)];
        state.pending = vec![(b, false)];
        assert!(f.frame_events(&state, &[], 0.0).is_empty());
        assert_eq!(f.sets.parked_len(), 2);
        assert!(!f.sets.pending.contains(a) && f.sets.pending.contains(b));
        // parked: no longer asked about at each frame
        state.asked.set(0);
        f.frame_events(&state, &[], 0.0);
        assert_eq!(state.asked.get(), 1);
        // another geometry changed: still parked
        f.sets.geometry_changed(8);
        assert_eq!(f.sets.parked_len(), 2);
        // theirs is ready: back in the round, synced
        f.sets.geometry_changed(7);
        state.waits.clear();
        state.pending = vec![(a, true), (b, false), (c, true)];
        assert_eq!(f.frame_events(&state, &[], 0.0), vec![a, c]);
        assert_eq!(f.sets.parked_len(), 0);
        f.sets.geometry_changed(7);

        // a full sync puts a parked object back in the round (or out of it)
        state.waits = vec![(a, 9), (c, 9)];
        f.frame_events(&state, &[], 0.0);
        assert_eq!(f.sets.parked_len(), 2);
        f.sets.geometry_wanted(a, true);
        f.sets.geometry_wanted(c, false);
        assert!(f.sets.pending.contains(a) && !f.sets.pending.contains(c));
        assert_eq!(f.sets.parked_len(), 0);
        // parked again for the same geometry: listed once for it
        f.frame_events(&state, &[], 0.0);
        assert_eq!(f.sets.parked_by[&9], vec![a, c]);
        // removed from the store: out of every set, its wake is skipped
        f.sets.forget(a);
        f.sets.geometry_changed(9);
        assert!(!f.sets.pending.contains(a) && !f.sets.parked.contains(a));
        // with the slice (the way before), nothing stays parked
        state.waits = vec![(b, 3)];
        f.frame_events(&state, &[], 0.0);
        assert!(f.sets.parked.contains(b));
        f.frame(&state);
        assert!(f.sets.pending.contains(b) && f.sets.parked_len() == 0);
    }

    #[test]
    fn parked_objects_come_back_to_the_round_in_case() {
        let mut f = Fixture::new();
        let objs: Vec<usize> = (0..48).map(|i| f.add(10_000 + i, 0, false)).collect();
        let mut state = State {
            waits: objs.iter().map(|&i| (i, 5)).collect(),
            ..State::default()
        };
        f.frame_events(&state, &[], 0.0);
        for &i in &objs {
            f.sets.pending.insert(i);
        }
        f.frame_events(&state, &[], 0.0);
        assert_eq!(f.sets.parked_len(), objs.len());
        // its geometry changed without a word: met again within the period
        let lost = objs[30];
        state.waits.retain(|w| w.0 != lost);
        state.pending = vec![(lost, true)];
        let dt = 1.0 / 60.0;
        let frames = (PARKED_SECONDS / dt).ceil() as usize + 1;
        state.asked.set(0);
        let mut listed = false;
        for _ in 0..frames {
            listed |= f.frame_events(&state, &[], dt).contains(&lost);
        }
        assert!(listed);
        // the others, asked about again too, stay parked
        assert!(state.asked.get() >= objs.len());
        assert_eq!(f.sets.parked_len(), objs.len() - 1);
    }

    #[test]
    fn full_sync_budget_counts_only_its_own_time() {
        let us = std::time::Duration::from_micros;
        let mut b = FullSyncBudget::new(us(1000));
        let mut done = 0;
        while b.allows() {
            b.spend(us(300));
            done += 1;
        }
        // the one that crosses the budget is the last
        assert_eq!(done, 4);
        // no time at all (a frame share of zero): the backlog still advances
        let mut b = FullSyncBudget::new(us(0));
        let mut done = 0;
        while b.allows() {
            b.spend(us(5000));
            done += 1;
        }
        assert_eq!(done, 1);
        // cheap syncs: as many as fit
        let mut b = FullSyncBudget::new(us(1000));
        let mut done = 0;
        while b.allows() {
            b.spend(us(1));
            done += 1;
        }
        assert_eq!(done, 1000);
    }

    #[test]
    fn backlog_serves_the_nearest_first() {
        let mut b = Backlog::default();
        assert!(b.is_empty() && b.pop().is_none());
        assert!(b.push(10, 40.0));
        assert!(b.push(11, 2.5));
        assert!(b.push(12, 150.0));
        assert!(b.push(13, 2.5));
        // already waiting: it keeps its place, whatever the new distance
        assert!(!b.push(12, 1.0));
        assert_eq!(b.len(), 4);
        assert!(b.contains(11) && !b.contains(99));
        // equal distances: the lowest index
        let order: Vec<usize> = std::iter::from_fn(|| b.pop()).collect();
        assert_eq!(order, [11, 13, 10, 12]);
        assert!(b.is_empty() && !b.contains(11));
        // a distance that is not a number counts as nearest; huge ones saturate
        b.push(1, f32::NAN);
        b.push(2, 1e30);
        b.push(3, -5.0);
        b.push(4, 0.2);
        let order: Vec<usize> = std::iter::from_fn(|| b.pop()).collect();
        assert_eq!(order, [1, 3, 4, 2]);
    }

    #[test]
    fn backlog_forgets_removed_objects() {
        let mut b = Backlog::default();
        b.push(5, 10.0);
        b.push(6, 20.0);
        b.push(7, 30.0);
        // synced another way, or its object removed
        b.remove(6);
        b.remove(6);
        b.remove(1234);
        assert_eq!(b.len(), 2);
        // the slot taken again by a new object, farther away: its old place
        // is not served twice
        b.remove(5);
        b.push(5, 90.0);
        assert_eq!(b.len(), 2);
        let order: Vec<usize> = std::iter::from_fn(|| b.pop()).collect();
        assert_eq!(order, [5, 7]);
        assert!(b.is_empty());
        // emptied by removals alone: nothing left to skip
        b.push(8, 1.0);
        b.remove(8);
        assert!(b.heap.is_empty() && b.pop().is_none());
    }

    #[test]
    fn arrivals_are_served_in_arrival_order() {
        let mut a = Arrivals::default();
        assert!(a.pop().is_none() && a.is_empty());
        assert!(a.push(12));
        assert!(a.push(3));
        assert!(a.push(40));
        // already waiting: it keeps its place
        assert!(!a.push(3));
        assert_eq!(a.len(), 3);
        assert!(a.contains(40) && !a.contains(4) && !a.contains(999));
        // removed while it waits, twice, and one that never waited
        a.remove(3);
        a.remove(3);
        a.remove(999);
        assert_eq!(a.len(), 2);
        // its slot taken again by a new object: served once, at the place
        // of the first entry met
        a.push(3);
        let order: Vec<usize> = std::iter::from_fn(|| a.pop()).collect();
        assert_eq!(order, [12, 3, 40]);
        assert!(a.is_empty() && !a.contains(12));
        // emptied by removals alone: nothing left to skip
        a.push(8);
        a.remove(8);
        assert!(a.queue.is_empty() && a.pop().is_none());
    }

    #[test]
    fn arrivals_wait_out_of_the_frame_list() {
        let mut f = Fixture::new();
        let state = State::default();
        let objs: Vec<usize> = (0..50).map(|i| f.add(9000 + i, 0, false)).collect();
        f.frame(&state);
        // a new object waiting for its first sync: the LOD slice passes it
        let waiting = objs[7];
        f.sets.arrivals.push(waiting);
        let mut seen = vec![0u32; objs.len()];
        for _ in 0..LOD_PERIOD {
            for idx in f.frame(&state) {
                seen[idx] += 1;
            }
        }
        assert_eq!(seen[waiting], 0);
        assert!(objs.iter().all(|&i| i == waiting || seen[i] == 1), "{seen:?}");
        // served: visited like the others from then on
        assert_eq!(f.sets.arrivals.pop(), Some(waiting));
        let visits: usize = (0..LOD_PERIOD)
            .map(|_| f.frame(&state).iter().filter(|&&i| i == waiting).count())
            .sum();
        assert_eq!(visits, 1);
        // a removed object leaves the arrivals with the other sets
        f.sets.arrivals.push(waiting);
        f.sets.forget(waiting);
        assert!(f.sets.arrivals.is_empty() && f.sets.arrivals.pop().is_none());
    }

    #[test]
    fn full_sync_budget_keeps_a_reserve() {
        let us = std::time::Duration::from_micros;
        // the first half of the budget, then what is left of it
        let mut b = FullSyncBudget::new(us(1000));
        let reserve = b.left() / 2;
        let mut first = 0;
        while b.allows_beyond(reserve) {
            b.spend(us(100));
            first += 1;
        }
        assert_eq!((first, b.left()), (5, us(500)));
        let mut rest = 0;
        while b.allows() {
            b.spend(us(100));
            rest += 1;
        }
        assert_eq!(rest, 5);
        // no time at all: the first of the frame is still done
        let b = FullSyncBudget::new(us(0));
        assert!(b.allows_beyond(us(0)) && b.allows_beyond(us(400)));
    }

    #[test]
    fn objects_waiting_in_the_backlog_are_not_listed_for_their_geometry() {
        let mut f = Fixture::new();
        let mut state = State::default();
        let a = f.add(9000, 0, false);
        let b = f.add(9001, 0, false);
        f.frame(&state);
        // both have their geometry ready, but the frame had no time for a
        f.sets.pending.insert(a);
        f.sets.pending.insert(b);
        state.pending = vec![(a, true), (b, true)];
        f.sets.backlog.push(a, 12.0);
        assert_eq!(f.quiet_frame(&[a, b], &state), vec![b]);
        // served from the backlog: listed again if it still waits
        assert_eq!(f.sets.backlog.pop(), Some(a));
        assert_eq!(f.quiet_frame(&[a, b], &state), vec![a, b]);
        // a removed object leaves the backlog with the other sets
        f.sets.backlog.push(b, 3.0);
        f.sets.forget(b);
        assert!(f.sets.backlog.is_empty() && !f.sets.pending.contains(b));
    }

    #[test]
    fn lod_slice_visits_each_object_once_per_period() {
        let mut f = Fixture::new();
        let state = State::default();
        let objs: Vec<usize> = (0..50).map(|i| f.add(9000 + i, 0, false)).collect();
        f.frame(&state);
        let mut seen = vec![0u32; objs.len()];
        for _ in 0..LOD_PERIOD {
            for idx in f.frame(&state) {
                seen[idx] += 1;
            }
        }
        assert!(seen.iter().all(|&s| s == 1), "{seen:?}");
    }

    #[test]
    fn rounds_visit_each_object_once_per_period_whatever_the_frame_rate() {
        // 60 frames per second: the slice's share; faster: less per frame
        for (dt, frames) in [(1.0 / 60.0, 24), (1.0 / 160.0, 64), (1.0 / 240.0, 96)] {
            let mut credit = 0.0;
            let total: usize = (0..frames).map(|_| round_share(&mut credit, 1200, dt, LOD_SECONDS)).sum();
            assert!((1199..=1201).contains(&total), "{total} in {frames} frames");
        }
        // slower, or after a stall: never more than a slice of them
        let mut credit = 0.0;
        for dt in [1.0 / 20.0, 0.5, 30.0, f32::NAN] {
            assert!(round_share(&mut credit, 1200, dt, LOD_SECONDS) <= 50);
        }
        assert_eq!(round_share(&mut credit, 0, 0.1, LOD_SECONDS), 0);
        // a few objects: one now and then
        let mut credit = 0.0;
        let total: usize = (0..600).map(|_| round_share(&mut credit, 3, 1.0 / 60.0, SWEEP_SECONDS)).sum();
        assert!((2..=3).contains(&total), "{total}");
    }

    /// Checks by events: objects at rest are listed when the camera has
    /// travelled their slack, a slice of them at most per frame.
    #[test]
    fn objects_at_rest_are_checked_when_the_camera_has_moved() {
        let mut f = Fixture::new();
        let state = State::default();
        let objs: Vec<usize> = (0..100).map(|i| f.add(9000 + i, 0, false)).collect();
        f.sets.lod.advance(Vec3::ZERO, 2.0, 1.0);
        assert_eq!(f.frame_events(&state, &[], 0.0), objs);
        for &idx in &objs {
            f.sets.lod.watch(idx, if idx % 2 == 0 { 1.0 } else { 30.0 });
        }
        // the camera at rest: nobody, frame after frame
        for _ in 0..3 * LOD_PERIOD {
            f.sets.lod.advance(Vec3::ZERO, 2.0, 1.0);
            assert_eq!(f.frame_events(&state, &[], 0.0), Vec::<usize>::new());
        }
        // 2 m: those 1 m from a change of level, a slice of the slab per frame
        f.sets.lod.advance(Vec3::X * 2.0, 2.0, 1.0);
        let n = f.world.objects.slots.len();
        let most = n.div_ceil(LOD_PERIOD as usize);
        let mut seen = Vec::new();
        for _ in 0..LOD_PERIOD {
            let listed = f.frame_events(&state, &[], 0.0);
            assert!(listed.len() <= most, "{} listed for a slice of {most}", listed.len());
            seen.extend(listed);
        }
        seen.sort_unstable();
        assert_eq!(seen, objs.iter().copied().filter(|i| i % 2 == 0).collect::<Vec<_>>());
        // (their checks watch them again; until then they are not listed)
        assert_eq!(f.frame_events(&state, &[], 0.0), Vec::<usize>::new());
        // one waits for its first sync: its turn does not list it
        f.sets.arrivals.push(objs[1]);
        f.sets.lod.advance(Vec3::X * 40.0, 2.0, 1.0);
        let mut seen = Vec::new();
        for _ in 0..LOD_PERIOD {
            seen.extend(f.frame_events(&state, &[], 0.0));
        }
        seen.sort_unstable();
        assert_eq!(
            seen,
            objs.iter().copied().filter(|&i| i % 2 == 1 && i != objs[1]).collect::<Vec<_>>()
        );
        // an updated object is listed by its update, whatever its wait
        f.sets.lod.watch(objs[4], 500.0);
        f.update(objs[4], |u| u.position += Vec3::X);
        assert_eq!(f.frame_events(&state, &[], 0.0), vec![objs[4]]);
        assert!(!f.sets.only_checked(objs[4]));
        // a removed one leaves the watch with the other sets
        f.sets.forget(objs[4]);
        assert!(!f.sets.lod.is_watched(objs[4]));
    }

    /// Checks by events: what the GPU places from an avatar is visited in
    /// turn every `LOD_SECONDS`, and every object every `SWEEP_SECONDS`.
    #[test]
    fn anchored_objects_and_the_sweep_come_round_by_time() {
        let mut f = Fixture::new();
        let mut state = State::default();
        let avatar = f.add(9000, 0, true);
        let worn: Vec<usize> = (0..60).map(|i| f.add(9001 + i, 9000, false)).collect();
        let still = f.add(9900, 0, false);
        f.sets.lod.advance(Vec3::ZERO, 2.0, 1.0);
        f.frame_events(&state, &[], 0.0);
        state.anchored.extend(worn.iter().copied());
        f.sets.lod.watch(still, 100.0);
        let n = f.world.objects.slots.len();
        // 240 frames per second: one round of the worn objects in 0.4 s
        let dt = 1.0 / 240.0;
        let frames = (LOD_SECONDS / dt).round() as usize;
        let mut seen = vec![0u32; n];
        let mut most = 0;
        for _ in 0..frames {
            let listed = f.frame_events(&state, &worn, dt);
            assert!(listed.contains(&avatar));
            most = most.max(listed.len() - 1);
            for idx in listed {
                seen[idx] += 1;
                assert!(idx == avatar || f.sets.only_checked(idx));
            }
        }
        // (the sweep adds a visit to a few of them)
        assert!(worn.iter().all(|&i| (1..=2).contains(&seen[i])), "{seen:?}");
        assert!(most <= 2, "{most} checks in a frame");
        // the object at rest: by the sweep alone, once in ten seconds
        let mut seen = vec![0u32; n];
        for _ in 0..(SWEEP_SECONDS / dt).round() as usize {
            for idx in f.frame_events(&state, &worn, dt) {
                seen[idx] += 1;
            }
        }
        assert!((1..=2).contains(&seen[still]), "{}", seen[still]);
        assert!(seen[worn[7]] >= 24);
        // at 30 frames per second: not more per frame than the slice gave
        let mut seen = vec![0u32; n];
        for _ in 0..LOD_PERIOD {
            let listed = f.frame_events(&state, &worn, 1.0 / 30.0);
            assert!(listed.len() - 1 <= 2 * n.div_ceil(LOD_PERIOD as usize));
            for idx in listed {
                seen[idx] += 1;
            }
        }
        assert!(worn.iter().all(|&i| seen[i] >= 1), "{seen:?}");
        // while the camera moves: every 24 frames whatever the frame rate,
        // as by the slice
        let mut seen = vec![0u32; n];
        for _ in 0..LOD_PERIOD {
            for idx in f.frame_checks(&state, &worn, dt, true) {
                seen[idx] += 1;
            }
        }
        assert!(worn.iter().all(|&i| (1..=2).contains(&seen[i])), "{seen:?}");
    }

    /// A HUD attachment stays where it is on the screen: it does not
    /// follow its wearer through the sync. What moves by itself on it
    /// still brings what is linked below, and an update brings the HUD.
    #[test]
    fn hud_attachments_do_not_follow_their_wearer() {
        let mut f = Fixture::new();
        let mut state = State::default();
        let avatar = f.add(9000, 0, true);
        let hud = f.add(9001, 9000, false);
        let button = f.add(9002, 9001, false);
        let needle = f.add(9003, 9002, false);
        let hat = f.add(9010, 9000, false);
        let all = [avatar, hud, button, needle, hat];
        f.frame(&state);
        // the slice way (AURORA_SYNC_SLICE): with the wearer at each frame
        assert_eq!(f.quiet_frame(&all, &state), all.to_vec());
        // found on a HUD point by their sync
        state.hud.extend([hud, button, needle]);
        assert_eq!(f.quiet_frame(&all, &state), vec![avatar, hat]);
        // the wearer walks: still not
        f.update(avatar, |u| u.velocity = Vec3::X);
        assert_eq!(f.frame(&state), vec![avatar, hat]);
        // the HUD is moved on the screen: placed again with its prims, once
        if let Some(o) = f.world.objects.get_mut(hud) {
            o.render.needs_records = true;
        }
        assert_eq!(f.quiet_frame(&all, &state), all.to_vec());
        assert_eq!(f.quiet_frame(&all, &state), vec![avatar, hat]);
        // a button starts spinning: it and what is linked below it, at
        // each frame; the rest of the HUD stays
        f.update(button, |u| u.angular_velocity = Vec3::Z);
        f.frame(&state);
        assert_eq!(f.quiet_frame(&all, &state), vec![avatar, button, needle, hat]);
    }

    #[test]
    fn frame_list_levels_put_parents_first() {
        let mut f = Fixture::new();
        let state = State::default();
        let avatar = f.add(9000, 0, true);
        let att = f.add(9001, 9000, false);
        let prim = f.add(9002, 9001, false);
        f.frame(&state);
        f.frame += 1;
        let n = f.world.objects.slots.len();
        let list = f
            .sets
            .frame_list(&f.world.objects, f.world.agent_id, f.frame, n, &state, LodChecks::Slice);
        let levels = list.levels();
        assert_eq!(levels.len(), 3);
        assert!(levels[0].contains(&avatar) && levels[1] == vec![att] && levels[2] == vec![prim]);
    }
}
