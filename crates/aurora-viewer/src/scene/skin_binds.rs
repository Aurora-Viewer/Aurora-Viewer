//! Skin bindings sent to the GPU: one range of `SkinBind` per skinned mesh
//! (inverse bind matrix + palette joint of each mesh joint), referenced by
//! `DrawRecord::flags[2]`. The default skeleton comes first (system avatar),
//! then the rigged meshes by asset id.
//!
//! A mesh's range is given back when its last geometry is freed and taken
//! again by the next meshes, as LLMeshRepository::notifyLoadedMeshes erases
//! the skin info nothing refers to any more (indra/newview/llmeshrepository.cpp):
//! the list used to grow with every rigged mesh met until the viewer closed.

use super::anim::Rig;
use aurora_render::SkinBind;
use aurora_render::arena::RangeAllocator;
use glam::Mat4;
use std::collections::HashMap;
use std::ops::Range;
use uuid::Uuid;

#[derive(Default)]
pub(super) struct SkinBinds {
    /// As on the GPU; the entries of a freed range keep their old values
    /// until a mesh takes them (no record refers to them).
    list: Vec<SkinBind>,
    /// (first binding, count) of each rigged mesh.
    ranges: HashMap<Uuid, (u32, u32)>,
    /// Free ranges of `list` (its capacity is the list's length).
    free: Option<RangeAllocator>,
    /// Ranges of `list` not sent to the GPU yet.
    dirty: Vec<Range<usize>>,
}

impl SkinBinds {
    pub(super) fn list(&self) -> &[SkinBind] {
        &self.list
    }

    /// First binding of a rigged mesh that has its range.
    pub(super) fn base_of(&self, mesh: &Uuid) -> Option<u32> {
        self.ranges.get(mesh).map(|r| r.0)
    }

    /// Bindings in use and in the list (diagnostics).
    pub(super) fn counts(&self) -> (usize, usize) {
        (self.free.as_ref().map_or(0, |a| a.used() as usize), self.list.len())
    }

    /// A range of `len` bindings: a freed one when one fits, else new
    /// entries at the end of the list.
    fn alloc(&mut self, len: u32) -> u32 {
        let free = self.free.get_or_insert_with(|| RangeAllocator::new(0));
        let base = match free.alloc(len) {
            Some(base) => base,
            None => {
                free.grow(free.capacity() + len);
                // (the new room, with what was free before it: always fits)
                free.alloc(len).unwrap_or(free.capacity() - len)
            }
        };
        let unset = SkinBind {
            inverse_bind: [[0.0; 4]; 4],
            joint: [0; 4],
        };
        self.list.resize(free.capacity() as usize, unset);
        base
    }

    fn mark(&mut self, range: Range<usize>) {
        if range.is_empty() {
            return;
        }
        // (a crowd loading appends mesh after mesh: one range to send)
        match self.dirty.last_mut() {
            Some(last) if last.end == range.start => last.end = range.end,
            _ => self.dirty.push(range),
        }
    }

    /// Skin bindings of the system avatar mesh: index = rig joint (plus the
    /// identity slot), inverse bind = inverse default joint matrix. Always
    /// the first range, never freed.
    pub(super) fn system(&mut self, rig: &Rig) -> u32 {
        if self.list.is_empty() {
            let n = rig.len();
            let base = self.alloc(n as u32 + 1) as usize;
            for j in 0..n {
                self.list[base + j] = SkinBind {
                    inverse_bind: rig.default_world_inv[j].to_cols_array_2d(),
                    joint: [j as u32, 0, 0, 0],
                };
            }
            self.list[base + n] = SkinBind {
                inverse_bind: Mat4::IDENTITY.to_cols_array_2d(),
                joint: [n as u32, 0, 0, 0],
            };
            self.mark(base..base + n + 1);
        }
        0
    }

    /// Skin bindings of a rigged mesh: its inverse bind matrices and the rig
    /// joints its joint names refer to ([`Rig::skin_bind`]: attachment
    /// points through their joint, other unknown names the pelvis).
    pub(super) fn mesh(&mut self, mesh: Uuid, skin: &aurora_assets::SkinInfo, rig: &Rig) -> u32 {
        self.system(rig);
        if let Some(&(base, _)) = self.ranges.get(&mesh) {
            return base;
        }
        let len = skin.joint_names.len().min(skin.inverse_bind.len());
        let base = self.alloc(len as u32);
        for (i, (name, ib)) in skin.joint_names.iter().zip(&skin.inverse_bind).enumerate() {
            let (joint, ib) = rig.skin_bind(name, *ib);
            self.list[base as usize + i] = SkinBind {
                inverse_bind: ib.to_cols_array_2d(),
                joint: [joint as u32, 0, 0, 0],
            };
        }
        self.ranges.insert(mesh, (base, len as u32));
        self.mark(base as usize..base as usize + len);
        base
    }

    /// Give back the range of a mesh no draw record refers to any more.
    pub(super) fn release(&mut self, mesh: &Uuid) {
        if let Some((base, len)) = self.ranges.remove(mesh)
            && let Some(free) = &mut self.free
        {
            free.free(base, len);
        }
    }

    /// Ranges of the list to send to the GPU.
    pub(super) fn take_dirty(&mut self) -> Vec<Range<usize>> {
        std::mem::take(&mut self.dirty)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rig(joints: usize) -> Rig {
        Rig {
            parent: vec![None; joints],
            local_pos: vec![glam::Vec3::ZERO; joints],
            local_rot: vec![glam::Quat::IDENTITY; joints],
            parent_scale: vec![glam::Vec3::ONE; joints],
            scale: vec![glam::Vec3::ONE; joints],
            default_world_inv: vec![Mat4::IDENTITY; joints],
            order: (0..joints).collect(),
            names: (0..joints).map(|j| (format!("j{j}"), j)).collect(),
            canonical_names: (0..joints).map(|j| format!("j{j}")).collect(),
            pelvis: 0,
            tag_chain: Vec::new(),
            skin_points: Default::default(),
        }
    }

    fn skin(joints: &[&str]) -> aurora_assets::SkinInfo {
        aurora_assets::SkinInfo {
            joint_names: joints.iter().map(|j| j.to_string()).collect(),
            inverse_bind: (0..joints.len())
                .map(|i| Mat4::from_translation(glam::Vec3::splat(i as f32 + 1.0)))
                .collect(),
            ..Default::default()
        }
    }

    /// (first, end) of the ranges to send.
    fn sent(b: &mut SkinBinds) -> Vec<(usize, usize)> {
        b.take_dirty().into_iter().map(|r| (r.start, r.end)).collect()
    }

    fn joints(b: &SkinBinds, base: u32, len: usize) -> Vec<u32> {
        b.list()[base as usize..base as usize + len].iter().map(|s| s.joint[0]).collect()
    }

    #[test]
    fn system_binds_come_first_and_meshes_follow() {
        let rig = rig(4);
        let mut b = SkinBinds::default();
        assert_eq!(b.system(&rig), 0);
        assert_eq!(joints(&b, 0, 5), [0, 1, 2, 3, 4]);
        let a = Uuid::from_u128(1);
        // An unknown joint follows the pelvis, retaining its inverse bind.
        let base = b.mesh(a, &skin(&["j2", "nope", "j0"]), &rig);
        assert_eq!(base, 5);
        assert_eq!(joints(&b, base, 3), [2, 0, 0]);
        assert_eq!(b.base_of(&a), Some(5));
        // asked again: the same range, nothing new to send
        assert_eq!(sent(&mut b), [(0, 8)]);
        assert_eq!(b.mesh(a, &skin(&["j2", "nope", "j0"]), &rig), 5);
        assert!(sent(&mut b).is_empty());
        assert_eq!(b.counts(), (8, 8));
    }

    #[test]
    fn rigged_skin_parity_unknown_joint_follows_the_posed_pelvis() {
        use glam::{Quat, Vec3};
        let mut rig = rig(3);
        // The fallback is the pelvis, independently of its Aurora index.
        rig.pelvis = 2;
        let mut binds = SkinBinds::default();
        let skin = skin(&["j1", "obsolete_joint"]);
        let base = binds.mesh(Uuid::from_u128(1), &skin, &rig) as usize;
        let pelvis = Mat4::from_rotation_translation(Quat::from_rotation_y(0.3), Vec3::new(0.2, -0.4, 1.1));
        let palette = [Mat4::IDENTITY, Mat4::IDENTITY, pelvis, Mat4::IDENTITY];
        let point = Vec3::new(0.1, 0.2, 1.8);
        let binding = &binds.list()[base + 1];
        let actual = (palette[binding.joint[0] as usize] * Mat4::from_cols_array_2d(&binding.inverse_bind)).transform_point3(point);
        let expected = (pelvis * skin.inverse_bind[1]).transform_point3(point);
        assert!(
            actual.distance(expected) < 1e-6,
            "unknown joint left the pelvis: {actual:?} != {expected:?}"
        );
        assert_eq!(binds.list()[base].joint[0], 1, "known joint is unchanged");
        assert_eq!(binding.inverse_bind, skin.inverse_bind[1].to_cols_array_2d());
        assert_eq!(
            binds.list()[rig.len()].joint[0],
            rig.len() as u32,
            "system identity slot is unchanged"
        );
    }

    #[test]
    fn a_released_range_is_taken_again() {
        let rig = rig(4);
        let mut b = SkinBinds::default();
        let (m1, m2, m3, m4) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3), Uuid::from_u128(4));
        assert_eq!(b.mesh(m1, &skin(&["j0", "j1", "j2"]), &rig), 5);
        assert_eq!(b.mesh(m2, &skin(&["j3", "j3"]), &rig), 8);
        b.take_dirty();
        b.release(&m1);
        assert_eq!(b.base_of(&m1), None);
        assert_eq!(b.counts(), (7, 10));
        // the other mesh keeps its bindings where its records expect them
        assert_eq!(b.base_of(&m2), Some(8));
        assert_eq!(joints(&b, 8, 2), [3, 3]);
        // fits in the hole: the list does not grow, only the hole is sent
        assert_eq!(b.mesh(m3, &skin(&["j1", "j1"]), &rig), 5);
        assert_eq!(joints(&b, 5, 2), [1, 1]);
        assert_eq!(sent(&mut b), [(5, 7)]);
        assert_eq!(b.counts(), (9, 10));
        // too long for what is left of the hole: at the end
        assert_eq!(b.mesh(m4, &skin(&["j0", "j1", "j2", "j3"]), &rig), 10);
        assert_eq!(sent(&mut b), [(10, 14)]);
        assert_eq!(b.counts(), (13, 14));
        // the system bindings never moved
        assert_eq!(b.system(&rig), 0);
        assert_eq!(joints(&b, 0, 5), [0, 1, 2, 3, 4]);
    }

    #[test]
    fn meshes_coming_and_going_do_not_grow_the_list() {
        let rig = rig(4);
        let mut b = SkinBinds::default();
        let names = ["j0", "j1", "j2", "j3", "j0", "j1", "j2"];
        let mut peak = 0;
        for round in 0..200u128 {
            // eight meshes of 1 to 7 joints at a time, never the same ones
            let ids: Vec<Uuid> = (0..8).map(|i| Uuid::from_u128(round * 8 + i + 1)).collect();
            for (i, id) in ids.iter().enumerate() {
                let n = 1 + (round as usize + i) % 7;
                let base = b.mesh(*id, &skin(&names[..n]), &rig);
                assert_eq!(joints(&b, base, n), (0..n as u32).map(|k| k % 4).collect::<Vec<_>>());
            }
            if round == 20 {
                peak = b.counts().1;
            }
            for id in &ids {
                b.release(id);
            }
            assert_eq!(b.counts().0, 5);
        }
        assert!(peak > 0 && b.counts().1 == peak, "list grew from {peak} to {}", b.counts().1);
    }

    #[test]
    fn dirty_ranges_merge_when_they_follow_each_other() {
        let rig = rig(2);
        let mut b = SkinBinds::default();
        b.mesh(Uuid::from_u128(1), &skin(&["j0", "j1"]), &rig);
        b.mesh(Uuid::from_u128(2), &skin(&["j0"]), &rig);
        assert_eq!(sent(&mut b), [(0, 6)]);
        b.release(&Uuid::from_u128(1));
        b.mesh(Uuid::from_u128(3), &skin(&["j1"]), &rig);
        b.mesh(Uuid::from_u128(4), &skin(&["j0", "j1", "j0"]), &rig);
        assert_eq!(sent(&mut b), [(3, 4), (6, 9)]);
        // a mesh without joints takes nothing and sends nothing
        b.mesh(Uuid::from_u128(5), &skin(&[]), &rig);
        assert!(sent(&mut b).is_empty());
        b.release(&Uuid::from_u128(5));
        assert_eq!(b.counts(), (8, 9));
    }
}
