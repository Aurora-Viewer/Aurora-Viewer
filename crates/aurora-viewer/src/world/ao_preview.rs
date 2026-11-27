//! The AO window's preview: a client-only copy of our avatar and of what it
//! wears (made as the animation editor's mirror is, mirror.rs; its worn
//! copies give its skeleton our rigged meshes' joint positions, without
//! their lights) that plays the animation the window previews. It is drawn
//! nowhere (the scene leaves it out of every view,
//! `Scene::sync_photo_isolation`); its joints pose our outfit on the
//! preview's stage, the 3D view of Inspect (`object_stage`,
//! app/ao_preview.rs).
//!
//! Nothing about it is sent to the region, and no one else sees it. The
//! Legacy Viewer has no such preview: it plays an animation on the avatar
//! itself (LLPreviewAnim's "Play Locally").

use super::*;

/// The copy's local ids, clear of the mirror's (0xFFF0_0000), the
/// mannequins' (0xFFF1_0000…) and the avatar cache's (0xFFE0_0000…).
const FIRST_LOCAL: u32 = 0xFFF3_0000;
/// How far we may walk before the copy follows (metres): it stays in range,
/// where the scene poses avatars.
const FOLLOW: f32 = 8.0;

#[derive(Debug, Clone)]
pub struct AoPreview {
    pub id: Uuid,
    root: ObjKey,
    /// Where it was put, to follow us.
    at: Vec3,
    /// What we wore when it was made: an outfit change makes it again, so
    /// that its joints stay those of what we wear.
    worn: Vec<Uuid>,
    /// The animations it plays: the previewed entry's assets.
    assets: Vec<Uuid>,
    animations: Vec<PlayingAnimation>,
}

/// The copy's avatar id for `agent`: fixed, so that a copy made again finds
/// the scene's state of the last one.
pub fn preview_id(agent: Uuid) -> Uuid {
    Uuid::from_u128(agent.as_u128() ^ 0x616f_7072_6576_0000_0000_0000_0000_0003)
}

impl World {
    /// The preview copy's avatar id, while there is one.
    pub fn ao_preview(&self) -> Option<Uuid> {
        self.ao_preview.as_ref().map(|p| p.id)
    }

    pub fn is_ao_preview(&self, id: &Uuid) -> bool {
        self.ao_preview.as_ref().is_some_and(|p| p.id == *id)
    }

    /// Whether the object at `idx` is the preview copy or worn by it.
    pub fn on_ao_preview(&self, idx: usize) -> bool {
        self.ao_preview.is_some()
            && crate::ui::context::wearer(self, idx)
                .or_else(|| self.objects.get(idx).map(|o| o.full_id))
                .is_some_and(|id| self.is_ao_preview(&id))
    }

    /// What the copy plays, for `animations_of`.
    pub(super) fn ao_preview_animations(&self, id: &Uuid) -> Option<&[PlayingAnimation]> {
        self.ao_preview.as_ref().filter(|p| p.id == *id).map(|p| p.animations.as_slice())
    }

    /// Make the copy (or keep it), where we stand and turned as we are, and
    /// have it play `assets` from `now` when they change. It follows us once
    /// we walk away from it. None while our avatar is not in the world.
    pub fn update_ao_preview(&mut self, assets: &[Uuid], now: Instant) -> Option<Uuid> {
        let id = preview_id(self.agent_id);
        let here = self.agent.position;
        let rotation = self.agent.body_rotation();
        let alive = self
            .ao_preview
            .as_ref()
            .is_some_and(|p| self.objects.index_of_uuid(&p.id).is_some());
        let worn = self.own_worn();
        let moved = self.ao_preview.as_ref().is_none_or(|p| p.at.distance(here) > FOLLOW);
        let dressed = self.ao_preview.as_ref().is_some_and(|p| p.worn != worn);
        if dressed {
            // (what we took off would stay on it: made again from nothing)
            if let Some(old) = &self.ao_preview {
                self.kill_objects(old.root.region, &[old.root.local_id]);
            }
        }
        if !alive || moved || dressed {
            // (moved in place: `upsert_preview` keeps the objects there)
            let root = self.copy_own_avatar(id, FIRST_LOCAL, here, rotation, false)?;
            let old = self.ao_preview.take();
            self.ao_preview = Some(AoPreview {
                id,
                root,
                at: here,
                worn,
                assets: old.as_ref().map_or_else(Vec::new, |p| p.assets.clone()),
                animations: old.map_or_else(Vec::new, |p| p.animations),
            });
            if !alive {
                log::info!("AO preview: a copy of our avatar plays the previewed animation");
            }
        }
        let preview = self.ao_preview.as_mut()?;
        if preview.assets != assets {
            preview.assets = assets.to_vec();
            preview.animations = assets
                .iter()
                .filter(|a| !a.is_nil())
                .map(|&a| PlayingAnimation::from_signal(a, 1, now, None))
                .collect();
        }
        Some(id)
    }

    /// What our avatar wears, by id (the copy's signature).
    fn own_worn(&self) -> Vec<Uuid> {
        let Some(avatar) = self.objects.index_of_uuid(&self.agent_id).and_then(|i| self.objects.get(i)) else {
            return Vec::new();
        };
        let mut worn: Vec<Uuid> = self
            .objects
            .children_of(&avatar.key)
            .iter()
            .filter_map(|&i| self.objects.get(i))
            .filter(|o| !o.is_avatar() && !o.is_client_preview())
            .map(|o| o.full_id)
            .collect();
        worn.sort_unstable();
        worn
    }

    /// Take the copy away, with what it wears.
    pub fn remove_ao_preview(&mut self) {
        if let Some(preview) = self.ao_preview.take() {
            self.kill_objects(preview.root.region, &[preview.root.local_id]);
            log::info!("AO preview: copy taken away");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world_with_avatar() -> World {
        let mut world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let me = Uuid::from_u128(0xA0E0_A6E1_0000_0000_0000_0000_0000_0001);
        world.agent_id = me;
        let (handle, avatar) = crate::demo::events()
            .into_iter()
            .find_map(|ev| match ev {
                aurora_net::NetEvent::ObjectUpdates { handle, objects } => {
                    objects.into_iter().find(|o| o.full_id == me).map(|o| (handle, o))
                }
                _ => None,
            })
            .expect("demo contains our avatar");
        world.main_region = Some(handle);
        world.ensure_region(handle, (256, 256));
        world.apply(aurora_net::NetEvent::ObjectUpdates {
            handle,
            objects: vec![avatar],
        });
        world
    }

    #[test]
    fn the_copy_is_ours_local_plays_the_preview_and_goes() {
        let mut world = world_with_avatar();
        let now = Instant::now();
        let anim = Uuid::from_u128(0xA11);
        let id = world.update_ao_preview(&[anim], now).expect("a copy");
        let idx = world.objects.index_of_uuid(&id).expect("in the world's objects");
        assert!(world.on_ao_preview(idx));
        assert!(world.is_local_copy(&id), "a local copy: not on the radar, our appearance");
        assert_eq!(world.appearance_of(&id), world.agent_id);
        assert_eq!(world.animations_of(&id).iter().map(|a| a.id).collect::<Vec<_>>(), [anim]);
        let own = world.objects.index_of_uuid(&world.agent_id).expect("our avatar");
        assert!(!world.on_ao_preview(own), "our own avatar is not the copy");
        // the same entry: the animation keeps its start
        let start = world.animations_of(&id)[0];
        world.update_ao_preview(&[anim], now + std::time::Duration::from_secs(2));
        assert_eq!(world.animations_of(&id)[0], start);
        world.remove_ao_preview();
        assert!(world.objects.index_of_uuid(&id).is_none());
        assert!(world.ao_preview().is_none());
    }
}
