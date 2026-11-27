//! The animation editor's stand-in partner: a mirror of our own avatar as
//! Person B, to make a couple animation alone. It is a client-only copy
//! (`objects::PREVIEW_REGION`, like Blender Live's previews) of our avatar
//! and of what it wears in the world (not the HUDs), under ids of its own,
//! so that the scene gives it its own skeleton and pose. Its shape and bakes
//! are read from ours (`appearance_of`), so it looks like us as we change.
//! Nothing about it is sent to the region, and no one else sees it.

use super::*;
use aurora_net::objects::ObjectUpdate;
use objects::PREVIEW_REGION;

/// The local ids of the mirror's objects, far above Blender Live's.
const FIRST_LOCAL: u32 = 0xFFF0_0000;
/// Where it appears: in front of us, facing us (metres).
const DISTANCE: f32 = 1.0;

/// The mirror's objects.
#[derive(Debug, Clone, PartialEq)]
pub struct Mirror {
    pub id: Uuid,
    root: ObjKey,
}

/// The mirror's avatar id for `agent`: fixed, so that a mirror made again
/// finds the scene's state of the last one gone.
pub fn mirror_id(agent: Uuid) -> Uuid {
    Uuid::from_u128(agent.as_u128() ^ 0x6d69_7272_6f72_0000_0000_0000_0000_0001)
}

/// A full update that makes `object` again, as a client-only copy.
pub(super) fn copy_of(object: &objects::Object, full_id: Uuid, local_id: u32, parent_id: u32) -> ObjectUpdate {
    ObjectUpdate {
        local_id,
        full_id,
        parent_id,
        pcode: object.pcode,
        state: object.state,
        crc: 0,
        material: object.prim_material,
        click_action: 0,
        scale: object.scale,
        position: object.position,
        rotation: object.rotation,
        velocity: Vec3::ZERO,
        acceleration: Vec3::ZERO,
        angular_velocity: Vec3::ZERO,
        update_flags: 0,
        owner_id: object.owner_id,
        volume: object.volume,
        texture_entry: object.te.clone(),
        extra: object.extra.clone(),
        name_values: object.name_values.clone(),
        text: String::new(),
        text_color: [0; 4],
        media_url: String::new(),
        tree_species: object.tree_species,
        texture_anim: object.tex_anim,
        foot_plane: None,
        particles: aurora_net::objects::ParticleUpdate::Keep,
        sound: Default::default(),
    }
}

impl World {
    /// The mirror's avatar id, while there is one.
    pub fn mirror(&self) -> Option<Uuid> {
        self.mirror.as_ref().map(|mirror| mirror.id)
    }

    pub fn is_mirror(&self, id: &Uuid) -> bool {
        self.mirror.as_ref().is_some_and(|mirror| mirror.id == *id)
    }

    /// Whose appearance `avatar` shows: ours for the mirror.
    pub(crate) fn appearance_of(&self, avatar: &Uuid) -> Uuid {
        if self.is_local_copy(avatar) { self.agent_id } else { *avatar }
    }

    /// Make the mirror (or keep the one there): a copy of our avatar and
    /// what it wears, standing `DISTANCE` in front of us and facing us.
    /// None while our avatar is not in the world.
    pub fn spawn_mirror(&mut self) -> Option<Uuid> {
        if let Some(id) = self.mirror()
            && self.objects.index_of_uuid(&id).is_some()
        {
            return Some(id);
        }
        self.remove_mirror();
        let id = mirror_id(self.agent_id);
        let facing = self.agent.body_rotation();
        let position = self.agent.position + facing * Vec3::new(DISTANCE, 0.0, 0.0);
        let root = self.copy_own_avatar(
            id,
            FIRST_LOCAL,
            position,
            facing * glam::Quat::from_rotation_z(std::f32::consts::PI),
        )?;
        self.mirror = Some(Mirror { id, root });
        log::info!("animation editor: a mirror of our avatar stands in as Person B");
        Some(id)
    }

    /// Make a client-only copy of our avatar and of what it wears (not the
    /// HUDs), as avatar `id` with local ids from `first_local`, its root at
    /// `position` turned by `rotation`. Shared by the mirror and the AO
    /// window's preview (ao_preview.rs). None while our avatar is not in the
    /// world.
    pub(super) fn copy_own_avatar(&mut self, id: Uuid, first_local: u32, position: Vec3, rotation: glam::Quat) -> Option<ObjKey> {
        let own = self.objects.index_of_uuid(&self.agent_id)?;
        let avatar = self.objects.get(own)?;
        let mut root = copy_of(avatar, id, first_local, 0);
        // a root of its own, even when we sit
        root.position = position;
        root.rotation = rotation;
        let mut copies = vec![(None, root)];
        let mut locals = HashMap::new();
        locals.insert(avatar.key, first_local);
        let mut next = first_local + 1;
        // what we wear, parents before their children; not the HUDs, which
        // would show twice on the screen
        for idx in self.objects.subtree(own).into_iter().skip(1) {
            // (nor the copies drawn from the cache, which go when the
            // region's objects come: the copy would keep them)
            let Some(object) = self.objects.get(idx).filter(|o| !super::avatar_cache::is_copy(o.key)) else {
                continue;
            };
            let Some(&parent) = object.parent_key().and_then(|key| locals.get(&key)) else {
                continue;
            };
            if parent == first_local && (object.is_avatar() || (31..=38).contains(&object.attachment_point())) {
                continue;
            }
            let full_id = Uuid::from_u128(id.as_u128() ^ object.full_id.as_u128().rotate_left(1));
            copies.push((
                Some(ObjKey {
                    region: PREVIEW_REGION,
                    local_id: parent,
                }),
                copy_of(object, full_id, next, parent),
            ));
            locals.insert(object.key, next);
            next += 1;
        }
        for (parent, update) in copies {
            self.objects.upsert_preview(parent, update);
        }
        Some(ObjKey {
            region: PREVIEW_REGION,
            local_id: first_local,
        })
    }

    /// Take the mirror away, with what it wears.
    pub fn remove_mirror(&mut self) {
        if let Some(mirror) = self.mirror.take() {
            self.kill_objects(mirror.root.region, &[mirror.root.local_id]);
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
        // a hat on the skull (2) and a HUD at the centre (35)
        let worn = |local_id: u32, point: u8| {
            let mut prim = avatar.clone();
            prim.pcode = 9;
            prim.local_id = local_id;
            prim.full_id = Uuid::from_u128(local_id as u128);
            prim.parent_id = avatar.local_id;
            prim.state = (point & 0x0F) << 4 | (point >> 4);
            prim.position = Vec3::ZERO;
            prim
        };
        let objects = vec![avatar.clone(), worn(70_001, 2), worn(70_002, 35)];
        world.apply(aurora_net::NetEvent::ObjectUpdates { handle, objects });
        world
    }

    #[test]
    fn the_mirror_copies_us_and_what_we_wear_but_not_the_huds() {
        let mut world = world_with_avatar();
        let id = world.spawn_mirror().expect("a mirror");
        assert_ne!(id, world.agent_id);
        assert_eq!(world.spawn_mirror(), Some(id), "the same one again");
        let idx = world.objects.index_of_uuid(&id).expect("drawn");
        let mirror = world.objects.get(idx).expect("mirror");
        assert!(mirror.is_avatar() && mirror.is_client_preview() && mirror.parent_id == 0);
        let worn: Vec<_> = world
            .objects
            .children_of(&mirror.key)
            .iter()
            .filter_map(|&i| world.objects.get(i))
            .map(|o| o.attachment_point())
            .collect();
        assert_eq!(worn, vec![2], "the hat, not the HUD");
        assert_eq!(world.appearance_of(&id), world.agent_id);
        world.remove_mirror();
        assert!(world.objects.index_of_uuid(&id).is_none());
        assert_eq!(world.objects.avatars().count(), 1);
        assert!(world.mirror().is_none());
    }
}
