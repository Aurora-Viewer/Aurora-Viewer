//! Offline test script of the build tools (demo mode):
//! `AURORA_DEMO_BUILD="mode,x,y[,part,dx,dy]"`.
//!
//! - mode: move | rotate | stretch | face | align | grab | focus | create |
//!   land | select
//!   cycle | cycle-linked: nested objects, three CTRL + wheel notches
//!   (deeper, deeper, nearer), centered on `object:99610`.
//!   linked-click | linked-click-ready: select a whole object, then switch
//!   to linked editing and click the requested part (ready stops before switching).
//! - x, y: click (physical pixels) at frame 220: selects the object there,
//!   creates a prim, or presses the land brush (held until the capture)
//! - part: handle grabbed at frame 250 (move: x y z xy yz xz; rotate: rx ry
//!   rz roll free; stretch: c0..c7, fx fy fz (positive faces)), then dragged
//!   by dx, dy pixels over 40 frames and held for the capture.
//!
//! Frames are counted from AURORA_DEMO_BUILD_FRAME (600 by default: after
//! the loading fade; capture at about +90 frames).
//! `mode,object:<local_id>[,part,dx,dy]` selects a demo object directly,
//! so an animated flexible shape needs no fixed screen click coordinate.
//!
//! `AURORA_DEMO_BUILD_TAB` shows a section: general | object | transform |
//! shape | features | texture | contents, the texture tab with `:pbr`, `:bp` or
//! `:media` (e.g. `texture:bp`), and opens `+weights`, `+grid`, `+media`
//! (e.g. `general+weights+grid`).
//! `AURORA_DEMO_BUILD_FOCUS=off|on` focuses the selected object before the
//! drag, with edit camera tracking disabled/enabled.
//! `AURORA_DEMO_BUILD_EDGE=left|right|top|bottom` moves the cursor to an
//! edge, holds it for 60 frames and releases the move handle at base +136.

use super::manip::Part;
use super::{BuildSettings, BuildTool, EditMode, Tool};
use crate::world::World;
use glam::Vec3;
use std::time::Instant;

pub enum Step {
    Open(Tool, EditMode, u8),
    Select(usize),
    Cursor(f32, f32),
    Press,
    Release,
    Focus(bool),
    Linked(bool),
    Wheel(f32),
    ReplaceBegin(bool),
    ReplacePick(usize),
    ReplaceApply,
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
    let f = super::manip::frame(world, s, &tool.selection, tool.selection_linked, now, None)?;
    let b = f.bounds;
    let g = f.grid;
    let cam = &tool.cam;
    let c = f.pivot;
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
            b.center + b.rotation * (b.half * sg)
        }
        Part::Face(i, pos) => b.center + b.rotation * unit(i) * b.half[i] * if pos { 1.0 } else { -1.0 },
    };
    cam.project_px(p)
}

/// AURORA_DEMO_BUILD_TAB: the floater tab (and texture sub-tab) to show.
pub fn apply_tab(tool: &mut BuildTool) {
    let Ok(v) = std::env::var("AURORA_DEMO_BUILD_TAB") else { return };
    // secondary floaters: +weights, +grid, +media
    let mut parts = v.split('+');
    let v = parts.next().unwrap_or("").to_owned();
    for extra in parts {
        match extra.trim() {
            "weights" => tool.ui.weights_open = true,
            "grid" => tool.ui.grid_options_open = true,
            "media" => tool.ui.media_settings = Some(super::ui::MediaSettingsState::new(None)),
            _ => {}
        }
    }
    let (tab, sub) = v.split_once(':').unwrap_or((v.as_str(), ""));
    tool.tab = match tab.trim() {
        "general" => 0,
        "object" | "transform" => 1,
        "shape" => 5,
        "features" => 2,
        "texture" => 3,
        "contents" => 4,
        _ => tool.tab,
    };
    tool.ui.tex_tab = match sub.trim() {
        "pbr" => 0,
        "bp" => 1,
        "media" => 2,
        _ => tool.ui.tex_tab,
    };
}

/// Optional direct object selection, used for animated-shape captures.
pub fn target_index(spec: &str, world: &World) -> Option<usize> {
    let id = spec.split(',').nth(1)?.trim().strip_prefix("object:")?.parse::<u32>().ok()?;
    world.objects.iter().find_map(|(i, o)| (o.key.local_id == id).then_some(i))
}

/// What to do at this frame.
pub fn steps(spec: &str, frame: u64, tool: &BuildTool, world: &World, s: &BuildSettings) -> Vec<Step> {
    let parts: Vec<&str> = spec.split(',').map(|x| x.trim()).collect();
    let num = |i: usize| parts.get(i).and_then(|v| v.parse::<f32>().ok());
    let object = target_index(spec, world);
    let (x, y) = match (object, num(1), num(2)) {
        (Some(_), _, _) => (0.0, 0.0),
        (_, Some(x), Some(y)) => (x, y),
        _ => return Vec::new(),
    };
    let cycle = matches!(parts[0], "cycle" | "cycle-linked");
    let (t, m, land) = match parts[0] {
        "rotate" => (Tool::Edit, EditMode::Rotate, 6),
        "stretch" => (Tool::Edit, EditMode::Stretch, 6),
        "face" => (Tool::Edit, EditMode::Face, 6),
        "align" => (Tool::Edit, EditMode::Align, 6),
        "grab" => (Tool::Grab, EditMode::Move, 6),
        "focus" => (Tool::Focus, EditMode::Move, 6),
        "create" => (Tool::Create, EditMode::Move, 6),
        "land" => (Tool::Land, EditMode::Move, 1),
        "select" => (Tool::Land, EditMode::Move, 6),
        _ => (Tool::Edit, EditMode::Move, 6),
    };
    let h = if object.is_some() { 2 } else { 3 };
    let handle = parts.get(h).and_then(|p| part_of(p));
    // script start (AURORA_DEMO_BUILD_FRAME, after the loading fade by default)
    let base: u64 = std::env::var("AURORA_DEMO_BUILD_FRAME")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(600);
    let Some(frame) = (frame + 215).checked_sub(base) else {
        return Vec::new();
    };
    if matches!(parts[0], "linked-click" | "linked-click-ready") {
        return match frame {
            215 => vec![Step::Open(Tool::Edit, EditMode::Move, 1), Step::Linked(false)],
            220 => object.into_iter().map(Step::Select).collect(),
            227 => vec![Step::Focus(false)],
            240 if parts[0] == "linked-click" => vec![Step::Linked(true)],
            250 if parts[0] == "linked-click" => object.into_iter().map(Step::Select).collect(),
            _ => Vec::new(),
        };
    }
    if parts[0].starts_with("replace") {
        return match frame {
            215 => vec![Step::Open(Tool::Edit, EditMode::Move, 1)],
            220 => {
                let mut steps = vec![Step::Linked(true)];
                steps.extend(object.into_iter().map(Step::Select));
                steps
            }
            227 => vec![Step::Focus(false)],
            230 if parts[0] != "replace-button" => vec![Step::ReplaceBegin(parts[0] == "replace-copy")],
            250 if parts[0] != "replace-button" => world
                .objects
                .index_of_uuid(&super::replace::demo::id(super::replace::demo::SOURCE))
                .map(|i| vec![Step::ReplacePick(i)])
                .unwrap_or_default(),
            270 if matches!(parts[0], "replace-run" | "replace-copy") => vec![Step::ReplaceApply],
            _ => Vec::new(),
        };
    }
    let (dx, dy) = (num(h + 1).unwrap_or(120.0), num(h + 2).unwrap_or(0.0));
    let edge = (t == Tool::Edit && m == EditMode::Move)
        .then(|| std::env::var("AURORA_DEMO_BUILD_EDGE").ok())
        .flatten()
        .and_then(|v| match v.as_str() {
            "left" => Some((1.0, y_start(tool))),
            "right" => Some((tool.cam.width - 1.0, y_start(tool))),
            "top" => Some((x_start(tool), 1.0)),
            "bottom" => Some((x_start(tool), tool.cam.height - 1.0)),
            _ => None,
        });
    match frame {
        215 => vec![Step::Open(t, m, land)],
        220 if object.is_some() => object.into_iter().map(Step::Select).collect(),
        220 => vec![Step::Cursor(x, y), Step::Press],
        // land brush / selection: held (land: no release), others released
        226 if t != Tool::Land => vec![Step::Release],
        227 if t == Tool::Edit => match std::env::var("AURORA_DEMO_BUILD_FOCUS").as_deref() {
            Ok("off") => vec![Step::Focus(false)],
            Ok("on") => vec![Step::Focus(true)],
            _ if cycle => vec![Step::Focus(false)],
            _ => Vec::new(),
        },
        240 if cycle => {
            let mut steps = vec![Step::Linked(parts[0] == "cycle-linked")];
            steps.extend(object.into_iter().map(Step::Select));
            steps
        }
        250 if cycle => object
            .and_then(|idx| crate::scene::Scene::object_transform(world, idx, Instant::now(), 0))
            .and_then(|(p, _, _)| tool.cam.project_px(p))
            .map(|(x, y)| vec![Step::Cursor(x, y)])
            .unwrap_or_default(),
        260 | 270 if cycle => vec![Step::Wheel(-1.0)],
        280 if cycle => vec![Step::Wheel(1.0)],
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
            let (ex, ey) = edge.unwrap_or((x_start(tool) + dx, y_start(tool) + dy));
            vec![Step::Cursor(
                x_start(tool) + (ex - x_start(tool)) * k,
                y_start(tool) + (ey - y_start(tool)) * k,
            )]
        }
        291..=350 if edge.is_some() && tool.manip.drag.is_some() => {
            let (x, y) = edge.unwrap_or_default();
            vec![Step::Cursor(x, y)]
        }
        351 if edge.is_some() => vec![Step::Release],
        _ => Vec::new(),
    }
}

fn x_start(tool: &BuildTool) -> f32 {
    tool.manip.drag.as_ref().map(|d| d.start_cursor.0).unwrap_or(0.0)
}

fn y_start(tool: &BuildTool) -> f32 {
    tool.manip.drag.as_ref().map(|d| d.start_cursor.1).unwrap_or(0.0)
}
