//! The camera: third person behind the avatar, alt-camera on a point or an
//! object, sit camera, mouselook, and the animated transitions between them.
//!
//! Port of LLAgentCamera (indra/newview/llagentcamera.cpp), of the camera
//! drag of LLToolCamera (lltoolfocus.cpp), of LLAgent::pitch (llagent.cpp)
//! and of process_camera_constraint / process_avatar_sit_response
//! (llviewermessage.cpp), originally LGPL 2.1.
//!
//! Deliberate difference: `orbit_yaw` turns the camera around the avatar
//! without turning the avatar (right drag, arrows while the left button is
//! held on the avatar); Firestorm turns the agent frame instead. Moving or
//! Escape folds it back the way resetView does for an alt-orbited camera.

pub mod demo;
mod focus;
pub(crate) mod follow;
pub mod free;
mod mouselook_zoom;
pub mod presets;
mod settings;

pub use focus::FocusObject;
pub use free::FreeCamera;
pub use settings::{CameraPreset, CameraSettings, CameraShot, DEFAULT_FOV, FOV_RANGE, VrView, zoom_lens};

/// Reference for a rolled view, shared with flycam handoff and AgentUpdate.
/// At a vertical pole the world Z axis cannot define an up direction.
pub(crate) fn view_reference_up(direction: Vec3) -> Vec3 {
    if direction.cross(Vec3::Z).length_squared() < 1e-6 {
        Vec3::Y
    } else {
        Vec3::Z
    }
}

use crate::agent::AgentState;
use crate::scene::Scene;
use crate::world::World;
use glam::{Mat4, Quat, Vec3, Vec4};
use std::f32::consts::{PI, TAU};
use std::time::Instant;
use uuid::Uuid;

// llagentcamera.cpp
const MIN_ZOOM_FRACTION: f32 = 0.25;
const MAX_ZOOM_FRACTION: f32 = 8.0;
const CAMERA_ZOOM_HALF_LIFE: f32 = 0.07;
const FOV_ZOOM_HALF_LIFE: f32 = 0.07;
const CAMERA_LAG_HALF_LIFE: f32 = 0.25;
const MIN_CAMERA_LAG: f32 = 0.5;
const MAX_CAMERA_LAG: f32 = 5.0;
const CAMERA_COLLIDE_EPSILON: f32 = 0.1;
const MIN_CAMERA_DISTANCE: f32 = 0.1;
const MAX_CAMERA_DISTANCE_FROM_OBJECT: f32 = 496.0;
const CAMERA_FUDGE_FROM_OBJECT: f32 = 16.0;
const MAX_CAMERA_SMOOTH_DISTANCE: f32 = 50.0;
const HEAD_BUFFER_SIZE: f32 = 0.3;
const LAND_MIN_ZOOM: f32 = 0.15;
const AVATAR_MIN_ZOOM: f32 = 0.5;
const OBJECT_MIN_ZOOM: f32 = 0.02;
const GROUND_TO_AIR_CAMERA_TRANSITION_TIME: f32 = 0.5;
const GROUND_TO_AIR_CAMERA_TRANSITION_START_TIME: f32 = 0.5;
const SMOOTHING_HALF_LIFE: f32 = 0.02;
const REGION_WIDTH: f32 = 256.0;
/// LLAgentCamera::updateCamera keyboard rates.
const ORBIT_RATE: f32 = PI / 2.0;
const PAN_RATE: f32 = 5.0;
/// LLToolCamera: pixels before a drag moves the camera.
const SLOP_RANGE: f32 = 4.0;
/// CAMERA_POSITION_THRESHOLD_SQUARED (llviewermessage.cpp).
const SIT_CAMERA_THRESHOLD_SQUARED: f32 = 0.001 * 0.001;
/// Avatar.ZoomIn framing (CameraZoomDistance, CameraZoomEyeZOffset,
/// CameraZoomFocusZOffset of settings.xml).
const AVATAR_ZOOM_DISTANCE: f32 = 2.5;
const AVATAR_ZOOM_EYE_Z: f32 = 1.0;
const AVATAR_ZOOM_FOCUS_Z: f32 = 0.6;
/// Aurora's right-drag orbit rate (radians per pixel at sensitivity 1).
pub(crate) const DRAG_RATE: f32 = 0.0042;

/// LLSmoothInterpolation::getInterpolant.
fn interpolant(half_life: f32, dt: f32) -> f32 {
    if half_life <= 0.0 {
        return 1.0;
    }
    (1.0 - 2f32.powf(-dt / half_life)).clamp(0.0, 1.0)
}

fn smoothstep(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

fn angle_between(a: Vec3, b: Vec3) -> f32 {
    let (a, b) = (a.normalize_or_zero(), b.normalize_or_zero());
    a.dot(b).clamp(-1.0, 1.0).acos()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraMode {
    ThirdPerson,
    Follow,
    Mouselook,
}

/// Alt-camera drag (LLToolCamera::handleHover): Alt = zoom (and turn
/// around), Ctrl+Alt = orbit, Ctrl+Alt+Shift = pan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolMode {
    Zoom,
    Orbit,
    Pan,
}

impl ToolMode {
    pub fn from_mods(ctrl: bool, shift: bool) -> ToolMode {
        match (ctrl, shift) {
            (true, true) => ToolMode::Pan,
            (true, false) => ToolMode::Orbit,
            _ => ToolMode::Zoom,
        }
    }
}

/// Camera keys held this frame, each with its rate (0 = up, ramping to 1
/// over the first 0.25 s like get_orbit_rate).
#[derive(Debug, Clone, Copy, Default)]
pub struct CameraKeys {
    pub orbit_left: f32,
    pub orbit_right: f32,
    pub orbit_up: f32,
    pub orbit_down: f32,
    pub zoom_in: f32,
    pub zoom_out: f32,
    pub pan_left: f32,
    pub pan_right: f32,
    pub pan_up: f32,
    pub pan_down: f32,
}

impl CameraKeys {
    fn any(&self) -> bool {
        [
            self.orbit_left,
            self.orbit_right,
            self.orbit_up,
            self.orbit_down,
            self.zoom_in,
            self.zoom_out,
            self.pan_left,
            self.pan_right,
            self.pan_up,
            self.pan_down,
        ]
        .iter()
        .any(|r| *r > 0.0)
    }
}

/// What the frame update needs besides the world.
pub struct FrameInput {
    pub dt: f32,
    pub now: Instant,
    /// Left button held on the avatar (mouse steering): no flight lag.
    pub steering: bool,
    /// Build tools open: no smoothing (LLToolMgr::inBuildMode).
    pub build_mode: bool,
    /// Focus object belongs to the edit selection: follow it only on request.
    pub edit_tracking: Option<bool>,
    /// Animated point between the eyes relative to the rendered avatar root.
    /// Sitting and optional eye following consume it after skeleton evaluation.
    pub eye_offset: Option<Vec3>,
    pub draw_distance: f32,
    pub aspect: f32,
}

/// Our avatar this frame (render space).
#[derive(Debug, Clone, Copy)]
struct AvatarFrame {
    /// getAvatarRootPosition: the root joint, hover height removed unless
    /// HoverHeightAffectsCamera.
    root: Vec3,
    /// gAgent.getPositionGlobal: the avatar object's position.
    position: Vec3,
    /// Scripted follow also respects the hover-camera preference.
    follow_root: Vec3,
    /// Seated avatar render rotation, including its local rotation.
    rotation: Option<Quat>,
    /// Complete rendered root, including hover even when camera hover is off.
    render_root: Vec3,
    /// Rendered body rotation, also available without a seat parent.
    render_rotation: Quat,
    /// Rotation of the object we sit on.
    seat: Option<Quat>,
}

#[derive(Debug, Clone, Copy)]
struct Animation {
    start_camera: Vec3,
    start_focus: Vec3,
    elapsed: f32,
    duration: f32,
    orbit: bool,
}

fn orbit_angle(from: Vec3, to: Vec3) -> f32 {
    let angle = to.y.atan2(to.x) - from.y.atan2(from.x);
    if angle > PI {
        angle - TAU
    } else if angle < -PI {
        angle + TAU
    } else {
        angle
    }
}

impl Animation {
    fn camera(&self, camera: Vec3, focus: Vec3, k: f32) -> Vec3 {
        if !self.orbit || k <= 0.0 || k >= 1.0 || (self.start_camera == camera && self.start_focus == focus) {
            return self.start_camera.lerp_monotonic(camera, k);
        }
        let from = self.start_camera - self.start_focus;
        let to = camera - focus;
        let (start_distance, end_distance) = (from.length(), to.length());
        if start_distance < MIN_CAMERA_DISTANCE || end_distance < MIN_CAMERA_DISTANCE {
            return self.start_camera.lerp_monotonic(camera, k);
        }
        // Appearance's half-turn must go around the avatar, not through it.
        // Interpolate elevation and distance independently of the azimuth.
        let yaw = from.y.atan2(from.x) + orbit_angle(from, to) * k;
        let pitch = from.z.atan2(from.truncate().length());
        let pitch = pitch + (to.z.atan2(to.truncate().length()) - pitch) * k;
        let distance = start_distance + (end_distance - start_distance) * k;
        self.start_focus.lerp_monotonic(focus, k) + Vec3::new(yaw.cos() * pitch.cos(), yaw.sin() * pitch.cos(), pitch.sin()) * distance
    }
}

/// llSetCameraEyeOffset / llSetCameraAtOffset of the object we sit on.
#[derive(Debug, Clone, Copy)]
struct SitCamera {
    object: Uuid,
    eye: Vec3,
    at: Vec3,
}

/// While seated the agent frame is relative to the seat (setupSitCamera).
#[derive(Debug, Clone, Copy)]
struct Seat {
    local_yaw: f32,
    written_yaw: f32,
}

/// An alt-camera drag in progress (LLToolCamera).
#[derive(Debug, Clone, Copy, Default)]
struct Tool {
    accum_x: f32,
    accum_y: f32,
    outside_x: bool,
    outside_y: bool,
    steering: bool,
}

#[derive(Clone)]
pub struct Camera {
    pub mode: CameraMode,
    last_mode: CameraMode,
    /// Final camera position and point looked at (render space).
    pub position: Vec3,
    pub target: Vec3,
    pub fov_y: f32,
    pub near: f32,
    /// Camera heading relative to the avatar (see the module note).
    pub orbit_yaw: f32,
    /// Pitch of the agent frame in third person (LLAgent::pitch), radians,
    /// positive looking down.
    pitch: f32,
    focus_on_avatar: bool,
    /// mFocusTargetGlobal.
    focus_target: Vec3,
    focus_object: Option<Uuid>,
    focus_geom: Option<FocusObject>,
    focus_object_offset: Vec3,
    /// mCameraFocusOffsetTarget / mCameraFocusOffset: camera minus focus.
    focus_offset_target: Vec3,
    focus_offset: Vec3,
    zoom_fraction: f32,
    target_distance: f32,
    current_distance: f32,
    lag: Vec3,
    time_in_air: f32,
    collide_plane: Vec4,
    sit_camera: Option<SitCamera>,
    follow: follow::Simulation,
    follow_available: bool,
    force_mouselook: bool,
    seat: Option<Seat>,
    fov_zoom: f32,
    current_fov_zoom: f32,
    /// The right button held in mouselook (`mouselook_zoom.rs`).
    ml_zoom: mouselook_zoom::MouselookZoom,
    /// mCameraVirtualPositionAgent: camera target before the FOV push-out.
    virtual_position: Vec3,
    anim: Option<Animation>,
    smoothing_last_global: Vec3,
    smoothing_last_agent: Vec3,
    smoothing_stop: bool,
    tool: Option<Tool>,
    avatar: AvatarFrame,
    head: Vec3,
    draw_distance: f32,
    /// DisableCameraConstraints, as of the last frame.
    unconstrained: bool,
    /// Our avatar's id (the alt-camera focus object of unlockView).
    own: Uuid,
    dt: f32,
    initialized: bool,
    /// The photo window's free camera: while set, the view is its own and
    /// the rest of the camera waits behind the avatar (`free.rs`).
    free: Option<FreeCamera>,
    pub flycam: Option<crate::joystick::flycam::Flycam>,
    /// Lens retained when flycam stops, until a reset or avatar movement.
    flycam_lens: Option<(f32, f32)>,
    /// A passing pan of the view only (the region restart's shake,
    /// `world::region_restart::view_shake`): the eye and what it looks at
    /// move together, the camera itself does not.
    pub shake: Vec3,
    /// Roll of the view around its direction (radians, positive turns the
    /// picture clockwise): the photo viewfinder's tilt, 0 elsewhere.
    pub roll: f32,
}

impl Default for Camera {
    fn default() -> Self {
        let s = CameraSettings::default();
        let distance = s.camera_offset().length() * s.offset_scale;
        let start = Vec3::new(128.0, 128.0, 25.0);
        Camera {
            mode: CameraMode::ThirdPerson,
            last_mode: CameraMode::ThirdPerson,
            position: Vec3::new(120.0, 128.0, 30.0),
            target: Vec3::new(128.0, 128.0, 28.0),
            fov_y: DEFAULT_FOV,
            near: 0.1,
            orbit_yaw: 0.0,
            pitch: 0.0,
            focus_on_avatar: true,
            focus_target: Vec3::ZERO,
            focus_object: None,
            focus_geom: None,
            focus_object_offset: Vec3::ZERO,
            // CameraOffsetBuild
            focus_offset_target: Vec3::new(-6.0, 0.0, 6.0),
            focus_offset: Vec3::new(-6.0, 0.0, 6.0),
            zoom_fraction: 1.0,
            target_distance: distance,
            current_distance: distance,
            lag: Vec3::ZERO,
            time_in_air: 0.0,
            collide_plane: Vec4::ZERO,
            sit_camera: None,
            follow: Default::default(),
            follow_available: false,
            force_mouselook: false,
            seat: None,
            fov_zoom: 0.0,
            current_fov_zoom: 0.0,
            ml_zoom: Default::default(),
            virtual_position: Vec3::ZERO,
            anim: None,
            smoothing_last_global: Vec3::ZERO,
            smoothing_last_agent: Vec3::ZERO,
            smoothing_stop: false,
            tool: None,
            avatar: AvatarFrame {
                root: start,
                position: start,
                follow_root: start,
                rotation: None,
                render_root: start,
                render_rotation: Quat::IDENTITY,
                seat: None,
            },
            head: start + Vec3::Z * 0.7,
            draw_distance: 128.0,
            unconstrained: false,
            own: Uuid::nil(),
            dt: 0.0,
            initialized: false,
            free: None,
            flycam: None,
            flycam_lens: None,
            shake: Vec3::ZERO,
            roll: 0.0,
        }
    }
}

impl Camera {
    pub fn mouselook(&self) -> bool {
        self.mode == CameraMode::Mouselook
    }

    /// LLAgentCamera::cameraMouselook: visibility changes only once the
    /// incoming transition reaches the head buffer, not on the mode request.
    pub fn first_person_view(&self) -> bool {
        self.mouselook() && self.last_mode == CameraMode::Mouselook
    }

    pub fn needs_animated_eyes(&self, agent: &AgentState, s: &CameraSettings) -> bool {
        self.mouselook() && (agent.is_sitting() || s.mouselook_follow_eyes)
    }

    pub fn forward(&self) -> Vec3 {
        (self.target - self.position).normalize_or(Vec3::X)
    }

    /// The point the camera is locked on (alt-camera), if not the avatar.
    pub fn focus_point(&self) -> Option<Vec3> {
        (!self.focus_on_avatar).then_some(self.focus_target)
    }

    pub fn focus_object_id(&self) -> Option<Uuid> {
        (!self.focus_on_avatar).then_some(self.focus_object).flatten()
    }

    pub fn third_person(&self) -> bool {
        self.mode == CameraMode::ThirdPerson
    }

    fn left_axis(&self) -> Vec3 {
        Vec3::Z.cross(self.forward()).normalize_or(Vec3::Y)
    }

    fn up_axis(&self) -> Vec3 {
        self.forward().cross(self.left_axis()).normalize_or(Vec3::Z)
    }

    /// The agent frame: heading and pitch, in the seat's frame when seated.
    fn frame_rotation(&self, agent: &AgentState) -> Quat {
        let pitch = Quat::from_rotation_y(self.pitch);
        match (self.avatar.seat, self.seat) {
            (Some(seat), Some(s)) => seat * Quat::from_rotation_z(s.local_yaw + self.orbit_yaw) * pitch,
            _ => Quat::from_rotation_z(agent.yaw + self.orbit_yaw) * pitch,
        }
    }

    /// LLAgentCamera::getCurrentCameraOffset and getCurrentFocusOffset:
    /// the camera and the point it looks at, in the avatar's frame (a saved
    /// camera position).
    pub fn current_offsets(&self, agent: &AgentState) -> (Vec3, Vec3) {
        let rotation = match (self.avatar.seat, self.seat) {
            (Some(seat), Some(s)) => seat * Quat::from_rotation_z(s.local_yaw),
            _ => Quat::from_rotation_z(agent.yaw),
        };
        let inverse = rotation.inverse();
        // mThirdPersonHeadOffset, as `update` places the camera
        let head = if self.avatar.seat.is_some() { Vec3::ZERO } else { Vec3::Z };
        (
            inverse * (self.position - self.avatar.root - head),
            inverse * (self.target - self.avatar.position),
        )
    }

    // ------------------------------------------------------------ transitions

    /// LLAgentCamera::startCameraAnimation.
    fn start_animation(&mut self, s: &CameraSettings) {
        self.anim = Some(Animation {
            start_camera: self.position,
            start_focus: self.target,
            elapsed: 0.0,
            duration: 0.0,
            orbit: false,
        });
        self.set_animation_duration(s.zoom_time, true);
    }

    /// LLAgentCamera::setAnimationDuration: never cut a running animation short.
    fn set_animation_duration(&mut self, duration: f32, restart: bool) {
        if let Some(a) = &mut self.anim {
            a.duration = if restart {
                duration
            } else {
                duration.max((a.duration - a.elapsed).max(0.0))
            };
        }
    }

    pub fn toggle_mouselook(&mut self, agent: &mut AgentState, s: &CameraSettings) {
        if self.mouselook() {
            self.change_to_default(agent, s);
        } else {
            self.change_to_mouselook(agent, s, true);
        }
    }

    /// LLAgentCamera::changeCameraToMouselook.
    pub fn change_to_mouselook(&mut self, agent: &mut AgentState, s: &CameraSettings, animate: bool) {
        if !s.enable_mouselook || self.mouselook() {
            return;
        }
        // mouselook ends the free camera (the flycam's toggle the other way)
        self.free = None;
        self.clear_flycam();
        // the view keeps its direction: the avatar turns to it
        agent.yaw += self.orbit_yaw;
        self.orbit_yaw = 0.0;
        agent.pitch = -self.pitch;
        self.pitch = 0.0;
        self.last_mode = self.mode;
        self.mode = CameraMode::Mouselook;
        if animate {
            self.start_animation(s);
        } else {
            self.anim = None;
        }
    }

    /// LLAgentCamera::changeCameraToDefault.
    pub fn change_to_default(&mut self, agent: &mut AgentState, s: &CameraSettings) {
        if self.follow_available {
            self.change_to_follow(agent, s);
        } else {
            self.change_to_third_person(agent, s, true);
        }
    }

    /// The viewing angle before the FOV zoom: the held mouselook zoom's, or
    /// the setting's (CameraAngle).
    fn base_fov(&self, s: &CameraSettings) -> f32 {
        match self.ml_zoom.angle() {
            Some(angle) if self.mouselook() => angle,
            _ => self
                .flycam_lens
                .filter(|(original, _)| *original == s.fov)
                .map_or(s.fov, |(_, lens)| lens),
        }
    }

    /// The viewing angle narrowed by the field of view correction
    /// (LLAgentCamera::updateCamera), never under LLCamera::getMinView (5°,
    /// more in a narrow window) as LLViewerCamera::setView caps it: the
    /// correction alone goes down to a thousandth of the angle, a sliver of
    /// the focused object filling the screen. An angle already narrower
    /// (`base_fov`) stays as it is.
    fn zoomed_fov(&self, s: &CameraSettings, aspect: f32) -> f32 {
        let base = self.base_fov(s);
        (base / (1.0 + self.current_fov_zoom)).max(mouselook_zoom::min_view(aspect).min(base))
    }

    /// Right button down in mouselook (LLToolCompGun::handleRightMouseDown):
    /// the remembered zoomed angle at once. `aspect`: the window's.
    pub fn press_mouselook_zoom(&mut self, s: &CameraSettings, aspect: f32) {
        if self.mouselook() {
            self.ml_zoom.press(s.mouselook_zoom_fov, s.fov, aspect);
        }
    }

    /// Wheel clicks (SL sign) while the zoom is held
    /// (LLToolCompGun::handleScrollWheel); false when it is not.
    pub fn scroll_mouselook_zoom(&mut self, clicks: f32, aspect: f32) -> bool {
        self.mouselook() && self.ml_zoom.scroll(clicks, aspect)
    }

    /// Right button up (LLToolCompGun::handleRightMouseUp), or mouselook left
    /// while it is held: the normal angle comes back, and the zoomed one is
    /// remembered in `s` when the wheel set it. True when it was held.
    pub fn release_mouselook_zoom(&mut self, s: &mut CameraSettings) -> bool {
        match self.ml_zoom.release() {
            Some(hold) => {
                if hold.scrolled {
                    s.mouselook_zoom_fov = Some(hold.angle);
                }
                true
            }
            None => false,
        }
    }

    pub fn mouselook_zoom_held(&self) -> bool {
        self.ml_zoom.angle().is_some()
    }

    fn follow_rotation(&self, agent: &AgentState) -> Quat {
        // Seated follow cam uses the avatar's render rotation, including
        // the seat and avatar local rotation (LLAgentCamera::updateCamera).
        self.avatar.rotation.unwrap_or_else(|| self.frame_rotation(agent))
    }

    fn change_to_follow(&mut self, agent: &AgentState, s: &CameraSettings) {
        if self.mode == CameraMode::Follow {
            return;
        }
        let animate = !self.mouselook();
        self.last_mode = self.mode;
        self.mode = CameraMode::Follow;
        self.fov_zoom = 0.0;
        // reset uses the current view, so activation does not jump.
        self.follow
            .reset(self.position, self.target, self.avatar.follow_root, self.follow_rotation(agent));
        if animate {
            self.start_animation(s);
        } else {
            self.anim = None;
        }
    }

    /// LLAgentCamera::changeCameraToThirdPerson.
    fn change_to_third_person(&mut self, agent: &mut AgentState, s: &CameraSettings, animate: bool) {
        let mut animate = animate;
        self.zoom_fraction = 1.0;
        if self.mode != CameraMode::ThirdPerson {
            self.lag = Vec3::ZERO;
            if self.mode == CameraMode::Mouselook {
                // zooms out of the head
                self.current_distance = MIN_CAMERA_DISTANCE;
                self.target_distance = MIN_CAMERA_DISTANCE;
                animate = false;
            }
            self.last_mode = self.mode;
            self.mode = CameraMode::ThirdPerson;
        }
        // remove any pitch from the avatar
        if self.avatar.seat.is_none() {
            self.pitch = 0.0;
            agent.pitch = 0.0;
        }
        if animate {
            self.start_animation(s);
        } else {
            self.anim = None;
        }
    }

    /// LLAgentCamera::setFocusOnAvatar.
    pub fn set_focus_on_avatar(&mut self, on: bool, animate: bool, reset_axes: bool, agent: &mut AgentState, s: &CameraSettings) {
        if on != self.focus_on_avatar {
            if animate {
                self.start_animation(s);
            } else {
                self.anim = None;
            }
        }
        if !self.focus_on_avatar && on && reset_axes {
            self.set_focus_global(None, None, None, s);
            self.fov_zoom = 0.0;
            if self.third_person() && self.avatar.seat.is_none() {
                // rear view: the avatar turns to where the camera looks;
                // front view: to the camera
                let off = s.camera_offset();
                let rotxy = off.y.atan2(off.x);
                let at = self.forward();
                if at.truncate().length_squared() > 1e-6 {
                    agent.yaw = at.y.atan2(at.x) + PI - rotxy;
                }
                self.orbit_yaw = 0.0;
                self.pitch = 0.0;
            }
        } else if self.focus_on_avatar && !on {
            // keep the focus point where it was, now unlocked
            let focus = self.avatar.position + self.frame_rotation(agent) * s.focus_offset();
            self.focus_on_avatar = false;
            self.set_focus_global(Some(focus), None, None, s);
        }
        self.focus_on_avatar = on;
    }

    /// LLAgentCamera::setFocusGlobal. `None` = our head.
    fn set_focus_global(&mut self, focus: Option<Vec3>, object: Option<Uuid>, geom: Option<FocusObject>, s: &CameraSettings) {
        self.focus_object = object;
        self.focus_geom = geom;
        match focus {
            None => {
                self.focus_target = self.head;
                self.focus_offset_target = self.position - self.focus_target;
                self.focus_offset = self.focus_offset_target;
            }
            Some(p) if p != self.focus_target => {
                self.focus_target = p;
                if object.is_none() {
                    self.fov_zoom = 0.0;
                }
                self.focus_offset_target = self.virtual_position - p;
                self.start_animation(s);
            }
            Some(p) => {
                self.focus_offset_target = (self.position - p) / (1.0 + self.fov_zoom);
                self.focus_offset = self.focus_offset_target;
            }
        }
        self.update_focus_offset();
    }

    /// LLAgentCamera::setCameraPosAndFocusGlobal.
    fn set_camera_pos_and_focus(&mut self, camera: Vec3, focus: Vec3, object: Option<(Uuid, FocusObject)>, s: &CameraSettings) {
        let old = if self.focus_target == Vec3::ZERO {
            focus
        } else {
            self.focus_target
        };
        let d2 = (old - focus).length_squared();
        if d2 > 0.0001 {
            self.start_animation(s);
        }
        self.focus_object = object.map(|o| o.0);
        self.focus_geom = object.map(|o| o.1);
        self.focus_target = focus;
        self.focus_offset_target = camera - focus;
        self.focus_offset = self.focus_offset_target;
        if self.anim.is_some() {
            // 10 m/s, 0.5 to 1 s (Andromeda radar cam patch)
            self.set_animation_duration((d2.sqrt() / 10.0).clamp(0.5, 1.0), false);
        }
        self.update_focus_offset();
    }

    fn update_focus_offset(&mut self) {
        if let Some(g) = self.focus_geom {
            self.focus_object_offset = self.focus_target - g.position;
        }
    }

    /// LLAgentCamera::unlockView: alt-camera on our head.
    fn unlock_view(&mut self, agent: &mut AgentState, s: &CameraSettings) {
        if self.focus_on_avatar {
            let own = (!self.own.is_nil()).then_some(self.own);
            self.set_focus_global(None, own, None, s);
            self.set_focus_on_avatar(false, false, true, agent, s);
        }
    }

    /// Alt+click (LLToolCamera::pickCallback): lock the camera on the
    /// clicked point, inside the clicked object when there is one.
    pub fn alt_focus(
        &mut self,
        point: Vec3,
        object: Option<(Uuid, FocusObject)>,
        ray: (Vec3, Vec3),
        agent: &mut AgentState,
        s: &CameraSettings,
    ) {
        self.set_focus_on_avatar(false, true, true, agent, s);
        let focus = match object {
            Some((_, g)) => {
                let virtual_camera = self.focus_target + (self.position - self.focus_target) / (1.0 + self.fov_zoom);
                g.position + focus::calc_focus_offset(&g, point, ray, self.position, self.forward(), virtual_camera)
            }
            None => point,
        };
        self.set_focus_global(Some(focus), object.map(|o| o.0), object.map(|o| o.1), s);
    }

    /// `pstcampos` chat command (cmdline_apply_camera): unlockView, then
    /// setCameraPosAndFocusGlobal.
    pub fn set_view(&mut self, camera: Vec3, focus: Vec3, agent: &mut AgentState, s: &CameraSettings) {
        self.unlock_view(agent, s);
        self.set_camera_pos_and_focus(camera, focus, None, s);
    }

    /// Appearance framing, including a changed eye with an unchanged focus.
    /// Like changeCameraToCustomizeAvatar (llagentcamera.cpp), allow time for
    /// the 1.35 s customise turn. Other camera operations retain their paths.
    pub fn set_appearance_view(&mut self, camera: Vec3, focus: Vec3, agent: &mut AgentState, s: &CameraSettings) {
        self.set_view(camera, focus, agent, s);
        self.start_animation(s);
        if let Some(a) = &mut self.anim {
            a.orbit = true;
            if s.zoom_time > 0.0 {
                a.duration = s
                    .zoom_time
                    .max(orbit_angle(a.start_camera - a.start_focus, camera - focus).abs() / PI * 1.35);
            }
        }
    }

    /// RlvForceHandler<RLV_BHVR_SETCAM_FOCUS>: retain the exact object UUID
    /// for tracking; avatar requests focus on the posed head supplied by Scene.
    pub fn set_script_focus(
        &mut self,
        camera: Vec3,
        focus: Vec3,
        object: Option<(Uuid, FocusObject)>,
        agent: &mut AgentState,
        s: &CameraSettings,
    ) {
        self.set_focus_on_avatar(false, true, true, agent, s);
        self.set_camera_pos_and_focus(camera, focus, object, s);
    }

    /// Context menu "Zoom" (handle_look_at_selection "zoom"): the focus stays
    /// attached to `object` when there is one, so the camera follows it as
    /// it moves (TrackFocusObject).
    pub fn zoom_to(&mut self, point: Vec3, object: Option<(Uuid, FocusObject)>, agent: &mut AgentState, s: &CameraSettings) {
        self.set_focus_on_avatar(false, true, true, agent, s);
        let dir = (self.position - point).normalize_or(-self.forward());
        self.set_camera_pos_and_focus(point + dir * 3.0, point, object, s);
    }

    /// Avatar.ZoomIn from a list (handle_zoom_to_object, llviewermenu.cpp):
    /// the camera faces the avatar, a little above its centre, and keeps it
    /// as the focus object so it follows the avatar as it moves.
    pub fn zoom_avatar(&mut self, avatar: (Uuid, FocusObject), agent: &mut AgentState, s: &CameraSettings) {
        self.set_focus_on_avatar(false, true, true, agent, s);
        let g = avatar.1;
        let eye = g.position + g.rotation * Vec3::new(AVATAR_ZOOM_DISTANCE, 0.0, AVATAR_ZOOM_EYE_Z);
        let focus = g.position + Vec3::Z * AVATAR_ZOOM_FOCUS_Z;
        self.set_camera_pos_and_focus(eye, focus, Some(avatar), s);
    }

    /// LLToolPie CLICK_ACTION_ZOOM: fit the clicked bounding box, keep its
    /// focus attached to the object, and animate from the current camera.
    pub fn zoom_object(
        &mut self,
        center: Vec3,
        extent: Vec3,
        object: Option<(Uuid, FocusObject)>,
        aspect: f32,
        agent: &mut AgentState,
        s: &CameraSettings,
    ) {
        self.set_focus_on_avatar(false, true, true, agent, s);
        let angle = (self.fov_y * aspect.max(1.0)).max(0.1);
        let distance = extent.length() * 2.0 / angle.atan();
        let dir = (self.position - center).normalize_or(-self.forward());
        self.set_camera_pos_and_focus(center + dir * distance.max(0.1), center, object, s);
    }

    /// "Reset the camera" of the preferences: the view goes back with the
    /// settings. What a shortcut left on the follow camera (orbit, pitch,
    /// zoom, field-of-view zoom) is dropped without turning the avatar, then
    /// the view returns behind it (LLAgentCamera::resetView).
    pub fn reset_to_defaults(&mut self, agent: &mut AgentState, s: &CameraSettings) {
        if self.third_person() {
            self.orbit_yaw = 0.0;
            self.pitch = 0.0;
        }
        self.zoom_fraction = 1.0;
        self.fov_zoom = 0.0;
        self.reset_view(agent, s, true, true, false, false);
    }

    /// LLAgentCamera::resetView: back behind the avatar. `movement` = called
    /// by a movement key (FSResetCameraOnMovement); `steering` = left button
    /// held on the avatar.
    pub fn reset_view(
        &mut self,
        agent: &mut AgentState,
        s: &CameraSettings,
        reset_camera: bool,
        change_camera: bool,
        movement: bool,
        steering: bool,
    ) {
        if movement && !s.reset_on_movement {
            return;
        }
        self.clear_flycam();
        if change_camera {
            self.change_to_default(agent, s);
        }
        if reset_camera {
            if !steering && self.third_person() {
                // leaving mouse steering: the pitch fades out
                self.pitch *= 1.0 - interpolant(0.3, self.dt);
            }
            // steering too: the avatar walks where the camera looks
            self.fold_orbit(agent);
            self.set_focus_on_avatar(true, true, true, agent, s);
            self.fov_zoom = 0.0;
        }
    }

    /// The avatar turns to where the orbited follow camera looks; the camera
    /// stays put.
    pub fn fold_orbit(&mut self, agent: &mut AgentState) {
        if self.third_person() && self.orbit_yaw != 0.0 {
            agent.yaw += self.orbit_yaw;
            self.orbit_yaw = 0.0;
        }
    }

    /// handle_reset_view (Escape, "Reset camera").
    pub fn handle_reset_view(&mut self, agent: &mut AgentState, s: &CameraSettings) {
        if !s.reset_view_turns_avatar {
            // setFocusOnAvatar(true, false, false): the avatar keeps its direction
            if !self.focus_on_avatar {
                self.anim = None;
                self.focus_object = None;
                self.focus_geom = None;
            }
            self.focus_on_avatar = true;
            self.orbit_yaw = 0.0;
        }
        self.reset_view(agent, s, true, true, false, false);
    }

    /// The free camera, while it flies.
    pub fn free(&self) -> Option<&FreeCamera> {
        self.free.as_ref()
    }

    pub fn free_mut(&mut self) -> Option<&mut FreeCamera> {
        self.free.as_mut()
    }

    pub fn start_flycam(&mut self, s: &CameraSettings) {
        let up = Quat::from_axis_angle(self.forward(), self.roll) * Vec3::Z;
        self.flycam = Some(crate::joystick::flycam::Flycam::from_view(
            self.position,
            self.target,
            up,
            self.fov_y,
        ));
        self.flycam_lens = Some((s.fov, self.fov_y));
    }

    /// On toggle-off the normal camera takes the exact view back, still
    /// available to Alt-orbit and the camera controls (toggleFlycam).
    pub fn retain_flycam_view(&mut self, agent: &mut AgentState, s: &CameraSettings) {
        if let Some(pose) = self.flycam.take() {
            self.set_view(pose.position, pose.position + pose.forward(), agent, s);
            self.roll = pose.roll();
            self.flycam_lens = Some((s.fov, pose.fov));
            self.fov_zoom = 0.0;
            self.current_fov_zoom = 0.0;
            self.virtual_position = pose.position;
            self.smoothing_stop = true;
            self.anim = None;
        }
    }

    pub fn clear_flycam(&mut self) {
        let pose = self.flycam.take().is_some();
        let lens = self.flycam_lens.take().is_some();
        if pose || lens {
            self.roll = 0.0;
        }
    }

    /// The camera leaves the avatar where it is, looking where it looks
    /// (moveFlycam's reset); mouselook ends first, like toggleFlycam's
    /// changeCameraToDefault.
    pub fn start_free(&mut self, agent: &mut AgentState, s: &CameraSettings) {
        self.clear_flycam();
        if self.free.is_some() {
            return;
        }
        if self.mouselook() {
            self.change_to_default(agent, s);
        }
        self.free = Some(FreeCamera::from_view(self.position, self.target));
    }

    /// Back to the avatar, gliding from where the free camera was.
    pub fn end_free(&mut self, s: &CameraSettings) {
        if self.free.take().is_some() {
            // the smoothing restarts from here, not from before the flight
            self.smoothing_stop = true;
            self.start_animation(s);
        }
    }

    /// Back to the default view without animation (login, teleport).
    pub fn snap(&mut self) {
        self.initialized = false;
        self.anim = None;
    }

    /// The region positions are relative to changed (LLAgent::setRegion
    /// moves the camera origin with the agent): everything kept in render
    /// space moves by `delta`, so that a border crossing shows nothing.
    pub fn shift(&mut self, delta: Vec3) {
        self.follow.shift(delta);
        for p in [
            &mut self.position,
            &mut self.target,
            &mut self.focus_target,
            &mut self.virtual_position,
            &mut self.smoothing_last_global,
            &mut self.smoothing_last_agent,
            &mut self.head,
            &mut self.avatar.root,
            &mut self.avatar.position,
            &mut self.avatar.follow_root,
        ] {
            *p += delta;
        }
        if let Some(free) = &mut self.free {
            free.position += delta;
        }
        if let Some(flycam) = &mut self.flycam {
            flycam.position += delta;
        }
        if let Some(a) = &mut self.anim {
            a.start_camera += delta;
            a.start_focus += delta;
        }
        if let Some(f) = &mut self.focus_geom {
            f.position += delta;
        }
        // (the new region sends its own plane; meanwhile the old one holds)
        self.collide_plane.w += self.collide_plane.truncate().dot(delta);
    }

    /// Arrival of a teleport (process_agent_movement_complete): the camera
    /// back on the avatar without animation; FSResetCameraOnTP also drops
    /// the orbit, the pitch and the zoom (resetView of process_teleport_local).
    pub fn on_teleport_arrival(&mut self, s: &CameraSettings) {
        self.clear_flycam();
        // a free camera stays in the region it left
        self.free = None;
        self.focus_on_avatar = true;
        self.focus_object = None;
        self.focus_geom = None;
        self.fov_zoom = 0.0;
        if s.reset_on_teleport && self.third_person() {
            self.orbit_yaw = 0.0;
            self.pitch = 0.0;
            self.zoom_fraction = 1.0;
        }
        self.snap();
    }

    /// AvatarSitResponse: the seat's camera and forced mouselook.
    pub fn on_sit_response(&mut self, object: Uuid, eye: Vec3, at: Vec3, force_mouselook: bool) {
        if eye.distance_squared(at) > SIT_CAMERA_THRESHOLD_SQUARED {
            self.sit_camera = Some(SitCamera { object, eye, at });
        }
        self.force_mouselook = force_mouselook;
    }

    /// CameraConstraint: the plane the simulator keeps the camera in front of.
    pub fn set_collide_plane(&mut self, plane: Vec4, s: &CameraSettings) {
        if !s.ignore_sim_constraints && plane.is_finite() {
            self.collide_plane = plane;
        }
    }

    /// AURORA_DEMO_CAM: heading offset, pitch and distance.
    pub fn demo_orbit(&mut self, yaw: f32, pitch: f32, distance: Option<f32>, s: &CameraSettings) {
        self.orbit_yaw = yaw;
        self.pitch = pitch;
        if let Some(d) = distance {
            self.zoom_fraction = d / (s.camera_offset().length() * s.offset_scale).max(0.001);
        }
    }

    // ------------------------------------------------------------------ input

    /// LLAgentCamera::cameraOrbitAround.
    fn orbit_around(&mut self, radians: f32, agent: &mut AgentState) {
        if self.focus_on_avatar && !self.mouselook() {
            agent.yaw += radians;
        } else {
            self.focus_offset_target = Quat::from_rotation_z(radians) * self.focus_offset_target;
            self.zoom_in(1.0);
        }
    }

    /// LLAgentCamera::cameraOrbitOver (positive = the camera goes up).
    fn orbit_over(&mut self, angle: f32) {
        if self.focus_on_avatar && self.third_person() {
            self.pitch_by(angle);
        } else {
            let unit = self.focus_offset_target.normalize_or(Vec3::Z);
            let from_up = unit.dot(Vec3::Z).clamp(-1.0, 1.0).acos();
            let new = (from_up - angle).clamp(1f32.to_radians(), 179f32.to_radians());
            self.focus_offset_target = Quat::from_axis_angle(self.left_axis(), from_up - new) * self.focus_offset_target;
            self.zoom_in(1.0);
        }
    }

    /// LLAgent::pitch: not past straight down, not closer than 5° to looking
    /// straight up.
    fn pitch_by(&mut self, angle: f32) {
        let mut angle = angle;
        if angle >= 0.0 {
            let from_sky = PI / 2.0 + self.pitch;
            let limit = 179f32.to_radians();
            if from_sky + angle > limit {
                angle = limit - from_sky;
            }
        } else {
            let from_sky = angle_between(self.target - self.position, Vec3::Z);
            let limit = 5f32.to_radians();
            if from_sky + angle < limit {
                angle = limit - from_sky;
            }
        }
        if angle.abs() > 1e-4 {
            self.pitch += angle;
        }
    }

    /// LLAgentCamera::getCameraMaxZoomDistance(true).
    fn max_zoom_distance(&self) -> f32 {
        if self.unconstrained {
            return f32::MAX;
        }
        MAX_CAMERA_DISTANCE_FROM_OBJECT
            .min(self.draw_distance - 1.0)
            .min(REGION_WIDTH - CAMERA_FUDGE_FROM_OBJECT)
    }

    /// Distance limits of the alt camera for its focus (min, max).
    fn zoom_limits(&self) -> (f32, f32) {
        let min = match self.focus_geom {
            Some(g) if g.avatar => AVATAR_MIN_ZOOM,
            Some(_) => OBJECT_MIN_ZOOM,
            None => LAND_MIN_ZOOM,
        };
        (min, self.max_zoom_distance().min(MAX_CAMERA_DISTANCE_FROM_OBJECT))
    }

    /// LLAgentCamera::getCameraZoomFraction: 0 zoomed all the way out, 1
    /// all the way in (the build floater's Focus slider).
    pub fn zoom_fraction(&self) -> f32 {
        let rescale = |v: f32, a: f32, b: f32| ((v - a) / (b - a)).clamp(0.0, 1.0);
        if self.focus_on_avatar && self.third_person() {
            return 1.0 - rescale(self.zoom_fraction, MIN_ZOOM_FRACTION, MAX_ZOOM_FRACTION);
        }
        let (min, max) = self.zoom_limits();
        1.0 - rescale(self.focus_offset_target.length(), min, max)
    }

    /// LLAgentCamera::setCameraZoomFraction.
    pub fn set_zoom_fraction(&mut self, fraction: f32) {
        let f = fraction.clamp(0.0, 1.0);
        if self.focus_on_avatar && self.third_person() {
            self.zoom_fraction = MAX_ZOOM_FRACTION + (MIN_ZOOM_FRACTION - MAX_ZOOM_FRACTION) * f;
        } else {
            let (min, max) = self.zoom_limits();
            let unit = self.focus_offset_target.normalize_or(Vec3::X);
            self.focus_offset_target = unit * (max + (min - max) * f);
        }
    }

    /// LLAgentCamera::cameraZoomIn: alt-camera distance × `fraction`, kept
    /// in bounds.
    fn zoom_in(&mut self, fraction: f32) {
        let current = self.focus_offset_target.length();
        let unit = self.focus_offset_target.normalize_or(Vec3::X);
        let mut new = current * fraction;
        if self.unconstrained {
            new = new.min(self.max_zoom_distance());
        } else {
            // don't move through the focus point
            let min = match self.focus_geom {
                Some(g) if g.avatar => {
                    focus::calc_camera_min_distance(&g, self.focus_object_offset, self.position, self.focus_target, self.near).0
                }
                Some(_) => OBJECT_MIN_ZOOM,
                None => LAND_MIN_ZOOM,
            };
            // MAINT-3154: at most 4× the current distance at once
            new = new.max(min).min(self.max_zoom_distance().min(current * 4.0));
        }
        self.focus_offset_target = unit * new;
    }

    /// LLAgentCamera::cameraOrbitIn: `meters` closer; zooming into the head
    /// enters mouselook.
    fn orbit_in(&mut self, meters: f32, agent: &mut AgentState, s: &CameraSettings) {
        if self.focus_on_avatar && self.third_person() {
            let initial = (s.camera_offset().length() * s.offset_scale).max(0.001);
            self.zoom_fraction = (self.target_distance - meters) / initial;
            if self.zoom_fraction < MIN_ZOOM_FRACTION && meters > 0.0 {
                // no need to animate, the camera is already there
                self.change_to_mouselook(agent, s, false);
            }
            if !s.disable_constraints {
                self.zoom_fraction = self.zoom_fraction.clamp(MIN_ZOOM_FRACTION, MAX_ZOOM_FRACTION);
            }
        } else {
            let unit = self.focus_offset_target.normalize_or(Vec3::X);
            let mut new = self.focus_offset_target.length() - meters;
            if !s.disable_constraints {
                new = new.max(match self.focus_geom {
                    Some(g) if g.avatar => AVATAR_MIN_ZOOM,
                    Some(_) => OBJECT_MIN_ZOOM,
                    None => LAND_MIN_ZOOM,
                });
            }
            self.focus_offset_target = unit * new.min(self.max_zoom_distance());
            self.zoom_in(1.0);
        }
    }

    /// Mouse wheel (LLAgentCamera::handleScrollWheel; LLToolCompGun in
    /// mouselook), `clicks` > 0 zooms out. Returns true when the settings
    /// changed: Ctrl / Shift + wheel raise the camera / the focus point.
    pub fn scroll(&mut self, clicks: f32, ctrl: bool, shift: bool, agent: &mut AgentState, s: &mut CameraSettings) -> bool {
        if self.mouselook() {
            if clicks > 0.0 && s.wheel_exits_mouselook {
                self.change_to_default(agent, s);
            }
            return false;
        }
        if self.mode == CameraMode::Follow && self.focus_on_avatar {
            if !s.disable_wheel_zoom && self.follow.zoom(clicks) {
                self.change_to_mouselook(agent, s, false);
            }
            return false;
        }
        if self.anim.is_some() || s.disable_wheel_zoom {
            return false;
        }
        let root_root_two = 2f32.sqrt().sqrt();
        if self.focus_on_avatar && self.third_person() {
            if shift {
                s.focus_offset[2] += 0.1 * clicks;
                s.preset = CameraPreset::Custom;
                return true;
            }
            if ctrl {
                s.camera_offset[2] += 0.1 * clicks;
                s.preset = CameraPreset::Custom;
                return true;
            }
            let initial = (s.camera_offset().length() * s.offset_scale).max(0.001);
            let fraction = self.target_distance / initial * (1.0 - root_root_two.powf(clicks));
            self.orbit_in(fraction * initial, agent, s);
        } else {
            let distance = self.focus_offset_target.length();
            self.orbit_in(distance * (1.0 - root_root_two.powf(clicks)), agent, s);
        }
        false
    }

    fn pan(&mut self, delta: Vec3) {
        self.focus_target += delta;
        self.target = self.focus_target;
        // panning moves the camera with the focus, not smoothed behind it
        self.smoothing_stop = true;
        self.zoom_in(1.0);
        self.update_focus_offset();
        self.smoothing_last_global = self.focus_target + self.focus_offset_target;
    }

    /// Already eased by the edit tool; translate the view in this frame,
    /// preserving the focus offset so tracking cannot undo the edge pan.
    pub fn pan_edit(&mut self, delta: Vec3, agent: &mut AgentState, s: &CameraSettings) -> bool {
        if delta == Vec3::ZERO || !delta.is_finite() || !self.third_person() {
            return false;
        }
        if self.focus_on_avatar {
            self.set_view(self.position, self.target, agent, s);
            self.anim = None;
        }
        self.focus_target += delta;
        self.target += delta;
        self.position += delta;
        self.virtual_position += delta;
        self.smoothing_last_global += delta;
        self.smoothing_last_agent += delta;
        self.smoothing_stop = true;
        if let Some(anim) = self.anim.as_mut() {
            anim.start_camera += delta;
            anim.start_focus += delta;
        }
        self.update_focus_offset();
        true
    }

    /// LLAgentCamera::cameraPanLeft.
    fn pan_left(&mut self, meters: f32) {
        let axis = self.left_axis();
        self.pan(axis * meters);
    }

    /// LLAgentCamera::cameraPanUp.
    fn pan_up(&mut self, meters: f32) {
        let axis = self.up_axis();
        self.pan(axis * meters);
    }

    /// A camera drag starts: Alt+click, or the left button held on our
    /// avatar (`steering`, LLToolCamera::mMouseSteering).
    pub fn begin_drag(&mut self, steering: bool) {
        self.tool = Some(Tool {
            steering,
            ..Default::default()
        });
    }

    pub fn end_drag(&mut self) {
        self.tool = None;
    }

    /// Mouse motion during a camera drag (LLToolCamera::handleHover), in
    /// window pixels (y down). A full window width turns 360°.
    pub fn drag(&mut self, mode: ToolMode, dx: f32, dy: f32, width: f32, agent: &mut AgentState) {
        let Some(t) = &mut self.tool else {
            return;
        };
        t.accum_x += dx.abs();
        t.accum_y += dy.abs();
        t.outside_x |= t.accum_x >= SLOP_RANGE;
        t.outside_y |= t.accum_y >= SLOP_RANGE;
        let (outside_x, outside_y, steering) = (t.outside_x, t.outside_y, t.steering);
        if !outside_x && !outside_y {
            return;
        }
        if steering {
            // the mouse turns the avatar from where the camera looks, a right
            // drag orbit included
            self.fold_orbit(agent);
        }
        let width = width.max(1.0);
        let k = TAU / width;
        match mode {
            ToolMode::Orbit => {
                if dx != 0.0 {
                    self.orbit_around(-dx * k, agent);
                }
                if dy != 0.0 {
                    self.orbit_over(dy * k);
                }
            }
            ToolMode::Pan => {
                let meters_per_pixel = 3.0 * self.position.distance(self.target) / width;
                if dx != 0.0 {
                    self.pan_left(dx * meters_per_pixel);
                }
                if dy != 0.0 {
                    self.pan_up(dy * meters_per_pixel);
                }
            }
            ToolMode::Zoom => {
                if dx != 0.0 {
                    // LLToolCamera::handleHover -> cameraOrbitAround turns
                    // the agent in avatar follow. Keep that steering turn
                    // when ClickOnAvatarKeepsCamera retains a detached focus.
                    if steering && !self.focus_on_avatar {
                        agent.yaw -= dx * k;
                    }
                    self.orbit_around(-dx * k, agent);
                }
                if dy != 0.0 && outside_y {
                    if steering {
                        self.orbit_over(dy * k);
                    } else {
                        self.zoom_in(0.99f32.powf(-dy));
                    }
                }
            }
        }
    }

    /// Right drag (Aurora): around the avatar without turning it, or around
    /// the alt-camera focus point.
    pub fn orbit_drag(&mut self, dx: f32, dy: f32, sensitivity: f32, agent: &mut AgentState) {
        let k = DRAG_RATE * sensitivity;
        if self.focus_on_avatar && self.third_person() {
            self.orbit_yaw -= dx * k;
            self.pitch_by(dy * k);
        } else {
            self.orbit_around(-dx * k, agent);
            self.orbit_over(dy * k);
        }
    }

    /// Mouselook: the mouse turns the avatar and tilts its view.
    pub fn look(&mut self, dx: f32, dy: f32, sensitivity: f32, agent: &mut AgentState) {
        let k = DRAG_RATE * sensitivity;
        agent.yaw -= dx * k;
        agent.pitch = (agent.pitch - dy * k).clamp(-1.5, 1.5);
    }

    /// Camera keys held this frame (LLAgentCamera::updateCamera); like
    /// camera_spin_around_cw & co they first unlock the view from the avatar.
    pub fn apply_keys(&mut self, keys: CameraKeys, agent: &mut AgentState, s: &CameraSettings, dt: f32) {
        if !keys.any() || self.mouselook() {
            return;
        }
        self.unlock_view(agent, s);
        let over = keys.orbit_up - keys.orbit_down;
        if over != 0.0 {
            self.orbit_over(over * ORBIT_RATE * dt);
        }
        let around = keys.orbit_left - keys.orbit_right;
        if around != 0.0 {
            self.orbit_around(around * ORBIT_RATE * dt, agent);
        }
        let inward = keys.zoom_in - keys.zoom_out;
        if inward != 0.0 {
            // the distance to the focus per second
            let distance = self.position.distance(self.focus_target);
            self.orbit_in(inward * distance * dt, agent, s);
        }
        let pan_x = keys.pan_right - keys.pan_left;
        if pan_x != 0.0 {
            self.pan_left(pan_x * -PAN_RATE * dt);
        }
        let pan_y = keys.pan_up - keys.pan_down;
        if pan_y != 0.0 {
            self.pan_up(pan_y * PAN_RATE * dt);
        }
    }

    /// The joystick's continuous pitch, as LLAgent::pitch updates the
    /// third-person camera; mouselook uses AgentState::pitch instead.
    pub fn joystick_pitch(&mut self, rate: f32, dt: f32) {
        if !self.mouselook() && rate != 0.0 {
            self.pitch_by(-rate * dt.clamp(0.0, 0.2));
        }
    }

    // ------------------------------------------------------------------ frame

    /// Once per frame, after the avatar moved (LLAgentCamera::updateCamera).
    pub fn update(&mut self, world: &mut World, s: &CameraSettings, f: &FrameInput) {
        if !world.rlva.enabled() {
            self.update_unrestricted(world, s, f);
            return;
        }
        let settings = world.rlva.camera_settings(s, f.aspect);
        let no_mouselook =
            world.rlva.camera_value("setcam_mouselook").is_some() || world.rlva.camera_limit("setcam_avdistmin", 0.0, true) > 0.0;
        if self.mouselook() && no_mouselook {
            self.change_to_default(&mut world.agent, &settings);
        }
        if world.rlva.camera_value("setcam_unlock").is_some() && !self.focus_on_avatar {
            self.set_focus_on_avatar(true, false, false, &mut world.agent, &settings);
        }
        self.update_unrestricted(world, &settings, f);
        for (origin, low, high) in [
            (world.agent.position, "setcam_avdistmin", "setcam_avdistmax"),
            (self.target, "setcam_origindistmin", "setcam_origindistmax"),
        ] {
            let min = world.rlva.camera_limit(low, 0.0, true);
            let max = world.rlva.camera_limit(high, f32::INFINITY, false).max(min);
            let offset = self.position - origin;
            let distance = offset.length();
            if distance < min || distance > max {
                let direction = offset.try_normalize().unwrap_or(-Vec3::X);
                self.position = origin + direction * distance.clamp(min, max);
            }
        }
        if let Some((min, max)) = world.rlva.camera_fov_limits(f.aspect) {
            self.fov_y = self.fov_y.clamp(min, max);
        }
        if let Some(flycam) = self.flycam.as_mut() {
            // Keep the integrated pose and AgentUpdate frame at the view
            // allowed by RLVa, including when the flight is toggled off.
            flycam.position = self.position;
            flycam.fov = self.fov_y;
            self.target = self.position + flycam.forward();
        }
    }

    fn update_unrestricted(&mut self, world: &mut World, s: &CameraSettings, f: &FrameInput) {
        if self.flycam_lens.is_some_and(|(original, _)| original != s.fov) {
            // A subsequent lens edit takes ownership of the value, including
            // when the user later returns to the pre-flight preference.
            self.flycam_lens = Some((s.fov, s.fov));
            if let Some(flycam) = &mut self.flycam {
                flycam.set_fov(s.fov);
            }
        }
        let dt = f.dt.max(0.0);
        self.dt = dt;
        self.draw_distance = f.draw_distance;
        self.unconstrained = s.disable_constraints;
        self.own = world.agent_id;
        self.avatar = avatar_frame(world, s, f.now);
        self.sync_seat(&mut world.agent, s);
        if let Some(flycam) = self.flycam {
            self.position = flycam.position;
            self.target = flycam.position + flycam.forward();
            self.fov_y = flycam.fov;
            self.roll = flycam.roll();
            self.anim = None;
            return;
        }
        if let Some(free) = self.free {
            // the free camera's own view; the avatar's waits for its return
            self.position = free.position;
            self.target = free.target();
            self.fov_y = self.zoomed_fov(s, f.aspect);
            self.anim = None;
            return;
        }
        self.follow_available = world.scripted_camera.active().is_some();
        if self.third_person() && self.focus_on_avatar && self.follow_available {
            self.change_to_follow(&world.agent, s);
        }
        if self.mode == CameraMode::Follow {
            if let Some(params) = world.scripted_camera.active() {
                if !self.initialized {
                    // Login / teleport starts at the avatar's new location;
                    // never smooth a follow simulation from the old region.
                    let eye = self
                        .sit_camera_point(world, f.now, true)
                        .unwrap_or_else(|| self.third_person_camera(&world.agent, s, f));
                    let at = self
                        .sit_camera_point(world, f.now, false)
                        .unwrap_or(self.avatar.position + self.frame_rotation(&world.agent) * s.focus_offset());
                    self.follow
                        .reset(eye, at, self.avatar.follow_root, self.follow_rotation(&world.agent));
                }
                self.follow
                    .update(params, self.avatar.follow_root, self.follow_rotation(&world.agent), dt);
            } else {
                self.change_to_third_person(&mut world.agent, s, true);
            }
        }
        let world = &*world;
        let agent = &world.agent;
        self.head = self.avatar.position + Vec3::Z * HEAD_HEIGHT;
        self.refresh_focus_object(world, s, f.now, s.track_focus_object && f.edit_tracking != Some(false));
        let offset_changed = self.focus_offset != self.focus_offset_target;
        // CAMERA_FOCUS_HALF_LIFE is 0: the offset follows at once
        self.focus_offset = self.focus_offset_target;
        let in_air = agent.flying || agent.velocity.z.abs() > 1.0;
        self.time_in_air = if in_air { self.time_in_air + dt } else { 0.0 };

        let mut camera_target = self.calc_camera_target(world, s, f);
        self.virtual_position = camera_target;
        let focus_target = self.calc_focus_target(world, s, f.now, camera_target);
        // field of view correction: the camera backs off, the view narrows
        // An edited object's growing extents must not move a frozen camera.
        if f.edit_tracking != Some(false) || offset_changed {
            self.fov_zoom = self.calc_fov_zoom();
        }
        camera_target = focus_target + (camera_target - focus_target) * (1.0 + self.fov_zoom);

        if !self.initialized {
            self.initialized = true;
            self.anim = None;
            self.last_mode = self.mode;
            self.lag = Vec3::ZERO;
            self.current_distance = self.target_distance;
            self.current_fov_zoom = self.fov_zoom;
            self.smoothing_last_global = camera_target;
            self.smoothing_last_agent = camera_target - self.avatar.position;
            self.position = camera_target;
            self.target = focus_target;
            self.fov_y = self.zoomed_fov(s, f.aspect);
            return;
        }

        // transition between modes / focus points
        let (mut camera, focus) = match self.anim {
            Some(mut a) => {
                a.elapsed += dt;
                let fraction = if a.duration > 0.0 { a.elapsed / a.duration } else { 1.0 };
                // into mouselook: stop short of the head
                let skip = if a.start_camera == camera_target {
                    0.0
                } else {
                    HEAD_BUFFER_SIZE / a.start_camera.distance(camera_target)
                };
                let finish = if self.mouselook() { 1.0 - skip } else { 1.0 };
                if fraction < finish {
                    self.anim = Some(a);
                    let k = smoothstep(fraction);
                    (
                        a.camera(camera_target, focus_target, k),
                        a.start_focus.lerp_monotonic(focus_target, k),
                    )
                } else {
                    self.anim = None;
                    self.last_mode = self.mode;
                    (camera_target, focus_target)
                }
            }
            None => {
                self.last_mode = self.mode;
                (camera_target, focus_target)
            }
        };

        // smoothing: relative to the avatar when following it (the avatar
        // moves too jerkily in world space to smooth there). `lerp_monotonic`
        // and not `lerp`: glam's `lerp` (a * (1 - k) + b * k) does not give
        // back `a` when both ends are equal, and which neighbor it rounds to
        // depends on `k`, so on the frame time: a camera at rest then moved
        // by one step of its coordinates in about one frame out of four.
        let agent_pos = self.avatar.position;
        let mut relative = camera - agent_pos;
        self.smoothing_stop |= f.build_mode;
        if self.third_person() && !self.smoothing_stop {
            let k = interpolant(s.smoothing * SMOOTHING_HALF_LIFE, dt);
            if self.focus_on_avatar && self.focus_object.is_none() {
                if (relative - self.smoothing_last_agent).length() < MAX_CAMERA_SMOOTH_DISTANCE {
                    relative = self.smoothing_last_agent.lerp_monotonic(relative, k);
                    camera = relative + agent_pos;
                }
            } else if (camera - self.smoothing_last_global).length() < MAX_CAMERA_SMOOTH_DISTANCE {
                camera = self.smoothing_last_global.lerp_monotonic(camera, k);
            }
        }
        self.smoothing_last_global = camera;
        self.smoothing_last_agent = relative;
        self.smoothing_stop = false;

        self.current_fov_zoom += (self.fov_zoom - self.current_fov_zoom) * interpolant(FOV_ZOOM_HALF_LIFE, dt);
        self.position = camera;
        self.target = focus;
        self.fov_y = self.zoomed_fov(s, f.aspect);
    }

    /// While seated the agent frame turns with the seat (setupSitCamera,
    /// LLVOAvatar::sitOnObject / getOffObject); `agent.yaw` stays a world
    /// heading for the rest of the viewer.
    fn sync_seat(&mut self, agent: &mut AgentState, s: &CameraSettings) {
        let flat = |v: Vec3| (v.truncate().length_squared() > 1e-6).then(|| v.y.atan2(v.x));
        match (self.avatar.seat, self.seat) {
            (Some(rot), None) => {
                let at = rot.inverse() * Vec3::new(agent.yaw.cos(), agent.yaw.sin(), 0.0);
                self.seat = Some(Seat {
                    local_yaw: flat(at).unwrap_or(0.0),
                    written_yaw: agent.yaw,
                });
                self.start_animation(s);
                if self.force_mouselook {
                    self.change_to_mouselook(agent, s, true);
                }
            }
            (Some(rot), Some(mut seat)) => {
                // turning keys and steering change the heading inside the seat
                seat.local_yaw += agent.yaw - seat.written_yaw;
                if let Some(yaw) = flat(rot * Vec3::new(seat.local_yaw.cos(), seat.local_yaw.sin(), 0.0)) {
                    agent.yaw = yaw;
                }
                seat.written_yaw = agent.yaw;
                self.seat = Some(seat);
            }
            (None, Some(_)) => {
                self.seat = None;
                self.sit_camera = None;
                self.pitch = 0.0;
                self.fov_zoom = 0.0;
                agent.pitch = 0.0;
            }
            (None, None) => {}
        }
    }

    /// LLAgentCamera::validateFocusObject and the object tracking of
    /// calcFocusPositionTargetGlobal.
    fn refresh_focus_object(&mut self, world: &World, s: &CameraSettings, now: Instant, track: bool) {
        let Some(id) = self.focus_object else {
            self.focus_geom = None;
            return;
        };
        match focus_object_geom(world, &id, now) {
            None => {
                // the object is gone: keep the point, animate from there
                self.start_animation(s);
                self.focus_object = None;
                self.focus_geom = None;
                self.focus_object_offset = Vec3::ZERO;
                self.fov_zoom = 0.0;
            }
            Some(g) => {
                if self.focus_geom.is_none() {
                    self.focus_object_offset = self.focus_target - g.position;
                }
                self.focus_geom = Some(g);
                if !self.focus_on_avatar {
                    if track {
                        self.focus_target = g.position + self.focus_object_offset;
                    } else {
                        self.focus_object_offset = self.focus_target - g.position;
                    }
                }
            }
        }
    }

    /// The seat's camera (llSetCameraEyeOffset / AtOffset), in render space.
    fn sit_camera_point(&self, world: &World, now: Instant, eye: bool) -> Option<Vec3> {
        let sit = self.sit_camera?;
        self.avatar.seat?;
        let idx = world.objects.index_of_uuid(&sit.object)?;
        let (p, r, _) = Scene::object_transform(world, idx, now, 0)?;
        Some(p + r * if eye { sit.eye } else { sit.at })
    }

    fn mouselook_eye(&self, agent: &AgentState, s: &CameraSettings, eye_offset: Option<Vec3>) -> Vec3 {
        let posed = eye_offset.filter(|p| p.is_finite());
        // Requested opt-in: follow the eye midpoint's translation, never the
        // head's rotation. calc_focus_target keeps the mouse's world-space aim.
        if s.mouselook_follow_eyes
            && let Some(eyes) = posed
        {
            return self.avatar.render_root + self.avatar.render_rotation * eyes;
        }
        if let Some(rotation) = self.avatar.rotation {
            // LLVOAvatar::updateHeadOffset / LLAgentCamera::calcCameraPositionTargetGlobal:
            // seated eyes follow the full animated pose through the avatar and
            // seat rotations, including the legacy 0.1 m local eye adjustment.
            let eyes = posed.unwrap_or(Vec3::new(0.12, 0.0, HEAD_HEIGHT));
            self.avatar.root + rotation * (eyes + Vec3::Z * 0.1)
        } else if agent.ground_sit {
            // Legacy Viewer's parentless sitting branch uses the posed eye
            // height and the same 0.1 m adjustment, without head X/Y motion.
            let height = posed.map_or(HEAD_HEIGHT, |eyes| eyes.z) + 0.1;
            self.avatar.root + self.avatar.render_rotation * Vec3::Z * height
        } else {
            self.avatar.position + Vec3::Z * HEAD_HEIGHT + agent.forward() * 0.12
        }
    }

    /// LLAgentCamera::calcCameraPositionTargetGlobal.
    fn calc_camera_target(&mut self, world: &World, s: &CameraSettings, f: &FrameInput) -> Vec3 {
        let agent = &world.agent;
        let mut camera = match self.mode {
            CameraMode::Mouselook => self.mouselook_eye(agent, s, f.eye_offset),
            CameraMode::Follow if self.focus_on_avatar => self.follow.eye,
            CameraMode::ThirdPerson if self.focus_on_avatar => match self.sit_camera_point(world, f.now, true) {
                Some(p) => p,
                None => self.third_person_camera(agent, s, f),
            },
            CameraMode::ThirdPerson | CameraMode::Follow => self.focus_target + self.focus_offset,
        };
        if !s.disable_constraints {
            let offset = camera - self.avatar.position;
            let distance = offset.length();
            if distance > f.draw_distance {
                camera = self.avatar.position + offset * (f.draw_distance / distance);
            }
        }
        // not underground (getCameraMinOffGround)
        let min_off_ground = if self.mouselook() {
            0.0
        } else if s.disable_constraints {
            -1000.0
        } else {
            0.5
        };
        if let Some(land) = world.ground_height(camera) {
            camera.z = camera.z.max(land + min_off_ground);
        }
        camera
    }

    /// Behind the avatar: the camera offset, kept in front of the plane the
    /// simulator found between the head and the camera, lagging behind in
    /// the air.
    fn third_person_camera(&mut self, agent: &AgentState, s: &CameraSettings, f: &FrameInput) -> Vec3 {
        let dt = self.dt;
        let frame = self.frame_rotation(agent);
        // mThirdPersonHeadOffset: none when seated
        let head_offset = if self.avatar.seat.is_some() { Vec3::ZERO } else { Vec3::Z };
        let base = self.avatar.root + head_offset;
        let offset = frame * (s.camera_offset() * self.zoom_fraction * s.offset_scale);
        let dir = offset.normalize_or_zero();
        let mut distance = offset.length();
        if !s.disable_constraints && self.collide_plane != Vec4::ZERO && self.avatar.seat.is_none() {
            let normal = self.collide_plane.truncate();
            let w = self.collide_plane.w;
            let mut offset_dot = offset.dot(normal);
            if offset_dot.abs() < 0.001 {
                offset_dot = 0.001;
            }
            let pos_dot = base.dot(normal);
            if pos_dot > w {
                if offset_dot + pos_dot < w {
                    distance *= (pos_dot - w - CAMERA_COLLIDE_EPSILON) / -offset_dot;
                }
            } else if offset_dot + pos_dot > w {
                distance *= (w - pos_dot - CAMERA_COLLIDE_EPSILON) / offset_dot;
            }
        }
        self.target_distance = distance.max(MIN_CAMERA_DISTANCE);
        if self.target_distance != self.current_distance {
            self.current_distance += (self.target_distance - self.current_distance) * interpolant(CAMERA_ZOOM_HALF_LIFE, dt);
        }
        let camera = base + dir * self.current_distance;
        // lag behind the avatar while in the air (DynamicCameraStrength)
        if self.anim.is_none() && self.time_in_air > GROUND_TO_AIR_CAMERA_TRANSITION_START_TIME {
            let mut at = frame * Vec3::X;
            at.z = 0.0;
            let at = at.normalize_or(Vec3::X);
            // ease in from the ground to avoid a camera pop
            let u =
                ((self.time_in_air - GROUND_TO_AIR_CAMERA_TRANSITION_START_TIME) / GROUND_TO_AIR_CAMERA_TRANSITION_TIME).clamp(0.0, 1.0);
            let target_lag = if f.steering {
                Vec3::ZERO
            } else {
                agent.velocity * s.dynamic_strength / 30.0
            };
            self.lag = self.lag.lerp_monotonic(target_lag, interpolant(CAMERA_LAG_HALF_LIFE, dt) * u);
            let lag = self.lag.length();
            if lag > MAX_CAMERA_LAG {
                self.lag *= MAX_CAMERA_LAG / lag;
            }
            // the avatar stays in front of the camera
            let min = MIN_CAMERA_LAG * u;
            let dot = (self.lag - at * min).dot(at);
            if dot < -min {
                self.lag -= (dot + min) * at;
            }
        } else {
            self.lag = self.lag.lerp_monotonic(Vec3::ZERO, interpolant(0.15, dt));
        }
        camera - self.lag
    }

    /// LLAgentCamera::calcFocusPositionTargetGlobal.
    fn calc_focus_target(&self, world: &World, s: &CameraSettings, now: Instant, camera_target: Vec3) -> Vec3 {
        let agent = &world.agent;
        match self.mode {
            CameraMode::Follow if self.focus_on_avatar => self.follow.at,
            CameraMode::Mouselook => {
                let (y, p) = (agent.yaw, agent.pitch);
                // Keep the mouse's aim even if terrain or range constraints
                // move the eye away from its requested position.
                camera_target + Vec3::new(y.cos() * p.cos(), y.sin() * p.cos(), p.sin())
            }
            CameraMode::ThirdPerson | CameraMode::Follow if !self.focus_on_avatar => self.focus_target,
            CameraMode::ThirdPerson | CameraMode::Follow => match self.sit_camera_point(world, now, false) {
                Some(p) => p,
                None => self.avatar.position + self.frame_rotation(agent) * s.focus_offset(),
            },
        }
    }

    /// LLAgentCamera::calcCameraFOVZoomFactor: closer to an object than its
    /// extents allow, the view narrows instead.
    fn calc_fov_zoom(&self) -> f32 {
        if self.mouselook() {
            return 0.0;
        }
        match self.focus_geom {
            Some(g) if !g.avatar && !self.focus_on_avatar => {
                let min = if self.unconstrained {
                    0.0
                } else {
                    focus::calc_camera_min_distance(&g, self.focus_object_offset, self.position, self.focus_target, self.near).0
                };
                let current = self.focus_offset.length().max(0.001);
                ((min - current) / current).clamp(0.0, 1000.0)
            }
            // focused on land or an avatar: keep it until the focus changes
            _ => self.fov_zoom,
        }
    }

    /// One log line (demo scenarios, diagnostics).
    pub fn describe(&self) -> String {
        format!(
            "{:?}, pos {:.2?}, target {:.2?}, fov {:.1}°, on avatar {}, orbit {:.2}, pitch {:.2}, zoom {:.2}, animating {}",
            self.mode,
            self.position,
            self.target,
            self.fov_y.to_degrees(),
            self.focus_on_avatar,
            self.orbit_yaw,
            self.pitch,
            self.zoom_fraction,
            self.anim.is_some()
        )
    }

    pub fn view(&self) -> Mat4 {
        if let Some(flycam) = self.flycam {
            return glam::camera::rh::view::look_to_mat4(self.position + self.shake, flycam.forward(), flycam.up());
        }
        // Scripts may set identical eye / focus points or look vertically.
        let direction = self.forward();
        let up = self.view_up();
        glam::camera::rh::view::look_to_mat4(self.position + self.shake, direction, up)
    }

    pub fn view_up(&self) -> Vec3 {
        if let Some(flycam) = self.flycam {
            return flycam.up();
        }
        let direction = self.forward();
        let up = self
            .seated_mouselook_up()
            .filter(|up| direction.cross(*up).length_squared() > 1e-6)
            .unwrap_or_else(|| view_reference_up(direction));
        glam::Quat::from_axis_angle(direction, self.roll) * up
    }

    /// LLAgentCamera::updateCamera (indra/newview/llagentcamera.cpp,
    /// originally LGPL 2.1): seated in mouselook, the camera's up is the
    /// avatar's +Z, so the view banks and pitches with the vehicle; the
    /// mode of a running transition is the one it leaves (mLastCameraMode).
    fn seated_mouselook_up(&self) -> Option<Vec3> {
        let mode = if self.anim.is_some() { self.last_mode } else { self.mode };
        if mode != CameraMode::Mouselook {
            return None;
        }
        self.avatar.rotation.map(|rotation| rotation * Vec3::Z)
    }

    /// LLAgent::getHeadRotation (indra/newview/llagent.cpp, LGPL 2.1): a
    /// complete orientation, not a pitch offset. Outside mouselook use the
    /// avatar object's facing, independent of camera orbit and body lag.
    /// Seated orientations are local to the seat in AgentUpdate.
    pub fn head_rotation(&self, agent: &AgentState) -> Quat {
        let rotation = if self.mouselook() {
            let at = self.forward();
            let left = self.view_up().cross(at).normalize_or(Vec3::Y);
            let up = at.cross(left).normalize_or(Vec3::Z);
            Quat::from_mat3(&glam::Mat3::from_cols(at, left, up))
        } else {
            self.avatar.rotation.unwrap_or_else(|| agent.body_rotation())
        };
        self.avatar.seat.map_or(rotation, |seat| (seat.inverse() * rotation).normalize())
    }

    /// The view panned by `right` and `up` metres across the screen.
    pub fn pan_offset(&self, right: f32, up: f32) -> Vec3 {
        let forward = self.forward();
        let side = forward.cross(Vec3::Z).normalize_or(Vec3::Y);
        let above = side.cross(forward).normalize_or(Vec3::Z);
        side * right + above * up
    }

    pub fn proj(&self, aspect: f32) -> Mat4 {
        glam::camera::rh::proj::directx::perspective_infinite_reverse(self.fov_y, aspect.max(0.1), self.near)
    }

    /// The projection of a photo of `aspect` taken from a window of
    /// `window_aspect`: exactly the part of the window's view the photo's
    /// proportions frame (what the full-frame view leaves clear). Narrower
    /// than the window it keeps the vertical field; wider, the horizontal one.
    pub fn photo_proj(&self, aspect: f32, window_aspect: f32) -> Mat4 {
        let (aspect, window_aspect) = (aspect.max(0.1), window_aspect.max(0.1));
        let fov = if aspect > window_aspect {
            2.0 * ((self.fov_y * 0.5).tan() * window_aspect / aspect).atan()
        } else {
            self.fov_y
        };
        glam::camera::rh::proj::directx::perspective_infinite_reverse(fov, aspect, self.near)
    }
}

/// Height of the eyes above the avatar's position (mouselook, unlockView).
const HEAD_HEIGHT: f32 = 0.7;

/// Our avatar's root, position and seat this frame.
fn avatar_frame(world: &World, s: &CameraSettings, now: Instant) -> AvatarFrame {
    let agent = &world.agent;
    let fallback = AvatarFrame {
        root: agent.position,
        position: agent.position,
        follow_root: agent.position,
        rotation: None,
        render_root: agent.position,
        render_rotation: Quat::from_rotation_z(agent.yaw),
        seat: None,
    };
    let id = world.agent_id;
    let Some(idx) = world.objects.index_of_uuid(&id) else {
        return fallback;
    };
    let Some((position, rotation, _)) = Scene::object_transform_unshifted(world, idx, now) else {
        return fallback;
    };
    let seat = world
        .objects
        .get(idx)
        .filter(|o| o.parent_id != 0)
        .and_then(|o| world.objects.parent_of(o))
        .and_then(|parent| Scene::object_transform(world, parent, now, 0))
        .map(|(_, r, _)| r);
    // LLAgentCamera::getAvatarRootPosition, computed without ever adding
    // hover when disabled. Camera updates run before the pose cache, so a
    // cached rendered root minus the current hover would jump for one frame.
    let shape = world.avatar_shape_dz.get(&id).copied().unwrap_or(0.0);
    let hover = if s.hover_affects_camera { world.appearance_hover(&id) } else { 0.0 };
    let axis = if seat.is_some() { rotation * Vec3::Z } else { Vec3::Z };
    let position = position + Vec3::Z * world.water_visual_dz(&id);
    let root = position + axis * (shape + hover);
    AvatarFrame {
        root,
        position,
        follow_root: root,
        rotation: seat.map(|_| rotation),
        render_root: position + axis * (shape + world.appearance_hover(&id)),
        render_rotation: rotation,
        seat,
    }
}

/// Where object `id` is and how it is turned (render space), unless it
/// is a HUD.
pub fn focus_object_geom(world: &World, id: &Uuid, now: Instant) -> Option<FocusObject> {
    let idx = world.objects.index_of_uuid(id)?;
    let o = world.objects.get(idx)?;
    let (position, rotation, hud) = Scene::object_transform_unshifted(world, idx, now)?;
    if hud {
        return None;
    }
    Some(FocusObject {
        position,
        rotation,
        scale: o.scale,
        avatar: o.is_avatar(),
        mesh: o.volume.is_mesh(),
    })
}

/// The alt-camera focus object of a clicked object: an attachment focuses
/// its avatar (LLAgentCamera::setFocusGlobal); HUDs are not focusable.
pub fn pick_focus_object(world: &World, idx: usize, now: Instant) -> Option<(Uuid, FocusObject)> {
    let mut focus = idx;
    let mut root = idx;
    for _ in 0..16 {
        let o = world.objects.get(root)?;
        if o.parent_id == 0 {
            break;
        }
        let parent = world.objects.parent_of(o)?;
        if world.objects.get(parent)?.is_avatar() {
            if o.state != 0 && !o.is_avatar() {
                focus = parent;
            }
            break;
        }
        root = parent;
    }
    let id = world.objects.get(focus)?.full_id;
    Some((id, focus_object_geom(world, &id, now)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_photo_frames_exactly_the_clear_part_of_the_window() {
        let cam = Camera::default();
        let window = 16.0 / 9.0;
        // (x, y) of a clip-space corner: the half-extent of the view
        let half = |m: Mat4| (1.0 / m.x_axis.x, 1.0 / m.y_axis.y);
        let (wx, wy) = half(cam.proj(window));
        // narrower: the same height, the middle of the width
        let (px, py) = half(cam.photo_proj(4.0 / 5.0, window));
        assert!((py - wy).abs() < 1e-5 && px < wx);
        // wider: the same width, the middle of the height
        let (px, py) = half(cam.photo_proj(3.0, window));
        assert!((px - wx).abs() < 1e-4 && py < wy);
        // the window's own proportions: its projection
        assert!(cam.photo_proj(window, window).abs_diff_eq(cam.proj(window), 1e-6));
    }
    use crate::scene::avatar::AvatarLibrary;

    fn hover_world(seated: bool) -> World {
        let mut world = world_at(Vec3::new(128.0, 128.0, 25.0));
        world.agent_id = crate::demo::DEMO_AGENT;
        for event in crate::demo::sit_events(700) {
            if let aurora_net::NetEvent::ObjectUpdates { handle, .. } = &event {
                world.main_region = Some(*handle);
            }
            world.apply(event);
        }
        world.apply(aurora_net::NetEvent::Capabilities {
            handle: world.main_region.unwrap(),
            caps: std::sync::Arc::new(Default::default()),
        });
        let region = world.regions.get_mut(&world.main_region.unwrap()).unwrap();
        region.avatar_hover_height = true;
        std::sync::Arc::make_mut(&mut region.caps).insert("AgentPreferences".into(), String::new());
        if seated {
            for frame in [250, 300] {
                for event in crate::demo::sit_events(frame) {
                    world.apply(event);
                }
            }
            let idx = world.objects.index_of_uuid(&world.agent_id).unwrap();
            let parent = world.objects.parent_of(world.objects.get(idx).unwrap()).unwrap();
            world.objects.get_mut(parent).unwrap().rotation = Quat::from_rotation_y(0.7) * Quat::from_rotation_z(1.2);
        }
        world
    }

    #[test]
    fn surface_float_camera_tracks_the_visual_body_even_with_hover_disabled() {
        let mut world = hover_world(false);
        world.movement_complete = true;
        let now = Instant::now();
        let settings = CameraSettings::default();
        let before = avatar_frame(&world, &settings, now);
        for i in 0..60 {
            world.water_float.update(1, world.agent.position, i as f32 / 60.0, 0.2, false);
        }
        let after = avatar_frame(&world, &settings, now);
        let shift = Vec3::Z * world.water_float.offset();
        assert!(after.root.abs_diff_eq(before.root + shift, 1e-5));
        assert!(after.position.abs_diff_eq(before.position + shift, 1e-5));
        assert!(after.follow_root.abs_diff_eq(before.follow_root + shift, 1e-5));
    }

    #[test]
    fn hover_changes_leave_the_disabled_camera_still_before_pose_updates() {
        for seated in [false, true] {
            for mode in ["third", "follow", "mouselook", "alt-self"] {
                let settings = CameraSettings::default();
                let setup = || {
                    let mut world = hover_world(seated);
                    let id = world.agent_id;
                    world.avatar_shape_dz.insert(id, 0.17);
                    let mut camera = Camera::default();
                    settle(&mut camera, &mut world, &settings);
                    match mode {
                        "follow" => {
                            world.apply(aurora_net::NetEvent::SetFollowCam {
                                handle: world.main_region.unwrap(),
                                object: Uuid::from_u128(123),
                                properties: vec![(12, 1.0), (7, 6.0)],
                            });
                        }
                        "mouselook" => camera.toggle_mouselook(&mut world.agent, &settings),
                        "alt-self" => {
                            let geom = focus_object_geom(&world, &id, Instant::now()).unwrap();
                            camera.alt_focus(
                                geom.position,
                                Some((id, geom)),
                                (camera.position, camera.forward()),
                                &mut world.agent,
                                &settings,
                            );
                        }
                        _ => {}
                    }
                    settle(&mut camera, &mut world, &settings);
                    (world, camera)
                };
                let (mut world, mut camera) = setup();
                let (mut control_world, mut control_camera) = setup();
                let dt = [1.0 / 144.0, 1.0 / 60.0, 1.0 / 30.0];
                // App updates the camera before poses. Keep the shape cache
                // untouched while dragging; compare against an identical
                // camera without hover edits at the same variable frame times.
                for i in 0..180 {
                    let height = if i < 120 { (i as f32 * 0.2).sin() * 3.0 } else { 0.0 };
                    world.hover_height.preview(height);
                    let input = frame(dt[i % dt.len()]);
                    camera.update(&mut world, &settings, &input);
                    control_camera.update(&mut control_world, &settings, &input);
                    assert_eq!(
                        (camera.position, camera.target, camera.view(), camera.fov_y),
                        (
                            control_camera.position,
                            control_camera.target,
                            control_camera.view(),
                            control_camera.fov_y
                        ),
                        "seated {seated}, {mode}, frame {i}, hover {height}"
                    );
                }
            }
        }
    }

    #[test]
    fn hover_camera_toggle_changes_the_camera_root_without_changing_agent_position() {
        for seated in [false, true] {
            let mut world = hover_world(seated);
            let now = Instant::now();
            let mut settings = CameraSettings::default();
            let baseline = avatar_frame(&world, &settings, now);
            let agent = world.agent.position;
            let axis = baseline.rotation.map_or(Vec3::Z, |r| r * Vec3::Z);
            for height in [-3.0, -0.25, 0.4, 3.0, 0.0] {
                world.hover_height.preview(height);
                let fixed = avatar_frame(&world, &settings, now);
                assert_eq!(fixed.root, baseline.root);
                settings.hover_affects_camera = true;
                let following = avatar_frame(&world, &settings, now);
                assert!(following.root.abs_diff_eq(baseline.root + axis * height, 1e-5));
                assert_eq!(following.follow_root, following.root);
                assert_eq!(following.position, baseline.position);
                assert_eq!(world.agent.position, agent);
                settings.hover_affects_camera = false;
            }
        }
    }

    fn world_at(pos: Vec3) -> World {
        let mut w = World::new(std::sync::Arc::new(AvatarLibrary::load()));
        w.agent.position = pos;
        w
    }

    fn frame(dt: f32) -> FrameInput {
        FrameInput {
            dt,
            now: Instant::now(),
            steering: false,
            build_mode: false,
            edit_tracking: None,
            eye_offset: None,
            draw_distance: 128.0,
            aspect: 1.0,
        }
    }

    fn settle(cam: &mut Camera, w: &mut World, s: &CameraSettings) {
        for _ in 0..300 {
            cam.update(w, s, &frame(1.0 / 60.0));
        }
    }

    #[test]
    fn appearance_half_turns_animate_at_constant_distance_in_both_directions() {
        for fps in [30, 60, 144] {
            let mut world = world_at(Vec3::new(128.0, 128.0, 25.0));
            let settings = CameraSettings {
                smoothing: 0.0,
                ..Default::default()
            };
            let mut camera = Camera::default();
            settle(&mut camera, &mut world, &settings);
            let focus = world.agent.position + Vec3::Z;
            camera.set_appearance_view(focus + Vec3::X * 3.0, focus, &mut world.agent, &settings);
            settle(&mut camera, &mut world, &settings);
            for side in [-1.0, 1.0] {
                let start = camera.position;
                let end = focus + Vec3::X * (side * 3.0);
                camera.set_appearance_view(end, focus, &mut world.agent, &settings);
                assert!(camera.anim.is_some_and(|a| a.duration > 1.3));
                camera.update(&mut world, &settings, &frame(0.0));
                assert_eq!(
                    camera.position, start,
                    "start at the displayed eye even when the focus did not change"
                );
                let mut previous = start;
                for _ in 0..fps * 2 {
                    camera.update(&mut world, &settings, &frame(1.0 / fps as f32));
                    assert!((camera.position.distance(focus) - 3.0).abs() < 1e-3, "never cut through the avatar");
                    assert!(camera.position.distance(previous) < 12.0 / fps as f32, "no jump between frames");
                    previous = camera.position;
                }
                assert!(camera.position.abs_diff_eq(end, 1e-4));
                assert!(camera.anim.is_none());
            }
        }
    }

    #[test]
    fn appearance_turn_can_reverse_midflight_and_cross_the_yaw_seam() {
        let mut world = world_at(Vec3::new(128.0, 128.0, 25.0));
        let settings = CameraSettings {
            smoothing: 0.0,
            ..Default::default()
        };
        let mut camera = Camera::default();
        settle(&mut camera, &mut world, &settings);
        let focus = world.agent.position + Vec3::Z;
        let front = focus + Vec3::X * 3.0;
        camera.set_appearance_view(front, focus, &mut world.agent, &settings);
        settle(&mut camera, &mut world, &settings);
        camera.set_appearance_view(focus - Vec3::X * 3.0, focus, &mut world.agent, &settings);
        for _ in 0..30 {
            camera.update(&mut world, &settings, &frame(1.0 / 60.0));
        }
        let interrupted = camera.position;
        camera.set_appearance_view(front, focus, &mut world.agent, &settings);
        camera.update(&mut world, &settings, &frame(0.0));
        assert_eq!(camera.position, interrupted);
        settle(&mut camera, &mut world, &settings);
        assert!(camera.position.abs_diff_eq(front, 1e-4));
        for yaw in [175.0_f32, -175.0] {
            let eye = focus + Quat::from_rotation_z(yaw.to_radians()) * Vec3::X * 3.0;
            camera.set_appearance_view(eye, focus, &mut world.agent, &settings);
            for _ in 0..120 {
                camera.update(&mut world, &settings, &frame(1.0 / 60.0));
                if yaw < 0.0 {
                    assert!(camera.position.x < focus.x - 2.9, "take the short arc across ±180 degrees");
                }
            }
            assert!(camera.position.abs_diff_eq(eye, 1e-4));
        }
    }

    fn script(w: &mut World, object: Uuid, properties: Vec<(i32, f32)>) {
        w.main_region = Some(1);
        w.apply(aurora_net::NetEvent::SetFollowCam {
            handle: 1,
            object,
            properties,
        });
    }

    #[test]
    fn scripted_camera_clear_returns_to_normal_and_mouselook_takes_precedence() {
        let s = CameraSettings::default();
        let mut w = world_at(Vec3::new(128.0, 128.0, 25.0));
        let mut cam = Camera::default();
        settle(&mut cam, &mut w, &s);
        let normal = (cam.position, cam.target);
        let object = Uuid::from_u128(123);
        script(
            &mut w,
            object,
            vec![
                (12, 1.0),
                (14, 125.0),
                (15, 124.0),
                (16, 30.0),
                (18, 129.0),
                (19, 128.0),
                (20, 26.0),
                (21, 1.0),
                (22, 1.0),
            ],
        );
        settle(&mut cam, &mut w, &s);
        assert_eq!(cam.mode, CameraMode::Follow);
        assert!(cam.position.distance(Vec3::new(125.0, 124.0, 30.0)) < 0.001);
        assert!(cam.target.distance(Vec3::new(129.0, 128.0, 26.0)) < 0.001);
        cam.toggle_mouselook(&mut w.agent, &s);
        settle(&mut cam, &mut w, &s);
        assert!(cam.mouselook());
        cam.toggle_mouselook(&mut w.agent, &s);
        settle(&mut cam, &mut w, &s);
        assert_eq!(cam.mode, CameraMode::Follow);
        w.apply(aurora_net::NetEvent::ClearFollowCam { object });
        settle(&mut cam, &mut w, &s);
        assert!(cam.third_person());
        assert!(cam.position.distance(normal.0) < 0.01);
        assert!(cam.target.distance(normal.1) < 0.01);
    }

    #[test]
    fn standing_from_scripted_sit_releases_the_locked_view_without_clear_messages() {
        let s = CameraSettings::default();
        let mut w = world_at(Vec3::new(132.0, 126.0, 32.9));
        w.agent_id = crate::demo::DEMO_AGENT;
        let mut cam = Camera::default();
        for at in [250, 300, 320, 500, 640] {
            for event in crate::demo::sit_events(at) {
                if let aurora_net::NetEvent::ObjectUpdates { handle, .. } = &event {
                    w.main_region = Some(*handle);
                }
                if let Some(aurora_net::NetEvent::SitResponse {
                    object,
                    camera_eye,
                    camera_at,
                    force_mouselook,
                }) = w.apply(event)
                {
                    cam.on_sit_response(object, camera_eye, camera_at, force_mouselook);
                }
            }
            if matches!(at, 320 | 500) {
                for event in demo::sit_follow_events(at, w.main_region.expect("seat region"), w.agent.position) {
                    w.apply(event);
                }
            }
            settle(&mut cam, &mut w, &s);
        }
        assert_eq!(cam.mode, CameraMode::Follow);
        assert!(cam.follow.position_locked);
        let locked = cam.position;
        for event in crate::demo::sit_events(700) {
            assert!(!matches!(event, aurora_net::NetEvent::ClearFollowCam { .. }));
            w.apply(event);
        }
        assert!(!w.scripted_camera.has_active());
        settle(&mut cam, &mut w, &s);
        assert!(cam.third_person());
        assert!(cam.sit_camera.is_none());
        assert!(cam.position.distance(locked) > 1.0);
        assert!(cam.view().is_finite());
    }

    #[test]
    fn the_free_camera_leaves_the_avatar_and_glides_back() {
        let s = CameraSettings::default();
        let mut w = world_at(Vec3::new(128.0, 128.0, 25.0));
        let mut cam = Camera::default();
        settle(&mut cam, &mut w, &s);
        let behind = (cam.position, cam.target);
        cam.start_free(&mut w.agent, &s);
        // it starts where the view was
        cam.update(&mut w, &s, &frame(1.0 / 60.0));
        assert!(cam.position.distance(behind.0) < 1e-4);
        assert!(cam.forward().dot((behind.1 - behind.0).normalize()) > 0.9999);
        // it flies on its own; the avatar walking away does not take it
        if let Some(f) = cam.free_mut() {
            f.position += Vec3::new(0.0, 5.0, 2.0);
        }
        w.agent.position += Vec3::new(3.0, 0.0, 0.0);
        settle(&mut cam, &mut w, &s);
        assert!(cam.position.distance(behind.0 + Vec3::new(0.0, 5.0, 2.0)) < 1e-4);
        // back: from there, gliding to behind the avatar
        cam.end_free(&s);
        cam.update(&mut w, &s, &frame(1.0 / 60.0));
        assert!(cam.free().is_none());
        assert!(cam.position.distance(behind.0 + Vec3::new(0.0, 5.0, 2.0)) < 1.0);
        settle(&mut cam, &mut w, &s);
        assert!(cam.position.distance(behind.0 + Vec3::new(3.0, 0.0, 0.0)) < 0.01);
        // mouselook and a teleport end it
        cam.start_free(&mut w.agent, &s);
        cam.toggle_mouselook(&mut w.agent, &s);
        assert!(cam.free().is_none() && cam.mouselook());
        cam.toggle_mouselook(&mut w.agent, &s);
        cam.start_free(&mut w.agent, &s);
        cam.on_teleport_arrival(&s);
        assert!(cam.free().is_none());
    }

    #[test]
    fn agent_update_head_rotation_keeps_heading_in_third_person() {
        let settings = CameraSettings::default();
        let mut world = world_at(Vec3::new(128.0, 128.0, 25.0));
        let mut camera = Camera::default();
        for yaw in [-3.5, -1.46, 0.73, 1.8, 3.5] {
            world.agent.yaw = yaw;
            world.agent.pitch = 0.6;
            settle(&mut camera, &mut world, &settings);
            let controls = world.agent.controls(&crate::agent::MoveInput::default(), &camera, 128.0);
            assert!(controls.head_rotation.dot(world.agent.body_rotation()).abs() > 0.99999);
            camera.orbit_drag(100.0, 60.0, 1.0, &mut world.agent);
            settle(&mut camera, &mut world, &settings);
            let orbited = world.agent.controls(&crate::agent::MoveInput::default(), &camera, 128.0);
            assert!(orbited.head_rotation.dot(controls.head_rotation).abs() > 0.99999);
            assert_eq!(orbited.control_flags & aurora_net::control::MOUSELOOK, 0);
        }
    }

    #[test]
    fn agent_update_head_rotation_tracks_full_mouselook_aim() {
        let settings = CameraSettings::default();
        let mut world = world_at(Vec3::new(128.0, 128.0, 25.0));
        let mut camera = Camera::default();
        camera.change_to_mouselook(&mut world.agent, &settings, false);
        for (yaw, pitch) in [(-2.7, -0.9), (0.7, 0.6), (2.4, 1.4), (0.0, 0.0)] {
            world.agent.yaw = yaw;
            world.agent.pitch = pitch;
            settle(&mut camera, &mut world, &settings);
            let controls = world.agent.controls(&crate::agent::MoveInput::default(), &camera, 128.0);
            assert!((controls.head_rotation * Vec3::X).abs_diff_eq(camera.forward(), 1e-5));
            assert!(controls.head_rotation.is_normalized());
            assert_ne!(controls.control_flags & aurora_net::control::MOUSELOOK, 0);
        }
        camera.change_to_default(&mut world.agent, &settings);
        world.agent.yaw = 1.1;
        settle(&mut camera, &mut world, &settings);
        let controls = world.agent.controls(&crate::agent::MoveInput::default(), &camera, 128.0);
        assert!(controls.head_rotation.dot(world.agent.body_rotation()).abs() > 0.99999);
    }

    #[test]
    fn agent_update_head_rotation_is_relative_to_the_rotating_seat() {
        let settings = CameraSettings::default();
        let mut world = hover_world(true);
        let avatar = world.objects.index_of_uuid(&world.agent_id).unwrap();
        let parent = world.objects.parent_of(world.objects.get(avatar).unwrap()).unwrap();
        let local = Quat::from_rotation_z(-0.6) * Quat::from_rotation_x(0.2);
        world.objects.get_mut(avatar).unwrap().rotation = local;
        let mut camera = Camera::default();
        for yaw in [-1.2, 0.8, 2.5] {
            let seat = Quat::from_rotation_z(yaw) * Quat::from_rotation_y(0.7);
            world.objects.get_mut(parent).unwrap().rotation = seat;
            camera.change_to_default(&mut world.agent, &settings);
            settle(&mut camera, &mut world, &settings);
            let controls = world.agent.controls(&crate::agent::MoveInput::default(), &camera, 128.0);
            assert!(controls.head_rotation.dot(local).abs() > 0.99999);
            camera.change_to_mouselook(&mut world.agent, &settings, false);
            camera.look(75.0, -40.0, 1.0, &mut world.agent);
            settle(&mut camera, &mut world, &settings);
            let controls = world.agent.controls(&crate::agent::MoveInput::default(), &camera, 128.0);
            assert!((seat * controls.head_rotation * Vec3::X).abs_diff_eq(camera.forward(), 1e-5));
        }
    }

    #[test]
    fn joystick_flycam_keeps_the_avatar_and_sends_its_camera_frame() {
        let s = CameraSettings::default();
        let mut w = world_at(Vec3::new(128.0, 128.0, 25.0));
        let mut cam = Camera::default();
        settle(&mut cam, &mut w, &s);
        let avatar = (w.agent.position, w.agent.yaw, w.agent.pitch, w.agent.flying);
        let mut flight = crate::joystick::flycam::Flycam::from_view(cam.position, cam.target, Vec3::Z, cam.fov_y);
        let tuning = crate::joystick::Settings {
            auto_level: false,
            ..Default::default()
        };
        flight.step([2.0, 0.0, 1.0, 0.4, 0.3, 0.2, -0.5], &tuning, 0.2);
        cam.flycam = Some(flight);
        settle(&mut cam, &mut w, &s);
        assert_eq!(avatar, (w.agent.position, w.agent.yaw, w.agent.pitch, w.agent.flying));
        assert!(cam.position.distance(flight.position) < 1e-4);
        assert!(cam.view().is_finite());
        assert_eq!(cam.fov_y, flight.fov);
        let update = w.agent.controls(&crate::agent::MoveInput::default(), &cam, 128.0);
        assert!(update.camera_up.distance(flight.up()) < 1e-4);
        // A pre-flight focus correction must not narrow the retained lens a
        // second time when the ordinary camera resumes.
        cam.fov_zoom = 0.7;
        cam.current_fov_zoom = 0.7;
        cam.retain_flycam_view(&mut w.agent, &s);
        assert!(cam.flycam.is_none());
        cam.update(&mut w, &s, &frame(1.0 / 60.0));
        assert!(
            cam.position.distance(flight.position) < 0.01,
            "stopping must not blend from the pre-flight view"
        );
        settle(&mut cam, &mut w, &s);
        assert!(cam.position.distance(flight.position) < 0.01);
        assert!((cam.fov_y - flight.fov).abs() < 1e-4);
        assert!((cam.roll - flight.roll()).abs() < 1e-4);
        let held = w.agent.controls(&crate::agent::MoveInput::default(), &cam, 128.0);
        assert!(held.camera_up.distance(flight.up()) < 1e-4);
        let changed_lens = CameraSettings { fov: 0.9, ..s.clone() };
        assert_eq!(cam.base_fov(&changed_lens), 0.9, "a subsequent lens setting takes precedence");
        cam.update(&mut w, &changed_lens, &frame(1.0 / 60.0));
        cam.update(&mut w, &s, &frame(1.0 / 60.0));
        assert!(
            (cam.fov_y - s.fov).abs() < 1e-4,
            "restoring a preference must not restore the old flycam zoom"
        );
        cam.handle_reset_view(&mut w.agent, &s);
        assert_eq!(cam.roll, 0.0);
        assert!(cam.flycam_lens.is_none());
        cam.flycam = Some(flight);
        cam.shift(Vec3::new(-256.0, 0.0, 0.0));
        assert!(cam.flycam.expect("flight").position.distance(flight.position - Vec3::X * 256.0) < 1e-4);
        cam.handle_reset_view(&mut w.agent, &s);
        assert!(cam.flycam.is_none());
        cam.flycam = Some(flight);
        cam.change_to_mouselook(&mut w.agent, &s, false);
        assert!(cam.flycam.is_none());
        cam.flycam = Some(flight);
        cam.on_teleport_arrival(&s);
        assert!(cam.flycam.is_none());
        cam.flycam = Some(flight);
        cam.start_free(&mut w.agent, &s);
        assert!(cam.flycam.is_none() && cam.free().is_some());
    }

    #[test]
    fn joystick_flycam_accepts_lens_edits_and_uses_them_as_the_absolute_zoom_reference() {
        let mut s = CameraSettings::default();
        let mut w = world_at(Vec3::new(128.0, 128.0, 25.0));
        let mut cam = Camera::default();
        settle(&mut cam, &mut w, &s);
        cam.start_flycam(&s);
        let mut tuning = crate::joystick::Settings::default();
        tuning.mapping[6] = 6;
        tuning.flycam.gain[6] = 1.0;
        let flight = cam.flycam.as_mut().expect("flight");
        flight.step([0.0, 0.0, 0.0, 0.0, 0.0, 0.0, -0.5], &tuning, 0.2);
        let position = flight.position;
        cam.update(&mut w, &s, &frame(1.0 / 60.0));
        assert!((cam.fov_y - (s.fov - 0.1)).abs() < 1e-4);
        s.fov = 0.9;
        cam.update(&mut w, &s, &frame(1.0 / 60.0));
        assert!(
            (cam.fov_y - 0.9).abs() < 1e-4,
            "a lens preference edit must reach the active flycam"
        );
        assert_eq!(cam.position, position, "a lens edit must keep the flight pose");
        tuning.direct_zoom = true;
        for (input, expected) in [(0.2, 1.1), (0.0, 0.9)] {
            let flight = cam.flycam.as_mut().expect("flight");
            flight.step([0.0, 0.0, 0.0, 0.0, 0.0, 0.0, input], &tuning, 0.1);
            cam.update(&mut w, &s, &frame(1.0 / 60.0));
            assert!((cam.fov_y - expected).abs() < 1e-4, "absolute zoom must use the edited reference");
        }
        s.fov = 0.7;
        cam.update(&mut w, &s, &frame(1.0 / 60.0));
        cam.flycam.as_mut().expect("flight").step([0.0; 7], &tuning, 0.1);
        cam.update(&mut w, &s, &frame(1.0 / 60.0));
        assert!((cam.fov_y - 0.7).abs() < 1e-4, "absolute zoom must retain subsequent lens edits");
        cam.retain_flycam_view(&mut w.agent, &s);
        settle(&mut cam, &mut w, &s);
        assert!((cam.fov_y - 0.7).abs() < 1e-4);
    }

    #[test]
    fn joystick_flycam_keeps_rlva_camera_limits() {
        let s = CameraSettings::default();
        let mut w = world_at(Vec3::new(128.0, 128.0, 25.0));
        let mut cam = Camera::default();
        settle(&mut cam, &mut w, &s);
        let avatar = w.agent.position;
        w.rlva.configure(true);
        assert!(w.rlva.receive(&aurora_net::ChatMessage {
            from_name: "Camera test object".to_owned(),
            source_id: Uuid::from_u128(1),
            owner_id: Uuid::from_u128(2),
            source_type: aurora_net::ChatSourceType::Object,
            chat_type: aurora_net::ChatType::OwnerSay,
            position: avatar,
            message: "@setcam=n,setcam_avdistmax:2=n,setcam_fovmin:0.8=n".to_owned(),
        }));
        cam.flycam = Some(crate::joystick::flycam::Flycam::from_view(
            avatar + Vec3::X * 50.0,
            avatar,
            Vec3::Z,
            0.5,
        ));
        let f = frame(1.0 / 60.0);
        cam.update(&mut w, &s, &f);
        assert_eq!(w.agent.position, avatar);
        assert!(cam.position.distance(avatar) <= 2.001);
        let (min, max) = w.rlva.camera_fov_limits(f.aspect).expect("camera lens limits");
        assert!((min..=max).contains(&cam.fov_y));
        let pose = cam.flycam.expect("flycam stays active within the limits");
        assert_eq!(pose.position, cam.position);
        assert_eq!(pose.fov, cam.fov_y);
        let update = w.agent.controls(&crate::agent::MoveInput::default(), &cam, 128.0);
        assert!(update.camera_at.distance(pose.forward()) < 1e-4);
        assert!(update.camera_up.distance(pose.up()) < 1e-4);
    }

    #[test]
    fn alt_camera_overrides_script_and_reset_restores_it() {
        let s = CameraSettings::default();
        let mut w = world_at(Vec3::new(128.0, 128.0, 25.0));
        let mut cam = Camera::default();
        script(&mut w, Uuid::from_u128(123), vec![(12, 1.0), (7, 6.0)]);
        settle(&mut cam, &mut w, &s);
        let focus = Vec3::new(135.0, 130.0, 27.0);
        cam.alt_focus(focus, None, (cam.position, cam.forward()), &mut w.agent, &s);
        settle(&mut cam, &mut w, &s);
        assert!(!cam.focus_on_avatar);
        assert!(cam.target.distance(focus) < 0.001);
        cam.handle_reset_view(&mut w.agent, &s);
        settle(&mut cam, &mut w, &s);
        assert!(cam.focus_on_avatar);
        assert_eq!(cam.mode, CameraMode::Follow);
        assert!(cam.target.distance(focus) > 2.0);
    }

    #[test]
    fn teleport_restarts_unlocked_follow_at_the_arriving_avatar() {
        let s = CameraSettings::default();
        let mut w = world_at(Vec3::new(128.0, 128.0, 25.0));
        let mut cam = Camera::default();
        script(&mut w, Uuid::from_u128(123), vec![(12, 1.0), (7, 6.0)]);
        settle(&mut cam, &mut w, &s);
        w.agent.position = Vec3::new(128.0, 128.0, 2100.0);
        cam.on_teleport_arrival(&s);
        cam.update(&mut w, &s, &frame(1.0 / 60.0));
        assert!(cam.position.distance(w.agent.position) < 10.0);
        assert!(cam.target.distance(w.agent.position) < 3.0);
    }

    #[test]
    fn scripted_coincident_and_vertical_focus_produce_finite_views() {
        let mut cam = Camera {
            position: Vec3::new(128.0, 128.0, 25.0),
            ..Default::default()
        };
        for target in [cam.position, cam.position + Vec3::Z, cam.position - Vec3::Z] {
            cam.target = target;
            let view = cam.view();
            assert!(view.is_finite());
            assert!((view.determinant() - 1.0).abs() < 0.001);
        }
    }

    #[test]
    fn killing_script_source_exposes_previous_source_and_logout_clears_it() {
        let s = CameraSettings::default();
        let mut w = world_at(Vec3::new(128.0, 128.0, 25.0));
        w.agent_id = crate::demo::DEMO_AGENT;
        let mut cam = Camera::default();
        for ev in crate::demo::sit_events(250) {
            if let aurora_net::NetEvent::ObjectUpdates { handle, .. } = &ev {
                w.main_region = Some(*handle);
            }
            w.apply(ev);
        }
        let source = w
            .objects
            .iter()
            .find(|(_, o)| !o.is_avatar())
            .map(|(_, o)| (o.full_id, o.key))
            .expect("seat");
        let previous = Uuid::from_u128(123);
        let handle = w.main_region.expect("region");
        for (object, distance) in [(previous, 6.0), (source.0, 9.0)] {
            w.apply(aurora_net::NetEvent::SetFollowCam {
                handle,
                object,
                properties: vec![(12, 1.0), (7, distance)],
            });
        }
        settle(&mut cam, &mut w, &s);
        assert_eq!(cam.follow.distance, 9.0);
        w.apply(aurora_net::NetEvent::ObjectsKilled {
            handle,
            local_ids: vec![source.1.local_id],
        });
        settle(&mut cam, &mut w, &s);
        assert_eq!(cam.follow.distance, 6.0);
        w.reset();
        assert!(w.scripted_camera.active().is_none());
    }

    #[test]
    fn rear_view_sits_behind_and_above_the_avatar() {
        let s = CameraSettings::default();
        let mut w = world_at(Vec3::new(128.0, 128.0, 25.0));
        let mut cam = Camera::default();
        settle(&mut cam, &mut w, &s);
        // CameraOffsetRearView (-3, 0, 0.75) above the 1 m head offset
        assert!(cam.position.distance(Vec3::new(125.0, 128.0, 26.75)) < 0.01, "{:?}", cam.position);
        // FocusOffsetRearView (1, 0, 1)
        assert!(cam.target.distance(Vec3::new(129.0, 128.0, 26.0)) < 0.01, "{:?}", cam.target);
    }

    /// Seated on an object that sets no seat camera, the rear view is behind
    /// the avatar on its seat, wherever it stood before sitting.
    #[test]
    fn seated_rear_view_goes_with_the_seat() {
        let s = CameraSettings::default();
        let stood = Vec3::new(132.0, 126.0, 32.9);
        let mut w = world_at(stood);
        w.agent_id = crate::demo::DEMO_AGENT;
        for frame in [250, 300] {
            // (the seat camera of the SitResponse is not given to the camera)
            for event in crate::demo::sit_events(frame) {
                if let aurora_net::NetEvent::ObjectUpdates { handle, .. } = &event {
                    w.main_region = Some(*handle);
                }
                w.apply(event);
            }
        }
        let mut cam = Camera::default();
        settle(&mut cam, &mut w, &s);
        let seated = Scene::seated_agent_position(&w, Instant::now()).expect("seated");
        assert!(seated.distance(stood) > 5.0, "{seated:?}");
        // CameraOffsetRearView (-3, 0, 0.75) and FocusOffsetRearView (1, 0, 1),
        // without the head offset of a standing avatar
        assert!((cam.position.distance(seated) - 3.09).abs() < 0.05, "{:?}", cam.position);
        assert!((cam.target.distance(seated) - 1.41).abs() < 0.05, "{:?}", cam.target);
    }

    /// LLAgentCamera::updateCamera: seated in mouselook the view's up is the
    /// avatar's, so it banks and pitches with a vehicle; in third person and
    /// standing it stays the world's.
    #[test]
    fn seated_mouselook_banks_with_the_vehicle() {
        let mut world = hover_world(true);
        let settings = CameraSettings::default();
        let mut camera = Camera::default();
        settle(&mut camera, &mut world, &settings);
        let seat = world
            .objects
            .iter()
            .find(|(_, o)| !o.is_avatar() && o.parent_id == 0)
            .map(|(idx, _)| idx)
            .unwrap();
        let banked = Quat::from_rotation_z(0.6) * Quat::from_rotation_x(0.5);
        world.objects.get_mut(seat).unwrap().rotation = banked;
        world.objects.settle(seat);
        camera.update(&mut world, &settings, &frame(1.0 / 60.0));
        assert!(camera.view_up().abs_diff_eq(Vec3::Z, 1e-5), "third person keeps the world's up");
        camera.change_to_mouselook(&mut world.agent, &settings, false);
        camera.update(&mut world, &settings, &frame(1.0 / 60.0));
        let index = world.objects.index_of_uuid(&world.agent_id).unwrap();
        let (_, rotation, _) = Scene::object_transform_unshifted(&world, index, Instant::now()).unwrap();
        let up = rotation * Vec3::Z;
        assert!(up.angle_between(Vec3::Z) > 0.3, "the seat is banked");
        assert!(camera.view_up().abs_diff_eq(up, 1e-4), "{:?} vs {up:?}", camera.view_up());
        // the vehicle turns and banks the other way: the view follows it
        let turned = Quat::from_rotation_z(-0.9) * Quat::from_rotation_x(-0.4);
        world.objects.get_mut(seat).unwrap().rotation = turned;
        world.objects.settle(seat);
        camera.update(&mut world, &settings, &frame(1.0 / 60.0));
        let (_, rotation, _) = Scene::object_transform_unshifted(&world, index, Instant::now()).unwrap();
        assert!(camera.view_up().abs_diff_eq(rotation * Vec3::Z, 1e-4));
        camera.change_to_default(&mut world.agent, &settings);
        camera.update(&mut world, &settings, &frame(10.0));
        assert!(camera.view_up().abs_diff_eq(Vec3::Z, 1e-5), "back in third person");
    }

    #[test]
    fn seated_mouselook_tracks_animated_eyes_without_moving_them_when_looking() {
        let mut world = hover_world(true);
        let mut settings = CameraSettings::default();
        let mut camera = Camera::default();
        settle(&mut camera, &mut world, &settings);
        let seat = world
            .objects
            .iter()
            .find(|(_, o)| !o.is_avatar() && o.parent_id == 0)
            .unwrap()
            .1
            .full_id;
        camera.on_sit_response(seat, Vec3::splat(8.0), Vec3::splat(-5.0), false);
        camera.change_to_mouselook(&mut world.agent, &settings, false);
        let index = world.objects.index_of_uuid(&world.agent_id).unwrap();
        let (position, rotation, _) = Scene::object_transform_unshifted(&world, index, Instant::now()).unwrap();
        world.avatar_shape_dz.insert(world.agent_id, 0.17);
        for follow_hover in [false, true] {
            settings.hover_affects_camera = follow_hover;
            for hover in [-0.4, 0.0, 0.6] {
                world.hover_height.preview(hover);
                // Upright, lowered and reclined animated eye positions.
                for eye_offset in [Vec3::new(0.1, 0.0, 0.7), Vec3::new(0.35, 0.2, 0.22), Vec3::new(0.6, -0.1, 0.1)] {
                    let mut input = frame(1.0 / 60.0);
                    input.eye_offset = Some(eye_offset);
                    let height = if follow_hover { hover } else { 0.0 };
                    let expected = position + rotation * (eye_offset + Vec3::Z * (0.17 + height + 0.1));
                    for (dx, dy) in [(0.0, 0.0), (40.0, -80.0), (-100.0, 140.0)] {
                        camera.look(dx, dy, 1.0, &mut world.agent);
                        camera.update(&mut world, &settings, &input);
                        assert!(camera.position.abs_diff_eq(expected, 2e-5), "{:?} vs {expected:?}", camera.position);
                        let (yaw, pitch) = (world.agent.yaw, world.agent.pitch);
                        let aim = Vec3::new(yaw.cos() * pitch.cos(), yaw.sin() * pitch.cos(), pitch.sin());
                        assert!(camera.forward().abs_diff_eq(aim, 2e-5));
                    }
                }
            }
        }
        // Standing resumes the existing first-person view, with no stale pose.
        for event in crate::demo::sit_events(700) {
            world.apply(event);
        }
        camera.update(&mut world, &settings, &frame(1.0 / 60.0));
        let position = Scene::object_transform_unshifted(&world, index, Instant::now()).unwrap().0;
        assert!(
            camera
                .position
                .abs_diff_eq(position + Vec3::Z * HEAD_HEIGHT + world.agent.forward() * 0.12, 2e-5)
        );
    }

    #[test]
    fn seated_mouselook_keeps_mouse_pitch_when_terrain_moves_the_eye() {
        let mut world = hover_world(true);
        let settings = CameraSettings::default();
        let mut camera = Camera::default();
        settle(&mut camera, &mut world, &settings);
        camera.change_to_mouselook(&mut world.agent, &settings, false);
        let mut input = frame(1.0 / 60.0);
        input.eye_offset = Some(Vec3::new(0.15, 0.0, 0.3));
        let at = avatar_frame(&world, &settings, input.now).position;
        let floor = at.z + 2.0;
        world
            .regions
            .get_mut(&world.main_region.unwrap())
            .unwrap()
            .heightmap
            .heights
            .fill(floor);
        for (dx, dy) in [(0.0, -130.0), (70.0, 260.0), (-40.0, -130.0)] {
            camera.look(dx, dy, 1.0, &mut world.agent);
            camera.update(&mut world, &settings, &input);
            assert_eq!(camera.position.z, floor);
            let (yaw, pitch) = (world.agent.yaw, world.agent.pitch);
            let aim = Vec3::new(yaw.cos() * pitch.cos(), yaw.sin() * pitch.cos(), pitch.sin());
            assert!(camera.forward().abs_diff_eq(aim, 2e-5), "{:?} vs {aim:?}", camera.forward());
            assert!((camera.target.distance(camera.position) - 1.0).abs() < 2e-5);
        }
    }

    /// A camera at rest must not move at all, whatever the frame times: high
    /// above the ground the smallest change of the view shows on the edges
    /// of close objects. Coordinates and frame times of the report (alt
    /// camera at 2 106 m, 110 to 180 frames per second); several views,
    /// since only some coordinates were unstable (about four in ten).
    #[test]
    fn a_camera_at_rest_stays_bit_identical_whatever_the_frame_time() {
        let s = CameraSettings::default();
        let avatar = Vec3::new(108.2, 187.6, 2105.8);
        for view in 0..24 {
            let mut w = world_at(avatar);
            let mut cam = Camera::default();
            settle(&mut cam, &mut w, &s);
            // even views: the alt camera on a point; odd ones: the rear view
            if view % 2 == 0 {
                let v = view as f32;
                let focus = avatar + Vec3::new(3.1 + 0.37 * v, -2.3 - 0.21 * v, 0.4 + 0.013 * v);
                cam.set_view(focus + Vec3::new(-0.21, 0.33 + 0.01 * v, 0.12), focus, &mut w.agent, &s);
            } else {
                w.agent.position += Vec3::new(0.731, 0.517, 0.0) * view as f32;
            }
            // (longer than the animation to the new view, then a long frame
            // so that the smoothing has nothing left to catch up)
            settle(&mut cam, &mut w, &s);
            cam.update(&mut w, &s, &frame(0.1));
            let rest = (cam.position, cam.target, cam.fov_y);
            let mut moved = 0;
            for i in 0..500 {
                let dt = 0.0055 + 0.0035 * ((i * 37 % 101) as f32 / 100.0);
                cam.update(&mut w, &s, &frame(dt));
                moved += usize::from((cam.position, cam.target, cam.fov_y) != rest);
            }
            assert_eq!(moved, 0, "frames away from the rest position in view {view}: {:?}", cam.position);
        }
    }

    #[test]
    fn wheel_zooms_out_by_a_fourth_root_of_two_then_into_mouselook() {
        let mut s = CameraSettings::default();
        let mut w = world_at(Vec3::new(128.0, 128.0, 25.0));
        let mut cam = Camera::default();
        settle(&mut cam, &mut w, &s);
        let before = cam.target_distance;
        let mut agent = w.agent.clone();
        cam.scroll(1.0, false, false, &mut agent, &mut s);
        settle(&mut cam, &mut w, &s);
        assert!((cam.target_distance / before - 2f32.sqrt().sqrt()).abs() < 0.01);
        for _ in 0..20 {
            cam.scroll(-1.0, false, false, &mut agent, &mut s);
            cam.update(&mut w, &s, &frame(1.0 / 60.0));
        }
        assert!(cam.mouselook());
    }

    #[test]
    fn ctrl_and_shift_wheel_raise_the_offsets() {
        let mut s = CameraSettings::default();
        let mut w = world_at(Vec3::new(128.0, 128.0, 25.0));
        let mut cam = Camera::default();
        settle(&mut cam, &mut w, &s);
        let mut agent = w.agent.clone();
        assert!(cam.scroll(2.0, true, false, &mut agent, &mut s));
        assert!((s.camera_offset[2] - 0.95).abs() < 1e-5);
        assert!(cam.scroll(-1.0, false, true, &mut agent, &mut s));
        assert!((s.focus_offset[2] - 0.9).abs() < 1e-5);
        assert_eq!(s.preset, CameraPreset::Custom);
    }

    /// The camera limits on, as in the Legacy Viewer (Aurora turns them off
    /// by default).
    fn constrained() -> CameraSettings {
        CameraSettings {
            disable_constraints: false,
            ignore_sim_constraints: false,
            ..CameraSettings::default()
        }
    }

    #[test]
    fn border_crossing_moves_the_camera_with_the_agent() {
        let s = constrained();
        let mut w = world_at(Vec3::new(1.0, 128.0, 25.0));
        let mut cam = Camera::default();
        cam.set_collide_plane(Vec4::new(1.0, 0.0, 0.0, -20.0), &s);
        settle(&mut cam, &mut w, &s);
        let (eye, at) = (cam.position, cam.target);
        // the region to the west becomes the agent's: + 256 m on x
        let delta = Vec3::new(256.0, 0.0, 0.0);
        w.agent.shift(delta);
        cam.shift(delta);
        assert_eq!(cam.collide_plane.w, 236.0);
        // the next frame shows the same view, not a camera on its way there
        cam.update(&mut w, &s, &frame(1.0 / 60.0));
        assert!(cam.position.distance(eye + delta) < 0.01, "{:?}", cam.position);
        assert!(cam.target.distance(at + delta) < 0.01, "{:?}", cam.target);
    }

    #[test]
    fn collide_plane_keeps_the_camera_in_front_of_a_wall() {
        let s = constrained();
        let mut w = world_at(Vec3::new(128.0, 128.0, 25.0));
        let mut cam = Camera::default();
        // a wall at x = 126.5 facing +x, the avatar on its positive side
        cam.set_collide_plane(Vec4::new(1.0, 0.0, 0.0, 126.5), &s);
        settle(&mut cam, &mut w, &s);
        assert!(cam.position.x > 126.5, "{:?}", cam.position);
        let unconstrained = CameraSettings {
            disable_constraints: true,
            ..constrained()
        };
        settle(&mut cam, &mut w, &unconstrained);
        assert!(cam.position.x < 126.0, "{:?}", cam.position);
    }

    #[test]
    fn latched_crouch_preserves_camera_focus_and_orbit_until_movement() {
        use crate::agent::MoveInput;
        use crate::movement::crouch_toggle::CrouchToggle;

        let s = CameraSettings::default();
        let mut w = world_at(Vec3::new(128.0, 128.0, 25.0));
        let mut cam = Camera::default();
        settle(&mut cam, &mut w, &s);
        let mut crouch = CrouchToggle::default();
        // Exercise both a held toggle key and the persistent state after release.
        for held in [true, true, false, false] {
            let i = MoveInput {
                down: crouch.apply(true, false, false, held, false),
                ..Default::default()
            };
            assert!(i.down, "the simulator still receives the crouch control");
            assert_ne!(w.agent.control_flags(&i, false) & aurora_net::control::UP_NEG, 0);
            for on_avatar in [true, false] {
                cam.handle_reset_view(&mut w.agent, &s);
                if !on_avatar {
                    cam.set_view(Vec3::new(120.0, 120.0, 28.0), Vec3::new(130.0, 128.0, 26.0), &mut w.agent, &s);
                    cam.pan(Vec3::new(0.0, 0.5, 0.3));
                }
                cam.orbit_drag(-100.0, 20.0, 1.0, &mut w.agent);
                cam.set_zoom_fraction(0.7);
                let focus = cam.focus_point();
                let orbit = cam.orbit_yaw;
                let pitch = cam.pitch;
                let zoom = cam.zoom_fraction();
                if i.resets_camera(crouch.latched()) {
                    cam.reset_view(&mut w.agent, &s, true, false, true, false);
                }
                assert_eq!(cam.focus_point(), focus, "crouching must retain the chosen focus");
                assert_eq!(cam.orbit_yaw, orbit, "crouching must retain the chosen orbit");
                assert_eq!(cam.pitch, pitch);
                assert_eq!(cam.zoom_fraction(), zoom);
            }
        }
        let i = MoveInput {
            forward: true,
            down: true,
            ..Default::default()
        };
        assert!(i.resets_camera(crouch.latched()), "walking still returns to the avatar");
        cam.reset_view(&mut w.agent, &s, true, false, true, false);
        assert_eq!(cam.focus_point(), None);
        assert_eq!(cam.orbit_yaw, 0.0);
    }

    #[test]
    fn moving_folds_the_orbit_back_into_the_avatar_heading() {
        let s = CameraSettings::default();
        let mut w = world_at(Vec3::new(128.0, 128.0, 25.0));
        let mut cam = Camera::default();
        settle(&mut cam, &mut w, &s);
        let mut agent = w.agent.clone();
        cam.orbit_drag(-200.0, 0.0, 1.0, &mut agent);
        assert!(cam.orbit_yaw > 0.5);
        let heading = agent.yaw + cam.orbit_yaw;
        cam.reset_view(&mut agent, &s, true, false, true, false);
        assert_eq!(cam.orbit_yaw, 0.0);
        assert!((agent.yaw - heading).abs() < 1e-5);
        // FSResetCameraOnMovement off: the orbit stays
        let keep = CameraSettings {
            reset_on_movement: false,
            ..CameraSettings::default()
        };
        cam.orbit_drag(-200.0, 0.0, 1.0, &mut agent);
        cam.reset_view(&mut agent, &keep, true, false, true, false);
        assert!(cam.orbit_yaw > 0.5);
    }

    #[test]
    fn mouse_steering_turns_the_avatar_from_where_the_camera_looks() {
        let s = CameraSettings::default();
        let mut w = world_at(Vec3::new(128.0, 128.0, 25.0));
        let mut cam = Camera::default();
        settle(&mut cam, &mut w, &s);
        let mut agent = w.agent.clone();
        // a right drag orbit, then the left button held on our avatar
        cam.orbit_drag(-200.0, 0.0, 1.0, &mut agent);
        let heading = agent.yaw + cam.orbit_yaw;
        cam.begin_drag(true);
        cam.drag(ToolMode::Zoom, 0.0, 0.0, 1280.0, &mut agent);
        assert!(cam.orbit_yaw > 0.5, "a still press changes nothing");
        cam.drag(ToolMode::Zoom, -40.0, 0.0, 1280.0, &mut agent);
        assert_eq!(cam.orbit_yaw, 0.0);
        let turned = TAU / 1280.0 * 40.0;
        assert!((agent.yaw - (heading + turned)).abs() < 1e-5, "the mouse turns the avatar itself");
        cam.end_drag();
        // a key pressed while steering: the avatar walks where the camera looks
        cam.orbit_drag(-200.0, 0.0, 1.0, &mut agent);
        let heading = agent.yaw + cam.orbit_yaw;
        cam.reset_view(&mut agent, &s, true, false, true, true);
        assert_eq!(cam.orbit_yaw, 0.0);
        assert!((agent.yaw - heading).abs() < 1e-5);
    }

    #[test]
    fn reset_to_defaults_drops_orbit_pitch_and_zoom_without_turning_the_avatar() {
        let s = CameraSettings::default();
        let mut w = world_at(Vec3::new(128.0, 128.0, 25.0));
        let mut cam = Camera::default();
        settle(&mut cam, &mut w, &s);
        let rest = cam.position;
        let mut agent = w.agent.clone();
        cam.orbit_drag(-200.0, 60.0, 1.0, &mut agent);
        cam.zoom_fraction = 0.4;
        cam.fov_zoom = 0.3;
        let heading = agent.yaw;
        cam.reset_to_defaults(&mut agent, &s);
        assert_eq!((cam.orbit_yaw, cam.pitch, cam.zoom_fraction, cam.fov_zoom), (0.0, 0.0, 1.0, 0.0));
        assert_eq!(agent.yaw, heading, "the avatar keeps its heading");
        w.agent = agent;
        settle(&mut cam, &mut w, &s);
        assert!(
            cam.position.distance(rest) < 0.05,
            "back to the rear view: {:?} vs {rest:?}",
            cam.position
        );
    }

    #[test]
    fn mouselook_transition_lasts_zoom_time() {
        let s = CameraSettings::default();
        let mut w = world_at(Vec3::new(128.0, 128.0, 25.0));
        let mut cam = Camera::default();
        settle(&mut cam, &mut w, &s);
        cam.change_to_mouselook(&mut w.agent, &s, true);
        cam.update(&mut w, &s, &frame(0.1));
        assert!(cam.anim.is_some());
        for _ in 0..4 {
            cam.update(&mut w, &s, &frame(0.1));
        }
        assert!(cam.anim.is_none());
        assert!(cam.position.distance(Vec3::new(128.12, 128.0, 25.7)) < 0.01, "{:?}", cam.position);
    }

    #[test]
    fn mouselook_visibility_waits_for_the_entry_transition() {
        let s = CameraSettings {
            zoom_time: 1.0,
            ..Default::default()
        };
        let mut world = world_at(Vec3::new(128.0, 128.0, 25.0));
        let mut camera = Camera::default();
        settle(&mut camera, &mut world, &s);
        camera.change_to_mouselook(&mut world.agent, &s, true);
        assert!(!camera.first_person_view(), "a mode request must not hide the head");
        let mut visible_frames = 0;
        for _ in 0..120 {
            camera.update(&mut world, &s, &frame(1.0 / 120.0));
            if camera.anim.is_some() {
                visible_frames += 1;
                assert!(!camera.first_person_view());
            } else {
                assert!(camera.first_person_view());
                assert!(
                    camera
                        .position
                        .abs_diff_eq(world.agent.position + Vec3::new(0.12, 0.0, HEAD_HEIGHT), 1e-5)
                );
                break;
            }
        }
        assert!(visible_frames > 60, "the head stays visible through the approach");
        camera.start_animation(&s);
        assert!(camera.first_person_view(), "a same-mode animation must not restore the head");
        camera.change_to_default(&mut world.agent, &s);
        assert!(!camera.first_person_view());
        camera.change_to_mouselook(&mut world.agent, &s, false);
        camera.update(&mut world, &s, &frame(1.0 / 60.0));
        assert!(camera.first_person_view(), "instant entry has no approach to wait for");
    }

    #[test]
    fn ground_mouselook_uses_the_posed_height_and_restores_standing() {
        let mut world = hover_world(false);
        let mut settings = CameraSettings::default();
        let mut camera = Camera::default();
        settle(&mut camera, &mut world, &settings);
        world.agent.ground_sit = true;
        camera.change_to_mouselook(&mut world.agent, &settings, false);
        for follow_hover in [false, true] {
            settings.hover_affects_camera = follow_hover;
            world.hover_height.preview(0.3);
            for height in [-0.25, 0.1, 0.4] {
                let mut input = frame(1.0 / 60.0);
                input.eye_offset = Some(Vec3::new(0.5, -0.3, height));
                assert!(camera.needs_animated_eyes(&world.agent, &settings));
                camera.update(&mut world, &settings, &input);
                let expected = camera.avatar.root + camera.avatar.render_rotation * Vec3::Z * (height + 0.1);
                assert!(camera.position.abs_diff_eq(expected, 1e-5), "{:?} vs {expected:?}", camera.position);
            }
        }
        world.agent.ground_sit = false;
        camera.update(&mut world, &settings, &frame(1.0 / 60.0));
        assert!(!camera.needs_animated_eyes(&world.agent, &settings));
        let expected = camera.avatar.position + Vec3::Z * HEAD_HEIGHT + world.agent.forward() * 0.12;
        assert!(camera.position.abs_diff_eq(expected, 1e-5));
    }

    #[test]
    fn mouselook_eye_following_moves_the_position_without_rotating_mouse_aim() {
        let mut settings = CameraSettings {
            mouselook_follow_eyes: true,
            hover_affects_camera: true,
            ..Default::default()
        };
        for (seated, ground) in [(false, false), (false, true), (true, false)] {
            let mut world = hover_world(seated);
            world.agent.ground_sit = ground;
            world.hover_height.preview(0.2);
            let mut camera = Camera::default();
            camera.change_to_mouselook(&mut world.agent, &settings, false);
            let mut previous = None;
            for (follow_hover, eyes) in [
                (false, Vec3::new(0.1, 0.0, 0.65)),
                (true, Vec3::new(-0.15, 0.25, 0.2)),
                (false, Vec3::new(0.3, -0.2, -0.1)),
            ] {
                settings.hover_affects_camera = follow_hover;
                let mut input = frame(1.0 / 60.0);
                input.eye_offset = Some(eyes);
                assert!(camera.needs_animated_eyes(&world.agent, &settings));
                camera.update(&mut world, &settings, &input);
                let index = world.objects.index_of_uuid(&world.agent_id).unwrap();
                let (root, rotation, _) = Scene::object_transform(&world, index, input.now, 0).unwrap();
                let expected = root + rotation * eyes;
                assert!(camera.position.abs_diff_eq(expected, 2e-5));
                let (yaw, pitch) = (world.agent.yaw, world.agent.pitch);
                let aim = Vec3::new(yaw.cos() * pitch.cos(), yaw.sin() * pitch.cos(), pitch.sin());
                assert!(camera.forward().abs_diff_eq(aim, 2e-5));
                if let Some(previous) = previous {
                    assert!(camera.position.distance(previous) > 0.1);
                }
                previous = Some(camera.position);
                camera.look(30.0, -20.0, 1.0, &mut world.agent);
            }
        }
    }

    #[test]
    fn the_held_right_button_zooms_mouselook_at_once() {
        // a viewing angle that is not the default 60°
        let normal = 59.5f32.to_radians();
        let mut s = CameraSettings {
            fov: normal,
            ..Default::default()
        };
        let mut w = world_at(Vec3::new(128.0, 128.0, 25.0));
        let mut cam = Camera::default();
        settle(&mut cam, &mut w, &s);
        let aspect = 16.0 / 9.0;
        // outside mouselook the button does not zoom
        cam.press_mouselook_zoom(&s, aspect);
        assert!(!cam.mouselook_zoom_held());
        cam.change_to_mouselook(&mut w.agent, &s, false);
        cam.update(&mut w, &s, &frame(1.0 / 60.0));
        // a right click without the wheel changes nothing, then or after
        cam.press_mouselook_zoom(&s, aspect);
        cam.update(&mut w, &s, &frame(1.0 / 60.0));
        assert_eq!(cam.fov_y, normal);
        assert!(cam.release_mouselook_zoom(&mut s));
        assert_eq!(s.mouselook_zoom_fov, None);
        cam.update(&mut w, &s, &frame(1.0 / 60.0));
        assert_eq!(cam.fov_y, normal);
        // the wheel while held: at once, remembered, the normal angle exact after
        cam.press_mouselook_zoom(&s, aspect);
        assert!(cam.scroll_mouselook_zoom(-3.0, aspect));
        cam.update(&mut w, &s, &frame(1.0 / 60.0));
        assert!((cam.fov_y - (normal - 0.3)).abs() < 1e-5, "instant: {}", cam.fov_y);
        assert!(cam.release_mouselook_zoom(&mut s));
        assert!((s.mouselook_zoom_fov.unwrap() - (normal - 0.3)).abs() < 1e-5);
        assert_eq!(s.fov, normal, "the normal angle is never written");
        cam.update(&mut w, &s, &frame(1.0 / 60.0));
        assert_eq!(cam.fov_y, normal);
        // without the button the wheel is the usual one
        assert!(!cam.scroll_mouselook_zoom(1.0, aspect));
    }

    #[test]
    fn smoothing_follows_with_a_delay() {
        let s = CameraSettings::default();
        let mut w = world_at(Vec3::new(128.0, 128.0, 25.0));
        let mut cam = Camera::default();
        settle(&mut cam, &mut w, &s);
        cam.orbit_drag(-100.0, 0.0, 1.0, &mut w.agent);
        cam.update(&mut w, &s, &frame(1.0 / 60.0));
        let first = cam.position;
        settle(&mut cam, &mut w, &s);
        assert!(first.distance(cam.position) > 0.01);
        let still = CameraSettings {
            smoothing: 0.0,
            ..CameraSettings::default()
        };
        cam.orbit_drag(-100.0, 0.0, 1.0, &mut w.agent);
        cam.update(&mut w, &still, &frame(1.0 / 60.0));
        let first = cam.position;
        settle(&mut cam, &mut w, &still);
        assert!(first.distance(cam.position) < 0.01);
    }

    fn focused_object() -> (World, Camera, CameraSettings, usize) {
        focused_object_with(CameraSettings::default())
    }

    fn focused_object_with(s: CameraSettings) -> (World, Camera, CameraSettings, usize) {
        let mut w = world_at(Vec3::new(128.0, 128.0, 25.0));
        for ev in crate::demo::events() {
            w.apply(ev);
        }
        let mut object = crate::demo::hud::object(31, false);
        object.parent_id = 0;
        object.state = 0;
        object.position = Vec3::new(128.0, 128.0, 32.0);
        object.scale = Vec3::ONE;
        let id = object.full_id;
        w.apply(aurora_net::NetEvent::ObjectUpdates {
            handle: w.main_region.unwrap(),
            objects: vec![object],
        });
        let idx = w.objects.index_of_uuid(&id).unwrap();
        let g = focus_object_geom(&w, &id, Instant::now()).unwrap();
        let mut cam = Camera {
            focus_on_avatar: false,
            ..Default::default()
        };
        cam.set_camera_pos_and_focus(g.position + Vec3::new(-8.0, -8.0, 5.0), g.position, Some((id, g)), &s);
        settle(&mut cam, &mut w, &s);
        (w, cam, s, idx)
    }

    #[test]
    fn mouse_steering_turns_the_drawn_avatar_with_follow_or_kept_focus() {
        for follow in [true, false] {
            for dx in [-256.0, 256.0] {
                let (mut w, mut cam, s, _) = focused_object();
                w.agent.yaw = 0.0;
                w.agent.reset_motion();
                if follow {
                    cam.set_focus_on_avatar(true, false, false, &mut w.agent, &s);
                    settle(&mut cam, &mut w, &s);
                }
                w.update_bodies(Instant::now(), 1.0 / 60.0, cam.forward(), false, false);
                let heading = w.agent.yaw;
                let body = w.bodies[&w.agent_id].rotation;
                let focus = (cam.focus_point(), cam.focus_object_id());
                let distance = cam.position.distance(cam.target);
                cam.begin_drag(true);
                cam.drag(ToolMode::Zoom, 2.0, 0.0, 1280.0, &mut w.agent);
                assert_eq!(w.agent.yaw, heading, "a click inside the drag threshold must not turn");
                cam.drag(ToolMode::Zoom, dx, 0.0, 1280.0, &mut w.agent);
                let expected = heading - dx * TAU / 1280.0;
                assert!((w.agent.yaw - expected).abs() < 1e-5, "follow={follow}, dx={dx}");
                assert_eq!((cam.focus_point(), cam.focus_object_id()), focus);
                assert_eq!(cam.focus_on_avatar, follow);
                let mut turning = false;
                for _ in 0..120 {
                    let input = FrameInput {
                        steering: true,
                        ..frame(1.0 / 60.0)
                    };
                    cam.update(&mut w, &s, &input);
                    let turn = w.update_bodies(input.now, input.dt, cam.forward(), false, false);
                    turning |= turn == if dx < 0.0 { 1 } else { -1 };
                }
                assert!(turning, "the drawn body must request the matching turn animation");
                assert!(body.angle_between(w.bodies[&w.agent_id].rotation) > 0.5);
                assert!((cam.position.distance(cam.target) - distance).abs() < 0.02);
                cam.end_drag();
                cam.drag(ToolMode::Zoom, dx, 0.0, 1280.0, &mut w.agent);
                assert!((w.agent.yaw - expected).abs() < 1e-5, "release ends steering");
            }
        }
    }

    #[test]
    fn focus_drag_and_vertical_steering_do_not_turn_the_avatar() {
        for mode in [ToolMode::Zoom, ToolMode::Orbit, ToolMode::Pan] {
            let (mut w, mut cam, _, _) = focused_object();
            let heading = w.agent.yaw;
            cam.begin_drag(false);
            cam.drag(mode, 256.0, 0.0, 1280.0, &mut w.agent);
            assert_eq!(w.agent.yaw, heading, "{mode:?} keeps independent camera control");
            cam.begin_drag(true);
            cam.drag(ToolMode::Zoom, 0.0, 40.0, 1280.0, &mut w.agent);
            assert_eq!(w.agent.yaw, heading, "vertical steering only tilts the view");
            cam.orbit_drag(256.0, 0.0, 1.0, &mut w.agent);
            assert_eq!(w.agent.yaw, heading, "right drag keeps independent camera control");
        }
    }

    #[test]
    fn edit_focus_freezes_movement_and_size_then_resumes_without_a_jump() {
        let (mut w, mut cam, s, idx) = focused_object();
        let before = (cam.position, cam.target, cam.fov_y);
        let mut f = frame(1.0 / 60.0);
        f.build_mode = true;
        f.edit_tracking = Some(false);
        for _ in 0..60 {
            let o = w.objects.get_mut(idx).unwrap();
            o.position += Vec3::Y * 0.05;
            o.scale += Vec3::ONE;
            cam.update(&mut w, &s, &f);
            assert!(cam.position.abs_diff_eq(before.0, 1e-4));
            assert!(cam.target.abs_diff_eq(before.1, 1e-4));
            assert!((cam.fov_y - before.2).abs() < 1e-5);
        }
        // Resume tracking at the object's new position without recentering.
        // Keep its size inside the camera distance for this position check.
        w.objects.get_mut(idx).unwrap().scale = Vec3::ONE;
        f.edit_tracking = Some(true);
        cam.update(&mut w, &s, &f);
        assert!(cam.position.abs_diff_eq(before.0, 1e-4));
        w.objects.get_mut(idx).unwrap().position += Vec3::Y * 0.1;
        cam.update(&mut w, &s, &f);
        assert!(cam.target.abs_diff_eq(before.1 + Vec3::Y * 0.1, 1e-4));
        f.edit_tracking = None;
        w.objects.get_mut(idx).unwrap().position += Vec3::Y * 0.1;
        cam.update(&mut w, &s, &f);
        assert!(cam.target.abs_diff_eq(before.1 + Vec3::Y * 0.2, 1e-4));
    }

    #[test]
    fn enabled_edit_tracking_follows_every_local_frame() {
        let (mut w, mut cam, s, idx) = focused_object();
        let mut f = frame(1.0 / 60.0);
        f.build_mode = true;
        f.edit_tracking = Some(true);
        for _ in 0..60 {
            let before = (cam.position, cam.target);
            w.objects.get_mut(idx).unwrap().position += Vec3::Y * 0.05;
            cam.update(&mut w, &s, &f);
            assert!(cam.position.abs_diff_eq(before.0 + Vec3::Y * 0.05, 1e-4));
            assert!(cam.target.abs_diff_eq(before.1 + Vec3::Y * 0.05, 1e-4));
        }
    }

    #[test]
    fn zoom_on_a_resident_faces_them_and_follows_them() {
        let s = CameraSettings::default();
        let mut w = world_at(Vec3::new(128.0, 128.0, 25.0));
        for ev in crate::demo::events() {
            w.apply(ev);
        }
        let mut cam = Camera::default();
        settle(&mut cam, &mut w, &s);
        let idx = w
            .objects
            .iter()
            .find(|(_, o)| o.is_avatar() && o.parent_id == 0 && o.full_id != cam.own)
            .map(|(i, _)| i)
            .expect("the demo has standing residents");
        let avatar = pick_focus_object(&w, idx, Instant::now()).unwrap();
        cam.zoom_avatar(avatar, &mut w.agent, &s);
        settle(&mut cam, &mut w, &s);
        let g = focus_object_geom(&w, &avatar.0, Instant::now()).unwrap();
        let focus = g.position + Vec3::Z * AVATAR_ZOOM_FOCUS_Z;
        let eye = g.position + g.rotation * Vec3::new(AVATAR_ZOOM_DISTANCE, 0.0, AVATAR_ZOOM_EYE_Z);
        assert!(cam.target.distance(focus) < 0.01, "{} vs {focus}", cam.target);
        assert!(cam.position.distance(eye) < 0.05, "{} vs {eye}", cam.position);
        // the resident walks away: the camera keeps the same view of them
        let step = Vec3::new(4.0, -3.0, 0.0);
        w.objects.get_mut(idx).unwrap().position += step;
        settle(&mut cam, &mut w, &s);
        assert!(cam.target.distance(focus + step) < 0.01, "{} vs {}", cam.target, focus + step);
        assert!(cam.position.distance(eye + step) < 0.05, "{} vs {}", cam.position, eye + step);
    }

    #[test]
    fn manual_zoom_keeps_fov_constraints_with_edit_tracking_disabled() {
        let (mut w, mut cam, s, _) = focused_object_with(constrained());
        let mut f = frame(1.0 / 60.0);
        f.build_mode = true;
        f.edit_tracking = Some(false);
        cam.zoom_in(0.01);
        for _ in 0..60 {
            cam.update(&mut w, &s, &f);
        }
        assert!(cam.fov_zoom > 0.0);
        assert!(cam.fov_y < s.fov);
    }

    #[test]
    fn fov_correction_stops_at_the_legacy_minimum_view() {
        // two centimetres from a 1 m cube: the correction alone would narrow
        // the view to under 2°; LLViewerCamera::setView keeps 5°, and more in
        // a narrow window so that its width keeps 5° (#1380)
        for (aspect, min) in [(16.0 / 9.0, 5f32), (0.5, 10.0)] {
            let (mut w, mut cam, s, _) = focused_object_with(constrained());
            cam.zoom_in(0.0001);
            let f = FrameInput {
                aspect,
                ..frame(1.0 / 60.0)
            };
            for _ in 0..300 {
                cam.update(&mut w, &s, &f);
            }
            assert!(s.fov / (1.0 + cam.fov_zoom) < min.to_radians() * 0.5, "{}", cam.fov_zoom);
            assert!(
                (cam.fov_y - min.to_radians()).abs() < 1e-4,
                "aspect {aspect}: {}°",
                cam.fov_y.to_degrees()
            );
            assert!(cam.proj(aspect).is_finite());
        }
    }

    #[test]
    fn edit_edge_pan_survives_the_camera_update_without_doubling() {
        let (mut w, mut cam, s, _) = focused_object();
        let before = (cam.position, cam.target);
        let delta = Vec3::new(0.0, 0.1, 0.05);
        let mut f = frame(1.0 / 60.0);
        f.build_mode = true;
        f.edit_tracking = Some(false);
        assert!(cam.pan_edit(delta, &mut w.agent, &s));
        cam.update(&mut w, &s, &f);
        assert!(cam.position.abs_diff_eq(before.0 + delta, 1e-4));
        assert!(cam.target.abs_diff_eq(before.1 + delta, 1e-4));
    }

    #[test]
    fn held_edge_cursor_keeps_moving_the_object_and_release_stops_the_camera() {
        use crate::build::{BuildSettings, BuildTool, EditMode, Mods, Tool, geom::Cam, manip::Part};
        for offset in [Vec3::X * 10.0, Vec3::new(6.8354, 5.5374, -1.4747)] {
            let (mut w, mut camera, s, idx) = focused_object();
            let g = camera.focus_geom.unwrap();
            camera.set_camera_pos_and_focus(g.position - offset, g.position, Some((camera.focus_object.unwrap(), g)), &s);
            settle(&mut camera, &mut w, &s);
            let mut tool = BuildTool::default();
            let settings = BuildSettings {
                snap: false,
                ..Default::default()
            };
            tool.open_build(Tool::Edit);
            tool.click_select(&mut w, &settings, Some(idx), false);
            let projection = |camera: &Camera| {
                Cam::new(
                    camera.position,
                    camera.target,
                    camera.proj(1600.0 / 900.0) * camera.view(),
                    camera.fov_y,
                    1600.0,
                    900.0,
                    1.0,
                )
            };
            tool.cam = projection(&camera);
            let key = w.objects.get(idx).unwrap().key;
            let center = Scene::object_transform(&w, idx, Instant::now(), 0).unwrap().0;
            let start = tool
                .cam
                .project_px(center + Vec3::Y * tool.cam.meters_for_pixels(center, 45.0))
                .unwrap();
            assert!(
                tool.manip
                    .try_grab(&mut w, &settings, &tool.cam, &[key], false, EditMode::Move, start, Instant::now())
            );
            assert_eq!(tool.manip.drag.as_ref().unwrap().part, Part::Arrow(1));
            let mut f = frame(1.0 / 60.0);
            f.build_mode = true;
            f.edit_tracking = Some(false);
            let mut held_start = Vec3::ZERO;
            let mut held_offset = Vec3::ZERO;
            for i in 0..150 {
                let k = (i as f32 / 60.0).min(1.0);
                let cursor = (start.0 + (1590.0 - start.0) * k, start.1);
                let pan = tool.manip.camera_pan(&settings, &tool.cam, cursor, f.dt);
                assert!(pan.length() < 0.17);
                assert!(pan.cross(Vec3::Y).length() < 1e-6);
                camera.pan_edit(pan, &mut w.agent, &s);
                tool.cam = projection(&camera);
                tool.update(&mut w, &settings, cursor, false, Mods::default());
                camera.update(&mut w, &s, &f);
                tool.cam = projection(&camera);
                let position = Scene::object_transform(&w, idx, Instant::now(), 0).unwrap().0;
                if i == 60 {
                    held_start = position;
                    held_offset = position - camera.position;
                }
                if i > 60 {
                    assert!(
                        (position - camera.position).abs_diff_eq(held_offset, 1e-3),
                        "offset drift: {:?}",
                        position - camera.position - held_offset
                    );
                    let (x, y) = tool.cam.project_px(position).unwrap();
                    assert!((0.0..1600.0).contains(&x) && (0.0..900.0).contains(&y), "{x},{y}");
                }
            }
            let held_end = Scene::object_transform(&w, idx, Instant::now(), 0).unwrap().0;
            assert!(held_end.distance(held_start) > 2.0);
            tool.mouse_up(&mut w, &settings, None);
            assert_eq!(tool.manip.camera_pan(&settings, &tool.cam, (1590.0, start.1), f.dt), Vec3::ZERO);
            let stopped = camera.position;
            for _ in 0..30 {
                camera.update(&mut w, &s, &f);
            }
            assert!(camera.position.abs_diff_eq(stopped, 1e-4));
        }
    }
}
