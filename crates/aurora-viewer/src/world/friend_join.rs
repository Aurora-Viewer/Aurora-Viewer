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
const FLIGHT_TIMEOUT: Duration = Duration::from_secs(5);
const HOVER_OBSERVATION: Duration = Duration::from_millis(300);

pub(crate) struct FriendJoin {
    avatar: Uuid,
    destination: RegionHandle,
    started: Instant,
    arrived: Option<Instant>,
    fresh_object: bool,
    fresh_physics: bool,
    flight: Option<bool>,
    coarse: Option<Vec3>,
}

pub(crate) struct FriendJoinTeleport {
    pub command: NetCommand,
    pub avatar: Uuid,
    pub fresh_physics: bool,
    pub fly_on_arrival: bool,
}

pub(super) struct FriendJoinFlight {
    avatar: Uuid,
    region: RegionHandle,
    started: Instant,
    was_flying: bool,
    wants_flight: bool,
    arrival_flight: bool,
    fresh_physics: bool,
    hover_since: Option<Instant>,
    arrived: Option<Instant>,
    waiting_parcel: bool,
}

impl World {
    pub(crate) fn friend_teleport_avatar(&self, command: &NetCommand) -> Option<Uuid> {
        self.map
            .friend_teleport_avatar(command)
            .filter(|id| self.social.can_map_friend(*id))
    }

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
            fresh_physics: false,
            flight: None,
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
        self.cancel_friend_join_flight();
    }

    /// Arm before sending the local hop, including a synchronous demo reply.
    /// Taking off must wait until its position has actually been applied.
    pub(crate) fn defer_friend_join_flight(&mut self, avatar: Uuid, fly: bool, fresh_physics: bool) {
        if let Some(region) = self.main_region {
            self.defer_friend_join_flight_at(avatar, fly, fresh_physics, region);
        }
    }

    pub(crate) fn defer_friend_join_flight_at(&mut self, avatar: Uuid, fly: bool, fresh_physics: bool, region: RegionHandle) {
        self.map.friend_join_flight = Some(FriendJoinFlight {
            avatar,
            region,
            started: Instant::now(),
            was_flying: self.agent.flying,
            wants_flight: fly,
            arrival_flight: false,
            fresh_physics,
            hover_since: None,
            arrived: None,
            waiting_parcel: false,
        });
        log::info!(
            "friend join: flight observation armed (requested {fly}, already flying {})",
            self.agent.flying
        );
        self.log_friend_join_physics();
    }

    pub(crate) fn cancel_friend_join_flight(&mut self) {
        self.map.friend_join_flight = None;
    }

    pub(super) fn friend_join_local_flight(&mut self, flying: bool) -> Option<bool> {
        let flight = self.map.friend_join_flight.as_mut()?;
        if self.received.is_some_and(|at| at < flight.started) || flight.started.elapsed() >= ARRIVAL_TIMEOUT {
            self.cancel_friend_join_flight();
            return None;
        }
        flight.arrival_flight = flying || self.fly_after_teleport;
        flight.wants_flight |= flight.arrival_flight;
        Some(flight.was_flying && self.agent.flying && !self.rlva.has("fly"))
    }

    pub(super) fn friend_join_flight_arrived(&mut self, handle: RegionHandle) {
        let matching_region = self
            .map
            .friend_join_flight
            .as_ref()
            .is_some_and(|f| self.region_contains_slot(handle, f.region));
        let Some(flight) = &mut self.map.friend_join_flight else { return };
        if matching_region && self.received.is_none_or(|at| at >= flight.started) && flight.started.elapsed() < ARRIVAL_TIMEOUT {
            flight.region = handle;
            flight.arrived.get_or_insert_with(Instant::now);
            log::info!("friend join: arrival confirmed for flight observation");
        } else {
            self.cancel_friend_join_flight();
        }
    }

    /// Observe briefly after arrival too: the target's flight animation and
    /// the landing parcel can arrive after TeleportLocal. Never consume a
    /// negative early snapshot as the final answer, or reuse another parcel.
    pub(crate) fn take_friend_join_flight(&mut self) -> bool {
        let Some(flight) = &self.map.friend_join_flight else { return false };
        let Some(arrived) = flight.arrived else { return false };
        if arrived.elapsed() >= FLIGHT_TIMEOUT
            || self.main_region != Some(flight.region)
            || (flight.was_flying && !self.agent.flying)
            || !self.social.can_map_friend(flight.avatar)
        {
            log::info!("friend join: flight observation expired or superseded");
            self.log_friend_join_physics();
            self.cancel_friend_join_flight();
            return false;
        }
        let inferred_hover = !flight.wants_flight;
        if inferred_hover {
            let hovering = self.friend_join_hover_candidate(flight);
            let Some(flight) = &mut self.map.friend_join_flight else {
                return false;
            };
            if !hovering {
                flight.hover_since = None;
                return false;
            }
            if flight.hover_since.get_or_insert_with(Instant::now).elapsed() < HOVER_OBSERVATION {
                return false;
            }
        }
        let Some(flight) = &self.map.friend_join_flight else { return false };
        if !flight.was_flying
            && self
                .objects
                .index_of_uuid(&flight.avatar)
                .and_then(|i| self.objects.get(i))
                .is_some_and(|o| o.parent_id != 0)
        {
            self.cancel_friend_join_flight();
            return false;
        }
        let parcel_is_current = self.parcel.as_ref().is_none_or(|p| {
            p.bitmap.is_empty()
                || self.main_origin().is_some_and(|origin| {
                    let position = self.agent.position.as_dvec3() + glam::DVec3::new(origin.0 as f64, origin.1 as f64, 0.0);
                    super::landmarks::in_parcel(&p.bitmap, origin, position)
                })
        });
        if !parcel_is_current {
            if let Some(flight) = &mut self.map.friend_join_flight
                && !flight.waiting_parcel
            {
                flight.waiting_parcel = true;
                log::info!("friend join: waiting for the landing parcel before flight");
            }
            return false;
        }
        self.cancel_friend_join_flight();
        let allowed = crate::autopilot::can_fly_for_join(self);
        if !allowed {
            log::info!("friend join: flight blocked by current landing policy");
        } else if inferred_hover {
            log::info!("friend join: stable unsupported hover without a native flight signal");
        }
        allowed
    }

    /// AO overrides can remove the native hover signal (AOEngine::enable,
    /// indra/newview/aoengine.cpp). LLWorld::resolveStepHeightGlobal
    /// (indra/newview/llworld.cpp) uses
    /// the foot plane as a possible support height, not as a contact flag:
    /// the last ground plane can remain far below a flying avatar.
    fn friend_join_hover_candidate(&self, flight: &FriendJoinFlight) -> bool {
        if !flight.fresh_physics {
            return false;
        }
        let Some(o) = self.objects.index_of_uuid(&flight.avatar).and_then(|i| self.objects.get(i)) else {
            return false;
        };
        if o.key.region != flight.region
            || o.parent_id != 0
            || o.foot_plane.is_none_or(|plane| !plane.is_finite())
            || !o.velocity.is_finite()
            || o.velocity.z.abs() > 0.25
            || !o.acceleration.is_finite()
            || o.acceleration.z.abs() > 0.5
            || !o.scale.z.is_finite()
            || o.scale.z <= 0.0
        {
            return false;
        }
        let mut signals = self.animations.get(&flight.avatar).into_iter().flatten().map(|a| a.id);
        if crate::autopilot::flight_from_signals(false, signals.clone()).is_some()
            || signals.any(|id| {
                matches!(
                    crate::ao::State::from_motion(id),
                    Some(crate::ao::State::Jumping | crate::ao::State::PreJumping | crate::ao::State::Falling)
                )
            })
        {
            return false;
        }
        crate::autopilot::avatar_target(self, flight.avatar, o.scale.z).is_some_and(|target| target.flight.flying.is_none())
    }

    fn log_friend_join_physics(&self) {
        let Some(flight) = &self.map.friend_join_flight else { return };
        if let Some(o) = self.objects.index_of_uuid(&flight.avatar).and_then(|i| self.objects.get(i)) {
            let observed = crate::autopilot::avatar_target(self, flight.avatar, o.scale.z).map(|target| target.flight.flying);
            log::info!(
                "friend join: target physics fresh {}, same region {}, seated {}, height {:.2}, scale {:.2}, plane {:?}, vz {:.3}, az {:.3}, observed flight {:?}",
                flight.fresh_physics,
                o.key.region == flight.region,
                o.parent_id != 0,
                o.position.z,
                o.scale.z,
                o.foot_plane,
                o.velocity.z,
                o.acceleration.z,
                observed
            );
        } else {
            log::info!("friend join: target physics not loaded");
        }
    }

    pub(super) fn note_friend_join_animations(&mut self, avatar: Uuid) {
        if !self.map.friend_join.as_ref().is_some_and(|j| j.avatar == avatar)
            && !self.map.friend_join_flight.as_ref().is_some_and(|f| f.avatar == avatar)
        {
            return;
        }
        let flying = crate::autopilot::flight_from_signals(false, self.animations.get(&avatar).into_iter().flatten().map(|a| a.id));
        let jumping_or_falling = self.animations.get(&avatar).into_iter().flatten().any(|a| {
            matches!(
                crate::ao::State::from_motion(a.id),
                Some(crate::ao::State::Jumping | crate::ao::State::PreJumping | crate::ao::State::Falling)
            )
        });
        if let Some(join) = &mut self.map.friend_join
            && join.avatar == avatar
            && self.received.is_none_or(|at| at >= join.started)
        {
            join.flight = flying;
        }
        if let Some(flight) = &mut self.map.friend_join_flight
            && flight.avatar == avatar
            && self.received.is_none_or(|at| at >= flight.started)
        {
            if let Some(flying) = flying {
                flight.wants_flight = flight.was_flying || flight.arrival_flight || flying;
            } else if jumping_or_falling {
                flight.wants_flight = flight.was_flying || flight.arrival_flight;
            }
            log::info!("friend join: target flight signal {flying:?}");
        }
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

    pub(super) fn note_friend_join_object(&mut self, handle: RegionHandle, avatar: Uuid, foot_plane: Option<glam::Vec4>) {
        if self.friend_join_report_matches(handle, avatar)
            && let Some(join) = &mut self.map.friend_join
        {
            join.fresh_object = true;
            join.fresh_physics |= foot_plane.is_some();
        }
        if let Some(flight) = &mut self.map.friend_join_flight
            && flight.avatar == avatar
            && flight.region == handle
            && self.received.is_none_or(|at| at >= flight.started)
        {
            flight.fresh_physics |= foot_plane.is_some();
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
    pub(crate) fn finish_friend_join(&mut self, now: Instant, cancelled: bool) -> Option<FriendJoinTeleport> {
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
        let seated = join.fresh_object
            && self
                .objects
                .index_of_uuid(&join.avatar)
                .and_then(|i| self.objects.get(i))
                .is_some_and(|o| o.parent_id != 0);
        let fly_on_arrival = self.agent.flying || (!seated && join.flight == Some(true));
        let avatar = join.avatar;
        let fresh_physics = join.fresh_physics;
        log::info!(
            "friend join: local hop (target flight {:?}, already flying {})",
            join.flight,
            self.agent.flying
        );
        self.cancel_friend_join();
        let (x, y) = aurora_net::handle_to_origin(main);
        self.map
            .agent_found(avatar, x as f64 + position.x as f64, y as f64 + position.y as f64, Some(position.z));
        Some(FriendJoinTeleport {
            command: super::worldmap::teleport_command(x as f64 + position.x as f64, y as f64 + position.y as f64, position.z),
            avatar,
            fresh_physics,
            fly_on_arrival,
        })
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
        signals(w, crate::ao::State::Standing);
    }

    fn avatar(w: &mut World, z: f32) {
        avatar_data(w, z);
        signals(w, crate::ao::State::Standing);
    }

    fn signals(w: &mut World, state: crate::ao::State) {
        w.apply(NetEvent::AvatarAnimations {
            avatar: friend(),
            anims: vec![(state.motion(), 1)],
            sources: vec![],
        });
    }

    fn avatar_data(w: &mut World, z: f32) {
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
            let hop = w.finish_friend_join(Instant::now(), false).expect("one local TP To");
            assert!(!hop.fly_on_arrival, "a standing friend does not turn on flight");
            let command = hop.command;
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
            matches!(w.finish_friend_join(Instant::now(), false).map(|hop| hop.command), Some(NetCommand::TeleportTo { position, .. }) if position.z == 3000.0)
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
        assert!(
            matches!(w.finish_friend_join(Instant::now(), false).map(|hop| hop.command), Some(NetCommand::TeleportTo { position, .. }) if position.z == 20.0)
        );
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

    fn allow_flight(w: &mut World) {
        let mut info = crate::demo::events()
            .into_iter()
            .find_map(|e| match e {
                NetEvent::RegionHandshake(info) => Some((*info).clone()),
                _ => None,
            })
            .unwrap();
        info.handle = destination();
        info.is_main = true;
        info.region_flags &= !aurora_net::region_flags::BLOCK_FLY;
        w.apply(NetEvent::RegionHandshake(std::sync::Arc::new(info)));
        w.parcel = Some(std::sync::Arc::new(aurora_net::ParcelInfo {
            flags: aurora_net::parcel_flags::ALLOW_FLY,
            ..Default::default()
        }));
    }

    #[test]
    fn friend_join_hops_before_flight_signals_and_accepts_them_after_arrival() {
        for known_policy in [false, true] {
            for initial in [None, Some(crate::ao::State::Standing), Some(crate::ao::State::Typing)] {
                let mut w = first_leg(true);
                arrive(&mut w, destination());
                avatar_data(&mut w, 2400.0);
                if let Some(initial) = initial {
                    signals(&mut w, initial);
                }
                let hop = w
                    .finish_friend_join(Instant::now(), false)
                    .expect("position is enough for the local hop");
                assert!(!hop.fly_on_arrival);
                assert!(matches!(hop.command, NetCommand::TeleportTo { position, .. } if position.z == 2400.0));
                w.begin_teleport(false);
                w.defer_friend_join_flight(hop.avatar, hop.fly_on_arrival, hop.fresh_physics);
                w.apply_received(
                    NetEvent::AvatarAnimations {
                        avatar: friend(),
                        anims: vec![(crate::ao::State::Hovering.motion(), 1)],
                        sources: vec![],
                    },
                    Instant::now() - Duration::from_secs(10),
                );
                local_arrival(&mut w, false);
                assert!(!w.take_friend_join_flight(), "old flight state is not enough");
                assert!(!w.agent.flying && w.agent.position.z == 2400.0);
                w.apply(NetEvent::AvatarAnimations {
                    avatar: Uuid::from_u128(8),
                    anims: vec![(crate::ao::State::Flying.motion(), 1)],
                    sources: vec![],
                });
                assert!(!w.take_friend_join_flight(), "another avatar's flight does not enable ours");
                if known_policy {
                    allow_flight(&mut w);
                }
                signals(&mut w, crate::ao::State::Hovering);
                assert!(
                    w.take_friend_join_flight(),
                    "a new flight report after arrival still enables flight"
                );
                assert!(!w.agent.flying, "takeoff is deferred until the local hop arrives");
            }
        }
    }

    #[test]
    fn friend_join_waits_for_the_landing_parcel_instead_of_consuming_departure_policy() {
        for allow in [false, true] {
            let mut w = first_leg(true);
            arrive(&mut w, destination());
            allow_flight(&mut w);
            let parcel = |x: usize, y: usize, allow: bool| {
                let mut bitmap = vec![0; 512];
                let cell = y / 4 * 64 + x / 4;
                bitmap[cell / 8] |= 1 << (cell % 8);
                std::sync::Arc::new(aurora_net::ParcelInfo {
                    bitmap,
                    flags: if allow { aurora_net::parcel_flags::ALLOW_FLY } else { 0 },
                    ..Default::default()
                })
            };
            w.parcel = Some(parcel(100, 100, !allow));
            w.begin_teleport(false);
            w.defer_friend_join_flight(friend(), true, true);
            local_arrival(&mut w, false);
            assert!(!w.take_friend_join_flight(), "the old parcel cannot approve or deny this landing");
            assert!(w.map.friend_join_flight.is_some());
            w.apply(NetEvent::AgentParcel(parcel(60, 200, allow)));
            assert_eq!(w.take_friend_join_flight(), allow);
            assert!(w.map.friend_join_flight.is_none());
        }
    }

    #[test]
    fn friend_join_late_flight_observation_is_bounded_and_standing_never_takes_off() {
        for standing in [false, true] {
            let mut w = first_leg(true);
            arrive(&mut w, destination());
            allow_flight(&mut w);
            w.begin_teleport(false);
            w.defer_friend_join_flight(friend(), false, true);
            local_arrival(&mut w, false);
            if standing {
                signals(&mut w, crate::ao::State::Standing);
            }
            assert!(!w.take_friend_join_flight() && !w.agent.flying);
            w.map.friend_join_flight.as_mut().unwrap().arrived = Some(Instant::now() - FLIGHT_TIMEOUT);
            arrive(&mut w, destination());
            assert!(!w.take_friend_join_flight());
            signals(&mut w, crate::ao::State::Flying);
            assert!(!w.take_friend_join_flight(), "later flight is outside the accepted join");
        }
    }

    #[test]
    fn friend_join_late_flight_respects_stop_map_edits_other_teleports_and_rights_loss() {
        for cancel in 0..6 {
            let mut w = first_leg(true);
            arrive(&mut w, destination());
            allow_flight(&mut w);
            w.begin_teleport(false);
            w.defer_friend_join_flight(friend(), false, true);
            local_arrival(&mut w, false);
            match cancel {
                0 => w.cancel_friend_join(),
                1 => w.map.set_track_altitude(50.0),
                2 => w.map.track_avatar(Uuid::from_u128(8), "Other demo friend"),
                3 => w.begin_teleport(false),
                4 => w.social.friends[0].rights_has = 0,
                _ => w.cancel_friend_join_flight(),
            }
            signals(&mut w, crate::ao::State::Hovering);
            assert!(!w.take_friend_join_flight() && !w.agent.flying, "cancellation {cancel}");
        }
    }

    #[test]
    fn friend_join_custom_animation_needs_a_fresh_sustained_unsupported_hover() {
        for case in 0..15 {
            let mut w = first_leg(true);
            arrive(&mut w, destination());
            allow_flight(&mut w);
            avatar_data(&mut w, 2400.0);
            signals(&mut w, crate::ao::State::Typing);
            let idx = w.objects.index_of_uuid(&friend()).unwrap();
            let o = w.objects.get_mut(idx).unwrap();
            o.foot_plane = Some(glam::Vec4::ZERO);
            o.velocity = Vec3::ZERO;
            o.acceleration = Vec3::ZERO;
            o.scale.z = 1.8;
            match case {
                1 => o.foot_plane = Some(glam::Vec4::new(0.0, 0.0, 1.0, 2399.1)),
                2 => o.foot_plane = None,
                3 => o.velocity.z = -3.0,
                4 => o.acceleration.z = -9.8,
                7 => o.parent_id = 99,
                9 => o.position.z = 0.9,
                12 => o.foot_plane = Some(glam::Vec4::new(0.0, 0.0, 1.0, 20.0)),
                13 => o.foot_plane = Some(glam::Vec4::new(0.0, 0.0, 2.0, 40.0)),
                14 => o.foot_plane = Some(glam::Vec4::splat(f32::NAN)),
                _ => {}
            }
            if case == 6 {
                signals(&mut w, crate::ao::State::Standing);
            }
            w.begin_teleport(false);
            w.defer_friend_join_flight(friend(), matches!(case, 5 | 10 | 11), case != 8);
            match case {
                5 => signals(&mut w, crate::ao::State::Falling),
                10 => signals(&mut w, crate::ao::State::Jumping),
                11 => signals(&mut w, crate::ao::State::PreJumping),
                _ => {}
            }
            local_arrival(&mut w, false);
            assert!(!w.take_friend_join_flight(), "a single observation must never start flight");
            if let Some(f) = &mut w.map.friend_join_flight
                && f.hover_since.is_some()
            {
                f.hover_since = Some(Instant::now() - HOVER_OBSERVATION);
            }
            assert_eq!(w.take_friend_join_flight(), matches!(case, 0 | 12 | 13), "physical case {case}");
            assert!(!w.agent.flying, "only the app enables flight after this decision");
        }
    }

    #[test]
    fn friend_join_defers_but_preserves_server_and_saved_flight_on_arrival() {
        for server_flying in [false, true] {
            for preference in [false, true] {
                let mut w = first_leg(true);
                arrive(&mut w, destination());
                allow_flight(&mut w);
                w.fly_after_teleport = preference;
                w.begin_teleport(false);
                w.defer_friend_join_flight(friend(), false, false);
                w.apply(NetEvent::TeleportLocal { flying: server_flying });
                signals(&mut w, crate::ao::State::Standing);
                assert!(!w.agent.flying && !w.take_friend_join_flight());
                arrive(&mut w, destination());
                assert_eq!(w.take_friend_join_flight(), server_flying || preference);
            }
        }
    }

    #[test]
    fn friend_join_still_teleports_when_takeoff_is_blocked() {
        for block in 0..3 {
            let mut w = first_leg(true);
            arrive(&mut w, destination());
            avatar_data(&mut w, 2400.0);
            if block != 0 {
                signals(&mut w, crate::ao::State::Flying);
            }
            allow_flight(&mut w);
            match block {
                0 => {
                    let r = w.regions.get_mut(&destination()).unwrap();
                    std::sync::Arc::make_mut(r.info.as_mut().unwrap()).region_flags |= aurora_net::region_flags::BLOCK_FLY;
                }
                1 => std::sync::Arc::make_mut(w.parcel.as_mut().unwrap()).flags = 0,
                _ => {
                    w.rlva.configure(true);
                    w.rlva_console_input("@fly=n");
                }
            }
            let hop = w
                .finish_friend_join(Instant::now(), false)
                .expect("a flight restriction does not cancel the hop");
            assert_eq!(hop.fly_on_arrival, block != 0);
            assert!(matches!(hop.command, NetCommand::TeleportTo { position, .. } if position.z == 2400.0));
            assert!(!w.agent.flying && w.map.friend_join.is_none());
            w.begin_teleport(false);
            w.defer_friend_join_flight(hop.avatar, hop.fly_on_arrival, hop.fresh_physics);
            local_arrival(&mut w, false);
            assert_eq!(w.agent.position.z, 2400.0, "the hop still arrives when flight is blocked");
            assert!(!w.take_friend_join_flight() && !w.agent.flying);
        }
    }

    #[test]
    fn friend_join_preserves_existing_flight_without_taking_off_for_a_standing_skybox_target() {
        for flying in [false, true] {
            let mut w = first_leg(true);
            arrive(&mut w, destination());
            w.agent.flying = flying;
            avatar(&mut w, 2400.0);
            w.apply_received(
                NetEvent::AvatarAnimations {
                    avatar: friend(),
                    anims: vec![(crate::ao::State::Hovering.motion(), 1)],
                    sources: vec![],
                },
                Instant::now() - Duration::from_secs(10),
            );
            let hop = w.finish_friend_join(Instant::now(), false).unwrap();
            assert_eq!(hop.fly_on_arrival, flying);
            assert_eq!(w.agent.flying, flying);
        }
    }

    #[test]
    fn friend_flight_survives_the_local_reply_once_and_never_leaks_into_another_teleport() {
        for server_flying in [false, true] {
            let mut w = first_leg(true);
            arrive(&mut w, destination());
            w.fly_after_teleport = false;
            w.agent.flying = true;
            w.begin_teleport(false);
            w.defer_friend_join_flight(friend(), true, true);
            w.apply(NetEvent::TeleportLocal { flying: server_flying });
            assert!(w.agent.flying);
            arrive(&mut w, destination());
            assert!(w.take_friend_join_flight());
            assert!(!w.take_friend_join_flight(), "consume even if the server already says flying");
            // A later teleport, including an accepted invitation, uses its own flags.
            w.begin_teleport(false);
            w.apply(NetEvent::TeleportLocal { flying: false });
            assert!(!w.agent.flying);
            w.agent.flying = true;
            w.begin_teleport(false);
            w.defer_friend_join_flight(friend(), true, true);
            w.apply(NetEvent::TeleportFailed {
                reason: "Offline test".into(),
            });
            assert!(w.map.friend_join_flight.is_none());
            w.begin_teleport(false);
            w.agent.flying = true;
            w.defer_friend_join_flight(friend(), true, true);
            w.agent.flying = false;
            local_arrival(&mut w, false);
            assert!(
                !w.take_friend_join_flight() && !w.agent.flying,
                "a new landing choice must not be undone"
            );
        }
    }

    fn local_arrival(w: &mut World, flying: bool) {
        w.apply(NetEvent::TeleportLocal { flying });
        w.apply(NetEvent::AgentMovementComplete {
            handle: destination(),
            position: Vec3::new(60.0, 200.0, 2400.0),
            look_at: Vec3::X,
        });
    }

    #[test]
    fn friend_join_takes_off_only_after_its_local_position_arrives() {
        for server_flying in [false, true] {
            let mut w = first_leg(true);
            arrive(&mut w, destination());
            allow_flight(&mut w);
            w.fly_after_teleport = false;
            w.begin_teleport(false);
            w.defer_friend_join_flight(friend(), true, true);
            assert!(!w.agent.flying && !w.take_friend_join_flight());
            w.apply(NetEvent::TeleportLocal { flying: server_flying });
            assert_eq!(w.agent.position.z, 0.0);
            assert!(
                !w.agent.flying && !w.take_friend_join_flight(),
                "the local flag alone must not enable flight"
            );
            w.apply(NetEvent::AgentMovementComplete {
                handle: destination(),
                position: Vec3::new(60.0, 200.0, 2400.0),
                look_at: Vec3::X,
            });
            assert_eq!(w.agent.position.z, 2400.0);
            assert!(
                !w.agent.flying && w.take_friend_join_flight(),
                "only the applied arrival allows the app to enable flight"
            );
            assert!(!w.take_friend_join_flight());
        }
    }

    #[test]
    fn friend_join_rechecks_flight_policy_at_the_landing_point() {
        for block in 0..3 {
            let mut w = first_leg(true);
            arrive(&mut w, destination());
            allow_flight(&mut w);
            w.begin_teleport(false);
            w.defer_friend_join_flight(friend(), true, true);
            match block {
                0 => {
                    let r = w.regions.get_mut(&destination()).unwrap();
                    std::sync::Arc::make_mut(r.info.as_mut().unwrap()).region_flags |= aurora_net::region_flags::BLOCK_FLY;
                }
                1 => std::sync::Arc::make_mut(w.parcel.as_mut().unwrap()).flags = 0,
                _ => {
                    w.rlva.configure(true);
                    w.rlva_console_input("@fly=n");
                }
            }
            local_arrival(&mut w, false);
            assert_eq!(w.agent.position.z, 2400.0);
            assert!(!w.take_friend_join_flight() && !w.agent.flying);
        }
    }

    #[test]
    fn friend_flight_accepts_the_remote_arrival_without_a_local_reply() {
        let mut w = first_leg(true);
        w.begin_teleport(true);
        w.defer_friend_join_flight_at(friend(), false, false, destination());
        signals(&mut w, crate::ao::State::Flying);
        assert!(
            !w.take_friend_join_flight() && !w.agent.flying,
            "flight waits for the destination position"
        );
        w.apply(NetEvent::AgentMovementComplete {
            handle: destination(),
            position: Vec3::new(60.0, 200.0, 2400.0),
            look_at: Vec3::X,
        });
        allow_flight(&mut w);
        assert_eq!(w.agent.position.z, 2400.0);
        assert!(w.take_friend_join_flight(), "the authoritative arrival is sufficient");
        assert!(!w.take_friend_join_flight(), "flight intent is consumed once");
    }

    #[test]
    fn friend_join_arrival_flight_expires_and_cannot_transfer_to_other_teleports() {
        for cancel in 0..7 {
            let mut w = first_leg(true);
            arrive(&mut w, destination());
            allow_flight(&mut w);
            w.begin_teleport(false);
            w.defer_friend_join_flight(friend(), true, true);
            match cancel {
                0 => w.begin_teleport(false),
                1 => {
                    w.apply(NetEvent::TeleportFailed {
                        reason: "Offline test".into(),
                    });
                }
                2 => w.cancel_friend_join_flight(),
                3 => {
                    w.map.friend_join_flight.as_mut().unwrap().started -= ARRIVAL_TIMEOUT;
                }
                4 => {
                    w.apply_received(NetEvent::TeleportLocal { flying: false }, Instant::now() - Duration::from_secs(10));
                    arrive(&mut w, destination());
                    assert!(!w.take_friend_join_flight());
                    continue;
                }
                5 => {
                    w.apply(NetEvent::TeleportLocal { flying: false });
                    arrive(&mut w, aurora_net::origin_to_handle(256_512, 256_000));
                    assert!(!w.take_friend_join_flight());
                    continue;
                }
                _ => {
                    w.apply_received(
                        NetEvent::AgentMovementComplete {
                            handle: destination(),
                            position: Vec3::new(60.0, 200.0, 2400.0),
                            look_at: Vec3::X,
                        },
                        Instant::now() - Duration::from_secs(10),
                    );
                    assert!(
                        !w.take_friend_join_flight(),
                        "an arrival received before the request cannot trigger flight"
                    );
                    continue;
                }
            }
            local_arrival(&mut w, false);
            assert!(!w.take_friend_join_flight() && !w.agent.flying, "cancellation {cancel}");
        }
    }
}
