//! A keyframe's camera moved by hand in the world (`ui::director::world`):
//! grabbed and dragged, it slides on the level it stands at; with Ctrl it
//! goes up and down; with Ctrl + Shift it turns on itself. These are the
//! build tools' grab keys (LLToolGrab::handleHoverActive,
//! indra/newview/lltoolgrab.cpp: Ctrl lifts, Ctrl + Shift spins), so a
//! keyframe moves as an object grabbed in world does. The path and the
//! Bézier handles follow while it moves; the whole drag is one step to undo
//! (`director::History` waits for the mouse to be released); Escape puts it
//! back where it was. The press is the interface's, never a walk
//! (`widgets::solid_background`), and the app moves nothing while RLVa
//! restricts the camera (`App::director_move_key`).
//!
//! Aurora's own: the Legacy Viewer has no camera path tool.

use super::director::Action;
use super::hud::Projector;
use crate::camera::track::Pose;
use crate::director::Director;
use crate::theme::Palette;
use egui::{Id, Pos2, Stroke};
use glam::{Quat, Vec3, Vec4};

/// Radians a keyframe's camera turns for each point the pointer goes
/// sideways (Ctrl + Shift).
const SPIN: f32 = 0.01;
/// A move further than this in one drag is a pointer lost near the horizon.
const FARTHEST: f32 = 500.0;

/// How the drag moves the keyframe, chosen by the keys held when it starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grab {
    /// On the level it stands at.
    Slide,
    /// Up and down (Ctrl).
    Lift,
    /// Turned on itself, its heading (Ctrl + Shift).
    Spin,
}

impl Grab {
    pub fn of(mods: egui::Modifiers) -> Grab {
        match (mods.command, mods.shift) {
            (true, true) => Grab::Spin,
            (true, false) => Grab::Lift,
            _ => Grab::Slide,
        }
    }
}

/// The ray under the pointer at `at` (egui points): a point on it and its
/// direction (render space).
pub fn ray(proj: &Projector, at: Pos2) -> Option<(Vec3, Vec3)> {
    if proj.width <= 0.0 || proj.height <= 0.0 {
        return None;
    }
    let x = at.x * proj.ppp / proj.width * 2.0 - 1.0;
    let y = 1.0 - at.y * proj.ppp / proj.height * 2.0;
    let inverse = proj.view_proj.inverse();
    let point = |z: f32| {
        let v = inverse * Vec4::new(x, y, z, 1.0);
        (v.w.abs() > 1e-9).then(|| v.truncate() / v.w).filter(|p| p.is_finite())
    };
    // (two depths every projection keeps finite, reversed or not: the line
    // is the same either way)
    let (a, b) = (point(0.25)?, point(0.75)?);
    let direction = (b - a).normalize_or_zero();
    (direction != Vec3::ZERO).then_some((a, direction))
}

/// Where the ray meets the plane through `point` square to `normal`, when it
/// does in front of the camera.
fn meet(proj: &Projector, (origin, direction): (Vec3, Vec3), point: Vec3, normal: Vec3) -> Option<Vec3> {
    let facing = direction.dot(normal);
    if facing.abs() < 1e-3 {
        return None;
    }
    let hit = origin + direction * ((point - origin).dot(normal) / facing);
    (hit.is_finite() && proj.project(hit).is_some()).then_some(hit)
}

/// The keyframe's view once the pointer went from `from` to `to`, the drag
/// having started on the view `start`. `None`: the pointer is where the
/// move cannot follow (the level seen edge on, past the horizon).
pub fn dragged(proj: &Projector, start: Pose, from: Pos2, to: Pos2, grab: Grab) -> Option<Pose> {
    match grab {
        Grab::Spin => {
            let turn = Quat::from_rotation_z(-(to.x - from.x) * SPIN);
            Some(Pose {
                rotation: (turn * start.rotation).normalize(),
                ..start
            })
        }
        Grab::Slide | Grab::Lift => {
            let (begin, now) = (ray(proj, from)?, ray(proj, to)?);
            let normal = match grab {
                Grab::Slide => Vec3::Z,
                // the upright plane through the keyframe that faces the camera
                _ => Vec3::new(begin.1.x, begin.1.y, 0.0).normalize_or_zero(),
            };
            if normal == Vec3::ZERO {
                return None;
            }
            let moved = meet(proj, now, start.position, normal)? - meet(proj, begin, start.position, normal)?;
            if moved.length() > FARTHEST {
                return None;
            }
            let moved = match grab {
                Grab::Slide => Vec3::new(moved.x, moved.y, 0.0),
                _ => Vec3::new(0.0, 0.0, moved.z),
            };
            Some(Pose {
                position: start.position + moved,
                ..start
            })
        }
    }
}

/// The drag under way: the keyframe, its view when it started, where the
/// pointer was and how it moves it; `cancelled` once Escape put it back.
#[derive(Debug, Clone, Copy)]
struct Held {
    index: usize,
    start: Pose,
    from: Pos2,
    grab: Grab,
    cancelled: bool,
}

fn held_id() -> Id {
    Id::unique("director_key_drag")
}

/// A keyframe's camera in the world, `response` its click target: lit and
/// a hand under the pointer, grabbed and moved by a drag (the keyframe is
/// selected), put back by Escape.
#[allow(clippy::too_many_arguments)]
pub fn key_drag(
    ctx: &egui::Context,
    p: &Palette,
    proj: &Projector,
    d: &mut Director,
    index: usize,
    pose: Pose,
    response: &egui::Response,
    actions: &mut Vec<Action>,
) {
    let held = ctx.data(|data| data.get_temp::<Held>(held_id())).filter(|held| held.index == index);
    if response.hovered() || held.is_some() {
        let painter = super::hud::world_painter(ctx);
        painter.circle_stroke(response.rect.center(), 9.0, Stroke::new(1.5, p.violet_light));
        ctx.set_cursor_icon(if held.is_some() {
            egui::CursorIcon::Grabbing
        } else {
            egui::CursorIcon::Grab
        });
    }
    if response.drag_started()
        && let Some(from) = response.interact_pointer_pos()
    {
        d.select(index);
        let grab = Grab::of(ctx.input(|i| i.modifiers));
        let held = Held {
            index,
            start: pose,
            from,
            grab,
            cancelled: false,
        };
        ctx.data_mut(|data| data.insert_temp(held_id(), held));
        return;
    }
    let Some(mut held) = held else {
        return;
    };
    if response.drag_stopped() || !response.dragged() {
        ctx.data_mut(|data| data.remove::<Held>(held_id()));
        return;
    }
    if held.cancelled {
        return;
    }
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        held.cancelled = true;
        ctx.data_mut(|data| data.insert_temp(held_id(), held));
        actions.push(Action::MoveKey(index, held.start));
        return;
    }
    if let Some(to) = response.interact_pointer_pos()
        && let Some(moved) = dragged(proj, held.start, held.from, to, held.grab)
        && moved != pose
    {
        actions.push(Action::MoveKey(index, moved));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A camera at `eye` looking at `at`, on a 1600 × 900 window.
    fn projector(eye: Vec3, at: Vec3) -> Projector {
        let view = glam::camera::rh::view::look_at_mat4(eye, at, Vec3::Z);
        let proj = glam::camera::rh::proj::vulkan::perspective(1.0, 1600.0 / 900.0, 0.1, 1000.0);
        Projector {
            view_proj: proj * view,
            width: 1600.0,
            height: 900.0,
            ppp: 1.0,
        }
    }

    fn on_screen(proj: &Projector, point: Vec3) -> Pos2 {
        proj.project(point).expect("on screen").0
    }

    #[test]
    fn the_ray_under_the_pointer_goes_through_what_is_there() {
        let proj = projector(Vec3::new(0.0, -20.0, 30.0), Vec3::new(10.0, 0.0, 20.0));
        let point = Vec3::new(12.0, 3.0, 21.0);
        let (origin, direction) = ray(&proj, on_screen(&proj, point)).expect("ray");
        let along = (point - origin).dot(direction);
        assert!((origin + direction * along).distance(point) < 1e-2);
    }

    #[test]
    fn a_keyframe_slides_on_its_level_and_lifts_with_ctrl() {
        let proj = projector(Vec3::new(0.0, -20.0, 30.0), Vec3::new(10.0, 0.0, 20.0));
        let start = Pose::from_view(Vec3::new(10.0, 0.0, 20.0), Vec3::X, Vec3::Z, 1.0);
        let from = on_screen(&proj, start.position);
        // slid: to the point under the pointer at the same height
        let there = Vec3::new(13.0, 4.0, 20.0);
        let slid = dragged(&proj, start, from, on_screen(&proj, there), Grab::Slide).expect("slid");
        assert!(slid.position.distance(there) < 1e-2, "{}", slid.position);
        assert_eq!((slid.rotation, slid.fov), (start.rotation, start.fov));
        // lifted: straight up, wherever the pointer goes sideways
        let above = Vec3::new(10.0, 0.0, 24.0);
        let lifted = dragged(&proj, start, from, on_screen(&proj, above), Grab::Lift).expect("lifted");
        assert!(lifted.position.distance(above) < 1e-2, "{}", lifted.position);
        let aside = on_screen(&proj, above) + egui::vec2(80.0, 0.0);
        let lifted = dragged(&proj, start, from, aside, Grab::Lift).expect("lifted");
        assert!((lifted.position.x - 10.0).abs() < 1e-4 && (lifted.position.y).abs() < 1e-4);
        // no move without a move of the pointer
        let still = dragged(&proj, start, from, from, Grab::Slide).expect("still");
        assert!(still.position.distance(start.position) < 1e-3);
    }

    #[test]
    fn a_keyframe_turns_on_itself_with_ctrl_and_shift() {
        let proj = projector(Vec3::new(0.0, -20.0, 30.0), Vec3::new(10.0, 0.0, 20.0));
        let start = Pose::from_angles(Vec3::new(10.0, 0.0, 20.0), 0.0, -0.2, 0.0, 1.0);
        let from = Pos2::new(800.0, 450.0);
        let turned = dragged(&proj, start, from, from + egui::vec2(-50.0, 30.0), Grab::Spin).expect("turned");
        let (yaw, pitch, _) = turned.angles();
        assert!((yaw - 50.0 * SPIN).abs() < 1e-3, "{yaw}");
        assert!((pitch + 0.2).abs() < 1e-3, "its pitch kept");
        assert_eq!(turned.position, start.position);
    }

    #[test]
    fn a_level_seen_edge_on_does_not_follow() {
        // the camera at the keyframe's height, looking level: the plane is a line
        let proj = projector(Vec3::new(0.0, -20.0, 20.0), Vec3::new(0.0, 0.0, 20.0));
        let start = Pose::from_view(Vec3::new(0.0, 0.0, 20.0), Vec3::X, Vec3::Z, 1.0);
        let from = on_screen(&proj, start.position);
        assert_eq!(dragged(&proj, start, from, from + egui::vec2(30.0, 0.0), Grab::Slide), None);
        // the keys held choose the move
        let mods = |command, shift| egui::Modifiers {
            command,
            shift,
            ..Default::default()
        };
        assert_eq!(Grab::of(mods(false, false)), Grab::Slide);
        assert_eq!(Grab::of(mods(false, true)), Grab::Slide);
        assert_eq!(Grab::of(mods(true, false)), Grab::Lift);
        assert_eq!(Grab::of(mods(true, true)), Grab::Spin);
    }
}
