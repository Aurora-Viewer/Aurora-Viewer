//! Follow window and bounded recovery through the ordinary teleport path.
use super::*;
use crate::autopilot;

pub(super) struct Recovery {
    leader: uuid::Uuid,
    started: Instant,
    pub arrived: bool,
}

#[derive(Debug, PartialEq, Eq)]
enum RecoveryStep {
    Cancel,
    Wait,
    Resume,
}

impl Recovery {
    fn step(&self, now: Instant, teleporting: bool, failed: bool, cancelled: bool) -> RecoveryStep {
        if cancelled || failed || now.saturating_duration_since(self.started) >= Duration::from_secs(30) {
            RecoveryStep::Cancel
        } else if self.arrived && !teleporting {
            RecoveryStep::Resume
        } else {
            RecoveryStep::Wait
        }
    }
}

impl App {
    pub(super) fn follow_window(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) {
        let Some(id) = self.world.following_avatar else { return };
        let name = self
            .world
            .person_name(&id)
            .unwrap_or_else(|| aurora_i18n::tr!("common-follow").into());
        let actions = self
            .follow_ui
            .show(ctx, p, &name, self.follow_recovery.is_some(), &mut self.settings.follow);
        if actions.stop {
            self.stop_autopilot(true);
        } else if actions.changed
            && let Some(ap) = &mut self.autopilot
        {
            ap.set_follow_distance(self.settings.follow.distance, self.world.agent.position);
        }
        if actions.save {
            self.settings.save();
        }
    }

    pub(super) fn recover_blocked_follow(&mut self) -> bool {
        let Some(ap) = &self.autopilot else { return false };
        if !self.settings.follow.teleport_if_stuck || !ap.can_recover() {
            return false;
        }
        let Some(leader) = ap.leader else { return false };
        let Some(target) = autopilot::avatar_target(&self.world, leader, self.pelvis_to_foot() * 2.0) else {
            return false;
        };
        let (Some(origin), Some(main)) = (self.world.main_origin(), self.world.main_region) else {
            return false;
        };
        let Some(destination) = autopilot::follow_recovery_position(self.world.agent.position, &target, self.settings.follow) else {
            return false;
        };
        let size = self.world.main().map_or(256.0, |region| region.heightmap.size_x as f32);
        let (slot, position) = autopilot::teleport_destination(origin, size, destination, 0.0);
        let here = slot == origin;
        let handle = if here { main } else { aurora_net::origin_to_handle(slot.0, slot.1) };
        if !self.request_user_teleport(
            NetCommand::TeleportTo {
                handle,
                position,
                look_at: self.camera.forward(),
            },
            self.world.person_name(&leader).unwrap_or_default(),
            !here,
        ) {
            return false;
        }
        // begin_teleport stops the old pilot. Local demo arrivals can already
        // be complete when the synchronous request returns.
        self.follow_recovery = Some(Recovery {
            leader,
            started: Instant::now(),
            arrived: !self.world.teleporting,
        });
        self.world.following_avatar = Some(leader);
        self.follow_ui.open = true;
        true
    }

    /// Keep cancellation live while teleporting/loading the precise target.
    /// No repeated requests on failures, missing targets or blocked arrivals.
    pub(super) fn resume_follow_recovery(&mut self, cancelled: bool) -> bool {
        let Some(recovery) = &self.follow_recovery else { return false };
        match recovery.step(Instant::now(), self.world.teleporting, self.world.tp_failed, cancelled) {
            RecoveryStep::Cancel => self.stop_autopilot(true),
            RecoveryStep::Wait => {}
            RecoveryStep::Resume => {
                let leader = recovery.leader;
                if autopilot::avatar_target(&self.world, leader, self.pelvis_to_foot() * 2.0).is_some() {
                    self.travel_to_avatar(leader, true);
                    if let Some(ap) = &mut self.autopilot {
                        ap.recovered(if ap.flying {
                            self.world.agent.server_pos
                        } else {
                            self.world.agent.position
                        });
                    }
                }
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follow_recovery_waits_for_arrival_and_cancels_on_failure_timeout_or_user_input() {
        let now = Instant::now();
        let mut recovery = Recovery {
            leader: uuid::Uuid::from_u128(7),
            started: now,
            arrived: false,
        };
        assert_eq!(recovery.step(now, true, false, false), RecoveryStep::Wait);
        assert_eq!(recovery.step(now, false, false, false), RecoveryStep::Wait);
        recovery.arrived = true;
        assert_eq!(recovery.step(now, true, false, false), RecoveryStep::Wait);
        assert_eq!(recovery.step(now, false, false, false), RecoveryStep::Resume);
        assert_eq!(recovery.step(now, false, true, false), RecoveryStep::Cancel);
        assert_eq!(recovery.step(now, true, false, true), RecoveryStep::Cancel);
        assert_eq!(
            recovery.step(now + Duration::from_secs(30), false, false, false),
            RecoveryStep::Cancel
        );
    }
}
