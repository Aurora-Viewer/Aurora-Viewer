//! Render-only dressing-room camera. The live agent, its heading and its
//! camera sent to the simulator are never changed by studio inspection.
//! Aurora addition; the existing wearable editors retain their normal saves.

use glam::{Mat4, Quat, Vec2, Vec3};

pub const FOV: f32 = 0.65;
pub const NEAR: f32 = 0.025;
const MIN_DISTANCE: f32 = 0.35;
const RESPONSE: f32 = 0.09;

const FADE_OUT: f32 = 0.12;
const FADE_IN: f32 = 0.18;
const WHITE_HOLD: f32 = FADE_IN + 0.08;
const WHITE_REVEAL: f32 = 0.85;

#[derive(Default)]
enum Fade {
    #[default]
    Idle,
    Out(f32),
    Covered,
    In(f32),
}

/// A covered frame on each side of the camera switch prevents a scene snap
/// even when presentation trails simulation by a frame. No animation clock
/// or avatar material is modified by the reveal.
#[derive(Default)]
pub struct Transition {
    fade: Fade,
    shown: bool,
    reveal_age: Option<f32>,
}

impl Transition {
    pub fn busy(&self) -> bool {
        !matches!(self.fade, Fade::Idle)
    }

    pub fn age(&self) -> Option<f32> {
        self.reveal_age
    }

    pub fn black(&self) -> f32 {
        match self.fade {
            Fade::Idle => 0.0,
            Fade::Out(age) => (age / FADE_OUT).clamp(0.0, 1.0),
            Fade::Covered => 1.0,
            Fade::In(age) => (1.0 - age / FADE_IN).clamp(0.0, 1.0),
        }
    }

    pub fn white(age: f32) -> f32 {
        (1.0 - (age - WHITE_HOLD).max(0.0) / WHITE_REVEAL).clamp(0.0, 1.0)
    }

    /// Return the scene to apply only after a fully black frame was drawn.
    /// Use the latest request at the midpoint, so quick toggles cannot leave
    /// the toolbar and presentation disagreeing or expose an uncovered cut.
    pub fn step(&mut self, requested: bool, dt: f32) -> Option<bool> {
        let dt = if dt.is_finite() { dt.clamp(0.0, 0.1) } else { 0.0 };
        if let Some(age) = &mut self.reveal_age {
            *age += dt;
            if *age > 1.4 {
                self.reveal_age = None;
            }
        }
        match self.fade {
            Fade::Idle if requested != self.shown => self.fade = Fade::Out(0.0),
            Fade::Out(age) => {
                self.fade = if age + dt >= FADE_OUT { Fade::Covered } else { Fade::Out(age + dt) };
            }
            Fade::Covered => {
                let changed = requested != self.shown;
                self.shown = requested;
                self.reveal_age = (changed && requested).then_some(0.0);
                self.fade = Fade::In(0.0);
                return changed.then_some(requested);
            }
            Fade::In(age) => {
                self.fade = if age + dt >= FADE_IN { Fade::Idle } else { Fade::In(age + dt) };
            }
            Fade::Idle => {}
        }
        None
    }

    pub fn cancel(&mut self) {
        *self = Self::default();
    }
}

#[derive(Default)]
pub struct Studio {
    pub rotating: bool,
    pub panning: bool,
    pub panning_plane: bool,
    pub active: bool,
    pub yaw: f32,
    pub facing: f32,
    pub floor_offset: f32,
    height: f32,
    focus_height: f32,
    target_height: f32,
    distance: f32,
    target_distance: f32,
    full_distance: f32,
    pan: Vec2,
    target_pan: Vec2,
}

#[derive(Clone, Copy)]
pub struct View {
    pub floor: Vec3,
    pub eye: Vec3,
    pub target: Vec3,
    pub view: Mat4,
    pub proj: Mat4,
}

impl Studio {
    pub fn height(&self) -> f32 {
        self.height
    }

    pub fn enter(&mut self, facing: f32, height: f32) {
        let height = height.clamp(0.5, 6.0);
        *self = Self {
            active: true,
            yaw: facing,
            facing,
            height,
            focus_height: height * 0.5,
            target_height: height * 0.5,
            distance: (height * 0.78 / (FOV * 0.5).tan()).clamp(1.5, 6.0),
            ..Self::default()
        };
        self.full_distance = self.distance;
        self.target_distance = self.distance;
    }

    pub fn rotate(&mut self, dx: f32) {
        if dx.is_finite() {
            self.yaw = (self.yaw - dx * 0.008).rem_euclid(std::f32::consts::TAU);
        }
    }

    pub fn view(&self, floor: Vec3, aspect: f32) -> View {
        let rotation = Quat::from_rotation_z(self.yaw);
        let right = rotation * Vec3::Y;
        let up = rotation * Vec3::new(-0.08, 0.0, 1.0).normalize();
        let target = floor + Vec3::Z * self.focus_height + right * self.pan.x + up * self.pan.y;
        let eye = target + rotation * Vec3::new(self.distance, 0.0, self.distance * 0.08);
        View {
            floor,
            eye,
            target,
            view: glam::camera::rh::view::look_to_mat4(eye, (target - eye).normalize(), Vec3::Z),
            proj: glam::camera::rh::proj::directx::perspective_infinite_reverse(FOV, aspect.max(0.1), NEAR),
        }
    }

    /// Right-drag adjusts the avatar-relative focus height. Middle-drag
    /// keeps a separate offset in the current viewing plane.
    /// Scale motion to the visible height so close inspection stays precise.
    pub fn pan_vertical(&mut self, dy: f32, viewport_height: f32) {
        if dy.is_finite() && viewport_height.is_finite() && viewport_height >= 1.0 {
            let metres_per_pixel = 2.0 * self.distance * (FOV * 0.5).tan() / viewport_height;
            self.target_height = (self.target_height - dy * metres_per_pixel).clamp(0.05, self.height);
        }
    }

    /// Grab the view: translate eye and target together, with no dolly or
    /// orbit. Screen-space offsets rotate with the camera so a later orbit
    /// preserves the chosen composition around the avatar.
    pub fn pan_view(&mut self, dx: f32, dy: f32, viewport_height: f32) {
        if dx.is_finite() && dy.is_finite() && viewport_height.is_finite() && viewport_height >= 1.0 {
            let metres_per_pixel = 2.0 * self.distance * (FOV * 0.5).tan() / viewport_height;
            self.target_pan = (self.target_pan + Vec2::new(-dx, dy) * metres_per_pixel).clamp_length_max(self.height * 2.0);
        }
    }

    /// The wheel changes distance alone, regardless of the pointer location.
    pub fn zoom(&mut self, steps: f32) {
        if steps.is_finite() {
            self.target_distance =
                (self.target_distance * (-steps.clamp(-10.0, 10.0) * 0.15).exp()).clamp(MIN_DISTANCE, self.full_distance);
        }
    }

    pub fn update(&mut self, dt: f32) {
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }
        let weight = 1.0 - (-dt.min(0.1) / RESPONSE).exp();
        self.focus_height += (self.target_height - self.focus_height) * weight;
        self.distance += (self.target_distance - self.distance) * weight;
        self.pan += (self.target_pan - self.pan) * weight;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn middle_drag_translates_eye_and_target_in_the_view_plane_at_any_orbit_angle() {
        for yaw in [0.0, 0.7, 1.8, 3.5] {
            let mut studio = Studio::default();
            studio.enter(yaw, 1.8);
            let before = studio.view(Vec3::ZERO, 1.5);
            studio.pan_view(100.0, 80.0, 900.0);
            assert_eq!(studio.view(Vec3::ZERO, 1.5).eye, before.eye, "pan eases from a target");
            studio.update(1.0 / 60.0);
            let after = studio.view(Vec3::ZERO, 1.5);
            let delta = after.eye - before.eye;
            assert!(delta.length() > 0.0);
            assert!((delta - (after.target - before.target)).length() < 0.00001);
            let forward = (before.target - before.eye).normalize();
            let right = forward.cross(Vec3::Z).normalize();
            let up = right.cross(forward);
            assert!(delta.dot(forward).abs() < 0.00001);
            assert!(delta.dot(right) < 0.0);
            assert!(delta.dot(up) > 0.0);
            assert_eq!(studio.focus_height, 0.9);
            assert_eq!(studio.distance, studio.full_distance);
            studio.enter(yaw, 1.8);
            assert_eq!(studio.view(Vec3::ZERO, 1.5).eye, before.eye, "reset recentres");
        }
    }

    #[test]
    fn view_pan_is_frame_rate_independent_and_preserves_composition_when_orbiting() {
        let mut a = Studio::default();
        let mut b = Studio::default();
        for studio in [&mut a, &mut b] {
            studio.enter(0.0, 1.8);
            studio.pan_view(120.0, -80.0, 900.0);
            let target = studio.target_pan;
            for (dx, dy, height) in [(f32::NAN, 0.0, 900.0), (0.0, f32::INFINITY, 900.0), (1.0, 1.0, 0.0)] {
                studio.pan_view(dx, dy, height);
                assert_eq!(studio.target_pan, target);
            }
        }
        for _ in 0..30 {
            a.update(1.0 / 30.0);
        }
        for _ in 0..120 {
            b.update(1.0 / 120.0);
        }
        assert!((a.pan - b.pan).length() < 0.00001);
        let before = a.view(Vec3::ZERO, 1.5);
        a.rotate(-80.0);
        let after = a.view(Vec3::ZERO, 1.5);
        let project = |v: View| v.proj * v.view * Vec3::ZERO.extend(1.0);
        let p = project(before);
        let q = project(after);
        assert!((p.truncate() / p.w - q.truncate() / q.w).length() < 0.00001);
    }

    #[test]
    fn camera_switches_only_between_fully_black_frames_in_both_directions() {
        for fps in [30, 60, 120] {
            let mut transition = Transition::default();
            for requested in [true, false] {
                let mut applied = false;
                for _ in 0..fps {
                    let before = transition.black();
                    if let Some(shown) = transition.step(requested, 1.0 / fps as f32) {
                        assert_eq!(shown, requested);
                        assert_eq!(before, 1.0);
                        assert_eq!(transition.black(), 1.0);
                        applied = true;
                    }
                }
                assert!(applied);
                assert!(!transition.busy());
                assert_eq!(transition.black(), 0.0);
            }
        }
    }

    #[test]
    fn white_reveal_is_linear_after_the_fade_and_latest_toggle_wins() {
        assert_eq!(Transition::white(FADE_IN), 1.0);
        assert!((Transition::white(WHITE_HOLD + WHITE_REVEAL * 0.25) - 0.75).abs() < 0.00001);
        assert!((Transition::white(WHITE_HOLD + WHITE_REVEAL * 0.75) - 0.25).abs() < 0.00001);
        assert_eq!(Transition::white(2.0), 0.0);
        let mut transition = Transition::default();
        transition.step(true, 0.0);
        for _ in 0..30 {
            assert!(transition.step(false, 1.0 / 60.0).is_none());
        }
        assert!(!transition.shown);
        assert!(!transition.busy());
        transition.step(true, f32::NAN);
        transition.cancel();
        assert_eq!(transition.black(), 0.0);
        assert!(transition.age().is_none());
    }

    #[test]
    fn zoom_keeps_the_focus_on_the_avatar_and_eases_towards_it() {
        let mut studio = Studio::default();
        studio.enter(0.4, 1.8);
        let floor = Vec3::new(132.0, 126.0, 30.0);
        let before = studio.view(floor, 1.5);
        studio.zoom(2.0);
        assert_eq!(studio.view(floor, 1.5).eye, before.eye, "wheel sets a target without a camera jump");
        studio.update(1.0 / 60.0);
        let after = studio.view(floor, 1.5);
        assert_eq!(after.target, before.target);
        assert_eq!(after.target.truncate(), floor.truncate());
        assert!(studio.distance > studio.target_distance, "zoom is still easing");
        assert!(after.eye.distance(after.target) < before.eye.distance(before.target));
    }

    #[test]
    fn right_drag_moves_only_height_with_a_frame_rate_independent_response() {
        let mut a = Studio::default();
        let mut b = Studio::default();
        for studio in [&mut a, &mut b] {
            studio.enter(0.0, 1.8);
            studio.pan_vertical(-150.0, 900.0);
            studio.zoom(2.0);
            assert_eq!(studio.focus_height, 0.9);
            assert!(studio.target_height > studio.focus_height);
        }
        for _ in 0..30 {
            a.update(1.0 / 30.0);
        }
        for _ in 0..120 {
            b.update(1.0 / 120.0);
        }
        assert!((a.focus_height - b.focus_height).abs() < 0.00001);
        assert!((a.distance - b.distance).abs() < 0.00001);
        assert_eq!(a.view(Vec3::ZERO, 1.5).target.truncate(), glam::Vec2::ZERO);
        let high = a.target_height;
        a.pan_vertical(50.0, 900.0);
        assert!(a.target_height < high);
    }

    #[test]
    fn inspection_is_bounded_and_reset_is_repeatable() {
        let mut studio = Studio::default();
        studio.enter(0.0, 1.8);
        let original = studio.view(Vec3::ZERO, 1.5);
        studio.rotate(500.0);
        studio.rotate(f32::NAN);
        for _ in 0..100 {
            studio.zoom(10.0);
            studio.pan_vertical(100000.0, 900.0);
            studio.update(1.0 / 60.0);
        }
        assert!(studio.view(Vec3::ZERO, 1.5).eye.is_finite());
        assert!(studio.distance >= MIN_DISTANCE);
        assert!(studio.focus_height >= 0.05);
        let focus = studio.target_height;
        for _ in 0..100 {
            studio.zoom(-10.0);
            studio.update(1.0 / 60.0);
        }
        assert_eq!(studio.target_height, focus, "zoom out does not change the chosen focus");
        studio.enter(0.0, 1.8);
        assert_eq!(studio.view(Vec3::ZERO, 1.5).eye, original.eye);
    }
}
