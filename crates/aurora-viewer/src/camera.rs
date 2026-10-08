//! Third-person follow camera with orbit/zoom, and mouselook.

use crate::agent::AgentState;
use glam::{Mat4, Vec3};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraMode {
    Follow,
    Mouselook,
}

pub struct Camera {
    pub mode: CameraMode,
    pub position: Vec3,
    pub target: Vec3,
    /// Orbit offsets relative to the avatar's facing.
    pub orbit_yaw: f32,
    pub orbit_pitch: f32,
    pub distance: f32,
    pub fov_y: f32,
    pub near: f32,
    initialized: bool,
    /// Ctrl+Alt+click focus point (SL "alt-zoom"): the camera orbits it
    /// instead of following the avatar.
    pub focus: Option<Vec3>,
    focus_yaw: f32,
    focus_pitch: f32,
    focus_distance: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Camera {
            mode: CameraMode::Follow,
            position: Vec3::new(120.0, 128.0, 30.0),
            target: Vec3::new(128.0, 128.0, 28.0),
            orbit_yaw: 0.0,
            orbit_pitch: 0.28,
            distance: 3.6,
            fov_y: 60f32.to_radians(),
            near: 0.1,
            initialized: false,
            focus: None,
            focus_yaw: 0.0,
            focus_pitch: 0.0,
            focus_distance: 5.0,
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

    pub fn toggle_mouselook(&mut self, agent: &mut AgentState) {
        self.mode = match self.mode {
            CameraMode::Follow => {
                agent.yaw += self.orbit_yaw;
                self.orbit_yaw = 0.0;
                agent.pitch = 0.0;
                CameraMode::Mouselook
            }
            CameraMode::Mouselook => CameraMode::Follow,
        };
    }

    /// Aim the camera at a point and keep orbiting it (Ctrl+Alt+click).
    pub fn set_focus(&mut self, point: Vec3) {
        let rel = self.position - point;
        let dist = rel.length().max(0.5);
        self.focus = Some(point);
        self.focus_distance = dist;
        self.focus_yaw = rel.y.atan2(rel.x);
        self.focus_pitch = (rel.z / dist).clamp(-1.0, 1.0).asin().clamp(-1.4, 1.5);
    }

    /// Context menu "Zoomer": aim at the point and come closer.
    pub fn zoom_to(&mut self, point: Vec3, distance: f32) {
        self.set_focus(point);
        self.focus_distance = self.focus_distance.min(distance.max(0.5));
    }

    /// Back to following the avatar (Escape, clicking the avatar, moving).
    pub fn clear_focus(&mut self) {
        self.focus = None;
    }

    /// Left-drag on the avatar: turn the avatar (and the camera behind it)
    /// horizontally, tilt the camera vertically.
    pub fn steer(&mut self, dx: f32, dy: f32, sensitivity: f32, agent: &mut AgentState) {
        let s = 0.0042 * sensitivity;
        agent.yaw -= dx * s;
        self.orbit_pitch = (self.orbit_pitch + dy * s).clamp(-1.2, 1.45);
    }

    /// Mouse drag (follow: orbit) or motion (mouselook: look).
    pub fn on_mouse_delta(&mut self, dx: f32, dy: f32, sensitivity: f32, agent: &mut AgentState) {
        let s = 0.0042 * sensitivity;
        match self.mode {
            CameraMode::Follow if self.focus.is_some() => {
                self.focus_yaw -= dx * s;
                self.focus_pitch = (self.focus_pitch + dy * s).clamp(-1.4, 1.5);
            }
            CameraMode::Follow => {
                self.orbit_yaw -= dx * s;
                self.orbit_pitch = (self.orbit_pitch + dy * s).clamp(-1.2, 1.45);
            }
            CameraMode::Mouselook => {
                agent.yaw -= dx * s;
                agent.pitch = (agent.pitch - dy * s).clamp(-1.5, 1.5);
            }
        }
    }

    pub fn zoom(&mut self, steps: f32) {
        if self.mode == CameraMode::Follow {
            let k = 1.0 - steps * 0.12;
            if self.focus.is_some() {
                self.focus_distance = (self.focus_distance * k).clamp(0.3, 250.0);
            } else {
                self.distance = (self.distance * k).clamp(0.6, 80.0);
            }
        }
    }

    pub fn reset_orbit(&mut self) {
        self.orbit_yaw = 0.0;
        self.orbit_pitch = 0.28;
    }

    pub fn update(&mut self, agent: &AgentState, ground: Option<f32>, dt: f32) {
        let head = agent.position + Vec3::new(0.0, 0.0, 0.55);
        match self.mode {
            CameraMode::Mouselook => {
                let dir = Vec3::new(
                    agent.yaw.cos() * agent.pitch.cos(),
                    agent.yaw.sin() * agent.pitch.cos(),
                    agent.pitch.sin(),
                );
                self.position = head + Vec3::new(0.0, 0.0, 0.15) + agent.forward() * 0.12;
                self.target = self.position + dir;
                self.initialized = true;
            }
            CameraMode::Follow => {
                let (target, mut desired) = match self.focus {
                    Some(f) => {
                        let (y, p) = (self.focus_yaw, self.focus_pitch);
                        let dir = Vec3::new(y.cos() * p.cos(), y.sin() * p.cos(), p.sin());
                        (f, f + dir * self.focus_distance)
                    }
                    None => {
                        let yaw = agent.yaw + self.orbit_yaw;
                        let pitch = self.orbit_pitch;
                        let back = Vec3::new(yaw.cos() * pitch.cos(), yaw.sin() * pitch.cos(), -pitch.sin());
                        let target = head + Vec3::new(0.0, 0.0, 0.15);
                        (target, target - back * self.distance)
                    }
                };
                if let Some(g) = ground {
                    desired.z = desired.z.max(g + 0.35);
                }
                if !self.initialized {
                    self.position = desired;
                    self.target = target;
                    self.initialized = true;
                } else {
                    let k = 1.0 - (-12.0 * dt).exp();
                    self.position += (desired - self.position) * k;
                    self.target += (target - self.target) * (1.0 - (-20.0 * dt).exp());
                }
            }
        }
    }

    pub fn snap(&mut self) {
        self.initialized = false;
    }

    pub fn view(&self) -> Mat4 {
        glam::camera::rh::view::look_at_mat4(self.position, self.target, Vec3::Z)
    }

    pub fn proj(&self, aspect: f32) -> Mat4 {
        glam::camera::rh::proj::directx::perspective_infinite_reverse(self.fov_y, aspect.max(0.1), self.near)
    }
}
