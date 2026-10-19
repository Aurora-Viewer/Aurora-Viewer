//! The bake an avatar shows: the last one that had pixels, until the new
//! one has its own. After LLVOAvatar::updateBakedTextures (use_lkg_baked_layer,
//! mLastTextureID) and onBakedTextureLoaded / useBakedTexture
//! (indra/newview/llvoavatar.cpp, originally LGPL 2.1): a new bake waits
//! for its first pixels behind the last good one, so a rebake (an outfit
//! change, a body applier, an alpha toggled by a script) never shows the
//! body grey (or, with the rez effect, as its grid) while it downloads.
//!
//! The new bake is held (`TextureStreamer::acquire`) so that it downloads
//! and counts for the avatar's loading (`loading.rs` only counts a bake
//! something uses); once it has pixels the world builds the avatar and
//! what shows its bakes again (`World::refresh_bakes`), with it.

use super::{Scene, avatar};
use crate::scene::textures::TexSource;
use aurora_render::Renderer;
use std::collections::HashMap;
use uuid::Uuid;

/// Priority of a bake held back (as the bakes of an avatar loading,
/// `loading.rs`).
const HELD_PIXELS: f32 = 512.0;

#[derive(Default)]
pub struct ShownBakes {
    /// What each bake slot of an avatar shows, by (avatar, texture-entry index).
    shown: HashMap<(Uuid, usize), Uuid>,
    /// New bakes loading behind the shown ones, held, and whether the
    /// world was asked to build their avatar again (once they have pixels).
    pending: HashMap<(Uuid, usize), (Uuid, bool)>,
    /// Holds to let go next frame, once the faces built again hold the
    /// bake themselves.
    release: Vec<Uuid>,
}

/// Whether a bake has nothing to wait for: none, the default or the
/// invisible one (never downloaded).
fn nothing_to_load(id: &Uuid) -> bool {
    id.is_nil() || *id == avatar::IMG_DEFAULT_AVATAR || *id == avatar::IMG_INVISIBLE
}

/// The bake to show: the new one, unless it has no pixels yet while the
/// one shown so far has.
fn pick(new: Uuid, shown: Option<Uuid>, new_ready: bool, shown_ready: bool) -> Uuid {
    match shown {
        Some(old) if old != new && !new_ready && shown_ready && !nothing_to_load(&old) => old,
        _ => new,
    }
}

impl Scene {
    /// The bake slot `te` of avatar `avatar_idx` as it is drawn
    /// (`World::bake_texture`, or the last one with pixels while the new
    /// one downloads).
    pub(super) fn shown_bake(
        &mut self,
        renderer: &mut Renderer,
        world: &crate::world::World,
        avatar_idx: usize,
        te: usize,
    ) -> Option<Uuid> {
        let new = world.bake_texture(avatar_idx, te)?;
        let id = world.objects.get(avatar_idx)?.full_id;
        let key = (id, te);
        let shown = self.shown_bakes.shown.get(&key).copied();
        let ready =
            |s: &Scene, t: &Uuid| nothing_to_load(t) || s.textures.is_loaded(t) || s.textures.is_local(t) || s.textures.is_unavailable(t);
        let new_ready = ready(self, &new);
        let shown_ready = shown.is_some_and(|t| self.textures.is_loaded(&t) || self.textures.is_local(&t));
        let pick = pick(new, shown, new_ready, shown_ready);
        if pick != new {
            if self.shown_bakes.pending.get(&key).map(|p| p.0) != Some(new) {
                let source = avatar::bake_name(te).map_or(TexSource::Asset, |b| world.bake_source(id, b, new));
                let _ = self.textures.acquire(renderer, new, source);
                if let Some((old, _)) = self.shown_bakes.pending.insert(key, (new, false)) {
                    self.textures.release(&old);
                }
            }
            return Some(pick);
        }
        self.shown_bakes.shown.insert(key, new);
        if let Some((held, _)) = self.shown_bakes.pending.remove(&key) {
            self.shown_bakes.release.push(held);
        }
        Some(new)
    }

    /// Once a frame, before the sync: the bakes held back that have pixels
    /// now are shown (their avatars are built again), those no longer
    /// wanted are let go.
    pub(super) fn update_shown_bakes(&mut self, world: &mut crate::world::World) {
        for t in std::mem::take(&mut self.shown_bakes.release) {
            self.textures.release(&t);
        }
        let mut refresh = Vec::new();
        let mut gone = Vec::new();
        for (&(avatar, te), (id, asked)) in self.shown_bakes.pending.iter_mut() {
            let current = world.objects.index_of_uuid(&avatar).and_then(|i| world.bake_texture(i, te));
            if current != Some(*id) {
                gone.push((avatar, te));
            } else if self.textures.is_loaded(id) || self.textures.is_unavailable(id) {
                // (once: the sync that builds the avatar swaps it in)
                if !std::mem::replace(asked, true) {
                    refresh.push(avatar);
                }
            } else {
                self.textures.note_usage(id, HELD_PIXELS);
            }
        }
        for key in gone {
            if let Some((t, _)) = self.shown_bakes.pending.remove(&key) {
                self.textures.release(&t);
            }
        }
        refresh.sort_unstable();
        refresh.dedup();
        for avatar in refresh {
            world.refresh_bakes(&avatar);
        }
        // (now and then: the avatars gone)
        if self.frame.is_multiple_of(120) {
            self.shown_bakes.shown.retain(|(a, _), _| world.objects.index_of_uuid(a).is_some());
        }
    }

    /// A bake of the avatar is held back behind the last good one.
    pub(super) fn holds_bake(&self, avatar: &Uuid) -> bool {
        self.shown_bakes.pending.keys().any(|(a, _)| a == avatar)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_bake_waits_behind_the_last_good_one() {
        let (old, new) = (Uuid::from_u128(1), Uuid::from_u128(2));
        // the first bake shows at once (nothing to keep)
        assert_eq!(pick(new, None, false, false), new);
        // a rebake loading: the last good one stays
        assert_eq!(pick(new, Some(old), false, true), old);
        // it has pixels: it shows
        assert_eq!(pick(new, Some(old), true, true), new);
        // the one shown had none either: nothing better to keep
        assert_eq!(pick(new, Some(old), false, false), new);
        // the same bake
        assert_eq!(pick(old, Some(old), false, true), old);
    }

    #[test]
    fn nothing_is_kept_behind_a_bake_with_nothing_to_load() {
        let new = Uuid::from_u128(2);
        // from the default or invisible bake: the new one, as it comes
        assert_eq!(pick(new, Some(avatar::IMG_DEFAULT_AVATAR), false, true), new);
        assert_eq!(pick(new, Some(avatar::IMG_INVISIBLE), false, true), new);
    }
}
