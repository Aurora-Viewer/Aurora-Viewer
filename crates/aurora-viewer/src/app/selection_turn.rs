//! The avatar turns to face what it selects (the Legacy Viewer's
//! FSTurnAvatarToSelectedObject, on). Port of the turn in
//! LLToolSelect::handleObjectSelection (indra/newview/lltoolselect.cpp) and
//! of the rotation-only walk it starts (LLAgent::startAutoPilotGlobal with a
//! target rotation, LLAgent::autoPilot, indra/newview/llagent.cpp).
//!
//! It turns only while the camera is off the avatar (an Alt zoom, the
//! camera moved by the build tools): with the camera behind the avatar, it
//! would swing the view around.

use super::*;

/// SELECTION_ROTATION_TRESHOLD: the turn ends within this angle (radians).
const DONE_ANGLE: f32 = 0.1;
/// LLAgent::autoPilot turns by 4 × the angle left each second.
const RATE: f32 = 4.0;
/// `gAgent.getAtAxis() * selection_dir < 0.6`: within about 53° of where
/// the avatar faces, it does not turn.
const FACING_DOT: f32 = 0.6;

/// The yaw to turn to, facing `center` from `position` on the ground plane,
/// or None when it is already near enough in front (or right above).
pub(super) fn wanted_yaw(position: Vec3, yaw: f32, center: Vec3) -> Option<f32> {
    let dir = (center - position).truncate().try_normalize()?;
    let at = glam::Vec2::new(yaw.cos(), yaw.sin());
    (at.dot(dir) < FACING_DOT).then(|| dir.y.atan2(dir.x))
}

/// One frame of the turn: the change of yaw (+ to the left) and whether the
/// turn is over.
pub(super) fn step(target: f32, yaw: f32, dt: f32) -> (f32, bool) {
    let left = (target - yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
    if left.abs() < DONE_ANGLE {
        return (0.0, true);
    }
    (left * (RATE * dt).min(1.0), false)
}

impl App {
    /// After a click selects `idx` (the build tools) or right-clicks it:
    /// turn to `center`, the selection's middle. Not an avatar, not what
    /// we wear, not while sitting (SELECTION_SITTING_ROTATION_TRESHOLD is
    /// more than half a turn, so a seated avatar never turns).
    pub(super) fn turn_to_selection(&mut self, idx: usize, center: Vec3) {
        if !self.settings.turn_to_selected
            || self.camera.focus_point().is_none()
            || self.world.agent.is_sitting()
            || self.autopilot.is_some()
            || !self.turns_toward(idx)
        {
            return;
        }
        let agent = &self.world.agent;
        self.selection_turn = wanted_yaw(agent.position, agent.yaw, center);
        if let Some(yaw) = self.selection_turn {
            log::info!("selection turn: to {:.0}°", yaw.to_degrees());
        }
    }

    /// Not an avatar and nothing an avatar wears (HUDs included). The
    /// Legacy Viewer leaves out our attachments only; Aurora also leaves out
    /// someone else's, whose right click opens that avatar's menu
    /// (app/context_menu.rs), so it is the avatar's rule that applies.
    fn turns_toward(&self, idx: usize) -> bool {
        self.world.objects.get(idx).is_some_and(|o| !o.is_avatar()) && ui::context::wearer(&self.world, idx).is_none()
    }

    /// A frame of the turn, after the keys are read: a movement key, a walk,
    /// sitting or leaving the world ends it, as they stop the Legacy
    /// Viewer's autopilot. The yaw flags go out with this frame's
    /// AgentUpdate, so the turning animation plays.
    pub(super) fn update_selection_turn(&mut self, dt: f32) {
        let Some(target) = self.selection_turn else {
            return;
        };
        let i = &self.input;
        let keys = i.forward
            || i.back
            || i.turn_left
            || i.turn_right
            || i.strafe_left
            || i.strafe_right
            || i.up
            || i.down
            || i.stop
            || i.joystick_yaw.abs() > 0.001;
        if keys
            || self.autopilot.is_some()
            || self.mouse_mode == MouseMode::Steer
            || self.world.agent.is_sitting()
            || !self.in_world()
        {
            self.selection_turn = None;
            return;
        }
        let (turn, done) = step(target, self.world.agent.yaw, dt.clamp(1e-3, 0.1));
        self.world.agent.yaw += turn;
        if turn > 0.0 {
            self.autopilot_flags |= aurora_net::control::YAW_POS;
        } else if turn < 0.0 {
            self.autopilot_flags |= aurora_net::control::YAW_NEG;
        }
        if done {
            self.selection_turn = None;
        }
    }
}

impl App {
    /// AURORA_DEMO_TURN_SELECT: at frame 300 the camera looks down on the
    /// avatar from above, off the avatar, and the build tools select the
    /// nearest object more than 53° to its side, as a click would; the log
    /// gives the yaw as the avatar turns.
    pub(super) fn demo_selection_turn(&mut self) {
        let now = Instant::now();
        if self.frame_count == 300 {
            let agent = &self.world.agent;
            let (position, yaw) = (agent.position, agent.yaw);
            let picked = self
                .world
                .objects
                .iter()
                .filter(|(i, o)| !o.is_avatar() && self.world.objects.parent_of(o).is_none() && self.turns_toward(*i))
                .filter_map(|(i, o)| {
                    let center = crate::build::bounds_of(&self.world, &[o.key], false, now)?.center;
                    let distance = (center - position).truncate().length();
                    ((2.0..15.0).contains(&distance) && wanted_yaw(position, yaw, center).is_some()).then_some((i, center, distance))
                })
                .min_by(|a, b| a.2.total_cmp(&b.2));
            let Some((idx, center, _)) = picked else {
                log::warn!("demo selection turn: no object to the side");
                return;
            };
            let middle = (position + center) / 2.0;
            self.camera
                .set_view(middle + Vec3::new(-0.5, 0.0, 14.0), middle, &mut self.world.agent, &self.settings.camera);
            self.build.open_build(crate::build::Tool::Edit);
            self.build.click_select(&mut self.world, &self.settings.build, Some(idx), false);
            self.flush_build();
            log::info!("demo selection turn: yaw {:.0}°, object at {center:?}", yaw.to_degrees());
            self.turn_to_selection(idx, center);
        } else if self.frame_count % 30 == 0 {
            log::info!(
                "demo selection turn: yaw {:.0}°, turning {}",
                self.world.agent.yaw.to_degrees(),
                self.selection_turn.is_some()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::{FRAC_PI_2, PI};

    #[test]
    fn an_object_in_front_does_not_turn_the_avatar() {
        // facing +X; 45° to the left is within the 0.6 cone
        assert_eq!(wanted_yaw(Vec3::ZERO, 0.0, Vec3::new(3.0, 3.0, 5.0)), None);
        // right above: no direction on the ground
        assert_eq!(wanted_yaw(Vec3::ZERO, 0.0, Vec3::new(0.0, 0.0, 5.0)), None);
    }

    #[test]
    fn an_object_to_the_side_or_behind_turns_the_avatar_to_it() {
        let yaw = wanted_yaw(Vec3::ZERO, 0.0, Vec3::new(0.0, 4.0, 1.0)).expect("turns");
        assert!((yaw - FRAC_PI_2).abs() < 1e-5, "{yaw}");
        let yaw = wanted_yaw(Vec3::new(10.0, 10.0, 0.0), 0.0, Vec3::new(5.0, 10.0, 0.0)).expect("turns");
        assert!((yaw.abs() - PI).abs() < 1e-5, "{yaw}");
    }

    #[test]
    fn the_turn_eases_in_the_short_way_and_ends_near_the_target() {
        let dt = 1.0 / 60.0;
        // from just left of behind to just right of behind: the short way
        // crosses ±180°, it does not go all the way round
        let (turn, done) = step(-PI + 0.2, PI - 0.2, dt);
        assert!(!done && turn > 0.0 && turn < 0.4, "{turn}");
        let mut yaw = 0.0;
        let mut frames = 0;
        loop {
            let (turn, done) = step(FRAC_PI_2, yaw, dt);
            yaw += turn;
            frames += 1;
            if done {
                break;
            }
            assert!(frames < 120, "never ends");
        }
        assert!((FRAC_PI_2 - yaw).abs() < DONE_ANGLE);
        // 4 × the angle left per second: from 90° to within 0.1 rad in
        // about ln(15.7) / 4 ≈ 0.7 s
        assert!((30..=50).contains(&frames), "{frames}");
    }
}
