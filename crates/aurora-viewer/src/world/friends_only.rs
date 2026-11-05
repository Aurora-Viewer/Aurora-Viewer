//! "Show Friends Only" (toolbar command RFO, World menu, FSRenderFriendsOnly
//! and FSRenderFriendsOnlyPersistsTP, both per account and off by default).
//!
//! Port of LLViewerObjectList::isNonFriendDerendered (the update of an avatar
//! that is neither us nor a friend is dropped,
//! indra/newview/llviewerobjectlist.cpp), of handleRenderFriendsOnlyChanged
//! (switching it on kills every such avatar present,
//! indra/newview/llviewercontrol.cpp) and of the teleport arrival of
//! processAgentMovementComplete (it is switched off unless it persists,
//! indra/newview/llviewermessage.cpp), originally LGPL 2.1.
//!
//! It rides on the derender machinery (`derender.rs`): the avatars hidden, and
//! what they wear, are kept to be put back. Control avatars (animesh) are
//! volumes, not avatars, and stay.
//!
//! Deliberate differences from the Legacy Viewer, whose tooltip says that a
//! teleport is required to see the others again: switched off, the hidden
//! avatars come back at once, like an entry leaving the derender list; a
//! friendship formed while it is on brings that avatar back at once, and one
//! lost hides it at once (the Legacy Viewer waits for its next update).

use super::World;
use serde::{Deserialize, Serialize};

/// The two settings of an account, kept in the settings file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    /// FSRenderFriendsOnly.
    pub enabled: bool,
    /// FSRenderFriendsOnlyPersistsTP.
    pub keep_after_teleport: bool,
}

/// What the world keeps beside the derender list.
#[derive(Debug, Default)]
pub struct State {
    /// FSRenderFriendsOnlyPersistsTP of the account.
    pub keep_after_teleport: bool,
    /// The friends the last pass saw (their count and an id fold).
    seen: (usize, u128),
    /// The login rebuilt the derender list: the app loads the account's
    /// choice again.
    pub reload: bool,
}

impl World {
    /// Whether this avatar's update is dropped (isNonFriendDerendered).
    pub(super) fn hides_non_friend(&self, u: &aurora_net::objects::ObjectUpdate) -> bool {
        self.derender.friends_only() && u.is_avatar() && u.full_id != self.agent_id && !self.social.is_friend(&u.full_id)
    }

    pub fn friends_only(&self) -> bool {
        self.derender.friends_only()
    }

    fn friends_signature(&self) -> (usize, u128) {
        let fold = self.social.friends.iter().fold(0u128, |a, f| a ^ f.id.as_u128());
        (self.social.friends.len(), fold)
    }

    /// handleRenderFriendsOnlyChanged: on takes out every avatar present
    /// that is neither us nor a friend; off puts back the ones it hid.
    pub fn set_friends_only(&mut self, on: bool) {
        if on == self.derender.friends_only() {
            return;
        }
        if on {
            self.derender.begin_friends_only();
            self.take_out_non_friends();
        } else {
            let restore = self.derender.end_friends_only();
            self.put_back(restore);
        }
        self.friends_only.seen = self.friends_signature();
    }

    /// Objects restored without passing the update filter (a region back
    /// from parking): the non-friends among them leave again.
    pub(super) fn friends_only_filter_present(&mut self) {
        if self.derender.friends_only() {
            self.take_out_non_friends();
        }
    }

    /// Avatars present that are neither us nor friends leave the world.
    fn take_out_non_friends(&mut self) {
        let me = self.agent_id;
        let strangers: Vec<_> = self
            .objects
            .iter()
            .filter(|(_, o)| o.is_avatar() && o.full_id != me && !self.social.is_friend(&o.full_id))
            .map(|(_, o)| o.full_id)
            .collect();
        for id in strangers {
            // (an earlier removal may have moved the indices)
            if let Some(idx) = self.objects.index_of_uuid(&id) {
                self.derender.friends_only_hides(id);
                self.take_out(idx, id);
            }
        }
    }

    /// A friendship formed or lost while it is on, seen from the friend list.
    pub fn follow_friends_for_friends_only(&mut self) {
        if !self.derender.friends_only() {
            return;
        }
        let signature = self.friends_signature();
        if signature == self.friends_only.seen {
            return;
        }
        self.friends_only.seen = signature;
        for id in self.derender.friends_only_hidden() {
            if self.social.is_friend(&id) {
                let restore = self.derender.friends_only_spares(id);
                self.put_back(restore);
            }
        }
        self.take_out_non_friends();
    }

    /// processAgentMovementComplete: after a teleport it is switched off
    /// unless the account keeps it.
    pub(super) fn friends_only_teleport_arrival(&mut self) {
        if !self.friends_only.keep_after_teleport {
            self.set_friends_only(false);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::world::derender::Kind;
    use crate::world::derender::tests_support::{demo_world, entry_for_tests};
    use aurora_net::NetEvent;
    use uuid::Uuid;

    fn friend(id: Uuid) -> crate::world::social::Friend {
        crate::world::social::Friend {
            id,
            online: false,
            rights_given: 0,
            rights_has: 0,
        }
    }

    fn present(w: &crate::world::World, id: &Uuid) -> bool {
        w.objects.index_of_uuid(id).is_some()
    }

    #[test]
    fn a_non_friend_and_what_it_wears_leave_while_we_stay() {
        let (mut world, handle) = demo_world();
        let (wolf, hat) = (crate::demo::DEMO_LOUP, crate::demo::wolf_hat());
        assert!(present(&world, &wolf) && present(&world, &hat) && present(&world, &world.agent_id));
        world.set_friends_only(true);
        assert!(!present(&world, &wolf) && !present(&world, &hat));
        assert!(present(&world, &world.agent_id), "we stay");
        // the simulator sends them again: dropped, attachment included
        let resent = world.derender.hidden_updates();
        assert!(resent.len() >= 3);
        let _ = world.apply(NetEvent::ObjectUpdates { handle, objects: resent });
        assert!(!present(&world, &wolf) && !present(&world, &hat));
        // switched off: back at once, with the request for them
        world.set_friends_only(false);
        assert!(present(&world, &wolf) && present(&world, &hat));
        assert!(!world.derender.take_requests().is_empty());
    }

    #[test]
    fn what_a_hidden_avatar_looks_like_comes_back_with_it() {
        let (mut world, _) = demo_world();
        let wolf = crate::demo::DEMO_LOUP;
        // the wolf's AvatarAppearance (shape and bakes) is held by id
        let look = crate::demo::classic_events(&world.avatar_lib.shape_params)
            .into_iter()
            .nth(1)
            .expect("wolf look");
        let _ = world.apply(look);
        let seen = |w: &crate::world::World| {
            let idx = w.objects.index_of_uuid(&wolf).expect("wolf");
            (
                w.appearance_generation(&wolf),
                w.visual_params(&wolf).map(<[u8]>::to_vec),
                w.bake_texture(idx, 8),
            )
        };
        let before = seen(&world);
        assert!(before.1.is_some() && before.2.is_some(), "the wolf has an appearance");
        world.set_friends_only(true);
        assert!(!present(&world, &wolf));
        assert!(world.visual_params(&wolf).is_some(), "kept while it is hidden");
        world.set_friends_only(false);
        assert_eq!(seen(&world), before, "the same look at once, nothing to wait for");
    }

    #[test]
    fn a_hidden_avatar_comes_back_where_it_is_now() {
        use aurora_net::objects::TerseUpdate;
        let (mut world, handle) = demo_world();
        let wolf = crate::demo::DEMO_LOUP;
        let key = world
            .objects
            .get(world.objects.index_of_uuid(&wolf).expect("wolf"))
            .expect("wolf")
            .key;
        world.set_friends_only(true);
        let there = glam::Vec3::new(40.0, 50.0, 30.0);
        let _ = world.apply(NetEvent::TerseUpdates {
            handle,
            updates: vec![TerseUpdate {
                local_id: key.local_id,
                state: 0,
                is_avatar: true,
                foot_plane: None,
                position: there,
                velocity: glam::Vec3::X,
                acceleration: glam::Vec3::ZERO,
                rotation: glam::Quat::IDENTITY,
                angular_velocity: glam::Vec3::ZERO,
                texture_entry: None,
            }],
        });
        world.set_friends_only(false);
        let o = world.objects.get(world.objects.index_of_uuid(&wolf).expect("back")).expect("back");
        assert_eq!((o.position, o.velocity), (there, glam::Vec3::X));
    }

    #[test]
    fn a_killed_avatar_is_forgotten_and_its_history_ends() {
        let (mut world, handle) = demo_world();
        let wolf = crate::demo::DEMO_LOUP;
        let key = world
            .objects
            .get(world.objects.index_of_uuid(&wolf).expect("wolf"))
            .expect("wolf")
            .key;
        world.set_friends_only(true);
        assert!(world.derender.hides_resident(&wolf), "kept while it is away");
        let _ = world.apply(NetEvent::ObjectsKilled {
            handle,
            local_ids: vec![key.local_id],
        });
        assert!(!world.derender.hides_resident(&wolf), "a departure ends it");
        // the region leaving does the same
        let (mut world, handle) = demo_world();
        world.set_friends_only(true);
        assert!(world.derender.hides_resident(&wolf));
        let _ = world.apply(NetEvent::RegionRemoved { handle });
        assert!(!world.derender.hides_resident(&wolf));
    }

    #[test]
    fn an_avatar_crossing_regions_comes_back_when_it_is_switched_off() {
        let (mut world, old_region) = demo_world();
        let wolf = crate::demo::DEMO_LOUP;
        let old_key = world
            .objects
            .get(world.objects.index_of_uuid(&wolf).expect("wolf"))
            .expect("wolf")
            .key;
        world.set_friends_only(true);
        // the next region sends the wolf before the old one kills it
        let next_region = old_region + (256u64 << 32);
        world.ensure_region(next_region, (256, 256));
        let mut crossed = world
            .derender
            .hidden_updates()
            .into_iter()
            .find(|u| u.full_id == wolf)
            .expect("wolf snapshot");
        crossed.local_id = 4242;
        let _ = world.apply(NetEvent::ObjectUpdates {
            handle: next_region,
            objects: vec![crossed],
        });
        let _ = world.apply(NetEvent::ObjectsKilled {
            handle: old_region,
            local_ids: vec![old_key.local_id],
        });
        assert!(!present(&world, &wolf));
        assert!(world.derender.hides_resident(&wolf), "still held under the next region's key");
        world.set_friends_only(false);
        let back = world.objects.get(world.objects.index_of_uuid(&wolf).expect("back")).expect("back");
        assert_eq!(back.key.region, next_region);
    }

    #[test]
    fn a_friend_stays_when_it_is_switched_on() {
        let (mut world, _) = demo_world();
        let wolf = crate::demo::DEMO_LOUP;
        world.social.friends.push(friend(wolf));
        world.set_friends_only(true);
        assert!(present(&world, &wolf));
        assert!(present(&world, &crate::demo::wolf_hat()));
    }

    #[test]
    fn a_derendered_avatar_stays_hidden_when_it_is_switched_off() {
        let (mut world, _) = demo_world();
        let wolf = crate::demo::DEMO_LOUP;
        world.set_friends_only(true);
        world.derender(wolf, entry_for_tests(Kind::Resident, false));
        world.set_friends_only(false);
        assert!(!present(&world, &wolf), "its entry still hides it");
        world.underender(&[wolf]);
        assert!(present(&world, &wolf));
        // the other way round: an entry removed while it is on
        world.derender(wolf, entry_for_tests(Kind::Resident, false));
        world.set_friends_only(true);
        world.underender(&[wolf]);
        assert!(!present(&world, &wolf), "it still hides it");
        world.set_friends_only(false);
        assert!(present(&world, &wolf));
    }

    #[test]
    fn a_new_friend_comes_back_and_a_lost_one_goes() {
        let (mut world, _) = demo_world();
        let (wolf, hat) = (crate::demo::DEMO_LOUP, crate::demo::wolf_hat());
        world.set_friends_only(true);
        world.social.friends.push(friend(wolf));
        world.follow_friends_for_friends_only();
        assert!(present(&world, &wolf) && present(&world, &hat));
        world.social.end_friendship(&wolf);
        world.follow_friends_for_friends_only();
        assert!(!present(&world, &wolf) && !present(&world, &hat));
    }

    #[test]
    fn a_teleport_switches_it_off_unless_it_is_kept() {
        let (mut world, _) = demo_world();
        let wolf = crate::demo::DEMO_LOUP;
        world.set_friends_only(true);
        world.friends_only_teleport_arrival();
        assert!(!world.friends_only() && present(&world, &wolf));
        world.friends_only.keep_after_teleport = true;
        world.set_friends_only(true);
        world.friends_only_teleport_arrival();
        assert!(world.friends_only() && !present(&world, &wolf));
    }

    #[test]
    fn nothing_is_hidden_while_it_is_off() {
        let (world, _) = demo_world();
        let (_, o) = world
            .objects
            .iter()
            .find(|(_, o)| o.full_id == crate::demo::DEMO_LOUP)
            .expect("wolf");
        let update = crate::build::replace::demo_update(o);
        assert!(!world.hides_non_friend(&update));
    }
}
