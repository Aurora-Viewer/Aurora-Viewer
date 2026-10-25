//! A map/contact teleport with unknown height gets one local TP To after
//! arrival. This extends FSRadar::teleportToAvatar (indra/newview/fsradar.cpp)
//! without changing received lures, coordinate destinations or explicit Z.

use super::World;
use aurora_net::{NetCommand, RegionHandle};
use glam::Vec3;
use std::time::{Duration, Instant};
use uuid::Uuid;

const ARRIVAL_TIMEOUT: Duration = Duration::from_secs(90);
const POSITION_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) struct FriendJoin {
    avatar: Uuid,
    destination: RegionHandle,
    started: Instant,
    arrived: Option<Instant>,
    fresh_object: bool,
    coarse: Option<Vec3>,
}

impl World {
    fn region_contains_slot(&self, region: RegionHandle, slot: RegionHandle) -> bool {
        let (x, y) = aurora_net::handle_to_origin(region);
        let (sx, sy) = aurora_net::handle_to_origin(slot);
        let size = self
            .regions
            .get(&region)
            .map(|r| (r.heightmap.size_x, r.heightmap.size_y))
            .or_else(|| self.map.sims.get(&(x / 256, y / 256)).map(|s| (s.size_x, s.size_y)))
            .unwrap_or((256, 256));
        sx >= x && sy >= y && sx - x < size.0 && sy - y < size.1
    }

    /// Called only for commands drained from the map/contact action path,
    /// before request_user_teleport. Lures never acquire this intent.
    pub(crate) fn prepare_friend_join(&self, command: &NetCommand) -> Option<FriendJoin> {
        let avatar = self.map.unknown_height_teleport_avatar(command)?;
        let NetCommand::TeleportTo { handle, .. } = command else {
            return None;
        };
        let main = self.main_region?;
        if !self.social.can_map_friend(avatar) || self.region_contains_slot(main, *handle) {
            return None;
        }
        Some(FriendJoin {
            avatar,
            destination: *handle,
            started: Instant::now(),
            arrived: None,
            fresh_object: false,
            coarse: None,
        })
    }

    /// Arm after the ordinary teleport preflight accepted the first leg.
    /// A synchronous demo arrival may already have completed by then.
    pub(crate) fn arm_friend_join(&mut self, mut join: FriendJoin) {
        if self.movement_complete && !self.teleporting && self.main_region.is_some_and(|h| self.region_contains_slot(h, join.destination)) {
            join.arrived = Some(Instant::now());
        }
        self.map.friend_join = Some(join);
    }

    pub(crate) fn cancel_friend_join(&mut self) {
        self.map.friend_join = None;
    }

    pub(super) fn friend_join_arrived(&mut self, handle: RegionHandle) {
        let Some(join) = &self.map.friend_join else { return };
        if self.region_contains_slot(handle, join.destination) {
            if let Some(join) = &mut self.map.friend_join {
                join.arrived.get_or_insert_with(Instant::now);
            }
        } else {
            self.cancel_friend_join();
        }
    }

    fn friend_join_report_matches(&self, handle: RegionHandle, avatar: Uuid) -> bool {
        self.map.friend_join.as_ref().is_some_and(|j| {
            j.avatar == avatar && self.received.is_none_or(|at| at >= j.started) && self.region_contains_slot(handle, j.destination)
        })
    }

    pub(super) fn note_friend_join_object(&mut self, handle: RegionHandle, avatar: Uuid) {
        if self.friend_join_report_matches(handle, avatar)
            && let Some(join) = &mut self.map.friend_join
        {
            join.fresh_object = true;
        }
    }

    pub(super) fn note_friend_join_coarse(&mut self, handle: RegionHandle, positions: &[(Uuid, Vec3)]) {
        let Some(target) = self.map.friend_join.as_ref().map(|j| j.avatar) else {
            return;
        };
        if self.friend_join_report_matches(handle, target)
            && let Some((_, p)) = positions.iter().find(|(id, _)| *id == target)
            && p.is_finite()
            && p.z > 0.0
            && p.z < 1020.0
            && let Some(join) = &mut self.map.friend_join
        {
            // LLWorld::getAvatars treats coarse byte 0/255 as unknown Z.
            join.coarse = Some(*p);
        }
    }

    /// Resolve from new destination-region reports, never a retained radar
    /// entry. Consume before dispatch, so a denial or landing-point redirect
    /// cannot create an automatic teleport loop.
    pub(crate) fn finish_friend_join(&mut self, now: Instant, cancelled: bool) -> Option<NetCommand> {
        let join = self.map.friend_join.as_ref()?;
        let expired = match join.arrived {
            Some(at) => now.saturating_duration_since(at) >= POSITION_TIMEOUT,
            None => now.saturating_duration_since(join.started) >= ARRIVAL_TIMEOUT,
        };
        if cancelled
            || expired
            || self.tp_failed
            || !self.social.can_map_friend(join.avatar)
            || self.map.avatar.as_ref().is_none_or(|a| a.id != join.avatar)
        {
            self.cancel_friend_join();
            return None;
        }
        if join.arrived.is_none() || self.teleporting || !self.movement_complete {
            return None;
        }
        let main = self.main_region?;
        if !self.region_contains_slot(main, join.destination) {
            self.cancel_friend_join();
            return None;
        }
        let position = if join.fresh_object {
            let o = self.objects.get(self.objects.index_of_uuid(&join.avatar)?)?;
            if o.key.region != main || !o.is_avatar() {
                self.cancel_friend_join();
                return None;
            }
            // The navigation resolver also handles seated avatars' parents.
            crate::autopilot::avatar_position(self, join.avatar)?
        } else {
            join.coarse?
        };
        if !position.is_finite() {
            return None;
        }
        let avatar = join.avatar;
        self.cancel_friend_join();
        let (x, y) = aurora_net::handle_to_origin(main);
        self.map
            .agent_found(avatar, x as f64 + position.x as f64, y as f64 + position.y as f64, Some(position.z));
        Some(super::worldmap::teleport_command(
            x as f64 + position.x as f64,
            y as f64 + position.y as f64,
            position.z,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::{
        social::{Friend, rights},
        worldmap::{SimInfo, teleport_command},
    };
    use aurora_net::NetEvent;

    fn destination() -> RegionHandle {
        aurora_net::origin_to_handle(256_256, 256_000)
    }
    fn friend() -> Uuid {
        Uuid::from_u128(7)
    }

    fn first_leg(contact: bool) -> World {
        let mut w = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        w.agent_id = Uuid::from_u128(1);
        w.main_region = Some(aurora_net::origin_to_handle(256_000, 256_000));
        w.social.friends.push(Friend {
            id: friend(),
            online: true,
            rights_given: 0,
            rights_has: rights::MAP_LOCATION,
        });
        w.map.sims.insert(
            (1001, 1000),
            SimInfo {
                name: "Remote demo".into(),
                access: 13,
                flags: 0,
                image: Uuid::nil(),
                size_x: 256,
                size_y: 256,
            },
        );
        assert!(w.locate_avatar(friend(), contact));
        w.apply(NetEvent::AgentFound {
            prey: friend(),
            x: 256_316.0,
            y: 256_200.0,
        });
        if !contact {
            let t = w.map.track.as_ref().unwrap();
            w.map.push(teleport_command(t.x, t.y, t.z));
        }
        let command = w
            .take_map_commands()
            .into_iter()
            .find(|c| matches!(c, NetCommand::TeleportTo { .. }))
            .unwrap();
        let join = w.prepare_friend_join(&command).expect("unknown-height map/contact teleport");
        w.begin_teleport(true);
        w.arm_friend_join(join);
        w
    }

    fn arrive(w: &mut World, handle: RegionHandle) {
        w.apply(NetEvent::AgentMovementComplete {
            handle,
            position: Vec3::new(60.0, 200.0, 0.0),
            look_at: Vec3::X,
        });
    }

    fn coarse(w: &mut World, z: f32) {
        w.apply(NetEvent::CoarseLocations {
            handle: destination(),
            positions: vec![(friend(), Vec3::new(60.0, 200.0, z))],
        });
    }

    fn avatar(w: &mut World, z: f32) {
        let mut o = crate::demo::action_avatar(false);
        o.full_id = friend();
        o.position = Vec3::new(60.0, 200.0, z);
        w.apply(NetEvent::ObjectUpdates {
            handle: destination(),
            objects: vec![o],
        });
    }

    #[test]
    fn map_and_contact_join_wait_for_arrival_then_teleport_locally_once() {
        for contact in [false, true] {
            let mut w = first_leg(contact);
            coarse(&mut w, 20.0);
            assert!(w.finish_friend_join(Instant::now(), false).is_none(), "still travelling");
            arrive(&mut w, destination());
            let command = w.finish_friend_join(Instant::now(), false).expect("one local TP To");
            assert!(
                matches!(command, NetCommand::TeleportTo { handle, position, .. } if handle == destination() && position == Vec3::new(60.0, 200.0, 20.0))
            );
            coarse(&mut w, 24.0);
            assert!(w.finish_friend_join(Instant::now(), false).is_none());
        }
    }

    #[test]
    fn friend_join_waits_for_delayed_high_avatar_instead_of_coarse_unknown_height() {
        let mut w = first_leg(true);
        arrive(&mut w, destination());
        for z in [0.0, 1020.0] {
            coarse(&mut w, z);
            assert!(w.finish_friend_join(Instant::now(), false).is_none());
        }
        avatar(&mut w, 3000.0);
        coarse(&mut w, 1020.0);
        assert!(
            matches!(w.finish_friend_join(Instant::now(), false), Some(NetCommand::TeleportTo { position, .. }) if position.z == 3000.0)
        );
    }

    #[test]
    fn friend_join_ignores_old_cache_and_packets_received_before_the_request() {
        let mut w = first_leg(true);
        // Cached data that was never observed by the active join cannot be used.
        let mut old = crate::demo::action_avatar(false);
        old.full_id = friend();
        old.position = Vec3::new(60.0, 200.0, 2500.0);
        w.objects.upsert_at(destination(), old.clone(), None);
        w.coarse.insert(destination(), vec![(friend(), Vec3::new(60.0, 200.0, 2500.0))]);
        arrive(&mut w, destination());
        assert!(w.finish_friend_join(Instant::now(), false).is_none());
        let received = Instant::now() - Duration::from_secs(10);
        w.apply_received(
            NetEvent::CoarseLocations {
                handle: destination(),
                positions: vec![(friend(), Vec3::new(60.0, 200.0, 20.0))],
            },
            received,
        );
        w.apply_received(
            NetEvent::ObjectUpdates {
                handle: destination(),
                objects: vec![old],
            },
            received,
        );
        assert!(w.finish_friend_join(Instant::now(), false).is_none());
        avatar(&mut w, 20.0);
        assert!(matches!(w.finish_friend_join(Instant::now(), false), Some(NetCommand::TeleportTo { position, .. }) if position.z == 20.0));
    }

    #[test]
    fn friend_join_cancels_on_user_actions_failure_revocation_and_timeout() {
        for cancel in 0..9 {
            let mut w = first_leg(true);
            arrive(&mut w, destination());
            coarse(&mut w, 20.0);
            let mut now = Instant::now();
            match cancel {
                0 => w.social.friends[0].rights_has = 0,
                1 => w.social.friends[0].online = false,
                2 => {
                    w.apply(NetEvent::TeleportFailed {
                        reason: "Offline test".into(),
                    });
                }
                3 => w.begin_teleport(false), // includes accepting a lure
                4 => w.map.clear_track(),
                5 => w.map.set_track_altitude(1000.0),
                6 => now += POSITION_TIMEOUT,
                7 => {
                    w.apply(NetEvent::TeleportStarted);
                }
                _ => {}
            }
            assert!(w.finish_friend_join(now, cancel == 8).is_none(), "cancellation {cancel}");
            assert!(w.map.friend_join.is_none(), "consume cancellation {cancel}");
        }
    }

    #[test]
    fn friend_join_is_never_armed_for_lures_coordinates_local_tp_or_explicit_height() {
        let mut w = first_leg(false);
        assert!(
            w.prepare_friend_join(&NetCommand::AcceptLure {
                lure_id: Uuid::from_u128(99),
                godlike: false
            })
            .is_none()
        );
        assert!(w.prepare_friend_join(&NetCommand::TeleportHome).is_none());
        assert!(w.prepare_friend_join(&teleport_command(256_320.0, 256_200.0, 0.0)).is_none());
        w.map.set_track_altitude(2000.0);
        assert!(w.prepare_friend_join(&teleport_command(256_316.0, 256_200.0, 2000.0)).is_none());
        w.map.clear_track();
        w.map.track_avatar(friend(), "Remote demo");
        w.map.agent_found(friend(), 256_316.0, 256_200.0, Some(20.0));
        assert!(w.prepare_friend_join(&teleport_command(256_316.0, 256_200.0, 20.0)).is_none());
        w.map.agent_found(friend(), 256_316.0, 256_200.0, None);
        w.main_region = Some(destination());
        assert!(w.prepare_friend_join(&teleport_command(256_316.0, 256_200.0, 0.0)).is_none());
        let base = aurora_net::origin_to_handle(256_000, 256_000);
        w.main_region = Some(base);
        w.map.sims.insert(
            (1000, 1000),
            SimInfo {
                name: "Large demo".into(),
                access: 13,
                flags: 0,
                image: Uuid::nil(),
                size_x: 512,
                size_y: 512,
            },
        );
        assert!(
            w.prepare_friend_join(&teleport_command(256_316.0, 256_200.0, 0.0)).is_none(),
            "a slot inside the same variable region is local"
        );
    }

    #[test]
    fn friend_join_rejects_other_arrivals_and_expires_before_arrival() {
        let mut w = first_leg(true);
        let now = Instant::now() + ARRIVAL_TIMEOUT;
        assert!(w.finish_friend_join(now, false).is_none());
        assert!(w.map.friend_join.is_none());
        let mut w = first_leg(true);
        arrive(&mut w, aurora_net::origin_to_handle(300_032, 300_032));
        coarse(&mut w, 20.0);
        assert!(w.finish_friend_join(Instant::now(), false).is_none());
        assert!(w.map.friend_join.is_none());
    }

    #[test]
    fn accepting_another_teleport_also_cancels_a_pending_initial_friend_lookup() {
        let mut w = first_leg(true);
        w.map.teleport_to_avatar(friend(), "Remote demo");
        w.take_map_commands();
        // The same begin_teleport path is used by AcceptLure.
        w.begin_teleport(false);
        w.apply(NetEvent::AgentFound {
            prey: friend(),
            x: 256_316.0,
            y: 256_200.0,
        });
        assert!(w.take_map_commands().iter().all(|c| !matches!(c, NetCommand::TeleportTo { .. })));
        assert!(w.finish_friend_join(Instant::now(), false).is_none());
    }
}
