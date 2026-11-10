//! Carry client-only appearances over simulator-created copies. The duplicate
//! protocol has no source UUID in its reply, so match new prims by their server
//! shape, texture entry and placement, then by their new parent for children.

use super::*;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(15);
const MAX_PENDING: usize = 64;
const MAX_CANDIDATES: usize = 4096;

#[derive(Clone)]
struct Prim {
    pcode: u8,
    position: Vec3,
    rotation: Quat,
    scale: Vec3,
    volume: VolumeParams,
    extra: ExtraParams,
    te: Option<Arc<TextureEntry>>,
    appearance: HashMap<Option<u8>, LocalAppearance>,
}

impl Prim {
    fn capture(object: &Object) -> Self {
        Self {
            pcode: object.pcode,
            position: object.position,
            rotation: object.rotation,
            scale: object.scale,
            volume: object.volume,
            extra: object.extra.clone(),
            te: object.te.clone(),
            appearance: object.render.local_appearance.clone(),
        }
    }

    fn matches(&self, object: &Object, position: Option<(Vec3, f32)>, rotation: bool) -> bool {
        self.pcode == object.pcode
            && self.volume == object.volume
            && self.extra == object.extra
            && self.te == object.te
            && self.scale.abs_diff_eq(object.scale, 0.001)
            && position.is_none_or(|(point, tolerance)| point.distance(object.position) <= tolerance)
            && (!rotation || self.rotation.abs_diff_eq(object.rotation, 0.001) || self.rotation.abs_diff_eq(-object.rotation, 0.001))
    }
}

struct Copy {
    at: Instant,
    region: RegionHandle,
    root: Prim,
    children: Vec<Prim>,
    parent: Option<ObjKey>,
    position: (Vec3, f32),
    rotation: bool,
    selected: bool,
}

impl Copy {
    fn matches_root(&self, object: &Object) -> bool {
        self.parent.is_none()
            && object.key.region == self.region
            && object.parent_id == 0
            && (!self.selected || object.update_flags & crate::build::FLAGS_CREATE_SELECTED != 0)
            && self.root.matches(object, Some(self.position), self.rotation)
    }

    fn same_appearance(&self, other: &Self) -> bool {
        self.root.appearance == other.root.appearance
            && self.children.len() == other.children.len()
            && self
                .children
                .iter()
                .zip(&other.children)
                .all(|(a, b)| a.position == b.position && a.rotation == b.rotation && a.appearance == b.appearance)
    }
}

#[derive(Default)]
pub(super) struct LocalDuplicates {
    copies: Vec<Copy>,
    candidates: Vec<Uuid>,
    pub changes: Vec<(Uuid, Option<u8>, Option<LocalAppearance>)>,
}

impl ObjectStore {
    /// Called only for roots admitted by LLSelectMgr::canDuplicate's shared
    /// copy-permission check (build::copyable_object). Local ids never go to
    /// the simulator as texture/material assignments.
    pub fn expect_local_duplicates(
        &mut self,
        region: RegionHandle,
        roots: &[u32],
        offset: Vec3,
        selected: bool,
        ray: Option<(Vec3, f32, bool)>,
    ) {
        let now = Instant::now();
        self.local_duplicates.copies.retain(|copy| now.duration_since(copy.at) < TIMEOUT);
        if self.local_duplicates.copies.is_empty() {
            self.local_duplicates.candidates.clear();
        }
        for &local_id in roots {
            let key = ObjKey { region, local_id };
            let Some(index) = self.index_of(&key) else { continue };
            let Some(object) = self.get(index) else { continue };
            let root = Prim::capture(object);
            let children: Vec<_> = self
                .children_of(&key)
                .iter()
                .filter_map(|&index| self.get(index).filter(|object| !object.is_avatar()).map(Prim::capture))
                .collect();
            if root.appearance.is_empty() && children.iter().all(|child| child.appearance.is_empty()) {
                continue;
            }
            let (position, rotation) = ray.map_or(((root.position + offset, 0.01), true), |(point, radius, rotates)| {
                ((point, radius), !rotates)
            });
            if self.local_duplicates.copies.len() == MAX_PENDING {
                self.local_duplicates.copies.remove(0);
            }
            self.local_duplicates.copies.push(Copy {
                at: now,
                region,
                root,
                children,
                parent: None,
                position,
                rotation,
                selected,
            });
        }
    }

    pub fn take_local_duplicate_changes(&mut self) -> Vec<(Uuid, Option<u8>, Option<LocalAppearance>)> {
        std::mem::take(&mut self.local_duplicates.changes)
    }

    pub fn cancel_local_duplicates(&mut self) {
        self.local_duplicates.copies.clear();
        self.local_duplicates.candidates.clear();
    }

    /// Observe full updates only: existing objects and region renumberings must
    /// never be mistaken for a newly duplicated prim. Children may arrive first.
    pub(super) fn apply_local_duplicates(&mut self, index: usize, new: bool) {
        let mut pending = std::mem::take(&mut self.local_duplicates);
        pending.copies.retain(|copy| copy.at.elapsed() < TIMEOUT);
        if !pending.copies.is_empty() {
            let mut relevant = self.get(index).is_some_and(|object| pending.candidates.contains(&object.full_id));
            if new
                && let Some(object) = self.get(index)
                && pending.candidates.len() < MAX_CANDIDATES
                && pending.copies.iter().any(|copy| {
                    copy.region == object.key.region
                        && if object.parent_id == 0 {
                            (!copy.selected || object.update_flags & crate::build::FLAGS_CREATE_SELECTED != 0)
                                && copy.root.matches(object, Some(copy.position), copy.rotation)
                        } else {
                            copy.children
                                .iter()
                                .any(|child| child.matches(object, Some((child.position, 0.01)), true))
                        }
                })
            {
                pending.candidates.push(object.full_id);
                relevant = true;
            }
            if !relevant {
                self.local_duplicates = pending;
                return;
            }
            for n in 0..pending.copies.len() {
                // Identical overlapping sources with different local bindings
                // cannot be distinguished by an ObjectUpdate. Leave those alone.
                let ambiguous =
                    pending.candidates.iter().filter_map(|id| self.index_of_uuid(id)).any(|idx| {
                        self.get(idx).is_some_and(|object| {
                            pending.copies[n].matches_root(object)
                                && pending.copies.iter().enumerate().any(|(other, copy)| {
                                    other != n && copy.matches_root(object) && !copy.same_appearance(&pending.copies[n])
                                })
                        })
                    });
                if ambiguous {
                    continue;
                }
                let copy = &mut pending.copies[n];
                if copy.parent.is_none() {
                    let matches: Vec<_> = pending
                        .candidates
                        .iter()
                        .filter_map(|id| self.index_of_uuid(id))
                        .filter(|&idx| self.get(idx).is_some_and(|object| copy.matches_root(object)))
                        .collect();
                    // An ambiguous reply cannot safely identify the source.
                    if matches.len() == 1 {
                        let idx = matches[0];
                        if let Some(object) = self.get_mut(idx) {
                            copy.parent = Some(object.key);
                            pending.candidates.retain(|id| *id != object.full_id);
                            apply(object, &copy.root, &mut pending.changes);
                        }
                    }
                }
                let Some(parent) = copy.parent else { continue };
                let ambiguous: HashSet<_> = pending
                    .candidates
                    .iter()
                    .filter_map(|id| self.index_of_uuid(id))
                    .filter_map(|idx| {
                        let object = self.get(idx)?;
                        if object.parent_key() != Some(parent) {
                            return None;
                        }
                        let mut matches = copy
                            .children
                            .iter()
                            .filter(|child| child.matches(object, Some((child.position, 0.01)), true));
                        let first = matches.next()?;
                        matches.any(|child| child.appearance != first.appearance).then_some(object.full_id)
                    })
                    .collect();
                copy.children.retain(|child| {
                    let matches: Vec<_> = pending
                        .candidates
                        .iter()
                        .filter_map(|id| self.index_of_uuid(id))
                        .filter(|&idx| {
                            self.get(idx).is_some_and(|object| {
                                !ambiguous.contains(&object.full_id)
                                    && object.parent_key() == Some(parent)
                                    && child.matches(object, Some((child.position, 0.01)), true)
                            })
                        })
                        .collect();
                    if matches.len() != 1 {
                        return true;
                    }
                    if let Some(object) = self.get_mut(matches[0]) {
                        pending.candidates.retain(|id| *id != object.full_id);
                        apply(object, child, &mut pending.changes);
                    }
                    false
                });
            }
            pending.copies.retain(|copy| copy.parent.is_none() || !copy.children.is_empty());
        }
        if pending.copies.is_empty() {
            pending.candidates.clear();
        }
        self.local_duplicates = pending;
    }
}

fn apply(object: &mut Object, source: &Prim, changes: &mut Vec<(Uuid, Option<u8>, Option<LocalAppearance>)>) {
    for (&face, &appearance) in &source.appearance {
        object.render.local_appearance.insert(face, appearance);
        changes.push((object.full_id, face, Some(appearance)));
    }
    if !source.appearance.is_empty() {
        object.material_dirty = true;
        object.render.needs_records = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update(id: u32, parent: u32, position: Vec3) -> ObjectUpdate {
        ObjectUpdate {
            local_id: id,
            full_id: Uuid::from_u128(id as u128),
            parent_id: parent,
            pcode: aurora_prim::params::LL_PCODE_VOLUME,
            position,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
            texture_entry: Some(Arc::new(TextureEntry::default())),
            update_flags: crate::build::FLAGS_CREATE_SELECTED,
            ..crate::demo::action_avatar(false)
        }
    }

    fn fixture() -> (ObjectStore, usize, usize) {
        let mut store = ObjectStore::default();
        let root = store.upsert(1, update(10, 0, Vec3::splat(10.0)));
        let child = store.upsert(1, update(11, 10, Vec3::X));
        let root_appearance = &mut store.get_mut(root).unwrap().render.local_appearance;
        root_appearance.insert(None, LocalAppearance::Texture(Uuid::from_u128(100)));
        let mut channels = Some(LocalAppearance::Material(Uuid::from_u128(101)));
        for (n, slot) in [
            LocalTextureSlot::Normal,
            LocalTextureSlot::Specular,
            LocalTextureSlot::Pbr(0),
            LocalTextureSlot::Pbr(1),
            LocalTextureSlot::Pbr(2),
            LocalTextureSlot::Pbr(3),
            LocalTextureSlot::Light,
        ]
        .into_iter()
        .enumerate()
        {
            channels = LocalAppearance::with_texture(channels, slot, Some(Uuid::from_u128(102 + n as u128)));
        }
        root_appearance.insert(Some(2), channels.unwrap());
        store
            .get_mut(child)
            .unwrap()
            .render
            .local_appearance
            .insert(Some(1), LocalAppearance::Material(Uuid::from_u128(200)));
        (store, root, child)
    }

    #[test]
    fn local_duplicates_keep_all_channels_and_children_arriving_before_or_after_root() {
        for child_first in [true, false] {
            let (mut store, root, child) = fixture();
            let original_root = store.get(root).unwrap().render.local_appearance.clone();
            let original_child = store.get(child).unwrap().render.local_appearance.clone();
            store.expect_local_duplicates(1, &[10], Vec3::X, true, None);
            // The sent snapshot survives later source edits.
            store.get_mut(root).unwrap().render.local_appearance.clear();
            let root_update = update(20, 0, Vec3::splat(10.0) + Vec3::X);
            let child_update = update(21, 20, Vec3::X);
            let server_te = root_update.texture_entry.clone();
            if child_first {
                store.upsert(1, child_update);
                store.upsert(1, root_update);
            } else {
                store.upsert(1, root_update);
                store.upsert(1, child_update);
            }
            let root = store.index_of_uuid(&Uuid::from_u128(20)).unwrap();
            let child = store.index_of_uuid(&Uuid::from_u128(21)).unwrap();
            assert_eq!(store.get(root).unwrap().render.local_appearance, original_root);
            assert_eq!(store.get(child).unwrap().render.local_appearance, original_child);
            assert_eq!(store.get(root).unwrap().te, server_te);
            assert!(store.get(root).unwrap().material_dirty && store.get(root).unwrap().render.needs_records);
            assert_eq!(
                store.take_local_duplicate_changes().len(),
                3,
                "every binding reaches the persistence/removal registry"
            );
            store.upsert(1, update(20, 0, Vec3::splat(10.0) + Vec3::X));
            assert_eq!(store.get(root).unwrap().render.local_appearance, original_root);
            assert!(store.take_local_duplicate_changes().is_empty());
            assert!(store.local_duplicates.copies.is_empty());
        }
    }

    #[test]
    fn local_duplicates_shift_drag_keeps_copy_at_original_position() {
        let (mut store, root, _) = fixture();
        let appearance = store.get(root).unwrap().render.local_appearance.clone();
        store.expect_local_duplicates(1, &[10], Vec3::ZERO, false, None);
        store.get_mut(root).unwrap().position += Vec3::X * 5.0;
        let mut copy = update(20, 0, Vec3::splat(10.0));
        copy.update_flags = 0;
        let index = store.upsert(1, copy);
        assert_eq!(store.get(index).unwrap().render.local_appearance, appearance);
    }

    #[test]
    fn local_duplicates_ignore_existing_rekeyed_foreign_and_nonmatching_objects() {
        let (mut store, _, _) = fixture();
        store.upsert(1, update(20, 0, Vec3::splat(10.0) + Vec3::X));
        store.expect_local_duplicates(1, &[10], Vec3::X, true, None);
        let existing = store.upsert(1, update(20, 0, Vec3::splat(10.0) + Vec3::X));
        assert!(store.get(existing).unwrap().render.local_appearance.is_empty());
        let mut rekeyed = update(22, 0, Vec3::splat(10.0) + Vec3::X);
        rekeyed.full_id = Uuid::from_u128(20);
        let existing = store.upsert(1, rekeyed);
        assert!(store.get(existing).unwrap().render.local_appearance.is_empty());
        for kind in 0..5 {
            let mut other = update(30 + kind, 0, Vec3::splat(10.0) + Vec3::X);
            let region = if kind == 0 { 2 } else { 1 };
            match kind {
                1 => other.position += Vec3::Z,
                2 => other.scale *= 2.0,
                3 => Arc::make_mut(other.texture_entry.as_mut().unwrap()).faces[0].texture = Uuid::new_v4(),
                4 => other.update_flags = 0,
                _ => {}
            }
            let index = store.upsert(region, other);
            assert!(store.get(index).unwrap().render.local_appearance.is_empty());
        }
        let index = store.upsert(1, update(40, 0, Vec3::splat(10.0) + Vec3::X));
        assert!(!store.get(index).unwrap().render.local_appearance.is_empty());
    }

    #[test]
    fn local_duplicates_expire_and_do_not_reuse_old_candidates() {
        let (mut store, _, _) = fixture();
        store.expect_local_duplicates(1, &[10], Vec3::X, true, None);
        store.upsert(1, update(21, 20, Vec3::X));
        for copy in &mut store.local_duplicates.copies {
            copy.at -= TIMEOUT;
        }
        store.expect_local_duplicates(1, &[10], Vec3::X, true, None);
        assert!(store.local_duplicates.candidates.is_empty());
        for copy in &mut store.local_duplicates.copies {
            copy.at -= TIMEOUT;
        }
        let index = store.upsert(1, update(20, 0, Vec3::splat(10.0) + Vec3::X));
        assert!(store.get(index).unwrap().render.local_appearance.is_empty());
        assert!(store.local_duplicates.copies.is_empty());
    }

    #[test]
    fn local_duplicates_on_ray_accept_surface_offset_and_rotation() {
        let (mut store, root, _) = fixture();
        let appearance = store.get(root).unwrap().render.local_appearance.clone();
        let hit = Vec3::splat(20.0);
        store.expect_local_duplicates(1, &[10], Vec3::ZERO, false, Some((hit, 2.0, true)));
        let mut copy = update(20, 0, hit + Vec3::Z * 0.5);
        copy.rotation = Quat::from_rotation_x(1.0);
        copy.update_flags = 0;
        let index = store.upsert(1, copy);
        assert_eq!(store.get(index).unwrap().render.local_appearance, appearance);
    }

    #[test]
    fn local_duplicates_do_not_swap_ambiguous_overlapping_bindings() {
        let (mut store, _, _) = fixture();
        let other = store.upsert(1, update(12, 0, Vec3::splat(10.0)));
        store
            .get_mut(other)
            .unwrap()
            .render
            .local_appearance
            .insert(None, LocalAppearance::Texture(Uuid::from_u128(300)));
        store.expect_local_duplicates(1, &[10, 12], Vec3::X, true, None);
        let index = store.upsert(1, update(20, 0, Vec3::splat(10.0) + Vec3::X));
        assert!(store.get(index).unwrap().render.local_appearance.is_empty());
        assert!(store.take_local_duplicate_changes().is_empty());
    }
}
