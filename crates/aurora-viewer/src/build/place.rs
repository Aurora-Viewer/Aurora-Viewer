//! "Place on surface": with the button on, the surface under the cursor
//! shows where the selection would go, and a click puts it there, a chosen
//! side against the surface, centred on the point clicked.
//!
//! An Aurora feature, beside its snap to objects (`object_snap.rs`); no
//! Legacy Viewer has it. The surfaces are those the snap uses: what objects
//! show (the triangles picking uses), and the ground. The move is sent as
//! the Align tool's (`TransformUpdate` per linkset root), so only the
//! selection's move right counts.

use super::object_snap::{self, support};
use super::{Bounds, BuildSettings, BuildTool, EditMode, SelectType, Tool, draw::Painter3d, draw::fade};
use crate::scene::Scene;
use crate::world::World;
use aurora_net::build::{BuildCmd, TransformUpdate};
use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::time::Instant;

/// The side of the selection put against the surface, in its own axes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    /// The side already facing the surface: nothing turns unless "Align to
    /// surface" is on.
    #[default]
    Auto,
    Bottom,
    Top,
    Front,
    Back,
    Left,
    Right,
}

impl Side {
    pub const ALL: [Side; 7] = [
        Side::Auto,
        Side::Bottom,
        Side::Top,
        Side::Front,
        Side::Back,
        Side::Left,
        Side::Right,
    ];

    /// Its outward direction in the object's frame (SL: X forward, Y left).
    fn local(self) -> Option<Vec3> {
        Some(match self {
            Side::Auto => return None,
            Side::Bottom => -Vec3::Z,
            Side::Top => Vec3::Z,
            Side::Front => Vec3::X,
            Side::Back => -Vec3::X,
            Side::Left => Vec3::Y,
            Side::Right => -Vec3::Y,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            Side::Auto => aurora_i18n::tr!("build-place-side-auto"),
            Side::Bottom => aurora_i18n::tr!("build-place-side-bottom"),
            Side::Top => aurora_i18n::tr!("build-place-side-top"),
            Side::Front => aurora_i18n::tr!("build-place-side-front"),
            Side::Back => aurora_i18n::tr!("build-place-side-back"),
            Side::Left => aurora_i18n::tr!("build-place-side-left"),
            Side::Right => aurora_i18n::tr!("build-place-side-right"),
        }
    }
}

/// Where a click would put the selection.
#[derive(Debug, Clone, Copy)]
pub struct Landing {
    /// The point under the cursor and its surface's normal (toward the camera).
    pub point: Vec3,
    pub normal: Vec3,
    /// The selection's shown box now, and where it lands.
    pub from: Bounds,
    pub to: Bounds,
    /// The turn about the box's centre (identity when it only moves).
    pub turn: Quat,
}

/// The button's state, for the session.
#[derive(Debug, Default)]
pub struct Place {
    pub active: bool,
    pub preview: Option<Landing>,
}

/// Where the selection's box `sel` goes on the surface at `point`: the
/// `side` against it (or, for Auto, the side already facing it), turned
/// flat on it when a side is chosen or `align` is on; resting on the plane,
/// its centre over the point.
pub fn landing(sel: &Bounds, point: Vec3, normal: Vec3, side: Side, align: bool) -> Landing {
    let n = normal.normalize_or(Vec3::Z);
    let facing = || {
        (0..3)
            .flat_map(|i| [1.0, -1.0].map(|s| sel.rotation * (unit(i) * s)))
            .min_by(|a, b| a.dot(n).total_cmp(&b.dot(n)))
            .unwrap_or(-n)
    };
    let (against, turns) = match side.local() {
        Some(l) => (sel.rotation * l, true),
        None => (facing(), align),
    };
    let turn = if turns {
        Quat::from_rotation_arc(against, -n)
    } else {
        Quat::IDENTITY
    };
    let mut to = Bounds {
        center: point,
        rotation: (turn * sel.rotation).normalize(),
        half: sel.half,
    };
    to.center = point + n * support(&to, n);
    Landing {
        point,
        normal: n,
        from: *sel,
        to,
        turn,
    }
}

fn unit(i: usize) -> Vec3 {
    let mut v = Vec3::ZERO;
    v[i] = 1.0;
    v
}

/// The ground's normal at `p`, from the height map around it.
fn ground_normal(world: &World, p: Vec3) -> Vec3 {
    let e = 0.5;
    let h = |dx: f32, dy: f32| world.ground_height(p + Vec3::new(dx, dy, 0.0)).unwrap_or(p.z);
    Vec3::new(h(-e, 0.0) - h(e, 0.0), h(0.0, -e) - h(0.0, e), 2.0 * e).normalize_or(Vec3::Z)
}

impl BuildTool {
    /// Whether the selection can be placed: whole objects in the world that
    /// may all be moved, with the Edit tool.
    pub fn can_place(&self, world: &World) -> bool {
        self.tool == Tool::Edit
            && !self.selection.is_empty()
            && !self.selection_linked
            && self.select_type(world) == SelectType::World
            && super::manip::can_transform_selection(world, &self.selection, false, EditMode::Move)
    }

    pub fn toggle_place(&mut self, world: &World) {
        self.place.active = !self.place.active && self.can_place(world);
        self.place.preview = None;
    }

    /// The landing under the cursor, every frame while the button is on.
    pub(super) fn place_hover(&mut self, world: &World, s: &BuildSettings, scene: Option<&Scene>, cursor: (f32, f32), over_ui: bool) {
        self.place.preview = None;
        if !self.can_place(world) {
            self.place.active = false;
            return;
        }
        if over_ui {
            return;
        }
        let now = Instant::now();
        let Some(edit_box) = super::bounds_of(world, &self.selection, false, now) else {
            return;
        };
        let moving: HashSet<usize> = super::highlighted_of(world, &self.selection, false)
            .into_iter()
            .map(|(i, _)| i)
            .collect();
        let sel = scene
            .and_then(|scene| object_snap::shown_bounds(world, scene, &moving, &edit_box, now))
            .unwrap_or(edit_box);
        let ray = self.cam.ray(cursor.0, cursor.1);
        // the first object surface along the ray, past the selection
        let object = scene.and_then(|scene| {
            scene
                .left_edit_ray_hits(world, ray, false, s.select_probes, s.select_limit())
                .into_iter()
                .filter(|i| !moving.contains(i) && world.objects.get(*i).is_some_and(|o| !o.is_avatar()))
                .find_map(|i| {
                    let o = world.objects.get(i)?;
                    let t = scene.left_touch_surface(world, i, ray)?;
                    Some((t.position + world.region_offset(o.key.region)?, t.normal))
                })
        });
        let ground = super::land::terrain_hit(world, ray.0, ray.1).map(|p| (p, ground_normal(world, p)));
        let nearest = [object, ground]
            .into_iter()
            .flatten()
            .min_by(|a, b| a.0.distance_squared(ray.0).total_cmp(&b.0.distance_squared(ray.0)));
        let Some((point, normal)) = nearest else { return };
        // the side the camera sees
        let normal = if normal.dot(ray.1) > 0.0 { -normal } else { normal };
        self.place.preview = Some(landing(&sel, point, normal, s.place_side, s.place_align));
    }

    /// A click while the button is on: put the selection where the preview
    /// shows, root by root (the Align tool's way). The button stays on with
    /// Shift.
    pub(super) fn place_click(&mut self, world: &mut World, keep: bool) {
        let Some(l) = self.place.preview.take() else { return };
        let now = Instant::now();
        let mut by_region: HashMap<aurora_net::RegionHandle, Vec<TransformUpdate>> = HashMap::new();
        for root in self.roots(world) {
            let Some((at, _, _)) = Scene::object_transform(world, root, now, 0) else {
                continue;
            };
            let delta = l.to.center + l.turn * (at - l.from.center) - at;
            let Some(o) = world.objects.get_mut(root) else { continue };
            o.position += delta;
            let turned = l.turn != Quat::IDENTITY;
            if turned {
                o.rotation = (l.turn * o.rotation).normalize();
            }
            o.velocity = Vec3::ZERO;
            o.render.needs_records = true;
            by_region.entry(o.key.region).or_default().push(TransformUpdate {
                local_id: o.key.local_id,
                position: Some(o.position),
                rotation: turned.then_some(o.rotation),
                scale: None,
                linked: true,
                uniform: false,
            });
        }
        for (handle, updates) in by_region {
            self.send(BuildCmd::Transform { handle, updates });
        }
        self.place.active = keep;
    }

    /// The preview: a pad on the surface under the landing, and the
    /// selection's box where it would go.
    pub(super) fn place_draw(&self, p: &mut Painter3d) {
        let Some(l) = self.place.preview else { return };
        let pal = p.pal;
        let (n, to) = (l.normal, l.to);
        // the box's two axes across the surface, flattened onto it
        let mut across: Vec<Vec3> = (0..3).map(|i| to.rotation * unit(i) * to.half[i]).collect();
        across.sort_by(|a, b| a.normalize_or_zero().dot(n).abs().total_cmp(&b.normalize_or_zero().dot(n).abs()));
        let flat = |v: Vec3| (v - n * v.dot(n)) * 1.2;
        let (a, b) = (flat(across[0]), flat(across[1]));
        let c = l.point + n * 0.002;
        let pad = [c - a - b, c + a - b, c + a + b, c - a + b];
        p.tri_plain(pad[0], pad[1], pad[2], fade(pal.violet, 0.25));
        p.tri_plain(pad[0], pad[2], pad[3], fade(pal.violet, 0.25));
        p.flush();
        for k in 0..4 {
            p.line_cased(pad[k], pad[(k + 1) % 4], pal.violet_light, 2.0);
        }
        // the ghost of the selection, edge by edge
        let corner = |m: u8| {
            let s = Vec3::new(
                if m & 1 != 0 { 1.0 } else { -1.0 },
                if m & 2 != 0 { 1.0 } else { -1.0 },
                if m & 4 != 0 { 1.0 } else { -1.0 },
            );
            to.center + to.rotation * (to.half * s)
        };
        for m in 0u8..8 {
            for bit in [1u8, 2, 4] {
                if m & bit == 0 {
                    p.line(corner(m), corner(m | bit), fade(pal.violet_light, 0.85), 1.5);
                }
            }
        }
        p.square(l.point, 6.0, pal.violet_light, egui::Stroke::new(1.0, fade(pal.navy, 0.5)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube(center: Vec3, half: Vec3, rotation: Quat) -> Bounds {
        Bounds { center, rotation, half }
    }

    #[test]
    fn auto_puts_the_side_facing_the_surface_on_it_without_turning() {
        let sel = cube(Vec3::new(5.0, 5.0, 9.0), Vec3::new(0.5, 0.3, 0.2), Quat::from_rotation_z(0.4));
        // on a floor: its bottom, centred over the point
        let l = landing(&sel, Vec3::new(1.0, 2.0, 3.0), Vec3::Z, Side::Auto, false);
        assert_eq!(l.turn, Quat::IDENTITY);
        assert!(l.to.center.abs_diff_eq(Vec3::new(1.0, 2.0, 3.2), 1e-5), "{:?}", l.to);
        // on a wall facing +X: its side toward the wall
        let l = landing(&sel, Vec3::new(1.0, 2.0, 3.0), Vec3::X, Side::Auto, false);
        assert!((l.to.center.x - (1.0 + support(&sel, Vec3::X))).abs() < 1e-5);
        assert_eq!((l.to.center.y, l.to.center.z), (2.0, 3.0));
    }

    #[test]
    fn a_tilted_selection_rests_its_corner_unless_aligned() {
        let tilted = Quat::from_rotation_x(0.3);
        let sel = cube(Vec3::ZERO, Vec3::splat(0.5), tilted);
        let l = landing(&sel, Vec3::ZERO, Vec3::Z, Side::Auto, false);
        assert_eq!(l.to.rotation, tilted);
        assert!(l.to.center.z > 0.5, "on its corner, higher than flat");
        let flat = landing(&sel, Vec3::ZERO, Vec3::Z, Side::Auto, true);
        assert!((flat.to.rotation * -Vec3::Z).abs_diff_eq(-Vec3::Z, 1e-5), "its bottom down");
        assert!((flat.to.center.z - 0.5).abs() < 1e-5);
    }

    #[test]
    fn a_chosen_side_turns_flat_against_the_surface() {
        let sel = cube(Vec3::ZERO, Vec3::new(0.1, 0.6, 0.4), Quat::IDENTITY);
        // a picture: its back against a wall facing -Y, slanted 10°
        let wall = (Quat::from_rotation_x(0.17) * -Vec3::Y).normalize();
        let l = landing(&sel, Vec3::new(3.0, 8.0, 2.0), wall, Side::Back, false);
        assert!((l.to.rotation * -Vec3::X).abs_diff_eq(-wall, 1e-5), "its back on the wall");
        // half its depth off the wall, centred on the point
        assert!((l.to.center - Vec3::new(3.0, 8.0, 2.0)).abs_diff_eq(wall * 0.1, 1e-5));
        // its top on a ceiling facing down
        let up = landing(&sel, Vec3::new(0.0, 0.0, 4.0), -Vec3::Z, Side::Top, false);
        assert!((up.to.rotation * Vec3::Z).abs_diff_eq(Vec3::Z, 1e-5));
        assert!((up.to.center.z - 3.6).abs() < 1e-5);
    }

    #[test]
    fn sides_and_settings_keep_their_names() {
        assert_eq!(serde_json::to_string(&Side::Bottom).unwrap(), "\"bottom\"");
        let s: BuildSettings = serde_json::from_str("{}").unwrap();
        assert_eq!((s.place_side, s.place_align), (Side::Auto, false));
    }
}
