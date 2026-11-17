//! The photo window's "Look at camera" and "Turn to camera" (Aurora's own:
//! the Legacy Viewer's photo tools have neither).
//!
//! - **Look at camera** keeps our look-at target on the camera while the
//!   window is open: a FOCUS target on a global point, the type the Legacy
//!   Viewer sends when the camera locks on a point (LLAgentCamera::
//!   setFocusGlobal → setLookAt(LOOKAT_TARGET_FOCUS), indra/newview/
//!   llagentcamera.cpp). It goes through `set_look_at`, so the privacy
//!   settings and the look-at range apply as for any other target, and the
//!   others see the same ViewerEffect the Legacy Viewer sends: no new
//!   message. The head and eyes take it as any target (LLHeadRotMotion's
//!   limits).
//! - **Turn to camera** turns the body once toward the camera, with the
//!   turn of FSTurnAvatarToSelectedObject (`selection_turn`: the yaw flags
//!   go out with the AgentUpdate). A camera that follows the avatar would
//!   swing around with it, so it is first locked where it looks, from where it stands (as a
//!   click on our avatar with ClickOnAvatarKeepsCamera).

use super::*;

/// What the look-at needs this frame, from whether the camera is wanted
/// (`want`) and was taken last frame (`was`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Gaze {
    /// The camera is the target.
    Camera,
    /// Just turned off: the target is cleared once (LOOKAT_TARGET_CLEAR, a
    /// FOCUS never times out), and the usual targets come back.
    Release,
    /// Nothing to do.
    Idle,
}

pub(super) fn gaze(want: bool, was: bool) -> Gaze {
    match (want, was) {
        (true, _) => Gaze::Camera,
        (false, true) => Gaze::Release,
        (false, false) => Gaze::Idle,
    }
}

/// The yaw (radians, from +X toward +Y) that faces `camera` from `position`
/// on the ground plane; None with the camera right above or below.
pub(super) fn yaw_to(position: Vec3, camera: Vec3) -> Option<f32> {
    let dir = (camera - position).truncate().try_normalize()?;
    Some(dir.y.atan2(dir.x))
}

impl App {
    /// "Look at camera" applies: the photo window is open, the switch on,
    /// and the camera outside our head (in mouselook it is where the eyes
    /// are).
    fn photo_gaze_wanted(&self) -> bool {
        self.panels.snapshot && self.settings.snapshot.look_at_camera && !self.camera.mouselook() && self.in_world()
    }

    /// Each frame, after the usual targets are set (`update_look_at`): FOCUS
    /// wins over them (attentions.xml), and is set again where the camera
    /// now stands. Sends are throttled by `lookat::Own::set` (5 cm, 0.25 s).
    pub(super) fn update_photo_gaze(&mut self) {
        use crate::world::lookat;
        match gaze(self.photo_gaze_wanted(), self.photo_gaze) {
            Gaze::Camera => {
                let Some((mx, my)) = self.world.main_origin() else {
                    return;
                };
                let at = glam::DVec3::new(mx as f64, my as f64, 0.0) + self.camera.position.as_dvec3();
                self.set_look_at(lookat::FOCUS, None, at);
                self.photo_gaze = true;
            }
            Gaze::Release => {
                self.set_look_at(lookat::CLEAR, None, glam::DVec3::ZERO);
                // a point the camera is locked on takes the gaze back
                self.look_focus = None;
                self.photo_gaze = false;
            }
            Gaze::Idle => {}
        }
    }

    /// "Turn to camera": the body turns to face the camera, as far as it
    /// can (not while sitting nor in mouselook).
    pub(super) fn photo_turn_to_camera(&mut self) {
        if self.world.agent.is_sitting() || self.camera.mouselook() || self.autopilot.is_some() || !self.in_world() {
            return;
        }
        let Some(yaw) = yaw_to(self.world.agent.position, self.camera.position) else {
            return;
        };
        if self.camera.focus_on_avatar() {
            let at = self.camera.target;
            self.camera.focus_own_surface(at, None, None, &self.settings.camera);
        }
        self.selection_turn = Some(yaw);
        log::info!("photo: turn to the camera, to {:.0}°", yaw.to_degrees());
    }
}

impl App {
    /// AURORA_DEMO_PHOTO_GAZE=look|turn|orbit, with the photo window open
    /// (`AURORA_DEMO_SNAPSHOT=open` or `viewfinder`, frame 600): at frame
    /// 610 the camera stands 3 m from our avatar, 70° to its left, at the
    /// height of its head. `look` turns "Look at camera" on (the head and
    /// eyes turn to the lens); `turn` presses "Turn to camera" at frame 660
    /// (the body turns, the camera stays); `orbit` turns "Look at camera"
    /// on and the camera goes round the avatar, a turn every 12 s, the head
    /// following as far as it can.
    pub(super) fn demo_photo_gaze(&mut self) {
        let Ok(mode) = std::env::var("AURORA_DEMO_PHOTO_GAZE") else {
            return;
        };
        let frame = self.frame_count;
        if frame < 610 || !self.panels.snapshot {
            return;
        }
        let me = self.world.agent_id;
        let Some(head) = self.scene.head_world(&self.world, &me, Instant::now()) else {
            return;
        };
        let mode = mode.trim();
        let side = |yaw: f32| head + Vec3::new(yaw.cos(), yaw.sin(), 0.0) * 3.0;
        let start = self.world.agent.yaw + 70f32.to_radians();
        if frame == 610 {
            if mode != "turn" {
                self.settings.snapshot.look_at_camera = true;
            }
            self.camera
                .set_view(side(start), head, &mut self.world.agent, &self.settings.camera);
            log::info!("photo gaze demo: {mode}, the camera 70° to the left");
        }
        if mode == "turn" && frame == 660 {
            self.photo_turn_to_camera();
        }
        if mode == "orbit" && frame > 610 {
            let turn = (frame - 610) as f32 / 720.0 * std::f32::consts::TAU;
            self.camera
                .set_view(side(start + turn), head, &mut self.world.agent, &self.settings.camera);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_camera_is_taken_while_wanted_and_released_once() {
        assert_eq!(gaze(true, false), Gaze::Camera);
        assert_eq!(gaze(true, true), Gaze::Camera);
        assert_eq!(gaze(false, true), Gaze::Release);
        assert_eq!(gaze(false, false), Gaze::Idle);
    }

    #[test]
    fn the_turn_faces_the_camera_on_the_ground_plane() {
        let at = Vec3::new(10.0, 10.0, 20.0);
        let close = |a: f32, b: f32| (a - b).abs() < 1e-5;
        // in front, higher up: the height does not count
        assert!(close(yaw_to(at, Vec3::new(14.0, 10.0, 25.0)).unwrap(), 0.0));
        // to the left (+Y), behind (−X)
        assert!(close(yaw_to(at, Vec3::new(10.0, 13.0, 20.0)).unwrap(), std::f32::consts::FRAC_PI_2));
        assert!(close(yaw_to(at, Vec3::new(6.0, 10.0, 20.0)).unwrap().abs(), std::f32::consts::PI));
        // right above: no direction
        assert_eq!(yaw_to(at, Vec3::new(10.0, 10.0, 30.0)), None);
    }
}
