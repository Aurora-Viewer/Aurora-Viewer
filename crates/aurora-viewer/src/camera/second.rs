//! The second camera's framing: where it stands to keep our avatar in view
//! (the face, the whole body, the back), turned by the user around the
//! avatar and following it smoothly. Aurora's own: the Legacy Viewer has no
//! second view. The idea comes from SnowPastel's multi-view-viewer (an
//! experimental viewer built on the Second Life viewer, LGPL 2.1); none of
//! its code is used here. The view is drawn by `aurora_render::second_view`.
//!
//! The camera turns with the avatar's body, not its head: a head that looks
//! around keeps the same picture, and a face shot stays in front of the
//! face when the avatar turns.

use glam::{Mat4, Vec3};

/// What the second camera frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Framing {
    /// The face, from the front: the fringe, make-up, an expression.
    #[default]
    Face,
    /// The whole body, from the front.
    Body,
    /// The whole body, from behind: the back of an outfit, the hair.
    Back,
}

impl Framing {
    pub const ALL: [Framing; 3] = [Framing::Face, Framing::Body, Framing::Back];

    /// Vertical field of view: a portrait lens for the face, which does not
    /// round it as a wide one would, a little wider for the body.
    fn fov_y(self) -> f32 {
        match self {
            Framing::Face => 30f32.to_radians(),
            Framing::Body | Framing::Back => 40f32.to_radians(),
        }
    }

    /// How far above the target the camera looks down from, in radians.
    fn pitch(self) -> f32 {
        match self {
            Framing::Face => 4f32.to_radians(),
            Framing::Body | Framing::Back => 8f32.to_radians(),
        }
    }

    /// Where the camera is turned to, from the avatar's front, in radians.
    fn yaw(self) -> f32 {
        match self {
            Framing::Face | Framing::Body => 0.0,
            Framing::Back => std::f32::consts::PI,
        }
    }
}

/// The user's turn of the camera around the avatar (dragged in the
/// window) and its zoom (the wheel), over the framing's own.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Orbit {
    pub yaw: f32,
    pub pitch: f32,
    /// Distance factor: below 1 nearer, above 1 farther.
    pub zoom: f32,
}

impl Default for Orbit {
    fn default() -> Self {
        Orbit {
            yaw: 0.0,
            pitch: 0.0,
            zoom: 1.0,
        }
    }
}

/// Bounds of the user's pitch (radians, over the framing's) and zoom.
const PITCH_MIN: f32 = -1.0;
const PITCH_MAX: f32 = 1.2;
const ZOOM_MIN: f32 = 0.4;
const ZOOM_MAX: f32 = 4.0;

impl Orbit {
    /// A drag of `dx`, `dy` points: right turns the camera to the left of
    /// the avatar (the avatar seems to turn right), down raises it.
    pub fn drag(&mut self, dx: f32, dy: f32) {
        self.yaw = (self.yaw - dx * 0.01).rem_euclid(std::f32::consts::TAU);
        self.pitch = (self.pitch + dy * 0.01).clamp(PITCH_MIN, PITCH_MAX);
    }

    /// A wheel of `delta` points: up comes nearer.
    pub fn wheel(&mut self, delta: f32) {
        self.zoom = (self.zoom * (-delta * 0.002).exp()).clamp(ZOOM_MIN, ZOOM_MAX);
    }
}

/// Our avatar as the second camera frames it, in the world, as posed last
/// frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Subject {
    /// The head joint (mHead).
    pub head: Vec3,
    /// Between the eyes, when the skeleton has them.
    pub eyes: Option<Vec3>,
    /// The way the body faces (its X axis).
    pub facing: Vec3,
    /// The lowest foot.
    pub feet: f32,
}

/// Height of the top of the head over the head joint, and the room around
/// the body in a body shot.
const HEAD_TOP: f32 = 0.16;
const BODY_MARGIN: f32 = 1.12;
/// Height the face shot frames (the head and the top of the shoulders),
/// and the body's width, which a narrow picture must still hold.
const FACE_HEIGHT: f32 = 0.42;
const BODY_WIDTH: f32 = 0.8;
/// The near plane: the camera can come close to the face.
pub const NEAR: f32 = 0.05;

/// Where the camera stands and looks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shot {
    pub eye: Vec3,
    pub target: Vec3,
    pub fov_y: f32,
}

impl Shot {
    pub fn view(&self) -> Mat4 {
        glam::camera::rh::view::look_to_mat4(self.eye, (self.target - self.eye).normalize_or(Vec3::X), Vec3::Z)
    }

    /// The projection of a picture `aspect` wide over high, reverse-Z
    /// infinite as the window's (`Camera::proj`).
    pub fn proj(&self, aspect: f32) -> Mat4 {
        glam::camera::rh::proj::directx::perspective_infinite_reverse(self.fov_y, aspect.max(0.1), NEAR)
    }
}

/// The shot of `framing`, turned by `orbit`, for a picture `aspect` wide
/// over high.
pub fn shot(subject: &Subject, framing: Framing, orbit: Orbit, aspect: f32) -> Shot {
    let fov_y = framing.fov_y();
    let half = (fov_y * 0.5).tan();
    let aspect = aspect.max(0.1);
    // the body's front, flat: a bowing avatar keeps a level camera
    let front = Vec3::new(subject.facing.x, subject.facing.y, 0.0).normalize_or(Vec3::X);
    let (target, height) = match framing {
        Framing::Face => (subject.eyes.unwrap_or(subject.head + Vec3::Z * 0.06), FACE_HEIGHT),
        Framing::Body | Framing::Back => {
            let top = subject.head.z + HEAD_TOP;
            let bottom = subject.feet.min(top - 0.3);
            (
                Vec3::new(subject.head.x, subject.head.y, (top + bottom) * 0.5),
                (top - bottom) * BODY_MARGIN,
            )
        }
    };
    // far enough for the height, and for the width in a narrow picture
    let width = match framing {
        Framing::Face => FACE_HEIGHT * 0.8,
        Framing::Body | Framing::Back => BODY_WIDTH,
    };
    let distance = (height.max(width / aspect) * 0.5 / half) * orbit.zoom;
    let yaw = framing.yaw() + orbit.yaw;
    let pitch = (framing.pitch() + orbit.pitch).clamp(PITCH_MIN, PITCH_MAX);
    let around = glam::Quat::from_rotation_z(yaw) * front;
    let out = around * pitch.cos() + Vec3::Z * pitch.sin();
    Shot {
        eye: target + out * distance,
        target,
        fov_y,
    }
}

/// How fast the camera catches up with its shot: the time for most of a
/// move (63 %), in seconds. Smooth enough that an idle animation's sway
/// does not shake the picture, quick enough to follow a walk.
const FOLLOW: f32 = 0.12;
/// A jump this far (a teleport, a region crossing, a seat): the camera is
/// put there at once.
const JUMP: f32 = 8.0;

/// The second camera: its framing, the user's orbit, and where it is now.
#[derive(Debug, Clone, Default)]
pub struct SecondCamera {
    pub framing: Framing,
    pub orbit: Orbit,
    now: Option<Shot>,
}

impl SecondCamera {
    /// The framing chosen, with the user's turn and zoom set back.
    pub fn set_framing(&mut self, framing: Framing) {
        self.framing = framing;
        self.orbit = Orbit::default();
    }

    /// Back to the framing's own camera.
    pub fn reset(&mut self) {
        self.orbit = Orbit::default();
    }

    /// Forget where the camera was: the next shot is taken at once (the
    /// window opened again, the avatar lost for a while).
    pub fn cut(&mut self) {
        self.now = None;
    }

    /// The camera `dt` seconds after the last frame, on its way to the shot
    /// of `subject`. The framing's distance follows the zoom at once (no
    /// lag between the wheel and the picture); only the subject's moves
    /// are smoothed.
    pub fn follow(&mut self, subject: &Subject, aspect: f32, dt: f32) -> Shot {
        let wanted = shot(subject, self.framing, self.orbit, aspect);
        let next = match self.now {
            Some(now) if now.target.distance(wanted.target) < JUMP => {
                let k = 1.0 - (-dt.max(0.0) / FOLLOW).exp();
                let target = now.target.lerp(wanted.target, k);
                Shot {
                    // the same offset as the wanted shot, from the smoothed target
                    eye: target + (wanted.eye - wanted.target),
                    target,
                    fov_y: wanted.fov_y,
                }
            }
            _ => wanted,
        };
        self.now = Some(next);
        next
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn subject() -> Subject {
        Subject {
            head: Vec3::new(10.0, 20.0, 31.6),
            eyes: Some(Vec3::new(10.08, 20.0, 31.66)),
            facing: Vec3::X,
            feet: 30.0,
        }
    }

    #[test]
    fn the_face_shot_stands_before_the_face_and_looks_at_the_eyes() {
        let s = subject();
        let shot = shot(&s, Framing::Face, Orbit::default(), 0.8);
        assert_eq!(shot.target, s.eyes.unwrap());
        // in front of the avatar (its +X), a little above the eyes
        assert!(shot.eye.x > s.head.x + 0.5, "{:?}", shot.eye);
        assert!((shot.eye.y - s.head.y).abs() < 1e-4);
        assert!(shot.eye.z > shot.target.z);
        // near: a face shot, not a body one
        assert!(shot.eye.distance(shot.target) < 1.5);
    }

    #[test]
    fn the_body_shot_holds_the_whole_avatar() {
        let s = subject();
        let shot = shot(&s, Framing::Body, Orbit::default(), 0.75);
        let view = shot.view();
        let proj = shot.proj(0.75);
        for point in [Vec3::new(10.0, 20.0, s.head.z + HEAD_TOP), Vec3::new(10.0, 20.0, s.feet)] {
            let clip = proj * view * point.extend(1.0);
            let ndc = clip.truncate() / clip.w;
            assert!(ndc.y.abs() < 1.0, "{point} at {ndc}");
        }
        // the body's width fits too in a narrow picture
        let narrow = super::shot(&s, Framing::Body, Orbit::default(), 0.3);
        let clip = narrow.proj(0.3) * narrow.view() * Vec3::new(10.0, 20.0 + BODY_WIDTH * 0.5, 30.8).extend(1.0);
        assert!((clip.x / clip.w).abs() <= 1.0);
    }

    #[test]
    fn the_back_shot_stands_behind() {
        let s = subject();
        let back = shot(&s, Framing::Back, Orbit::default(), 1.0);
        let front = shot(&s, Framing::Body, Orbit::default(), 1.0);
        assert!(back.eye.x < s.head.x - 1.0);
        assert!(front.eye.x > s.head.x + 1.0);
        assert_eq!(back.target, front.target);
    }

    #[test]
    fn the_camera_turns_with_the_body() {
        let mut s = subject();
        s.facing = Vec3::Y;
        let shot = shot(&s, Framing::Body, Orbit::default(), 1.0);
        assert!(shot.eye.y > s.head.y + 1.0);
        assert!((shot.eye.x - s.head.x).abs() < 1e-3);
        // a bowing body (its front pointing down) keeps a level camera
        s.facing = Vec3::new(0.0, 0.3, -0.95);
        let bowed = super::shot(&s, Framing::Body, Orbit::default(), 1.0);
        assert!((bowed.eye - shot.eye).length() < 1e-3);
    }

    #[test]
    fn the_orbit_turns_raises_and_zooms_within_bounds() {
        let s = subject();
        let mut orbit = Orbit::default();
        orbit.drag(-157.0, 0.0);
        // about a quarter turn to the avatar's left
        let turned = shot(&s, Framing::Body, orbit, 1.0);
        assert!(turned.eye.y > s.head.y + 1.0, "{:?}", turned.eye);
        orbit.drag(0.0, 10_000.0);
        assert_eq!(orbit.pitch, PITCH_MAX);
        orbit.drag(0.0, -10_000.0);
        assert_eq!(orbit.pitch, PITCH_MIN);
        let far = shot(&s, Framing::Face, Orbit::default(), 1.0).eye.distance(s.eyes.unwrap());
        orbit = Orbit::default();
        orbit.wheel(200.0);
        let near = shot(&s, Framing::Face, orbit, 1.0).eye.distance(s.eyes.unwrap());
        assert!(near < far);
        orbit.wheel(1e6);
        assert_eq!(orbit.zoom, ZOOM_MIN);
        orbit.wheel(-1e6);
        assert_eq!(orbit.zoom, ZOOM_MAX);
    }

    #[test]
    fn a_skeleton_without_eyes_frames_the_head() {
        let mut s = subject();
        s.eyes = None;
        let shot = shot(&s, Framing::Face, Orbit::default(), 1.0);
        assert!((shot.target - (s.head + Vec3::Z * 0.06)).length() < 1e-5);
    }

    #[test]
    fn following_is_smooth_but_jumps_are_taken_at_once() {
        let mut camera = SecondCamera::default();
        let mut s = subject();
        let first = camera.follow(&s, 1.0, 1.0 / 60.0);
        assert_eq!(first, shot(&s, Framing::Face, Orbit::default(), 1.0));
        // a step of 20 cm: a part of the way after one frame
        s.head.x += 0.2;
        s.eyes = s.eyes.map(|e| e + Vec3::X * 0.2);
        let next = camera.follow(&s, 1.0, 1.0 / 60.0);
        let moved = next.target.x - first.target.x;
        assert!(moved > 0.0 && moved < 0.1, "{moved}");
        // after a second, there
        let mut last = next;
        for _ in 0..60 {
            last = camera.follow(&s, 1.0, 1.0 / 60.0);
        }
        assert!((last.target.x - s.eyes.unwrap().x).abs() < 1e-3);
        // a teleport: at once
        s.head.x += 100.0;
        s.eyes = s.eyes.map(|e| e + Vec3::X * 100.0);
        let jumped = camera.follow(&s, 1.0, 1.0 / 60.0);
        assert_eq!(jumped.target, s.eyes.unwrap());
    }

    #[test]
    fn a_new_framing_starts_from_its_own_camera() {
        let mut camera = SecondCamera::default();
        camera.orbit.drag(50.0, 20.0);
        camera.orbit.wheel(100.0);
        camera.set_framing(Framing::Back);
        assert_eq!(camera.framing, Framing::Back);
        assert_eq!(camera.orbit, Orbit::default());
    }
}
