//! Native Movelock orchestration; protocol ownership lives in movement/movelock.
use super::*;
use crate::{movement::movelock::Frame, world::bridge::LockMode};

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
        if self.demo && std::env::var("AURORA_DEMO_MOVELOCK").is_ok_and(|m| m == "native-cycle") {
            crate::ao::demo::native_anchor_cycle(&mut self.world, &mut self.input, self.frame_count);
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
            || self.autopilot.is_some()
            || self.autopilot_flags != 0;
        let water_modes = self
            .settings
            .movements
            .get(&self.world.agent_id)
            .is_some_and(|p| p.swimming || p.water_walk);
        let near_water = water_modes && self.world.agent.position.z - self.movement_height() * 0.5 < self.world.main_water_height() + 0.8;
        let transient = !self.world.agent.flying
            && self.world.own_signals().iter().any(|a| {
                matches!(
                    crate::ao::State::from_motion(a.id),
                    Some(crate::ao::State::PreJumping | crate::ao::State::Jumping | crate::ao::State::Falling | crate::ao::State::Landing)
                )
            });
        let frame = Frame {
            requested,
            region: self.world.main_region,
            available: !self.world.teleporting && self.world.movement_complete,
            voluntary,
            moving: self.world.own_motion_moving(),
            eligible: !near_water && !transient && self.movement_runtime.allows_idle_anchor() && self.autopilot.is_none(),
            pose_ready,
            pose_warm: self.ao_runtime.anchor_prepared(self.world.main_region),
            ground_sit: self.world.agent.ground_sit,
            on_object: self.world.agent.seated,
        };
        if let Some(flags) = self.native_anchor.update(frame, Instant::now()) {
            if flags == control::SIT_ON_GROUND {
                self.anchor_pose = set.cloned().map(|set| (revision, set));
            }
            let command = self.native_anchor.command(flags);
            self.send(command);
        }
    }
}
