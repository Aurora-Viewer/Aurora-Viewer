//! Which objects the scene sync visits each frame, kept up to date from
//! events instead of a pass over every object.
//!
//! - **Moving**: objects that move by themselves (velocity, acceleration or
//!   spin, avatars, our agent, rigged meshes) and everything that follows
//!   them (linkset children, attachments and their prims). Only the objects
//!   moving by themselves are stored; the followers are found each frame
//!   through the store's child lists, so links, unlinks, attachments and
//!   detachments need no bookkeeping.
//! - **Dirty**: objects changed through the store (`ObjectStore::take_touched`),
//!   material users on a new material generation, and objects whose last
//!   sync could not finish. An object updated with `needs_records` brings
//!   its whole subtree along (its children are placed relative to it).
//! - **Pending**: objects whose wanted geometry is not bound yet; visited
//!   when that geometry can make progress (ready, or to be requested again).
//! - **LOD slice**: each object is visited every [`LOD_PERIOD`] frames
//!   (detail level, and a safety net for state changed behind the store's
//!   back). Only that frame's slice of the slab is walked.

use crate::world::objects::{Object, ObjectStore};
use glam::{Quat, Vec3};
use uuid::Uuid;

/// Frames between two detail-level checks of an object.
pub const LOD_PERIOD: u32 = 24;

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
    o.has_motion() || o.is_avatar() || o.full_id == agent || rigged
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
    /// Rigged mesh: deformed by its skeleton, placed again every frame.
    fn rigged(&self, idx: usize) -> bool;
    /// Must be synced: changed, or its last sync could not finish.
    fn needs_sync(&self, idx: usize, o: &Object) -> bool;
    /// Pending geometry: None when nothing is pending any more, else
    /// whether a sync can make progress now.
    fn pending_due(&self, idx: usize) -> Option<bool>;
}

/// The event-driven sets of the scene sync (see the module documentation).
#[derive(Default)]
pub struct SyncSets {
    /// Objects moving by themselves (validated each frame, removed lazily).
    pub own_moving: IndexSet,
    pub dirty: IndexSet,
    pub pending: IndexSet,
    /// Objects whose faces use materials (glTF or legacy).
    pub material_users: IndexSet,
    /// Material generation the material users were last checked against.
    pub seen_mat_gen: u64,
    listed: FrameMarks,
    expanded: FrameMarks,
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
        self.material_users.remove(idx);
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
    /// geometry that can progress and the frame's LOD slice. Each once.
    pub fn frame_list(&mut self, objects: &ObjectStore, agent: Uuid, frame: u32, n: usize, state: &impl SyncState) -> FrameList {
        self.listed.next_frame(n);
        self.expanded.next_frame(n);
        let mut list = FrameList::default();
        let mut i = 0;
        while i < self.own_moving.len() {
            let idx = self.own_moving.items[i];
            match objects.get(idx) {
                Some(o) if moves_by_itself(o, agent, state.rigged(idx)) => {
                    self.push_tree(objects, idx, &mut list);
                    i += 1;
                }
                // stopped, or removed: swap-removed, the next one takes its place
                _ => {
                    self.own_moving.remove(idx);
                }
            }
        }
        for idx in self.dirty.take() {
            let Some(o) = objects.get(idx) else {
                continue;
            };
            if !state.needs_sync(idx, o) {
                continue;
            }
            if o.render.needs_records {
                self.push_tree(objects, idx, &mut list);
            } else {
                self.push_one(objects, idx, &mut list);
            }
        }
        let mut i = 0;
        while i < self.pending.len() {
            let idx = self.pending.items[i];
            match state.pending_due(idx) {
                None => {
                    self.pending.remove(idx);
                    continue;
                }
                Some(true) => self.push_one(objects, idx, &mut list),
                Some(false) => {}
            }
            i += 1;
        }
        // LOD slice: (frame + idx) is a multiple of LOD_PERIOD
        let start = ((LOD_PERIOD - frame % LOD_PERIOD) % LOD_PERIOD) as usize;
        for idx in (start..n).step_by(LOD_PERIOD as usize) {
            let Some(o) = objects.get(idx) else {
                continue;
            };
            if moves_by_itself(o, agent, state.rigged(idx)) {
                self.own_moving.insert(idx);
            }
            self.push_one(objects, idx, &mut list);
        }
        list
    }

    fn push_one(&mut self, objects: &ObjectStore, idx: usize, list: &mut FrameList) {
        if self.listed.mark(idx) {
            list.items.push((idx, depth(objects, idx)));
        }
    }

    /// An object and everything below it (children, attachments, their prims).
    fn push_tree(&mut self, objects: &ObjectStore, root: usize, list: &mut FrameList) {
        if !self.expanded.mark(root) {
            return;
        }
        let mut stack = vec![(root, depth(objects, root))];
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
                // a subtree already listed whole (another moving object)
                if self.expanded.mark(c) {
                    stack.push((c, d + 1));
                }
            }
        }
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
        extra: HashSet<usize>,
        pending: Vec<(usize, bool)>,
    }

    impl SyncState for State {
        fn rigged(&self, idx: usize) -> bool {
            self.rigged.contains(&idx)
        }
        fn needs_sync(&self, idx: usize, o: &Object) -> bool {
            o.render.needs_records || o.shape_dirty || o.material_dirty || self.extra.contains(&idx)
        }
        fn pending_due(&self, idx: usize) -> Option<bool> {
            self.pending.iter().find(|p| p.0 == idx).map(|p| p.1)
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
            let list = self.sets.frame_list(&self.world.objects, agent, self.frame, n, state);
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
    fn frame_list_levels_put_parents_first() {
        let mut f = Fixture::new();
        let state = State::default();
        let avatar = f.add(9000, 0, true);
        let att = f.add(9001, 9000, false);
        let prim = f.add(9002, 9001, false);
        f.frame(&state);
        f.frame += 1;
        let n = f.world.objects.slots.len();
        let list = f.sets.frame_list(&f.world.objects, f.world.agent_id, f.frame, n, &state);
        let levels = list.levels();
        assert_eq!(levels.len(), 3);
        assert!(levels[0].contains(&avatar) && levels[1] == vec![att] && levels[2] == vec![prim]);
    }
}
