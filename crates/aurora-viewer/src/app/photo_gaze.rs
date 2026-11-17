//! The photo window's "Look at camera", "Others look at camera" and "Turn
//! to camera" (Aurora's own: the Legacy Viewer's photo tools have none).
//!
//! - **Look at camera** keeps our look-at target on the camera while the
//!   window is open: a FOCUS target on a global point, the type the Legacy
//!   Viewer sends when the camera locks on a point (LLAgentCamera::
//!   setFocusGlobal → setLookAt(LOOKAT_TARGET_FOCUS), indra/newview/
//!   llagentcamera.cpp). What is sent goes through `set_look_at`, so the
//!   privacy settings and the look-at range apply as for any other target,
//!   and the others see the same ViewerEffect: no new message.
//! - **Others look at camera** turns, on our viewer only, the heads of the
//!   nearby avatars whose owners accepted our posing request
//!   (`world::poser_consent`, the poser's consent): nothing is sent for
//!   them, and a refusal or a withdrawal stops it at once.
//! - On our viewer, the avatars that look (`lookat::PhotoLook`) take the
//!   camera even with "no look-at target, even for me" (ours: a choice made
//!   for the photo), within LLHeadRotMotion's limits and over every
//!   animation (`scene::anim::Controller::look_over_animations`): an AO
//!   stand that keys the head would otherwise hold it. Other viewers blend
//!   the animations as the Legacy Viewer does, so they see the stand's head
//!   where it keys one.
//! - **Turn to camera** turns our body once toward the camera, with the
//!   turn of FSTurnAvatarToSelectedObject (`selection_turn`: the yaw flags
//!   go out with the AgentUpdate). A camera that follows the avatar would
//!   swing round with it, so it is first locked where it looks, from where
//!   it stands (as a click on our avatar with ClickOnAvatarKeepsCamera).

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

/// The share of the turn left to the eyes for a "Look intensity" of
/// `percent` (100: the head faces the camera, 0: the eyes alone).
pub(super) fn glance(percent: u8) -> f32 {
    1.0 - f32::from(percent.min(100)) / 100.0
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
        let wanted = self.photo_gaze_wanted();
        let camera = self
            .world
            .main_origin()
            .map(|(mx, my)| glam::DVec3::new(mx as f64, my as f64, 0.0) + self.camera.position.as_dvec3());
        // our head on our viewer, even with "no look-at target, even for
        // me": the switch is a choice made for the photo; and the heads of
        // those who allowed posing ("Others look at camera")
        let others = self.panels.snapshot && self.settings.snapshot.look_at_camera_others && self.in_world();
        let consent = &self.world.poser_consent;
        let allowed: Vec<uuid::Uuid> = if others {
            consent
                .accepted_peers()
                .filter(|&peer| consent.can_pose(&self.world, peer))
                .collect()
        } else {
            Vec::new()
        };
        let photo = &mut self.world.look_at.photo;
        photo.others.clear();
        photo.others.extend(allowed);
        photo.own = wanted;
        photo.glance = glance(self.settings.snapshot.look_intensity);
        photo.camera = camera.filter(|_| photo.own || !photo.others.is_empty());
        match gaze(wanted, self.photo_gaze) {
            Gaze::Camera => {
                let Some(at) = camera else {
                    return;
                };
                // what the others get: through the privacy settings
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
    /// AURORA_DEMO_PHOTO_GAZE=look[:<percent>]|turn|orbit, with the photo window open
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
        if mode.trim() == "others" {
            self.demo_photo_gaze_others(frame);
            return;
        }
        if frame < 610 || !self.panels.snapshot {
            return;
        }
        let me = self.world.agent_id;
        let Some(head) = self.scene.head_world(&self.world, &me, Instant::now()) else {
            return;
        };
        // look:<percent>: "Look intensity"
        let (mode, percent) = mode.trim().split_once(':').unwrap_or((mode.trim(), ""));
        let percent: Option<u8> = percent.parse().ok();
        let side = |yaw: f32| head + Vec3::new(yaw.cos(), yaw.sin(), 0.0) * 3.0;
        let start = self.world.agent.yaw + 70f32.to_radians();
        if frame == 610 {
            if mode != "turn" {
                self.settings.snapshot.look_at_camera = true;
                self.settings.snapshot.look_intensity = percent.unwrap_or(100).min(100);
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

impl App {
    /// AURORA_DEMO_PHOTO_GAZE=others: at frame 400 the poser's crowd of five
    /// residents stands in front of us (`demo::poser::crowd`); at 600 every
    /// avatar nearby that is not muted is asked to allow posing and accepts
    /// (the poser's consent fixture); at
    /// 610 "Others look at camera" turns on, "Look at camera" stays off,
    /// and the camera stands 3 m from the nearest of them, 70° to its left,
    /// at the height of its head: its head and eyes turn to the lens. The
    /// log gives `photo gaze demo: others, <n> allowed of <m> nearby`.
    fn demo_photo_gaze_others(&mut self, frame: u64) {
        let now = Instant::now();
        // the poser's crowd: five residents in an arc in front of us
        if frame == 400
            && let Some(handle) = self.world.main_region
        {
            self.world.apply(aurora_net::NetEvent::ObjectUpdates {
                handle,
                objects: crate::demo::poser::crowd(),
            });
        }
        let me = self.world.agent_id;
        let peers: Vec<uuid::Uuid> = self
            .world
            .objects
            .avatars()
            .map(|(_, o)| o.full_id)
            .filter(|&id| id != me && crate::world::poser_consent::nearby(&self.world, id))
            .collect();
        if frame == 600 || frame == 601 {
            let mut consent = std::mem::take(&mut self.world.poser_consent);
            for &peer in &peers {
                if frame == 600 {
                    consent.request(&self.world, peer, now);
                } else {
                    consent.demo_accept(&self.world, peer, std::time::Duration::ZERO);
                }
            }
            self.world.poser_consent = consent;
        }
        if frame != 610 || !self.panels.snapshot {
            return;
        }
        self.settings.snapshot.look_at_camera = false;
        self.settings.snapshot.look_at_camera_others = true;
        let allowed = peers.iter().filter(|&&p| self.world.poser_consent.can_pose(&self.world, p)).count();
        let nearest = peers
            .iter()
            .filter_map(|&p| Some((crate::world::poser_consent::distance(&self.world, p)?, p)))
            .min_by(|a, b| a.0.total_cmp(&b.0));
        log::info!("photo gaze demo: others, {allowed} allowed of {} nearby", peers.len());
        let Some((_, subject)) = nearest else {
            return;
        };
        let Some(idx) = self.world.objects.index_of_uuid(&subject) else {
            return;
        };
        let (Some(head), Some((_, rot, _))) = (
            self.scene.head_world(&self.world, &subject, now),
            crate::scene::Scene::object_transform(&self.world, idx, now, 0),
        ) else {
            return;
        };
        let ahead = rot * Vec3::X;
        let yaw = ahead.y.atan2(ahead.x) + 70f32.to_radians();
        let eye = head + Vec3::new(yaw.cos(), yaw.sin(), 0.0) * 3.0;
        self.camera.set_view(eye, head, &mut self.world.agent, &self.settings.camera);
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
    fn the_intensity_leaves_the_rest_of_the_turn_to_the_eyes() {
        assert_eq!(glance(100), 0.0);
        assert_eq!(glance(50), 0.5);
        assert_eq!(glance(0), 1.0);
        assert_eq!(glance(250), 0.0);
        use crate::snapshot::next_look_intensity as next;
        assert_eq!(next(100), 75);
        assert_eq!(next(25), 0);
        assert_eq!(next(0), 100);
        assert_eq!(next(60), 50);
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
