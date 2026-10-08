//! Our own avatar: movement input, smoothing of server positions, and the
//! `AgentUpdate` control state sent to the simulator.

use aurora_net::AgentControls;
use aurora_net::control;
use glam::{Quat, Vec3};
use std::time::Instant;

#[derive(Debug, Clone, Copy, Default)]
pub struct MoveInput {
    pub forward: bool,
    pub back: bool,
    pub turn_left: bool,
    pub turn_right: bool,
    pub strafe_left: bool,
    pub strafe_right: bool,
    pub up: bool,
    pub down: bool,
    pub run: bool,
    /// First moments of a key press: small "nudge" steps (LLAgent::moveAtNudge).
    pub nudge_fb: bool,
    pub nudge_lr: bool,
    /// Seconds the turn key has been held (LLFloaterMove::getYawRate).
    pub turn_held: f32,
}

#[derive(Debug, Clone)]
pub struct AgentState {
    /// Render-space position (main-region local).
    pub position: Vec3,
    pub velocity: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub flying: bool,
    pub seated: bool,
    /// The drawn body is turning (+1 left, -1 right): TURN_LEFT / RIGHT.
    pub body_turn: i8,
    pub server_pos: Vec3,
    server_vel: Vec3,
    last_server: Instant,
    has_server: bool,
}

impl Default for AgentState {
    fn default() -> Self {
        AgentState {
            position: Vec3::new(128.0, 128.0, 25.0),
            velocity: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            flying: false,
            seated: false,
            body_turn: 0,
            server_pos: Vec3::new(128.0, 128.0, 25.0),
            server_vel: Vec3::ZERO,
            last_server: Instant::now(),
            has_server: false,
        }
    }
}

/// Keyboard turn: 90°/s (LLAgent::propagate), ramping up from 5 % over the
/// first 0.25 s of the press (LLFloaterMove::getYawRate).
const TURN_RATE: f32 = std::f32::consts::FRAC_PI_2;

fn yaw_rate(held: f32) -> f32 {
    const NUDGE_TIME: f32 = 0.25;
    const YAW_NUDGE_RATE: f32 = 0.05;
    if held < NUDGE_TIME {
        YAW_NUDGE_RATE + held * (1.0 - YAW_NUDGE_RATE) / NUDGE_TIME
    } else {
        1.0
    }
}

impl AgentState {
    pub fn has_local_control(&self) -> bool {
        self.has_server && !self.seated
    }

    pub fn body_rotation(&self) -> Quat {
        Quat::from_rotation_z(self.yaw)
    }

    pub fn forward(&self) -> Vec3 {
        Vec3::new(self.yaw.cos(), self.yaw.sin(), 0.0)
    }

    pub fn set_look_at(&mut self, look: Vec3) {
        if look.truncate().length_squared() > 1e-6 {
            self.yaw = look.y.atan2(look.x);
        }
    }

    /// Seconds since the simulator last reported our position.
    pub fn server_age(&self) -> f32 {
        self.last_server.elapsed().as_secs_f32()
    }

    pub fn reset_motion(&mut self) {
        self.server_pos = self.position;
        self.server_vel = Vec3::ZERO;
        self.velocity = Vec3::ZERO;
        self.last_server = Instant::now();
    }

    /// Position report from the simulator (only meaningful when not seated).
    pub fn on_server_update(&mut self, pos: Vec3, vel: Vec3, seated: bool, in_main_region: bool) {
        self.seated = seated;
        if seated || !in_main_region {
            return;
        }
        if !self.has_server || pos.distance(self.position) > 8.0 {
            self.position = pos; // teleport / first fix: snap
        }
        // diagnostic: the simulator moved us up or down by a step
        if self.has_server && (pos.z - self.server_pos.z).abs() > 0.5 {
            log::info!(
                "agent z {:.2} -> {:.2} (vel {:.2}, flying {}, seated {seated})",
                self.server_pos.z,
                pos.z,
                vel.z,
                self.flying
            );
        }
        self.server_pos = pos;
        self.server_vel = vel;
        self.last_server = Instant::now();
        self.has_server = true;
    }

    /// Offline demo only: stand in for the simulator's avatar physics so the
    /// controls can be tried without a grid (walk 3.2 m/s, run 5.1 m/s, fly,
    /// jump, terrain following). On a grid the simulator does this.
    pub fn simulate_locally(&mut self, input: &MoveInput, mouselook: bool, ground: Option<f32>, dt: f32) {
        // object center above the floor: half the default body height (SL
        // places the pelvis 0.13 m above it, see Scene::skeleton_of)
        const STAND: f32 = 0.84;
        let dt = dt.min(0.1);
        let fwd = self.forward();
        let left = Vec3::new(-fwd.y, fwd.x, 0.0);
        let mut dir = Vec3::ZERO;
        if input.forward {
            dir += fwd;
        }
        if input.back {
            dir -= fwd;
        }
        if input.strafe_left || (mouselook && input.turn_left) {
            dir += left;
        }
        if input.strafe_right || (mouselook && input.turn_right) {
            dir -= left;
        }
        let speed = if self.flying {
            10.0
        } else if input.down {
            1.2 // crouch-walk
        } else if input.run {
            5.13
        } else {
            3.2
        };
        let nudge = if input.nudge_fb || input.nudge_lr { 0.4 } else { 1.0 };
        let horiz = dir.normalize_or_zero() * speed * nudge;
        let floor = ground.map(|g| g + STAND);
        let mut pos = self.server_pos;
        let mut vz = self.server_vel.z;
        let grounded = floor.is_some_and(|f| pos.z <= f + 0.05);
        if self.flying {
            vz = if input.up {
                4.0
            } else if input.down {
                -4.0
            } else {
                0.0
            };
        } else if grounded {
            vz = if input.up { 5.0 } else { 0.0 };
        } else {
            vz -= 9.8 * dt;
        }
        pos += Vec3::new(horiz.x, horiz.y, vz) * dt;
        if let Some(f) = floor
            && pos.z < f
        {
            pos.z = f;
            vz = vz.max(0.0);
            if self.flying && input.down {
                self.flying = false; // landing
            }
        }
        self.server_pos = pos;
        self.server_vel = Vec3::new(horiz.x, horiz.y, vz);
        self.last_server = Instant::now();
        self.has_server = true;
    }

    /// Advance smoothing and turning.
    pub fn update(&mut self, input: &MoveInput, mouselook: bool, dt: f32) {
        if !mouselook && input.turn_left != input.turn_right {
            let step = TURN_RATE * yaw_rate(input.turn_held) * dt;
            self.yaw += if input.turn_left { step } else { -step };
        }
        self.yaw = (self.yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
        // Dead-reckon from the last server state, then smooth toward it.
        let t = self.last_server.elapsed().as_secs_f32().min(0.5);
        let target = self.server_pos + self.server_vel * t;
        let k = 1.0 - (-14.0 * dt).exp();
        self.position += (target - self.position) * k;
        self.velocity = self.server_vel;
    }

    /// Control flags for this frame.
    pub fn control_flags(&self, input: &MoveInput, mouselook: bool) -> u32 {
        let mut f = 0;
        // a held key always sends FAST_* (LLAgent::moveAt / moveLeft /
        // moveUp); running is SetAlwaysRun
        if input.forward {
            f |= if input.nudge_fb {
                control::NUDGE_AT_POS
            } else {
                control::AT_POS | control::FAST_AT
            };
        }
        if input.back {
            f |= if input.nudge_fb {
                control::NUDGE_AT_NEG
            } else {
                control::AT_NEG | control::FAST_AT
            };
        }
        let strafe_l = input.strafe_left || (mouselook && input.turn_left);
        let strafe_r = input.strafe_right || (mouselook && input.turn_right);
        if strafe_l {
            f |= if input.nudge_lr {
                control::NUDGE_LEFT_POS
            } else {
                control::LEFT_POS | control::FAST_LEFT
            };
        }
        if strafe_r {
            f |= if input.nudge_lr {
                control::NUDGE_LEFT_NEG
            } else {
                control::LEFT_NEG | control::FAST_LEFT
            };
        }
        if !mouselook {
            if input.turn_left {
                f |= control::YAW_POS;
            }
            if input.turn_right {
                f |= control::YAW_NEG;
            }
        }
        // the turn animations follow the drawn body, not the keys
        // (LLVOAvatar::updateOrientation)
        match self.body_turn {
            1 => f |= control::TURN_LEFT,
            -1 => f |= control::TURN_RIGHT,
            _ => {}
        }
        if input.up {
            f |= control::UP_POS | control::FAST_UP;
        }
        if input.down {
            f |= control::UP_NEG | control::FAST_UP;
        }
        if self.flying {
            f |= control::FLY;
        }
        if mouselook {
            f |= control::MOUSELOOK;
        }
        f
    }

    pub fn controls(&self, input: &MoveInput, cam: &crate::camera::Camera, far: f32) -> AgentControls {
        let at = cam.forward();
        let up = Vec3::Z;
        let left = up.cross(at).normalize_or(Vec3::Y);
        let cam_up = at.cross(left).normalize_or(Vec3::Z);
        AgentControls {
            control_flags: self.control_flags(input, cam.mouselook()),
            body_rotation: self.body_rotation(),
            head_rotation: Quat::from_rotation_y(-self.pitch * 0.5),
            camera_center: cam.position,
            camera_at: at,
            camera_left: left,
            camera_up: cam_up,
            far,
            state: 0,
            flags: 0,
        }
    }
}
