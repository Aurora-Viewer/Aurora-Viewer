//! What the draw lists look for in every object slot at each frame.
//!
//! With the GPU lists, `Scene::build_lists` and `Scene::build_huds` only
//! pick a few objects among all the slots: the avatars, the objects with
//! blended or glowing faces, a thirtieth of the drawn ones for the texture
//! usage, the HUD attachments. Reading that in `ObjGpu` (a few hundred
//! bytes a slot) walked megabytes at each frame, for every slot used since
//! the session began, as the slab never shrinks: 0.45 ms a frame in a
//! skybox of 12,000 objects after a region of 23,000 (0.15 ms before it),
//! 0.18 ms in a world emptied after 19,500 objects.
//!
//! One byte per slot holds it instead. It is written where the GPU object
//! table is, which reads the same state: after an object's sync, when its
//! faces change class, when its slot is released.
//!
//! `AURORA_LIST_INDEX=0` walks the objects as before, to compare;
//! `check` compares the index with the objects every thirtieth frame and
//! logs the slots that differ (there must be none).

use super::ObjGpu;
use std::sync::OnceLock;

/// Has faces and is not a HUD attachment: what the lists of the world draw.
pub const DRAWN: u8 = 1;
/// `ObjGpu::special`: blended or glowing faces, listed by the CPU.
pub const SPECIAL: u8 = 2;
pub const AVATAR: u8 = 4;
/// A HUD attachment, with or without faces.
pub const HUD: u8 = 8;
/// System bodies and everything worn in world space, for portal cuts.
pub const AVATAR_PART: u8 = 16;

/// AURORA_LIST_INDEX.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// The lists read the index.
    Index,
    /// They walk the objects, as before the index.
    Walk,
    /// They read the index, compared with the objects now and then.
    Check,
}

pub fn mode() -> Mode {
    static MODE: OnceLock<Mode> = OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("AURORA_LIST_INDEX").as_deref() {
        Ok("0") => Mode::Walk,
        Ok("check") => Mode::Check,
        _ => Mode::Index,
    })
}

/// One byte per object slot (0 for a slot never written).
#[derive(Default)]
pub struct ListIndex {
    bits: Vec<u8>,
    /// `Mode::Check`: differences already logged (the first few only).
    reported: u32,
}

impl ListIndex {
    /// The byte of an object as its last sync left it.
    pub fn of(g: &ObjGpu) -> u8 {
        let mut b = 0;
        if !g.faces.is_empty() && !g.hud {
            b |= DRAWN;
        }
        if g.special {
            b |= SPECIAL;
        }
        if g.is_avatar {
            b |= AVATAR;
        }
        if g.hud {
            b |= HUD;
        }
        if !g.hud && (g.is_avatar || g.owner_avatar.is_some()) {
            b |= AVATAR_PART;
        }
        b
    }

    pub fn set(&mut self, idx: usize, g: &ObjGpu) {
        let b = Self::of(g);
        if idx >= self.bits.len() {
            if b == 0 {
                return;
            }
            self.bits.resize(idx + 1, 0);
        }
        self.bits[idx] = b;
    }

    /// The slot holds nothing any more.
    pub fn remove(&mut self, idx: usize) {
        if let Some(b) = self.bits.get_mut(idx) {
            *b = 0;
        }
    }

    /// Everything is gone (teleport, logout).
    pub fn reset(&mut self) {
        self.bits.clear();
    }

    /// The slots holding any bit of `mask`, in slot order, with their byte.
    pub fn with(&self, mask: u8) -> impl Iterator<Item = (usize, u8)> + '_ {
        self.bits
            .iter()
            .enumerate()
            .filter(move |(_, b)| **b & mask != 0)
            .map(|(i, b)| (i, *b))
    }

    /// `Mode::Check`: every slot must hold what its object says.
    pub fn verify(&mut self, gpu: &[ObjGpu]) {
        for (idx, g) in gpu.iter().enumerate() {
            let (held, wanted) = (self.bits.get(idx).copied().unwrap_or(0), Self::of(g));
            if held != wanted && self.reported < 20 {
                self.reported += 1;
                log::warn!("list index check: slot {idx} holds {held:#06b}, its object says {wanted:#06b}");
            }
        }
        if let Some(idx) = (gpu.len()..self.bits.len()).find(|&i| self.bits[i] != 0)
            && self.reported < 20
        {
            self.reported += 1;
            log::warn!("list index check: slot {idx} is past the objects and holds {:#06b}", self.bits[idx]);
        }
    }
}

#[cfg(test)]
impl super::Scene {
    /// Tests that set the objects' state by hand, without a sync: the
    /// index as the sync would leave it.
    pub(in crate::scene) fn index_objects(&mut self) {
        for (idx, g) in self.gpu.iter().enumerate() {
            self.list_index.set(idx, g);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::Pass;
    use crate::scene::tests::test_face;

    #[test]
    fn the_byte_follows_what_the_lists_read_of_an_object() {
        let mut g = ObjGpu::default();
        assert_eq!(ListIndex::of(&g), 0);
        // an avatar is listed before it has faces (its state rules what it wears)
        g.is_avatar = true;
        assert_eq!(ListIndex::of(&g), AVATAR | AVATAR_PART);
        g.faces.push(test_face(Pass::Opaque, false));
        assert_eq!(ListIndex::of(&g), AVATAR | AVATAR_PART | DRAWN);
        g.is_avatar = false;
        g.special = true;
        assert_eq!(ListIndex::of(&g), DRAWN | SPECIAL);
        g.owner_avatar = Some(4);
        assert_eq!(ListIndex::of(&g), DRAWN | SPECIAL | AVATAR_PART);
        // a HUD attachment is not drawn by the lists of the world
        g.hud = true;
        assert_eq!(ListIndex::of(&g), SPECIAL | HUD);
        g.faces.clear();
        g.special = false;
        assert_eq!(ListIndex::of(&g), HUD);
    }

    #[test]
    fn slots_are_found_by_bit_in_slot_order_until_released() {
        let mut index = ListIndex::default();
        let face = || test_face(Pass::Opaque, false);
        let drawn = ObjGpu {
            faces: vec![face()],
            ..Default::default()
        };
        let blended = ObjGpu {
            faces: vec![face()],
            special: true,
            ..Default::default()
        };
        let avatar = ObjGpu {
            is_avatar: true,
            ..Default::default()
        };
        // an empty object past the end takes no room
        index.set(900, &ObjGpu::default());
        assert_eq!(index.with(u8::MAX).count(), 0);
        index.set(7, &blended);
        index.set(2, &drawn);
        index.set(40, &avatar);
        let found = |index: &ListIndex, mask| index.with(mask).map(|(i, _)| i).collect::<Vec<_>>();
        assert_eq!(found(&index, DRAWN), [2, 7]);
        assert_eq!(found(&index, SPECIAL), [7]);
        assert_eq!(found(&index, AVATAR), [40]);
        assert_eq!(found(&index, AVATAR_PART), [40]);
        assert_eq!(index.with(DRAWN).map(|(_, b)| b).collect::<Vec<_>>(), [DRAWN, DRAWN | SPECIAL]);
        // the object changes, then its slot is released
        index.set(7, &drawn);
        assert_eq!(found(&index, SPECIAL), [] as [usize; 0]);
        index.remove(2);
        index.remove(5000);
        assert_eq!(found(&index, DRAWN), [7]);
        index.reset();
        assert_eq!(index.with(u8::MAX).count(), 0);
    }
}
