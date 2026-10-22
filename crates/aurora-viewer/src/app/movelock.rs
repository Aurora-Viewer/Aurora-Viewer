//! Native Movelock orchestration; protocol ownership lives in movement/movelock.
use super::*;
use crate::{
    movement::movelock::{self, Frame, Travel},
    world::bridge::LockMode,
};

impl App {
    pub(super) fn release_native_movelock(&mut self) {
        if let Some(flags) = self.native_anchor.release(self.world.agent.seated, Instant::now()) {
            let command = self.native_anchor.command(flags);
            self.send(command);
        }
    }
    pub(super) fn update_native_movelock(&mut self) {
        if !self.in_world() {
            self.native_anchor = Default::default();
            return;
        }
        if self.demo
            && let Ok(mode) = std::env::var("AURORA_DEMO_MOVELOCK")
            && matches!(mode.as_str(), "native-cycle" | "native-rearm")
        {
            crate::ao::demo::native_anchor_cycle(&mut self.world, &mut self.input, self.frame_count, mode == "native-rearm");
        }
        if self.demo
            && self.frame_count > 300
            && std::env::var("AURORA_DEMO_MOVELOCK").is_ok_and(|mode| mode == "native-fallback")
            && let Some(account) = self.settings.ao.accounts.get_mut(&self.world.agent_id)
        {
            account.enabled = !(700..1000).contains(&self.frame_count);
        }
        let prefs = self.settings.bridge.get(&self.world.agent_id).copied().unwrap_or_default();
        let requested = prefs.enabled && prefs.mode == LockMode::NativeSit;
        let account = self.settings.ao.accounts.get(&self.world.agent_id).cloned().unwrap_or_default();
        let set = self.ao_catalog.selected(account.selected);
        let pose_ready = (requested || self.native_anchor.owns_sit())
            && account.enabled
            && !self.ao_runtime.paused
            && self.ao_runtime.stands
            && !set.is_some_and(|s| s.mouselook && self.camera.mouselook())
            && self.anchor_stands.ready(set, |id| self.scene.anims.get(id));
        let revision = self.world.inventory.generation;
        if pose_ready
            && self.native_anchor.owns_sit()
            && self
                .anchor_pose
                .as_ref()
                .is_none_or(|(old_revision, old)| *old_revision != revision || set.is_none_or(|s| s.id != old.id))
        {
            self.anchor_pose = set.cloned().map(|set| (revision, set));
        }
        let input = &self.input;
        let voluntary = (self.demo && std::env::var("AURORA_DEMO_MOVELOCK").is_ok_and(|m| m == "native-moving"))
            || input.forward
            || input.back
            || input.strafe_left
            || input.strafe_right
            || input.up
            || input.down
            || input.turn_left
            || input.turn_right
            || input.joystick_yaw != 0.0
            || input.joystick_pitch != 0.0
            || self.autopilot.is_some()
            || self.autopilot_flags != 0;
        let water_modes = self
            .settings
            .movements
            .get(&self.world.agent_id)
            .is_some_and(|p| p.swimming || p.water_walk);
        let now = Instant::now();
        let height = self.movement_height();
        let position = self.world.agent.server_pos;
        let water = self.world.main_water_height();
        let near_water = water_modes && position.z - height * 0.5 < water + 0.8;
        let raw = self.world.own_signals();
        // Use physical contact, not the AO pose or visual prediction. A dry
        // platform near water is not swimming, and a stale fall is not travel.
        let needs_support = requested
            && !(self.native_anchor.owns_sit() && self.world.agent.ground_sit)
            && (near_water
                || raw.iter().any(|a| {
                    matches!(
                        crate::ao::State::from_motion(a.id),
                        Some(
                            crate::ao::State::PreJumping
                                | crate::ao::State::Jumping
                                | crate::ao::State::Falling
                                | crate::ao::State::Landing
                                | crate::ao::State::SoftLanding
                        )
                    )
                }));
        let supported = needs_support
            && (crate::movement::run::on_ground(
                position,
                height,
                self.world
                    .objects
                    .index_of_uuid(&self.world.agent_id)
                    .and_then(|i| self.world.objects.get(i))
                    .filter(|o| Some(o.key.region) == self.world.main_region)
                    .and_then(|o| o.foot_plane),
                self.world.ground_height(position),
                false,
            ) || self.scene.movement_support(&self.world, position, height));
        let transient = movelock::airborne(raw, self.world.agent.flying, supported, self.world.agent.velocity.z, now, |id| {
            self.scene.anims.get(id).map(|a| Duration::from_secs_f32(a.anim.duration))
        });
        let travel = if !self.movement_runtime.allows_idle_anchor() || self.autopilot.is_some() {
            Travel::Other
        } else if near_water && !(supported && position.z - height * 0.5 >= water - 0.1) {
            Travel::Water
        } else if transient {
            Travel::Airborne
        } else {
            Travel::Ready
        };
        let frame = Frame {
            requested,
            region: self.world.main_region,
            available: !self.world.teleporting && self.world.movement_complete,
            voluntary,
            moving: self.world.own_motion_moving(),
            travel,
            pose_ready,
            pose_warm: self.ao_runtime.anchor_prepared(self.world.main_region),
            ground_sit: self.world.agent.ground_sit,
            on_object: self.world.agent.seated,
        };
        let previous = self.native_anchor.status;
        if let Some(flags) = self.native_anchor.update(frame, now) {
            if flags == control::SIT_ON_GROUND {
                self.anchor_pose = set.cloned().map(|set| (revision, set));
            }
            let command = self.native_anchor.command(flags);
            self.send(command);
        }
        if self.native_anchor.status != previous {
            log::info!(
                "native movelock: {:?} -> {:?} (available {}, input {}, moving {}, ground sit {}, object seat {})",
                previous,
                self.native_anchor.status,
                frame.available,
                frame.voluntary,
                frame.moving,
                frame.ground_sit,
                frame.on_object
            );
        }
    }
}
