//! The « Déplacer » tool: grab an object and drag it, lift it (Ctrl) or
//! spin it (Ctrl+Shift). The simulator moves it from our ObjectGrabUpdate /
//! ObjectSpinUpdate messages, physical or not.
//!
//! Port of LLToolGrab (indra/newview/lltoolgrab.cpp, originally LGPL 2.1):
//! startGrab, handleHoverActive / handleHoverNonPhysical, startSpin,
//! stopGrab. Not ported: the camera orbiting when the cursor reaches the
//! edge of the screen, and the hidden, re-centered cursor.

use super::geom::Cam;
use super::{BuildTool, GrabMode, Mods};
use crate::scene::Scene;
use crate::world::World;
use aurora_net::build::BuildCmd;
use aurora_net::{NetCommand, TouchSurface};
use glam::{Quat, Vec3};
use std::time::Instant;
use uuid::Uuid;

/// GRAB_SENSITIVITY_X / Y: meters per pixel, whatever the distance.
const GRAB_SENSITIVITY: f32 = 0.0075;
/// RADIANS_PER_PIXEL_X / Y of the spin.
const SPIN_RADIANS_PER_PIXEL: f32 = 0.01;
/// SLOP_DIST_SQ: pixels² before a press becomes a drag.
const SLOP_DIST_SQ: f32 = 4.0;
/// update_flags bits (llprimitive.h).
const FLAGS_USE_PHYSICS: u32 = 1 << 0;
const FLAGS_HANDLE_TOUCH: u32 = 1 << 7;
const FLAGS_OBJECT_MOVE: u32 = 1 << 8;
const FLAGS_CHARACTER: u32 = 1 << 13;
/// LLWorld::getRegionMaxHeight.
const MAX_HEIGHT: f32 = 4096.0;

/// LLToolGrab::EGrabMode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrabKind {
    /// Moved by the simulator from the cursor (ACTIVE_CENTER).
    Active,
    /// Touch drag of a scripted, non-physical object (NONPHYSICAL).
    NonPhysical,
    /// Cannot be moved (LOCKED): only the touch.
    Locked,
}

#[derive(Debug, Clone)]
pub struct GrabState {
    pub kind: GrabKind,
    object_id: Uuid,
    local_id: u32,
    region: aurora_net::RegionHandle,
    /// Root position relative to the camera, moved by the mouse
    /// (mGrabHiddenOffsetFromCamera).
    offset_from_camera: Vec3,
    /// GrabOffsetInitial of ObjectGrabUpdate.
    grab_offset: Vec3,
    /// Where the press was, and the last cursor seen (pixels).
    down: (f32, f32),
    last: (f32, f32),
    dragging: bool,
    vertical: bool,
    spinning: bool,
    spin_rotation: Quat,
    last_send: Instant,
    surface: TouchSurface,
}

#[derive(Debug, Default)]
pub struct Grab {
    pub state: Option<GrabState>,
}

impl BuildTool {
    /// Press with the grab tool on object `idx` at `point` (LLToolGrab::
    /// handleObjectHit + startGrab).
    pub fn grab_down(&mut self, world: &mut World, idx: usize, point: Vec3, surface: TouchSurface, cursor: (f32, f32), mods: Mods) {
        let Some(o) = world.objects.get(idx) else { return };
        if o.is_avatar() {
            return;
        }
        if !mods.shift {
            self.deselect_all(world);
        }
        let root = super::root_of(world, idx);
        let now = Instant::now();
        let (Some(root_obj), Some((root_pos, root_rot, _))) = (world.objects.get(root), Scene::object_transform(world, root, now, 0))
        else {
            return;
        };
        let flags = root_obj.update_flags | o.update_flags;
        let movable = flags & FLAGS_OBJECT_MOVE != 0 && flags & FLAGS_CHARACTER == 0;
        let physical = root_obj.update_flags & FLAGS_USE_PHYSICS != 0;
        let kind = if !physical && flags & FLAGS_HANDLE_TOUCH != 0 {
            GrabKind::NonPhysical
        } else if movable {
            GrabKind::Active
        } else {
            GrabKind::Locked
        };
        let grab_offset = root_rot.inverse() * (root_pos - point);
        let state = GrabState {
            kind,
            object_id: root_obj.full_id,
            local_id: o.key.local_id,
            region: o.key.region,
            offset_from_camera: root_pos - self.cam.eye,
            grab_offset,
            down: cursor,
            last: cursor,
            dragging: false,
            vertical: mods.ctrl && !mods.shift || self.grab_mode == GrabMode::Lift,
            spinning: false,
            spin_rotation: root_rot,
            last_send: now,
            surface,
        };
        // the touch / grab messages of the click actions (session/object_actions.rs)
        self.out.push(NetCommand::ObjectGrab {
            handle: o.key.region,
            local_id: o.key.local_id,
            offset: grab_offset,
            surface,
        });
        if mods.ctrl && mods.shift || self.grab_mode == GrabMode::Spin {
            self.start_spin_of(&state);
            self.grab.state = Some(GrabState { spinning: true, ..state });
        } else {
            self.grab.state = Some(state);
        }
    }

    fn start_spin_of(&mut self, g: &GrabState) {
        self.send(BuildCmd::SpinStart {
            handle: g.region,
            object_id: g.object_id,
        });
    }

    /// Cursor moved while grabbing (handleHoverActive / NonPhysical).
    pub fn grab_hover(&mut self, world: &World, cursor: (f32, f32), mods: Mods) {
        let cam: Cam = self.cam;
        let Some(mut g) = self.grab.state.take() else { return };
        let (dx, dy) = (cursor.0 - g.last.0, cursor.1 - g.last.1);
        g.last = cursor;
        if !g.dragging {
            let (sx, sy) = (cursor.0 - g.down.0, cursor.1 - g.down.1);
            g.dragging = sx * sx + sy * sy > SLOP_DIST_SQ;
        }
        if !g.dragging || g.kind == GrabKind::Locked {
            self.grab.state = Some(g);
            return;
        }
        let now = Instant::now();
        // the modifiers switch between spin, lift and move while dragging
        let want_spin = mods.ctrl && mods.shift || self.grab_mode == GrabMode::Spin;
        if want_spin != g.spinning {
            if want_spin {
                self.start_spin_of(&g);
                g.spin_rotation = world
                    .objects
                    .index_of_uuid(&g.object_id)
                    .and_then(|i| Scene::object_transform(world, i, now, 0))
                    .map(|(_, r, _)| r)
                    .unwrap_or(g.spin_rotation);
            } else {
                self.send(BuildCmd::SpinStop {
                    handle: g.region,
                    object_id: g.object_id,
                });
            }
            g.spinning = want_spin;
        }
        if g.spinning {
            // LLToolGrab::handleHoverActive, spin branch
            let yaw = Quat::from_axis_angle(Vec3::Z, dx * SPIN_RADIANS_PER_PIXEL);
            let pitch = Quat::from_axis_angle(cam.left, dy * SPIN_RADIANS_PER_PIXEL);
            g.spin_rotation = (pitch * (yaw * g.spin_rotation)).normalize();
            self.send(BuildCmd::SpinUpdate {
                handle: g.region,
                object_id: g.object_id,
                rotation: g.spin_rotation,
            });
            self.grab.state = Some(g);
            return;
        }
        g.vertical = mods.ctrl && !mods.shift || self.grab_mode == GrabMode::Lift;
        let x_part = Vec3::new(cam.left.x, cam.left.y, 0.0).try_normalize().unwrap_or(Vec3::Y);
        let y_part = if g.vertical {
            cam.up
        } else {
            // away from the camera (x_part % z_axis): screen up pushes it
            x_part.cross(Vec3::Z).try_normalize().unwrap_or(Vec3::X)
        };
        // screen y grows downwards here, upwards in LL
        g.offset_from_camera += x_part * (-dx * GRAB_SENSITIVITY) + y_part * (-dy * GRAB_SENSITIVITY);
        let mut point = cam.eye + g.offset_from_camera;
        if let Some(h) = world.ground_height(point) {
            point.z = point.z.max(h);
        }
        point.z = point.z.min(MAX_HEIGHT);
        g.offset_from_camera = point - cam.eye;
        let Some(off) = world.region_offset(g.region) else {
            self.grab.state = Some(g);
            return;
        };
        let ms = now.duration_since(g.last_send).as_millis().min(u32::MAX as u128) as u32;
        g.last_send = now;
        self.out.push(NetCommand::ObjectGrabUpdate {
            handle: g.region,
            object: g.object_id,
            offset: g.grab_offset,
            position: point - off,
            elapsed_ms: ms,
            surface: g.surface,
        });
        self.grab.state = Some(g);
    }

    /// Release (LLToolGrab::stopGrab): ObjectDeGrab, SpinStop if spinning.
    pub fn grab_up(&mut self) {
        let Some(g) = self.grab.state.take() else { return };
        if g.spinning {
            self.send(BuildCmd::SpinStop {
                handle: g.region,
                object_id: g.object_id,
            });
        }
        self.out.push(NetCommand::ObjectRelease {
            handle: g.region,
            local_id: g.local_id,
            surface: g.surface,
        });
    }

    /// Grab tool radio as shown: the held modifiers win (updatePopup).
    pub fn effective_grab_mode(&self, m: Mods) -> GrabMode {
        match (m.ctrl, m.shift) {
            (true, true) => GrabMode::Spin,
            (true, false) => GrabMode::Lift,
            _ => self.grab_mode,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_mode_follows_modifiers() {
        let t = BuildTool::default();
        assert_eq!(t.effective_grab_mode(Mods::default()), GrabMode::Move);
        let ctrl = Mods {
            ctrl: true,
            ..Default::default()
        };
        assert_eq!(t.effective_grab_mode(ctrl), GrabMode::Lift);
        let both = Mods {
            ctrl: true,
            shift: true,
            ..Default::default()
        };
        assert_eq!(t.effective_grab_mode(both), GrabMode::Spin);
    }
}
