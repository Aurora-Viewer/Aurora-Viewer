//! Offline test script of the build tools (demo mode):
//! `AURORA_DEMO_BUILD="mode,x,y[,part,dx,dy]"`.
//!
//! - mode: move | rotate | stretch | create | land | select
//! - x, y: click (physical pixels) at frame 220: selects the object there,
//!   creates a prim, or presses the land brush (held until the capture)
//! - part: handle grabbed at frame 250 (move: x y z xy yz xz; rotate: rx ry
//!   rz roll free; stretch: c0..c7, fx fy fz (positive faces)), then dragged
//!   by dx, dy pixels over 40 frames and held for the capture.
//!
//! Frames are counted from AURORA_DEMO_BUILD_FRAME (600 by default: after
//! the loading fade; capture at about +90 frames).

use super::manip::Part;
use super::{BuildSettings, BuildTool, EditMode, Tool};
use crate::world::World;
use glam::Vec3;
use std::time::Instant;

pub enum Step {
    Open(Tool, EditMode, u8),
    Cursor(f32, f32),
    Press,
    Release,
}

fn part_of(s: &str) -> Option<Part> {
    Some(match s {
        "x" => Part::Arrow(0),
        "y" => Part::Arrow(1),
        "z" => Part::Arrow(2),
        "yz" => Part::Plane(0),
        "xz" => Part::Plane(1),
        "xy" => Part::Plane(2),
        "rx" => Part::Ring(0),
        "ry" => Part::Ring(1),
        "rz" => Part::Ring(2),
        "roll" => Part::Roll,
        "free" => Part::Free,
        "fx" => Part::Face(0, true),
        "fy" => Part::Face(1, true),
        "fz" => Part::Face(2, true),
        c if c.starts_with('c') => Part::Corner(c[1..].parse().ok()?),
        _ => return None,
    })
}

/// Screen position (physical pixels) of a handle of the current selection.
fn handle_px(tool: &BuildTool, world: &World, s: &BuildSettings, part: Part) -> Option<(f32, f32)> {
    let now = Instant::now();
    let b = tool.bounds(world, now)?;
    let g = super::grid_of(s, Some(&b));
    let cam = &tool.cam;
    let c = b.center;
    let len = cam.meters_for_pixels(c, 50.0);
    let unit = |i: usize| {
        let mut v = Vec3::ZERO;
        v[i] = 1.0;
        v
    };
    let p = match part {
        Part::Arrow(i) => c + g.rotation * unit(i) * len * 0.9,
        Part::Plane(i) => {
            let at_g = g.rotation.inverse() * cam.at;
            let mut v = Vec3::ZERO;
            for j in 0..3 {
                if j != i {
                    v[j] = at_g[j].signum() * 1.8 * len;
                }
            }
            c + g.rotation * v
        }
        Part::Ring(i) => {
            let r = cam.depth(c).max(0.01) * (100.0 / cam.height * cam.fov_y).tan();
            let a = g.rotation * unit(i);
            // the ring point facing the camera most
            let to_cam = (cam.eye - c).normalize_or_zero();
            let d = (to_cam - a * to_cam.dot(a)).try_normalize().unwrap_or(a.any_orthonormal_vector());
            c + d * r
        }
        Part::Free => c,
        Part::Roll => {
            let r = cam.depth(c).max(0.01) * (100.0 / cam.height * cam.fov_y).tan();
            c + cam.up * r * 1.08
        }
        Part::Corner(k) => {
            let sg = Vec3::new(
                if k & 1 != 0 { 1.0 } else { -1.0 },
                if k & 2 != 0 { 1.0 } else { -1.0 },
                if k & 4 != 0 { 1.0 } else { -1.0 },
            );
            c + b.rotation * (b.half * sg)
        }
        Part::Face(i, pos) => c + b.rotation * unit(i) * b.half[i] * if pos { 1.0 } else { -1.0 },
    };
    cam.project_px(p)
}

/// What to do at this frame.
pub fn steps(spec: &str, frame: u64, tool: &BuildTool, world: &World, s: &BuildSettings) -> Vec<Step> {
    let parts: Vec<&str> = spec.split(',').map(|x| x.trim()).collect();
    let num = |i: usize| parts.get(i).and_then(|v| v.parse::<f32>().ok());
    let (Some(x), Some(y)) = (num(1), num(2)) else {
        return Vec::new();
    };
    let (t, m, land) = match parts[0] {
        "rotate" => (Tool::Edit, EditMode::Rotate, 6),
        "stretch" => (Tool::Edit, EditMode::Stretch, 6),
        "create" => (Tool::Create, EditMode::Move, 6),
        "land" => (Tool::Land, EditMode::Move, 1),
        "select" => (Tool::Land, EditMode::Move, 6),
        _ => (Tool::Edit, EditMode::Move, 6),
    };
    let handle = parts.get(3).and_then(|p| part_of(p));
    // script start (AURORA_DEMO_BUILD_FRAME, after the loading fade by default)
    let base: u64 = std::env::var("AURORA_DEMO_BUILD_FRAME")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(600);
    let Some(frame) = (frame + 215).checked_sub(base) else {
        return Vec::new();
    };
    let (dx, dy) = (num(4).unwrap_or(120.0), num(5).unwrap_or(0.0));
    match frame {
        215 => vec![Step::Open(t, m, land)],
        220 => vec![Step::Cursor(x, y), Step::Press],
        // land brush / selection: held (land: no release), others released
        226 if t != Tool::Land => vec![Step::Release],
        235 if parts[0] == "select" => vec![Step::Cursor(x + dx, y + dy)],
        240 if parts[0] == "select" => vec![Step::Release],
        250 => match handle.and_then(|h| handle_px(tool, world, s, h)) {
            Some((hx, hy)) => {
                log::info!("demo build: grabbing {:?} at {hx:.0},{hy:.0}", handle);
                vec![Step::Cursor(hx, hy), Step::Press]
            }
            None => Vec::new(),
        },
        251..=290 if handle.is_some() && tool.manip.drag.is_some() => {
            let k = (frame - 250) as f32 / 40.0;
            vec![Step::Cursor(x_start(tool) + dx * k, y_start(tool) + dy * k)]
        }
        _ => Vec::new(),
    }
}

fn x_start(tool: &BuildTool) -> f32 {
    tool.manip.drag.as_ref().map(|d| d.start_cursor.0).unwrap_or(0.0)
}

fn y_start(tool: &BuildTool) -> f32 {
    tool.manip.drag.as_ref().map(|d| d.start_cursor.1).unwrap_or(0.0)
}
