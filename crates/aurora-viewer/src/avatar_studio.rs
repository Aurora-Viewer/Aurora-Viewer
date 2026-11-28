//! Render-only dressing-room camera. The live agent, its heading and its
//! camera sent to the simulator are never changed by studio inspection.
//! Aurora addition; the existing wearable editors retain their normal saves.

use glam::{Mat4, Quat, Vec2, Vec3};

/// The studio camera's default elevation over its focus: a slight look
/// down, `atan(0.08)`.
const BASE_ELEVATION: f32 = 0.079_829_99;
/// How far the tilted orbit may go above or below the focus (80°).
const MAX_ELEVATION: f32 = 1.396_263_4;
/// The lowest a tilt brings the eye, over the stage's floor.
const EYE_CLEARANCE: f32 = 0.05;
pub const FOV: f32 = 0.65;
pub const NEAR: f32 = 0.025;
/// Existing approximation from the foot joint to the studio's sole plane.
pub const SOLE_HEIGHT: f32 = 0.06;
// Numerical separation only: no avatar-framing zoom limit.
const MIN_DISTANCE: f32 = 0.001;
/// The head close up (`inspect_head`), m: about half a metre of view, the
/// head filling half of it.
const HEAD_DISTANCE: f32 = 0.75;
const RESPONSE: f32 = 0.09;

/// How far the camera stands to frame a whole avatar `height` tall.
fn body_distance(height: f32) -> f32 {
    (height * 0.78 / (FOV * 0.5).tan()).clamp(1.5, 6.0)
}

const FADE_OUT: f32 = 0.12;
const FADE_IN: f32 = 0.18;
const WHITE_HOLD: f32 = FADE_IN + 0.08;
const WHITE_REVEAL: f32 = 0.85;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Pose {
    #[default]
    Animated,
    T,
    A,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Lighting {
    #[default]
    Studio,
    Environment,
}

/// Temporary photography camera; the ordinary world camera and agent aim
/// stay available for simulator updates and are restored on exit.
pub struct PhotoSession {
    pub camera: crate::camera::Camera,
    yaw: f32,
    pitch: f32,
}

impl PhotoSession {
    pub fn enter(
        camera: &mut crate::camera::Camera,
        agent: &crate::agent::AgentState,
        settings: &crate::camera::CameraSettings,
        view: View,
    ) -> Self {
        let mut local = crate::camera::Camera::default();
        local.set_view(view.eye, view.target, &mut agent.clone(), settings);
        local.position = view.eye;
        local.target = view.target;
        local.near = NEAR;
        local.fov_y = FOV;
        if camera.free().is_some() {
            local.start_free(&mut agent.clone(), settings);
        }
        local.snap();
        Self {
            camera: std::mem::replace(camera, local),
            yaw: agent.yaw,
            pitch: agent.pitch,
        }
    }

    pub fn controls(&self, agent: &crate::agent::AgentState, input: &crate::agent::MoveInput, far: f32) -> aurora_net::AgentControls {
        let mut agent = agent.clone();
        agent.yaw = self.yaw;
        agent.pitch = self.pitch;
        agent.controls(input, &self.camera, far)
    }

    pub fn restore(self, camera: &mut crate::camera::Camera, agent: &mut crate::agent::AgentState) {
        *camera = self.camera;
        agent.yaw = self.yaw;
        agent.pitch = self.pitch;
    }
}

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
    pub pose: Pose,
    pub lighting: Lighting,
    pub animation_editor: bool,
    pub reset_animation_confirm: bool,
    pub rotating: bool,
    pub panning: bool,
    pub panning_plane: bool,
    pub active: bool,
    pub yaw: f32,
    /// Tilt of the orbit above (positive) or below the studio's slight
    /// default elevation, radians; a vertical left drag changes it.
    pub tilt: f32,
    pub facing: f32,
    pub floor_offset: f32,
    /// Session-only vertical offset for the displayed studio avatar.
    pub hover_height: f32,
    /// Smoothed display value, so small changes do not visibly step the rig.
    pub display_hover_height: f32,
    /// The animation editor's Person B (couple mode): their own hover, or
    /// None to follow `hover_height`, so the pair keeps its spacing.
    pub partner_hover_height: Option<f32>,
    pub display_partner_hover_height: f32,
    /// Whether the Hover menu sets Person B's hover rather than the user's.
    pub hover_partner: bool,
    /// Furniture root and linkset-bounds centre in that root's local frame.
    pub podium_center: Option<(uuid::Uuid, Vec3)>,
    height: f32,
    focus_height: f32,
    target_height: f32,
    distance: f32,
    target_distance: f32,
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
            distance: body_distance(height),
            ..Self::default()
        };
        self.target_distance = self.distance;
    }

    pub fn rotate(&mut self, dx: f32) {
        if dx.is_finite() {
            self.yaw = (self.yaw - dx * 0.008).rem_euclid(std::f32::consts::TAU);
        }
    }

    /// A vertical left drag tilts the orbit: dragging down looks from
    /// higher up, dragging up from lower down, short of straight above or
    /// below so the view never flips.
    pub fn tilt(&mut self, dy: f32) {
        if dy.is_finite() {
            let base = BASE_ELEVATION;
            self.tilt = (self.tilt + dy * 0.008).clamp(-MAX_ELEVATION - base, MAX_ELEVATION - base);
            // a drag past the floor does not pile up: the way back starts at once
            self.tilt = self.elevation() - base;
        }
    }

    /// The orbit's radius: at no tilt, the studio's former eye offset.
    fn radius(&self) -> f32 {
        self.distance * (1.0 + 0.08 * 0.08_f32).sqrt()
    }

    /// The orbit's elevation: the studio's slight look down, plus the tilt.
    /// A tilt below that default stops where the eye would dip under the
    /// stage's floor: the stage's water lies just below it, and its fog would
    /// hide the avatar. A focus lowered far enough to put the default view
    /// under the floor keeps that view, as before the tilt existed.
    fn elevation(&self) -> f32 {
        let wanted = BASE_ELEVATION + self.tilt;
        let radius = self.radius();
        // the eye's height over the floor: the focus, the pan along the
        // tilted view plane, then the orbit's rise
        let eye = |elevation: f32| self.focus_height + self.pan.y * elevation.cos() + radius * elevation.sin();
        if wanted >= BASE_ELEVATION || eye(wanted) >= EYE_CLEARANCE {
            return wanted;
        }
        if eye(BASE_ELEVATION) < EYE_CLEARANCE {
            return BASE_ELEVATION;
        }
        let (mut low, mut high) = (wanted, BASE_ELEVATION);
        for _ in 0..24 {
            let middle = (low + high) * 0.5;
            if eye(middle) >= EYE_CLEARANCE {
                high = middle;
            } else {
                low = middle;
            }
        }
        high
    }

    pub fn view(&self, floor: Vec3, aspect: f32) -> View {
        let rotation = Quat::from_rotation_z(self.yaw);
        let right = rotation * Vec3::Y;
        let (sin, cos) = self.elevation().sin_cos();
        let up = rotation * Vec3::new(-sin, 0.0, cos);
        let target = floor + Vec3::Z * self.focus_height + right * self.pan.x + up * self.pan.y;
        let offset = rotation * Vec3::new(cos, 0.0, sin) * self.radius();
        let eye = target + offset;
        View {
            floor,
            eye,
            target,
            view: glam::camera::rh::view::look_to_mat4(eye, -offset.normalize(), Vec3::Z),
            proj: glam::camera::rh::proj::directx::perspective_infinite_reverse(FOV, aspect.max(0.1), NEAR),
        }
    }

    /// Right-drag adjusts the avatar-relative focus height. Middle-drag
    /// keeps a separate offset in the current viewing plane.
    /// Scale motion to the visible height so close inspection stays precise.
    pub fn pan_vertical(&mut self, dy: f32, viewport_height: f32) {
        if dy.is_finite() && viewport_height.is_finite() && viewport_height >= 1.0 {
            let metres_per_pixel = 2.0 * self.distance * (FOV * 0.5).tan() / viewport_height;
            let next = self.target_height - dy * metres_per_pixel;
            if next.is_finite() {
                self.target_height = next;
            }
        }
    }

    /// Grab the view: translate eye and target together, with no dolly or
    /// orbit. Screen-space offsets rotate with the camera so a later orbit
    /// preserves the chosen composition around the avatar.
    pub fn pan_view(&mut self, dx: f32, dy: f32, viewport_height: f32) {
        if dx.is_finite() && dy.is_finite() && viewport_height.is_finite() && viewport_height >= 1.0 {
            let metres_per_pixel = 2.0 * self.distance * (FOV * 0.5).tan() / viewport_height;
            let next = self.target_pan + Vec2::new(-dx, dy) * metres_per_pixel;
            if next.is_finite() {
                self.target_pan = next;
            }
        }
    }

    /// The wheel changes distance alone, regardless of the pointer location.
    pub fn zoom(&mut self, steps: f32) {
        if steps.is_finite() {
            let next = (f64::from(self.target_distance) * (-f64::from(steps) * 0.15).exp()) as f32;
            if next.is_finite() {
                self.target_distance = next.max(MIN_DISTANCE);
            }
        }
    }

    pub fn inspect_feet(&mut self) {
        self.target_height = 0.08;
        self.zoom(1.0);
    }

    /// The head close up (the shape handles' Head), `at` from the floor's
    /// point the Studio turns around: the head in the middle of the view, a
    /// face's width of view around it, eased like every Studio move. A head
    /// off the turning axis (a feral's, ahead of its body) is reached by the
    /// sideways pan and by moving the focus along the line of sight to the
    /// axis' plane, the distance making up for it; an upright avatar's
    /// head, on the axis, gets no pan.
    pub fn inspect_head(&mut self, at: Vec3) {
        if !at.is_finite() {
            return;
        }
        let rotation = Quat::from_rotation_z(self.yaw);
        let (forward, right) = (rotation * Vec3::X, rotation * Vec3::Y);
        let (sin, cos) = self.elevation().sin_cos();
        // the eye looks along -(forward·cos + up·sin): the focus slides along
        // that line, `along` metres away from the eye, to the axis' plane
        let along = at.dot(forward) / cos.max(0.2);
        let focus = at - (forward * cos + Vec3::Z * sin) * along;
        self.target_height = focus.z;
        self.target_pan = Vec2::new(focus.dot(right), 0.0);
        self.target_distance = (HEAD_DISTANCE + along / (1.0 + 0.08 * 0.08_f32).sqrt()).max(MIN_DISTANCE);
    }

    /// Back to the whole body (the shape handles' Body): the framing entering
    /// Studio gives, eased like the head close up, the orbit kept.
    pub fn inspect_body(&mut self) {
        self.target_height = self.height * 0.5;
        self.target_distance = body_distance(self.height);
        self.target_pan = Vec2::ZERO;
    }

    pub fn update(&mut self, dt: f32) {
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }
        let weight = 1.0 - (-dt.min(0.1) / RESPONSE).exp();
        self.focus_height += (self.target_height - self.focus_height) * weight;
        self.distance += (self.target_distance - self.distance) * weight;
        self.pan += (self.target_pan - self.pan) * weight;
        self.display_hover_height += (self.hover_height - self.display_hover_height) * weight;
        let partner = self.partner_hover_height.unwrap_or(self.hover_height);
        self.display_partner_hover_height += (partner - self.display_partner_hover_height) * weight;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A feral's head, ahead of its body and low, ends in the middle of the
    /// view at a face's distance, whichever way the Studio looks at it.
    #[test]
    fn the_head_close_up_finds_a_head_off_the_turning_axis() {
        for yaw in [0.0, 0.9, 2.5, -1.7] {
            let mut studio = Studio::default();
            studio.enter(yaw, 1.0);
            let head = Vec3::new(0.55, -0.1, 0.7);
            studio.inspect_head(head);
            for _ in 0..300 {
                studio.update(0.05);
            }
            let view = studio.view(Vec3::ZERO, 1.6);
            let sight = (view.target - view.eye).normalize();
            let off = (head - view.eye).reject_from_normalized(sight).length();
            assert!(off < 0.01, "yaw {yaw}: the head {off} m off the line of sight");
            let distance = (head - view.eye).length();
            assert!((distance - HEAD_DISTANCE).abs() < 0.05, "yaw {yaw}: {distance} m from the head");
        }
    }

    #[test]
    fn the_head_close_up_eases_onto_the_head_and_reset_comes_back() {
        let mut studio = Studio::default();
        studio.enter(0.0, 1.8);
        studio.pan_view(40.0, 0.0, 900.0);
        let (body_distance, body_height) = (studio.target_distance, studio.target_height);
        studio.inspect_head(Vec3::new(0.0, 0.0, 1.65));
        for _ in 0..200 {
            studio.update(0.05);
        }
        let view = studio.view(Vec3::ZERO, 1.6);
        assert!((view.target.z - 1.65).abs() < 0.02, "focus on the head: {}", view.target.z);
        assert!(view.eye.distance(view.target) < body_distance, "closer than the body view");
        assert!(studio.pan.length() < 1e-3, "no sideways pan left");
        // a non-finite height changes nothing
        studio.inspect_head(Vec3::NAN);
        assert!((studio.target_height - 1.65).abs() < 1e-6);
        // back to the body: eased, not a jump, the orbit kept
        studio.rotate(50.0);
        let yaw = studio.yaw;
        studio.inspect_body();
        studio.update(0.016);
        assert!(studio.distance < body_distance, "eases out, no jump: {}", studio.distance);
        for _ in 0..200 {
            studio.update(0.05);
        }
        assert!((studio.distance - body_distance).abs() < 1e-3);
        assert!((studio.focus_height - body_height).abs() < 1e-3);
        assert_eq!(studio.yaw, yaw);
        studio.enter(0.0, 1.8);
        assert_eq!((studio.target_distance, studio.target_height), (body_distance, body_height));
    }

    #[test]
    fn person_b_follows_the_user_s_hover_until_given_their_own() {
        let mut studio = Studio {
            hover_height: 0.3,
            ..Studio::default()
        };
        for _ in 0..200 {
            studio.update(0.05);
        }
        assert!((studio.display_partner_hover_height - 0.3).abs() < 1e-4, "B follows the user");
        studio.partner_hover_height = Some(0.8);
        for _ in 0..200 {
            studio.update(0.05);
        }
        assert!((studio.display_partner_hover_height - 0.8).abs() < 1e-4, "B's own hover");
        assert!((studio.display_hover_height - 0.3).abs() < 1e-4, "the user's stays");
    }

    #[test]
    fn photo_handoff_frames_the_studio_and_restores_the_world_camera_and_aim() {
        let lib = std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load());
        let mut world = crate::world::World::new(lib);
        let mut camera = crate::camera::Camera::default();
        camera.roll = 0.15;
        let original = camera.describe();
        let original_position = camera.position;
        let yaw = world.agent.yaw;
        let pitch = world.agent.pitch;
        let settings = crate::camera::CameraSettings::default();
        let mut studio = Studio::default();
        studio.enter(0.7, 1.8);
        let view = studio.view(world.agent.position - Vec3::Z * 0.9, 1.5);
        let session = PhotoSession::enter(&mut camera, &world.agent, &settings, view);
        assert!(camera.position.abs_diff_eq(view.eye, 1e-6));
        assert!(camera.target.abs_diff_eq(view.target, 1e-6));
        camera.update(
            &mut world,
            &settings,
            &crate::camera::FrameInput {
                dt: 1.0 / 60.0,
                now: std::time::Instant::now(),
                steering: false,
                build_mode: false,
                edit_tracking: None,
                eye_offset: None,
                follow_joint_offset: None,
                head_offset: None,
                draw_distance: 128.0,
                region_width: 256.0,
                aspect: 1.5,
            },
        );
        assert!(
            camera.position.abs_diff_eq(view.eye, 1e-4),
            "photo mode must start at the displayed studio camera"
        );
        camera.position += Vec3::X * 2.0;
        camera.roll = 0.8;
        world.agent.yaw += 0.3;
        world.agent.pitch += 0.2;
        let controls = session.controls(&world.agent, &crate::agent::MoveInput::default(), 128.0);
        assert_eq!(controls.camera_center, original_position);
        assert!(controls.body_rotation.abs_diff_eq(glam::Quat::from_rotation_z(yaw), 1e-6));
        session.restore(&mut camera, &mut world.agent);
        assert_eq!(camera.describe(), original);
        assert_eq!(camera.roll, 0.15);
        assert_eq!(world.agent.yaw, yaw);
        assert_eq!(world.agent.pitch, pitch);
    }

    #[test]
    fn middle_drag_translates_eye_and_target_in_the_view_plane_at_any_orbit_angle() {
        for yaw in [0.0, 0.7, 1.8, 3.5] {
            let mut studio = Studio::default();
            studio.enter(yaw, 1.8);
            let original_distance = studio.distance;
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
            assert_eq!(studio.distance, original_distance);
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
    fn display_hover_eases_toward_small_slider_changes_without_overshoot() {
        let mut studio = Studio {
            hover_height: 0.05,
            ..Studio::default()
        };

        studio.update(1.0 / 60.0);
        let first = studio.display_hover_height;
        assert!(first > 0.0 && first < 0.05);

        for _ in 0..120 {
            let previous = studio.display_hover_height;
            studio.update(1.0 / 60.0);
            assert!(studio.display_hover_height >= previous);
            assert!(studio.display_hover_height <= 0.05);
        }
        assert!((studio.display_hover_height - 0.05).abs() < 1e-5);
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
    fn inspection_has_no_framing_limits_and_reset_is_repeatable() {
        let mut studio = Studio::default();
        studio.enter(0.0, 1.8);
        let original_distance = studio.distance;
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
        assert!(studio.distance < 0.01, "zoom reaches beyond the former 35 cm limit");
        assert!(studio.focus_height < -1.8, "focus can move below the avatar");
        studio.enter(0.0, 1.8);
        studio.pan_vertical(-100000.0, 900.0);
        studio.pan_view(100000.0, 100000.0, 900.0);
        assert!(studio.target_height > studio.height);
        assert!(studio.target_pan.length() > studio.height * 2.0);
        let focus = studio.target_height;
        for _ in 0..5 {
            studio.zoom(-10.0);
            studio.update(1.0 / 60.0);
        }
        assert!(studio.target_distance > original_distance);
        assert!(studio.view(Vec3::ZERO, 1.5).view.is_finite());
        assert_eq!(studio.target_height, focus, "zoom out does not change the chosen focus");
        studio.enter(0.0, 1.8);
        assert_eq!(studio.view(Vec3::ZERO, 1.5).eye, original.eye);
    }

    #[test]
    fn a_vertical_drag_tilts_the_orbit_without_flipping_and_entering_resets_it() {
        let mut studio = Studio::default();
        studio.enter(0.0, 1.8);
        let level = studio.view(Vec3::ZERO, 1.5);
        let target = level.target;
        let radius = (level.eye - target).length();
        studio.tilt(f32::NAN);
        assert_eq!(studio.view(Vec3::ZERO, 1.5).eye, level.eye);
        // dragging down looks from higher up, at the same distance and focus
        studio.tilt(60.0);
        let above = studio.view(Vec3::ZERO, 1.5);
        assert!(above.eye.z > level.eye.z + 0.5);
        assert!(((above.eye - above.target).length() - radius).abs() < 1e-4);
        assert!((above.target - target).length() < 1e-5);
        // dragging up goes below the focus, never under the stage's floor
        studio.tilt(-150.0);
        let below = studio.view(Vec3::ZERO, 1.5);
        assert!(below.eye.z < target.z);
        assert!(below.eye.z >= EYE_CLEARANCE - 1e-4, "the eye stays over the floor: {}", below.eye.z);
        // far drags stop short of straight above, and at the floor below
        for dy in [100000.0, -100000.0] {
            studio.tilt(dy);
            let view = studio.view(Vec3::ZERO, 1.5);
            let elevation = ((view.eye - view.target).z / radius).asin();
            assert!(elevation.abs() <= MAX_ELEVATION + 1e-4);
            assert!(view.eye.z >= EYE_CLEARANCE - 1e-4);
            assert!(view.view.is_finite());
        }
        // a drag past the floor does not pile up: the way back starts at once
        let held = studio.view(Vec3::ZERO, 1.5).eye.z;
        studio.tilt(5.0);
        assert!(studio.view(Vec3::ZERO, 1.5).eye.z > held + 0.01);
        // close to a raised focus, the view does go well below it
        studio.pan_vertical(-200.0, 900.0);
        for _ in 0..12 {
            studio.zoom(1.0);
        }
        for _ in 0..200 {
            studio.update(1.0 / 60.0);
        }
        studio.tilt(-100000.0);
        let under = studio.view(Vec3::ZERO, 1.5);
        assert!(((under.eye - under.target).z / (under.eye - under.target).length()).asin() < -1.0);
        assert!(under.eye.z >= EYE_CLEARANCE - 1e-4);
        // a focus lowered under the floor keeps the default view, tilt or not
        let mut low = Studio::default();
        low.enter(0.0, 1.8);
        low.pan_vertical(100000.0, 900.0);
        for _ in 0..200 {
            low.update(1.0 / 60.0);
        }
        let before = low.view(Vec3::ZERO, 1.5).eye;
        low.tilt(-300.0);
        assert_eq!(low.view(Vec3::ZERO, 1.5).eye, before);
        studio.enter(0.0, 1.8);
        // a pan still moves within the tilted view plane
        studio.tilt(80.0);
        let before = studio.view(Vec3::ZERO, 1.5);
        studio.pan_view(0.0, -120.0, 900.0);
        for _ in 0..200 {
            studio.update(1.0 / 60.0);
        }
        let after = studio.view(Vec3::ZERO, 1.5);
        let forward = (before.target - before.eye).normalize();
        assert!((after.target - before.target).dot(forward).abs() < 1e-4);
        studio.enter(0.0, 1.8);
        assert_eq!(studio.view(Vec3::ZERO, 1.5).eye, level.eye);
    }
}
