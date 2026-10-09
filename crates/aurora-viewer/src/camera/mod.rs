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
mod settings;

pub use focus::FocusObject;
pub use settings::{CameraPreset, CameraSettings, DEFAULT_FOV, FOV_RANGE};

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
/// Aurora's right-drag orbit rate (radians per pixel at sensitivity 1).
const DRAG_RATE: f32 = 0.0042;

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
    pub draw_distance: f32,
}

/// Our avatar this frame (render space).
#[derive(Debug, Clone, Copy)]
struct AvatarFrame {
    /// getAvatarRootPosition: the root joint, hover height removed unless
    /// HoverHeightAffectsCamera.
    root: Vec3,
    /// gAgent.getPositionGlobal: the avatar object's position.
    position: Vec3,
    /// Rotation of the object we sit on.
    seat: Option<Quat>,
}

#[derive(Debug, Clone, Copy)]
struct Animation {
    start_camera: Vec3,
    start_focus: Vec3,
    elapsed: f32,
    duration: f32,
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
    force_mouselook: bool,
    seat: Option<Seat>,
    fov_zoom: f32,
    current_fov_zoom: f32,
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
            force_mouselook: false,
            seat: None,
            fov_zoom: 0.0,
            current_fov_zoom: 0.0,
            virtual_position: Vec3::ZERO,
            anim: None,
            smoothing_last_global: Vec3::ZERO,
            smoothing_last_agent: Vec3::ZERO,
            smoothing_stop: false,
            tool: None,
            avatar: AvatarFrame {
                root: start,
                position: start,
                seat: None,
            },
            head: start + Vec3::Z * 0.7,
            draw_distance: 128.0,
            unconstrained: false,
            own: Uuid::nil(),
            dt: 0.0,
            initialized: false,
        }
    }
}

impl Camera {
    pub fn mouselook(&self) -> bool {
        self.mode == CameraMode::Mouselook
    }

    pub fn forward(&self) -> Vec3 {
        (self.target - self.position).normalize_or(Vec3::X)
    }

    /// The point the camera is locked on (alt-camera), if not the avatar.
    pub fn focus_point(&self) -> Option<Vec3> {
        (!self.focus_on_avatar).then_some(self.focus_target)
    }

    fn third_person(&self) -> bool {
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

    // ------------------------------------------------------------ transitions

    /// LLAgentCamera::startCameraAnimation.
    fn start_animation(&mut self, s: &CameraSettings) {
        self.anim = Some(Animation {
            start_camera: self.position,
            start_focus: self.target,
            elapsed: 0.0,
            duration: 0.0,
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

    /// LLAgentCamera::changeCameraToDefault (no follow cam yet).
    pub fn change_to_default(&mut self, agent: &mut AgentState, s: &CameraSettings) {
        self.change_to_third_person(agent, s, true);
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

    /// Context menu « Zoomer » (handle_zoom_to_object).
    pub fn zoom_to(&mut self, point: Vec3, agent: &mut AgentState, s: &CameraSettings) {
        self.set_focus_on_avatar(false, true, true, agent, s);
        let dir = (self.position - point).normalize_or(-self.forward());
        self.set_camera_pos_and_focus(point + dir * 3.0, point, None, s);
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
        if change_camera {
            self.change_to_default(agent, s);
        }
        if reset_camera {
            if !steering && self.third_person() {
                // leaving mouse steering: the pitch fades out
                self.pitch *= 1.0 - interpolant(0.3, self.dt);
                if self.orbit_yaw != 0.0 {
                    agent.yaw += self.orbit_yaw;
                    self.orbit_yaw = 0.0;
                }
            }
            self.set_focus_on_avatar(true, true, true, agent, s);
            self.fov_zoom = 0.0;
        }
    }

    /// handle_reset_view (Escape, « Réinitialiser la caméra »).
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

    /// Back to the default view without animation (login, teleport).
    pub fn snap(&mut self) {
        self.initialized = false;
        self.anim = None;
    }

    /// Arrival of a teleport (process_agent_movement_complete): the camera
    /// back on the avatar without animation; FSResetCameraOnTP also drops
    /// the orbit, the pitch and the zoom (resetView of process_teleport_local).
    pub fn on_teleport_arrival(&mut self, s: &CameraSettings) {
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
        if self.focus_on_avatar && self.third_person() {
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

    // ------------------------------------------------------------------ frame

    /// Once per frame, after the avatar moved (LLAgentCamera::updateCamera).
    pub fn update(&mut self, world: &mut World, s: &CameraSettings, f: &FrameInput) {
        let dt = f.dt.max(0.0);
        self.dt = dt;
        self.draw_distance = f.draw_distance;
        self.unconstrained = s.disable_constraints;
        self.own = world.agent_id;
        self.avatar = avatar_frame(world, s, f.now);
        self.sync_seat(&mut world.agent, s);
        let world = &*world;
        let agent = &world.agent;
        self.head = self.avatar.position + Vec3::Z * HEAD_HEIGHT;
        self.refresh_focus_object(world, s, f.now);
        // CAMERA_FOCUS_HALF_LIFE is 0: the offset follows at once
        self.focus_offset = self.focus_offset_target;
        let in_air = agent.flying || agent.velocity.z.abs() > 1.0;
        self.time_in_air = if in_air { self.time_in_air + dt } else { 0.0 };

        let mut camera_target = self.calc_camera_target(world, s, f);
        self.virtual_position = camera_target;
        let focus_target = self.calc_focus_target(world, s, f.now);
        // field of view correction: the camera backs off, the view narrows
        self.fov_zoom = self.calc_fov_zoom();
        camera_target = focus_target + (camera_target - focus_target) * (1.0 + self.fov_zoom);

        if !self.initialized {
            self.initialized = true;
            self.anim = None;
            self.lag = Vec3::ZERO;
            self.current_distance = self.target_distance;
            self.current_fov_zoom = self.fov_zoom;
            self.smoothing_last_global = camera_target;
            self.smoothing_last_agent = camera_target - self.avatar.position;
            self.position = camera_target;
            self.target = focus_target;
            self.fov_y = s.fov / (1.0 + self.current_fov_zoom);
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
                    (a.start_camera.lerp(camera_target, k), a.start_focus.lerp(focus_target, k))
                } else {
                    self.anim = None;
                    (camera_target, focus_target)
                }
            }
            None => (camera_target, focus_target),
        };

        // smoothing: relative to the avatar when following it (the avatar
        // moves too jerkily in world space to smooth there)
        let agent_pos = self.avatar.position;
        let mut relative = camera - agent_pos;
        self.smoothing_stop |= f.build_mode;
        if self.third_person() && !self.smoothing_stop {
            let k = interpolant(s.smoothing * SMOOTHING_HALF_LIFE, dt);
            if self.focus_on_avatar && self.focus_object.is_none() {
                if (relative - self.smoothing_last_agent).length() < MAX_CAMERA_SMOOTH_DISTANCE {
                    relative = self.smoothing_last_agent.lerp(relative, k);
                    camera = relative + agent_pos;
                }
            } else if (camera - self.smoothing_last_global).length() < MAX_CAMERA_SMOOTH_DISTANCE {
                camera = self.smoothing_last_global.lerp(camera, k);
            }
        }
        self.smoothing_last_global = camera;
        self.smoothing_last_agent = relative;
        self.smoothing_stop = false;

        self.current_fov_zoom += (self.fov_zoom - self.current_fov_zoom) * interpolant(FOV_ZOOM_HALF_LIFE, dt);
        self.position = camera;
        self.target = focus;
        self.fov_y = s.fov / (1.0 + self.current_fov_zoom);
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
            }
            (None, None) => {}
        }
    }

    /// LLAgentCamera::validateFocusObject and the object tracking of
    /// calcFocusPositionTargetGlobal.
    fn refresh_focus_object(&mut self, world: &World, s: &CameraSettings, now: Instant) {
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
                    if s.track_focus_object {
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

    fn mouselook_eye(&self, agent: &AgentState) -> Vec3 {
        let up = if self.avatar.seat.is_some() {
            HEAD_HEIGHT + 0.1
        } else {
            HEAD_HEIGHT
        };
        self.avatar.position + Vec3::Z * up + agent.forward() * 0.12
    }

    /// LLAgentCamera::calcCameraPositionTargetGlobal.
    fn calc_camera_target(&mut self, world: &World, s: &CameraSettings, f: &FrameInput) -> Vec3 {
        let agent = &world.agent;
        let mut camera = match self.mode {
            CameraMode::Mouselook => self.mouselook_eye(agent),
            CameraMode::ThirdPerson if self.focus_on_avatar => match self.sit_camera_point(world, f.now, true) {
                Some(p) => p,
                None => self.third_person_camera(agent, s, f),
            },
            CameraMode::ThirdPerson => self.focus_target + self.focus_offset,
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
            self.lag = self.lag.lerp(target_lag, interpolant(CAMERA_LAG_HALF_LIFE, dt) * u);
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
            self.lag = self.lag.lerp(Vec3::ZERO, interpolant(0.15, dt));
        }
        camera - self.lag
    }

    /// LLAgentCamera::calcFocusPositionTargetGlobal.
    fn calc_focus_target(&self, world: &World, s: &CameraSettings, now: Instant) -> Vec3 {
        let agent = &world.agent;
        match self.mode {
            CameraMode::Mouselook => {
                let (y, p) = (agent.yaw, agent.pitch);
                self.mouselook_eye(agent) + Vec3::new(y.cos() * p.cos(), y.sin() * p.cos(), p.sin())
            }
            CameraMode::ThirdPerson if !self.focus_on_avatar => self.focus_target,
            CameraMode::ThirdPerson => match self.sit_camera_point(world, now, false) {
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
        glam::camera::rh::view::look_at_mat4(self.position, self.target, Vec3::Z)
    }

    pub fn proj(&self, aspect: f32) -> Mat4 {
        glam::camera::rh::proj::directx::perspective_infinite_reverse(self.fov_y, aspect.max(0.1), self.near)
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
        seat: None,
    };
    let id = world.agent_id;
    let Some(idx) = world.objects.index_of_uuid(&id) else {
        return fallback;
    };
    let Some((p, _, _)) = Scene::object_transform(world, idx, now, 0) else {
        return fallback;
    };
    let seat = world
        .objects
        .get(idx)
        .filter(|o| o.parent_id != 0)
        .and_then(|o| world.objects.parent_of(o))
        .and_then(|parent| Scene::object_transform(world, parent, now, 0))
        .map(|(_, r, _)| r);
    // the scene's root offset includes the hover height (standing only)
    let dz = world.avatar_root_dz.get(&id).copied().unwrap_or(0.0);
    let hover = if seat.is_none() && !s.hover_affects_camera {
        world.appearance_hover(&id)
    } else {
        0.0
    };
    AvatarFrame {
        root: p - Vec3::Z * hover,
        position: p - Vec3::Z * dz,
        seat,
    }
}

fn focus_object_geom(world: &World, id: &Uuid, now: Instant) -> Option<FocusObject> {
    let idx = world.objects.index_of_uuid(id)?;
    let o = world.objects.get(idx)?;
    let (p, rotation, hud) = Scene::object_transform(world, idx, now, 0)?;
    if hud {
        return None;
    }
    let position = if o.is_avatar() {
        p - Vec3::Z * world.avatar_root_dz.get(id).copied().unwrap_or(0.0)
    } else {
        p
    };
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
    use crate::scene::avatar::AvatarLibrary;

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
            draw_distance: 128.0,
        }
    }

    fn settle(cam: &mut Camera, w: &mut World, s: &CameraSettings) {
        for _ in 0..300 {
            cam.update(w, s, &frame(1.0 / 60.0));
        }
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

    #[test]
    fn collide_plane_keeps_the_camera_in_front_of_a_wall() {
        let s = CameraSettings::default();
        let mut w = world_at(Vec3::new(128.0, 128.0, 25.0));
        let mut cam = Camera::default();
        // a wall at x = 126.5 facing +x, the avatar on its positive side
        cam.set_collide_plane(Vec4::new(1.0, 0.0, 0.0, 126.5), &s);
        settle(&mut cam, &mut w, &s);
        assert!(cam.position.x > 126.5, "{:?}", cam.position);
        let unconstrained = CameraSettings {
            disable_constraints: true,
            ..CameraSettings::default()
        };
        settle(&mut cam, &mut w, &unconstrained);
        assert!(cam.position.x < 126.0, "{:?}", cam.position);
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
}
